"""Tests for Tri-Polar Graph Engineering (`DAG-ULW++`) and `oh-my-hermes`-style TUI CLI."""

from __future__ import annotations

import ast
import json
from pathlib import Path

import pytest

from app.core.cli_tui import HeterarchyTuiSession
from app.core.ctl import HeterarchyCliController
from app.core.dag_enums import (
    DagHazardCode,
    DagIsolationMode,
    DagNodeKind,
    DagNodeRoleKind,
    DagRunState,
    DagSettlementStage,
)
from app.repositories.job_store import JobStore
from app.schemas.boundary_schemas import JsonObject
from app.schemas.dag_input_schemas import (
    DagNodeInputModel,
    DagSpawnPlanInputModel,
    DagUlwRequestModel,
)
from app.server import heterarchy_dag_ulw
from app.services.dag_ulw_service import DagUlwExecutionService


@pytest.mark.asyncio
async def test_dag_ulw_plan_only_and_default_triad_topology(tmp_path: Path) -> None:
    """Verify `heterarchy_dag_ulw(plan_only=True)` compiles a 4-node 3-wave Tri-Polar graph."""
    raw = await heterarchy_dag_ulw(
        campaign_goal="Build 3-tier schema validator",
        workspace_dir=str(tmp_path),
        plan_only=True,
    )
    payload = json.loads(raw)
    assert payload["ok"] is True
    assert payload["run_state"] == str(DagRunState.PLANNED)
    assert payload["plan_only"] is True
    plan = payload["compiled_plan"]
    assert plan["total_nodes"] == 4
    assert plan["total_waves"] == 3
    assert plan["blocking_hazards_count"] == 0
    assert "Wave 0" in plan["ascii_graph"]
    assert "n1_topology_spec" in payload["wave_tree_view"]


@pytest.mark.asyncio
async def test_dag_ulw_detects_waw_raw_cycle_and_spawn_plan_hazards(tmp_path: Path) -> None:
    """Verify WAW conflict, missing RAW dependency, cycle, and SpawnPlanGate hazards."""
    service = DagUlwExecutionService()
    store = JobStore(state_dir=tmp_path / "jobs")

    async def _noop_exec(**_kw: object) -> JsonObject:
        return {"ok": True, "status": "succeeded"}

    # 1. WAW conflict in Wave 0 + missing SpawnPlan on plan_only=False
    waw_req = DagUlwRequestModel(
        campaign_goal="Concurrent write conflict test",
        workspace_dir=str(tmp_path),
        plan_only=False,
        nodes=(
            DagNodeInputModel(
                node_id="w1",
                role=DagNodeRoleKind.DISJOINT_BUILDER,
                task="Write target.py",
                write_files=("app/target.py",),
            ),
            DagNodeInputModel(
                node_id="w2",
                role=DagNodeRoleKind.DISJOINT_BUILDER,
                task="Also write target.py",
                write_files=("app/target.py",),
            ),
        ),
    )
    waw_out = json.loads(
        await service.run_dag_campaign(
            req=waw_req,
            job_store=store,
            run_agy_fn=_noop_exec,
            execute_fn=_noop_exec,
        )
    )
    assert waw_out["ok"] is False
    assert waw_out["run_state"] == str(DagRunState.REJECTED_PREFLIGHT)
    codes = {h["code"] for h in waw_out["compiled_plan"]["hazards"]}
    assert str(DagHazardCode.DAG_WAW_SCOPE_CONFLICT) in codes
    assert str(DagHazardCode.DAG_SPAWN_PLAN_REQUIRED) in codes

    # 2. Cycle detection
    cyc_req = DagUlwRequestModel(
        campaign_goal="Cycle test",
        workspace_dir=str(tmp_path),
        plan_only=True,
        nodes=(
            DagNodeInputModel(node_id="a", task="A", depends_on=("b",)),
            DagNodeInputModel(node_id="b", task="B", depends_on=("a",)),
        ),
    )
    cyc_out = json.loads(
        await service.run_dag_campaign(
            req=cyc_req,
            job_store=store,
            run_agy_fn=_noop_exec,
            execute_fn=_noop_exec,
        )
    )
    assert cyc_out["ok"] is False
    cyc_codes = {h["code"] for h in cyc_out["compiled_plan"]["hazards"]}
    assert str(DagHazardCode.DAG_CYCLE_DETECTED) in cyc_codes


