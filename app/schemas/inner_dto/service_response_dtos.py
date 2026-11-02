"""Internal frozen dataclass DTOs for MCP tool responses, health checks, and decision packs."""

from __future__ import annotations

from dataclasses import dataclass

from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types import (
    AgyModelConfigEntryDict,
    AgyRpcBoundaryHealthDict,
    AuthBoundariesHealthDict,
    BridgeHealthResponseDict,
    CodexCliBoundaryHealthDict,
    CodexJobRecordDict,
    DecisionAuthoritativeEvidenceDict,
    DecisionCallerContentDict,
    DecisionDigestPayloadDict,
    DecisionPackResponseDict,
    HandoffCreateResponseDict,
    HandoffInboxResponseDict,
    HandoffResolveResponseDict,
    HandoffTicketDict,
    JobListResponseDict,
    JobStatusResponseDict,
    JobTelemetrySummaryDict,
    ModelSpecDict,
    PythonRuntimeInfoDict,
    RoutingContractHealthDict,
    ToolErrorResponseDict,
)


@dataclass(frozen=True, slots=True)
class ToolErrorResponse:
    """Internal DTO for a standard resource-not-found or validation error payload."""

    error: str
    ok: bool = False

    def to_dict(self) -> ToolErrorResponseDict:
        """Serialize the tool error response to a TypedDict.

        Returns:
            Typed `ToolErrorResponseDict` mapping.
        """
        return ToolErrorResponseDict(ok=self.ok, error=self.error)


@dataclass(frozen=True, slots=True)
class JobStatusResponse:
    """Internal DTO for `codex_job_status` response payload."""

    ok: bool
    job: CodexJobRecordDict
    outcome_state: str
    cancel_requested: bool

    def to_dict(self) -> JobStatusResponseDict:
        """Serialize the job status response to a TypedDict.

        Returns:
            Typed `JobStatusResponseDict` mapping.
        """
        return JobStatusResponseDict(
            ok=self.ok,
            job=self.job,
            outcome_state=self.outcome_state,
            cancel_requested=self.cancel_requested,
        )


@dataclass(frozen=True, slots=True)
class JobListResponse:
    """Internal DTO for `codex_job_list` response payload."""

    ok: bool
    workspace_filter: str | None
    jobs: list[CodexJobRecordDict]

    def to_dict(self) -> JobListResponseDict:
        """Serialize the job list response to a TypedDict.

        Returns:
            Typed `JobListResponseDict` mapping.
        """
        return JobListResponseDict(
            ok=self.ok,
            workspace_filter=self.workspace_filter,
            jobs=self.jobs,
        )


@dataclass(frozen=True, slots=True)
class HandoffCreateResponse:
    """Internal DTO for `heterarchy_handoff_create` response payload."""

    ok: bool
    ticket: HandoffTicketDict
    collaboration_role_state: str
    execution_completed: bool
    consensus_eligible: bool
    message: str

    def to_dict(self) -> HandoffCreateResponseDict:
        """Serialize the handoff creation response to a TypedDict.

        Returns:
            Typed `HandoffCreateResponseDict` mapping.
        """
        return HandoffCreateResponseDict(
            ok=self.ok,
            ticket=self.ticket,
            collaboration_role_state=self.collaboration_role_state,
            execution_completed=self.execution_completed,
            consensus_eligible=self.consensus_eligible,
            message=self.message,
        )


@dataclass(frozen=True, slots=True)
class HandoffInboxResponse:
    """Internal DTO for `heterarchy_handoff_inbox` response payload."""

    ok: bool
    recipient_filter: str | None
    status_filter: str | None
    workspace_filter: str | None
    count: int
    tickets: list[HandoffTicketDict]

    def to_dict(self) -> HandoffInboxResponseDict:
        """Serialize the handoff inbox response to a TypedDict.

        Returns:
            Typed `HandoffInboxResponseDict` mapping.
        """
        return HandoffInboxResponseDict(
            ok=self.ok,
            recipient_filter=self.recipient_filter,
            status_filter=self.status_filter,
            workspace_filter=self.workspace_filter,
            count=self.count,
            tickets=self.tickets,
        )


