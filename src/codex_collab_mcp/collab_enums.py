"""Canonical StrEnum definitions for routing, execution, consensus, schemas, and CLI control."""

from __future__ import annotations

from enum import StrEnum


class ProviderKind(StrEnum):
    """Supported model providers in the Tri-Polar Heterarchy."""

    ANTIGRAVITY = "antigravity"
    OPENAI_CODEX = "openai_codex"
    LOCAL_HTTP = "local_http"


class EngineKind(StrEnum):
    """Supported execution transport engines."""

    ANTIGRAVITY_LOCAL_RPC = "antigravity_local_rpc"
    CODEX_CLI = "codex_cli"
    HTTP = "http"


class AuthContextKind(StrEnum):
    """Authentication context boundary identifiers."""

    ANTIGRAVITY_LOCAL_CSRF = "antigravity_local_csrf"
    CODEX_CHATGPT_OAUTH = "codex_chatgpt_oauth"
    HTTP_BEARER = "http_bearer"


class ReasoningTier(StrEnum):
    """Supported model reasoning effort tiers."""

    MINIMAL = "minimal"
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    XHIGH = "xhigh"


class CanonicalModelId(StrEnum):
    """Canonical model identifiers across Antigravity and OpenAI Codex."""

    CLAUDE_SONNET_5_5_MEDIUM = "claude-sonnet-5-5-medium"
    CLAUDE_SONNET_5_5_HIGH = "claude-sonnet-5-5-high"
    CLAUDE_SONNET_5_5_LOW = "claude-sonnet-5-5-low"
    CLAUDE_OPUS_5_5_MEDIUM = "claude-opus-5-5-medium"
    CLAUDE_OPUS_5_5_HIGH = "claude-opus-5-5-high"
    CLAUDE_OPUS_5_5_LOW = "claude-opus-5-5-low"
    GEMINI_4_ARGON_HIGH = "gemini-4-argon-high"
    GEMINI_4_ARGON_MEDIUM = "gemini-4-argon-medium"
    GEMINI_4_ARGON_LOW = "gemini-4-argon-low"
    GEMINI_3_8_FLASH_HIGH = "gemini-3.8-flash-high"
    GEMINI_3_8_FLASH_MEDIUM = "gemini-3.8-flash-medium"
    GEMINI_3_8_FLASH_LOW = "gemini-3.8-flash-low"
    GPT_6_1_SOL = "gpt-6.1-sol"
    GPT_5_4 = "gpt-5.4"
    GPT_5_CODEX = "gpt-5-codex"
    GPT_5_1 = "gpt-5.1"
    GPT_5 = "gpt-5"
    GPT_5_NANO = "gpt-5-nano"
    O3 = "o3"
    O4_MINI = "o4-mini"
    CODEX_DEFAULT = "codex-default"


class TriadRole(StrEnum):
    """Roles participating in Tri-Polar collaboration and handoffs."""

    ARGON = "argon"
    SONNET = "sonnet"
    OPUS = "opus"
    CODEX = "codex"


class RoleExecutionState(StrEnum):
    """Per-role execution lifecycle states."""

    NOT_STARTED = "not_started"
    QUEUED = "queued"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    TIMED_OUT_UNKNOWN = "timed_out_unknown"


class CollaborationState(StrEnum):
    """Aggregate collaboration completion states."""

    COMPLETE = "complete"
    PARTIAL = "partial"
    FAILED = "failed"


class ConsensusState(StrEnum):
    """Aggregate Tri-Polar consensus adjudication states."""

    CONSENSUS_VERIFIED = "consensus_verified"
    INCOMPLETE_NO_CONSENSUS = "incomplete_no_consensus"
    CONFLICT_REQUIRES_ADJUDICATION = "conflict_requires_adjudication"
    BLOCKED_BY_RED_TEAM = "blocked_by_red_team"


class AuditVerdict(StrEnum):
    """Explicit structured review and audit verdicts."""

    PASS = "PASS"  # noqa: S105
    WARN = "WARN"
    BLOCK = "BLOCK"


class AuditSeverity(StrEnum):
    """Severity levels for adversarial audit findings."""

    CRITICAL = "CRITICAL"
    HIGH = "HIGH"
    MEDIUM = "MEDIUM"
    LOW = "LOW"


class EvidenceDimensionState(StrEnum):
    """Verification states for individual evidence dimensions."""

    EXECUTED_PASS = "executed_pass"  # noqa: S105
    EXECUTED_FAIL = "executed_fail"
    PERFORMED = "performed"
    GENERATED_NOT_EXECUTED = "generated_not_executed"
    NOT_RUN = "not_run"


class JobStatusKind(StrEnum):
    """Durable job store status values."""

    RUNNING = "running"
    COMPLETED = "completed"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    TIMED_OUT_UNKNOWN = "timed_out_unknown"
    CANCELLED = "cancelled"
    REJECTED_PRE_DISPATCH = "rejected_pre_dispatch"
    INPUT_BUDGET_EXCEEDED = "input_budget_exceeded"


class OutcomeStateKind(StrEnum):
    """Explicit outcome state classifications for execution receipts and jobs."""

    IN_FLIGHT = "in_flight"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CONFIRMED_SUCCESS = "confirmed_success"
    CONFIRMED_FAILURE = "confirmed_failure"
    OUTCOME_UNKNOWN = "outcome_unknown"
    CANCELLED = "cancelled"
    REJECTED_PRE_DISPATCH = "rejected_pre_dispatch"


class HandoffStatusKind(StrEnum):
    """Handoff blackboard ticket lifecycle statuses."""

    PENDING = "pending"
    IN_PROGRESS = "in_progress"
    COMPLETED = "completed"
    REJECTED = "rejected"


class HandoffPriorityKind(StrEnum):
    """Priority tiers for handoff tickets."""

    LOW = "low"
    NORMAL = "normal"
    HIGH = "high"
    CRITICAL = "critical"


class OrchestrationMode(StrEnum):
    """Tri-Polar orchestration execution topologies."""

    PIPELINE = "pipeline"
    COUNCIL = "council"


class SchemaName(StrEnum):
    """Canonical structured output schema identifiers."""

    ADVERSARIAL_AUDIT = "adversarial_audit"
    TARGETED_SYNTHESIS = "targeted_synthesis"
    DIFFERENTIAL_CANDIDATES = "differential_candidates"
    RESCUE_DIAGNOSIS = "rescue_diagnosis"
    CLAUDE_CRAFT = "claude_craft"
    ARGON_REVIEW = "argon_review"


class CtlRelativePath(StrEnum):
    """Relative filesystem paths managed by the hcodex CLI controller."""

    PLUGIN_SRC = "plugin/heterarchy-codex"
    SKILL_SRC = "plugin/heterarchy-codex/skills/codex-heterarchy"
    REPO_SKILL_SRC = "skills/codex-heterarchy"
    AGY_PLUGIN_ACTIVE = ".gemini/config/plugins/heterarchy-codex"
    AGY_PLUGIN_DISABLED = ".gemini/config/plugins-disabled/heterarchy-codex"
    AGY_GLOBAL_SKILL = ".gemini/config/skills/codex-heterarchy"
    CODEX_GLOBAL_SKILL = ".codex/skills/codex-heterarchy"
    CODEX_CONFIG_PATH = ".codex/config.toml"
    VENV_DIR = ".venv"


class CtlCommand(StrEnum):
    """Supported subcommands for the hcodex CLI controller."""

    STATUS = "status"
    ON = "on"
    OFF = "off"
    RESTART = "restart"
    INBOX = "inbox"
