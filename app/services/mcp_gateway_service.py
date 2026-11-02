"""MCP execution gateway for routing validation, background jobs, and Claude/Codex dispatch."""

from __future__ import annotations

import asyncio
import json
import threading
import uuid
from collections.abc import Callable, Coroutine
from dataclasses import replace
from datetime import UTC, datetime

from app.core.collab_enums import EngineKind, JobStatusKind, SchemaName
from app.core.model_contract_enums import TaskTypeKind
from app.core.prompts import build_claude_craft_prompt
from app.evidence.codegraph_slicer import CodegraphContextSlicer
from app.evidence.scope_builder import EvidenceScopeBuilder
from app.repositories.job_store import JobStore
from app.routing.auth_redaction import redact_sensitive_text
from app.routing.policy_engine import RoutingPolicyEngine
from app.schemas.boundary_schemas import (
    ADVERSARIAL_AUDIT_SCHEMA,
    DIFFERENTIAL_CANDIDATES_SCHEMA,
    RESCUE_DIAGNOSIS_SCHEMA,
    TARGETED_SYNTHESIS_SCHEMA,
    JsonObject,
    as_json_object,
    validate_structured_output,
)
from app.schemas.inner_dto.contract_dtos import (
    BackgroundJobDeduplicatedResult,
    BackgroundJobDispatchedResult,
    GatewayRejectedAuthBoundary,
)
from app.schemas.inner_dto.service_request_dtos import (
    BackgroundJobDispatchRequest,
    ClaudeCraftRequest,
    CodexSchemaToolRequest,
    EngineDispatchRequest,
    ServerGatewayBindings,
)

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]

