"""Explicit TypedDict and frozen dataclass contracts for Antigravity RPC and Codex CLI boundaries."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TypedDict

from codex_collab_mcp.routing.contract_dicts import ExternalMcpOAuthWarningDict
from codex_collab_mcp.schemas import JsonValue


class AgyAuthBoundaryDict(TypedDict):
    """Authentication boundary metadata for local Antigravity LanguageServer RPC."""

    auth_context: str
    antigravity_rpc_connected: bool
    codex_cli_oauth_affected: bool
    auto_credential_refresh_performed: bool


@dataclass(frozen=True, slots=True)
class AgyAuthBoundary:
    """Immutable internal DTO for Antigravity local CSRF authentication boundary metadata."""

    antigravity_rpc_connected: bool
    auth_context: str = "antigravity_local_csrf"
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


class AgyModelConfigEntryDict(TypedDict):
    """Normalized model catalog entry returned by AntigravityModelCatalog."""

    label: str
    model_id: str
    placeholder: str
    remaining_quota: float | None
    provider: str
    engine: str
    routable: bool


@dataclass(frozen=True, slots=True)
class AgyModelConfigEntry:
    """Immutable internal DTO for a normalized Antigravity model catalog entry."""

    label: str
    model_id: str
    placeholder: str
    remaining_quota: float | None
    routable: bool
    provider: str = "antigravity"
    engine: str = "antigravity_local_rpc"

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


class CodexCliAuthBoundaryDict(TypedDict):
    """Authentication boundary metadata for Codex CLI subprocess executions."""

    auth_context: str
    antigravity_rpc_affected: bool
    auto_credential_refresh_performed: bool
    isolated_external_mcp_oauth_warnings: list[ExternalMcpOAuthWarningDict] | JsonValue


@dataclass(frozen=True, slots=True)
class CodexCliAuthBoundary:
    """Immutable internal DTO for Codex CLI authentication boundary metadata."""

    isolated_external_mcp_oauth_warnings: list[ExternalMcpOAuthWarningDict] | JsonValue
    auth_context: str = "codex_chatgpt_oauth"
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
