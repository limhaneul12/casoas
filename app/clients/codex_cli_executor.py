"""Core Codex CLI subprocess execution (`codex exec`) and lifecycle management."""

from __future__ import annotations

import asyncio
import contextlib
import os
import signal
import tempfile
import time
import uuid
from datetime import UTC, datetime

from app.clients.codex_output_parser import (
    CodexCommandBuilder,
    CodexOutputParser,
    ResolveBinaryFn,
)
from app.core.collab_enums import (
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    OutcomeStateKind,
)
from app.core.model_contract_enums import (
    BudgetTerminationReason,
    RoutingErrorCode,
    SandboxModeKind,
    TaskTypeKind,
)
from app.evidence.workspace_digests import WorkspaceDigestCalculator
from app.repositories.job_store import JobStore
from app.routing.auth_redaction import (
    classify_stderr_and_auth_boundaries,
    redact_sensitive_text,
)
from app.routing.policy_engine import RoutingPolicyEngine
from app.routing.receipt_budget import BudgetEnvelope
from app.schemas.boundary_schemas import JsonObject, JsonValue, as_json_object
from app.schemas.inner_dto.execution_result_dtos import (
    CodexBudgetExtraFields,
    CodexExecCompletedResult,
    CodexExecTimeoutResult,
    CodexRejectedExtraFields,
    CodexTimeoutPartialResult,
)


