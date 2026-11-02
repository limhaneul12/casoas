"""Provider/engine/model routing validation, execution receipts, and budget evaluation."""

from __future__ import annotations

from app.core.collab_enums import (
    AuthContextKind,
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    OutcomeStateKind,
    ProviderKind,
    ReasoningTier,
)
from app.core.model_contract_enums import (
    EngineAliasKind,
    ProviderAliasKind,
    RoutingErrorCode,
)
from app.routing.auth_redaction import redact_sensitive_text
from app.routing.model_specs import (
    ANTIGRAVITY_MODEL_SPECS,
    CODEX_MODEL_ALIASES,
    CODEX_SUPPORTED_MODELS,
    EXPLICIT_MODEL_ALIASES,
    VALID_REASONING_TIERS,
)
from app.routing.receipt_budget import (
    BudgetEnvelope,
    ExecutionReceipt,
    RoutingValidationResult,
    compute_output_digest,
)
from app.schemas.boundary_schemas import JsonObject, as_json_object
from app.schemas.inner_dto.contract_dtos import (
    InputBudgetExceededResponse,
    PreDispatchRejectedResponse,
)
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import (
    BudgetEnvelopeDict,
    ExecutionReceiptDict,
)


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
        if e in (EngineAliasKind.CLI, EngineKind.CODEX_CLI, EngineAliasKind.CODEX_CLI_DASH):
            return str(EngineKind.CODEX_CLI)
        if e in (
            EngineAliasKind.AGY,
            ProviderKind.ANTIGRAVITY,
            agy_rpc,
            EngineAliasKind.ANTIGRAVITY_RPC_DASH,
            EngineAliasKind.RPC,
        ):
            return agy_rpc
        return (
            str(EngineKind.HTTP)
            if e in (EngineKind.HTTP, ProviderKind.LOCAL_HTTP, EngineAliasKind.OPENAI_HTTP)
            else e
        )

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
            valid=False,
            provider=provider,
            engine=engine,
            requested_model=req_model,
            resolved_model=resolved,
            auth_context=auth,
            reasoning_tier=tier,
            placeholder=placeholder,
            error_code=code,
            error_message=msg,
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
        eng = self.normalize_engine(engine=engine)
        raw_m, norm_p = (model or "").strip(), (provider or "").strip().lower()
        norm_m, tier = raw_m.lower(), (reasoning_effort or ReasoningTier.HIGH).strip().lower()
        if tier not in VALID_REASONING_TIERS:
            msg = f"Unsupported tier '{reasoning_effort}'."
            return self._reject_route(
                provider=norm_p or "unknown",
                engine=eng,
                req_model=raw_m or "default",
                tier=tier,
                code=str(RoutingErrorCode.INVALID_REASONING_TIER),
                msg=msg,
            )
        if eng not in (EngineKind.CODEX_CLI, EngineKind.ANTIGRAVITY_LOCAL_RPC, EngineKind.HTTP):
            msg = f"Unsupported engine '{engine}'."
            return self._reject_route(
                provider=norm_p or "unknown",
                engine=eng,
                req_model=raw_m or "default",
                tier=tier,
                code=str(RoutingErrorCode.UNSUPPORTED_ENGINE),
                msg=msg,
            )
        if eng == EngineKind.CODEX_CLI:
            codex_p = str(ProviderKind.OPENAI_CODEX)
            oauth = str(AuthContextKind.CODEX_CHATGPT_OAUTH)
            raw_eff = norm_m or str(CanonicalModelId.CODEX_DEFAULT)
            eff = CODEX_MODEL_ALIASES.get(raw_eff, raw_eff.replace("gpt-6-1-", "gpt-6.1-"))
            if norm_p and norm_p not in (
                ProviderAliasKind.OPENAI,
                codex_p,
                ProviderAliasKind.CODEX,
            ):
                msg = f"Provider '{provider}' mismatch."
                return self._reject_route(
                    provider=norm_p,
                    engine=eng,
                    req_model=raw_m or eff,
                    tier=tier,
                    code=str(RoutingErrorCode.PROVIDER_ENGINE_MISMATCH),
                    msg=msg,
                    auth=oauth,
                )
            if (
                eff in ANTIGRAVITY_MODEL_SPECS
                or eff in EXPLICIT_MODEL_ALIASES
                or eff.startswith(("gemini-", "claude-", "model_placeholder_"))
            ):
                msg = f"Model '{raw_m}' is unsupported on 'codex_cli'."
                return self._reject_route(
                    provider=norm_p or codex_p,
                    engine=eng,
                    req_model=raw_m,
                    tier=tier,
                    code=str(RoutingErrorCode.MODEL_NOT_SUPPORTED_BY_ENGINE),
                    msg=msg,
                    auth=oauth,
                )
            if eff not in CODEX_SUPPORTED_MODELS:
                msg = f"Model '{raw_m}' is not in the Codex CLI contract."
                return self._reject_route(
                    provider=codex_p,
                    engine=eng,
                    req_model=raw_m,
                    tier=tier,
                    code=str(RoutingErrorCode.UNRECOGNIZED_CODEX_MODEL),
                    msg=msg,
                    auth=oauth,
                )
            return RoutingValidationResult(
                valid=True,
                provider=codex_p,
                engine=eng,
                requested_model=raw_m or eff,
                resolved_model=eff,
                auth_context=oauth,
                reasoning_tier=tier,
                placeholder=None,
            )
        if eng == EngineKind.ANTIGRAVITY_LOCAL_RPC:
            agy_p = str(ProviderKind.ANTIGRAVITY)
            csrf = str(AuthContextKind.ANTIGRAVITY_LOCAL_CSRF)
            if norm_p and norm_p not in (
                agy_p,
                ProviderAliasKind.GOOGLE_ANTIGRAVITY,
                ProviderAliasKind.AGY,
            ):
                msg = f"Provider '{provider}' mismatch."
                return self._reject_route(
                    provider=norm_p,
                    engine=eng,
                    req_model=raw_m,
                    tier=tier,
                    code=str(RoutingErrorCode.PROVIDER_ENGINE_MISMATCH),
                    msg=msg,
                    auth=csrf,
                )
            res_id = EXPLICIT_MODEL_ALIASES.get(norm_m, norm_m) or str(
                CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM
            )
            if res_id not in ANTIGRAVITY_MODEL_SPECS:
                msg = f"Unsupported model '{raw_m}'."
                return self._reject_route(
                    provider=agy_p,
                    engine=eng,
                    req_model=raw_m,
                    tier=tier,
                    code=str(RoutingErrorCode.MODEL_NOT_SUPPORTED_BY_ENGINE),
                    msg=msg,
                    auth=csrf,
                )
            spec = ANTIGRAVITY_MODEL_SPECS[res_id]
            if (
                enforce_claude_medium_policy
                and spec.model_id.startswith("claude-")
                and spec.reasoning_tier != ReasoningTier.MEDIUM
            ):
                msg = f"Claude model '{raw_m}' violates Medium-only policy."
                return self._reject_route(
                    provider=agy_p,
                    engine=eng,
                    req_model=raw_m,
                    tier=spec.reasoning_tier,
                    code=str(RoutingErrorCode.CLAUDE_TIER_POLICY_VIOLATION),
                    msg=msg,
                    auth=csrf,
                    resolved=spec.model_id,
                    placeholder=spec.placeholder,
                )
            return RoutingValidationResult(
                valid=True,
                provider=spec.provider,
                engine=spec.engine,
                requested_model=raw_m or spec.model_id,
                resolved_model=spec.model_id,
                auth_context=spec.auth_context,
                reasoning_tier=spec.reasoning_tier,
                placeholder=spec.placeholder,
            )
        eff_http = norm_m or str(CanonicalModelId.GPT_6_1_SOL)
        http_p = norm_p or str(ProviderKind.LOCAL_HTTP)
        http_auth = str(AuthContextKind.HTTP_BEARER)
        return RoutingValidationResult(
            valid=True,
            provider=http_p,
            engine=str(EngineKind.HTTP),
            requested_model=raw_m or eff_http,
            resolved_model=eff_http,
            auth_context=http_auth,
            reasoning_tier=tier,
            placeholder=None,
        )

    def build_receipt(
        self,
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
    ) -> ExecutionReceiptDict:
        """Build a verifiable, credential-free execution receipt TypedDict.

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
            Serialized ExecutionReceiptDict TypedDict.
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
            else (redact_sensitive_text(text=failure_reason) if failure_reason else None)
        )
        return ExecutionReceipt(
            request_id=request_id,
            idempotency_key=idempotency_key,
            provider=route.provider,
            engine=route.engine,
            requested_model=route.requested_model,
            resolved_model=route.resolved_model,
            actual_model=actual_model,
            model_mismatch=mismatch,
            call_path=call_path,
            routing_decision=route.to_dict(),
            started_at=started_at_iso,
            ended_at=ended_at_iso,
            duration_seconds=round(duration_seconds, 2),
            status=str(JobStatusKind.FAILED) if mismatch else status,
            outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE) if mismatch else outcome_state,
            attempt_count=attempt_count,
            output_digest=compute_output_digest(output_text=output_text),
            result_ref=result_ref,
            failure_reason=err,
            credentials_redacted=True,
        ).to_dict()

    def build_rejected_response(
        self,
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
        payload = as_json_object(
            record=PreDispatchRejectedResponse(
                ok=False,
                status=str(JobStatusKind.REJECTED_PRE_DISPATCH),
                outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
                request_id=req_id,
                provider=route.provider,
                engine=route.engine,
                requested_model=route.requested_model,
                error_code=route.error_code,
                error=route.error_message,
                receipt=as_json_object(record=receipt),
            ).to_dict()
        )
        if prompt_text is not None:
            payload["budget"] = as_json_object(
                record=BudgetEnvelope.evaluate(
                    prompt_text=prompt_text,
                    max_input_bytes=max_input_bytes,
                    timeout_seconds=timeout_seconds,
                    elapsed_seconds=0.0,
                    termination_reason=str(JobStatusKind.REJECTED_PRE_DISPATCH),
                )
            )
        if extra_fields:
            payload.update(extra_fields)
        return payload

    def build_budget_exceeded_response(
        self,
        req_id: str,
        idempotency_key: str | None,
        route: RoutingValidationResult,
        call_path: str,
        started_iso: str,
        ended_iso: str,
        pre_budget: BudgetEnvelopeDict | JsonObject,
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
            pre_budget: Computed budget envelope TypedDict or JSON dictionary.
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
        payload = as_json_object(
            record=InputBudgetExceededResponse(
                ok=False,
                status=str(JobStatusKind.FAILED),
                outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
                request_id=req_id,
                provider=route.provider,
                engine=route.engine,
                error_code=str(RoutingErrorCode.INPUT_BUDGET_EXCEEDED),
                error=err_msg,
                receipt=as_json_object(record=receipt),
                budget=as_json_object(record=pre_budget),
            ).to_dict()
        )
        if extra_fields:
            payload.update(extra_fields)
        return payload