@pytest.mark.asyncio
async def test_dag_ulw_two_stage_settlement_and_out_of_scope_write_rejection(
    tmp_path: Path,
) -> None:
    """Verify `STOPPED != SETTLED` rejects out-of-scope file writes and blocks downstream."""
    ws = tmp_path / "ws"
    ws.mkdir()
    (ws / "allowed.py").write_text("x = 1\n", encoding="utf-8")
    (ws / "forbidden.py").write_text("y = 1\n", encoding="utf-8")
    store = JobStore(state_dir=tmp_path / "jobs")
    service = DagUlwExecutionService()

    async def _rogue_writer(**_kw: object) -> JsonObject:
        # Mutate forbidden.py which is NOT in write_files!
        (ws / "forbidden.py").write_text("y = 999\n", encoding="utf-8")
        return {
            "ok": True,
            "status": "succeeded",
            "model_id": "claude-sonnet-5-5-medium",
            "structured_output": {"verdict": "PASS", "summary": "Mutated forbidden file"},
        }

    req = DagUlwRequestModel(
        campaign_goal="Test out-of-scope write rejection",
        workspace_dir=str(ws),
        plan_only=False,
        spawn_plan=DagSpawnPlanInputModel(
            plan_summary="Acknowledge 2-node execution",
            hazard_acknowledgment=True,
            isolation_mode=DagIsolationMode.DIRECT,
        ),
        nodes=(
            DagNodeInputModel(
                node_id="n1_rogue",
                role=DagNodeRoleKind.DISJOINT_BUILDER,
                task="Modify allowed.py only",
                write_files=("allowed.py",),
            ),
            DagNodeInputModel(
                node_id="n2_downstream",
                role=DagNodeRoleKind.VERIFICATION_FAN_IN,
                task="Verify after n1_rogue",
                depends_on=("n1_rogue",),
            ),
        ),
    )
    res = json.loads(
        await service.run_dag_campaign(
            req=req,
            job_store=store,
            run_agy_fn=_rogue_writer,
            execute_fn=_rogue_writer,
        )
    )
    assert res["ok"] is False
    assert res["run_state"] == str(DagRunState.FAILED)
    receipts = {r["node_id"]: r for r in res["node_receipts"]}
    assert receipts["n1_rogue"]["settlement_stage"] == str(
        DagSettlementStage.REJECTED_OUT_OF_SCOPE_WRITE
    )
    assert "forbidden.py" in receipts["n1_rogue"]["out_of_scope_writes"]
    assert receipts["n2_downstream"]["settlement_stage"] == str(DagSettlementStage.BLOCKED_UPSTREAM)


@pytest.mark.asyncio
async def test_dag_ulw_reproduction_hold_and_checkpoint_resume(tmp_path: Path) -> None:
    """Verify ReproductionHold gate and `resume_dag_id` checkpoint reuse of SETTLED nodes."""
    ws = tmp_path / "ws"
    ws.mkdir()
    (ws / "mod.py").write_text("val = 42\n", encoding="utf-8")
    store = JobStore(state_dir=tmp_path / "jobs")
    service = DagUlwExecutionService()

    call_count = 0

    async def _clean_exec(**_kw: object) -> JsonObject:
        nonlocal call_count
        call_count += 1
        return {
            "ok": True,
            "status": "succeeded",
            "model_id": "gpt-6-astra",
            "structured_output": {
                "verdict": "BLOCK",
                "summary": "Reproduced bug in mod.py",
            },
        }

    req = DagUlwRequestModel(
        campaign_goal="Reproduce and settle",
        workspace_dir=str(ws),
        plan_only=False,
        spawn_plan=DagSpawnPlanInputModel(plan_summary="Run repro + fix"),
        nodes=(
            DagNodeInputModel(
                node_id="n1_repro",
                role=DagNodeRoleKind.REPRODUCTION_PROBER,
                node_kind=DagNodeKind.REPRODUCTION,
                task="Reproduce bug",
                read_files=("mod.py",),
            ),
            DagNodeInputModel(
                node_id="n2_verify",
                role=DagNodeRoleKind.VERIFICATION_FAN_IN,
                node_kind=DagNodeKind.VERIFICATION,
                task="Verify fix",
                depends_on=("n1_repro",),
            ),
        ),
    )
    first_run = json.loads(
        await service.run_dag_campaign(
            req=req,
            job_store=store,
            run_agy_fn=_clean_exec,
            execute_fn=_clean_exec,
        )
    )
    assert first_run["ok"] is True
    assert first_run["run_state"] == str(DagRunState.SETTLED)
    assert first_run["settled_count"] == 2
    assert call_count == 2

    # Resume from first_run["dag_id"]: both nodes should be resumed without re-executing!
    resumed_req = DagUlwRequestModel(
        campaign_goal="Reproduce and settle",
        workspace_dir=str(ws),
        plan_only=False,
        resume_dag_id=first_run["dag_id"],
        spawn_plan=DagSpawnPlanInputModel(plan_summary="Run repro + fix"),
        nodes=req.nodes,
    )
    second_run = json.loads(
        await service.run_dag_campaign(
            req=resumed_req,
            job_store=store,
            run_agy_fn=_clean_exec,
            execute_fn=_clean_exec,
        )
    )
    assert second_run["ok"] is True
    assert set(second_run["resumed_nodes"]) == {"n1_repro", "n2_verify"}
    assert call_count == 2


