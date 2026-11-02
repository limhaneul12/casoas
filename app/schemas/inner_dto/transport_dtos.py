"""Internal frozen dataclass DTOs for Antigravity RPC and Codex CLI transport boundaries."""

from __future__ import annotations

from dataclasses import dataclass

from app.core.collab_enums import AuthContextKind, EngineKind, ProviderKind
from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types import (
    AgyAuthBoundaryDict,
    AgyDeduplicatedResponseDict,
    AgyEarlyFailResultDict,
    AgyModelConfigEntryDict,
    AgyPromptSuccessResultDict,
    AgyRouteExtraFieldsDict,
    AgyRpcPromptPayloadDict,
    AgyTimeoutPartialResultDict,
    AgyTransportErrorResultDict,
    CodexCliAuthBoundaryDict,
    ExternalMcpOAuthWarningDict,
)


@dataclass(frozen=True, slots=True)
class AgyAuthBoundary:
    """Immutable internal DTO for Antigravity local CSRF authentication boundary metadata."""

    antigravity_rpc_connected: bool
    auth_context: str = AuthContextKind.ANTIGRAVITY_LOCAL_CSRF
    codex_cli_oauth_affected: bool = False
    auto_credential_refresh_performed: bool = False

    def to_dict(self) -> AgyAuthBoundaryDict:
        """Serialize the authentication boundary into a TypedDict.

        Returns:
            Typed `AgyAuthBoundaryDict` representation.
        """
        return AgyAuthBoundaryDict(
            auth_context=self.auth_context,
            antigravity_rpc_connected=self.antigravity_rpc_connected,
            codex_cli_oauth_affected=self.codex_cli_oauth_affected,
            auto_credential_refresh_performed=self.auto_credential_refresh_performed,
        )


@dataclass(frozen=True, slots=True)
class AgyModelConfigEntry:
    """Immutable internal DTO for a normalized Antigravity model catalog entry."""

    label: str
    model_id: str
    placeholder: str
    remaining_quota: float | None
    routable: bool
    provider: str = ProviderKind.ANTIGRAVITY
    engine: str = EngineKind.ANTIGRAVITY_LOCAL_RPC

    def to_dict(self) -> AgyModelConfigEntryDict:
        """Serialize the model catalog entry into a TypedDict.

        Returns:
            Typed `AgyModelConfigEntryDict` representation.
        """
        return AgyModelConfigEntryDict(
            label=self.label,
            model_id=self.model_id,
            placeholder=self.placeholder,
            remaining_quota=self.remaining_quota,
            provider=self.provider,
            engine=self.engine,
            routable=self.routable,
        )


@dataclass(frozen=True, slots=True)
class CodexCliAuthBoundary:
    """Immutable internal DTO for Codex CLI authentication boundary metadata."""

    isolated_external_mcp_oauth_warnings: list[ExternalMcpOAuthWarningDict] | JsonValue
    auth_context: str = AuthContextKind.CODEX_CHATGPT_OAUTH
    antigravity_rpc_affected: bool = False
    auto_credential_refresh_performed: bool = False

    def to_dict(self) -> CodexCliAuthBoundaryDict:
        """Serialize the Codex CLI authentication boundary into a TypedDict.

        Returns:
            Typed `CodexCliAuthBoundaryDict` representation.
        """
        return CodexCliAuthBoundaryDict(
            auth_context=self.auth_context,
            antigravity_rpc_affected=self.antigravity_rpc_affected,
            auto_credential_refresh_performed=self.auto_credential_refresh_performed,
            isolated_external_mcp_oauth_warnings=self.isolated_external_mcp_oauth_warnings,
        )


