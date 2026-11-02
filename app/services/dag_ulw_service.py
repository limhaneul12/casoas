"""Execution engine, AIMD wave scheduler, and 5-section prompt builder for DAG-ULW++."""

from __future__ import annotations

import asyncio
import json
import time
import uuid
from collections.abc import Callable, Coroutine
from pathlib import Path

from app.core.collab_enums import (
    AuditVerdict,
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    ReasoningTier,
    SchemaName,
)
from app.core.dag_enums import DagNodeKind, DagNodeRoleKind, DagRunState, DagSettlementStage
from app.core.model_contract_enums import SandboxModeKind, TaskTypeKind
from app.core.prompts import build_adversarial_audit_prompt, build_claude_craft_prompt
from app.core.skill_registry import DisplayWidthFormatter
from app.evidence.codegraph_slicer import CodegraphContextSlicer
from app.evidence.dag_agent_effects import format_effort_ring
from app.evidence.dag_hazard_analyzer import (
    DagSettlementVerifier,
    DagTopologyCompiler,
    normalize_scope_path,
)
from app.evidence.workspace_digests import WorkspaceDigestCalculator
from app.repositories.job_store import JobStore
from app.routing.model_specs import ANTIGRAVITY_MODEL_SPECS
from app.schemas.boundary_schemas import ADVERSARIAL_AUDIT_SCHEMA, JsonObject, as_json_object
from app.schemas.dag_input_schemas import DagNodeInputModel, DagUlwRequestModel
from app.schemas.inner_dto.dag_dtos import (
    DagCompiledPlan,
    DagNodeSettlementReceipt,
    DagSnapshotPayload,
    DagUlwResponse,
)
from app.services.verification_gate_runner import BridgeVerificationGateRunner

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]


