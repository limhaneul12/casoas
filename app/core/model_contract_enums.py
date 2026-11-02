"""Canonical StrEnum definitions for model capabilities, aliases, placeholders, and error codes."""

from __future__ import annotations

from enum import StrEnum


class ModelCapabilityKind(StrEnum):
    """Capabilities supported by models in the Tri-Polar Heterarchy."""

    CODE_CRAFT = "code_craft"
    ADVISORY_REVIEW = "advisory_review"
    TRIAD_COUNCIL = "triad_council"
    MACRO_ARCHITECTURE = "macro_architecture"
    FAST_REVIEW = "fast_review"
    ADVERSARIAL_AUDIT = "adversarial_audit"
    TARGETED_SYNTHESIS = "targeted_synthesis"
    RESCUE = "rescue"


class AntigravityModelPlaceholder(StrEnum):
    """Internal LanguageServer model placeholder tokens for Antigravity RPC."""

    M404 = "MODEL_PLACEHOLDER_M404"
    M405 = "MODEL_PLACEHOLDER_M405"
    M403 = "MODEL_PLACEHOLDER_M403"
    M401 = "MODEL_PLACEHOLDER_M401"
    M402 = "MODEL_PLACEHOLDER_M402"
    M400 = "MODEL_PLACEHOLDER_M400"
    M417 = "MODEL_PLACEHOLDER_M417"
    M418 = "MODEL_PLACEHOLDER_M418"
    M419 = "MODEL_PLACEHOLDER_M419"
    M318 = "MODEL_PLACEHOLDER_M318"
    M319 = "MODEL_PLACEHOLDER_M319"
    M320 = "MODEL_PLACEHOLDER_M320"


class SonnetModelAlias(StrEnum):
    """Supported shorthand aliases that resolve to Claude Sonnet 5.5 Medium."""

    SONNET = "sonnet"
    SONNET_MEDIUM = "sonnet-medium"
    SONNET_5_5_DOT = "sonnet-5.5"
    SONNET_5_5_DASH = "sonnet-5-5"
    SONNET_5_5_COMPACT = "sonnet5.5"
    CLAUDE = "claude"
    CLAUDE_SONNET = "claude-sonnet"
    CLAUDE_SONNET_5_5_DOT = "claude-sonnet-5.5"
    CLAUDE_SONNET_5_5_MEDIUM_DOT = "claude-sonnet-5.5-medium"


class OpusModelAlias(StrEnum):
    """Supported shorthand aliases that resolve to Claude Opus 5.5 Medium."""

    OPUS = "opus"
    OPUS_MEDIUM = "opus-medium"
    OPUS_5_5_DOT = "opus-5.5"
    OPUS_5_5_DASH = "opus-5-5"
    OPUS_5_5_COMPACT = "opus5.5"
    CLAUDE_OPUS = "claude-opus"
    CLAUDE_OPUS_5_5_DOT = "claude-opus-5.5"
    CLAUDE_OPUS_5_5_MEDIUM_DOT = "claude-opus-5.5-medium"


class TieredModelAlias(StrEnum):
    """Explicit reasoning-tiered aliases for Sonnet, Opus, and Gemini 4 Argon."""

    SONNET_HIGH = "sonnet-high"
    SONNET_LOW = "sonnet-low"
    OPUS_HIGH = "opus-high"
    OPUS_LOW = "opus-low"
    ARGON = "argon"
    GEMINI_4_ARGON = "gemini-4-argon"
    ARGON_HIGH = "argon-high"
    ARGON_MEDIUM = "argon-medium"
    ARGON_LOW = "argon-low"


class ArgonModelAlias(StrEnum):
    """Supported shorthand aliases that resolve to Gemini 4 Argon High."""

    ARGON = "argon"
    ARGON4 = "argon4"
    ARGON_4 = "argon-4"
    GEMINI_4_ARGON = "gemini-4-argon"
    GEMINI_4_ARGON_HIGH = "gemini-4-argon-high"
    SHORT_4_ARGON = "4-argon"


class FlashModelAlias(StrEnum):
    """Supported shorthand aliases that resolve to Gemini 3.8 Flash High."""

    FLASH = "flash"
    GEMINI_3_8_FLASH = "gemini-3-8-flash"
    GEMINI_3_8_FLASH_HIGH_DASH = "gemini-3-8-flash-high"
    GEMINI_3_8_FLASH_HIGH = "gemini-3.8-flash-high"
    SHORT_3_8_FLASH = "3.8-flash"


class Codex6TerraAlias(StrEnum):
    """Supported shorthand aliases that resolve to Codex `gpt-6-terra` (`테라(6)`)."""

    TERRA = "terra"
    SIX_TERRA = "6-terra"
    SIX_SP_TERRA = "6 terra"
    TERRA_PAREN_6 = "terra(6)"
    KO_TERRA = "테라"
    KO_TERRA_PAREN_6 = "테라(6)"
    SIX_SP_KO_TERRA = "6 테라"
    GPT_6_TERRA = "gpt-6-terra"
    GPT6_TERRA = "gpt6-terra"


