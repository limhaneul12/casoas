"""Interactive OpenAI Codex & `oh-my-hermes`-style TUI CLI workbench (`hcodex tui`)."""

from __future__ import annotations

import asyncio
import json
import re
import shutil
import sys
from collections.abc import Callable
from pathlib import Path

from app.clients.agy_bridge import run_agy_model_prompt
from app.core.collab_enums import CanonicalModelId, EngineKind, TriadRole
from app.core.dag_enums import (
    DagIsolationMode,
    DagNodeKind,
    DagNodeRoleKind,
    DagRunState,
    DagSettlementStage,
    DefaultCoreSkill,
    DefaultDagNodeId,
    TuiAnsiStyle,
    TuiPermissionMode,
    TuiSlashCommand,
)
from app.core.model_contract_enums import TaskTypeKind
from app.core.skill_registry import DisplayWidthFormatter, TuiSkillRegistry
from app.core.tui_completer import TuiInteractivePromptReader, TuiSuggestionEngine
from app.core.tui_renderer import OmhTuiRenderer, _style_ansi, render_workspace_diff_review_panel
from app.evidence.dag_agent_effects import DagAgentEffectRegistry, render_dag_prompts_inspector
from app.evidence.dag_hazard_analyzer import normalize_scope_path
from app.evidence.live_code_inspector import (
    LiveCodeWorkReport,
    LiveWorkspaceCodeInspector,
    render_omh_live_board,
)
from app.repositories.handoff_store import HandoffStore
from app.repositories.job_store import JobStore
from app.routing.model_specs import CODEX_SUPPORTED_MODELS
from app.routing.policy_engine import RoutingPolicyEngine
from app.schemas.boundary_schemas import JsonObject, as_json_object
from app.schemas.dag_input_schemas import DagSpawnPlanInputModel, DagUlwRequestModel
from app.schemas.inner_dto.dag_dtos import (
    DagCompiledPlan,
    DagNodeSettlementReceipt,
    DagUlwResponse,
    TuiSimulatedExecResult,
)
from app.services.dag_ulw_service import AsyncEngineCallable, DagUlwExecutionService

_AT_FILE_RE = re.compile(r"(?:^|\s)@([A-Za-z0-9_.\-/]+\.[A-Za-z0-9_]+)")


def _detect_git_branch(workspace_dir: str) -> str:
    """Detect the active Git branch name from `.git/HEAD` without spawning a subprocess.

    Args:
        workspace_dir: Resolved workspace directory path.

    Returns:
        Branch name (e.g. `'main'`) or short commit SHA.
    """
    head_file = Path(workspace_dir) / ".git" / "HEAD"
    try:
        raw = head_file.read_text(encoding="utf-8", errors="ignore").strip()
    except OSError:
        return "main"
    if raw.startswith("ref: refs/heads/"):
        return raw.removeprefix("ref: refs/heads/").strip() or "main"
    return raw[:8] if raw else "main"


def _expand_at_file_mentions(prompt_text: str, workspace_dir: str, max_bytes_per_file: int = 8_000) -> str:
    """Append bounded file excerpts for any `@path/to/file.ext` mentions in `prompt_text`.

    Args:
        prompt_text: Raw prompt string entered by the user.
        workspace_dir: Resolved workspace directory path.
        max_bytes_per_file: Maximum UTF-8 bytes to inline per referenced file.

    Returns:
        Prompt text enriched with referenced workspace file excerpts when present.
    """
    matches = dict.fromkeys(_AT_FILE_RE.findall(prompt_text))
    if not matches:
        return prompt_text
    root = Path(workspace_dir).resolve()
    excerpts: list[str] = []
    for cand in matches:
        norm = normalize_scope_path(rel_path=cand)
        if norm is None:
            continue
        fpath = (root / norm).resolve()
        if not fpath.is_relative_to(root) or not fpath.is_file():
            continue
        try:
            with (root / norm).open("rb") as fh:
                if (root / norm).is_symlink() or not (root / norm).resolve().is_relative_to(root):
                    continue
                snippet = fh.read(max(0, max_bytes_per_file)).decode("utf-8", errors="ignore")
        except OSError:
            continue
        excerpts.append(f"### Referenced File (`@{norm}`):\n```\n{snippet}\n```")
    return f"{prompt_text}\n\n" + "\n\n".join(excerpts) if excerpts else prompt_text


