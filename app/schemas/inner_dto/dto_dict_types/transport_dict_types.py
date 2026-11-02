"""TypedDict return-type contracts for `transport_dtos.py` `.to_dict(self)` methods."""

from __future__ import annotations

from typing import TypedDict

from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import (
    ExternalMcpOAuthWarningDict,
)


class AgyAuthBoundaryDict(TypedDict):
    """Serialized representation of an `AgyAuthBoundary` dataclass."""

    auth_context: str
    antigravity_rpc_connected: bool
    codex_cli_oauth_affected: bool
    auto_credential_refresh_performed: bool


class AgyModelConfigEntryDict(TypedDict):
    """Serialized representation of an `AgyModelConfigEntry` dataclass."""

    label: str
    model_id: str
    placeholder: str
    remaining_quota: float | None
    provider: str
    engine: str
    routable: bool


class CodexCliAuthBoundaryDict(TypedDict):
    """Serialized representation of a `CodexCliAuthBoundary` dataclass."""

    auth_context: str
    antigravity_rpc_affected: bool
    auto_credential_refresh_performed: bool
    isolated_external_mcp_oauth_warnings: list[ExternalMcpOAuthWarningDict] | JsonValue


class AgyRouteExtraFieldsDict(TypedDict):
    """Serialized representation of an `AgyRouteExtraFields` dataclass."""

    model_requested: str
    auth_boundary: JsonObject


class AgyRpcPromptPayloadDict(TypedDict):
    """Serialized representation of an `AgyRpcPromptPayload` dataclass."""

    model: str
    prompt: str


class AgyTimeoutPartialResultDict(TypedDict):
    """Serialized representation of an `AgyTimeoutPartialResult` dataclass."""

    request_id: str
    model_id: str
    placeholder: str
    status: str
    outcome_state: str


class AgyDeduplicatedResponseDict(TypedDict):
    """Serialized representation of an `AgyDeduplicatedResponse` dataclass."""

    ok: bool
    deduplicated_existing_request: bool
    request_id: str
    job_id: str
    status: str
    outcome_state: str
    attempt_count: int
    provider: str
    engine: str
    model_id: str
    result: JsonValue
    error: str | None
    receipt: JsonObject | None


class AgyEarlyFailResultDict(TypedDict):
    """Serialized representation of an `AgyEarlyFailResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    model_requested: str
    model_id: str
    error: str
    receipt: JsonObject
    budget: JsonObject
    auth_boundary: JsonObject


class AgyTransportErrorResultDict(TypedDict):
    """Serialized representation of an `AgyTransportErrorResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    model_id: str
    placeholder: str
    duration_seconds: float
    error: str
    receipt: JsonObject
    budget: JsonObject
    auth_boundary: JsonObject


class AgyPromptSuccessResultDict(TypedDict):
    """Serialized representation of an `AgyPromptSuccessResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    model_id: str
    requested_model: str
    placeholder: str
    duration_seconds: float
    structured_output: JsonObject | None
    raw_output: str
    receipt: JsonObject
    budget: JsonObject
    auth_boundary: JsonObject


__all__ = [
    "AgyAuthBoundaryDict",
    "AgyDeduplicatedResponseDict",
    "AgyEarlyFailResultDict",
    "AgyModelConfigEntryDict",
    "AgyPromptSuccessResultDict",
    "AgyRouteExtraFieldsDict",
    "AgyRpcPromptPayloadDict",
    "AgyTimeoutPartialResultDict",
    "AgyTransportErrorResultDict",
    "CodexCliAuthBoundaryDict",
]
