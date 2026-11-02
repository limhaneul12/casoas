"""Local Antigravity LanguageServer bridge for invoking Claude Sonnet/Opus and Gemini models."""

from __future__ import annotations

import asyncio
import time
import urllib.error
import uuid
from datetime import UTC, datetime

from app.clients.agy_rpc_transport import (
    AgyEndpoint,
    AntigravityCircuitBreaker,
    AntigravityModelCatalog,
    AntigravityRpcClient,
    RpcTransportFn,
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
    TaskTypeKind,
)
from app.repositories.job_store import JobStore
from app.routing.auth_redaction import redact_sensitive_text
from app.routing.policy_engine import RoutingPolicyEngine
from app.routing.receipt_budget import BudgetEnvelope
from app.schemas.boundary_schemas import JsonObject, as_json_object
from app.schemas.inner_dto.dto_dict_types.transport_dict_types import (
    AgyModelConfigEntryDict,
)
from app.schemas.inner_dto.transport_dtos import (
    AgyDeduplicatedResponse,
    AgyEarlyFailResult,
    AgyPromptSuccessResult,
    AgyRouteExtraFields,
    AgyRpcPromptPayload,
    AgyTimeoutPartialResult,
    AgyTransportErrorResult,
)

DEFAULT_CLAUDE_MODEL: str = str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM)


def extract_json_from_model_text(raw_text: str) -> JsonObject | None:
    """Extract a JSON object from model response text, stripping markdown fences if present.

    Args:
        raw_text: Raw text output returned by the language model.

    Returns:
        Parsed JSON object dictionary if valid JSON is found, otherwise `None`.
    """
    return AntigravityModelCatalog.extract_json_from_model_text(raw_text=raw_text)


async def discover_agy_endpoint() -> AgyEndpoint | None:
    """Discover the live Antigravity `language_server` HTTPS endpoint and CSRF token.

    Returns:
        Discovered `AgyEndpoint` instance if a live server responds, otherwise `None`.
    """
    return await AntigravityRpcClient.discover_endpoint()


async def list_agy_models(
    endpoint: AgyEndpoint | None = None,
    rpc_transport: RpcTransportFn | None = None,
) -> list[AgyModelConfigEntryDict]:
    """Return available models and quotas from the local Antigravity LanguageServer.

    Args:
        endpoint: Optional pre-discovered `AgyEndpoint` to query.
        rpc_transport: Optional synchronous Connect-RPC transport callable override.

    Returns:
        List of normalized model catalog dictionaries.
    """
    return await AntigravityModelCatalog.list_models(
        endpoint=endpoint,
        rpc_transport=rpc_transport,
        discover_fn=discover_agy_endpoint,
    )


