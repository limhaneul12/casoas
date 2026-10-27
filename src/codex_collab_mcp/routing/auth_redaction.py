"""CPython 3.14t runtime inspection, secret redaction, and stderr OAuth boundary classification."""

from __future__ import annotations

import re
import sys
import sysconfig
from dataclasses import dataclass

from codex_collab_mcp.routing.contract_dicts import (
    ExternalMcpOAuthWarningDict,
    PythonRuntimeInfoDict,
    StderrAuthClassificationDict,
)

_SECRET_PATTERNS: tuple[tuple[re.Pattern[str], str], ...] = (
    (re.compile(r"(--csrf_token(?:=|\s+))([^\s\"']+)", re.IGNORECASE), r"\1***REDACTED***"),
    (re.compile(r"(--host_bridge_token(?:=|\s+))([^\s\"']+)", re.IGNORECASE), r"\1***REDACTED***"),
    (re.compile(r"(Bearer\s+)([A-Za-z0-9._\-]+)", re.IGNORECASE), r"\1***REDACTED***"),
    (
        re.compile(
            r"((?:api_key|access_token|refresh_token|client_secret|csrf_token|password)"
            r"[\"'=:\s]+)([^\s\"',}]+)",
            re.IGNORECASE,
        ),
        r"\1***REDACTED***",
    ),
    (re.compile(r"\bsk-[A-Za-z0-9_\-]{12,}\b"), "sk-***REDACTED***"),
)
_OAUTH_SERVER_RE: re.Pattern[str] = re.compile(r"for server ([A-Za-z0-9_\-]+):")


@dataclass(frozen=True, slots=True)
class PythonRuntimeInfo:
    """CPython runtime version, ABI flags, and free-threading status."""

    version: str
    version_full: str
    abiflags: str
    free_threading_build: bool
    gil_enabled: bool

    def to_dict(self) -> PythonRuntimeInfoDict:
        """Serialize the runtime info record to a TypedDict.

        Returns:
            Serialized PythonRuntimeInfoDict mapping.
        """
        return PythonRuntimeInfoDict(
            version=self.version,
            version_full=self.version_full,
            abiflags=self.abiflags,
            free_threading_build=self.free_threading_build,
            gil_enabled=self.gil_enabled,
        )


@dataclass(frozen=True, slots=True)
class StderrAuthClassification:
    """Sanitized stderr and isolated external OAuth warning classification."""

    sanitized_stderr: str
    isolated_external_mcp_oauth_warnings: list[ExternalMcpOAuthWarningDict]
    antigravity_rpc_affected: bool
    auto_credential_refresh_performed: bool

    def to_dict(self) -> StderrAuthClassificationDict:
        """Serialize the stderr auth classification to a TypedDict.

        Returns:
            Serialized StderrAuthClassificationDict mapping.
        """
        return StderrAuthClassificationDict(
            sanitized_stderr=self.sanitized_stderr,
            isolated_external_mcp_oauth_warnings=self.isolated_external_mcp_oauth_warnings,
            antigravity_rpc_affected=self.antigravity_rpc_affected,
            auto_credential_refresh_performed=self.auto_credential_refresh_performed,
        )


def get_python_runtime_info() -> PythonRuntimeInfoDict:
    """Return CPython runtime version, ABI flags, and free-threading (No-GIL) status.

    Returns:
        Typed PythonRuntimeInfoDict describing interpreter version, ABI flags, and GIL state.
    """
    vi, abiflags = sys.version_info, sys.abiflags
    return PythonRuntimeInfo(
        version=f"{vi.major}.{vi.minor}.{vi.micro}{abiflags}",
        version_full=sys.version.splitlines()[0],
        abiflags=abiflags,
        free_threading_build="t" in abiflags or sysconfig.get_config_var("Py_GIL_DISABLED") == 1,
        gil_enabled=sys._is_gil_enabled(),
    ).to_dict()


def redact_sensitive_text(text: str) -> str:
    """Strip tokens, API keys, CSRF tokens, and bearer secrets from logs and error strings.

    Args:
        text: Raw text potentially containing secrets.

    Returns:
        Sanitized text with secrets replaced by redaction markers.
    """
    cleaned = text
    for pattern, replacement in _SECRET_PATTERNS:
        cleaned = pattern.sub(replacement, cleaned)
    return cleaned


def classify_stderr_and_auth_boundaries(stderr_text: str) -> StderrAuthClassificationDict:
    """Separate downstream MCP OAuth `invalid_grant` noise from primary engine/auth status.

    Args:
        stderr_text: Raw stderr output captured from subprocess execution.

    Returns:
        Typed StderrAuthClassificationDict with sanitized stderr and isolated OAuth warnings.
    """
    redacted = redact_sensitive_text(stderr_text)
    warnings: list[ExternalMcpOAuthWarningDict] = []
    other_lines: list[str] = []
    for line in redacted.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if "codex_rmcp_client::oauth" in stripped or "invalid_grant" in stripped:
            m = _OAUTH_SERVER_RE.search(stripped)
            warnings.append(
                ExternalMcpOAuthWarningDict(
                    external_mcp_server=m.group(1) if m else "unknown_external_mcp",
                    error_type="external_mcp_oauth_invalid_grant",
                    message=stripped,
                    affects_antigravity_rpc=False,
                    affects_codex_model_routing=False,
                    auto_reauth_triggered=False,
                )
            )
        else:
            other_lines.append(stripped)
    return StderrAuthClassification(
        sanitized_stderr="\n".join(other_lines),
        isolated_external_mcp_oauth_warnings=warnings,
        antigravity_rpc_affected=False,
        auto_credential_refresh_performed=False,
    ).to_dict()
