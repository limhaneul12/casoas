"""Internal frozen dataclass DTOs for routing, budgets, and background job handles."""

from __future__ import annotations

from dataclasses import dataclass

from app.schemas.boundary_schemas import JsonObject
from app.schemas.inner_dto.dto_dict_types import (
    BackgroundJobDeduplicatedResultDict,
    BackgroundJobDispatchedResultDict,
    CouncilRoundRoleErrorResultDict,
    ExternalMcpOAuthWarningDict,
    GatewayRejectedAuthBoundaryDict,
    HandoffResolutionVerdictNoteDict,
    InputBudgetExceededResponseDict,
    JobTelemetrySummaryDict,
    PreDispatchRejectedResponseDict,
    ResumedRoleResponseDict,
    VerificationGateReceiptDict,
)


@dataclass(frozen=True, slots=True)
class ExternalMcpOAuthWarning:
    """Internal DTO for an isolated downstream MCP OAuth `invalid_grant` warning."""

    external_mcp_server: str
    error_type: str
    message: str
    affects_antigravity_rpc: bool = False
    affects_codex_model_routing: bool = False
    auto_reauth_triggered: bool = False

    def to_dict(self) -> ExternalMcpOAuthWarningDict:
        """Serialize the external OAuth warning to a TypedDict.

        Returns:
            Typed `ExternalMcpOAuthWarningDict` mapping.
        """
        return ExternalMcpOAuthWarningDict(
            external_mcp_server=self.external_mcp_server,
            error_type=self.error_type,
            message=self.message,
            affects_antigravity_rpc=self.affects_antigravity_rpc,
            affects_codex_model_routing=self.affects_codex_model_routing,
            auto_reauth_triggered=self.auto_reauth_triggered,
        )


@dataclass(frozen=True, slots=True)
class JobTelemetrySummary:
    """Internal DTO for aggregated SQLite job execution telemetry."""

    total_jobs: int = 0
    completed_jobs: int = 0
    failed_jobs: int = 0
    timed_out_unknown_jobs: int = 0
    running_jobs: int = 0
    cancelled_jobs: int = 0
    avg_duration_seconds: float = 0.0

    def to_dict(self) -> JobTelemetrySummaryDict:
        """Serialize the job telemetry summary to a TypedDict.

        Returns:
            Typed `JobTelemetrySummaryDict` mapping.
        """
        return JobTelemetrySummaryDict(
            total_jobs=self.total_jobs,
            completed_jobs=self.completed_jobs,
            failed_jobs=self.failed_jobs,
            timed_out_unknown_jobs=self.timed_out_unknown_jobs,
            running_jobs=self.running_jobs,
            cancelled_jobs=self.cancelled_jobs,
            avg_duration_seconds=self.avg_duration_seconds,
        )


@dataclass(frozen=True, slots=True)
class PreDispatchRejectedResponse:
    """Internal DTO for a pre-dispatch routing rejection envelope."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    provider: str
    engine: str
    requested_model: str
    error_code: str | None
    error: str | None
    receipt: JsonObject

    def to_dict(self) -> PreDispatchRejectedResponseDict:
        """Serialize the pre-dispatch rejection envelope to a TypedDict.

        Returns:
            Typed `PreDispatchRejectedResponseDict` mapping.
        """
        return PreDispatchRejectedResponseDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            provider=self.provider,
            engine=self.engine,
            requested_model=self.requested_model,
            error_code=self.error_code,
            error=self.error,
            receipt=self.receipt,
        )


@dataclass(frozen=True, slots=True)
class InputBudgetExceededResponse:
    """Internal DTO for an input-budget-exceeded rejection envelope."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    provider: str
    engine: str
    error_code: str
    error: str
    receipt: JsonObject
    budget: JsonObject

    def to_dict(self) -> InputBudgetExceededResponseDict:
        """Serialize the input-budget-exceeded envelope to a TypedDict.

        Returns:
            Typed `InputBudgetExceededResponseDict` mapping.
        """
        return InputBudgetExceededResponseDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            provider=self.provider,
            engine=self.engine,
            error_code=self.error_code,
            error=self.error,
            receipt=self.receipt,
            budget=self.budget,
        )


@dataclass(frozen=True, slots=True)
class GatewayRejectedAuthBoundary:
    """Internal DTO for authentication boundary metadata attached on gateway route rejection."""

    auth_context: str
    antigravity_rpc_affected: bool = False
    auto_credential_refresh_performed: bool = False

    def to_dict(self) -> GatewayRejectedAuthBoundaryDict:
        """Serialize the gateway rejected auth boundary to a TypedDict.

        Returns:
            Typed `GatewayRejectedAuthBoundaryDict` mapping.
        """
        return GatewayRejectedAuthBoundaryDict(
            auth_context=self.auth_context,
            antigravity_rpc_affected=self.antigravity_rpc_affected,
            auto_credential_refresh_performed=self.auto_credential_refresh_performed,
        )


