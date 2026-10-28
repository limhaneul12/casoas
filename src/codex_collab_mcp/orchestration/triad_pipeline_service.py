"""Tri-Polar Heterarchy orchestration service (Pipeline and Council execution modes)."""

from __future__ import annotations

import asyncio
import json
import time
from collections.abc import Callable, Coroutine
from dataclasses import dataclass, field
from pathlib import Path

from codex_collab_mcp.collab_enums import (
    CanonicalModelId,
    CollaborationState,
    OrchestrationMode,
    RoleExecutionState,
    SchemaName,
    TriadRole,
)
from codex_collab_mcp.evidence.codegraph_slicer import slice_codegraph_symbols
from codex_collab_mcp.evidence.scope_builder import prepare_triad_role_evidence
from codex_collab_mcp.evidence.triad_adjudicator import extract_verdict, map_response_to_role_state
from codex_collab_mcp.evidence.triad_role_resolver import TriadRoleResolver
from codex_collab_mcp.evidence.workspace_digests import compute_task_digest, resolve_workspace_dir
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.orchestration.service_dicts import TriadOrchestrationRequest
from codex_collab_mcp.orchestration.verification_gate_runner import BridgeVerificationGateRunner
from codex_collab_mcp.prompts import build_adversarial_audit_prompt, build_claude_craft_prompt
from codex_collab_mcp.schemas import ADVERSARIAL_AUDIT_SCHEMA, JsonObject, as_json_object

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]


