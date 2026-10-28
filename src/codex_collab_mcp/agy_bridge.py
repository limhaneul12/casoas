"""Local Antigravity LanguageServer bridge for invoking Claude Sonnet/Opus and Gemini models."""

from __future__ import annotations

import asyncio
import time
import urllib.error
import uuid
from datetime import UTC, datetime

from codex_collab_mcp.collab_enums import (
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    OutcomeStateKind,
)
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.routing.auth_redaction import redact_sensitive_text
from codex_collab_mcp.routing.policy_engine import (
    build_execution_receipt,
    build_input_budget_exceeded_response,
    build_rejected_pre_dispatch_response,
    validate_and_resolve_route,
)
from codex_collab_mcp.routing.receipt_budget import evaluate_budget_envelope
from codex_collab_mcp.schemas import JsonObject, as_json_object
from codex_collab_mcp.transports.agy_rpc_transport import (
    AgyEndpoint,
    AntigravityCircuitBreaker,
    AntigravityModelCatalog,
    AntigravityRpcClient,
    RpcTransportFn,
)
from codex_collab_mcp.transports.transport_dicts import AgyModelConfigEntryDict

DEFAULT_CLAUDE_MODEL: str = CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM

extract_json_from_model_text = AntigravityModelCatalog.extract_json_from_model_text

