"""Routing validation DTOs, verifiable execution receipts, and input/timeout budget envelopes."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass

from codex_collab_mcp.collab_enums import JobStatusKind
from codex_collab_mcp.routing.contract_dicts import (
    BudgetEnvelopeDict,
    ExecutionReceiptDict,
    RoutingValidationDict,
)


@dataclass(frozen=True, slots=True)
class RoutingValidationResult:
    """Result of validating a `(provider, engine, model_id, reasoning_tier)` route pre-dispatch."""

    valid: bool
    provider: str
    engine: str
    requested_model: str
    resolved_model: str
    auth_context: str
    reasoning_tier: str
    placeholder: str | None = None
    error_code: str | None = None
    error_message: str | None = None

    def to_dict(self) -> RoutingValidationDict:
        """Serialize the routing validation result to a TypedDict.

        Returns:
            Serialized RoutingValidationDict mapping.
        """
        return RoutingValidationDict(
            valid=self.valid,
            provider=self.provider,
            engine=self.engine,
            requested_model=self.requested_model,
            resolved_model=self.resolved_model,
            auth_context=self.auth_context,
            reasoning_tier=self.reasoning_tier,
            placeholder=self.placeholder,
            error_code=self.error_code,
            error_message=self.error_message,
        )


@dataclass(frozen=True, slots=True)
class ExecutionReceipt:
    """Verifiable, credential-free execution receipt record for a collaboration call."""

    request_id: str
    idempotency_key: str | None
    provider: str
    engine: str
    requested_model: str
    resolved_model: str
    actual_model: str
    model_mismatch: bool
    call_path: str
    routing_decision: RoutingValidationDict
    started_at: str
    ended_at: str
    duration_seconds: float
    status: str
    outcome_state: str
    attempt_count: int = 1
    output_digest: str | None = None
    result_ref: str | None = None
    failure_reason: str | None = None
    credentials_redacted: bool = True

    def to_dict(self) -> ExecutionReceiptDict:
        """Serialize the execution receipt to a TypedDict.

        Returns:
            Serialized ExecutionReceiptDict mapping.
        """
        return ExecutionReceiptDict(
            request_id=self.request_id,
            idempotency_key=self.idempotency_key,
            provider=self.provider,
            engine=self.engine,
            requested_model=self.requested_model,
            resolved_model=self.resolved_model,
            actual_model=self.actual_model,
            model_mismatch=self.model_mismatch,
            call_path=self.call_path,
            routing_decision=self.routing_decision,
            started_at=self.started_at,
            ended_at=self.ended_at,
            duration_seconds=self.duration_seconds,
            status=self.status,
            outcome_state=self.outcome_state,
            attempt_count=self.attempt_count,
            output_digest=self.output_digest,
            result_ref=self.result_ref,
            failure_reason=self.failure_reason,
            credentials_redacted=self.credentials_redacted,
        )


@dataclass(frozen=True, slots=True)
class BudgetEnvelope:
    """Input byte/token usage and timeout budget record for a role execution."""

    max_input_bytes: int
    actual_input_bytes: int
    max_input_tokens: int
    estimated_input_tokens: int
    timeout_seconds: float
    elapsed_seconds: float
    input_budget_exceeded: bool
    time_budget_exceeded: bool
    termination_reason: str

    def to_dict(self) -> BudgetEnvelopeDict:
        """Serialize the budget envelope to a TypedDict.

        Returns:
            Serialized BudgetEnvelopeDict mapping.
        """
        return BudgetEnvelopeDict(
            max_input_bytes=self.max_input_bytes,
            actual_input_bytes=self.actual_input_bytes,
            max_input_tokens=self.max_input_tokens,
            estimated_input_tokens=self.estimated_input_tokens,
            timeout_seconds=self.timeout_seconds,
            elapsed_seconds=self.elapsed_seconds,
            input_budget_exceeded=self.input_budget_exceeded,
            time_budget_exceeded=self.time_budget_exceeded,
            termination_reason=self.termination_reason,
        )

    @classmethod
    def evaluate(
        cls,
        *,
        prompt_text: str,
        max_input_bytes: int = 256_000,
        max_input_tokens: int = 64_000,
        timeout_seconds: float = 180.0,
        elapsed_seconds: float = 0.0,
        termination_reason: str = "completed",
    ) -> BudgetEnvelopeDict:
        """Compute input byte/token usage and timeout budget status for a role execution.

        Args:
            prompt_text: Full prompt text to measure.
            max_input_bytes: Maximum allowed UTF-8 input byte count.
            max_input_tokens: Maximum allowed estimated input token count.
            timeout_seconds: Configured timeout budget in seconds.
            elapsed_seconds: Actual elapsed execution time in seconds.
            termination_reason: Reason for execution completion or termination.

        Returns:
            Typed BudgetEnvelopeDict summarizing input and timeout budget metrics.
        """
        actual_bytes = len(prompt_text.encode("utf-8"))
        est_tokens = max(1, actual_bytes // 4) if actual_bytes > 0 else 0
        in_exceeded = actual_bytes > max_input_bytes or est_tokens > max_input_tokens
        time_exceeded = elapsed_seconds >= timeout_seconds and termination_reason in (
            "timeout_budget_reached",
            JobStatusKind.TIMED_OUT_UNKNOWN,
        )
        reason = str(JobStatusKind.INPUT_BUDGET_EXCEEDED) if in_exceeded else termination_reason
        return cls(
            max_input_bytes,
            actual_bytes,
            max_input_tokens,
            est_tokens,
            timeout_seconds,
            round(elapsed_seconds, 2),
            in_exceeded,
            time_exceeded,
            reason,
        ).to_dict()


def compute_sha256_hex(content: str | bytes) -> str:
    """Return a deterministic SHA-256 hex digest of a UTF-8 string or raw bytes.

    Args:
        content: Input UTF-8 string or raw bytes to hash.

    Returns:
        Lowercase 64-character SHA-256 hexadecimal digest.
    """
    raw = content if isinstance(content, bytes) else content.encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def compute_output_digest(output_text: str | None) -> str | None:
    """Compute a `sha256:...` digest for non-empty model output text.

    Args:
        output_text: Raw output string or None.

    Returns:
        Prefixed SHA-256 digest string when output_text is non-empty, else None.
    """
    return f"sha256:{compute_sha256_hex(output_text)}" if output_text else None


evaluate_budget_envelope = BudgetEnvelope.evaluate
