"""Model specifications, aliases, secret redaction patterns, and typed receipt/budget records."""

from __future__ import annotations

import hashlib
import re
import sys
import sysconfig
from dataclasses import dataclass

from codex_collab_mcp.collab_enums import (
    AuthContextKind,
    CanonicalModelId,
    EngineKind,
    JobStatusKind,
    ProviderKind,
    ReasoningTier,
)
from codex_collab_mcp.domain.contract_dicts import (
    BudgetEnvelopeDict,
    ExecutionReceiptDict,
    ModelSpecDict,
    PythonRuntimeInfoDict,
    RoutingValidationDict,
    StderrAuthClassificationDict,
)
from codex_collab_mcp.schemas import JsonObject, JsonValue


@dataclass(frozen=True, slots=True)
class ModelSpec:
    """Authoritative capability and routing specification for a single model."""

    provider: str
    engine: str
    model_id: str
    auth_context: str
    reasoning_tier: str
    placeholder: str | None
    capabilities: tuple[str, ...]

    def to_dict(self) -> ModelSpecDict:
        """Serialize the model specification to a TypedDict.

        Returns:
            Serialized ModelSpecDict mapping.
        """
        return ModelSpecDict(
            provider=self.provider,
            engine=self.engine,
            model_id=self.model_id,
            auth_context=self.auth_context,
            reasoning_tier=self.reasoning_tier,
            placeholder=self.placeholder,
            capabilities=list(self.capabilities),
        )


_CRAFT_COUNCIL = ("code_craft", "advisory_review", "triad_council")
_CRAFT_ONLY = ("code_craft", "advisory_review")
_MACRO_COUNCIL = ("macro_architecture", "advisory_review", "triad_council")
_MACRO_ONLY = ("macro_architecture", "advisory_review")
_FAST_CAPS = ("fast_review",)
_CODEX_CAPS = ("adversarial_audit", "targeted_synthesis", "rescue", "triad_council")
_M = CanonicalModelId
_T = ReasoningTier