_CODEX_TOOL_SCHEMAS: dict[str, tuple[str, JsonObject]] = {
    SchemaName.ADVERSARIAL_AUDIT: (SchemaName.ADVERSARIAL_AUDIT, ADVERSARIAL_AUDIT_SCHEMA),
    SchemaName.TARGETED_SYNTHESIS: (SchemaName.TARGETED_SYNTHESIS, TARGETED_SYNTHESIS_SCHEMA),
    SchemaName.DIFFERENTIAL_CANDIDATES: (
        SchemaName.DIFFERENTIAL_CANDIDATES,
        DIFFERENTIAL_CANDIDATES_SCHEMA,
    ),
    TaskTypeKind.RESCUE_SANDBOX: (SchemaName.RESCUE_DIAGNOSIS, RESCUE_DIAGNOSIS_SCHEMA),
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
        self._tasks_by_job: dict[str, asyncio.Task[None]] = {}
        self._task_lock: threading.RLock = threading.RLock()
        self._policy_engine = RoutingPolicyEngine()
        self._codegraph_slicer = CodegraphContextSlicer()
        self._scope_builder = EvidenceScopeBuilder()

    def cancel_background_task(self, job_id: str) -> bool:
        """Cancel a live in-flight asyncio.Task associated with `job_id` if present.

        Args:
            job_id: Unique job/request identifier whose background task should be cancelled.

        Returns:
            True if a live asyncio.Task was found and scheduled for cancellation, else False.
        """
        jid = job_id.strip()
        if not jid:
            return False
        with self._task_lock:
            task = self._tasks_by_job.get(jid)
        if task is None or task.done():
            return False
        try:
            running_loop = asyncio.get_running_loop()
        except RuntimeError:
            running_loop = None
        task_loop = task.get_loop()
        if running_loop is task_loop:
            task.cancel()
        elif task_loop.is_running():
            task_loop.call_soon_threadsafe(task.cancel)
        else:
            task.cancel()
        return True

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
            is_valid, normalized, _err = validate_structured_output(
                schema_name=schema_name,
                payload=struct_dict,
            )
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
        route = self._policy_engine.validate_route(
            engine=req.engine,
            model=req.model,
            provider=req.provider,
            reasoning_effort=req.reasoning_effort,
            enforce_claude_medium_policy=True,
        )
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (req.request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            c_path = f"server._execute_with_engine(engine='{req.engine}') -> REJECTED_PRE_DISPATCH"
            auth_b = as_json_object(
                record=GatewayRejectedAuthBoundary(
                    auth_context=route.auth_context,
                    antigravity_rpc_affected=False,
                    auto_credential_refresh_performed=False,
                ).to_dict()
            )
            return self._policy_engine.build_rejected_response(
                req_id=req_id,
                idempotency_key=req.idempotency_key,
                route=route,
                call_path=c_path,
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
        return self.apply_boundary_schema_validation(result=result, schema_name=req.schema_name)

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
        route = self._policy_engine.validate_route(
            engine=req.engine,
            model=req.model,
            provider=req.provider,
            reasoning_effort=req.reasoning_effort,
            enforce_claude_medium_policy=True,
        )
        if not route.valid:
            now_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (req.request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            return self._policy_engine.build_rejected_response(
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
            metadata=dict((("sandbox_mode", req.sandbox_mode), ("engine", route.engine))),
        )
        if not should_dispatch:
            rcpt = as_json_object(record=record.receipt) if record.receipt else None
            dedup_dto = BackgroundJobDeduplicatedResult(
                job_id=record.job_id,
                request_id=record.job_id,
                status=record.status,
                outcome_state=record.outcome_state,
                attempt_count=record.attempt_count,
                task_type=req.task_type,
                receipt=rcpt,
            )
            return as_json_object(record=dedup_dto.to_dict())

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
                    r_raw = res.get("receipt")
                    r_obj = r_raw if isinstance(r_raw, dict) else None
                    err_msg = str(res.get("error") or "Rejected pre-dispatch")
                    job_store.fail_job(job_id=record.job_id, error=err_msg, receipt=r_obj)
            except asyncio.CancelledError:
                job_store.cancel_job(job_id=record.job_id, reason="Background task cancelled.")
                raise
            except Exception as exc:
                job_store.fail_job(job_id=record.job_id, error=redact_sensitive_text(str(exc)))

        def _cleanup_task(done_task: asyncio.Task[None]) -> None:
            """Remove the completed background task from active tracking sets.

            Args:
                done_task: Completed or cancelled asyncio.Task instance.
            """
            self._background_tasks.discard(done_task)
            with self._task_lock:
                if self._tasks_by_job.get(record.job_id) is done_task:
                    self._tasks_by_job.pop(record.job_id, None)

        task = asyncio.create_task(_runner())
        with self._task_lock:
            self._tasks_by_job[record.job_id] = task
        self._background_tasks.add(task)
        task.add_done_callback(_cleanup_task)

        msg = f"Background Codex job '{record.job_id}' started. Query `codex_job_status(job_id='{record.job_id}')` to retrieve the result."
        disp_dto = BackgroundJobDispatchedResult(
            job_id=record.job_id,
            request_id=record.job_id,
            status=record.status,
            outcome_state=record.outcome_state,
            task_type=req.task_type,
            message=msg,
        )
        return as_json_object(record=disp_dto.to_dict())

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
        has_cg_inputs = bool(req.symbols_inspected or req.files_inspected or req.provided_context_or_code.strip())
        sliced = (
            await asyncio.to_thread(
                self._codegraph_slicer.format_context_block,
                workspace_dir=req.workspace_dir,
                symbols=req.symbols_inspected,
                files_inspected=req.files_inspected,
                inline_text=req.provided_context_or_code,
            )
            if has_cg_inputs
            else ""
        )
        effective_prompt = f"{req.prompt}\n\n[CodeGraph Sliced Symbol Context]:\n{sliced}" if sliced else req.prompt
        if req.async_background:
            bg_req = BackgroundJobDispatchRequest.from_codex_schema_request(
                req=req,
                prompt=effective_prompt,
                schema_name=schema_name,
                output_schema=output_schema,
            )
            bg_out = self.spawn_background_job(req=bg_req, job_store=job_store, execute_fn=execute_fn)
            return json.dumps(bg_out, ensure_ascii=False, indent=2)

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
        self._scope_builder.attach_scope(
            result=result,
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
        has_cg_inputs = bool(req.symbols_inspected or req.files_inspected or req.target_code_or_context.strip())
        sliced = (
            await asyncio.to_thread(
                self._codegraph_slicer.format_context_block,
                workspace_dir=req.workspace_dir,
                symbols=req.symbols_inspected,
                max_bytes=16_000,
                files_inspected=req.files_inspected,
                inline_text=req.target_code_or_context,
            )
            if has_cg_inputs
            else ""
        )
        effective_ctx = f"{req.target_code_or_context}\n\n[CodeGraph Sliced Symbol Context]:\n{sliced}" if sliced else req.target_code_or_context
        prompt = build_claude_craft_prompt(
            task_spec=req.task_spec,
            target_code_or_context=effective_ctx,
            invariants_and_rules=req.invariants_and_rules,
            language=req.language,
            claude_model=req.claude_model,
        )
        if req.async_background:

            async def _craft_exec(**kw: object) -> JsonObject:
                """Run synchronous Claude craft execution inside a background job.

                Args:
                    **kw: Background dispatch keyword arguments including `request_id`.

                Returns:
                    Parsed execution result dictionary.
                """
                rid = kw.get("request_id")
                eff_rid = rid if isinstance(rid, str) else req.request_id
                s_req = replace(req, async_background=False, request_id=eff_rid, retry_same_request=True)
                raw = await self.run_claude_code_craft(req=s_req, job_store=job_store, run_agy_fn=run_agy_fn)
                parsed = json.loads(raw)
                return parsed if isinstance(parsed, dict) else {}

            bg_req = BackgroundJobDispatchRequest.from_claude_craft_request(req=req, prompt=prompt)
            bg_out = self.spawn_background_job(req=bg_req, job_store=job_store, execute_fn=_craft_exec)
            return json.dumps(bg_out, ensure_ascii=False, indent=2)

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
        self.apply_boundary_schema_validation(result=res, schema_name=SchemaName.CLAUDE_CRAFT)
        self._scope_builder.attach_scope(
            result=res,
            workspace_dir=req.workspace_dir,
            provided_context_or_code=f"{req.task_spec}\n{req.target_code_or_context}",
            files_inspected=req.files_inspected,
            symbols_inspected=req.symbols_inspected,
            executed_test_commands=req.executed_test_commands,
        )
        eff_jid = str(res.get("job_id") or req.request_id or res.get("request_id") or "").strip()
        if res.get("ok") is True and eff_jid and job_store.get_job(eff_jid) is not None:
            job_store.complete_job(job_id=eff_jid, result=res)
        return json.dumps(res, ensure_ascii=False, indent=2)