@dataclass(frozen=True, slots=True)
class _TriadExecutionContext:
    """Immutable internal execution context for a single Triad orchestration run."""

    req: TriadOrchestrationRequest
    normalized_mode: str
    workspace_dir: str
    effective_target_code: str
    sonnet_prompt: str
    argon_prompt: str
    source_task_digest: str
    job_store: JobStore
    handoff_store: HandoffStore
    run_agy_fn: AsyncEngineCallable
    execute_fn: AsyncEngineCallable
    started_at: float = field(default_factory=time.time)

    @classmethod
    def from_inputs(
        cls,
        req: TriadOrchestrationRequest,
        job_store: JobStore,
        handoff_store: HandoffStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
    ) -> _TriadExecutionContext:
        """Validate workspace directory and build the immutable execution context.

        Args:
            req: Validated TriadOrchestrationRequest bundle.
            job_store: Active JobStore instance.
            handoff_store: Active HandoffStore instance.
            run_agy_fn: Antigravity RPC invocation callable.
            execute_fn: Engine execution callable.

        Returns:
            Initialized `_TriadExecutionContext` instance with validated workspace and prompts.
        """
        resolved_workspace = resolve_workspace_dir(req.workspace_dir)
        if resolved_workspace is None or not Path(resolved_workspace).is_dir():
            raise ValueError("Workspace directory must resolve to an existing directory.")
        normalized_mode = req.mode.strip().lower()
        source_task_digest = compute_task_digest(
            req.task_spec,
            req.target_code_or_context,
            req.invariants_and_rules,
            req.language,
            normalized_mode,
        )
        sliced = (
            slice_codegraph_symbols(resolved_workspace, req.symbols_inspected)
            if req.symbols_inspected
            else ""
        )
        effective_target_code = (
            f"{req.target_code_or_context}\n\n[CodeGraph Sliced Symbol Context]:\n{sliced}"
            if sliced
            else req.target_code_or_context
        )
        sonnet_prompt = build_claude_craft_prompt(
            req.task_spec,
            effective_target_code,
            req.invariants_and_rules,
            req.language,
            req.claude_model,
        )
        argon_prompt = (
            "You are Gemini 4 Argon (Macro-Architect) adjudicating a Tri-Polar Heterarchy task.\n"
            f"[Task Specification]:\n{req.task_spec}\n\n"
            f"[Invariants & Rules]:\n{req.invariants_and_rules}\n\n"
            f"[Target Code / Context]:\n{effective_target_code}\n\n"
            "Respond strictly with a JSON object containing:\n"
            '{"verdict": "PASS" | "WARN" | "BLOCK", "summary": "...", '
            '"architectural_risks": ["..."], "final_recommendation": "..."}'
        )
        return cls(
            req=req,
            normalized_mode=normalized_mode,
            workspace_dir=resolved_workspace,
            effective_target_code=effective_target_code,
            sonnet_prompt=sonnet_prompt,
            argon_prompt=argon_prompt,
            source_task_digest=source_task_digest,
            job_store=job_store,
            handoff_store=handoff_store,
            run_agy_fn=run_agy_fn,
            execute_fn=execute_fn,
        )

    async def invoke_sonnet(self) -> JsonObject:
        """Invoke the Claude Sonnet/Opus craftsman pole with this context's configuration.

        Returns:
            Claude role response dictionary.
        """
        return await self.run_agy_fn(
            prompt=self.sonnet_prompt,
            model=self.req.claude_model,
            expect_json=True,
            timeout_seconds=self.req.timeout_seconds,
            max_input_bytes=self.req.max_input_bytes,
            workspace_dir=self.workspace_dir,
            job_store=self.job_store,
        )

    async def invoke_codex(self, prompt: str) -> JsonObject:
        """Invoke the OpenAI Codex red-team auditor pole with this context's configuration.

        Args:
            prompt: Adversarial audit prompt string.

        Returns:
            Codex role response dictionary.
        """
        return await self.execute_fn(
            prompt=prompt,
            schema_name=SchemaName.ADVERSARIAL_AUDIT,
            output_schema=ADVERSARIAL_AUDIT_SCHEMA,
            workspace_dir=self.workspace_dir,
            sandbox_mode="read-only",
            model=self.req.codex_model,
            reasoning_effort=self.req.codex_reasoning_effort,
            engine="cli",
            timeout_seconds=self.req.timeout_seconds,
            max_input_bytes=self.req.max_input_bytes,
        )

    async def invoke_argon(
        self, resolver: TriadRoleResolver, extra_context: str = ""
    ) -> JsonObject | None:
        """Invoke or resolve the Gemini 4 Argon macro-architect pole.

        Args:
            resolver: TriadRoleResolver helper instance.
            extra_context: Optional downstream role context to append.

        Returns:
            Argon role response dictionary or None.
        """
        return await resolver.resolve_argon_role(
            argon_model=self.req.argon_model,
            argon_prompt=self.argon_prompt,
            extra_context=extra_context,
            argon_handoff_ticket_id=self.req.argon_handoff_ticket_id,
            invoke_argon_rpc=self.req.invoke_argon_rpc,
            timeout_seconds=self.req.timeout_seconds,
            max_input_bytes=self.req.max_input_bytes,
            workspace_dir=self.workspace_dir,
            job_store=self.job_store,
            handoff_store=self.handoff_store,
            run_agy_fn=self.run_agy_fn,
        )


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

    def _finalize_triad_snapshot(
        self,
        ctx: _TriadExecutionContext,
        *,
        mode: str,
        provided_context_or_code: str,
        argon_res: JsonObject | None,
        sonnet_res: JsonObject,
        codex_res: JsonObject,
        codex_verdict: str | None,
        resumed_roles: list[str] | None = None,
        repair_rounds_executed: int | None = None,
        bridge_executed_gates: list[JsonObject] | None = None,
    ) -> str:
        """Assemble the 3-pole response payload, attach evidence scope, and serialize JSON.

        Args:
            ctx: Immutable execution context for the current triad run.
            mode: Normalized orchestration mode (`council` or `pipeline`).
            provided_context_or_code: Inline code or context string to hash for evidence scope.
            argon_res: Gemini 4 Argon role response dictionary or None.
            sonnet_res: Claude Sonnet/Opus role response dictionary.
            codex_res: OpenAI Codex role response dictionary.
            codex_verdict: Extracted verdict string from Codex response, or None.
            resumed_roles: Optional sorted list of role names resumed from a prior checkpoint.
            repair_rounds_executed: Optional count of auto-repair rounds executed.
            bridge_executed_gates: Optional bridge-executed verification gate records.

        Returns:
            Pretty-printed JSON string representing the completed triad payload.
        """
        adjudication, role_snapshots, role_evidence_dimensions = prepare_triad_role_evidence(
            {TriadRole.ARGON: argon_res, TriadRole.SONNET: sonnet_res, TriadRole.CODEX: codex_res}
        )
        consensus_reached = adjudication["consensus_reached"]
        collab_status = adjudication["collaboration_status"]
        req = ctx.req
        participants: JsonObject = {
            "macro_architect": req.argon_model,
            "code_craftsman": sonnet_res.get("model_id", req.claude_model),
            "red_team_auditor": codex_res.get(
                "model", req.codex_model or CanonicalModelId.CODEX_DEFAULT
            ),
        }
        payload: JsonObject = {
            "ok": collab_status == CollaborationState.COMPLETE and consensus_reached,
            "collaboration_status": collab_status,
            "consensus_status": adjudication["consensus_status"],
            "consensus_reached": consensus_reached,
            "triad_consensus_ready": consensus_reached,
            "orchestration_mode": mode,
            "participants": participants,
            "adjudication": as_json_object(adjudication),
            "argon_review": argon_res,
            "sonnet_craft": sonnet_res,
            "codex_audit": codex_res,
            "codex_verdict": codex_verdict,
        }
        if resumed_roles:
            payload["resumed_roles"] = list(resumed_roles)
            payload["resumed_from_collaboration_id"] = req.resume_collaboration_id
        if repair_rounds_executed is not None:
            payload["repair_rounds_executed"] = repair_rounds_executed
        self._resolver.attach_triad_evidence_and_persist(
            payload,
            workspace_dir=ctx.workspace_dir,
            provided_context_or_code=provided_context_or_code,
            files_inspected=req.files_inspected,
            symbols_inspected=req.symbols_inspected,
            executed_test_commands=req.executed_test_commands,
            bridge_executed_gates=bridge_executed_gates,
            orchestration_mode=mode,
            source_task_digest=ctx.source_task_digest,
            role_snapshots=role_snapshots,
            role_evidence_dimensions=role_evidence_dimensions,
            adjudication=adjudication,
            job_store=ctx.job_store,
            started_at=ctx.started_at,
        )
        return json.dumps(payload, ensure_ascii=False, indent=2)

    async def _execute_council_roles(self, ctx: _TriadExecutionContext) -> str:
        """Execute parallel 3-pole Council mode and return serialized JSON payload.

        Args:
            ctx: Immutable execution context for the current triad run.

        Returns:
            Pretty-printed JSON string containing the council orchestration outcome.
        """
        req = ctx.req
        resumed = self._gate_runner.load_resumable_roles(
            req.resume_collaboration_id, ctx.workspace_dir, ctx.source_task_digest, ctx.job_store
        )
        codex_audit_prompt = build_adversarial_audit_prompt(
            target_code_or_diff=ctx.effective_target_code or req.task_spec,
            design_rationale=f"{req.task_spec}\n\nInvariants:\n{req.invariants_and_rules}",
            attack_vectors=[],
            context_notes=(
                "Parallel Triad Council mode alongside Claude Sonnet 5.5 and Gemini 4 Argon."
            ),
        )
        sonnet_task: asyncio.Task[JsonObject] | None = None
        codex_task: asyncio.Task[JsonObject] | None = None
        argon_task: asyncio.Task[JsonObject | None] | None = None
        async with asyncio.TaskGroup() as tg:
            if TriadRole.SONNET not in resumed:
                sonnet_task = tg.create_task(ctx.invoke_sonnet())
            if TriadRole.CODEX not in resumed:
                codex_task = tg.create_task(ctx.invoke_codex(codex_audit_prompt))
            if TriadRole.ARGON not in resumed:
                argon_task = tg.create_task(ctx.invoke_argon(self._resolver))
        sonnet_res = resumed[TriadRole.SONNET] if sonnet_task is None else sonnet_task.result()
        codex_res = resumed[TriadRole.CODEX] if codex_task is None else codex_task.result()
        argon_res = resumed.get(TriadRole.ARGON) if argon_task is None else argon_task.result()
        bridge_gates = (
            await self._gate_runner.run_allowlisted_gates(ctx.workspace_dir, req.timeout_seconds)
            if req.run_verification_gates
            else None
        )
        return self._finalize_triad_snapshot(
            ctx,
            mode=OrchestrationMode.COUNCIL,
            provided_context_or_code=f"{req.task_spec}\n{ctx.effective_target_code}",
            argon_res=argon_res,
            sonnet_res=sonnet_res,
            codex_res=codex_res,
            codex_verdict=extract_verdict(codex_res),
            resumed_roles=sorted(resumed.keys()) if resumed else None,
            bridge_executed_gates=bridge_gates,
        )

    async def _execute_pipeline_roles(self, ctx: _TriadExecutionContext) -> str:
        """Execute sequential 3-pole Pipeline mode and return serialized JSON payload.

        Args:
            ctx: Immutable execution context for the current triad run.

        Returns:
            Pretty-printed JSON string containing the pipeline orchestration outcome.
        """
        req = ctx.req
        resumed = self._gate_runner.load_resumable_roles(
            req.resume_collaboration_id, ctx.workspace_dir, ctx.source_task_digest, ctx.job_store
        )
        sonnet_res = (
            resumed[TriadRole.SONNET] if TriadRole.SONNET in resumed else await ctx.invoke_sonnet()
        )
        if map_response_to_role_state(sonnet_res) != RoleExecutionState.SUCCEEDED:
            return self._resolver.build_sonnet_abort_payload(
                sonnet_res,
                workspace_dir=ctx.workspace_dir,
                provided_context_or_code=f"{req.task_spec}\n{ctx.effective_target_code}",
                files_inspected=req.files_inspected,
                symbols_inspected=req.symbols_inspected,
                executed_test_commands=req.executed_test_commands,
                source_task_digest=ctx.source_task_digest,
                job_store=ctx.job_store,
                started_at=ctx.started_at,
            )

        sonnet_struct = sonnet_res.get("structured_output")
        s_dict = sonnet_struct if isinstance(sonnet_struct, dict) else {}
        crafted_code = str(s_dict.get("crafted_code", ""))
        sonnet_summary = str(s_dict.get("summary", ""))
        handoff_notes = str(s_dict.get("handoff_notes_for_codex_audit", ""))
        audit_target = crafted_code or ctx.effective_target_code or req.task_spec

        if TriadRole.CODEX in resumed:
            codex_res = resumed[TriadRole.CODEX]
        else:
            audit_prompt = build_adversarial_audit_prompt(
                target_code_or_diff=audit_target,
                design_rationale=(
                    f"[Argon Spec]: {req.task_spec}\n[Invariants]: {req.invariants_and_rules}\n"
                    f"[Claude Sonnet Craft Summary]: {sonnet_summary}"
                ),
                attack_vectors=[],
                context_notes=f"Sonnet Handoff Notes for Red-Team Audit: {handoff_notes}",
            )
            codex_res = await ctx.invoke_codex(audit_prompt)
        codex_verdict = extract_verdict(codex_res)
        repair_rounds: int | None = None
        if req.max_rounds > 1 and TriadRole.CODEX not in resumed:
            (
                sonnet_res,
                codex_res,
                codex_verdict,
                repair_rounds,
            ) = await self._gate_runner.run_pipeline_repair_rounds(
                sonnet_res,
                codex_res,
                codex_verdict,
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
                audit_target = str(rep_struct.get("crafted_code", "")) or audit_target

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
        argon_res = (
            resumed[TriadRole.ARGON]
            if TriadRole.ARGON in resumed
            else await ctx.invoke_argon(self._resolver, extra_ctx)
        )
        bridge_gates = (
            await self._gate_runner.run_allowlisted_gates(ctx.workspace_dir, req.timeout_seconds)
            if req.run_verification_gates
            else None
        )
        return self._finalize_triad_snapshot(
            ctx,
            mode=OrchestrationMode.PIPELINE,
            provided_context_or_code=audit_target,
            argon_res=argon_res,
            sonnet_res=sonnet_res,
            codex_res=codex_res,
            codex_verdict=codex_verdict,
            resumed_roles=sorted(resumed.keys()) if resumed else None,
            repair_rounds_executed=repair_rounds,
            bridge_executed_gates=bridge_gates,
        )

    async def orchestrate_triad(
        self,
        req: TriadOrchestrationRequest,
        job_store: JobStore,
        handoff_store: HandoffStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
    ) -> str:
        """Orchestrate Gemini 4 Argon + Claude Sonnet 5.5 + OpenAI Codex with explicit role states.

        Args:
            req: Validated TriadOrchestrationRequest bundle.
            job_store: Active JobStore instance.
            handoff_store: Active HandoffStore instance.
            run_agy_fn: Antigravity RPC invocation callable.
            execute_fn: Engine execution callable.

        Returns:
            Pretty-printed JSON string containing the orchestration outcome and evidence scope.
        """
        ctx = await asyncio.to_thread(
            _TriadExecutionContext.from_inputs,
            req,
            job_store,
            handoff_store,
            run_agy_fn,
            execute_fn,
        )
        if ctx.normalized_mode == OrchestrationMode.COUNCIL:
            return await self._execute_council_roles(ctx)
        return await self._execute_pipeline_roles(ctx)