class Codex6LunaAlias(StrEnum):
    """Supported shorthand aliases that resolve to Codex `gpt-6-luna` (`루나(6)`)."""

    LUNA = "luna"
    SIX_LUNA = "6-luna"
    SIX_SP_LUNA = "6 luna"
    LUNA_PAREN_6 = "luna(6)"
    KO_LUNA = "루나"
    KO_LUNA_PAREN_6 = "루나(6)"
    SIX_SP_KO_LUNA = "6 루나"
    GPT_6_LUNA = "gpt-6-luna"
    GPT6_LUNA = "gpt6-luna"


class Codex6SolAlias(StrEnum):
    """Supported shorthand aliases that resolve to Codex `gpt-6-sol` (`솔(6)`)."""

    SOL = "sol"
    SIX_SOL = "6-sol"
    SIX_SP_SOL = "6 sol"
    SOL_PAREN_6 = "sol(6)"
    KO_SOL = "솔"
    KO_SOL_PAREN_6 = "솔(6)"
    SIX_SP_KO_SOL = "6 솔"
    GPT_6_SOL = "gpt-6-sol"
    GPT6_SOL = "gpt6-sol"


class Codex6AstraAlias(StrEnum):
    """Supported shorthand aliases that resolve to Codex `gpt-6-astra` (`아스트라(6)`)."""

    ASTRA = "astra"
    SIX = "6"
    SIX_ASTRA = "6-astra"
    SIX_SP_ASTRA = "6 astra"
    ASTRA_PAREN_6 = "astra(6)"
    KO_ASTRA = "아스트라"
    KO_ASTRA_PAREN_6 = "아스트라(6)"
    SIX_SP_KO_ASTRA = "6 아스트라"
    GPT_6_ASTRA = "gpt-6-astra"
    GPT6_ASTRA = "gpt6-astra"


class Codex61TerraAlias(StrEnum):
    """Supported shorthand aliases that resolve to Codex `gpt-6.1-terra` (`테라(6-1)`)."""

    SIX_1_DASH_TERRA = "6-1-terra"
    SIX_1_DOT_TERRA = "6.1-terra"
    SIX_1_SP_TERRA = "6-1 terra"
    SIX_1_DOT_SP_TERRA = "6.1 terra"
    GPT_6_1_DASH_TERRA = "gpt-6-1-terra"
    GPT_6_1_TERRA = "gpt-6.1-terra"
    GPT6_1_TERRA = "gpt6.1-terra"
    TERRA_PAREN_6_1 = "terra(6-1)"
    TERRA_PAREN_6_1_DOT = "terra(6.1)"
    KO_TERRA_PAREN_6_1 = "테라(6-1)"
    KO_TERRA_PAREN_6_1_DOT = "테라(6.1)"
    SIX_1_SP_KO_TERRA = "6-1 테라"
    SIX_1_DOT_SP_KO_TERRA = "6.1 테라"


class Codex61SolAlias(StrEnum):
    """Supported shorthand aliases that resolve to Codex `gpt-6.1-sol` (`솔(6-1)`)."""

    SIX_1_DASH = "6-1"
    SIX_1_DOT = "6.1"
    SIX_1_DASH_SOL = "6-1-sol"
    SIX_1_DOT_SOL = "6.1-sol"
    SIX_1_SP_SOL = "6-1 sol"
    SIX_1_DOT_SP_SOL = "6.1 sol"
    GPT_6_1_DASH_SOL = "gpt-6-1-sol"
    GPT_6_1_SOL = "gpt-6.1-sol"
    GPT6_1_SOL = "gpt6.1-sol"
    SOL_PAREN_6_1 = "sol(6-1)"
    SOL_PAREN_6_1_DOT = "sol(6.1)"
    KO_SOL_PAREN_6_1 = "솔(6-1)"
    KO_SOL_PAREN_6_1_DOT = "솔(6.1)"
    SIX_1_SP_KO_SOL = "6-1 솔"
    SIX_1_DOT_SP_KO_SOL = "6.1 솔"


class EngineAliasKind(StrEnum):
    """Shorthand engine aliases normalized by RoutingPolicyEngine."""

    CLI = "cli"
    CODEX_CLI_DASH = "codex-cli"
    AGY = "agy"
    ANTIGRAVITY_RPC_DASH = "antigravity-rpc"
    RPC = "rpc"
    OPENAI_HTTP = "openai_http"


class ProviderAliasKind(StrEnum):
    """Shorthand provider aliases normalized by RoutingPolicyEngine."""

    OPENAI = "openai"
    CODEX = "codex"
    GOOGLE_ANTIGRAVITY = "google_antigravity"
    AGY = "agy"