def test_oh_my_hermes_style_tui_session_and_cli_commands(tmp_path: Path) -> None:
    """Verify `HeterarchyTuiSession` slash commands and `hcodex` TUI entrypoints."""
    session = HeterarchyTuiSession(
        workspace_dir=str(tmp_path),
        state_root=tmp_path / "state",
        use_ansi=False,
    )
    # Default active skill on startup is codex-heterarchy and default model is gemini-4-argon-high (argon4)
    assert session.skill_registry.active_skills == ["codex-heterarchy"]
    assert session.active_model == "gemini-4-argon-high"

    help_out, should_quit = session.handle_command(raw_line="/help")
    assert not should_quit
    assert "/dag-ulw <goal>" in help_out
    assert "/dag-mode" in help_out
    assert "/dag-mode exit" in help_out
    assert "/models" in help_out

    models_out, _ = session.handle_command(raw_line="/models")
    assert "Tri-Polar Lane & Model Chain Matrix" in models_out
    assert "claude-sonnet-5-5-medium" in models_out
    for codex_6_m in (
        "gpt-6-terra",
        "gpt-6-luna",
        "gpt-6-sol",
        "gpt-6-astra",
        "gpt-6.1-terra",
        "gpt-6.1-sol",
    ):
        assert codex_6_m in models_out
    assert "gpt-5.4" not in models_out
    assert "gpt-5.3" not in models_out

    dag_exec_out, _ = session.handle_command(raw_line="/dag-ulw Refactor auth boundary")
    assert "Topological Box-Drawing Dependency Graph" in dag_exec_out
    assert "┌──────────────────────────┴──────────────────────────┐" in dag_exec_out
    assert "└──────────────────────────┬──────────────────────────┘" in dag_exec_out
    assert "▸ [n1_topology_spec] Prompt:" in dag_exec_out
    assert "▸ Work  : ● SETTLED" in dag_exec_out
    assert "Wave & Agent Prompting Trace" in dag_exec_out
    assert "Prompting (Task) ➔ Agent Synthesis" in dag_exec_out
    assert "↳ Prompt:" in dag_exec_out
    assert "➔ Work:" in dag_exec_out
    assert "n1_topology_spec" in dag_exec_out
    assert "Settled=4/4" in dag_exec_out
    assert "OMH / DAG · Completed · Done 4/4" in dag_exec_out
    assert "✓ Accepted" in dag_exec_out
    assert "accepted 0.8s · ↓4.2k" in dag_exec_out
    assert "[3] Live Code Work & Workspace AST/Source Stream" in dag_exec_out
    assert "delegate_task(" in dag_exec_out
    assert "omh_dag(settle)" in dag_exec_out
    assert "소요 시간:" in dag_exec_out
    assert "토큰 사용량: 18,400" in dag_exec_out
    assert "사용 모델:" in dag_exec_out
    assert "done 4/4" in session.suggestion_engine.omh_status_badge
    assert len(dag_exec_out.splitlines()) <= 29

    omh_board_out, _ = session.handle_command(raw_line="/omh")
    assert "OMH / DAG LIVE LANE TREE & EVIDENCE BOARD" in omh_board_out
    assert "Done. Every lane passed its checks" in omh_board_out
    assert "▾ layer 1 · 1 node · 1 done" in omh_board_out
    assert "▾ layer 2 · 2 nodes · 2 done (Same frontier)" in omh_board_out
    assert "▾ layer 3 · 1 node · 1 done" in omh_board_out
    assert "소요 시간:" in omh_board_out
    back_from_omh, _ = session.handle_command(raw_line="/back")
    assert back_from_omh == dag_exec_out

    prompts_all_out, _ = session.handle_command(raw_line="/promt")
    assert "5-Section Agent Prompting & Synthesis Trace" in prompts_all_out
    assert "1. TASK & SKILLS" in prompts_all_out
    assert "4. AGENT OUTPUT" in prompts_all_out
    assert "/back" in prompts_all_out

    # Returning to previous page (/back) restores the exact DAG workbench view
    back_to_dag_out, _ = session.handle_command(raw_line="/back")
    assert back_to_dag_out == dag_exec_out

    prompts_claude_out, _ = session.handle_command(raw_line="/prompts claude")
    assert "n2_interface_craft" in prompts_claude_out
    assert "n1_topology_spec" in prompts_claude_out and "Wave 0" not in prompts_claude_out
    back_via_prompts_arg, _ = session.handle_command(raw_line="/prompts back")
    assert back_via_prompts_arg == dag_exec_out

    dag_plan_only_out, _ = session.handle_command(
        raw_line="/dag-ulw --plan-only Plan auth boundary"
    )
    assert "PLANNED" in dag_plan_only_out
    assert "Settled=0/4" in dag_plan_only_out

    dag_run_out, _ = session.handle_command(raw_line="/dag-run Settle auth boundary")
    assert "SETTLED" in dag_run_out
    assert "Two-Stage Settlement Receipts" in dag_run_out

    # Verify /effect and /effort DAG agent configuration & visual effect themes
    effect_mat_out, _ = session.handle_command(raw_line="/effect")
    assert "DAG-ULW++ Agent Effect & Reasoning Effort Matrix" in effect_mat_out
    assert "neon-pulse" in effect_mat_out

    eff_codex_out, _ = session.handle_command(raw_line="/effect codex low")
    assert "◔ low" in eff_codex_out
    eff_terra6_out, _ = session.handle_command(raw_line="/effect codex 테라(6) high")
    assert "gpt-6-terra" in eff_terra6_out
    assert "◕ high" in eff_terra6_out
    eff_luna6_out, _ = session.handle_command(raw_line="/effect codex 6 luna xhigh")
    assert "gpt-6-luna" in eff_luna6_out
    assert "● xhigh" in eff_luna6_out
    eff_sol6_out, _ = session.handle_command(raw_line="/effect codex 6 sol medium")
    assert "gpt-6-sol" in eff_sol6_out
    assert "◑ medium" in eff_sol6_out
    eff_astra_out, _ = session.handle_command(raw_line="/effect codex 6 astra xhigh")
    assert "gpt-6-astra" in eff_astra_out
    assert "● xhigh" in eff_astra_out
    eff_terra61_out, _ = session.handle_command(raw_line="/effect codex 6-1 terra high")
    assert "gpt-6.1-terra" in eff_terra61_out
    assert "◕ high" in eff_terra61_out
    eff_sol61_out, _ = session.handle_command(raw_line="/effect codex 솔(6-1) high")
    assert "gpt-6.1-sol" in eff_sol61_out
    assert "◕ high" in eff_sol61_out
    eff_dep54_out, _ = session.handle_command(raw_line="/effect codex 5.4 high")
    assert "지원하지 않는" in eff_dep54_out
    eff_claude_out, _ = session.handle_command(raw_line="/effect claude opus")
    assert "claude-opus-5-5-medium" in eff_claude_out
    eff_style_out, _ = session.handle_command(raw_line="/effect style cyber-matrix")
    assert "cyber-matrix" in eff_style_out

    custom_dag_out, _ = session.handle_command(raw_line="/dag-ulw Verify custom agent effects")
    assert "effect:cyber-matrix" in custom_dag_out
    assert "opus@◑ medium" in custom_dag_out
    assert "gpt-6.1-sol@◕ high" in custom_dag_out
    assert "gpt-6.1-sol @ ◕ high" in custom_dag_out

    use_astra_out, _ = session.handle_command(raw_line="/use gpt-6-astra")
    assert "gpt-6-astra" in use_astra_out
    assert session.active_model == "gpt-6-astra"
    use_argon4_out, _ = session.handle_command(raw_line="/use argon4")
    assert "gemini-4-argon-high" in use_argon4_out
    assert session.active_model == "gemini-4-argon-high"

    eff_reset_out, _ = session.handle_command(raw_line="/effect reset")
    assert "EFFECT RESET" in eff_reset_out
    assert "neon-pulse" in eff_reset_out

    # Verify /dag-mode activation, auto-DAG prompt execution, and /dag-mode exit
    assert not session.dag_mode
    dag_mode_on_out, _ = session.handle_command(raw_line="/dag-mode")
    assert session.dag_mode
    assert session.prompt_reader.dag_mode
    assert "D A G - H E T E R A R C H Y" in dag_mode_on_out

    auto_dag_out, _ = session.handle_command(raw_line="Verify 3-tier inner_dto schema")
    assert "D A G - H E T E R A R C H Y · AUTO-CAMPAIGN" in auto_dag_out
    assert "SETTLED" in auto_dag_out

    dag_mode_off_out, _ = session.handle_command(raw_line="/dag-mode exit")
    assert not session.dag_mode
    assert not session.prompt_reader.dag_mode
    assert "DAG-MODE EXIT" in dag_mode_off_out

    jobs_out, _ = session.handle_command(raw_line="/jobs")
    assert "Recent Jobs" in jobs_out

    theme_out, _ = session.handle_command(raw_line="/theme")
    assert "tui-theme-boot" in theme_out

    skills_out, _ = session.handle_command(raw_line="/skills")
    assert "Registered Skills Catalog" in skills_out
    assert "$codex-heterarchy" in skills_out
    assert "$codegraph-operator" in skills_out
    assert "$codebase-refactor-guardian" in skills_out

    skill_clear_out, _ = session.handle_command(raw_line="/skill clear")
    assert "해제" in skill_clear_out
    assert session.skill_registry.active_skills == []

    skill_def_out, _ = session.handle_command(raw_line="/skill default")
    assert "codex-heterarchy" in skill_def_out
    assert session.skill_registry.active_skills == ["codex-heterarchy"]

    skill_core_out, _ = session.handle_command(raw_line="/skill core")
    assert "codex-heterarchy" in skill_core_out
    assert "codex-heterarchy" in session.skill_registry.active_skills

    inline_matched, remaining_text = session.skill_registry.extract_inline_skill_mentions(
        raw_line="$codex-heterarchy /codegraph-operator 안녕하세요"
    )
    assert inline_matched == ["codex-heterarchy", "codegraph-operator"]
    assert remaining_text == "안녕하세요"

    # Verify live suggestion engine ($ prefix, / prefix, Korean keyword, and Codex prompt bar)
    sug_dollar, rep_start, _ = session.suggestion_engine.suggest(buffer="$cod", cursor_pos=4)
    assert rep_start == 0
    assert any(it.display_label == "$codex-heterarchy" for it in sug_dollar)
    completed_buf, completed_cur = session.suggestion_engine.apply_completion(
        buffer="$cod", cursor_pos=4, item=sug_dollar[0], replace_start=rep_start
    )
    assert completed_buf.startswith("$")
    assert completed_cur == len(completed_buf)

    bar_ansi = session.suggestion_engine.render_codex_prompt_bar(
        buffer="$heterarchy-alexandria:connection-hub ",
        dag_mode=True,
        use_ansi=True,
        term_cols=180,
    )
    assert len(bar_ansi) == 5
    assert "\033[48;5;236m" in bar_ansi[0]
    assert "\033[48;5;236m" in bar_ansi[1]
    assert "\033[48;5;236;1;38;5;117m$heterarchy-alexandria:connection-hub" in bar_ansi[1]
    assert "\033[48;5;236m" in bar_ansi[2]
    assert "D A G - H E T E R A R C H Y" in bar_ansi[4]
    # Ensure no line in the prompt bar ever reaches term_cols (preventing terminal auto-wrap duplication)
    from app.core.skill_registry import DisplayWidthFormatter

    for line_s in bar_ansi:
        assert DisplayWidthFormatter.visible_width(ansi_text=line_s) < 180

    sug_ko, _, _ = session.suggestion_engine.suggest(buffer="오퍼스", cursor_pos=3)
    assert any("opus" in it.display_label for it in sug_ko)

    sug_use_a, _, _ = session.suggestion_engine.suggest(buffer="/use a", cursor_pos=6)
    assert sug_use_a[0].insert_text == "/use argon"
    sug_use_o, _, _ = session.suggestion_engine.suggest(buffer="/use o", cursor_pos=6)
    assert sug_use_o[0].insert_text == "/use opus"
    sug_dag_e, _, _ = session.suggestion_engine.suggest(buffer="/dag-mode e", cursor_pos=11)
    assert sug_dag_e[0].insert_text == "/dag-mode exit"
    sug_slash, _, _ = session.suggestion_engine.suggest(buffer="/", cursor_pos=1)
    assert all(it.display_label.startswith("/") for it in sug_slash)

    dropdown_lines = session.suggestion_engine.render_dropdown_box(
        items=sug_dollar, selected_index=0, use_ansi=False
    )
    assert len(dropdown_lines) >= 3
    assert "추천 자동완성" in dropdown_lines[0]

    # OpenAI Codex CLI benchmarked slash commands: /reasoning, /diff, /review, /compact, /clear, /permissions, /resume, @file
    reason_out, _ = session.handle_command(raw_line="/reasoning codex 6 astra xhigh")
    assert "AGENT EFFECT UPDATED" in reason_out

    diff_clean_out, _ = session.handle_command(raw_line="/diff")
    assert "WORKSPACE MANIFEST DIFF" in diff_clean_out
    assert "0 mutated files" in diff_clean_out

    (tmp_path / "bench_target.py").write_text(
        "def hello() -> int:\n    return 42\n", encoding="utf-8"
    )
    diff_mut_out, _ = session.handle_command(raw_line="/diff")
    assert "+ [ADDED]    bench_target.py" in diff_mut_out

    review_out, _ = session.handle_command(raw_line="/review Concurrency & WAW gates")
    assert "WORKING-TREE CODE REVIEW" in review_out
    assert "Concurrency & WAW gates" in review_out

    at_file_out, _ = session.handle_command(raw_line="@bench_target.py 리팩토링 점검")
    assert "LIVE PROMPT & CODE WORKBENCH" in at_file_out
    assert "bench_target.py" in at_file_out
    assert "[L1] def hello() -> int:" in at_file_out
    assert session.suggestion_engine.tokens_in > 0
    assert session.suggestion_engine.tokens_out > 0

    compact_out, _ = session.handle_command(raw_line="/compact")
    assert "CONTEXT COMPACTED" in compact_out

    perm_out, _ = session.handle_command(raw_line="/permissions read-only")
    assert "read-only" in perm_out
    assert session.permission_mode == "read-only"

    resume_out, _ = session.handle_command(raw_line="/resume")
    assert "DAG-ULW++ WORKBENCH" in resume_out

    clear_out, _ = session.handle_command(raw_line="/clear")
    assert "Heterarchy Codex" in clear_out
    assert session.suggestion_engine.tokens_in == 0

    quit_out, should_quit = session.handle_command(raw_line="/quit")
    assert should_quit is True
    assert "Goodbye" in quit_out

    inputs = iter(["/status", "/quit"])
    rc = session.run_interactive_loop(input_fn=lambda _prompt: next(inputs))
    assert rc == 0

    ctl = HeterarchyCliController()
    assert ctl.dispatch(argv=["models"]) == 0
    assert ctl.dispatch(argv=["skills"]) == 0
    assert ctl.dispatch(argv=["dag-ulw", "Test CLI DAG", "--workspace-dir", str(tmp_path)]) == 0

    # Verify Codex Red-Team regressions: sibling-prefix symlink guard, bounded excerpt read, and unverified board state
    from app.core.cli_tui import _expand_at_file_mentions
    from app.evidence.live_code_inspector import LiveCodeWorkReport, render_omh_live_board

    ws_root = tmp_path / "workspace"
    ws_sibling = tmp_path / "workspace-private"
    ws_root.mkdir(exist_ok=True)
    ws_sibling.mkdir(exist_ok=True)
    secret_file = ws_sibling / "secret.txt"
    secret_file.write_text("PRIVATE_SENTINEL", encoding="utf-8")
    (ws_root / "leak.txt").symlink_to(secret_file)
    mention_res = _expand_at_file_mentions(prompt_text="Inspect @leak.txt", workspace_dir=str(ws_root))
    assert "PRIVATE_SENTINEL" not in mention_res

    large_file = ws_root / "large.txt"
    large_file.write_bytes(b"x" * 1000)
    bounded_res = _expand_at_file_mentions(
        prompt_text="Inspect @large.txt", workspace_dir=str(ws_root), max_bytes_per_file=16
    )
    assert "x" * 16 in bounded_res
    assert "x" * 17 not in bounded_res

    empty_rep = LiveCodeWorkReport(
        total_py_files=0,
        slices=(),
        n1_argon_summary="unverified",
        n2_claude_summary="unverified",
        n3_codex_summary="unverified",
        n4_verify_summary="no files inspected",
        stage_code_boxes=(("s0",), ("s1",), ("s2",), ("s3",)),
    )
    unverified_board = render_omh_live_board(
        goal="Inspect workspace",
        argon_model="argon",
        claude_model="claude",
        codex_model="codex",
        report=empty_rep,
        settled_count=0,
        use_ansi=False,
    )
    assert "9/9 mechanical gates PASS" not in unverified_board
    assert "Every lane passed its checks and settled" not in unverified_board
    assert "· accepted " not in unverified_board
    assert "pending/unverified" in unverified_board