async def run_agy_model_prompt(
    prompt: str,
    model: str = DEFAULT_CLAUDE_MODEL,
    expect_json: bool = True,
    timeout_seconds: float = 180.0,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    max_input_bytes: int = 256_000,
    workspace_dir: str = ".",
    job_store: JobStore | None = None,
    endpoint: AgyEndpoint | None = None,
    rpc_transport: RpcTransportFn | None = None,
) -> JsonObject:
    """Invoke Claude Sonnet 5.5, Claude Opus 5.5, or Gemini 4 Argon via local Antigravity RPC.

    Args:
        prompt: Prompt text sent to the Antigravity LanguageServer.
        model: Target model ID or alias (defaults to Claude Sonnet 5.5 Medium).
        expect_json: Whether to extract structured JSON from the model response.
        timeout_seconds: Read timeout in seconds for the Connect-RPC invocation.
        provider: Optional explicit provider name for routing validation.
        request_id: Optional caller-supplied request identifier (`cdx-...`).
        idempotency_key: Optional idempotency key for deduplicating dispatches.
        retry_same_request: Whether to explicitly re-dispatch an existing idempotency key.
        max_input_bytes: Maximum UTF-8 byte budget for `prompt`.
        workspace_dir: Workspace directory associated with the job record.
        job_store: Optional durable `JobStore` for tracking request lifecycle.
        endpoint: Optional pre-discovered `AgyEndpoint` override.
        rpc_transport: Optional synchronous Connect-RPC transport callable override.

    Returns:
        Execution result dictionary including status, outcome_state, receipt, and output.
    """
    policy_engine = RoutingPolicyEngine()
    started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    started_mono = time.monotonic()
    call_path = "agy_bridge.run_agy_model_prompt -> LanguageServerService/GetModelResponse"

    route = policy_engine.validate_route(
        engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
        model=model,
        provider=provider,
        enforce_claude_medium_policy=True,
    )
    if not route.valid:
        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
        auth_b = as_json_object(record=AntigravityModelCatalog.build_auth_boundary(connected=False))
        extra_f = as_json_object(record=AgyRouteExtraFields(model_requested=model, auth_boundary=auth_b).to_dict())
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
            extra_fields=extra_f,
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
            extra_fields={"model_id": route.resolved_model},
        )

    attempt_count = 1
    if job_store is not None:
        record, should_dispatch = job_store.resolve_or_register_request(
            task_type=str(TaskTypeKind.AGY_RPC_PROMPT),
            workspace_dir=workspace_dir,
            model=route.resolved_model,
            provider=route.provider,
            engine=route.engine,
            request_id=request_id,
            idempotency_key=idempotency_key,
            retry_same_request=retry_same_request,
            metadata={"placeholder": route.placeholder or ""},
        )
        req_id, attempt_count = record.job_id, record.attempt_count
        if not should_dispatch:
            rcpt_obj = as_json_object(record=record.receipt) if record.receipt else None
            dedup = AgyDeduplicatedResponse(
                ok=record.status == JobStatusKind.COMPLETED,
                request_id=record.job_id,
                job_id=record.job_id,
                status=record.status,
                outcome_state=record.outcome_state,
                attempt_count=record.attempt_count,
                provider=route.provider,
                engine=route.engine,
                model_id=route.resolved_model,
                result=record.result,
                error=record.error,
                receipt=rcpt_obj,
            )
            return as_json_object(record=dedup.to_dict())
    else:
        req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"

    def _early_fail(
        err_msg: str,
        term_reason: str,
        actual_model: str = "",
        rpc_connected: bool = False,
        error_code: str | None = None,
    ) -> JsonObject:
        """Build and persist a confirmed early-failure receipt and payload.

        Args:
            err_msg: Redacted failure message.
            term_reason: Budget termination reason string.
            actual_model: Resolved model string or empty string.
            rpc_connected: Whether the local LanguageServer endpoint was connected.
            error_code: Optional structured error code.

        Returns:
            Structured failure response dictionary with receipt and budget envelope.
        """
        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        duration = round(time.monotonic() - started_mono, 2)
        receipt = policy_engine.build_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model=actual_model,
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
        if job_store is not None:
            job_store.fail_job(job_id=req_id, error=err_msg, receipt=receipt)
        budget = BudgetEnvelope.evaluate(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=duration,
            termination_reason=term_reason,
        )
        auth_b = AntigravityModelCatalog.build_auth_boundary(connected=rpc_connected)
        fail_dto = AgyEarlyFailResult(
            ok=False,
            status=str(JobStatusKind.FAILED),
            outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
            request_id=req_id,
            job_id=req_id,
            provider=route.provider,
            engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
            model_requested=model,
            model_id=route.resolved_model,
            error=err_msg,
            receipt=as_json_object(record=receipt),
            budget=as_json_object(record=budget),
            auth_boundary=as_json_object(record=auth_b),
        )
        out = as_json_object(record=fail_dto.to_dict())
        if error_code is not None:
            out["error_code"] = error_code
        return out

    cb_open, cb_rem, cb_last_err = AntigravityCircuitBreaker.check_open()
    if cb_open:
        return _early_fail(
            err_msg=f"Antigravity RPC circuit breaker is OPEN (cooldown={cb_rem}s remaining). Last failure: {cb_last_err}",
            term_reason=str(BudgetTerminationReason.CIRCUIT_BREAKER_OPEN),
            error_code=str(RoutingErrorCode.ANTIGRAVITY_CIRCUIT_OPEN),
        )

    try:
        resolved_ep = endpoint or await discover_agy_endpoint()
        if resolved_ep is None:
            unavail_msg = "Could not discover a running local Antigravity language_server process. Ensure Antigravity IDE / App is open."
            AntigravityCircuitBreaker.record_failure(error_message=unavail_msg)
            return _early_fail(err_msg=unavail_msg, term_reason=str(BudgetTerminationReason.ENDPOINT_UNAVAILABLE))
        resolved_model_id, placeholder = await AntigravityModelCatalog.resolve_model_placeholder(
            requested_model=model,
            endpoint=resolved_ep,
            rpc_transport=rpc_transport,
            discover_fn=discover_agy_endpoint,
        )
    except PermissionError as exc:
        AntigravityCircuitBreaker.record_success()
        return _early_fail(
            err_msg=redact_sensitive_text(text=str(exc)),
            term_reason=str(BudgetTerminationReason.QUOTA_EXHAUSTED),
            actual_model=route.resolved_model,
            rpc_connected=True,
            error_code=str(RoutingErrorCode.ANTIGRAVITY_QUOTA_EXHAUSTED),
        )
    except asyncio.CancelledError:
        if job_store is not None:
            job_store.cancel_job(job_id=req_id, reason="Antigravity RPC request cancelled.")
        raise
    except Exception as exc:
        redacted_err = redact_sensitive_text(text=str(exc))
        AntigravityCircuitBreaker.record_failure(error_message=redacted_err)
        if job_store is not None:
            job_store.fail_job(job_id=req_id, error=redacted_err)
        raise

    transport = rpc_transport or AntigravityRpcClient.post_connect_rpc_sync
    rpc_payload = as_json_object(record=AgyRpcPromptPayload(model=placeholder, prompt=prompt).to_dict())

    try:
        rpc_resp = await asyncio.to_thread(
            transport,
            resolved_ep.base_url,
            resolved_ep.csrf_token,
            "GetModelResponse",
            rpc_payload,
            timeout_seconds,
        )
    except asyncio.CancelledError:
        if job_store is not None:
            job_store.cancel_job(job_id=req_id, reason="Antigravity RPC request cancelled.")
        raise
    except Exception as exc:
        if endpoint is None:
            with AntigravityRpcClient._cache_lock:
                AntigravityRpcClient._cached_endpoint = None
        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        duration = round(time.monotonic() - started_mono, 2)
        is_timeout = AntigravityRpcClient.is_timeout_exception(exc=exc)
        if is_timeout:
            err_msg = redact_sensitive_text(text=f"Antigravity RPC read timed out after {duration}s (limit {timeout_seconds}s) for model '{resolved_model_id}'. Provider accepted the request, so completion is unknown. Query `codex_job_status(job_id='{req_id}')` before retrying.")
            status_val, outcome_val, term_val = (
                str(JobStatusKind.TIMED_OUT_UNKNOWN),
                str(OutcomeStateKind.OUTCOME_UNKNOWN),
                str(BudgetTerminationReason.TIMEOUT_BUDGET_REACHED),
            )
        elif isinstance(exc, urllib.error.HTTPError):
            err_body = exc.read().decode("utf-8", errors="replace")
            err_msg = redact_sensitive_text(text=f"HTTP {exc.code}: {err_body[:1000]}")
            status_val, outcome_val, term_val = (
                str(JobStatusKind.FAILED),
                str(OutcomeStateKind.CONFIRMED_FAILURE),
                str(BudgetTerminationReason.RPC_ERROR),
            )
        else:
            err_msg = redact_sensitive_text(text=f"Antigravity RPC invocation failed: {exc}")
            status_val, outcome_val, term_val = (
                str(JobStatusKind.FAILED),
                str(OutcomeStateKind.CONFIRMED_FAILURE),
                str(BudgetTerminationReason.RPC_ERROR),
            )

        receipt = policy_engine.build_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model=resolved_model_id,
            call_path=call_path,
            started_at_iso=started_iso,
            ended_at_iso=ended_iso,
            duration_seconds=duration,
            status=status_val,
            outcome_state=outcome_val,
            attempt_count=attempt_count,
            result_ref=f"job://{req_id}",
            failure_reason=err_msg,
        )
        AntigravityCircuitBreaker.record_failure(error_message=err_msg)
        budget = BudgetEnvelope.evaluate(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=duration,
            termination_reason=term_val,
        )
        if job_store is not None:
            if is_timeout:
                partial_dto = AgyTimeoutPartialResult(
                    request_id=req_id,
                    model_id=resolved_model_id,
                    placeholder=placeholder,
                    status=status_val,
                    outcome_state=outcome_val,
                )
                job_store.mark_timed_out_unknown(
                    job_id=req_id,
                    error=err_msg,
                    partial_result=as_json_object(record=partial_dto.to_dict()),
                    receipt=receipt,
                )
            else:
                job_store.fail_job(job_id=req_id, error=err_msg, receipt=receipt)
        auth_b = as_json_object(record=AntigravityModelCatalog.build_auth_boundary(connected=True))
        err_dto = AgyTransportErrorResult(
            ok=False,
            status=status_val,
            outcome_state=outcome_val,
            request_id=req_id,
            job_id=req_id,
            provider=route.provider,
            engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
            model_id=resolved_model_id,
            placeholder=placeholder,
            duration_seconds=duration,
            error=err_msg,
            receipt=as_json_object(record=receipt),
            budget=as_json_object(record=budget),
            auth_boundary=auth_b,
        )
        return as_json_object(record=err_dto.to_dict())

    ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    AntigravityCircuitBreaker.record_success()
    duration = round(time.monotonic() - started_mono, 2)
    raw_response = str(rpc_resp.get("response", ""))
    rep_m = rpc_resp.get("modelId") or rpc_resp.get("actualModel")
    actual_model_id = rep_m.strip() if isinstance(rep_m, str) and rep_m.strip() else resolved_model_id
    structured = extract_json_from_model_text(raw_text=raw_response) if expect_json else None
    receipt = policy_engine.build_receipt(
        request_id=req_id,
        idempotency_key=idempotency_key,
        route=route,
        actual_model=actual_model_id,
        call_path=call_path,
        started_at_iso=started_iso,
        ended_at_iso=ended_iso,
        duration_seconds=duration,
        status=str(JobStatusKind.SUCCEEDED),
        outcome_state=str(OutcomeStateKind.CONFIRMED_SUCCESS),
        attempt_count=attempt_count,
        output_text=raw_response,
        result_ref=f"job://{req_id}",
    )
    model_mismatch = receipt["model_mismatch"]
    term_reason = str(BudgetTerminationReason.MODEL_MISMATCH if model_mismatch else BudgetTerminationReason.COMPLETED)
    budget = BudgetEnvelope.evaluate(
        prompt_text=prompt,
        max_input_bytes=max_input_bytes,
        timeout_seconds=timeout_seconds,
        elapsed_seconds=duration,
        termination_reason=term_reason,
    )
    auth_b = as_json_object(record=AntigravityModelCatalog.build_auth_boundary(connected=True))
    status_s = str(JobStatusKind.FAILED if model_mismatch else JobStatusKind.SUCCEEDED)
    outcome_s = str(OutcomeStateKind.CONFIRMED_FAILURE if model_mismatch else OutcomeStateKind.CONFIRMED_SUCCESS)
    response_payload = as_json_object(
        record=AgyPromptSuccessResult(
            ok=not model_mismatch,
            status=status_s,
            outcome_state=outcome_s,
            request_id=req_id,
            job_id=req_id,
            provider=route.provider,
            engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
            model_id=actual_model_id,
            requested_model=model,
            placeholder=placeholder,
            duration_seconds=duration,
            structured_output=structured,
            raw_output=raw_response,
            receipt=as_json_object(record=receipt),
            budget=as_json_object(record=budget),
            auth_boundary=auth_b,
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
