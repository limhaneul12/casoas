"""Explicit TypedDict contracts for internal dataclass serialization and routing/budget envelopes."""

from __future__ import annotations

from typing import TypedDict

from codex_collab_mcp.schemas import JsonObject, JsonValue


class ModelSpecDict(TypedDict):
    """Serialized representation of a ModelSpec dataclass."""

    provider: str
    engine: str
    model_id: str
    auth_context: str
    reasoning_tier: str
    placeholder: str | None
    capabilities: list[str]


class RoutingValidationDict(TypedDict):
    """Serialized representation of a RoutingValidationResult dataclass."""

    valid: bool
    provider: str
    engine: str
    requested_model: str
    resolved_model: str
    auth_context: str
    reasoning_tier: str
    placeholder: str | None
    error_code: str | None
    error_message: str | None


class ExecutionReceiptDict(TypedDict):
    """Serialized representation of an ExecutionReceipt dataclass."""

    request_id: str
    idempotency_key: str | None
    provider: str
    engine: str
    requested_model: str
    resolved_model: str
    actual_model: str
    model_mismatch: bool
    call_path: str
    routing_decision: JsonObject
    started_at: str
    ended_at: str
    duration_seconds: float
    status: str
    outcome_state: str
    attempt_count: int
    output_digest: str | None
    result_ref: str | None
    failure_reason: str | None
    credentials_redacted: bool


class BudgetEnvelopeDict(TypedDict):
    """Serialized representation of a BudgetEnvelope dataclass."""

    max_input_bytes: int
    actual_input_bytes: int
    max_input_tokens: int
    estimated_input_tokens: int
    timeout_seconds: float
    elapsed_seconds: float
    input_budget_exceeded: bool
    time_budget_exceeded: bool
    termination_reason: str


class CodexJobRecordDict(TypedDict):
    """Serialized representation of a CodexJobRecord dataclass."""

    job_id: str
    request_id: str
    task_type: str
    status: str
    outcome_state: str
    idempotency_key: str | None
    provider: str | None
    engine: str | None
    model: str | None
    attempt_count: int
    cancel_requested: bool
    cancelled_at: float | None
    created_at: float
    workspace_dir: str
    session_id: str | None
    completed_at: float | None
    result: JsonValue
    error: str | None
    receipt: JsonObject | None
    metadata: JsonObject
    elapsed_seconds: float


class HandoffTicketDict(TypedDict):
    """Serialized representation of a HandoffTicket dataclass."""

    ticket_id: str
    sender: str
    recipient: str
    title: str
    spec_or_payload: str
    workspace_dir: str
    priority: str
    status: str
    collaboration_role_state: str
    execution_completed: bool
    consensus_eligible: bool
    created_at: float
    updated_at: float
    resolution_note: str | None
    alexandria_ref: str | None


class PythonRuntimeInfoDict(TypedDict):
    """Serialized representation of a PythonRuntimeInfo dataclass."""

    version: str
    version_full: str
    abiflags: str
    free_threading_build: bool
    gil_enabled: bool


class StderrAuthClassificationDict(TypedDict):
    """Serialized representation of a StderrAuthClassification dataclass."""

    sanitized_stderr: str
    isolated_external_mcp_oauth_warnings: list[JsonValue]
    antigravity_rpc_affected: bool
    auto_credential_refresh_performed: bool