def test_three_tier_schema_and_inner_dto_contracts() -> None:
    """Verify the 3-tier schema architecture (`schemas/` -> `inner_dto/` -> `dto_dict_types/`)."""
    repo_root = Path(__file__).resolve().parent.parent
    schemas_dir = repo_root / "app" / "schemas"
    inner_dto_dir = schemas_dir / "inner_dto"
    dict_types_dir = inner_dto_dir / "dto_dict_types"

    top_files = {p.name for p in schemas_dir.glob("*.py")}
    assert top_files == {"__init__.py", "boundary_schemas.py", "dag_input_schemas.py"}
    assert inner_dto_dir.is_dir()
    assert dict_types_dir.is_dir()

    # Ensure no TypedDict is defined outside app/schemas/inner_dto/dto_dict_types/
    # and no *Dict(...) is called outside a to_dict method across app/.
    for py_path in (repo_root / "app").rglob("*.py"):
        tree = ast.parse(py_path.read_text(encoding="utf-8"))
        in_dict_types = py_path.parent == dict_types_dir
        for node in ast.walk(tree):
            if isinstance(node, ast.ClassDef):
                base_names = [b.id for b in node.bases if isinstance(b, ast.Name)]
                if "TypedDict" in base_names:
                    assert in_dict_types, (
                        f"TypedDict {node.name} found outside dto_dict_types in {py_path}"
                    )
                elif node.name != "__init__":
                    for item in node.body:
                        if isinstance(item, ast.FunctionDef) and item.name != "to_dict":
                            for sub in ast.walk(item):
                                if isinstance(sub, ast.Call) and isinstance(sub.func, ast.Name):
                                    assert not (
                                        sub.func.id.endswith("Dict")
                                        and sub.func.id not in ("ConfigDict", "OrderedDict")
                                    ), (
                                        f"Direct TypedDict call {sub.func.id}(...) outside to_dict in {py_path}"
                                    )


