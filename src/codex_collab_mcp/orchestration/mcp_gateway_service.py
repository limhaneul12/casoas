"""MCP execution gateway for routing validation, background jobs, and Claude/Codex dispatch."""

from __future__ import annotations

import asyncio
import json
import uuid
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime

from codex_collab_mcp.collab_enums import EngineKind, JobStatusKind, SchemaName
from codex_collab_mcp.evidence.codegraph_slicer import slice_codegraph_symbols
from codex_collab_mcp.evidence.scope_builder import attach_evidence_scope
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.orchestration.service_dicts import (
    BackgroundJobDispatchRequest,
    ClaudeCraftRequest,
    CodexSchemaToolRequest,
    EngineDispatchRequest,
    ServerGatewayBindings,
)
from codex_collab_mcp.prompts import build_claude_craft_prompt
from codex_collab_mcp.routing.auth_redaction import redact_sensitive_text
from codex_collab_mcp.routing.policy_engine import (
    build_rejected_pre_dispatch_response,
    validate_and_resolve_route,
)
from codex_collab_mcp.schemas import (
    ADVERSARIAL_AUDIT_SCHEMA,
    DIFFERENTIAL_CANDIDATES_SCHEMA,
    RESCUE_DIAGNOSIS_SCHEMA,
    TARGETED_SYNTHESIS_SCHEMA,
    JsonObject,
    as_json_object,
    validate_structured_output,
)

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]

_CODEX_TOOL_SCHEMAS: dict[str, tuple[str, JsonObject]] = {
    SchemaName.ADVERSARIAL_AUDIT: (SchemaName.ADVERSARIAL_AUDIT, ADVERSARIAL_AUDIT_SCHEMA),
    SchemaName.TARGETED_SYNTHESIS: (SchemaName.TARGETED_SYNTHESIS, TARGETED_SYNTHESIS_SCHEMA),
    SchemaName.DIFFERENTIAL_CANDIDATES: (
        SchemaName.DIFFERENTIAL_CANDIDATES,
        DIFFERENTIAL_CANDIDATES_SCHEMA,
    ),
    "rescue_sandbox": (SchemaName.RESCUE_DIAGNOSIS, RESCUE_DIAGNOSIS_SCHEMA),
}