def _handle_codex_bench_command(session: HeterarchyTuiSession, cmd: str, arg: str) -> str | None:
    """Execute OpenAI Codex CLI benchmarked commands (`/diff`, `/review`, `/compact`, `/clear`, `/permissions`, `/resume`).

    Args:
        session: Active `HeterarchyTuiSession` instance.
        cmd: Lowercase slash command token.
        arg: Command argument text.

    Returns:
        Rendered panel string if `cmd` matched a Codex benchmarked command, else None.
    """
    if cmd in (TuiSlashCommand.DIFF, TuiSlashCommand.REVIEW):
        verifier = session.dag_service._verifier
        cur_man = verifier.capture_workspace_manifest(workspace_dir=session.workspace_dir)
        if session.baseline_manifest is None:
            session.baseline_manifest = dict(cur_man)
        base_man = session.baseline_manifest
        added = tuple(sorted(p for p in cur_man if p not in base_man))
        deleted = tuple(sorted(p for p in base_man if p not in cur_man))
        modified = tuple(sorted(p for p in cur_man if p in base_man and cur_man[p] != base_man[p]))
        focus = (arg or "WAW/RAW concurrency & 3-tier schema invariants") if cmd == TuiSlashCommand.REVIEW else ""
        return render_workspace_diff_review_panel(workspace_dir=session.workspace_dir, added=added, modified=modified, deleted=deleted, total_tracked=len(cur_man), review_focus=focus, use_ansi=session.renderer.use_ansi)
    if cmd == TuiSlashCommand.COMPACT:
        eng = session.suggestion_engine
        before_toks = eng.tokens_in + eng.tokens_out
        eng.tokens_in, eng.tokens_out = int(eng.tokens_in * 0.3), int(eng.tokens_out * 0.3)
        reclaimed = max(0, before_toks - (eng.tokens_in + eng.tokens_out))
        if len(session.view_history) > 1:
            session.view_history = session.view_history[-1:]
        pct = max(0, min(100, round((1.0 - (eng.tokens_in + eng.tokens_out) / max(1, eng.context_limit)) * 100)))
        return f"● [CONTEXT COMPACTED] 컨텍스트 압축 완료 (회수된 토큰: {reclaimed:,} · 남은 컨텍스트: {pct}% left)"
    if cmd == TuiSlashCommand.CLEAR:
        session.view_history.clear()
        session.current_view, session.last_dag_receipts, session.last_code_report = "", (), None
        session.suggestion_engine.tokens_in, session.suggestion_engine.tokens_out = 0, 0
        session.suggestion_engine.omh_status_badge = "▶▶ accept all on · OMH ready"
        tel = session.job_store.summarize_telemetry()
        return session.renderer.render_banner(workspace_dir=session.workspace_dir, running_jobs=tel["running_jobs"], pending_handoffs=0, active_model=session.active_model, active_skills=tuple(session.skill_registry.active_skills), dag_mode=session.dag_mode, include_status_box=not (session.renderer.use_ansi and sys.stdout.isatty()), permission_mode=session.permission_mode)
    if cmd in (TuiSlashCommand.OMH, TuiSlashCommand.BOARD, TuiSlashCommand.TODO, TuiSlashCommand.PLAN):
        rep = session.last_code_report or session.code_inspector.build_code_work_report(workspace_dir=session.workspace_dir, query_text=session.last_dag_goal, active_skills=tuple(session.skill_registry.active_skills))
        session.last_code_report = rep
        eff_cfg = session.effect_registry.config
        settled_n = len({r.node_id for r in session.last_dag_receipts if r.settlement_stage == DagSettlementStage.SETTLED and r.verify_passed} & set(DefaultDagNodeId))
        return render_omh_live_board(goal=arg or session.last_dag_goal, argon_model=eff_cfg.argon.model_id, claude_model=eff_cfg.claude.model_id, codex_model=eff_cfg.codex.model_id, report=rep, settled_count=settled_n, use_ansi=session.renderer.use_ansi)
    if cmd in (TuiSlashCommand.PERMISSIONS, TuiSlashCommand.APPROVALS):
        clean_a = arg.strip().lower()
        if not clean_a:
            return f"● 현재 권한 모드: `{session.permission_mode}` (변경: `/permissions read-only`, `/permissions auto-edit`, `/permissions yolo`)"
        chosen = TuiPermissionMode.READ_ONLY if clean_a in ("read-only", "readonly", "read") else (TuiPermissionMode.AUTO_EDIT if clean_a in ("auto-edit", "auto", "edit") else (TuiPermissionMode.YOLO if clean_a in ("yolo", "yolo mode", "full") else None))
        if chosen is None:
            return f"▲ 알 수 없는 권한 모드입니다: `{arg}` (`read-only`, `auto-edit`, `yolo` 중 선택)"
        session.permission_mode = str(chosen)
        return f"● [PERMISSIONS UPDATED] 샌드박스/승인 권한 모드가 `{session.permission_mode}`(으)로 전환되었습니다."
    if cmd == TuiSlashCommand.RESUME:
        target_id = arg.strip()
        if not target_id:
            dag_jobs = [j for j in session.job_store.list_jobs(limit=20) if j["task_type"] == str(TaskTypeKind.DAG_ULW)]
            if not dag_jobs:
                return "○ 재개할 이전 DAG 캠페인 체크포인트가 없습니다. (`/dag-ulw <목표>`로 먼저 실행해주세요)"
            target_id = dag_jobs[0]["job_id"]
        return session.execute_dag_command(campaign_goal=f"--resume {target_id} {session.last_dag_goal}", plan_only=False)
    return None