def test_sql_files_and_sql_injection_defenses(tmp_path: Path) -> None:
    """Verify `{feature}.sql` query files and multi-layer SQL injection defenses."""
    import sqlite3

    import pytest

    from app.repositories import (
        SQL_LOADER,
        CodegraphSqlQueryId,
        HandoffSqlQueryId,
        HandoffStore,
        JobSqlQueryId,
        JobStore,
        SqlFeatureFile,
        SqlQueryLoader,
        build_readonly_sqlite_uri,
    )

    repo_root = Path(__file__).resolve().parent.parent
    sql_dir = repo_root / "app" / "repositories" / "sql"
    assert {p.name for p in sql_dir.glob("*.sql")} == {
        str(SqlFeatureFile.JOB_STORE),
        str(SqlFeatureFile.HANDOFF_STORE),
        str(SqlFeatureFile.CODEGRAPH_SLICER),
    }

    job_queries = SQL_LOADER.load_feature_queries(feature_file=SqlFeatureFile.JOB_STORE)
    assert set(job_queries.keys()) == {str(q) for q in JobSqlQueryId}

    handoff_queries = SQL_LOADER.load_feature_queries(feature_file=SqlFeatureFile.HANDOFF_STORE)
    assert set(handoff_queries.keys()) == {str(q) for q in HandoffSqlQueryId}

    cg_queries = SQL_LOADER.load_feature_queries(feature_file=SqlFeatureFile.CODEGRAPH_SLICER)
    assert set(cg_queries.keys()) == {str(q) for q in CodegraphSqlQueryId}

    # Verify LIKE wildcard escaping and SQLite URI parameter injection defense
    assert SQL_LOADER.escape_like_suffix(symbol_name="my_sym%test\\x") == r"%.my\_sym\%test\\x"
    with pytest.raises(ValueError, match="NUL byte"):
        SQL_LOADER.escape_like_suffix(symbol_name="bad\x00sym")

    evil_db = tmp_path / "evil?mode=rwc#frag.db"
    evil_db.write_bytes(b"")
    ro_uri = build_readonly_sqlite_uri(db_path=evil_db)
    assert "%3Fmode%3Drwc%23frag.db?mode=ro" in ro_uri

    # Verify stacked-statement and forbidden meta-command rejection in custom .sql files
    bad_sql_dir = tmp_path / "bad_sql"
    bad_sql_dir.mkdir()
    (bad_sql_dir / "job_store.sql").write_text(
        "-- name: select_job_by_id\nSELECT * FROM jobs WHERE job_id = ?; DROP TABLE jobs\n",
        encoding="utf-8",
    )
    bad_loader = SqlQueryLoader(sql_dir=bad_sql_dir)
    with pytest.raises(ValueError, match="Stacked SQL statements"):
        bad_loader.load_feature_queries(feature_file=SqlFeatureFile.JOB_STORE)

    (bad_sql_dir / "handoff_store.sql").write_text(
        "-- name: select_handoff_by_id\nATTACH DATABASE 'evil.db' AS evil\n",
        encoding="utf-8",
    )
    with pytest.raises(ValueError, match="Forbidden SQL meta-command"):
        bad_loader.load_feature_queries(feature_file=SqlFeatureFile.HANDOFF_STORE)

    # Verify parameter arity, NUL byte, and scalar type guards on execute_safe
    conn = sqlite3.connect(":memory:")
    SQL_LOADER.execute_script(
        conn=conn,
        feature_file=SqlFeatureFile.JOB_STORE,
        query_id=JobSqlQueryId.INIT_SCHEMA,
    )
    with pytest.raises(ValueError, match="parameter arity mismatch"):
        SQL_LOADER.execute_safe(
            conn=conn,
            feature_file=SqlFeatureFile.JOB_STORE,
            query_id=JobSqlQueryId.SELECT_JOB_BY_ID,
            params=("id1", "id2"),
        )
    with pytest.raises(ValueError, match="NUL byte injection blocked"):
        SQL_LOADER.execute_safe(
            conn=conn,
            feature_file=SqlFeatureFile.JOB_STORE,
            query_id=JobSqlQueryId.SELECT_JOB_BY_ID,
            params=("cdx-\x00evil",),
        )
    conn.close()

    # Verify live SQL injection payloads against JobStore and HandoffStore
    jstore = JobStore(state_dir=tmp_path / "jobs_sqli")
    sqli_payload = "'; DROP TABLE jobs; --"
    rec = jstore.create_job(
        task_type="adversarial_audit",
        workspace_dir=str(tmp_path),
        idempotency_key=sqli_payload,
    )
    assert jstore.find_by_idempotency_key(idempotency_key=sqli_payload) == rec
    assert jstore.get_job(job_id="' OR '1'='1") is None

    hstore = HandoffStore(state_dir=tmp_path / "handoffs_sqli")
    ticket = hstore.create_ticket(
        sender="argon",
        recipient="codex",
        title="'; DELETE FROM handoffs; --",
        spec_or_payload="' OR 1=1 --",
        workspace_dir=str(tmp_path),
    )
    assert hstore.get_ticket(ticket_id=ticket.ticket_id) is not None
    assert hstore.get_ticket(ticket_id="' OR '1'='1") is None
    listed = hstore.list_tickets(recipient="codex' OR '1'='1", status="pending")
    assert listed == []
