"""Provider/engine/model routing validation, execution receipts, and budget evaluation."""

from __future__ import annotations

from codex_collab_mcp.collab_enums import (
    AuthContextKind,
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    OutcomeStateKind,
    ProviderKind,
    ReasoningTier,
)
from codex_collab_mcp.domain.routing_catalog import (
    ANTIGRAVITY_MODEL_SPECS,
    CODEX_SUPPORTED_MODELS,
    EXPLICIT_MODEL_ALIASES,
    VALID_REASONING_TIERS,
    ExecutionReceipt,
    RoutingValidationResult,
    compute_output_digest,
    evaluate_budget_envelope,
    redact_sensitive_text,
)
from codex_collab_mcp.schemas import JsonObject


class RoutingPolicyEngine:
    """Cohesive engine for routing validation, execution receipts, and pre-dispatch responses."""

    def normalize_engine(self, engine: str) -> str:
        """Normalize shorthand engine names to canonical identifiers.

        Args:
            engine: Raw engine identifier string.

        Returns:
            Canonical engine name.
        """
        e, agy_rpc = engine.strip().lower(), str(EngineKind.ANTIGRAVITY_LOCAL_RPC)
        if e in ("cli", EngineKind.CODEX_CLI, "codex-cli"):
            return str(EngineKind.CODEX_CLI)
        if e in ("agy", ProviderKind.ANTIGRAVITY, agy_rpc, "antigravity-rpc", "rpc"):
            return agy_rpc
        return str(EngineKind.HTTP) if e in ("http", "local_http", "openai_http") else e

    def _reject_route(
        self,
        provider: str,
        engine: str,
        req_model: str,
        tier: str,
        code: str,
        msg: str,
        auth: str = "unknown",
        resolved: str = "",
        placeholder: str | None = None,
    ) -> RoutingValidationResult:
        """Build a rejected RoutingValidationResult.

        Args:
            provider: Provider identifier.
            engine: Canonical engine name.
            req_model: Requested model string.
            tier: Reasoning effort tier.
            code: Machine-readable rejection error code.
            msg: Human-readable rejection explanation.
            auth: Authentication context identifier.
            resolved: Resolved model identifier if applicable.
            placeholder: Model placeholder if applicable.

        Returns:
            Invalid RoutingValidationResult with error metadata populated.
        """
        return RoutingValidationResult(
            False, provider, engine, req_model, resolved, auth, tier, placeholder, code, msg
        )

    def validate_route(
        self,
        engine: str,
        model: str | None = None,
        provider: str | None = None,
        reasoning_effort: str | None = None,
        enforce_claude_medium_policy: bool = True,
    ) -> RoutingValidationResult:
        """Validate `(provider, engine, model, reasoning_effort)` before any dispatch occurs.

        Args:
            engine: Target execution engine name.
            model: Optional requested model identifier or alias.
            provider: Optional requested provider identifier.
            reasoning_effort: Optional reasoning effort tier.
            enforce_claude_medium_policy: Whether to reject non-Medium Claude models.

        Returns:
            RoutingValidationResult describing validity, resolved model, or rejection reason.
        """
        eng = self.normalize_engine(engine)
        raw_m, norm_p = (model or "").strip(), (provider or "").strip().lower()
        norm_m, tier = raw_m.lower(), (reasoning_effort or ReasoningTier.HIGH).strip().lower()
        if tier not in VALID_REASONING_TIERS:
            msg = f"Unsupported tier '{reasoning_effort}'."
            return self._reject_route(
                norm_p or "unknown", eng, raw_m or "default", tier, "INVALID_REASONING_TIER", msg
            )
        if eng not in (EngineKind.CODEX_CLI, EngineKind.ANTIGRAVITY_LOCAL_RPC, EngineKind.HTTP):
            msg = f"Unsupported engine '{engine}'."
            return self._reject_route(
                norm_p or "unknown", eng, raw_m or "default", tier, "UNSUPPORTED_ENGINE", msg
            )
        if eng == EngineKind.CODEX_CLI:
            codex_p, oauth = (
                str(ProviderKind.OPENAI_CODEX),
                str(AuthContextKind.CODEX_CHATGPT_OAUTH),
            )
            eff = norm_m or str(CanonicalModelId.CODEX_DEFAULT)
            if norm_p and norm_p not in ("openai", codex_p, "codex"):
                msg = f"Provider '{provider}' mismatch."
                return self._reject_route(
                    norm_p, eng, raw_m or eff, tier, "PROVIDER_ENGINE_MISMATCH", msg, oauth
                )
            if (
                eff in ANTIGRAVITY_MODEL_SPECS
                or eff in EXPLICIT_MODEL_ALIASES
                or eff.startswith(("gemini-", "claude-", "model_placeholder_"))
            ):
                msg = f"Model '{raw_m}' is unsupported on 'codex_cli'."
                return self._reject_route(
                    norm_p or codex_p, eng, raw_m, tier, "MODEL_NOT_SUPPORTED_BY_ENGINE", msg, oauth
                )
            if eff not in CODEX_SUPPORTED_MODELS:
                msg = f"Model '{raw_m}' is not in the Codex CLI contract."
                return self._reject_route(
                    codex_p, eng, raw_m, tier, "UNRECOGNIZED_CODEX_MODEL", msg, oauth
                )
            return RoutingValidationResult(True, codex_p, eng, raw_m or eff, eff, oauth, tier, None)
        if eng == EngineKind.ANTIGRAVITY_LOCAL_RPC:
            agy_p, csrf = str(ProviderKind.ANTIGRAVITY), str(AuthContextKind.ANTIGRAVITY_LOCAL_CSRF)
            if norm_p and norm_p not in (agy_p, "google_antigravity", "agy"):
                msg = f"Provider '{provider}' mismatch."
                return self._reject_route(
                    norm_p, eng, raw_m, tier, "PROVIDER_ENGINE_MISMATCH", msg, csrf
                )
            res_id = EXPLICIT_MODEL_ALIASES.get(norm_m, norm_m) or str(
                CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM
            )
            if res_id not in ANTIGRAVITY_MODEL_SPECS:
                msg = f"Unsupported model '{raw_m}'."
                return self._reject_route(
                    agy_p, eng, raw_m, tier, "MODEL_NOT_SUPPORTED_BY_ENGINE", msg, csrf
                )
            spec = ANTIGRAVITY_MODEL_SPECS[res_id]
            if (
                enforce_claude_medium_policy
                and spec.model_id.startswith("claude-")
                and spec.reasoning_tier != ReasoningTier.MEDIUM
            ):
                msg = f"Claude model '{raw_m}' violates Medium-only policy."
                return self._reject_route(
                    agy_p,
                    eng,
                    raw_m,
                    spec.reasoning_tier,
                    "CLAUDE_TIER_POLICY_VIOLATION",
                    msg,
                    csrf,
                    spec.model_id,
                    spec.placeholder,
                )
            return RoutingValidationResult(
                True,
                spec.provider,
                spec.engine,
                raw_m or spec.model_id,
                spec.model_id,
                spec.auth_context,
                spec.reasoning_tier,
                spec.placeholder,
            )
        eff_http = norm_m or str(CanonicalModelId.GPT_6_1_SOL)
        http_p, http_auth = norm_p or str(ProviderKind.LOCAL_HTTP), str(AuthContextKind.HTTP_BEARER)
        return RoutingValidationResult(
            True, http_p, str(EngineKind.HTTP), raw_m or eff_http, eff_http, http_auth, tier, None
        )

    def build_receipt(
        self,
        *,
        request_id: str,
        idempotency_key: str | None,
        route: RoutingValidationResult,
        actual_model: str,
        call_path: str,
        started_at_iso: str,
        ended_at_iso: str,
        duration_seconds: float,
        status: str,
        outcome_state: str,
        attempt_count: int = 1,
        output_text: str | None = None,
        result_ref: str | None = None,
        failure_reason: str | None = None,
    ) -> JsonObject:
        """Build a verifiable, credential-free execution receipt for any collaboration call.

        Args:
            request_id: Unique request identifier.
            idempotency_key: Optional caller idempotency key.
            route: Pre-dispatch routing validation result.
            actual_model: Model identifier reported by the execution engine.
            call_path: String describing the transport execution path.
            started_at_iso: UTC ISO-8601 start timestamp.
            ended_at_iso: UTC ISO-8601 completion timestamp.
            duration_seconds: Elapsed wall-clock execution duration in seconds.
            status: Execution status string.
            outcome_state: Outcome classification string.
            attempt_count: Dispatch attempt counter (1-based).
            output_text: Optional raw model output text for digest computation.
            result_ref: Optional reference to persisted result or job ID.
            failure_reason: Optional error description if execution failed.

        Returns:
            Serialized execution receipt JSON object.
        """
        req_norm = EXPLICIT_MODEL_ALIASES.get(
            route.requested_model.lower(), route.requested_model.lower()
        )
        act_norm = EXPLICIT_MODEL_ALIASES.get(actual_model.lower(), actual_model.lower())
        mismatch = (
            route.valid
            and req_norm not in ("", "default", str(CanonicalModelId.CODEX_DEFAULT))
            and act_norm != ""
            and req_norm != act_norm
        )
        err = (
            f"Model mismatch detected: requested '{route.requested_model}' ({req_norm}) "
            f"!= actual '{actual_model}' ({act_norm})."
            if mismatch
            else (redact_sensitive_text(failure_reason) if failure_reason else None)
        )
        rd = route.to_dict()
        routing_decision: JsonObject = {
            "valid": rd["valid"],
            "provider": rd["provider"],
            "engine": rd["engine"],
            "requested_model": rd["requested_model"],
            "resolved_model": rd["resolved_model"],
            "auth_context": rd["auth_context"],
            "reasoning_tier": rd["reasoning_tier"],
            "placeholder": rd["placeholder"],
            "error_code": rd["error_code"],
            "error_message": rd["error_message"],
        }
        receipt = ExecutionReceipt(
            request_id=request_id,
            idempotency_key=idempotency_key,
            provider=route.provider,
            engine=route.engine,
            requested_model=route.requested_model,
            resolved_model=route.resolved_model,
            actual_model=actual_model,
            model_mismatch=mismatch,
            call_path=call_path,
            routing_decision=routing_decision,
            started_at=started_at_iso,
            ended_at=ended_at_iso,
            duration_seconds=round(duration_seconds, 2),
            status=str(JobStatusKind.FAILED) if mismatch else status,
            outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE) if mismatch else outcome_state,
            attempt_count=attempt_count,
            output_digest=compute_output_digest(output_text),
            result_ref=result_ref,
            failure_reason=err,
            credentials_redacted=True,
        ).to_dict()
        return {
            "request_id": receipt["request_id"],
            "idempotency_key": receipt["idempotency_key"],
            "provider": receipt["provider"],
            "engine": receipt["engine"],
            "requested_model": receipt["requested_model"],
            "resolved_model": receipt["resolved_model"],
            "actual_model": receipt["actual_model"],
            "model_mismatch": receipt["model_mismatch"],
            "call_path": receipt["call_path"],
            "routing_decision": receipt["routing_decision"],
            "started_at": receipt["started_at"],
            "ended_at": receipt["ended_at"],
            "duration_seconds": receipt["duration_seconds"],
            "status": receipt["status"],
            "outcome_state": receipt["outcome_state"],
            "attempt_count": receipt["attempt_count"],
            "output_digest": receipt["output_digest"],
            "result_ref": receipt["result_ref"],
            "failure_reason": receipt["failure_reason"],
            "credentials_redacted": receipt["credentials_redacted"],
        }

    def build_rejected_response(
        self,
        *,
        req_id: str,
        idempotency_key: str | None,
        route: RoutingValidationResult,
        call_path: str,
        started_iso: str,
        ended_iso: str,
        prompt_text: str | None = None,
        max_input_bytes: int = 256_000,
        timeout_seconds: float = 300.0,
        extra_fields: JsonObject | None = None,
    ) -> JsonObject:
        """Build a unified pre-dispatch validation failure envelope with receipt and budget.

        Args:
            req_id: Request identifier string.
            idempotency_key: Optional caller idempotency key.
            route: Failed pre-dispatch routing validation result.
            call_path: String describing the call path where rejection occurred.
            started_iso: UTC ISO-8601 start timestamp.
            ended_iso: UTC ISO-8601 end timestamp.
            prompt_text: Optional prompt text for budget calculation.
            max_input_bytes: Maximum allowed input bytes.
            timeout_seconds: Configured timeout in seconds.
            extra_fields: Optional extra key-value fields to merge into response.

        Returns:
            Structured pre-dispatch failure JSON object.
        """
        receipt = self.build_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model="",
            call_path=call_path,
            started_at_iso=started_iso,
            ended_at_iso=ended_iso,
            duration_seconds=0.0,
            status=str(JobStatusKind.REJECTED_PRE_DISPATCH),
            outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
            attempt_count=1,
            failure_reason=route.error_message,
        )
        payload: JsonObject = {
            "ok": False,
            "status": str(JobStatusKind.REJECTED_PRE_DISPATCH),
            "outcome_state": str(OutcomeStateKind.CONFIRMED_FAILURE),
            "request_id": req_id,
            "provider": route.provider,
            "engine": route.engine,
            "requested_model": route.requested_model,
            "error_code": route.error_code,
            "error": route.error_message,
            "receipt": receipt,
        }
        if prompt_text is not None:
            payload["budget"] = evaluate_budget_envelope(
                prompt_text=prompt_text,
                max_input_bytes=max_input_bytes,
                timeout_seconds=timeout_seconds,
                elapsed_seconds=0.0,
                termination_reason=str(JobStatusKind.REJECTED_PRE_DISPATCH),
            )
        if extra_fields:
            payload.update(extra_fields)
        return payload

    def build_budget_exceeded_response(
        self,
        *,
        req_id: str,
        idempotency_key: str | None,
        route: RoutingValidationResult,
        call_path: str,
        started_iso: str,
        ended_iso: str,
        pre_budget: JsonObject,
        max_input_bytes: int,
        extra_fields: JsonObject | None = None,
    ) -> JsonObject:
        """Build a unified input-budget-exceeded failure envelope with receipt.

        Args:
            req_id: Request identifier string.
            idempotency_key: Optional caller idempotency key.
            route: Validated routing result.
            call_path: String describing the call path where budget rejection occurred.
            started_iso: UTC ISO-8601 start timestamp.
            ended_iso: UTC ISO-8601 end timestamp.
            pre_budget: Computed budget envelope dictionary.
            max_input_bytes: Maximum allowed input bytes.
            extra_fields: Optional extra key-value fields to merge into response.

        Returns:
            Structured input-budget-exceeded failure JSON object.
        """
        err_msg = (
            f"Prompt input size ({pre_budget['actual_input_bytes']} bytes) exceeds "
            f"max_input_bytes ({max_input_bytes} bytes)."
        )
        receipt = self.build_receipt(
            request_id=req_id,
            idempotency_key=idempotency_key,
            route=route,
            actual_model="",
            call_path=call_path,
            started_at_iso=started_iso,
            ended_at_iso=ended_iso,
            duration_seconds=0.0,
            status=str(JobStatusKind.FAILED),
            outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
            attempt_count=1,
            failure_reason=err_msg,
        )
        payload: JsonObject = {
            "ok": False,
            "status": str(JobStatusKind.FAILED),
            "outcome_state": str(OutcomeStateKind.CONFIRMED_FAILURE),
            "request_id": req_id,
            "provider": route.provider,
            "engine": route.engine,
            "error_code": "INPUT_BUDGET_EXCEEDED",
            "error": err_msg,
            "receipt": receipt,
            "budget": pre_budget,
        }
        if extra_fields:
            payload.update(extra_fields)
        return payload


_POLICY_ENGINE = RoutingPolicyEngine()
validate_and_resolve_route = _POLICY_ENGINE.validate_route
build_execution_receipt = _POLICY_ENGINE.build_receipt
build_rejected_pre_dispatch_response = _POLICY_ENGINE.build_rejected_response
build_input_budget_exceeded_response = _POLICY_ENGINE.build_budget_exceeded_response
