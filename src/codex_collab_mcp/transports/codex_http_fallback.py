"""Direct HTTP completion fallback runner for OpenAI-compatible local proxy endpoints."""

from __future__ import annotations

import asyncio
import os
import time
import uuid
from datetime import UTC, datetime

import httpx

from codex_collab_mcp.collab_enums import (
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    OutcomeStateKind,
    ProviderKind,
)
from codex_collab_mcp.evidence.workspace_digests import resolve_workspace_dir
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.routing.auth_redaction import redact_sensitive_text
from codex_collab_mcp.routing.policy_engine import (
    build_execution_receipt,
    build_input_budget_exceeded_response,
    build_rejected_pre_dispatch_response,
    validate_and_resolve_route,
)
from codex_collab_mcp.routing.receipt_budget import evaluate_budget_envelope
from codex_collab_mcp.schemas import JsonObject, JsonValue, as_json_object
from codex_collab_mcp.transports.codex_output_parser import CodexCommandBuilder, CodexOutputParser


class CodexHttpFallbackRunner:
    """Runner for direct HTTP `/chat/completions` execution with receipts and budget guards."""

    @classmethod
    async def run_http_fallback(
        cls,
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None = None,
        model: str | None = None,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout_seconds: float = 180.0,
        *,
        request_id: str | None = None,
        idempotency_key: str | None = None,
        retry_same_request: bool = False,
        max_input_bytes: int = 256_000,
        workspace_dir: str | None = None,
        job_store: JobStore | None = None,
    ) -> JsonObject:
        """Direct HTTP execution via local proxy (127.0.0.1:8787/v1) or OpenAI-compatible API.

        Args:
            prompt: User prompt text sent in chat completion messages.
            schema_name: Strict JSON schema name for `response_format`.
            output_schema: Optional JSON schema object for structured responses.
            model: Optional model identifier override.
            base_url: Optional OpenAI-compatible base URL override.
            api_key: Optional bearer token API key override.
            timeout_seconds: Request timeout in seconds.
            request_id: Optional caller-supplied request identifier (`cdx-...`).
            idempotency_key: Optional idempotency key for receipt correlation.
            retry_same_request: Whether to re-dispatch an existing idempotency key.
            max_input_bytes: Maximum UTF-8 byte budget for `prompt`.
            workspace_dir: Optional workspace directory associated with the job record.
            job_store: Optional `JobStore` instance for durable state persistence.

        Returns:
            HTTP completion result dictionary including receipt, budget, and structured output.
        """
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        started_mono = time.monotonic()
        req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
        resolved_cwd = str(resolve_workspace_dir(workspace_dir or os.getcwd()))
        call_path = "runner.run_http_fallback -> POST /chat/completions"
        resolved_url = (
            base_url
            or os.environ.get("CODEX_BASE_URL")
            or os.environ.get("OPENAI_BASE_URL")
            or "http://127.0.0.1:8787/v1"
        ).rstrip("/")
        resolved_key = api_key or os.environ.get("OPENAI_API_KEY") or "headroom-local"
        resolved_model = model or os.environ.get("CODEX_MODEL") or CanonicalModelId.GPT_6_1_SOL

        route = validate_and_resolve_route(EngineKind.HTTP, resolved_model, ProviderKind.LOCAL_HTTP)
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
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
                extra_fields={"model": resolved_model},
            )

        pre_budget = evaluate_budget_envelope(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=0.0,
            termination_reason="completed",
        )
        if pre_budget["input_budget_exceeded"]:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            return build_input_budget_exceeded_response(
                req_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                pre_budget=pre_budget,
                max_input_bytes=max_input_bytes,
                extra_fields={"model": resolved_model},
            )

        attempt_count = 1
        if job_store is not None:
            record, should_dispatch = job_store.resolve_or_register_request(
                task_type=schema_name or "http_fallback",
                workspace_dir=resolved_cwd,
                model=route.resolved_model,
                provider=route.provider,
                engine=route.engine,
                request_id=request_id,
                idempotency_key=idempotency_key,
                retry_same_request=retry_same_request,
                metadata={"schema_name": schema_name},
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

        sys_msg = (
            "You are OpenAI Codex collaborating with Gemini 4 Argon (Antigravity) "
            "in a Heterarchy architecture. Respond with high technical precision."
        )
        payload: JsonObject = {
            "model": resolved_model,
            "messages": [
                {"role": "system", "content": sys_msg},
                {"role": "user", "content": prompt},
            ],
        }
        if output_schema is not None:
            payload["response_format"] = {
                "type": "json_schema",
                "json_schema": {
                    "name": schema_name,
                    "strict": True,
                    "schema": output_schema,
                },
            }

        try:
            async with httpx.AsyncClient(timeout=timeout_seconds) as client:
                resp = await client.post(
                    f"{resolved_url}/chat/completions",
                    headers={"Authorization": f"Bearer {resolved_key}"},
                    json=payload,
                )
                resp.raise_for_status()
                body = resp.json()
            if not isinstance(body, dict):
                raise TypeError("HTTP response body is not a JSON object.")
            content = str(body["choices"][0]["message"]["content"])
            reported_model = str(body.get("model") or resolved_model)
        except asyncio.CancelledError:
            if job_store is not None:
                job_store.cancel_job(req_id, reason="HTTP fallback request cancelled.")
            raise
        except httpx.TimeoutException as exc:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            err_msg = redact_sensitive_text(f"HTTP transport timed out after {duration}s: {exc}")
            receipt = build_execution_receipt(
                request_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                actual_model=resolved_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status=JobStatusKind.TIMED_OUT_UNKNOWN,
                outcome_state=OutcomeStateKind.OUTCOME_UNKNOWN,
                attempt_count=attempt_count,
                result_ref=f"job://{req_id}",
                failure_reason=err_msg,
            )
            budget = evaluate_budget_envelope(
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=duration,
                termination_reason="timeout_budget_reached",
            )
            if job_store is not None:
                job_store.mark_timed_out_unknown(req_id, error=err_msg, receipt=receipt)
            return {
                "ok": False,
                "status": JobStatusKind.TIMED_OUT_UNKNOWN,
                "outcome_state": OutcomeStateKind.OUTCOME_UNKNOWN,
                "request_id": req_id,
                "job_id": req_id,
                "provider": route.provider,
                "engine": EngineKind.HTTP,
                "model": resolved_model,
                "error": err_msg,
                "receipt": as_json_object(receipt),
                "budget": as_json_object(budget),
            }
        except (httpx.HTTPError, KeyError, IndexError, TypeError, ValueError) as exc:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            err_msg = redact_sensitive_text(f"HTTP transport failed: {exc}")
            receipt = build_execution_receipt(
                request_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                actual_model=resolved_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status=JobStatusKind.FAILED,
                outcome_state=OutcomeStateKind.CONFIRMED_FAILURE,
                attempt_count=attempt_count,
                result_ref=f"job://{req_id}",
                failure_reason=err_msg,
            )
            budget = evaluate_budget_envelope(
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=duration,
                termination_reason="http_error",
            )
            if job_store is not None:
                job_store.fail_job(req_id, error=err_msg, receipt=receipt)
            return {
                "ok": False,
                "status": JobStatusKind.FAILED,
                "outcome_state": OutcomeStateKind.CONFIRMED_FAILURE,
                "request_id": req_id,
                "job_id": req_id,
                "provider": route.provider,
                "engine": EngineKind.HTTP,
                "model": resolved_model,
                "error": err_msg,
                "receipt": as_json_object(receipt),
                "budget": as_json_object(budget),
            }

        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        duration = round(time.monotonic() - started_mono, 2)
        parsed: JsonValue = (
            CodexOutputParser.parse_structured_response(content) if output_schema else content
        )
        structured_obj: JsonObject | None = (
            parsed if isinstance(parsed_data := parsed, dict) else None
        )

        receipt = build_execution_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model=reported_model,
            call_path=call_path,
            started_at_iso=started_iso,
            ended_at_iso=ended_iso,
            duration_seconds=duration,
            status=JobStatusKind.SUCCEEDED,
            outcome_state=OutcomeStateKind.CONFIRMED_SUCCESS,
            attempt_count=attempt_count,
            output_text=content,
            result_ref=f"job://{req_id}",
        )
        model_mismatch = receipt["model_mismatch"]
        budget = evaluate_budget_envelope(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=duration,
            termination_reason="model_mismatch" if model_mismatch else "completed",
        )
        response_payload: JsonObject = {
            "ok": not model_mismatch,
            "status": JobStatusKind.FAILED if model_mismatch else JobStatusKind.SUCCEEDED,
            "outcome_state": (
                OutcomeStateKind.CONFIRMED_FAILURE
                if model_mismatch
                else OutcomeStateKind.CONFIRMED_SUCCESS
            ),
            "request_id": req_id,
            "job_id": req_id,
            "provider": route.provider,
            "engine": EngineKind.HTTP,
            "model": reported_model,
            "data": parsed_data,
            "structured_output": structured_obj,
            "receipt": as_json_object(receipt),
            "budget": as_json_object(budget),
        }
        if model_mismatch:
            err_text = receipt["failure_reason"] or "Model mismatch detected."
            response_payload["error"] = err_text
            response_payload["error_code"] = "ACTUAL_MODEL_MISMATCH"
            if job_store is not None:
                job_store.fail_job(req_id, error=err_text, receipt=receipt)
        elif job_store is not None:
            job_store.complete_job(req_id, result=response_payload, receipt=receipt)

        return response_payload
