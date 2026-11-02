"""Direct HTTP completion fallback runner for OpenAI-compatible local proxy endpoints."""

from __future__ import annotations

import asyncio
import os
import time
import uuid
from datetime import UTC, datetime

import httpx

from app.clients.codex_output_parser import CodexCommandBuilder, CodexOutputParser
from app.core.collab_enums import (
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    OutcomeStateKind,
    ProviderKind,
)
from app.core.model_contract_enums import (
    BudgetTerminationReason,
    RoutingErrorCode,
    TaskTypeKind,
)
from app.evidence.workspace_digests import WorkspaceDigestCalculator
from app.repositories.job_store import JobStore
from app.routing.auth_redaction import redact_sensitive_text
from app.routing.policy_engine import RoutingPolicyEngine
from app.routing.receipt_budget import BudgetEnvelope
from app.schemas.boundary_schemas import JsonObject, JsonValue, as_json_object
from app.schemas.inner_dto.execution_result_dtos import (
    HttpChatCompletionRequest,
    HttpChatMessage,
    HttpCompletedExecutionResult,
    HttpErrorExecutionResult,
    HttpJsonSchemaSpec,
    HttpResponseFormat,
)


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
        policy_engine = RoutingPolicyEngine()
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        started_mono = time.monotonic()
        req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
        resolved_cwd = str(
            WorkspaceDigestCalculator().resolve_dir(workspace_dir=workspace_dir or os.getcwd())
        )
        call_path = "runner.run_http_fallback -> POST /chat/completions"
        resolved_url = (
            base_url
            or os.environ.get("CODEX_BASE_URL")
            or os.environ.get("OPENAI_BASE_URL")
            or "http://127.0.0.1:8787/v1"
        ).rstrip("/")
        resolved_key = api_key or os.environ.get("OPENAI_API_KEY") or "headroom-local"
        resolved_model = model or os.environ.get("CODEX_MODEL") or str(CanonicalModelId.GPT_6_1_SOL)

        route = policy_engine.validate_route(
            engine=str(EngineKind.HTTP),
            model=resolved_model,
            provider=str(ProviderKind.LOCAL_HTTP),
        )
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
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
                extra_fields={"model": resolved_model},
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
            return policy_engine.build_budget_exceeded_response(
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
                task_type=schema_name or str(TaskTypeKind.HTTP_FALLBACK),
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
                    record=record,
                    provider=route.provider,
                    engine=route.engine,
                    resolved_cwd=resolved_cwd,
                    resolved_model=route.resolved_model,
                )

        sys_msg = (
            "You are OpenAI Codex collaborating with Gemini 4 Argon (Antigravity) "
            "in a Heterarchy architecture. Respond with high technical precision."
        )
        payload = as_json_object(
            record=HttpChatCompletionRequest(
                model=resolved_model,
                messages=[
                    as_json_object(
                        record=HttpChatMessage(role="system", content=sys_msg).to_dict()
                    ),
                    as_json_object(record=HttpChatMessage(role="user", content=prompt).to_dict()),
                ],
            ).to_dict()
        )
        if output_schema is not None:
            payload["response_format"] = as_json_object(
                record=HttpResponseFormat(
                    type="json_schema",
                    json_schema=HttpJsonSchemaSpec(
                        name=schema_name,
                        strict=True,
                        schema=output_schema,
                    ).to_dict(),
                ).to_dict()
            )

        try:
            async with httpx.AsyncClient(timeout=timeout_seconds) as client:
                resp = await client.post(
                    url=f"{resolved_url}/chat/completions",
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
                job_store.cancel_job(job_id=req_id, reason="HTTP fallback request cancelled.")
            raise
        except httpx.TimeoutException as exc:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            err_msg = redact_sensitive_text(
                text=f"HTTP transport timed out after {duration}s: {exc}"
            )
            receipt = policy_engine.build_receipt(
                request_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                actual_model=resolved_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
                attempt_count=attempt_count,
                result_ref=f"job://{req_id}",
                failure_reason=err_msg,
            )
            budget = BudgetEnvelope.evaluate(
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=duration,
                termination_reason=str(BudgetTerminationReason.TIMEOUT_BUDGET_REACHED),
            )
            if job_store is not None:
                job_store.mark_timed_out_unknown(job_id=req_id, error=err_msg, receipt=receipt)
            return as_json_object(
                record=HttpErrorExecutionResult(
                    ok=False,
                    status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                    outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
                    request_id=req_id,
                    job_id=req_id,
                    provider=route.provider,
                    engine=str(EngineKind.HTTP),
                    model=resolved_model,
                    error=err_msg,
                    receipt=as_json_object(record=receipt),
                    budget=as_json_object(record=budget),
                ).to_dict()
            )
        except (httpx.HTTPError, KeyError, IndexError, TypeError, ValueError) as exc:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            err_msg = redact_sensitive_text(text=f"HTTP transport failed: {exc}")
            receipt = policy_engine.build_receipt(
                request_id=req_id,
                idempotency_key=idempotency_key,
                route=route,
                actual_model=resolved_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status=str(JobStatusKind.FAILED),
                outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
                attempt_count=attempt_count,
                result_ref=f"job://{req_id}",
                failure_reason=err_msg,
            )
            budget = BudgetEnvelope.evaluate(
                prompt_text=prompt,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=duration,
                termination_reason=str(BudgetTerminationReason.HTTP_ERROR),
            )
            if job_store is not None:
                job_store.fail_job(job_id=req_id, error=err_msg, receipt=receipt)
            return as_json_object(
                record=HttpErrorExecutionResult(
                    ok=False,
                    status=str(JobStatusKind.FAILED),
                    outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
                    request_id=req_id,
                    job_id=req_id,
                    provider=route.provider,
                    engine=str(EngineKind.HTTP),
                    model=resolved_model,
                    error=err_msg,
                    receipt=as_json_object(record=receipt),
                    budget=as_json_object(record=budget),
                ).to_dict()
            )

        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        duration = round(time.monotonic() - started_mono, 2)
        parsed: JsonValue = (
            CodexOutputParser.parse_structured_response(raw_response=content)
            if output_schema
            else content
        )
        structured_obj: JsonObject | None = (
            parsed if isinstance(parsed_data := parsed, dict) else None
        )

        receipt = policy_engine.build_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model=reported_model,
            call_path=call_path,
            started_at_iso=started_iso,
            ended_at_iso=ended_iso,
            duration_seconds=duration,
            status=str(JobStatusKind.SUCCEEDED),
            outcome_state=str(OutcomeStateKind.CONFIRMED_SUCCESS),
            attempt_count=attempt_count,
            output_text=content,
            result_ref=f"job://{req_id}",
        )
        model_mismatch = receipt["model_mismatch"]
        budget = BudgetEnvelope.evaluate(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=duration,
            termination_reason=(
                str(BudgetTerminationReason.MODEL_MISMATCH)
                if model_mismatch
                else str(BudgetTerminationReason.COMPLETED)
            ),
        )
        response_payload = as_json_object(
            record=HttpCompletedExecutionResult(
                ok=not model_mismatch,
                status=str(JobStatusKind.FAILED)
                if model_mismatch
                else str(JobStatusKind.SUCCEEDED),
                outcome_state=(
                    str(OutcomeStateKind.CONFIRMED_FAILURE)
                    if model_mismatch
                    else str(OutcomeStateKind.CONFIRMED_SUCCESS)
                ),
                request_id=req_id,
                job_id=req_id,
                provider=route.provider,
                engine=str(EngineKind.HTTP),
                model=reported_model,
                data=parsed_data,
                structured_output=structured_obj,
                receipt=as_json_object(record=receipt),
                budget=as_json_object(record=budget),
            ).to_dict()
        )
        if model_mismatch:
            err_text = receipt["failure_reason"] or "Model mismatch detected."
            response_payload["error"] = err_text
            response_payload["error_code"] = str(RoutingErrorCode.ACTUAL_MODEL_MISMATCH)
            if job_store is not None:
                job_store.fail_job(job_id=req_id, error=err_text, receipt=receipt)
        elif job_store is not None:
            job_store.complete_job(job_id=req_id, result=response_payload, receipt=receipt)

        return response_payload


async def run_http_fallback(
    prompt: str,
    schema_name: str,
    output_schema: JsonObject | None = None,
    model: str | None = None,
    base_url: str | None = None,
    api_key: str | None = None,
    timeout_seconds: float = 180.0,
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
    return await CodexHttpFallbackRunner.run_http_fallback(
        prompt=prompt,
        schema_name=schema_name,
        output_schema=output_schema,
        model=model,
        base_url=base_url,
        api_key=api_key,
        timeout_seconds=timeout_seconds,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        workspace_dir=workspace_dir,
        job_store=job_store,
    )
