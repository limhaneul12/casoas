"""Immutable execution context and pole invocation helpers for Tri-Polar orchestration."""

from __future__ import annotations

import time
from collections.abc import Callable, Coroutine
from dataclasses import dataclass, field
from pathlib import Path

from app.core.collab_enums import (
    JobStatusKind,
    SchemaName,
)
from app.core.model_contract_enums import (
    EngineAliasKind,
    SandboxModeKind,
    TaskTypeKind,
)
from app.core.prompts import build_claude_craft_prompt
from app.evidence.codegraph_slicer import CodegraphContextSlicer
from app.evidence.triad_role_resolver import TriadRoleResolver
from app.evidence.workspace_digests import WorkspaceDigestCalculator
from app.repositories.handoff_store import HandoffStore
from app.repositories.job_store import JobStore
from app.schemas.boundary_schemas import (
    ADVERSARIAL_AUDIT_SCHEMA,
    JsonObject,
    validate_structured_output,
)
from app.schemas.inner_dto.service_request_dtos import TriadOrchestrationRequest

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]


@dataclass(frozen=True, slots=True)
class TriadExecutionContext:
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
    active_job_id: str | None = None
    started_at: float = field(default_factory=time.time)

    @classmethod
    def from_inputs(
        cls,
        req: TriadOrchestrationRequest,
        job_store: JobStore,
        handoff_store: HandoffStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
    ) -> TriadExecutionContext:
        """Validate workspace directory and build the immutable execution context.

        Args:
            req: Validated TriadOrchestrationRequest bundle.
            job_store: Active JobStore instance.
            handoff_store: Active HandoffStore instance.
            run_agy_fn: Antigravity RPC invocation callable.
            execute_fn: Engine execution callable.

        Returns:
            Initialized `TriadExecutionContext` instance with validated workspace and prompts.
        """
        digest_calc = WorkspaceDigestCalculator()
        resolved_workspace = digest_calc.resolve_dir(workspace_dir=req.workspace_dir)
        if resolved_workspace is None or not Path(resolved_workspace).is_dir():
            raise ValueError("Workspace directory must resolve to an existing directory.")
        normalized_mode = req.mode.strip().lower()
        source_task_digest = digest_calc.compute_task_hash(
            task_spec=req.task_spec,
            target_code_or_context=req.target_code_or_context,
            invariants_and_rules=req.invariants_and_rules,
            language=req.language,
            mode=normalized_mode,
        )
        sliced = CodegraphContextSlicer().format_context_block(
            workspace_dir=resolved_workspace,
            symbols=req.symbols_inspected,
            max_bytes=16_000,
            files_inspected=req.files_inspected,
            inline_text=req.target_code_or_context,
        )
        effective_target_code = (
            f"{req.target_code_or_context}\n\n[CodeGraph Sliced Symbol Context]:\n{sliced}"
            if sliced
            else req.target_code_or_context
        )
        rid = (req.request_id or "").strip()
        existing_job = job_store.get_job(rid) if rid else None
        is_active = (
            existing_job is not None
            and existing_job.task_type == TaskTypeKind.TRIAD_ORCHESTRATION
            and existing_job.status == JobStatusKind.RUNNING
        )
        sonnet_prompt = build_claude_craft_prompt(
            task_spec=req.task_spec,
            target_code_or_context=effective_target_code,
            invariants_and_rules=req.invariants_and_rules,
            language=req.language,
            claude_model=req.claude_model,
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
            active_job_id=rid if is_active else None,
        )

    async def invoke_sonnet(self, prompt_override: str | None = None) -> JsonObject:
        """Invoke the Claude Sonnet/Opus craftsman pole with this context's configuration.

        Args:
            prompt_override: Optional custom prompt string (e.g. for Round 2 adjudication).

        Returns:
            Claude role response dictionary.
        """
        res = await self.run_agy_fn(
            prompt=prompt_override or self.sonnet_prompt,
            model=self.req.claude_model,
            expect_json=True,
            timeout_seconds=self.req.timeout_seconds,
            max_input_bytes=self.req.max_input_bytes,
            workspace_dir=self.workspace_dir,
            job_store=self.job_store,
        )
        if isinstance(res, dict) and isinstance(st := res.get("structured_output"), dict):
            res = dict(res)
            is_valid, _, _ = validate_structured_output(
                schema_name=SchemaName.CLAUDE_CRAFT,
                payload=st,
            )
            res["schema_valid"] = is_valid
        return res

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
            sandbox_mode=SandboxModeKind.READ_ONLY,
            model=self.req.codex_model,
            reasoning_effort=self.req.codex_reasoning_effort,
            engine=EngineAliasKind.CLI,
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

    def abort_at_sonnet(self, resolver: TriadRoleResolver, sonnet_res: JsonObject) -> str:
        """Delegate aborted pipeline payload assembly and persistence when Sonnet fails.

        Args:
            resolver: TriadRoleResolver helper instance.
            sonnet_res: Non-succeeded Claude Sonnet/Opus role response dictionary.

        Returns:
            Pretty-printed JSON string representing the aborted pipeline outcome.
        """
        return resolver.build_sonnet_abort_payload(
            sonnet_res=sonnet_res,
            workspace_dir=self.workspace_dir,
            provided_context_or_code=f"{self.req.task_spec}\n{self.effective_target_code}",
            files_inspected=self.req.files_inspected,
            symbols_inspected=self.req.symbols_inspected,
            executed_test_commands=self.req.executed_test_commands,
            source_task_digest=self.source_task_digest,
            job_store=self.job_store,
            started_at=self.started_at,
            collaboration_id_override=self.active_job_id,
            request_id=self.req.request_id,
            idempotency_key=self.req.idempotency_key,
        )

    def finalize_snapshot(
        self,
        resolver: TriadRoleResolver,
        mode: str,
        provided_context_or_code: str,
        argon_res: JsonObject | None,
        sonnet_res: JsonObject,
        codex_res: JsonObject,
        codex_verdict: str | None,
        resumed: dict[str, JsonObject],
        repair_rounds: int | None,
        bridge_gates: list[JsonObject] | None,
    ) -> str:
        """Delegate final triad snapshot assembly and persistence to `resolver`.

        Args:
            resolver: TriadRoleResolver helper instance.
            mode: Normalized orchestration mode (`council` or `pipeline`).
            provided_context_or_code: Inline code or context string for evidence scope.
            argon_res: Gemini 4 Argon role response dictionary, or None.
            sonnet_res: Claude Sonnet/Opus role response dictionary.
            codex_res: OpenAI Codex role response dictionary.
            codex_verdict: Extracted Codex verdict (`PASS`, `WARN`, `BLOCK`, or None).
            resumed: Map of resumed role names to their restored responses.
            repair_rounds: Optional count of auto-repair or adjudication rounds executed.
            bridge_gates: Optional bridge-executed verification gate records.

        Returns:
            Pretty-printed JSON string representing the completed triad payload.
        """
        return resolver.build_completed_triad_payload(
            mode=mode,
            workspace_dir=self.workspace_dir,
            provided_context_or_code=provided_context_or_code,
            source_task_digest=self.source_task_digest,
            argon_model=self.req.argon_model,
            claude_model=self.req.claude_model,
            codex_model=self.req.codex_model,
            argon_res=argon_res,
            sonnet_res=sonnet_res,
            codex_res=codex_res,
            codex_verdict=codex_verdict,
            files_inspected=self.req.files_inspected,
            symbols_inspected=self.req.symbols_inspected,
            executed_test_commands=self.req.executed_test_commands,
            job_store=self.job_store,
            resumed_roles=sorted(resumed.keys()) if resumed else None,
            resume_collaboration_id=self.req.resume_collaboration_id,
            repair_rounds_executed=repair_rounds,
            bridge_executed_gates=bridge_gates,
            started_at=self.started_at,
            collaboration_id_override=self.active_job_id,
            request_id=self.req.request_id,
            idempotency_key=self.req.idempotency_key,
        )
