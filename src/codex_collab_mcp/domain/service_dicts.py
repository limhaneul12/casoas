"""Explicit TypedDict and dataclass contracts for MCP gateway, tool handlers, and decisions."""

from __future__ import annotations

from collections.abc import Callable, Coroutine
from dataclasses import dataclass
from typing import TypedDict

from codex_collab_mcp.collab_enums import CanonicalModelId
from codex_collab_mcp.domain.contract_dicts import (
    CodexJobRecordDict,
    HandoffTicketDict,
    ModelSpecDict,
)
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.schemas import JsonObject, JsonValue

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]


@dataclass(frozen=True, slots=True)
class EngineDispatchRequest:
    """Validated request parameters for synchronous engine execution."""

    prompt: str
    schema_name: str
    output_schema: JsonObject | None
    workspace_dir: str | None = None
    sandbox_mode: str = "read-only"
    model: str | None = None
    reasoning_effort: str | None = None
    engine: str = "cli"
    timeout_seconds: float = 300.0
    provider: str | None = None
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000


@dataclass(frozen=True, slots=True)
class BackgroundJobDispatchRequest:
    """Validated request parameters for asynchronous background job dispatch."""

    task_type: str
    prompt: str
    schema_name: str
    output_schema: JsonObject | None
    workspace_dir: str
    sandbox_mode: str
    model: str | None
    reasoning_effort: str | None
    engine: str
    timeout_seconds: float
    provider: str | None = None
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000


@dataclass(frozen=True, slots=True)
class CodexSchemaToolRequest:
    """Validated request bundle for structured Codex schema tool dispatch."""

    task_type: str
    prompt: str
    provided_context_or_code: str
    workspace_dir: str = "."
    sandbox_mode: str = "read-only"
    model: str | None = None
    reasoning_effort: str | None = None
    async_background: bool = False
    engine: str = "cli"
    timeout_seconds: float = 300.0
    provider: str | None = None
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000
    files_inspected: list[str] | None = None
    symbols_inspected: list[str] | None = None
    executed_test_commands: list[JsonObject] | None = None


@dataclass(frozen=True, slots=True)
class ClaudeCraftRequest:
    """Validated request bundle for Claude Sonnet/Opus surgical code craft."""

    task_spec: str
    target_code_or_context: str = ""
    invariants_and_rules: str = ""
    language: str = "python"
    claude_model: str = CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM
    timeout_seconds: float = 180.0
    workspace_dir: str = "."
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000
    files_inspected: list[str] | None = None
    symbols_inspected: list[str] | None = None
    executed_test_commands: list[JsonObject] | None = None


@dataclass(frozen=True, slots=True)
class DecisionPackRequest:
    """Validated request bundle for Alexandria decision pack synthesis."""

    project_scope: str
    topic_title: str
    argon_architecture_intent: str
    codex_audit_or_synthesis_summary: str
    final_resolution: str
    sonnet_craft_or_review_summary: str = ""
    verification_evidence: str = ""
    collaboration_id: str | None = None
    workspace_dir: str | None = None
    expected_scope_digest: str | None = None


@dataclass(frozen=True, slots=True)
class ServerGatewayBindings:
    """Runtime store and transport callables injected from the server module."""

    job_store: JobStore
    run_agy_fn: AsyncEngineCallable
    run_http_fn: AsyncEngineCallable
    run_codex_fn: AsyncEngineCallable


class ToolErrorResponseDict(TypedDict):
    """Standard error payload returned when a requested resource is not found."""

    ok: bool
    error: str


class JobStatusResponseDict(TypedDict):
    """Serialized response payload for codex_job_status."""

    ok: bool
    job: CodexJobRecordDict
    outcome_state: str
    cancel_requested: bool


class JobListResponseDict(TypedDict):
    """Serialized response payload for codex_job_list."""

    ok: bool
    workspace_filter: str | None
    jobs: list[CodexJobRecordDict]


class HandoffCreateResponseDict(TypedDict):
    """Serialized response payload for heterarchy_handoff_create."""

    ok: bool
    ticket: HandoffTicketDict
    collaboration_role_state: str
    execution_completed: bool
    consensus_eligible: bool
    message: str


class HandoffInboxResponseDict(TypedDict):
    """Serialized response payload for heterarchy_handoff_inbox."""

    ok: bool
    recipient_filter: str | None
    status_filter: str | None
    workspace_filter: str | None
    count: int
    tickets: list[HandoffTicketDict]


class HandoffResolveResponseDict(TypedDict):
    """Serialized response payload for heterarchy_handoff_resolve."""

    ok: bool
    ticket: HandoffTicketDict


class AgyRpcBoundaryHealthDict(TypedDict):
    """Antigravity local RPC authentication boundary status."""

    auth_context: str
    connected: bool
    endpoint: str | None
    csrf_token_redacted: bool
    independent_of_codex_cli_oauth: bool


class CodexCliBoundaryHealthDict(TypedDict):
    """Codex CLI OAuth authentication boundary status."""

    auth_context: str
    cli_binary_ready: bool
    independent_of_antigravity_rpc: bool
    auto_reauth_enabled: bool


class AuthBoundariesHealthDict(TypedDict):
    """Combined authentication boundary health status."""

    antigravity_rpc: AgyRpcBoundaryHealthDict
    codex_cli_oauth: CodexCliBoundaryHealthDict


class RoutingContractHealthDict(TypedDict):
    """Verified pre-dispatch routing contract status."""

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
    """Serialized response payload for codex_bridge_health."""

    ok: bool
    verified_at_utc: str
    python_runtime: JsonObject
    codex_binary: str
    codex_version: str
    agy_bridge_ready: bool
    agy_endpoint: str | None
    antigravity_models: list[JsonObject]
    routing_contract: RoutingContractHealthDict
    auth_boundaries: AuthBoundariesHealthDict
    active_background_jobs: int
    pending_argon_handoffs: int
    pending_sonnet_handoffs: int
    pending_opus_handoffs: int
    pending_codex_handoffs: int


class DecisionCallerContentDict(TypedDict):
    """Caller-supplied decision notes and scope fences for digest calculation."""

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
    """Authoritative triad evidence fields included in the decision pack digest."""

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
    """Combined payload hashed to produce the deterministic Alexandria idempotency key."""

    caller_content: DecisionCallerContentDict
    authoritative_evidence: DecisionAuthoritativeEvidenceDict


class DecisionPackResponseDict(TypedDict):
    """Serialized response payload for codex_alexandria_decision_pack."""

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