class CodexCliProcessRunner:
    """Subprocess lifecycle manager and command runner for `codex exec` and `codex review`."""

    @staticmethod
    async def communicate_with_cleanup(
        proc: asyncio.subprocess.Process,
        input_bytes: bytes | None,
        timeout_seconds: float,
    ) -> tuple[bytes, bytes]:
        """Communicate with a subprocess under a timeout, killing and reaping on timeout/cancel.

        Args:
            proc: Active asyncio subprocess instance.
            input_bytes: Optional UTF-8 encoded stdin bytes to send to the subprocess.
            timeout_seconds: Maximum execution window in seconds.

        Returns:
            Tuple of `(stdout_bytes, stderr_bytes)` from the completed subprocess.
        """
        try:
            return await asyncio.wait_for(
                proc.communicate(input=input_bytes),
                timeout=timeout_seconds,
            )
        except BaseException:
            if proc.returncode is None:
                if proc.pid is not None:
                    with contextlib.suppress(ProcessLookupError, PermissionError, OSError):
                        os.killpg(proc.pid, signal.SIGKILL)
                with contextlib.suppress(ProcessLookupError):
                    proc.kill()
                await proc.wait()
            raise

    @classmethod
    async def run_exec(
        cls,
        prompt: str,
        workspace_dir: str | None = None,
        output_schema: JsonObject | None = None,
        sandbox_mode: str = SandboxModeKind.READ_ONLY,
        model: str | None = None,
        reasoning_effort: str | None = None,
        timeout_seconds: float = 300.0,
        provider: str | None = None,
        request_id: str | None = None,
        idempotency_key: str | None = None,
        retry_same_request: bool = False,
        max_input_bytes: int = 256_000,
        job_store: JobStore | None = None,
        resolve_binary_fn: ResolveBinaryFn | None = None,
    ) -> JsonObject:
        """Execute a non-interactive Codex task via `codex exec` with verified routing and receipts.

        Args:
            prompt: Task prompt piped to `codex exec` over stdin.
            workspace_dir: Target workspace directory path.
            output_schema: Optional strict JSON schema for structured output.
            sandbox_mode: Sandbox mode passed to `--sandbox`.
            model: Optional Codex model identifier.
            reasoning_effort: Optional model reasoning effort level.
            timeout_seconds: Maximum execution time in seconds before timeout handling.
            provider: Optional explicit provider name for route verification.
            request_id: Optional caller-specified request ID (`cdx-...`).
            idempotency_key: Optional idempotency key for deduplicating dispatches.
            retry_same_request: Whether to re-dispatch an existing idempotency key.
            max_input_bytes: Maximum UTF-8 byte budget for `prompt`.
            job_store: Optional `JobStore` instance for durable state persistence.
            resolve_binary_fn: Optional callable used to resolve the `codex` binary path.

        Returns:
            Execution result dictionary with receipt, budget envelope, and structured output.
        """
        policy_engine = RoutingPolicyEngine()
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        started_mono = time.monotonic()
        resolved_cwd = str(
            WorkspaceDigestCalculator().resolve_dir(workspace_dir=workspace_dir or os.getcwd())
        )
        call_path = "runner.run_codex_exec -> subprocess(codex exec --json)"
        default_auth = as_json_object(record=CodexCommandBuilder.build_auth_boundary())

        route = policy_engine.validate_route(
            engine=str(EngineKind.CODEX_CLI),
            model=model,
            provider=provider,
            reasoning_effort=reasoning_effort,
        )
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            return policy_engine.build_rejected_response(
                req_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                extra_fields=as_json_object(
                    record=CodexRejectedExtraFields(
                        workspace_dir=resolved_cwd,
                        model=model or str(CanonicalModelId.CODEX_DEFAULT),
                        auth_boundary=default_auth,
                    ).to_dict()
                ),
            )

        pre_budget = BudgetEnvelope.evaluate(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=0.0,
            termination_reason=str(BudgetTerminationReason.COMPLETED),
        )
        if pre_budget["input_budget_exceeded"]:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            return policy_engine.build_budget_exceeded_response(
                req_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                pre_budget=pre_budget,
                max_input_bytes=max_input_bytes,
                extra_fields=as_json_object(
                    record=CodexBudgetExtraFields(
                        workspace_dir=resolved_cwd,
                        model=route.resolved_model,
                    ).to_dict()
                ),
            )

        attempt_count = 1
        if job_store is not None:
            record, should_dispatch = job_store.resolve_or_register_request(
                task_type=str(TaskTypeKind.CODEX_CLI_EXEC),
                workspace_dir=resolved_cwd,
                model=route.resolved_model,
                provider=route.provider,
                engine=route.engine,
                request_id=request_id,
                idempotency_key=idempotency_key,
                retry_same_request=retry_same_request,
                metadata={"sandbox_mode": sandbox_mode},
            )
            req_id = record.job_id
            attempt_count = record.attempt_count
            if not should_dispatch:
                return CodexCommandBuilder.build_deduplicated_response(
                    record=record,
                    provider=route.provider,
                    engine=route.engine,
                    resolved_cwd=resolved_cwd,
                    resolved_model=route.resolved_model,
                )
        else:
            req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"

        try:
            codex_bin = CodexCommandBuilder.resolve_binary(resolve_binary_fn=resolve_binary_fn)
        except Exception as exc:
            if job_store is not None:
                job_store.fail_job(job_id=req_id, error=redact_sensitive_text(text=str(exc)))
            raise

        with tempfile.TemporaryDirectory(prefix="codex_mcp_") as tmp_dir:
            try:
                output_file, schema_file_str = CodexOutputParser.prepare_temp_files(
                    tmp_dir=tmp_dir,
                    output_schema=output_schema,
                )

                cmd = CodexCommandBuilder.build_exec_command(
                    codex_bin=codex_bin,
                    workspace_dir=resolved_cwd,
                    output_file=str(output_file),
                    schema_file=schema_file_str,
                    sandbox_mode=sandbox_mode,
                    model=model,
                    reasoning_effort=reasoning_effort,
                )

                proc = await asyncio.create_subprocess_exec(
                    *cmd,
                    stdin=asyncio.subprocess.PIPE,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE,
                    cwd=resolved_cwd,
                    start_new_session=True,
                )
            except asyncio.CancelledError:
                if job_store is not None:
                    job_store.cancel_job(job_id=req_id, reason="Codex CLI execution cancelled.")
                raise
            except Exception as exc:
                if job_store is not None:
                    job_store.fail_job(job_id=req_id, error=redact_sensitive_text(text=str(exc)))
                raise

            try:
                stdout_bytes, stderr_bytes = await cls.communicate_with_cleanup(
                    proc=proc,
                    input_bytes=prompt.encode("utf-8"),
                    timeout_seconds=timeout_seconds,
                )
            except asyncio.CancelledError:
                if job_store is not None:
                    job_store.cancel_job(job_id=req_id, reason="Codex CLI execution cancelled.")
                raise
            except TimeoutError:
                ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
                duration = round(time.monotonic() - started_mono, 2)
                timeout_msg = (
                    f"Codex execution timed out after {duration}s (limit {timeout_seconds}s) "
                    f"in {resolved_cwd}. Provider accepted request '{req_id}', so outcome is "
                    f"unknown. Query `codex_job_status(job_id='{req_id}')` before retrying."
                )
                receipt = policy_engine.build_receipt(
                    request_id=req_id,
                    idempotency_key=idempotency_key,
                    route=route,
                    actual_model=route.resolved_model,
                    call_path=call_path,
                    started_at_iso=started_iso,
                    ended_at_iso=ended_iso,
                    duration_seconds=duration,
                    status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                    outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
                    attempt_count=attempt_count,
                    result_ref=f"job://{req_id}",
                    failure_reason=timeout_msg,
                )
                budget = BudgetEnvelope.evaluate(
                    prompt_text=prompt,
                    max_input_bytes=max_input_bytes,
                    timeout_seconds=timeout_seconds,
                    elapsed_seconds=duration,
                    termination_reason=str(BudgetTerminationReason.TIMEOUT_BUDGET_REACHED),
                )
                if job_store is not None:
                    job_store.mark_timed_out_unknown(
                        job_id=req_id,
                        error=timeout_msg,
                        partial_result=as_json_object(
                            record=CodexTimeoutPartialResult(
                                request_id=req_id,
                                status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                                outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
                            ).to_dict()
                        ),
                        receipt=receipt,
                    )
                return as_json_object(
                    record=CodexExecTimeoutResult(
                        ok=False,
                        status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                        outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
                        request_id=req_id,
                        job_id=req_id,
                        provider=route.provider,
                        engine=str(EngineKind.CODEX_CLI),
                        workspace_dir=resolved_cwd,
                        model=route.resolved_model,
                        duration_seconds=duration,
                        error=timeout_msg,
                        receipt=as_json_object(record=receipt),
                        budget=as_json_object(record=budget),
                        auth_boundary=default_auth,
                    ).to_dict()
                )

            stdout_text = stdout_bytes.decode("utf-8", errors="replace")
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            session_id, event_messages, reported_model = (
                CodexOutputParser.parse_jsonl_events_with_model(stdout_text=stdout_text)
            )
            auth_diag = classify_stderr_and_auth_boundaries(stderr_text=stderr_text)
            sanitized_stderr = auth_diag["sanitized_stderr"]
            raw_response = CodexOutputParser.resolve_raw_response(
                output_file=output_file,
                event_messages=event_messages,
                stdout_text=stdout_text,
            )

            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            actual_model = reported_model or route.resolved_model
            exit_ok = proc.returncode == 0
            failure_reason: str | None = (
                None
                if exit_ok
                else redact_sensitive_text(
                    text=(
                        sanitized_stderr
                        or raw_response
                        or f"codex exec exited with {proc.returncode}"
                    )
                )
            )

            receipt = policy_engine.build_receipt(
                request_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                actual_model=actual_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status=str(JobStatusKind.SUCCEEDED) if exit_ok else str(JobStatusKind.FAILED),
                outcome_state=(
                    str(OutcomeStateKind.CONFIRMED_SUCCESS)
                    if exit_ok
                    else str(OutcomeStateKind.CONFIRMED_FAILURE)
                ),
                attempt_count=attempt_count,
                output_text=raw_response if exit_ok else None,
                result_ref=f"job://{req_id}",
                failure_reason=failure_reason,
            )
            model_mismatch = receipt["model_mismatch"]
            final_ok = exit_ok and not model_mismatch

            parsed_data: JsonValue = (
                CodexOutputParser.parse_structured_response(raw_response=raw_response)
                if (output_schema is not None and raw_response)
                else raw_response
            )
            structured_obj: JsonObject | None = (
                parsed_data if isinstance(parsed_data, dict) else None
            )
            term_reason = (
                str(BudgetTerminationReason.MODEL_MISMATCH)
                if model_mismatch
                else (
                    str(BudgetTerminationReason.COMPLETED)
                    if exit_ok
                    else str(BudgetTerminationReason.PROCESS_EXIT_ERROR)
                )
            )
            budget = BudgetEnvelope.evaluate(
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=duration,
                termination_reason=term_reason,
            )

            payload = as_json_object(
                record=CodexExecCompletedResult(
                    ok=final_ok,
                    status=str(JobStatusKind.SUCCEEDED) if final_ok else str(JobStatusKind.FAILED),
                    outcome_state=(
                        str(OutcomeStateKind.CONFIRMED_SUCCESS)
                        if final_ok
                        else str(OutcomeStateKind.CONFIRMED_FAILURE)
                    ),
                    request_id=req_id,
                    job_id=req_id,
                    provider=route.provider,
                    engine=str(EngineKind.CODEX_CLI),
                    exit_code=proc.returncode,
                    session_id=session_id,
                    workspace_dir=resolved_cwd,
                    model=actual_model,
                    requested_model=route.requested_model,
                    duration_seconds=duration,
                    data=parsed_data,
                    structured_output=structured_obj,
                    stderr=sanitized_stderr if not final_ok else "",
                    receipt=as_json_object(record=receipt),
                    budget=as_json_object(record=budget),
                    auth_boundary=as_json_object(
                        record=CodexCommandBuilder.build_auth_boundary(
                            warnings=auth_diag["isolated_external_mcp_oauth_warnings"]
                        )
                    ),
                ).to_dict()
            )
            if not final_ok:
                err_str = receipt["failure_reason"] or failure_reason or "Codex CLI failed."
                payload["error"] = err_str
                if model_mismatch:
                    payload["error_code"] = str(RoutingErrorCode.ACTUAL_MODEL_MISMATCH)
                if job_store is not None:
                    job_store.fail_job(
                        job_id=req_id,
                        error=err_str,
                        session_id=session_id,
                        receipt=receipt,
                    )
            elif job_store is not None:
                job_store.complete_job(
                    job_id=req_id,
                    result=payload,
                    session_id=session_id,
                    receipt=receipt,
                )

            return payload