@dataclass(frozen=True, slots=True)
class HandoffResolutionVerdictNote:
    """Internal DTO for a structured verdict and summary note when resolving a handoff ticket."""

    verdict: str
    summary: str

    def to_dict(self) -> HandoffResolutionVerdictNoteDict:
        """Serialize the handoff resolution verdict note to a TypedDict.

        Returns:
            Typed `HandoffResolutionVerdictNoteDict` mapping.
        """
        return HandoffResolutionVerdictNoteDict(verdict=self.verdict, summary=self.summary)


@dataclass(frozen=True, slots=True)
class CouncilRoundRoleErrorResult:
    """Internal DTO for an error or timeout envelope during a Council Round 2+ re-evaluation."""

    ok: bool
    status: str
    outcome_state: str
    error: str

    def to_dict(self) -> CouncilRoundRoleErrorResultDict:
        """Serialize the council round role error result to a TypedDict.

        Returns:
            Typed `CouncilRoundRoleErrorResultDict` mapping.
        """
        return CouncilRoundRoleErrorResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            error=self.error,
        )


@dataclass(frozen=True, slots=True)
class BackgroundJobDeduplicatedResult:
    """Internal DTO for a deduplicated background job dispatch handle."""

    job_id: str
    request_id: str
    status: str
    outcome_state: str
    attempt_count: int
    task_type: str
    receipt: JsonObject | None
    async_dispatched: bool = False
    deduplicated_existing_request: bool = True

    def to_dict(self) -> BackgroundJobDeduplicatedResultDict:
        """Serialize the deduplicated background job handle to a TypedDict.

        Returns:
            Typed `BackgroundJobDeduplicatedResultDict` mapping.
        """
        return BackgroundJobDeduplicatedResultDict(
            async_dispatched=self.async_dispatched,
            deduplicated_existing_request=self.deduplicated_existing_request,
            job_id=self.job_id,
            request_id=self.request_id,
            status=self.status,
            outcome_state=self.outcome_state,
            attempt_count=self.attempt_count,
            task_type=self.task_type,
            receipt=self.receipt,
        )


@dataclass(frozen=True, slots=True)
class BackgroundJobDispatchedResult:
    """Internal DTO for a newly spawned background job handle."""

    job_id: str
    request_id: str
    status: str
    outcome_state: str
    task_type: str
    message: str
    async_dispatched: bool = True

    def to_dict(self) -> BackgroundJobDispatchedResultDict:
        """Serialize the spawned background job handle to a TypedDict.

        Returns:
            Typed `BackgroundJobDispatchedResultDict` mapping.
        """
        return BackgroundJobDispatchedResultDict(
            async_dispatched=self.async_dispatched,
            job_id=self.job_id,
            request_id=self.request_id,
            status=self.status,
            outcome_state=self.outcome_state,
            task_type=self.task_type,
            message=self.message,
        )


@dataclass(frozen=True, slots=True)
class VerificationGateReceipt:
    """Internal DTO for a bridge-executed verification gate receipt."""

    command: str
    exit_code: int
    passed: bool
    duration_seconds: float
    stdout_digest: str
    origin: str

    def to_dict(self) -> VerificationGateReceiptDict:
        """Serialize the verification gate receipt to a TypedDict.

        Returns:
            Typed `VerificationGateReceiptDict` mapping.
        """
        return VerificationGateReceiptDict(
            command=self.command,
            exit_code=self.exit_code,
            passed=self.passed,
            duration_seconds=self.duration_seconds,
            stdout_digest=self.stdout_digest,
            origin=self.origin,
        )


@dataclass(frozen=True, slots=True)
class ResumedRoleResponse:
    """Internal DTO for a resumed role response from a verified triad checkpoint."""

    ok: bool
    status: str
    outcome_state: str
    resumed_from_collaboration_id: str
    structured_output: JsonObject

    def to_dict(self) -> ResumedRoleResponseDict:
        """Serialize the resumed role response to a TypedDict.

        Returns:
            Typed `ResumedRoleResponseDict` mapping.
        """
        return ResumedRoleResponseDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            resumed_from_collaboration_id=self.resumed_from_collaboration_id,
            structured_output=self.structured_output,
        )


__all__ = [
    "BackgroundJobDeduplicatedResult",
    "BackgroundJobDispatchedResult",
    "CouncilRoundRoleErrorResult",
    "ExternalMcpOAuthWarning",
    "GatewayRejectedAuthBoundary",
    "HandoffResolutionVerdictNote",
    "InputBudgetExceededResponse",
    "JobTelemetrySummary",
    "PreDispatchRejectedResponse",
    "ResumedRoleResponse",
    "VerificationGateReceipt",
]