class RoutingErrorCode(StrEnum):
    """Machine-readable pre-dispatch and execution error codes."""

    INVALID_REASONING_TIER = "INVALID_REASONING_TIER"
    UNSUPPORTED_ENGINE = "UNSUPPORTED_ENGINE"
    PROVIDER_ENGINE_MISMATCH = "PROVIDER_ENGINE_MISMATCH"
    MODEL_NOT_SUPPORTED_BY_ENGINE = "MODEL_NOT_SUPPORTED_BY_ENGINE"
    UNRECOGNIZED_CODEX_MODEL = "UNRECOGNIZED_CODEX_MODEL"
    CLAUDE_TIER_POLICY_VIOLATION = "CLAUDE_TIER_POLICY_VIOLATION"
    INPUT_BUDGET_EXCEEDED = "INPUT_BUDGET_EXCEEDED"
    ACTUAL_MODEL_MISMATCH = "ACTUAL_MODEL_MISMATCH"
    ANTIGRAVITY_CIRCUIT_OPEN = "ANTIGRAVITY_CIRCUIT_OPEN"
    ANTIGRAVITY_QUOTA_EXHAUSTED = "ANTIGRAVITY_QUOTA_EXHAUSTED"


class BudgetTerminationReason(StrEnum):
    """Termination reasons recorded in BudgetEnvelope."""

    COMPLETED = "completed"
    TIMEOUT_BUDGET_REACHED = "timeout_budget_reached"
    MODEL_MISMATCH = "model_mismatch"
    PROCESS_EXIT_ERROR = "process_exit_error"
    HTTP_ERROR = "http_error"
    RPC_ERROR = "rpc_error"
    CIRCUIT_BREAKER_OPEN = "circuit_breaker_open"
    ENDPOINT_UNAVAILABLE = "endpoint_unavailable"
    QUOTA_EXHAUSTED = "quota_exhausted"


class DecisionAuthorityKind(StrEnum):
    """Adjudication authority identifiers for Tri-Polar collaboration outcomes."""

    GEMINI_4_ARGON = "gemini-4-argon"
    PENDING_ARGON_OR_USER = "pending_argon_or_user_adjudication"
    ABORTED_AT_SONNET_STAGE = "aborted_at_sonnet_stage"
    REQUIRES_CALLER_OR_ARGON = "requires_caller_or_argon_adjudication"


class CodegraphStatusKind(StrEnum):
    """Status values for `.codegraph/codegraph.db` workspace inspection."""

    INDEXED = "indexed"
    UNINDEXED = "unindexed"


class TestEvidenceOriginKind(StrEnum):
    """Origin classifications for executed test command evidence."""

    BRIDGE_EXECUTED = "bridge_executed"
    CALLER_REPORTED = "caller_reported"


class CollaborationPersistenceState(StrEnum):
    """Persistence states for triad collaboration snapshots in JobStore."""

    RECORDED = "recorded"
    CANCELLED = "cancelled"
    FAILED = "failed"


class CompactScopeField(StrEnum):
    """Digest-oriented evidence scope keys preserved during snapshot compaction."""

    CODEGRAPH = "codegraph"
    SCOPE_DIGEST = "scope_digest"
    INLINE_CONTEXT_BYTES = "inline_context_bytes"
    INLINE_CONTEXT_DIGEST = "inline_context_digest"
    FILES_INSPECTED_DIGESTS = "files_inspected_digests"
    UNVERIFIED_OR_MISSING_FILES = "unverified_or_missing_files"
    SYMBOLS_INSPECTED = "symbols_inspected"
    EVIDENCE_DIMENSIONS = "evidence_dimensions"
    TEST_EXECUTION_EVIDENCE_ORIGIN = "test_execution_evidence_origin"


class OptimizationAxisKind(StrEnum):
    """Default trade-off optimization axes for differential candidate synthesis."""

    LATENCY_AND_THROUGHPUT = "latency_and_throughput"
    MEMORY_FOOTPRINT = "memory_footprint"
    MAINTAINABILITY_AND_CLARITY = "maintainability_and_clarity"


class SandboxModeKind(StrEnum):
    """Execution sandbox policies for Codex CLI."""

    READ_ONLY = "read-only"
    WORKSPACE_WRITE = "workspace-write"


class TaskTypeKind(StrEnum):
    """Durable JobStore task category identifiers."""

    TRIAD_ORCHESTRATION = "triad_orchestration"
    DAG_ULW = "dag_ulw"
    CODEX_CLI_EXEC = "codex_cli_exec"
    AGY_RPC_PROMPT = "agy_rpc_prompt"
    HTTP_FALLBACK = "http_fallback"
    RESCUE_SANDBOX = "rescue_sandbox"


class CodexBinaryCandidatePath(StrEnum):
    """Default filesystem candidate paths for locating the `codex` CLI executable."""

    HOMEBREW_BIN = "/opt/homebrew/bin/codex"
    USR_LOCAL_BIN = "/usr/local/bin/codex"
    CHATGPT_APP_BUNDLE = (
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex"
    )