class McpExecutionGateway:
    """Cohesive execution gateway for MCP tool dispatch and background jobs."""

    def __init__(
        self,
        background_tasks: set[asyncio.Task[None]],
        background_job_semaphore: asyncio.Semaphore,
    ) -> None:
        """Initialize the execution gateway with shared background task state.

        Args:
            background_tasks: Active background task set owned by the server module.
            background_job_semaphore: Concurrency semaphore bounding background jobs.
        """
        self._background_tasks = background_tasks
        self._background_job_semaphore = background_job_semaphore

    def apply_boundary_schema_validation(self, result: JsonObject, schema_name: str) -> JsonObject:
        """Validate structured output against its closed-boundary Pydantic model when present.

        Args:
            result: Raw execution response dictionary from a model transport.
            schema_name: Registered schema identifier for boundary validation.

        Returns:
            Updated execution response dictionary with `schema_valid` and normalized output.
        """
        struct_dict = result.get("structured_output")
        if isinstance(struct_dict, dict):
            is_valid, normalized, _err = validate_structured_output(schema_name, struct_dict)
            result["schema_valid"] = is_valid
            if is_valid and normalized is not None:
                result["structured_output"] = normalized
        return result

    async def execute_with_engine(
        self,
        req: EngineDispatchRequest,
        bindings: ServerGatewayBindings,
    ) -> JsonObject:
        """Validate `(provider, engine, model, reasoning_effort)` pre-dispatch and execute.

        Args:
            req: Validated synchronous engine dispatch request DTO.
            bindings: Runtime JobStore and engine transport callables.

        Returns:
            Execution result dictionary with receipt, status, and validated output.
        """
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        route = validate_and_resolve_route(
            engine=req.engine,
            model=req.model,
            provider=req.provider,
            reasoning_effort=req.reasoning_effort,
            enforce_claude_medium_policy=True,
        )
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (req.request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            call_path = (
                f"server._execute_with_engine(engine='{req.engine}') -> REJECTED_PRE_DISPATCH"
            )
            auth_b: JsonObject = {
                "auth_context": route.auth_context,
                "antigravity_rpc_affected": False,
                "auto_credential_refresh_performed": False,
            }
            return build_rejected_pre_dispatch_response(
                req_id=req_id,
                idempotency_key=req.idempotency_key,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                prompt_text=req.prompt,
                max_input_bytes=req.max_input_bytes,
                timeout_seconds=req.timeout_seconds,
                extra_fields={"auth_boundary": auth_b},
            )

        if route.engine == EngineKind.ANTIGRAVITY_LOCAL_RPC:
            result = await bindings.run_agy_fn(
                prompt=req.prompt,
                model=route.resolved_model,
                expect_json=req.output_schema is not None,
                timeout_seconds=req.timeout_seconds,
                provider=route.provider,
                request_id=req.request_id,
                idempotency_key=req.idempotency_key,
                retry_same_request=req.retry_same_request,
                max_input_bytes=req.max_input_bytes,
                workspace_dir=req.workspace_dir or ".",
                job_store=bindings.job_store,
            )
        elif route.engine == EngineKind.HTTP:
            result = await bindings.run_http_fn(
                prompt=req.prompt,
                schema_name=req.schema_name,
                output_schema=req.output_schema,
                model=route.resolved_model,
                timeout_seconds=req.timeout_seconds,
                request_id=req.request_id,
                idempotency_key=req.idempotency_key,
                retry_same_request=req.retry_same_request,
                max_input_bytes=req.max_input_bytes,
                workspace_dir=req.workspace_dir or ".",
                job_store=bindings.job_store,
            )
        else:
            result = await bindings.run_codex_fn(
                prompt=req.prompt,
                workspace_dir=req.workspace_dir,
                output_schema=req.output_schema,
                sandbox_mode=req.sandbox_mode,
                model=req.model,
                reasoning_effort=req.reasoning_effort,
                timeout_seconds=req.timeout_seconds,
                provider=route.provider,
                request_id=req.request_id,
                idempotency_key=req.idempotency_key,
                retry_same_request=req.retry_same_request,
                max_input_bytes=req.max_input_bytes,
                job_store=bindings.job_store,
            )
        return self.apply_boundary_schema_validation(result, req.schema_name)

    def spawn_background_job(
        self,
        req: BackgroundJobDispatchRequest,
        job_store: JobStore,
        execute_fn: AsyncEngineCallable,
    ) -> JsonObject:
        """Validate route pre-dispatch and launch a background job or return existing handle.

        Args:
            req: Validated asynchronous background job dispatch request DTO.
            job_store: Active JobStore instance for job tracking.
            execute_fn: Engine execution callable to invoke inside the background task.

        Returns:
            Dictionary describing the background job handle or pre-dispatch rejection.
        """
        ws_dir = req.workspace_dir or "."

        route = validate_and_resolve_route(
            engine=req.engine,
            model=req.model,
            provider=req.provider,
            reasoning_effort=req.reasoning_effort,
            enforce_claude_medium_policy=True,
        )
        if not route.valid:
            now_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (req.request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            return build_rejected_pre_dispatch_response(
                req_id=req_id,
                idempotency_key=req.idempotency_key,
                route=route,
                call_path="server._spawn_background_job -> REJECTED_PRE_DISPATCH",
                started_iso=now_iso,
                ended_iso=now_iso,
                extra_fields={"async_dispatched": False},
            )

        record, should_dispatch = job_store.resolve_or_register_request(
            task_type=req.task_type,
            workspace_dir=ws_dir,
            model=route.resolved_model,
            provider=route.provider,
            engine=route.engine,
            request_id=req.request_id,
            idempotency_key=req.idempotency_key,
            retry_same_request=req.retry_same_request,
            metadata={"sandbox_mode": req.sandbox_mode, "engine": route.engine},
        )
        if not should_dispatch:
            return {
                "async_dispatched": False,
                "deduplicated_existing_request": True,
                "job_id": record.job_id,
                "request_id": record.job_id,
                "status": record.status,
                "outcome_state": record.outcome_state,
                "attempt_count": record.attempt_count,
                "task_type": req.task_type,
                "receipt": as_json_object(record.receipt) if record.receipt else None,
            }

        async def _runner() -> None:
            """Execute the background job task under the concurrency semaphore."""
            try:
                async with self._background_job_semaphore:
                    res = await execute_fn(
                        prompt=req.prompt,
                        schema_name=req.schema_name,
                        output_schema=req.output_schema,
                        workspace_dir=ws_dir,
                        sandbox_mode=req.sandbox_mode,
                        model=req.model,
                        reasoning_effort=req.reasoning_effort,
                        engine=req.engine,
                        timeout_seconds=req.timeout_seconds,
                        provider=req.provider,
                        request_id=record.job_id,
                        idempotency_key=req.idempotency_key,
                        retry_same_request=True,
                        max_input_bytes=req.max_input_bytes,
                    )
                if res.get("status") == JobStatusKind.REJECTED_PRE_DISPATCH:
                    raw_receipt = res.get("receipt")
                    receipt_obj = raw_receipt if isinstance(raw_receipt, dict) else None
                    job_store.fail_job(
                        record.job_id,
                        error=str(res.get("error") or "Rejected pre-dispatch"),
                        receipt=receipt_obj,
                    )
            except asyncio.CancelledError:
                job_store.cancel_job(record.job_id, reason="Background task cancelled.")
                raise
            except Exception as exc:
                job_store.fail_job(record.job_id, error=redact_sensitive_text(str(exc)))

        task = asyncio.create_task(_runner())
        self._background_tasks.add(task)
        task.add_done_callback(self._background_tasks.discard)

        msg = (
            f"Background Codex job '{record.job_id}' started. "
            f"Query `codex_job_status(job_id='{record.job_id}')` to retrieve the result."
        )
        return {
            "async_dispatched": True,
            "job_id": record.job_id,
            "request_id": record.job_id,
            "status": record.status,
            "outcome_state": record.outcome_state,
            "task_type": req.task_type,
            "message": msg,
        }

    async def dispatch_codex_schema_tool(
        self,
        req: CodexSchemaToolRequest,
        job_store: JobStore,
        execute_fn: AsyncEngineCallable,
    ) -> str:
        """Dispatch a structured Codex schema tool synchronously or in the background.

        Args:
            req: Validated request bundle for Codex schema tool execution.
            job_store: Active JobStore instance for job tracking.
            execute_fn: Engine execution callable to invoke.

        Returns:
            Pretty-printed JSON string containing the execution result or job handle.
        """
        schema_name, output_schema = _CODEX_TOOL_SCHEMAS[req.task_type]
        sliced = (
            await asyncio.to_thread(
                slice_codegraph_symbols, req.workspace_dir, req.symbols_inspected
            )
            if req.symbols_inspected
            else ""
        )
        effective_prompt = (
            f"{req.prompt}\n\n[CodeGraph Sliced Symbol Context]:\n{sliced}"
            if sliced
            else req.prompt
        )

        if req.async_background:
            bg_req = BackgroundJobDispatchRequest(
                task_type=req.task_type,
                prompt=effective_prompt,
                schema_name=schema_name,
                output_schema=output_schema,
                workspace_dir=req.workspace_dir,
                sandbox_mode=req.sandbox_mode,
                model=req.model,
                reasoning_effort=req.reasoning_effort,
                engine=req.engine,
                timeout_seconds=req.timeout_seconds,
                provider=req.provider,
                request_id=req.request_id,
                idempotency_key=req.idempotency_key,
                retry_same_request=req.retry_same_request,
                max_input_bytes=req.max_input_bytes,
            )
            handle = self.spawn_background_job(bg_req, job_store, execute_fn)
            return json.dumps(handle, ensure_ascii=False, indent=2)

        result = await execute_fn(
            prompt=effective_prompt,
            schema_name=schema_name,
            output_schema=output_schema,
            workspace_dir=req.workspace_dir,
            sandbox_mode=req.sandbox_mode,
            model=req.model,
            reasoning_effort=req.reasoning_effort,
            engine=req.engine,
            timeout_seconds=req.timeout_seconds,
            provider=req.provider,
            request_id=req.request_id,
            idempotency_key=req.idempotency_key,
            retry_same_request=req.retry_same_request,
            max_input_bytes=req.max_input_bytes,
        )
        attach_evidence_scope(
            result,
            workspace_dir=req.workspace_dir,
            provided_context_or_code=req.provided_context_or_code,
            files_inspected=req.files_inspected,
            symbols_inspected=req.symbols_inspected,
            executed_test_commands=req.executed_test_commands,
        )
        return json.dumps(result, ensure_ascii=False, indent=2)

    async def run_claude_code_craft(
        self,
        req: ClaudeCraftRequest,
        job_store: JobStore,
        run_agy_fn: AsyncEngineCallable,
    ) -> str:
        """Invoke Claude Sonnet 5.5 or Opus 5.5 (Medium tier) and attach evidence scope.

        Args:
            req: Validated request bundle for Claude surgical code craft.
            job_store: Active JobStore instance for job tracking.
            run_agy_fn: Antigravity local RPC execution callable.

        Returns:
            Pretty-printed JSON string containing the Claude craft result and receipt.
        """
        sliced = (
            await asyncio.to_thread(
                slice_codegraph_symbols, req.workspace_dir, req.symbols_inspected
            )
            if req.symbols_inspected
            else ""
        )
        effective_ctx = (
            f"{req.target_code_or_context}\n\n[CodeGraph Sliced Symbol Context]:\n{sliced}"
            if sliced
            else req.target_code_or_context
        )
        prompt = build_claude_craft_prompt(
            task_spec=req.task_spec,
            target_code_or_context=effective_ctx,
            invariants_and_rules=req.invariants_and_rules,
            language=req.language,
            claude_model=req.claude_model,
        )
        res = await run_agy_fn(
            prompt=prompt,
            model=req.claude_model,
            expect_json=True,
            timeout_seconds=req.timeout_seconds,
            request_id=req.request_id,
            idempotency_key=req.idempotency_key,
            retry_same_request=req.retry_same_request,
            max_input_bytes=req.max_input_bytes,
            workspace_dir=req.workspace_dir,
            job_store=job_store,
        )
        self.apply_boundary_schema_validation(res, SchemaName.CLAUDE_CRAFT)
        attach_evidence_scope(
            res,
            workspace_dir=req.workspace_dir,
            provided_context_or_code=f"{req.task_spec}\n{req.target_code_or_context}",
            files_inspected=req.files_inspected,
            symbols_inspected=req.symbols_inspected,
            executed_test_commands=req.executed_test_commands,
        )
        return json.dumps(res, ensure_ascii=False, indent=2)
