"""Tri-Polar Heterarchy orchestration service (Pipeline and Council execution modes)."""

from __future__ import annotations

import asyncio
import json
from collections.abc import Callable, Coroutine
from dataclasses import replace

from app.core.collab_enums import (
    JobStatusKind,
    OrchestrationMode,
    RoleExecutionState,
    TriadRole,
    TriadStageKind,
)
from app.core.prompts import build_adversarial_audit_prompt
from app.evidence.triad_adjudicator import RoleArtifactInspector
from app.evidence.triad_role_resolver import TriadRoleResolver
from app.repositories.handoff_store import HandoffStore
from app.repositories.job_store import JobStore
from app.schemas.boundary_schemas import JsonObject
from app.schemas.inner_dto.service_request_dtos import (
    BackgroundJobDispatchRequest,
    TriadOrchestrationRequest,
)
from app.services.triad_execution_context import TriadExecutionContext
from app.services.verification_gate_runner import BridgeVerificationGateRunner

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]
type SpawnBgCallable = Callable[
    [BackgroundJobDispatchRequest, JobStore, AsyncEngineCallable], JsonObject
]


class TriadOrchestrationService:
    """Orchestrate Gemini 4 Argon, Claude Sonnet/Opus 5.5, and OpenAI Codex collaborations."""

    def __init__(
        self,
        role_resolver: TriadRoleResolver | None = None,
        gate_runner: BridgeVerificationGateRunner | None = None,
    ) -> None:
        """Initialize the triad orchestration service.

        Args:
            role_resolver: Optional TriadRoleResolver helper instance.
            gate_runner: Optional BridgeVerificationGateRunner helper instance.
        """
        self._resolver = role_resolver or TriadRoleResolver()
        self._gate_runner = gate_runner or BridgeVerificationGateRunner()
        self._inspector = RoleArtifactInspector()

    async def _execute_council_roles(self, ctx: TriadExecutionContext) -> str:
        """Execute parallel 3-pole Council mode and return serialized JSON payload.

        Args:
            ctx: Immutable execution context for the current triad run.

        Returns:
            Pretty-printed JSON string containing the council orchestration outcome.
        """
        req = ctx.req
        resumed = await asyncio.to_thread(
            self._gate_runner.load_resumable_roles,
            collaboration_id=req.resume_collaboration_id,
            workspace_dir=ctx.workspace_dir,
            source_task_digest=ctx.source_task_digest,
            job_store=ctx.job_store,
            files_inspected=req.files_inspected,
        )
        codex_audit_prompt = build_adversarial_audit_prompt(
            target_code_or_diff=ctx.effective_target_code or req.task_spec,
            design_rationale=f"{req.task_spec}\n\nInvariants:\n{req.invariants_and_rules}",
            attack_vectors=[],
            context_notes="Parallel Triad Council mode alongside Claude Sonnet 5.5 and Gemini 4 Argon.",
        )
        if ctx.active_job_id:
            ctx.job_store.update_job_stage(
                job_id=ctx.active_job_id,
                stage=str(TriadStageKind.COUNCIL_PARALLEL),
            )
        sonnet_task = codex_task = None
        argon_task: asyncio.Task[JsonObject | None] | None = None
        async with asyncio.TaskGroup() as tg:
            if TriadRole.SONNET not in resumed:
                sonnet_task = tg.create_task(ctx.invoke_sonnet())
            if TriadRole.CODEX not in resumed:
                codex_task = tg.create_task(ctx.invoke_codex(prompt=codex_audit_prompt))
            if TriadRole.ARGON not in resumed:
                argon_task = tg.create_task(ctx.invoke_argon(resolver=self._resolver))
        sonnet_res = resumed[TriadRole.SONNET] if sonnet_task is None else sonnet_task.result()
        codex_res = resumed[TriadRole.CODEX] if codex_task is None else codex_task.result()
        argon_res = resumed.get(TriadRole.ARGON) if argon_task is None else argon_task.result()
        repair_rounds: int | None = None
        if req.max_rounds > 1 and not resumed:
            if ctx.active_job_id:
                ctx.job_store.update_job_stage(
                    job_id=ctx.active_job_id,
                    stage=str(TriadStageKind.COUNCIL_ADJUDICATION),
                )
            (
                argon_res,
                sonnet_res,
                codex_res,
                repair_rounds,
            ) = await self._gate_runner.run_council_adjudication_round(
                argon_res=argon_res,
                sonnet_res=sonnet_res,
                codex_res=codex_res,
                max_rounds=req.max_rounds,
                task_spec=req.task_spec,
                invariants_and_rules=req.invariants_and_rules,
                target_code_or_context=ctx.effective_target_code,
                language=req.language,
                claude_model=req.claude_model,
                invoke_sonnet_fn=ctx.invoke_sonnet,
                invoke_codex_fn=ctx.invoke_codex,
                invoke_argon_fn=lambda extra: ctx.invoke_argon(
                    resolver=self._resolver,
                    extra_context=extra,
                ),
            )
        if ctx.active_job_id and req.run_verification_gates:
            ctx.job_store.update_job_stage(
                job_id=ctx.active_job_id,
                stage=str(TriadStageKind.VERIFICATION_GATES),
            )
        bridge_gates = (
            await self._gate_runner.run_allowlisted_gates(
                workspace_dir=ctx.workspace_dir,
                timeout_seconds=req.timeout_seconds,
            )
            if req.run_verification_gates
            else None
        )
        return ctx.finalize_snapshot(
            resolver=self._resolver,
            mode=OrchestrationMode.COUNCIL,
            provided_context_or_code=f"{req.task_spec}\n{req.target_code_or_context}",
            argon_res=argon_res,
            sonnet_res=sonnet_res,
            codex_res=codex_res,
            codex_verdict=self._inspector.extract_verdict(codex_res),
            resumed=resumed,
            repair_rounds=repair_rounds,
            bridge_gates=bridge_gates,
        )

    async def _execute_pipeline_roles(self, ctx: TriadExecutionContext) -> str:
        """Execute sequential 3-pole Pipeline mode and return serialized JSON payload.

        Args:
            ctx: Immutable execution context for the current triad run.

        Returns:
            Pretty-printed JSON string containing the pipeline orchestration outcome.
        """
        req = ctx.req
        resumed = await asyncio.to_thread(
            self._gate_runner.load_resumable_roles,
            collaboration_id=req.resume_collaboration_id,
            workspace_dir=ctx.workspace_dir,
            source_task_digest=ctx.source_task_digest,
            job_store=ctx.job_store,
            files_inspected=req.files_inspected,
        )
        if ctx.active_job_id:
            ctx.job_store.update_job_stage(
                job_id=ctx.active_job_id,
                stage=str(TriadStageKind.SONNET_CRAFT),
            )
        sonnet_res = (
            resumed[TriadRole.SONNET] if TriadRole.SONNET in resumed else await ctx.invoke_sonnet()
        )
        if self._inspector.map_state(res=sonnet_res) != RoleExecutionState.SUCCEEDED:
            return ctx.abort_at_sonnet(resolver=self._resolver, sonnet_res=sonnet_res)

        s_dict = (
            sonnet_struct
            if isinstance(sonnet_struct := sonnet_res.get("structured_output"), dict)
            else {}
        )
        crafted_code = str(s_dict.get("crafted_code", ""))
        sonnet_summary = str(s_dict.get("summary", ""))
        handoff_notes = str(s_dict.get("handoff_notes_for_codex_audit", ""))
        audit_target = crafted_code or ctx.effective_target_code or req.task_spec
        evidence_target = crafted_code or req.target_code_or_context or req.task_spec

        if TriadRole.CODEX in resumed:
            codex_res = resumed[TriadRole.CODEX]
        else:
            if ctx.active_job_id:
                ctx.job_store.update_job_stage(
                    job_id=ctx.active_job_id,
                    stage=str(TriadStageKind.CODEX_AUDIT),
                )
            audit_prompt = build_adversarial_audit_prompt(
                target_code_or_diff=audit_target,
                design_rationale=(
                    f"[Argon Spec]: {req.task_spec}\n[Invariants]: {req.invariants_and_rules}\n"
                    f"[Claude Sonnet Craft Summary]: {sonnet_summary}"
                ),
                attack_vectors=[],
                context_notes=f"Sonnet Handoff Notes for Red-Team Audit: {handoff_notes}",
            )
            codex_res = await ctx.invoke_codex(prompt=audit_prompt)
        codex_verdict = self._inspector.extract_verdict(codex_res)
        repair_rounds: int | None = None
        if req.max_rounds > 1 and TriadRole.CODEX not in resumed:
            if ctx.active_job_id:
                ctx.job_store.update_job_stage(
                    job_id=ctx.active_job_id,
                    stage=str(TriadStageKind.AUTO_REPAIR),
                )
            (
                sonnet_res,
                codex_res,
                codex_verdict,
                repair_rounds,
            ) = await self._gate_runner.run_pipeline_repair_rounds(
                sonnet_res=sonnet_res,
                codex_res=codex_res,
                codex_verdict=codex_verdict,
                max_rounds=req.max_rounds,
                task_spec=req.task_spec,
                invariants_and_rules=req.invariants_and_rules,
                target_code_or_context=ctx.effective_target_code,
                language=req.language,
                workspace_dir=ctx.workspace_dir,
                claude_model=req.claude_model,
                codex_model=req.codex_model,
                codex_reasoning_effort=req.codex_reasoning_effort,
                timeout_seconds=req.timeout_seconds,
                max_input_bytes=req.max_input_bytes,
                job_store=ctx.job_store,
                run_agy_fn=ctx.run_agy_fn,
                execute_fn=ctx.execute_fn,
            )
            if isinstance(rep_struct := sonnet_res.get("structured_output"), dict):
                sonnet_summary = str(rep_struct.get("summary", sonnet_summary))
                rep_crafted = str(rep_struct.get("crafted_code", ""))
                audit_target = rep_crafted or audit_target
                evidence_target = rep_crafted or evidence_target

        codex_critique = ""
        if isinstance(c_struct := codex_res.get("structured_output"), dict):
            codex_critique = str(
                c_struct.get("executive_critique") or c_struct.get("summary") or ""
            )
        extra_ctx = (
            f"[Claude Sonnet Summary]:\n{sonnet_summary}\n\n"
            f"[Crafted / Audited Target Code]:\n{audit_target}\n\n"
            f"[Codex Red-Team Verdict]: {codex_verdict}\n"
            f"[Codex Red-Team Critique]: {codex_critique}"
        )
        if ctx.active_job_id and TriadRole.ARGON not in resumed:
            ctx.job_store.update_job_stage(
                job_id=ctx.active_job_id,
                stage=str(TriadStageKind.ARGON_ADJUDICATION),
            )
        argon_res = (
            resumed[TriadRole.ARGON]
            if TriadRole.ARGON in resumed
            else await ctx.invoke_argon(resolver=self._resolver, extra_context=extra_ctx)
        )
        if ctx.active_job_id and req.run_verification_gates:
            ctx.job_store.update_job_stage(
                job_id=ctx.active_job_id,
                stage=str(TriadStageKind.VERIFICATION_GATES),
            )
        bridge_gates = (
            await self._gate_runner.run_allowlisted_gates(
                workspace_dir=ctx.workspace_dir,
                timeout_seconds=req.timeout_seconds,
            )
            if req.run_verification_gates
            else None
        )
        return ctx.finalize_snapshot(
            resolver=self._resolver,
            mode=OrchestrationMode.PIPELINE,
            provided_context_or_code=evidence_target,
            argon_res=argon_res,
            sonnet_res=sonnet_res,
            codex_res=codex_res,
            codex_verdict=codex_verdict,
            resumed=resumed,
            repair_rounds=repair_rounds,
            bridge_gates=bridge_gates,
        )

    async def orchestrate_triad(
        self,
        req: TriadOrchestrationRequest,
        job_store: JobStore,
        handoff_store: HandoffStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
        spawn_bg_fn: SpawnBgCallable | None = None,
    ) -> str:
        """Orchestrate Gemini 4 Argon + Claude Sonnet 5.5 + OpenAI Codex with explicit role states.

        Args:
            req: Validated TriadOrchestrationRequest bundle.
            job_store: Active JobStore instance.
            handoff_store: Active HandoffStore instance.
            run_agy_fn: Antigravity RPC invocation callable.
            execute_fn: Engine execution callable.
            spawn_bg_fn: Optional background job dispatcher callable when `async_background=True`.

        Returns:
            Pretty-printed JSON string containing the orchestration outcome and evidence scope.
        """
        if req.async_background and spawn_bg_fn is not None:

            async def _triad_bg_exec(**kw: object) -> JsonObject:
                """Execute synchronous triad orchestration inside a background job.

                Args:
                    **kw: Background dispatch keyword arguments including `request_id`.

                Returns:
                    Parsed triad orchestration result dictionary.
                """
                rid = kw.get("request_id")
                if isinstance(rid, str) and rid:
                    job_store.update_job_stage(
                        job_id=rid,
                        stage=str(TriadStageKind.INITIALIZING),
                    )
                eff_rid = rid if isinstance(rid, str) else req.request_id
                sync_req = replace(
                    req,
                    async_background=False,
                    request_id=eff_rid,
                    retry_same_request=True,
                )
                raw_json = await self.orchestrate_triad(
                    req=sync_req,
                    job_store=job_store,
                    handoff_store=handoff_store,
                    run_agy_fn=run_agy_fn,
                    execute_fn=execute_fn,
                )
                parsed = json.loads(raw_json)
                res_obj = parsed if isinstance(parsed, dict) else {}
                if (
                    isinstance(rid, str)
                    and rid
                    and (bg_j := job_store.get_job(rid))
                    and bg_j.status == JobStatusKind.RUNNING
                ):
                    job_store.complete_job(
                        job_id=rid,
                        result=res_obj,
                        metadata_updates={"current_stage": str(TriadStageKind.COMPLETED)},
                    )
                return res_obj

            bg_req = BackgroundJobDispatchRequest.from_triad_request(req=req)
            return json.dumps(
                spawn_bg_fn(bg_req, job_store, _triad_bg_exec),
                ensure_ascii=False,
                indent=2,
            )

        ctx = await asyncio.to_thread(
            TriadExecutionContext.from_inputs,
            req=req,
            job_store=job_store,
            handoff_store=handoff_store,
            run_agy_fn=run_agy_fn,
            execute_fn=execute_fn,
        )
        if ctx.normalized_mode == OrchestrationMode.COUNCIL:
            return await self._execute_council_roles(ctx=ctx)
        return await self._execute_pipeline_roles(ctx=ctx)