@dataclass(frozen=True, slots=True)
class AgyRouteExtraFields:
    """Internal DTO for extra pre-dispatch rejection fields in Antigravity RPC."""

    model_requested: str
    auth_boundary: JsonObject

    def to_dict(self) -> AgyRouteExtraFieldsDict:
        """Serialize extra route rejection fields into a TypedDict.

        Returns:
            Typed `AgyRouteExtraFieldsDict` representation.
        """
        return AgyRouteExtraFieldsDict(
            model_requested=self.model_requested,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class AgyRpcPromptPayload:
    """Internal DTO for the Connect-RPC request body of `GetModelResponse`."""

    model: str
    prompt: str

    def to_dict(self) -> AgyRpcPromptPayloadDict:
        """Serialize the Connect-RPC prompt payload into a TypedDict.

        Returns:
            Typed `AgyRpcPromptPayloadDict` representation.
        """
        return AgyRpcPromptPayloadDict(model=self.model, prompt=self.prompt)


@dataclass(frozen=True, slots=True)
class AgyTimeoutPartialResult:
    """Internal DTO for an Antigravity RPC timeout partial result."""

    request_id: str
    model_id: str
    placeholder: str
    status: str
    outcome_state: str

    def to_dict(self) -> AgyTimeoutPartialResultDict:
        """Serialize the timeout partial result to a TypedDict.

        Returns:
            Typed `AgyTimeoutPartialResultDict` mapping.
        """
        return AgyTimeoutPartialResultDict(
            request_id=self.request_id,
            model_id=self.model_id,
            placeholder=self.placeholder,
            status=self.status,
            outcome_state=self.outcome_state,
        )


@dataclass(frozen=True, slots=True)
class AgyDeduplicatedResponse:
    """Internal DTO for a deduplicated Antigravity RPC request outcome."""

    ok: bool
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
    deduplicated_existing_request: bool = True

    def to_dict(self) -> AgyDeduplicatedResponseDict:
        """Serialize the deduplicated Antigravity RPC response to a TypedDict.

        Returns:
            Typed `AgyDeduplicatedResponseDict` mapping.
        """
        return AgyDeduplicatedResponseDict(
            ok=self.ok,
            deduplicated_existing_request=self.deduplicated_existing_request,
            request_id=self.request_id,
            job_id=self.job_id,
            status=self.status,
            outcome_state=self.outcome_state,
            attempt_count=self.attempt_count,
            provider=self.provider,
            engine=self.engine,
            model_id=self.model_id,
            result=self.result,
            error=self.error,
            receipt=self.receipt,
        )


@dataclass(frozen=True, slots=True)
class AgyEarlyFailResult:
    """Internal DTO for an Antigravity early failure outcome."""

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

    def to_dict(self) -> AgyEarlyFailResultDict:
        """Serialize the early failure result to a TypedDict.

        Returns:
            Typed `AgyEarlyFailResultDict` mapping.
        """
        return AgyEarlyFailResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            model_requested=self.model_requested,
            model_id=self.model_id,
            error=self.error,
            receipt=self.receipt,
            budget=self.budget,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class AgyTransportErrorResult:
    """Internal DTO for an Antigravity transport error or timeout outcome."""

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

    def to_dict(self) -> AgyTransportErrorResultDict:
        """Serialize the transport error result to a TypedDict.

        Returns:
            Typed `AgyTransportErrorResultDict` mapping.
        """
        return AgyTransportErrorResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            model_id=self.model_id,
            placeholder=self.placeholder,
            duration_seconds=self.duration_seconds,
            error=self.error,
            receipt=self.receipt,
            budget=self.budget,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class AgyPromptSuccessResult:
    """Internal DTO for a completed Antigravity RPC prompt response."""

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

    def to_dict(self) -> AgyPromptSuccessResultDict:
        """Serialize the completed Antigravity RPC response to a TypedDict.

        Returns:
            Typed `AgyPromptSuccessResultDict` mapping.
        """
        return AgyPromptSuccessResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            model_id=self.model_id,
            requested_model=self.requested_model,
            placeholder=self.placeholder,
            duration_seconds=self.duration_seconds,
            structured_output=self.structured_output,
            raw_output=self.raw_output,
            receipt=self.receipt,
            budget=self.budget,
            auth_boundary=self.auth_boundary,
        )


__all__ = [
    "AgyAuthBoundary",
    "AgyDeduplicatedResponse",
    "AgyEarlyFailResult",
    "AgyModelConfigEntry",
    "AgyPromptSuccessResult",
    "AgyRouteExtraFields",
    "AgyRpcPromptPayload",
    "AgyTimeoutPartialResult",
    "AgyTransportErrorResult",
    "CodexCliAuthBoundary",
]