@dataclass(frozen=True, slots=True)
class HandoffResolveResponse:
    """Internal DTO for `heterarchy_handoff_resolve` response payload."""

    ok: bool
    ticket: HandoffTicketDict

    def to_dict(self) -> HandoffResolveResponseDict:
        """Serialize the handoff resolve response to a TypedDict.

        Returns:
            Typed `HandoffResolveResponseDict` mapping.
        """
        return HandoffResolveResponseDict(ok=self.ok, ticket=self.ticket)


@dataclass(frozen=True, slots=True)
class AgyRpcBoundaryHealth:
    """Internal DTO for Antigravity local RPC authentication boundary status."""

    auth_context: str
    connected: bool
    endpoint: str | None
    csrf_token_redacted: bool
    independent_of_codex_cli_oauth: bool
    circuit_breaker_open: bool

    def to_dict(self) -> AgyRpcBoundaryHealthDict:
        """Serialize the Antigravity RPC boundary health to a TypedDict.

        Returns:
            Typed `AgyRpcBoundaryHealthDict` mapping.
        """
        return AgyRpcBoundaryHealthDict(
            auth_context=self.auth_context,
            connected=self.connected,
            endpoint=self.endpoint,
            csrf_token_redacted=self.csrf_token_redacted,
            independent_of_codex_cli_oauth=self.independent_of_codex_cli_oauth,
            circuit_breaker_open=self.circuit_breaker_open,
        )


@dataclass(frozen=True, slots=True)
class CodexCliBoundaryHealth:
    """Internal DTO for Codex CLI OAuth authentication boundary status."""

    auth_context: str
    cli_binary_ready: bool
    independent_of_antigravity_rpc: bool
    auto_reauth_enabled: bool

    def to_dict(self) -> CodexCliBoundaryHealthDict:
        """Serialize the Codex CLI boundary health to a TypedDict.

        Returns:
            Typed `CodexCliBoundaryHealthDict` mapping.
        """
        return CodexCliBoundaryHealthDict(
            auth_context=self.auth_context,
            cli_binary_ready=self.cli_binary_ready,
            independent_of_antigravity_rpc=self.independent_of_antigravity_rpc,
            auto_reauth_enabled=self.auto_reauth_enabled,
        )


@dataclass(frozen=True, slots=True)
class AuthBoundariesHealth:
    """Internal DTO for combined authentication boundary health status."""

    antigravity_rpc: AgyRpcBoundaryHealthDict
    codex_cli_oauth: CodexCliBoundaryHealthDict

    def to_dict(self) -> AuthBoundariesHealthDict:
        """Serialize the combined auth boundaries health to a TypedDict.

        Returns:
            Typed `AuthBoundariesHealthDict` mapping.
        """
        return AuthBoundariesHealthDict(
            antigravity_rpc=self.antigravity_rpc,
            codex_cli_oauth=self.codex_cli_oauth,
        )


@dataclass(frozen=True, slots=True)
class RoutingContractHealth:
    """Internal DTO for verified pre-dispatch routing contract status."""

    verified_at_utc: str
    silent_fallback_enabled: bool
    claude_tier_policy: str
    advertised_antigravity_models: list[ModelSpecDict]
    routable_antigravity_models: list[str]
    antigravity_routable: bool
    advertised_codex_models: list[str]
    routable_codex_models: list[str]
    codex_cli_routable: bool

    def to_dict(self) -> RoutingContractHealthDict:
        """Serialize the routing contract health to a TypedDict.

        Returns:
            Typed `RoutingContractHealthDict` mapping.
        """
        return RoutingContractHealthDict(
            verified_at_utc=self.verified_at_utc,
            silent_fallback_enabled=self.silent_fallback_enabled,
            claude_tier_policy=self.claude_tier_policy,
            advertised_antigravity_models=self.advertised_antigravity_models,
            routable_antigravity_models=self.routable_antigravity_models,
            antigravity_routable=self.antigravity_routable,
            advertised_codex_models=self.advertised_codex_models,
            routable_codex_models=self.routable_codex_models,
            codex_cli_routable=self.codex_cli_routable,
        )


