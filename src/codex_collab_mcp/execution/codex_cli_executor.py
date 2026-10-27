"""Core Codex CLI subprocess execution (`codex exec`) and lifecycle management."""

from __future__ import annotations

import asyncio
import os
import tempfile
import time
import uuid
from datetime import UTC, datetime

from codex_collab_mcp.contracts import (
    build_execution_receipt,
    build_input_budget_exceeded_response,
    build_rejected_pre_dispatch_response,
    classify_stderr_and_auth_boundaries,
    evaluate_budget_envelope,
    redact_sensitive_text,
    resolve_workspace_dir,
    validate_and_resolve_route,
)
from codex_collab_mcp.execution.codex_output_parser import (
    CodexCommandBuilder,
    CodexOutputParser,
    ResolveBinaryFn,
)
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.schemas import JsonObject, JsonValue

__all__ = [
    "CodexCliProcessRunner",
    "CodexCommandBuilder",
    "CodexOutputParser",
    "ResolveBinaryFn",
]


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
                proc.kill()
                await proc.wait()
            raise

    @classmethod
    async def run_exec(
        cls,
        prompt: str,
        workspace_dir: str | None = None,
        output_schema: JsonObject | None = None,
        sandbox_mode: str = "read-only",
        model: str | None = None,
        reasoning_effort: str | None = None,
        timeout_seconds: float = 300.0,
        *,
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
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        started_mono = time.monotonic()
        resolved_cwd = str(resolve_workspace_dir(workspace_dir or os.getcwd()))
        call_path = "runner.run_codex_exec -> subprocess(codex exec --json)"
        default_auth = CodexCommandBuilder.build_auth_boundary()

        route = validate_and_resolve_route("codex_cli", model, provider, reasoning_effort)
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            return build_rejected_pre_dispatch_response(
                req_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                extra_fields={
                    "workspace_dir": resolved_cwd,
                    "model": model or "codex-default",
                    "auth_boundary": default_auth,
                },
            )

        pre_budget = evaluate_budget_envelope(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=0.0,
            termination_reason="completed",
        )
        if bool(pre_budget.get("input_budget_exceeded")):
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            return build_input_budget_exceeded_response(
                req_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                pre_budget=pre_budget,
                max_input_bytes=max_input_bytes,
                extra_fields={
                    "workspace_dir": resolved_cwd,
                    "model": route.resolved_model,
                },
            )

        attempt_count = 1
        if job_store is not None:
            record, should_dispatch = job_store.resolve_or_register_request(
                task_type="codex_cli_exec",
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
                    record,
                    route.provider,
                    route.engine,
                    resolved_cwd,
                    route.resolved_model,
                )
        else:
            req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"

        codex_bin = CodexCommandBuilder.resolve_binary(resolve_binary_fn)

        with tempfile.TemporaryDirectory(prefix="codex_mcp_") as tmp_dir:
            output_file, schema_file_str = CodexOutputParser.prepare_temp_files(
                tmp_dir,
                output_schema,
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
            )

            try:
                stdout_bytes, stderr_bytes = await cls.communicate_with_cleanup(
                    proc,
                    prompt.encode("utf-8"),
                    timeout_seconds,
                )
            except TimeoutError:
                ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
                duration = round(time.monotonic() - started_mono, 2)
                timeout_msg = (
                    f"Codex execution timed out after {duration}s (limit {timeout_seconds}s) "
                    f"in {resolved_cwd}. Provider accepted request '{req_id}', so outcome is "
                    f"unknown. Query `codex_job_status(job_id='{req_id}')` before retrying."
                )
                receipt = build_execution_receipt(
                    request_id=req_id,
                    idempotency_key=idempotency_key,
                    route=route,
                    actual_model=route.resolved_model,
                    call_path=call_path,
                    started_at_iso=started_iso,
                    ended_at_iso=ended_iso,
                    duration_seconds=duration,
                    status="timed_out_unknown",
                    outcome_state="outcome_unknown",
                    attempt_count=attempt_count,
                    result_ref=f"job://{req_id}",
                    failure_reason=timeout_msg,
                )
                budget = evaluate_budget_envelope(
                    prompt_text=prompt,
                    max_input_bytes=max_input_bytes,
                    timeout_seconds=timeout_seconds,
                    elapsed_seconds=duration,
                    termination_reason="timeout_budget_reached",
                )
                if job_store is not None:
                    job_store.mark_timed_out_unknown(
                        req_id,
                        error=timeout_msg,
                        partial_result={
                            "request_id": req_id,
                            "status": "timed_out_unknown",
                            "outcome_state": "outcome_unknown",
                        },
                        receipt=receipt,
                    )
                return {
                    "ok": False,
                    "status": "timed_out_unknown",
                    "outcome_state": "outcome_unknown",
                    "request_id": req_id,
                    "job_id": req_id,
                    "provider": route.provider,
                    "engine": "codex_cli",
                    "workspace_dir": resolved_cwd,
                    "model": route.resolved_model,
                    "duration_seconds": duration,
                    "error": timeout_msg,
                    "receipt": receipt,
                    "budget": budget,
                    "auth_boundary": default_auth,
                }

            stdout_text = stdout_bytes.decode("utf-8", errors="replace")
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            session_id, event_messages, reported_model = (
                CodexOutputParser.parse_jsonl_events_with_model(stdout_text)
            )
            auth_diag = classify_stderr_and_auth_boundaries(stderr_text)
            sanitized_stderr = str(auth_diag.get("sanitized_stderr", ""))
            raw_response = CodexOutputParser.resolve_raw_response(
                output_file,
                event_messages,
                stdout_text,
            )

            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            actual_model = reported_model or route.resolved_model
            exit_ok = proc.returncode == 0
            failure_reason: str | None = (
                None
                if exit_ok
                else redact_sensitive_text(
                    sanitized_stderr or raw_response or f"codex exec exited with {proc.returncode}"
                )
            )

            receipt = build_execution_receipt(
                request_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                actual_model=actual_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status="succeeded" if exit_ok else "failed",
                outcome_state="confirmed_success" if exit_ok else "confirmed_failure",
                attempt_count=attempt_count,
                output_text=raw_response if exit_ok else None,
                result_ref=f"job://{req_id}",
                failure_reason=failure_reason,
            )
            model_mismatch = bool(receipt.get("model_mismatch"))
            final_ok = exit_ok and not model_mismatch

            parsed_data: JsonValue = (
                CodexOutputParser.parse_structured_response(raw_response)
                if (output_schema is not None and raw_response)
                else raw_response
            )
            structured_obj: JsonObject | None = (
                parsed_data if isinstance(parsed_data, dict) else None
            )
            term_reason = (
                "model_mismatch"
                if model_mismatch
                else ("completed" if exit_ok else "process_exit_error")
            )
            budget = evaluate_budget_envelope(
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=duration,
                termination_reason=term_reason,
            )

            payload: JsonObject = {
                "ok": final_ok,
                "status": "succeeded" if final_ok else "failed",
                "outcome_state": "confirmed_success" if final_ok else "confirmed_failure",
                "request_id": req_id,
                "job_id": req_id,
                "provider": route.provider,
                "engine": "codex_cli",
                "exit_code": proc.returncode,
                "session_id": session_id,
                "workspace_dir": resolved_cwd,
                "model": actual_model,
                "requested_model": route.requested_model,
                "duration_seconds": duration,
                "data": parsed_data,
                "structured_output": structured_obj,
                "stderr": sanitized_stderr if not final_ok else "",
                "receipt": receipt,
                "budget": budget,
                "auth_boundary": CodexCommandBuilder.build_auth_boundary(
                    auth_diag.get("isolated_external_mcp_oauth_warnings", [])
                ),
            }
            if not final_ok:
                err_str = str(
                    receipt.get("failure_reason") or failure_reason or "Codex CLI failed."
                )
                payload["error"] = err_str
                if model_mismatch:
                    payload["error_code"] = "ACTUAL_MODEL_MISMATCH"
                if job_store is not None:
                    job_store.fail_job(req_id, err_str, session_id=session_id, receipt=receipt)
            elif job_store is not None:
                job_store.complete_job(req_id, payload, session_id=session_id, receipt=receipt)

            return payload
