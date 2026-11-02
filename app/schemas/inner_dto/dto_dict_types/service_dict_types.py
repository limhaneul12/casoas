"""TypedDict return-type contracts for `service_dtos.py` `.to_dict(self)` methods."""

from __future__ import annotations

from typing import TypedDict

from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import (
    CodexJobRecordDict,
    HandoffTicketDict,
    JobTelemetrySummaryDict,
    ModelSpecDict,
    PythonRuntimeInfoDict,
)
from app.schemas.inner_dto.dto_dict_types.transport_dict_types import (
    AgyModelConfigEntryDict,
)


class ToolErrorResponseDict(TypedDict):
    """Serialized representation of a `ToolErrorResponse` dataclass."""

    ok: bool
    error: str


class JobStatusResponseDict(TypedDict):
    """Serialized representation of a `JobStatusResponse` dataclass."""

    ok: bool
    job: CodexJobRecordDict
    outcome_state: str
    cancel_requested: bool


class JobListResponseDict(TypedDict):
    """Serialized representation of a `JobListResponse` dataclass."""

    ok: bool
    workspace_filter: str | None
    jobs: list[CodexJobRecordDict]


class HandoffCreateResponseDict(TypedDict):
    """Serialized representation of a `HandoffCreateResponse` dataclass."""

    ok: bool
    ticket: HandoffTicketDict
    collaboration_role_state: str
    execution_completed: bool
    consensus_eligible: bool
    message: str


class HandoffInboxResponseDict(TypedDict):
    """Serialized representation of a `HandoffInboxResponse` dataclass."""

    ok: bool
    recipient_filter: str | None
    status_filter: str | None
    workspace_filter: str | None
    count: int
    tickets: list[HandoffTicketDict]


class HandoffResolveResponseDict(TypedDict):
    """Serialized representation of a `HandoffResolveResponse` dataclass."""

    ok: bool
    ticket: HandoffTicketDict


class AgyRpcBoundaryHealthDict(TypedDict):
    """Serialized representation of an `AgyRpcBoundaryHealth` dataclass."""

    auth_context: str
    connected: bool
    endpoint: str | None
    csrf_token_redacted: bool
    independent_of_codex_cli_oauth: bool
    circuit_breaker_open: bool


class CodexCliBoundaryHealthDict(TypedDict):
    """Serialized representation of a `CodexCliBoundaryHealth` dataclass."""

    auth_context: str
    cli_binary_ready: bool
    independent_of_antigravity_rpc: bool
    auto_reauth_enabled: bool


class AuthBoundariesHealthDict(TypedDict):
    """Serialized representation of an `AuthBoundariesHealth` dataclass."""

    antigravity_rpc: AgyRpcBoundaryHealthDict
    codex_cli_oauth: CodexCliBoundaryHealthDict


class RoutingContractHealthDict(TypedDict):
    """Serialized representation of a `RoutingContractHealth` dataclass."""

    verified_at_utc: str
    silent_fallback_enabled: bool
    claude_tier_policy: str
    advertised_antigravity_models: list[ModelSpecDict]
    routable_antigravity_models: list[str]
    antigravity_routable: bool
    advertised_codex_models: list[str]
    routable_codex_models: list[str]
    codex_cli_routable: bool


class BridgeHealthResponseDict(TypedDict):
    """Serialized representation of a `BridgeHealthResponse` dataclass."""

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


class DecisionCallerContentDict(TypedDict):
    """Serialized representation of a `DecisionCallerContent` dataclass."""

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


class DecisionAuthoritativeEvidenceDict(TypedDict):
    """Serialized representation of a `DecisionAuthoritativeEvidence` dataclass."""

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


class DecisionDigestPayloadDict(TypedDict):
    """Serialized representation of a `DecisionDigestPayload` dataclass."""

    caller_content: DecisionCallerContentDict
    authoritative_evidence: DecisionAuthoritativeEvidenceDict


class DecisionPackResponseDict(TypedDict):
    """Serialized representation of a `DecisionPackResponse` dataclass."""

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


__all__ = [
    "AgyRpcBoundaryHealthDict",
    "AuthBoundariesHealthDict",
    "BridgeHealthResponseDict",
    "CodexCliBoundaryHealthDict",
    "DecisionAuthoritativeEvidenceDict",
    "DecisionCallerContentDict",
    "DecisionDigestPayloadDict",
    "DecisionPackResponseDict",
    "HandoffCreateResponseDict",
    "HandoffInboxResponseDict",
    "HandoffResolveResponseDict",
    "JobListResponseDict",
    "JobStatusResponseDict",
    "RoutingContractHealthDict",
    "ToolErrorResponseDict",
]