__all__ = [
    "DEFAULT_CLAUDE_MODEL",
    "AgyEndpoint",
    "AntigravityCircuitBreaker",
    "discover_agy_endpoint",
    "extract_json_from_model_text",
    "list_agy_models",
    "run_agy_model_prompt",
]


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
    *,
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
    started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    started_mono = time.monotonic()
    call_path = "agy_bridge.run_agy_model_prompt -> LanguageServerService/GetModelResponse"

    route = validate_and_resolve_route(
        engine=EngineKind.ANTIGRAVITY_LOCAL_RPC,
        model=model,
        provider=provider,
        enforce_claude_medium_policy=True,
    )
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
                "model_requested": model,
                "auth_boundary": as_json_object(AntigravityModelCatalog.build_auth_boundary(False)),
            },
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
            extra_fields={"model_id": route.resolved_model},
        )

    attempt_count = 1
    if job_store is not None:
        record, should_dispatch = job_store.resolve_or_register_request(
            task_type="agy_rpc_prompt",
            workspace_dir=workspace_dir,
            model=route.resolved_model,
            provider=route.provider,
            engine=route.engine,
            request_id=request_id,
            idempotency_key=idempotency_key,
            retry_same_request=retry_same_request,
            metadata={"placeholder": route.placeholder or ""},
        )
        req_id = record.job_id
        attempt_count = record.attempt_count
        if not should_dispatch:
            return {
                "ok": record.status == JobStatusKind.COMPLETED,
                "deduplicated_existing_request": True,
                "request_id": record.job_id,
                "job_id": record.job_id,
                "status": record.status,
                "outcome_state": record.outcome_state,
                "attempt_count": record.attempt_count,
                "provider": route.provider,
                "engine": route.engine,
                "model_id": route.resolved_model,
                "result": record.result,
                "error": record.error,
                "receipt": as_json_object(record.receipt) if record.receipt else None,
            }
    else:
        req_id = (request_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"

    def _early_fail(
        err_msg: str,
        term_reason: str,
        *,
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
        receipt = build_execution_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model=actual_model,
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
        if job_store is not None:
            job_store.fail_job(req_id, error=err_msg, receipt=receipt)
        budget = evaluate_budget_envelope(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=duration,
            termination_reason=term_reason,
        )
        auth_b = AntigravityModelCatalog.build_auth_boundary(rpc_connected)
        out: JsonObject = {
            "ok": False,
            "status": JobStatusKind.FAILED,
            "outcome_state": OutcomeStateKind.CONFIRMED_FAILURE,
            "request_id": req_id,
            "job_id": req_id,
            "provider": route.provider,
            "engine": EngineKind.ANTIGRAVITY_LOCAL_RPC,
            "model_requested": model,
            "model_id": route.resolved_model,
            "error": err_msg,
            "receipt": as_json_object(receipt),
            "budget": as_json_object(budget),
            "auth_boundary": as_json_object(auth_b),
        }
        if error_code is not None:
            out["error_code"] = error_code
        return out

    cb_open, cb_rem, cb_last_err = AntigravityCircuitBreaker.check_open()
    if cb_open:
        return _early_fail(
            f"Antigravity RPC circuit breaker is OPEN (cooldown={cb_rem}s remaining). "
            f"Last failure: {cb_last_err}",
            "circuit_breaker_open",
            error_code="ANTIGRAVITY_CIRCUIT_OPEN",
        )

    try:
        resolved_ep = endpoint or await discover_agy_endpoint()
        if resolved_ep is None:
            unavail_msg = (
                "Could not discover a running local Antigravity language_server process. "
                "Ensure Antigravity IDE / App is open."
            )
            AntigravityCircuitBreaker.record_failure(unavail_msg)
            return _early_fail(unavail_msg, "endpoint_unavailable")
        resolved_model_id, placeholder = await AntigravityModelCatalog.resolve_model_placeholder(
            requested_model=model,
            endpoint=resolved_ep,
            rpc_transport=rpc_transport,
            discover_fn=discover_agy_endpoint,
        )
    except PermissionError as exc:
        AntigravityCircuitBreaker.record_success()
        return _early_fail(
            redact_sensitive_text(str(exc)),
            "quota_exhausted",
            actual_model=route.resolved_model,
            rpc_connected=True,
            error_code="ANTIGRAVITY_QUOTA_EXHAUSTED",
        )
    except asyncio.CancelledError:
        if job_store is not None:
            job_store.cancel_job(req_id, reason="Antigravity RPC request cancelled.")
        raise
    except Exception as exc:
        AntigravityCircuitBreaker.record_failure(redact_sensitive_text(str(exc)))
        if job_store is not None:
            job_store.fail_job(req_id, error=redact_sensitive_text(str(exc)))
        raise

    transport = rpc_transport or AntigravityRpcClient.post_connect_rpc_sync
    rpc_payload: JsonObject = {"model": placeholder, "prompt": prompt}

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
            job_store.cancel_job(req_id, reason="Antigravity RPC request cancelled.")
        raise
    except Exception as exc:
        if endpoint is None:
            with AntigravityRpcClient._cache_lock:
                AntigravityRpcClient._cached_endpoint = None
        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        duration = round(time.monotonic() - started_mono, 2)
        is_timeout = AntigravityRpcClient.is_timeout_exception(exc)
        if is_timeout:
            err_msg = redact_sensitive_text(
                f"Antigravity RPC read timed out after {duration}s (limit {timeout_seconds}s) "
                f"for model '{resolved_model_id}'. Provider accepted the request, so completion "
                f"is unknown. Query `codex_job_status(job_id='{req_id}')` before retrying."
            )
            status_val = JobStatusKind.TIMED_OUT_UNKNOWN
            outcome_val = OutcomeStateKind.OUTCOME_UNKNOWN
            term_val = "timeout_budget_reached"
        elif isinstance(exc, urllib.error.HTTPError):
            err_body = exc.read().decode("utf-8", errors="replace")
            err_msg = redact_sensitive_text(f"HTTP {exc.code}: {err_body[:1000]}")
            status_val = JobStatusKind.FAILED
            outcome_val = OutcomeStateKind.CONFIRMED_FAILURE
            term_val = "rpc_error"
        else:
            err_msg = redact_sensitive_text(f"Antigravity RPC invocation failed: {exc}")
            status_val = JobStatusKind.FAILED
            outcome_val = OutcomeStateKind.CONFIRMED_FAILURE
            term_val = "rpc_error"

        receipt = build_execution_receipt(
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
        AntigravityCircuitBreaker.record_failure(err_msg)
        budget = evaluate_budget_envelope(
            prompt_text=prompt,
            max_input_bytes=max_input_bytes,
            timeout_seconds=timeout_seconds,
            elapsed_seconds=duration,
            termination_reason=term_val,
        )
        if job_store is not None:
            if is_timeout:
                job_store.mark_timed_out_unknown(
                    req_id,
                    error=err_msg,
                    partial_result={
                        "request_id": req_id,
                        "model_id": resolved_model_id,
                        "placeholder": placeholder,
                        "status": status_val,
                        "outcome_state": outcome_val,
                    },
                    receipt=receipt,
                )
            else:
                job_store.fail_job(req_id, error=err_msg, receipt=receipt)
        return {
            "ok": False,
            "status": status_val,
            "outcome_state": outcome_val,
            "request_id": req_id,
            "job_id": req_id,
            "provider": route.provider,
            "engine": EngineKind.ANTIGRAVITY_LOCAL_RPC,
            "model_id": resolved_model_id,
            "placeholder": placeholder,
            "duration_seconds": duration,
            "error": err_msg,
            "receipt": as_json_object(receipt),
            "budget": as_json_object(budget),
            "auth_boundary": as_json_object(AntigravityModelCatalog.build_auth_boundary(True)),
        }

    ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    AntigravityCircuitBreaker.record_success()
    duration = round(time.monotonic() - started_mono, 2)
    raw_response = str(rpc_resp.get("response", ""))
    reported_model_raw = rpc_resp.get("modelId") or rpc_resp.get("actualModel")
    actual_model_id = (
        reported_model_raw.strip()
        if isinstance(reported_model_raw, str) and reported_model_raw.strip()
        else resolved_model_id
    )

    structured = extract_json_from_model_text(raw_response) if expect_json else None
    receipt = build_execution_receipt(
        request_id=req_id,
        idempotency_key=idempotency_key,
        route=route,
        actual_model=actual_model_id,
        call_path=call_path,
        started_at_iso=started_iso,
        ended_at_iso=ended_iso,
        duration_seconds=duration,
        status=JobStatusKind.SUCCEEDED,
        outcome_state=OutcomeStateKind.CONFIRMED_SUCCESS,
        attempt_count=attempt_count,
        output_text=raw_response,
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
        "engine": EngineKind.ANTIGRAVITY_LOCAL_RPC,
        "model_id": actual_model_id,
        "requested_model": model,
        "placeholder": placeholder,
        "duration_seconds": duration,
        "structured_output": structured,
        "raw_output": raw_response,
        "receipt": as_json_object(receipt),
        "budget": as_json_object(budget),
        "auth_boundary": as_json_object(AntigravityModelCatalog.build_auth_boundary(True)),
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