ANTIGRAVITY_MODEL_SPECS: dict[str, ModelSpec] = {
    str(m): ModelSpec(
        ProviderKind.ANTIGRAVITY,
        EngineKind.ANTIGRAVITY_LOCAL_RPC,
        str(m),
        AuthContextKind.ANTIGRAVITY_LOCAL_CSRF,
        str(t),
        p,
        c,
    )
    for m, t, p, c in (
        (_M.CLAUDE_SONNET_5_5_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M404", _CRAFT_COUNCIL),
        (_M.CLAUDE_SONNET_5_5_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M405", _CRAFT_COUNCIL),
        (_M.CLAUDE_SONNET_5_5_LOW, _T.LOW, "MODEL_PLACEHOLDER_M403", _CRAFT_ONLY),
        (_M.CLAUDE_OPUS_5_5_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M401", _CRAFT_COUNCIL),
        (_M.CLAUDE_OPUS_5_5_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M402", _CRAFT_COUNCIL),
        (_M.CLAUDE_OPUS_5_5_LOW, _T.LOW, "MODEL_PLACEHOLDER_M400", _CRAFT_ONLY),
        (_M.GEMINI_4_ARGON_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M417", _MACRO_COUNCIL),
        (_M.GEMINI_4_ARGON_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M418", _MACRO_COUNCIL),
        (_M.GEMINI_4_ARGON_LOW, _T.LOW, "MODEL_PLACEHOLDER_M419", _MACRO_ONLY),
        (_M.GEMINI_3_8_FLASH_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M318", _FAST_CAPS),
        (_M.GEMINI_3_8_FLASH_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M319", _FAST_CAPS),
        (_M.GEMINI_3_8_FLASH_LOW, _T.LOW, "MODEL_PLACEHOLDER_M320", _FAST_CAPS),
    )
}

CODEX_MODEL_SPECS: dict[str, ModelSpec] = {
    str(m): ModelSpec(
        ProviderKind.OPENAI_CODEX,
        EngineKind.CODEX_CLI,
        str(m),
        AuthContextKind.CODEX_CHATGPT_OAUTH,
        str(t),
        None,
        _CODEX_CAPS,
    )
    for m, t in (
        (_M.GPT_6_1_SOL, _T.XHIGH),
        (_M.GPT_5_4, _T.HIGH),
        (_M.GPT_5_CODEX, _T.HIGH),
        (_M.GPT_5_1, _T.HIGH),
        (_M.GPT_5, _T.HIGH),
        (_M.GPT_5_NANO, _T.LOW),
        (_M.O3, _T.HIGH),
        (_M.O4_MINI, _T.MEDIUM),
        (_M.CODEX_DEFAULT, _T.HIGH),
    )
}

_SONNET_ALIASES = (
    "sonnet",
    "sonnet-medium",
    "sonnet-5.5",
    "sonnet-5-5",
    "sonnet5.5",
    "claude",
    "claude-sonnet",
    "claude-sonnet-5.5",
    "claude-sonnet-5.5-medium",
)
_OPUS_ALIASES = (
    "opus",
    "opus-medium",
    "opus-5.5",
    "opus-5-5",
    "opus5.5",
    "claude-opus",
    "claude-opus-5.5",
    "claude-opus-5.5-medium",
)
EXPLICIT_MODEL_ALIASES: dict[str, str] = dict(
    (
        *((k, str(_M.CLAUDE_SONNET_5_5_MEDIUM)) for k in _SONNET_ALIASES),
        ("sonnet-high", str(_M.CLAUDE_SONNET_5_5_HIGH)),
        ("sonnet-low", str(_M.CLAUDE_SONNET_5_5_LOW)),
        *((k, str(_M.CLAUDE_OPUS_5_5_MEDIUM)) for k in _OPUS_ALIASES),
        ("opus-high", str(_M.CLAUDE_OPUS_5_5_HIGH)),
        ("opus-low", str(_M.CLAUDE_OPUS_5_5_LOW)),
        ("argon", str(_M.GEMINI_4_ARGON_HIGH)),
        ("gemini-4-argon", str(_M.GEMINI_4_ARGON_HIGH)),
        ("argon-high", str(_M.GEMINI_4_ARGON_HIGH)),
        ("argon-medium", str(_M.GEMINI_4_ARGON_MEDIUM)),
        ("argon-low", str(_M.GEMINI_4_ARGON_LOW)),
    )
)

CODEX_SUPPORTED_MODELS: set[str] = set(CODEX_MODEL_SPECS.keys())
VALID_REASONING_TIERS: set[str] = {tier.value for tier in ReasoningTier}

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
    routing_decision: JsonObject
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
    ) -> JsonObject:
        """Compute input byte/token usage and timeout budget status for a role execution.

        Args:
            prompt_text: Full prompt text to measure.
            max_input_bytes: Maximum allowed UTF-8 input byte count.
            max_input_tokens: Maximum allowed estimated input token count.
            timeout_seconds: Configured timeout budget in seconds.
            elapsed_seconds: Actual elapsed execution time in seconds.
            termination_reason: Reason for execution completion or termination.

        Returns:
            JSON object summarizing input and timeout budget metrics.
        """
        actual_bytes = len(prompt_text.encode("utf-8"))
        est_tokens = max(1, actual_bytes // 4) if actual_bytes > 0 else 0
        in_exceeded = actual_bytes > max_input_bytes or est_tokens > max_input_tokens
        time_exceeded = elapsed_seconds >= timeout_seconds and termination_reason in (
            "timeout_budget_reached",
            JobStatusKind.TIMED_OUT_UNKNOWN,
        )
        reason = str(JobStatusKind.INPUT_BUDGET_EXCEEDED) if in_exceeded else termination_reason
        env = cls(
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
        return {
            "max_input_bytes": env["max_input_bytes"],
            "actual_input_bytes": env["actual_input_bytes"],
            "max_input_tokens": env["max_input_tokens"],
            "estimated_input_tokens": env["estimated_input_tokens"],
            "timeout_seconds": env["timeout_seconds"],
            "elapsed_seconds": env["elapsed_seconds"],
            "input_budget_exceeded": env["input_budget_exceeded"],
            "time_budget_exceeded": env["time_budget_exceeded"],
            "termination_reason": env["termination_reason"],
        }


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
    isolated_external_mcp_oauth_warnings: list[JsonValue]
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


def compute_sha256_hex(content: str) -> str:
    """Return a deterministic SHA-256 hex digest of a UTF-8 string.

    Args:
        content: Input UTF-8 string to hash.

    Returns:
        Lowercase 64-character SHA-256 hexadecimal digest.
    """
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def compute_output_digest(output_text: str | None) -> str | None:
    """Compute a `sha256:...` digest for non-empty model output text.

    Args:
        output_text: Raw output string or None.

    Returns:
        Prefixed SHA-256 digest string when output_text is non-empty, else None.
    """
    return f"sha256:{compute_sha256_hex(output_text)}" if output_text else None


def get_python_runtime_info() -> JsonObject:
    """Return CPython runtime version, ABI flags, and free-threading (No-GIL) status.

    Returns:
        JSON object describing interpreter version, ABI flags, and GIL state.
    """
    vi, abiflags = sys.version_info, sys.abiflags
    info = PythonRuntimeInfo(
        version=f"{vi.major}.{vi.minor}.{vi.micro}{abiflags}",
        version_full=sys.version.splitlines()[0],
        abiflags=abiflags,
        free_threading_build="t" in abiflags or sysconfig.get_config_var("Py_GIL_DISABLED") == 1,
        gil_enabled=sys._is_gil_enabled(),
    ).to_dict()
    return {
        "version": info["version"],
        "version_full": info["version_full"],
        "abiflags": info["abiflags"],
        "free_threading_build": info["free_threading_build"],
        "gil_enabled": info["gil_enabled"],
    }


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


def classify_stderr_and_auth_boundaries(stderr_text: str) -> JsonObject:
    """Separate downstream MCP OAuth `invalid_grant` noise from primary engine/auth status.

    Args:
        stderr_text: Raw stderr output captured from subprocess execution.

    Returns:
        JSON object with sanitized stderr and isolated external OAuth warnings.
    """
    redacted = redact_sensitive_text(stderr_text)
    warnings: list[JsonValue] = []
    other_lines: list[str] = []
    for line in redacted.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if "codex_rmcp_client::oauth" in stripped or "invalid_grant" in stripped:
            m = _OAUTH_SERVER_RE.search(stripped)
            warn_entry: JsonObject = {
                "external_mcp_server": m.group(1) if m else "unknown_external_mcp",
                "error_type": "external_mcp_oauth_invalid_grant",
                "message": stripped,
                "affects_antigravity_rpc": False,
                "affects_codex_model_routing": False,
                "auto_reauth_triggered": False,
            }
            warnings.append(warn_entry)
        else:
            other_lines.append(stripped)
    cls_dict = StderrAuthClassification(
        sanitized_stderr="\n".join(other_lines),
        isolated_external_mcp_oauth_warnings=warnings,
        antigravity_rpc_affected=False,
        auto_credential_refresh_performed=False,
    ).to_dict()
    return {
        "sanitized_stderr": cls_dict["sanitized_stderr"],
        "isolated_external_mcp_oauth_warnings": cls_dict["isolated_external_mcp_oauth_warnings"],
        "antigravity_rpc_affected": cls_dict["antigravity_rpc_affected"],
        "auto_credential_refresh_performed": cls_dict["auto_credential_refresh_performed"],
    }


evaluate_budget_envelope = BudgetEnvelope.evaluate