@dataclass(frozen=True, slots=True)
class BridgeHealthResponse:
    """Internal DTO for `codex_bridge_health` response payload."""

    ok: bool
    verified_at_utc: str
    python_runtime: PythonRuntimeInfoDict
    codex_binary: str
    codex_version: str
    agy_bridge_ready: bool
    agy_endpoint: str | None
    antigravity_models: list[AgyModelConfigEntryDict]
    routing_contract: RoutingContractHealthDict
    auth_boundaries: AuthBoundariesHealthDict
    active_background_jobs: int
    pending_argon_handoffs: int
    pending_sonnet_handoffs: int
    pending_opus_handoffs: int
    pending_codex_handoffs: int
    job_telemetry: JobTelemetrySummaryDict

    def to_dict(self) -> BridgeHealthResponseDict:
        """Serialize the bridge health response to a TypedDict.

        Returns:
            Typed `BridgeHealthResponseDict` mapping.
        """
        return BridgeHealthResponseDict(
            ok=self.ok,
            verified_at_utc=self.verified_at_utc,
            python_runtime=self.python_runtime,
            codex_binary=self.codex_binary,
            codex_version=self.codex_version,
            agy_bridge_ready=self.agy_bridge_ready,
            agy_endpoint=self.agy_endpoint,
            antigravity_models=self.antigravity_models,
            routing_contract=self.routing_contract,
            auth_boundaries=self.auth_boundaries,
            active_background_jobs=self.active_background_jobs,
            pending_argon_handoffs=self.pending_argon_handoffs,
            pending_sonnet_handoffs=self.pending_sonnet_handoffs,
            pending_opus_handoffs=self.pending_opus_handoffs,
            pending_codex_handoffs=self.pending_codex_handoffs,
            job_telemetry=self.job_telemetry,
        )


@dataclass(frozen=True, slots=True)
class DecisionCallerContent:
    """Internal DTO for caller-supplied decision notes and scope fences."""

    project_scope: str
    topic_title: str
    argon_architecture_intent: str
    sonnet_craft_or_review_summary: str
    codex_audit_or_synthesis_summary: str
    final_resolution: str
    verification_evidence: str
    requested_collaboration_id: str | None
    requested_workspace_dir: str | None
    expected_scope_digest: str | None

    def to_dict(self) -> DecisionCallerContentDict:
        """Serialize the caller decision content to a TypedDict.

        Returns:
            Typed `DecisionCallerContentDict` mapping.
        """
        return DecisionCallerContentDict(
            project_scope=self.project_scope,
            topic_title=self.topic_title,
            argon_architecture_intent=self.argon_architecture_intent,
            sonnet_craft_or_review_summary=self.sonnet_craft_or_review_summary,
            codex_audit_or_synthesis_summary=self.codex_audit_or_synthesis_summary,
            final_resolution=self.final_resolution,
            verification_evidence=self.verification_evidence,
            requested_collaboration_id=self.requested_collaboration_id,
            requested_workspace_dir=self.requested_workspace_dir,
            expected_scope_digest=self.expected_scope_digest,
        )