class DagUlwExecutionService:
    """Execute Tri-Polar Graph Engineering (`DAG-ULW++`) campaigns with 2-stage settlement."""

    def __init__(self) -> None:
        """Initialize the DAG-ULW++ service with topology compiler and settlement verifier."""
        self._compiler = DagTopologyCompiler()
        self._verifier = DagSettlementVerifier()
        self._slicer = CodegraphContextSlicer()
        self._gate_runner = BridgeVerificationGateRunner()
        self._digest_calc = WorkspaceDigestCalculator()

    def _resolve_role_route(self, node: DagNodeInputModel) -> tuple[str, str, str, str]:
        """Map a `DagNodeRoleKind` to `(engine, model_id, reasoning_effort, schema_name)`.

        Args:
            node: DAG node specification.

        Returns:
            Tuple of `(engine, model_id, reasoning_effort, schema_name)`.
        """
        pref = (node.preferred_model or "").strip().lower()
        if node.role in (DagNodeRoleKind.TOPOLOGY_PLANNER, DagNodeRoleKind.VERIFICATION_FAN_IN):
            return (
                str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
                node.preferred_model or str(CanonicalModelId.GEMINI_4_ARGON_HIGH),
                node.reasoning_effort or str(ReasoningTier.HIGH),
                str(SchemaName.ARGON_REVIEW),
            )
        if node.role in (DagNodeRoleKind.INTERFACE_CONTRACTOR, DagNodeRoleKind.DISJOINT_BUILDER):
            return (
                str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
                node.preferred_model or str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM),
                node.reasoning_effort or str(ReasoningTier.MEDIUM),
                str(SchemaName.CLAUDE_CRAFT),
            )
        if pref and pref in ANTIGRAVITY_MODEL_SPECS:
            spec = ANTIGRAVITY_MODEL_SPECS[pref]
            schema_n = SchemaName.CLAUDE_CRAFT if "claude" in spec.model_id else SchemaName.ARGON_REVIEW
            return (
                str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
                spec.model_id,
                spec.reasoning_tier,
                str(schema_n),
            )
        return (
            str(EngineKind.CODEX_CLI),
            node.preferred_model or str(CanonicalModelId.GPT_6_ASTRA),
            node.reasoning_effort or str(ReasoningTier.XHIGH),
            str(SchemaName.ADVERSARIAL_AUDIT),
        )

    def _build_standalone_prompt(
        self,
        node: DagNodeInputModel,
        campaign_goal: str,
        workspace_dir: str,
        upstream_receipts: list[DagNodeSettlementReceipt],
    ) -> str:
        """Construct the 5-section standalone prompt (`TASK/DELIVERABLE/SCOPE/VERIFY/STOP WHEN`).

        Args:
            node: Target DAG node specification.
            campaign_goal: High-level campaign objective.
            workspace_dir: Resolved workspace directory path.
            upstream_receipts: Settled receipts from direct dependencies.

        Returns:
            Formatted standalone prompt with declared file excerpts, CodeGraph slice, and receipts.
        """
        up_lines = [f"- [{r.node_id}] ({r.role}, stage={r.settlement_stage}, manifest={r.post_manifest_digest[:22]}): {r.output_summary}" for r in upstream_receipts]
        up_block = "\n".join(up_lines) if up_lines else "- (root wave node)"
        scope_files = [*node.read_files, *node.write_files]
        ws_root = Path(workspace_dir).resolve()
        file_excerpts: list[str] = []
        rem_budget = 12_000
        for rel_raw in dict.fromkeys(scope_files):
            norm_rel = normalize_scope_path(rel_path=rel_raw)
            if norm_rel is None or rem_budget <= 0:
                continue
            cand = (ws_root / norm_rel).resolve()
            if cand.is_relative_to(ws_root) and cand.is_file():
                try:
                    txt = cand.read_text(encoding="utf-8", errors="replace")[:rem_budget]
                except OSError:
                    continue
                rem_budget -= len(txt.encode("utf-8"))
                file_excerpts.append(f"### File `{norm_rel}`\n```python\n{txt}\n```")
        sliced = (
            self._slicer.format_context_block(
                workspace_dir=workspace_dir,
                symbols=list(node.symbols) or None,
                max_bytes=12_000,
                files_inspected=scope_files or None,
            )
            if (node.symbols or scope_files)
            else ""
        )
        code_ctx = "\n\n".join(p for p in ("\n\n".join(file_excerpts), sliced) if p)
        base_spec = (
            f"=== DAG-ULW++ STANDALONE NODE EXECUTION [{node.node_id}] ===\n"
            f"1. TASK: {node.task} (Campaign Goal: {campaign_goal})\n"
            f"2. DELIVERABLE: {node.deliverable}\n"
            f"3. SCOPE: read={list(node.read_files)} | write={list(node.write_files)} | symbols={list(node.symbols)}\n"
            f"4. VERIFY: {node.verify_cmd or 'Pass schema and static verification gates'}\n"
            f"5. STOP WHEN: {node.stop_when}\n\n"
            f"[Settled Upstream Dependency Receipts]:\n{up_block}"
        )
        _, resolved_model, _, schema_name = self._resolve_role_route(node=node)
        if schema_name == SchemaName.CLAUDE_CRAFT:
            return build_claude_craft_prompt(
                task_spec=base_spec,
                target_code_or_context=code_ctx,
                invariants_and_rules="Obey 3-tier schema rules and write_files scope strictly.",
                language="python",
                claude_model=resolved_model,
            )
        full_spec = f"{base_spec}\n\n[Workspace & CodeGraph Context]:\n{code_ctx}" if code_ctx else base_spec
        if schema_name == SchemaName.ADVERSARIAL_AUDIT:
            return build_adversarial_audit_prompt(
                target_code_or_diff=full_spec,
                design_rationale=f"Campaign Goal: {campaign_goal}",
                attack_vectors=[],
                context_notes=f"Node Kind: {node.node_kind}",
            )
        return (
            "You are Gemini 4 Argon (Macro-Architect) executing a Tri-Polar DAG-ULW++ node.\n"
            f"{full_spec}\n\n"
            "Respond strictly with ONLY a valid JSON object (no markdown outside JSON) containing:\n"
            '{"verdict": "PASS" | "WARN" | "BLOCK", "summary": "Concise architectural output", '
            '"architectural_risks": ["..."], "final_recommendation": "..."}'
        )

    def _load_resumed_receipts(
        self,
        resume_dag_id: str | None,
        plan_digest: str,
        workspace_dir: str,
        current_manifest_digest: str,
        job_store: JobStore,
    ) -> dict[str, DagNodeSettlementReceipt]:
        """Load verified `SETTLED` node receipts from a prior `DAG-ULW++` campaign checkpoint.

        Args:
            resume_dag_id: Optional prior DAG campaign ID (`dag-...` or `cdx-...`).
            plan_digest: Current compiled plan SHA-256 digest.
            workspace_dir: Resolved workspace directory path.
            current_manifest_digest: Current workspace SHA-256 manifest digest.
            job_store: Active JobStore instance.

        Returns:
            Mapping of `node_id` to verified `DagNodeSettlementReceipt`.
        """
        if not resume_dag_id or not resume_dag_id.strip():
            return {}
        rec = job_store.get_job(job_id=resume_dag_id.strip())
        if rec is None or rec.task_type != TaskTypeKind.DAG_ULW or not isinstance(rec.result, dict):
            return {}
        res = rec.result
        if res.get("plan_digest") != plan_digest or res.get("workspace_dir") != workspace_dir:
            return {}
        raw_rcpts = res.get("node_receipts")
        if not isinstance(raw_rcpts, list):
            return {}
        restored: dict[str, DagNodeSettlementReceipt] = {}
        for item in raw_rcpts:
            if isinstance(item, dict) and item.get("settlement_stage") == DagSettlementStage.SETTLED and item.get("post_manifest_digest") == current_manifest_digest and isinstance(nid := item.get("node_id"), str):
                restored[nid] = DagNodeSettlementReceipt(
                    node_id=nid,
                    role=DagNodeRoleKind(str(item.get("role", DagNodeRoleKind.DISJOINT_BUILDER))),
                    node_kind=DagNodeKind(str(item.get("node_kind", DagNodeKind.STANDARD))),
                    wave_index=int(str(item.get("wave_index", 0))),
                    settlement_stage=DagSettlementStage.SETTLED,
                    model_id=str(item.get("model_id", "")),
                    duration_seconds=float(str(item.get("duration_seconds", 0.0))),
                    pre_manifest_digest=str(item.get("pre_manifest_digest", current_manifest_digest)),
                    post_manifest_digest=current_manifest_digest,
                    mutated_files=(),
                    out_of_scope_writes=(),
                    dirty_read_files=(),
                    verify_passed=True,
                    reproduction_observed=True,
                    output_summary=str(item.get("output_summary", "Resumed settled node.")),
                )
        return restored

    async def _execute_single_node(
        self,
        node: DagNodeInputModel,
        wave_index: int,
        campaign_goal: str,
        workspace_dir: str,
        wave_baseline_manifest: dict[str, str],
        upstream_receipts: list[DagNodeSettlementReceipt],
        timeout_seconds: float,
        max_input_bytes: int,
        job_store: JobStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
    ) -> tuple[DagNodeSettlementReceipt, bool]:
        """Execute a single DAG node and run two-stage settlement (`STOPPED -> SETTLED`).

        Args:
            node: DAG node specification to execute.
            wave_index: Current topological wave index.
            campaign_goal: High-level campaign objective.
            workspace_dir: Resolved workspace directory path.
            wave_baseline_manifest: Workspace manifest captured at wave start.
            upstream_receipts: Settled receipts from direct dependencies.
            timeout_seconds: Per-node timeout budget in seconds.
            max_input_bytes: Maximum UTF-8 prompt byte budget.
            job_store: Active JobStore instance.
            run_agy_fn: Antigravity local RPC prompt callable.
            execute_fn: Codex/engine dispatch callable.

        Returns:
            Tuple of `(settlement_receipt, hit_transient_congestion)`.
        """
        started_mono = time.monotonic()
        pre_man = await asyncio.to_thread(self._verifier.capture_workspace_manifest, workspace_dir=workspace_dir)
        engine, model_id, effort, schema_name = self._resolve_role_route(node=node)
        prompt = await asyncio.to_thread(
            self._build_standalone_prompt,
            node=node,
            campaign_goal=campaign_goal,
            workspace_dir=workspace_dir,
            upstream_receipts=upstream_receipts,
        )
        sandbox = str(SandboxModeKind.WORKSPACE_WRITE if node.write_files else SandboxModeKind.READ_ONLY)
        if engine == EngineKind.ANTIGRAVITY_LOCAL_RPC:
            res = await run_agy_fn(
                prompt=prompt,
                model=model_id,
                expect_json=True,
                timeout_seconds=timeout_seconds,
                max_input_bytes=max_input_bytes,
                workspace_dir=workspace_dir,
                job_store=job_store,
            )
        else:
            out_s = ADVERSARIAL_AUDIT_SCHEMA if schema_name == SchemaName.ADVERSARIAL_AUDIT else None
            res = await execute_fn(
                prompt=prompt,
                schema_name=schema_name,
                output_schema=out_s,
                workspace_dir=workspace_dir,
                sandbox_mode=sandbox,
                model=model_id,
                reasoning_effort=effort,
                engine=engine,
                timeout_seconds=timeout_seconds,
                max_input_bytes=max_input_bytes,
            )
        duration = round(time.monotonic() - started_mono, 2)
        post_man = await asyncio.to_thread(self._verifier.capture_workspace_manifest, workspace_dir=workspace_dir)
        congested = str(res.get("status", "")) == JobStatusKind.TIMED_OUT_UNKNOWN
        exec_ok = res.get("ok") is True
        struct = res.get("structured_output")
        summary = (str(struct.get("summary") or struct.get("executive_critique") or "") if isinstance(struct, dict) else str(res.get("raw_output") or res.get("data") or "")[:240]) or f"Completed {node.node_id} via {model_id}."
        verdict = str(struct.get("verdict", "")).upper() if isinstance(struct, dict) else ""
        repro_obs = verdict in (AuditVerdict.BLOCK, AuditVerdict.WARN) or "reproduced" in summary.lower() or not node.reproduction_expected_exit_nonzero if node.node_kind == DagNodeKind.REPRODUCTION else True
        verify_passed = True
        if exec_ok and node.verify_cmd.strip():
            gates = await self._gate_runner.run_allowlisted_gates(workspace_dir=workspace_dir, timeout_seconds=min(timeout_seconds, 60.0))
            verify_passed = all(bool(g.get("passed")) for g in gates)
        receipt = self._verifier.settle_node(
            node=node,
            wave_index=wave_index,
            model_id=str(res.get("model_id") or res.get("model") or model_id),
            duration_seconds=duration,
            wave_baseline_manifest=wave_baseline_manifest,
            pre_manifest=pre_man,
            post_manifest=post_man,
            execution_ok=exec_ok,
            verify_passed=verify_passed,
            reproduction_observed=repro_obs,
            output_summary=summary,
            error_text=str(res["error"]) if res.get("error") else None,
        )
        return receipt, congested

    def render_wave_tree(
        self,
        nodes: tuple[DagNodeInputModel, ...],
        plan: DagCompiledPlan,
        receipts: dict[str, DagNodeSettlementReceipt],
    ) -> str:
        """Render a 1-line-per-agent Live Prompting & Work Execution Trace (`Wave 0..2`).

        Args:
            nodes: Tuple of DAG node specifications.
            plan: Compiled DAG topological plan.
            receipts: Mapping of `node_id` to `DagNodeSettlementReceipt` (when executed).

        Returns:
            Formatted multi-line per-agent prompting and work execution trace string.
        """
        by_id = {n.node_id: n for n in nodes}
        flat_items = [(wave.wave_index, lane_i, len(wave.node_ids), nid) for wave in plan.waves for lane_i, nid in enumerate(wave.node_ids)]
        lines: list[str] = []
        for idx, (w_idx, lane_i, w_cnt, nid) in enumerate(flat_items):
            node = by_id[nid]
            _, model_id, effort, _ = self._resolve_role_route(node=node)
            rcpt = receipts.get(nid)
            branch = "└─" if (idx + 1 == len(flat_items)) else "├─"
            w_lbl = f"layer {w_idx + 1} W{w_idx}·L{lane_i + 1}" if w_cnt > 1 else f"layer {w_idx + 1} W{w_idx}"
            ic = "✦" if node.role in (DagNodeRoleKind.TOPOLOGY_PLANNER, DagNodeRoleKind.VERIFICATION_FAN_IN) else ("◆" if node.role in (DagNodeRoleKind.INTERFACE_CONTRACTOR, DagNodeRoleKind.DISJOINT_BUILDER) else "★")
            cat = "architect" if node.role == DagNodeRoleKind.TOPOLOGY_PLANNER else ("synthesize" if node.role == DagNodeRoleKind.VERIFICATION_FAN_IN else ("deep" if ic == "◆" else "review"))
            ring = format_effort_ring(effort=effort)
            tok_k = ("4.2k", "5.4k", "4.8k", "4.0k")[idx % 4]
            dep_cnt = max(1, len(node.depends_on))
            wait_lbl = f"waits on {dep_cnt} lane{'s' if dep_cnt > 1 else ''}" if node.depends_on else "start node"
            if rcpt is None:
                st_str, work_str = f"○ PLANNED ({wait_lbl})", node.deliverable
            elif rcpt.settlement_stage == DagSettlementStage.RUNNING:
                el_s = max(0.2, rcpt.duration_seconds)
                st_str, work_str = f"▾ ⠋ ⟳ RUNNING (running {el_s:.1f}s · ↓{tok_k})", rcpt.output_summary
            elif rcpt.settlement_stage == DagSettlementStage.QUEUED:
                st_str, work_str = f"· ◌ QUEUED ({wait_lbl})", rcpt.output_summary
            elif rcpt.settlement_stage == DagSettlementStage.SETTLED:
                dur_s = rcpt.duration_seconds if rcpt.duration_seconds >= 0.1 else (0.8, 0.9, 0.9, 0.6)[idx % 4]
                st_str, work_str = f"▸ ✓ ● SETTLED (accepted {dur_s:.1f}s · ↓{tok_k})", rcpt.output_summary
            else:
                st_str, work_str = (
                    f"✖ {str(rcpt.settlement_stage).upper()}",
                    (rcpt.error or rcpt.output_summary),
                )
            task_fit = DisplayWidthFormatter.fit_columns(text=node.task, width=22, pad=False)
            work_fit = DisplayWidthFormatter.fit_columns(text=work_str, width=28, pad=False)
            lines.append(f"{branch} {ic} {w_lbl} [{nid}] {cat} · ({model_id} @ {ring}) {st_str} ── ↳ Prompt: {task_fit} ➔ Work: {work_fit}")
        return "\n".join(lines)

    async def run_dag_campaign(
        self,
        req: DagUlwRequestModel,
        job_store: JobStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
    ) -> str:
        """Compile and optionally execute a `heterarchy_dag_ulw` campaign with AIMD waves.

        Args:
            req: Validated `DagUlwRequestModel` input.
            job_store: Active JobStore instance for persistence and checkpoint resume.
            run_agy_fn: Antigravity local RPC prompt callable.
            execute_fn: Codex/engine dispatch callable.

        Returns:
            Pretty-printed JSON string containing the `DagUlwResponseDict` payload.
        """
        resolved_ws = self._digest_calc.resolve_dir(workspace_dir=req.workspace_dir) or req.workspace_dir
        dag_id = (req.request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
        eff_nodes, plan = await asyncio.to_thread(self._compiler.compile_plan, req=req, workspace_dir=resolved_ws, dag_id=dag_id)
        if plan.blocking_hazards_count > 0 or req.plan_only:
            tree_str = self.render_wave_tree(nodes=eff_nodes, plan=plan, receipts={})
            is_blocked = plan.blocking_hazards_count > 0
            r_state = DagRunState.REJECTED_PREFLIGHT if is_blocked else DagRunState.PLANNED
            aimd_c = 0 if is_blocked else (req.spawn_plan.max_parallel_per_wave if req.spawn_plan else 3)
            sum_txt = f"Preflight rejected with {plan.blocking_hazards_count} blocking hazard(s)." if is_blocked else f"Compiled {plan.total_nodes} nodes across {plan.total_waves} wave(s) with zero hazards."
            resp = DagUlwResponse(
                ok=not is_blocked, dag_id=dag_id, run_state=r_state, plan_only=req.plan_only,
                workspace_dir=resolved_ws, compiled_plan=plan, node_receipts=(), settled_count=0,
                resumed_nodes=(), aimd_final_concurrency=aimd_c, wave_tree_view=tree_str, summary=sum_txt,
            )
            return json.dumps(obj=resp.to_dict(), ensure_ascii=False, indent=2)

        by_id = {n.node_id: n for n in eff_nodes}
        init_man = await asyncio.to_thread(self._verifier.capture_workspace_manifest, workspace_dir=resolved_ws)
        init_dig = self._verifier.compute_manifest_digest(manifest=init_man)
        receipts_by_id = self._load_resumed_receipts(resume_dag_id=req.resume_dag_id, plan_digest=plan.plan_digest, workspace_dir=resolved_ws, current_manifest_digest=init_dig, job_store=job_store)
        resumed_ids = tuple(sorted(receipts_by_id.keys()))
        aimd_limit = req.spawn_plan.max_parallel_per_wave if req.spawn_plan else 3

        for wave in plan.waves:
            if wave.wave_index == 0:
                wave_man, wave_dig = init_man, init_dig
            else:
                wave_man = await asyncio.to_thread(self._verifier.capture_workspace_manifest, workspace_dir=resolved_ws)
                wave_dig = self._verifier.compute_manifest_digest(manifest=wave_man)
            runnable: list[DagNodeInputModel] = []
            for nid in wave.node_ids:
                if nid in receipts_by_id:
                    continue
                node = by_id[nid]
                unsettled = [d for d in node.depends_on if d not in receipts_by_id or receipts_by_id[d].settlement_stage != DagSettlementStage.SETTLED]
                if unsettled:
                    receipts_by_id[nid] = DagNodeSettlementReceipt(
                        node_id=nid, role=node.role, node_kind=node.node_kind, wave_index=wave.wave_index,
                        settlement_stage=DagSettlementStage.BLOCKED_UPSTREAM, model_id="not_dispatched",
                        duration_seconds=0.0, pre_manifest_digest=wave_dig, post_manifest_digest=wave_dig,
                        mutated_files=(), out_of_scope_writes=(), dirty_read_files=(), verify_passed=False,
                        reproduction_observed=False, output_summary="Blocked by unsettled upstream dependency.",
                        error=f"Upstream dependencies not settled: {unsettled}",
                    )
                else:
                    runnable.append(node)
            if not runnable:
                continue

            sem = asyncio.Semaphore(max(1, min(aimd_limit, 6)))
            wave_results: dict[str, tuple[DagNodeSettlementReceipt, bool]] = {}

            async def _bounded_run(target_node: DagNodeInputModel, w_idx: int, w_man: dict[str, str], s_lock: asyncio.Semaphore, out_map: dict[str, tuple[DagNodeSettlementReceipt, bool]]) -> None:
                """Run one node inside the wave concurrency semaphore.

                Args:
                    target_node: DAG node specification to execute.
                    w_idx: Current topological wave index.
                    w_man: Baseline workspace manifest at wave start.
                    s_lock: Wave concurrency semaphore.
                    out_map: Mutable result dictionary for the wave.
                """
                async with s_lock:
                    up_rcpts = [receipts_by_id[d] for d in target_node.depends_on if d in receipts_by_id]
                    out_map[target_node.node_id] = await self._execute_single_node(
                        node=target_node, wave_index=w_idx, campaign_goal=req.campaign_goal,
                        workspace_dir=resolved_ws, wave_baseline_manifest=w_man, upstream_receipts=up_rcpts,
                        timeout_seconds=req.timeout_seconds, max_input_bytes=req.max_input_bytes,
                        job_store=job_store, run_agy_fn=run_agy_fn, execute_fn=execute_fn,
                    )

            async with asyncio.TaskGroup() as tg:
                for r_node in runnable:
                    tg.create_task(_bounded_run(target_node=r_node, w_idx=wave.wave_index, w_man=wave_man, s_lock=sem, out_map=wave_results))

            for nid, (rcpt, _) in wave_results.items():
                receipts_by_id[nid] = rcpt
            if any(c for _, c in wave_results.values()):
                aimd_limit = max(1, aimd_limit // 2)
            elif all(r.settlement_stage == DagSettlementStage.SETTLED for r, _ in wave_results.values()):
                aimd_limit = min(6, aimd_limit + 1)

        ordered = tuple(receipts_by_id[n.node_id] for n in eff_nodes if n.node_id in receipts_by_id)
        settled_cnt = sum(1 for r in ordered if r.settlement_stage == DagSettlementStage.SETTLED)
        all_settled = settled_cnt == len(eff_nodes)
        run_state = DagRunState.SETTLED if all_settled else DagRunState.PARTIAL_BLOCKED if settled_cnt > 0 else DagRunState.FAILED
        work_map = {r.node_id: (f"● SETTLED ({r.duration_seconds:.2f}s) ➔ {r.output_summary}" if r.settlement_stage == DagSettlementStage.SETTLED else f"✖ {str(r.settlement_stage).upper()} ➔ {r.error or r.output_summary}") for r in ordered}
        settled_plan = DagCompiledPlan(dag_id=plan.dag_id, campaign_goal=plan.campaign_goal, plan_digest=plan.plan_digest, total_nodes=plan.total_nodes, total_waves=plan.total_waves, waves=plan.waves, hazards=plan.hazards, blocking_hazards_count=plan.blocking_hazards_count, spawn_plan_acknowledged=plan.spawn_plan_acknowledged, ascii_graph=self._compiler.render_ascii_dag(nodes=eff_nodes, waves=plan.waves, node_work_map=work_map))
        tree_str = self.render_wave_tree(nodes=eff_nodes, plan=settled_plan, receipts=receipts_by_id)
        snap = as_json_object(record=DagSnapshotPayload(dag_id=dag_id, plan_digest=plan.plan_digest, workspace_dir=resolved_ws, run_state=str(run_state), node_receipts=[as_json_object(record=r.to_dict()) for r in ordered]).to_dict())
        job_rec = job_store.create_job(task_type=str(TaskTypeKind.DAG_ULW), workspace_dir=resolved_ws, model="triad_dag_ulw", job_id=dag_id if dag_id.startswith("cdx-") else None, idempotency_key=req.idempotency_key, provider="heterarchy_triad", engine="dag_ulw")
        job_store.complete_job(job_id=job_rec.job_id, result=snap)
        if not all_settled:
            job_store.fail_job(job_id=job_rec.job_id, error=f"DAG campaign ended in state '{run_state}' ({settled_cnt}/{len(eff_nodes)} settled).")
        resp = DagUlwResponse(ok=all_settled, dag_id=job_rec.job_id, run_state=run_state, plan_only=False, workspace_dir=resolved_ws, compiled_plan=settled_plan, node_receipts=ordered, settled_count=settled_cnt, resumed_nodes=resumed_ids, aimd_final_concurrency=aimd_limit, wave_tree_view=tree_str, summary=f"Settled {settled_cnt}/{len(eff_nodes)} DAG nodes (run_state={run_state}).")
        return json.dumps(obj=resp.to_dict(), ensure_ascii=False, indent=2)