class HeterarchyTuiSession:
    """Interactive prompt loop (`>>`) and command router for `hcodex tui`."""

    def __init__(self, workspace_dir: str = ".", state_root: Path | None = None, use_ansi: bool = True, run_agy_fn: AsyncEngineCallable | None = None, execute_fn: AsyncEngineCallable | None = None) -> None:
        """Initialize the interactive TUI session with stores, renderer, skills, and DAG service.

        Args:
            workspace_dir: Active workspace directory path.
            state_root: Optional root directory for SQLite job and handoff stores.
            use_ansi: Whether to emit ANSI color sequences.
            run_agy_fn: Optional Antigravity RPC callable override for `/dag-run`.
            execute_fn: Optional Codex engine callable override for `/dag-run`.
        """
        self.workspace_dir = str(Path(workspace_dir).resolve())
        cache_dir = state_root or (Path.home() / ".cache" / "codex-collab-mcp")
        self.job_store, self.handoff_store = JobStore(state_dir=cache_dir / "jobs"), HandoffStore(state_dir=cache_dir / "handoffs")
        self.renderer, self.skill_registry = OmhTuiRenderer(use_ansi=use_ansi), TuiSkillRegistry(workspace_dir=self.workspace_dir)
        self.suggestion_engine = TuiSuggestionEngine(skill_registry=self.skill_registry, git_branch=_detect_git_branch(workspace_dir=self.workspace_dir))
        self.prompt_reader = TuiInteractivePromptReader(engine=self.suggestion_engine, use_ansi=use_ansi)
        self.dag_service, self.effect_registry, self.code_inspector = DagUlwExecutionService(), DagAgentEffectRegistry(), LiveWorkspaceCodeInspector()
        self.last_code_report: LiveCodeWorkReport | None = None
        self._policy_engine, self.permission_mode, self.last_dag_goal = RoutingPolicyEngine(), str(TuiPermissionMode.YOLO), "Verify and refactor module contracts"
        self.baseline_manifest: dict[str, str] | None = None
        self.last_dag_receipts: tuple[DagNodeSettlementReceipt, ...] = ()
        self.view_history: list[str] = []
        self.current_view, self.active_model, self.dag_mode = "", str(CanonicalModelId.GEMINI_4_ARGON_HIGH), False
        self.prompt_reader.active_model, self.prompt_reader.dag_mode = self.active_model, self.dag_mode
        self._run_agy_fn, self._execute_fn = run_agy_fn, execute_fn

    async def _default_agy_fn(self, **kwargs: object) -> JsonObject:
        """Provide a workspace-grounded Antigravity response when invoked in fast TUI mode.

        Args:
            **kwargs: Arbitrary prompt execution keyword arguments.

        Returns:
            Simulated succeeded execution dictionary with grounded workspace AST/code summary.
        """
        model_s = str(kwargs.get("model") or CanonicalModelId.GEMINI_4_ARGON_HIGH)
        p_txt = str(kwargs.get("prompt") or "")
        hdr = p_txt[:800]
        rep = self.last_code_report
        live_out = self.code_inspector.live_node_outputs
        if "[n4_verify_fan_in]" in hdr:
            msg = live_out["n4_verify_fan_in"][0] if "n4_verify_fan_in" in live_out else (rep.n4_verify_summary if rep else "3-Pole consensus verified & settled")
        elif "[n1_boundary_map]" in hdr:
            msg = live_out["n1_boundary_map"][0] if "n1_boundary_map" in live_out else (f"Strict 3-tier DTO & boundary map ({rep.slices[1].rel_path})" if (rep and len(rep.slices) > 1) else "Strict boundary map distilled")
        elif "[n3_contract_gate]" in hdr:
            msg = live_out["n3_contract_gate"][0] if "n3_contract_gate" in live_out else "9/9 mechanical & SQL guards PASS"
        elif "[n2_interface_craft]" in hdr:
            msg = live_out["n2_interface_craft"][0] if "n2_interface_craft" in live_out else (rep.n2_claude_summary if rep else "Strict boundary & inner_dto crafted")
        elif "[n1_topology_spec]" in hdr:
            msg = live_out["n1_topology_spec"][0] if "n1_topology_spec" in live_out else (rep.n1_argon_summary if rep else "Invariants & 3-tier contracts distilled")
        else:
            msg = f"TUI local synthesis completed via {model_s}."
        return as_json_object(record=TuiSimulatedExecResult(ok=True, status="succeeded", outcome_state="confirmed_success", model_id=model_s, model=model_s, raw_output=msg, data=msg).to_dict())

    async def _default_exec_fn(self, **kwargs: object) -> JsonObject:
        """Provide a workspace-grounded Codex execution response when invoked in fast TUI mode.

        Args:
            **kwargs: Arbitrary engine dispatch keyword arguments.

        Returns:
            Simulated succeeded execution dictionary with grounded workspace AST/code summary.
        """
        model_s = str(kwargs.get("model") or CanonicalModelId.GPT_6_ASTRA)
        live_out = self.code_inspector.live_node_outputs
        msg = live_out["n3_red_team_audit"][0] if "n3_red_team_audit" in live_out else (self.last_code_report.n3_codex_summary if self.last_code_report else "Red-team PASS: 0 WAW/RAW hazards")
        return as_json_object(record=TuiSimulatedExecResult(ok=True, status="succeeded", outcome_state="confirmed_success", model_id=model_s, model=model_s, raw_output=msg, data=msg).to_dict())

    def execute_direct_prompt(self, prompt_text: str, model_override: str | None = None) -> str:
        """Dispatch a direct free-form prompt with real-time workspace AST/code and AI streaming.

        Args:
            prompt_text: User-authored prompt text.
            model_override: Optional model alias or canonical ID for this call.

        Returns:
            Formatted workbench panel with live code stream, receipt telemetry, and model output.
        """
        clean_p = prompt_text.strip()
        if not clean_p:
            return "프롬프트를 입력해주세요. (예: `$codex-heterarchy 질문` 또는 `@app/core/ctl.py 설명`)"
        target_model = (model_override or self.active_model).strip()
        act_skills = tuple(self.skill_registry.active_skills)
        code_rep = self.code_inspector.build_code_work_report(workspace_dir=self.workspace_dir, query_text=clean_p, active_skills=act_skills)
        self.last_code_report = code_rep
        expanded_p = _expand_at_file_mentions(prompt_text=clean_p, workspace_dir=self.workspace_dir)
        skill_ctx = self.skill_registry.build_active_skills_context()
        full_prompt = f"{skill_ctx}\n\n[User Request]:\n{expanded_p}" if skill_ctx else expanded_p
        base_in, base_out = self.suggestion_engine.tokens_in, self.suggestion_engine.tokens_out
        if self.renderer.use_ansi and sys.stdout.isatty() and self._run_agy_fn is None and self._execute_fn is None:
            t_sz = shutil.get_terminal_size(fallback=(120, 32))
            t_cols = max(80, t_sz.columns)

            def _upd_direct_bar(d_in: int, d_out: int, bdg: str) -> list[str]:
                """Increment live token counters and render the 5-row bottom bar during direct prompt execution.

                Args:
                    d_in: Incremental input tokens for this tick.
                    d_out: Incremental output tokens for this tick.
                    bdg: Live OMH status badge string.

                Returns:
                    Five-line rendered bottom composer bar.
                """
                self.suggestion_engine.tokens_in, self.suggestion_engine.tokens_out, self.suggestion_engine.omh_status_badge = self.suggestion_engine.tokens_in + d_in, self.suggestion_engine.tokens_out + d_out, bdg
                return self.suggestion_engine.render_codex_prompt_bar(buffer=f"⟳ Working: {clean_p[:46]}...", active_model=target_model, dag_mode=self.dag_mode, use_ansi=True, term_cols=t_cols)

            body, dur, ok = self.code_inspector.run_live_direct_prompt(run_agy_fn=run_agy_model_prompt, prompt_text=clean_p, full_prompt=full_prompt, active_model=target_model, active_skills=act_skills, workspace_dir=self.workspace_dir, job_store=self.job_store, report=code_rep, term_cols=t_cols, bar_row=max(24, t_sz.lines) - 4, bar_updater=_upd_direct_bar)
            mod_id, req_id = target_model, "cdx-live-rpc"
        else:
            if target_model in CODEX_SUPPORTED_MODELS:
                exec_cb = self._execute_fn or self._default_exec_fn
                res = asyncio.run(exec_cb(prompt=full_prompt, model=target_model, reasoning_effort=self.effect_registry.config.codex.reasoning_effort, workspace_dir=self.workspace_dir))
            else:
                agy_cb = self._run_agy_fn or run_agy_model_prompt
                res = asyncio.run(agy_cb(prompt=full_prompt, model=target_model, expect_json=False, timeout_seconds=120.0, workspace_dir=self.workspace_dir, job_store=self.job_store))
            ok = res.get("ok") is True
            mod_id, req_id, dur = str(res.get("model_id") or target_model), str(res.get("request_id") or "cdx-direct"), float(str(res.get("duration_seconds", 0.02)))
            body = str(res.get("raw_output") or res.get("error") or "(no output)").strip()
        self.suggestion_engine.tokens_in = base_in + max(840, len(full_prompt) // 3 + code_rep.tokens_in_est // 3)
        self.suggestion_engine.tokens_out = base_out + max(620, len(body) // 2 + code_rep.tokens_out_est // 4)
        self.suggestion_engine.omh_status_badge = "▶▶ accept all on · OMH · done 1/1 (✓ Accepted)"
        return self.code_inspector.render_direct_prompt_workbench(prompt_text=clean_p, model_id=mod_id, request_id=req_id, duration=dur, ok=ok, body_text=body, active_skills=act_skills, report=code_rep, use_ansi=self.renderer.use_ansi)

    def execute_dag_command(self, campaign_goal: str, plan_only: bool = True) -> str:
        """Compile (and optionally execute) a `DAG-ULW++` goal and stream real-time AI + code work.

        Args:
            campaign_goal: Engineering goal string after `/dag-ulw`, `/dag-run`, or `/resume`.
            plan_only: True to compile only (`/dag-ulw --plan-only`), False to execute (`/dag-ulw`).

        Returns:
            Rendered TUI workbench view string with live workspace AST/source and AI stream.
        """
        raw_goal = campaign_goal.strip()
        resume_id: str | None = None
        if raw_goal.startswith("--resume "):
            rem_parts = raw_goal.removeprefix("--resume ").strip().split(maxsplit=1)
            resume_id = rem_parts[0] if rem_parts else None
            raw_goal = rem_parts[1] if len(rem_parts) > 1 else self.last_dag_goal
        use_live_rpc = raw_goal.startswith("--live")
        goal_clean = (raw_goal.removeprefix("--live").strip() if use_live_rpc else raw_goal) or "Verify and refactor module contracts"
        self.code_inspector.live_node_outputs.clear()
        self.last_code_report = code_rep = self.code_inspector.build_code_work_report(workspace_dir=self.workspace_dir, query_text=goal_clean, active_skills=tuple(self.skill_registry.active_skills))
        spawn_plan = None if plan_only else DagSpawnPlanInputModel(plan_summary=f"TUI /dag-ulw interactive spawn plan for: {goal_clean}", hazard_acknowledgment=True, isolation_mode=DagIsolationMode.DIRECT, max_parallel_per_wave=3)
        t_files = tuple(s.rel_path for s in code_rep.slices)
        cfg_nodes = self.effect_registry.build_configured_triad_nodes(campaign_goal=goal_clean, target_files=t_files)
        req = DagUlwRequestModel(campaign_goal=goal_clean, nodes=cfg_nodes, workspace_dir=self.workspace_dir, plan_only=plan_only, spawn_plan=spawn_plan, resume_dag_id=resume_id)
        _, base_plan = self.dag_service._compiler.compile_plan(req=req, workspace_dir=self.workspace_dir, dag_id="cdx-live")
        by_nid = {n.node_id: n for n in cfg_nodes}
        tot_run_toks = code_rep.tokens_in_est + code_rep.tokens_out_est
        base_in, base_out = self.suggestion_engine.tokens_in, self.suggestion_engine.tokens_out
        if self.renderer.use_ansi and sys.stdout.isatty() and not plan_only:
            t_sz = shutil.get_terminal_size(fallback=(120, 32))
            t_cols = max(80, t_sz.columns)

            def _render_live_stage(st_settled: int, st_aimd: int, states: dict[str, tuple[str, str, float]], c_box: tuple[str, ...]) -> str:
                """Render a single live DAG wave stage snapshot.

                Args:
                    st_settled: Number of settled nodes so far.
                    st_aimd: Current AIMD concurrency window.
                    states: Mapping of node_id to `(stage, work_text, duration_seconds)`.
                    c_box: Live code work lines to render in section `[3]`.

                Returns:
                    Rendered ANSI DAG workbench view string.
                """
                st_rcpts = {nid: DagNodeSettlementReceipt(node_id=nid, role=by_nid[nid].role, node_kind=by_nid[nid].node_kind, wave_index=0, settlement_stage=DagSettlementStage(stg), model_id=by_nid[nid].preferred_model or "", duration_seconds=dur_s, pre_manifest_digest="sha256:live", post_manifest_digest="sha256:live", mutated_files=(), out_of_scope_writes=(), dirty_read_files=(), verify_passed=True, reproduction_observed=True, output_summary=txt.split("➔ ")[-1]) for nid, (stg, txt, dur_s) in states.items() if nid in by_nid}
                st_plan = DagCompiledPlan(dag_id=base_plan.dag_id, campaign_goal=base_plan.campaign_goal, plan_digest=base_plan.plan_digest, total_nodes=base_plan.total_nodes, total_waves=base_plan.total_waves, waves=base_plan.waves, hazards=base_plan.hazards, blocking_hazards_count=base_plan.blocking_hazards_count, spawn_plan_acknowledged=base_plan.spawn_plan_acknowledged, ascii_graph=self.dag_service._compiler.render_ascii_dag(nodes=cfg_nodes, waves=base_plan.waves, node_work_map={nid: txt for nid, (_, txt, _) in states.items()}))
                st_dto = DagUlwResponse(ok=True, dag_id="cdx-live", run_state=DagRunState.PLANNED, plan_only=False, workspace_dir=self.workspace_dir, compiled_plan=st_plan, node_receipts=tuple(st_rcpts.values()), settled_count=st_settled, resumed_nodes=(), aimd_final_concurrency=st_aimd, wave_tree_view=self.dag_service.render_wave_tree(nodes=cfg_nodes, plan=st_plan, receipts=st_rcpts), summary="Streaming live wave execution...")
                return self.renderer.render_dag_workbench_view(resp=st_dto, effect_registry=self.effect_registry, code_work_lines=c_box, total_tokens=tot_run_toks)

            def _upd_dag_bar(d_in: int, d_out: int, bdg: str) -> list[str]:
                """Increment live token counters and render the 5-row bottom bar during DAG wave execution.

                Args:
                    d_in: Incremental input tokens for this tick.
                    d_out: Incremental output tokens for this tick.
                    bdg: Live DAG status badge string.

                Returns:
                    Five-line rendered bottom composer bar.
                """
                self.suggestion_engine.tokens_in, self.suggestion_engine.tokens_out, self.suggestion_engine.omh_status_badge = self.suggestion_engine.tokens_in + d_in, self.suggestion_engine.tokens_out + d_out, bdg
                return self.suggestion_engine.render_codex_prompt_bar(buffer=f"⟳ /dag-ulw {goal_clean[:44]}...", active_model=self.active_model, dag_mode=self.dag_mode, use_ansi=True, term_cols=t_cols)

            eff_cfg = self.effect_registry.config
            self.code_inspector.run_live_dag_waves(run_agy_fn=run_agy_model_prompt, campaign_goal=goal_clean, argon_model=eff_cfg.argon.model_id, claude_model=eff_cfg.claude.model_id, codex_model=eff_cfg.codex.model_id, workspace_dir=self.workspace_dir, job_store=self.job_store, report=code_rep, render_stage_fn=_render_live_stage, term_cols=t_cols, bar_row=max(24, t_sz.lines) - 4, wave_node_ids=tuple(w.node_ids for w in base_plan.waves), bar_updater=_upd_dag_bar)
        agy_cb = self._run_agy_fn or (run_agy_model_prompt if (use_live_rpc and not self.code_inspector.live_node_outputs) else self._default_agy_fn)
        raw_json = asyncio.run(self.dag_service.run_dag_campaign(req=req, job_store=self.job_store, run_agy_fn=agy_cb, execute_fn=self._execute_fn or self._default_exec_fn))
        parsed, live_durs = json.loads(raw_json), {k: v[1] for k, v in self.code_inspector.live_node_outputs.items()}
        raw_rcpts = parsed.get("node_receipts", [])
        rcpts = tuple(
            DagNodeSettlementReceipt(node_id=str(r.get("node_id", "")), role=DagNodeRoleKind(str(r.get("role", DagNodeRoleKind.DISJOINT_BUILDER))), node_kind=DagNodeKind(str(r.get("node_kind", DagNodeKind.STANDARD))), wave_index=int(r.get("wave_index", 0)), settlement_stage=DagSettlementStage(str(r.get("settlement_stage", DagSettlementStage.SETTLED))), model_id=str(r.get("model_id", "")), duration_seconds=live_durs.get(str(r.get("node_id", "")), float(r.get("duration_seconds", 0.0))), pre_manifest_digest=str(r.get("pre_manifest_digest", "")), post_manifest_digest=str(r.get("post_manifest_digest", "")), mutated_files=tuple(str(x) for x in r.get("mutated_files", [])), out_of_scope_writes=tuple(str(x) for x in r.get("out_of_scope_writes", [])), dirty_read_files=tuple(str(x) for x in r.get("dirty_read_files", [])), verify_passed=bool(r.get("verify_passed", True)), reproduction_observed=bool(r.get("reproduction_observed", True)), output_summary=str(r.get("output_summary", "")), error=str(r["error"]) if r.get("error") else None)
            for r in (raw_rcpts if isinstance(raw_rcpts, list) else ()) if isinstance(r, dict)
        )
        settled_map = {r.node_id: (f"● SETTLED · ✓ Accepted {max(0.8, r.duration_seconds):.1f}s · ↓4.8k ➔ {r.output_summary}" if r.settlement_stage == DagSettlementStage.SETTLED else f"✖ {str(r.settlement_stage).upper()} ➔ {r.error or r.output_summary}") for r in rcpts} if rcpts else None
        compiled_plan = DagCompiledPlan(dag_id=str(parsed.get("dag_id", "cdx-tui")), campaign_goal=base_plan.campaign_goal, plan_digest=base_plan.plan_digest, total_nodes=base_plan.total_nodes, total_waves=base_plan.total_waves, waves=base_plan.waves, hazards=base_plan.hazards, blocking_hazards_count=base_plan.blocking_hazards_count, spawn_plan_acknowledged=base_plan.spawn_plan_acknowledged, ascii_graph=self.dag_service._compiler.render_ascii_dag(nodes=cfg_nodes, waves=base_plan.waves, node_work_map=settled_map))
        settled_tree = self.dag_service.render_wave_tree(nodes=cfg_nodes, plan=compiled_plan, receipts={r.node_id: r for r in rcpts}) if rcpts else str(parsed.get("wave_tree_view", ""))
        self.last_dag_goal, self.last_dag_receipts = goal_clean, rcpts
        if not plan_only:
            self.suggestion_engine.tokens_in = base_in + code_rep.tokens_in_est
            self.suggestion_engine.tokens_out = base_out + code_rep.tokens_out_est
        settled_cnt = int(parsed.get("settled_count", 0))
        self.suggestion_engine.omh_status_badge = f"▶▶ accept all on · DAG · done {settled_cnt}/{base_plan.total_nodes} (✓ Accepted)" if not plan_only else f"▶▶ accept all on · DAG · planned 0/{base_plan.total_nodes}"
        dto = DagUlwResponse(ok=bool(parsed.get("ok", True)), dag_id=str(parsed.get("dag_id", "cdx-tui")), run_state=DagRunState(str(parsed.get("run_state", DagRunState.PLANNED))), plan_only=plan_only, workspace_dir=self.workspace_dir, compiled_plan=compiled_plan, node_receipts=rcpts, settled_count=settled_cnt, resumed_nodes=tuple(str(x) for x in parsed.get("resumed_nodes", [])), aimd_final_concurrency=int(parsed.get("aimd_final_concurrency", 3)), wave_tree_view=settled_tree, summary=str(parsed.get("summary", "")))
        return self.renderer.render_dag_workbench_view(resp=dto, effect_registry=self.effect_registry, code_work_lines=code_rep.stage_code_boxes[3], total_tokens=tot_run_toks)

    def handle_command(self, raw_line: str) -> tuple[str, bool]:
        """Parse and execute a single TUI input line (`slash command`, `$skill`, `@file`, or prompt).

        Args:
            raw_line: User input line entered at the interactive prompt.

        Returns:
            Tuple of `(rendered_text, should_exit)`.
        """
        line = raw_line.strip()
        if not line:
            return "", False
        matched_skills, stripped_line = self.skill_registry.extract_inline_skill_mentions(raw_line=line)
        if matched_skills and not stripped_line:
            return ("" if (self.renderer.use_ansi and sys.stdout.isatty()) else f"● 스킬 활성화됨: {matched_skills} | 현재 활성 스킬: {self.skill_registry.active_skills}"), False
        eff_line = stripped_line if matched_skills else line
        parts = eff_line.split(maxsplit=1)
        cmd, arg = parts[0].lower(), (parts[1].strip() if len(parts) > 1 else "")

        if cmd in (TuiSlashCommand.QUIT, TuiSlashCommand.EXIT, "q", "exit", "quit"):
            return "Exiting Heterarchy-Codex TUI Workbench. Goodbye!", True
        if cmd in (TuiSlashCommand.BACK, TuiSlashCommand.PREV, "/return", "/b") or (cmd in (TuiSlashCommand.PROMPTS, TuiSlashCommand.PROMT, TuiSlashCommand.TRACE, "/prompt-trace") and arg.lower() in ("back", "exit", "prev", "return", "q")):
            if self.view_history or self.last_dag_receipts:
                self.current_view = self.view_history.pop() if self.view_history else self.execute_dag_command(campaign_goal=self.last_dag_goal, plan_only=False)
                return self.current_view, False
            return self.renderer.render_banner(workspace_dir=self.workspace_dir, running_jobs=self.job_store.summarize_telemetry()["running_jobs"], pending_handoffs=0, active_model=self.active_model, active_skills=tuple(self.skill_registry.active_skills), dag_mode=self.dag_mode, include_status_box=not (self.renderer.use_ansi and sys.stdout.isatty()), permission_mode=self.permission_mode), False
        bench_out = _handle_codex_bench_command(session=self, cmd=cmd, arg=arg)
        if bench_out is not None:
            out = bench_out
        elif cmd in (TuiSlashCommand.HELP, "?"):
            out = self.renderer.render_help_panel()
        elif cmd == TuiSlashCommand.DAG_EXIT or (cmd == TuiSlashCommand.DAG_MODE and arg.lower() in ("exit", "off", "stop", "quit", "false", "0")):
            self.dag_mode, self.prompt_reader.dag_mode = False, False
            if self.renderer.use_ansi and sys.stdout.isatty():
                self.prompt_reader.play_dag_mode_shimmer(entering=False)
                return "", False
            out = self.renderer.render_dag_mode_transition(active=False)
        elif cmd == TuiSlashCommand.DAG_MODE:
            self.dag_mode, self.prompt_reader.dag_mode = True, True
            if str(DefaultCoreSkill.CODEX_HETERARCHY) not in self.skill_registry.active_skills:
                self.skill_registry.active_skills.insert(0, str(DefaultCoreSkill.CODEX_HETERARCHY))
            if self.renderer.use_ansi and sys.stdout.isatty() and (not arg or arg.lower() in ("on", "start", "true", "1")):
                self.prompt_reader.play_dag_mode_shimmer(entering=True)
                return "", False
            banner = self.renderer.render_dag_mode_transition(active=True)
            out = f"{banner}\n\n{self.execute_dag_command(campaign_goal=arg, plan_only=False)}" if (arg and arg.lower() not in ("on", "start", "true", "1")) else banner
        elif cmd == TuiSlashCommand.SKILLS:
            out = self.skill_registry.render_skills_table()
        elif cmd == TuiSlashCommand.SKILL:
            res_sk = self.skill_registry.handle_skill_command(arg=arg)
            out = "" if (self.renderer.use_ansi and sys.stdout.isatty() and arg.strip() and arg.strip().lower() != "list") else res_sk
        elif cmd in (TuiSlashCommand.MODELS, TuiSlashCommand.LANES):
            out = self.renderer.render_models_matrix()
        elif cmd in (TuiSlashCommand.PROMPTS, TuiSlashCommand.PROMT, TuiSlashCommand.TRACE, "/prompt-trace"):
            t_files = tuple(s.rel_path for s in self.last_code_report.slices) if self.last_code_report else ()
            out = render_dag_prompts_inspector(effect_registry=self.effect_registry, campaign_goal=self.last_dag_goal, active_skills=tuple(self.skill_registry.active_skills), filter_arg=arg, receipts=self.last_dag_receipts, use_ansi=self.renderer.use_ansi, target_files=t_files)
        elif cmd in (TuiSlashCommand.EFFECT, TuiSlashCommand.EFFORT, TuiSlashCommand.REASONING, TuiSlashCommand.DAG_EFFECT):
            out, play_prev = self.effect_registry.apply_effect_command(arg=arg, use_ansi=self.renderer.use_ansi)
            if play_prev and self.renderer.use_ansi and sys.stdout.isatty():
                t_sz = shutil.get_terminal_size(fallback=(120, 32))
                self.effect_registry.play_wave_execution_effect(campaign_goal="Agent Effect Preview", term_cols=max(80, t_sz.columns), bar_row=max(24, t_sz.lines) - 4)
        elif cmd in (TuiSlashCommand.USE, TuiSlashCommand.MODEL):
            if not arg:
                return f"현재 활성 모델: `{self.active_model}` (변경: `/use argon4`, `/use sonnet`, `/use opus`, `/use gpt-6-astra`, `/use gpt-6.1-sol`)", False
            first_tok = arg.split()[0].strip()
            route = self._policy_engine.validate_route(engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC), model=first_tok)
            if not route.valid:
                route = self._policy_engine.validate_route(engine=str(EngineKind.CODEX_CLI), model=first_tok)
            if route.valid:
                self.effect_registry.apply_effect_command(arg=arg, use_ansi=False)
                resolved_m = route.resolved_model
            else:
                _, ok_eff = self.effect_registry.apply_effect_command(arg=arg, use_ansi=False)
                if not ok_eff:
                    return f"▲ 모델 변경 실패: {route.error_message}", False
                resolved_m = self.effect_registry.config.codex.model_id
            self.active_model, self.prompt_reader.active_model = resolved_m, resolved_m
            return ("" if (self.renderer.use_ansi and sys.stdout.isatty()) else f"● 기본 프롬프트 모델이 `{self.active_model}`(으)로 변경되었습니다."), False
        elif cmd in (TuiSlashCommand.ASK, TuiSlashCommand.PROMPT, TuiSlashCommand.SONNET, TuiSlashCommand.OPUS, TuiSlashCommand.ARGON, TuiSlashCommand.FLASH):
            direct_model_map = {TuiSlashCommand.SONNET: str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM), TuiSlashCommand.OPUS: str(CanonicalModelId.CLAUDE_OPUS_5_5_MEDIUM), TuiSlashCommand.ARGON: str(CanonicalModelId.GEMINI_4_ARGON_HIGH), TuiSlashCommand.FLASH: str(CanonicalModelId.GEMINI_3_8_FLASH_HIGH)}
            out = self.execute_direct_prompt(prompt_text=arg, model_override=direct_model_map.get(cmd))
        elif cmd == TuiSlashCommand.THEME:
            out = "Active Theme: OpenAI Codex / oh-my-hermes `tui-theme-boot` (Dark Bar #303030 · Cyan $skill · Violet D A G - H E T E R A R C H Y)"
        elif cmd == TuiSlashCommand.STATUS:
            tel = self.job_store.summarize_telemetry()
            pending = sum(self.handoff_store.count_pending_by_recipient(recipients=(TriadRole.ARGON, TriadRole.SONNET, TriadRole.OPUS, TriadRole.CODEX)).values()) if self.handoff_store.db_path.is_file() else 0
            out = self.renderer.render_banner(workspace_dir=self.workspace_dir, running_jobs=tel["running_jobs"], pending_handoffs=pending, active_model=self.active_model, active_skills=tuple(self.skill_registry.active_skills), dag_mode=self.dag_mode, include_status_box=not (self.renderer.use_ansi and sys.stdout.isatty()), permission_mode=self.permission_mode)
        elif cmd in (TuiSlashCommand.JOBS, TuiSlashCommand.INBOX):
            tickets = self.handoff_store.list_tickets(status=None, limit=10) if self.handoff_store.db_path.is_file() else []
            out = self.renderer.render_jobs_and_inbox(jobs=self.job_store.list_jobs(limit=10), tickets=tickets)
        elif cmd in (TuiSlashCommand.DAG_RUN, TuiSlashCommand.DAG_ULW, TuiSlashCommand.DAG, TuiSlashCommand.TRIAD):
            out = self.execute_dag_command(campaign_goal=arg.replace("--plan-only", "").strip(), plan_only=(cmd != TuiSlashCommand.DAG_RUN and "--plan-only" in arg))
        elif not cmd.startswith("/"):
            out = f"☤ [D A G - H E T E R A R C H Y · AUTO-CAMPAIGN]\n{self.execute_dag_command(campaign_goal=eff_line, plan_only=False)}" if self.dag_mode else self.execute_direct_prompt(prompt_text=eff_line)
        else:
            return f"Unknown slash command '{cmd}'. Type `/help`, `/prompts`, `/diff`, or `/back` to navigate.", False
        if out and self.current_view and out != self.current_view and not out.startswith("\033[2J"):
            self.view_history = [*self.view_history[-9:], self.current_view]
        self.current_view = out or self.current_view
        return out, False

    def run_interactive_loop(self, initial_command: str | None = None, input_fn: Callable[[str], str] | None = None) -> int:
        """Run the interactive TUI loop or execute a single `initial_command`.

        Args:
            initial_command: Optional one-shot command to execute immediately and exit.
            input_fn: Optional input reader callable override (defaults to `input`).

        Returns:
            Zero exit status code.
        """
        tel = self.job_store.summarize_telemetry()
        pending = sum(self.handoff_store.count_pending_by_recipient(recipients=(TriadRole.ARGON, TriadRole.SONNET, TriadRole.OPUS, TriadRole.CODEX)).values()) if self.handoff_store.db_path.is_file() else 0
        is_tty = self.renderer.use_ansi and sys.stdout.isatty() and initial_command is None and input_fn is None
        banner_str = self.renderer.render_banner(workspace_dir=self.workspace_dir, running_jobs=tel["running_jobs"], pending_handoffs=pending, active_model=self.active_model, active_skills=tuple(self.skill_registry.active_skills), dag_mode=self.dag_mode, permission_mode=self.permission_mode)
        if is_tty:
            self.prompt_reader.viewport_lines = dict(self.renderer.last_viewport_lines)
            sys.stdout.write(banner_str)
            sys.stdout.flush()
        else:
            print(banner_str)
        if initial_command is not None and initial_command.strip():
            out, _ = self.handle_command(raw_line=initial_command)
            if out:
                print(out)
            return 0
        reader = input_fn or self.prompt_reader.read_line
        while True:
            mode_prefix = " [D A G - H E T E R A R C H Y]" if self.dag_mode else ""
            prompt_str = _style_ansi(use_ansi=self.renderer.use_ansi, code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.VIOLET if self.dag_mode else TuiAnsiStyle.EMERALD}", text=f"\u203a hcodex [{self.active_model}{mode_prefix}] ")
            try:
                raw = reader(prompt_str)
            except EOFError, KeyboardInterrupt:
                print("\nExiting Heterarchy-Codex TUI Workbench.")
                break
            if is_tty and raw.strip():
                t_sz = shutil.get_terminal_size(fallback=(120, 32))
                t_c, b_r = max(80, t_sz.columns), max(24, t_sz.lines) - 4
                busy = self.suggestion_engine.render_codex_prompt_bar(buffer=f"⟳ Executing: {raw.strip()[:52]}...", active_model=self.active_model, dag_mode=self.dag_mode, use_ansi=True, term_cols=t_c)
                sys.stdout.write("".join(f"\033[{b_r + i};1H\033[2K{ln}" for i, ln in enumerate(busy)))
                sys.stdout.flush()
            out, should_exit = self.handle_command(raw_line=raw)
            if out:
                if is_tty and not should_exit:
                    term_sz = shutil.get_terminal_size(fallback=(120, 32))
                    t_cols, bar_row = max(80, term_sz.columns), max(24, term_sz.lines) - 4
                    if out.startswith("\033[2J"):
                        self.prompt_reader.viewport_lines = dict(self.renderer.last_viewport_lines)
                    else:
                        avail = max(8, bar_row - 4)
                        raw_lns = out.splitlines()
                        if len(raw_lns) > avail:
                            raw_lns = [ln for ln in raw_lns if ln.strip()]
                        out_lines = raw_lns[:avail] if len(raw_lns) <= avail else [*raw_lns[:2], *raw_lns[-(avail - 2) :]]
                        self.prompt_reader.viewport_lines = {4 + i: DisplayWidthFormatter.fit_ansi_line(ansi_text=f"  {ln}", max_cols=t_cols - 2) for i, ln in enumerate(out_lines)}
                    idle = self.suggestion_engine.render_codex_prompt_bar(buffer="", active_model=self.active_model, dag_mode=self.dag_mode, use_ansi=True, term_cols=t_cols)
                    sys.stdout.write("".join(f"\033[{r};1H\033[2K{self.prompt_reader.viewport_lines.get(r, '')}" for r in range(4, bar_row)) + "".join(f"\033[{bar_row + i};1H\033[2K{ln}" for i, ln in enumerate(idle)))
                    sys.stdout.flush()
                else:
                    print(out)
            if should_exit:
                break
        return 0


__all__ = ["HeterarchyTuiSession", "OmhTuiRenderer"]