@dataclass(frozen=True, slots=True)
class DecisionAuthoritativeEvidence:
    """Internal DTO for authoritative triad evidence fields in a decision pack digest."""

    evidence_status: str
    collaboration_id: JsonValue
    source_task_digest: str | None
    workspace_dir: str | None
    scope_digest: str | None
    snapshot_digest: str | None
    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    review_approved: bool
    resolution_verified: bool

    def to_dict(self) -> DecisionAuthoritativeEvidenceDict:
        """Serialize authoritative decision evidence to a TypedDict.

        Returns:
            Typed `DecisionAuthoritativeEvidenceDict` mapping.
        """
        return DecisionAuthoritativeEvidenceDict(
            evidence_status=self.evidence_status,
            collaboration_id=self.collaboration_id,
            source_task_digest=self.source_task_digest,
            workspace_dir=self.workspace_dir,
            scope_digest=self.scope_digest,
            snapshot_digest=self.snapshot_digest,
            collaboration_status=self.collaboration_status,
            consensus_status=self.consensus_status,
            consensus_reached=self.consensus_reached,
            review_approved=self.review_approved,
            resolution_verified=self.resolution_verified,
        )


@dataclass(frozen=True, slots=True)
class DecisionDigestPayload:
    """Internal DTO hashed to produce the deterministic Alexandria idempotency key."""

    caller_content: DecisionCallerContentDict
    authoritative_evidence: DecisionAuthoritativeEvidenceDict

    def to_dict(self) -> DecisionDigestPayloadDict:
        """Serialize the decision digest payload to a TypedDict.

        Returns:
            Typed `DecisionDigestPayloadDict` mapping.
        """
        return DecisionDigestPayloadDict(
            caller_content=self.caller_content,
            authoritative_evidence=self.authoritative_evidence,
        )


@dataclass(frozen=True, slots=True)
class DecisionPackResponse:
    """Internal DTO for `codex_alexandria_decision_pack` response payload."""

    ok: bool
    project_scope: str
    requested_collaboration_id: str | None
    collaboration_id: JsonValue
    evidence_status: str
    source_task_digest: str | None
    workspace_dir: str | None
    expected_workspace_dir: str | None
    expected_scope_digest: str | None
    evidence_scope: JsonObject | None
    role_evidence_dimensions: JsonObject
    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    review_approved: bool
    resolution_verified: bool
    idempotency_key: str
    logical_identity: str
    markdown_content: str
    recommended_next_step: str

    def to_dict(self) -> DecisionPackResponseDict:
        """Serialize the decision pack response to a TypedDict.

        Returns:
            Typed `DecisionPackResponseDict` mapping.
        """
        return DecisionPackResponseDict(
            ok=self.ok,
            project_scope=self.project_scope,
            requested_collaboration_id=self.requested_collaboration_id,
            collaboration_id=self.collaboration_id,
            evidence_status=self.evidence_status,
            source_task_digest=self.source_task_digest,
            workspace_dir=self.workspace_dir,
            expected_workspace_dir=self.expected_workspace_dir,
            expected_scope_digest=self.expected_scope_digest,
            evidence_scope=self.evidence_scope,
            role_evidence_dimensions=self.role_evidence_dimensions,
            collaboration_status=self.collaboration_status,
            consensus_status=self.consensus_status,
            consensus_reached=self.consensus_reached,
            review_approved=self.review_approved,
            resolution_verified=self.resolution_verified,
            idempotency_key=self.idempotency_key,
            logical_identity=self.logical_identity,
            markdown_content=self.markdown_content,
            recommended_next_step=self.recommended_next_step,
        )


__all__ = [
    "AgyRpcBoundaryHealth",
    "AuthBoundariesHealth",
    "BridgeHealthResponse",
    "CodexCliBoundaryHealth",
    "DecisionAuthoritativeEvidence",
    "DecisionCallerContent",
    "DecisionDigestPayload",
    "DecisionPackResponse",
    "HandoffCreateResponse",
    "HandoffInboxResponse",
    "HandoffResolveResponse",
    "JobListResponse",
    "JobStatusResponse",
    "RoutingContractHealth",
    "ToolErrorResponse",
]
