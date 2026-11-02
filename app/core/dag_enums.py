"""Canonical StrEnum definitions for Tri-Polar Graph Engineering (`DAG-ULW++`)."""

from __future__ import annotations

from enum import StrEnum


class DagNodeRoleKind(StrEnum):
    """Role archetypes in the Tri-Polar Graph Engineering (`DAG-ULW++`) topology."""

    TOPOLOGY_PLANNER = "topology_planner"
    REPRODUCTION_PROBER = "reproduction_prober"
    INTERFACE_CONTRACTOR = "interface_contractor"
    DISJOINT_BUILDER = "disjoint_builder"
    ADVERSARIAL_AUDITOR = "adversarial_auditor"
    VERIFICATION_FAN_IN = "verification_fan_in"


class DagNodeKind(StrEnum):
    """Execution semantics classification for a DAG node."""

    STANDARD = "standard"
    REPRODUCTION = "reproduction"
    VERIFICATION = "verification"


class DagHazardCode(StrEnum):
    """Machine-readable preflight hazard and contract rejection codes for DAG compilation."""

    DAG_EMPTY_GRAPH = "DAG_EMPTY_GRAPH"
    DAG_DUPLICATE_NODE_ID = "DAG_DUPLICATE_NODE_ID"
    DAG_UNKNOWN_DEPENDENCY = "DAG_UNKNOWN_DEPENDENCY"
    DAG_SELF_DEPENDENCY = "DAG_SELF_DEPENDENCY"
    DAG_CYCLE_DETECTED = "DAG_CYCLE_DETECTED"
    DAG_INVALID_SCOPE_PATH = "DAG_INVALID_SCOPE_PATH"
    DAG_SPAWN_PLAN_REQUIRED = "DAG_SPAWN_PLAN_REQUIRED"
    DAG_WAW_SCOPE_CONFLICT = "DAG_WAW_SCOPE_CONFLICT"
    DAG_MISSING_RAW_EDGE = "DAG_MISSING_RAW_EDGE"


class DagIsolationMode(StrEnum):
    """Per-wave workspace isolation strategies for concurrent writers."""

    DIRECT = "direct"
    SERIAL = "serial"


class DagSettlementStage(StrEnum):
    """Two-stage settlement lifecycle states (`STOPPED != SETTLED`) for DAG nodes."""

    QUEUED = "queued"
    RUNNING = "running"
    SETTLED = "settled"
    REJECTED_OUT_OF_SCOPE_WRITE = "rejected_out_of_scope_write"
    REJECTED_DIRTY_READ = "rejected_dirty_read"
    REJECTED_VERIFY_FAILED = "rejected_verify_failed"
    REJECTED_REPRODUCTION_NOT_OBSERVED = "rejected_reproduction_not_observed"
    BLOCKED_UPSTREAM = "blocked_upstream"
    FAILED_EXECUTION = "failed_execution"


class DagRunState(StrEnum):
    """Aggregate lifecycle state of a `heterarchy_dag_ulw` campaign."""

    PLANNED = "planned"
    REJECTED_PREFLIGHT = "rejected_preflight"
    SETTLED = "settled"
    PARTIAL_BLOCKED = "partial_blocked"
    FAILED = "failed"


class DagIgnoredDirectory(StrEnum):
    """Workspace directory names excluded from full-workspace SHA-256 manifest scans."""

    GIT = ".git"
    CODEGRAPH = ".codegraph"
    PYCACHE = "__pycache__"
    VENV = ".venv"
    NODE_MODULES = "node_modules"
    PYTEST_CACHE = ".pytest_cache"
    RUFF_CACHE = ".ruff_cache"
    MYPY_CACHE = ".mypy_cache"


class TuiSlashCommand(StrEnum):
    """Interactive slash commands supported by the `hcodex tui` workbench."""

    HELP = "/help"
    ASK = "/ask"
    PROMPT = "/prompt"
    SONNET = "/sonnet"
    OPUS = "/opus"
    ARGON = "/argon"
    FLASH = "/flash"
    USE = "/use"
    MODEL = "/model"
    SKILLS = "/skills"
    SKILL = "/skill"
    DAG_MODE = "/dag-mode"
    DAG_EXIT = "/dag-exit"
    DAG_ULW = "/dag-ulw"
    DAG = "/dag"
    DAG_RUN = "/dag-run"
    EFFECT = "/effect"
    EFFORT = "/effort"
    REASONING = "/reasoning"
    DAG_EFFECT = "/dag-effect"
    PROMPTS = "/prompts"
    PROMT = "/promt"
    TRACE = "/trace"
    BACK = "/back"
    PREV = "/prev"
    COMPACT = "/compact"
    CLEAR = "/clear"
    DIFF = "/diff"
    REVIEW = "/review"
    PERMISSIONS = "/permissions"
    APPROVALS = "/approvals"
    RESUME = "/resume"
    TRIAD = "/triad"
    MODELS = "/models"
    LANES = "/lanes"
    OMH = "/omh"
    BOARD = "/board"
    TODO = "/todo"
    PLAN = "/plan"
    STATUS = "/status"
    INBOX = "/inbox"
    JOBS = "/jobs"
    THEME = "/theme"
    QUIT = "/quit"
    EXIT = "/exit"


class DagVisualEffectStyle(StrEnum):
    """Visual and wave-animation effect styles for DAG-ULW++ agent execution."""

    NEON_PULSE = "neon-pulse"
    CYBER_MATRIX = "cyber-matrix"
    ROYAL_PRISM = "royal-prism"
    MINIMAL_MONO = "minimal-mono"


class DagAgentPoleSelector(StrEnum):
    """Agent pole selectors for `/effect` and `/effort` DAG configuration."""

    ARGON = "argon"
    CLAUDE = "claude"
    SONNET = "sonnet"
    OPUS = "opus"
    CODEX = "codex"
    ALL = "all"


class DagEffectListToken(StrEnum):
    """Subcommand tokens that display the `/effect` configuration matrix."""

    LIST = "list"
    STATUS = "status"
    MATRIX = "matrix"
    SHOW = "show"


class DefaultCoreSkill(StrEnum):
    """Default pre-activated core skills inside `hcodex tui`."""

    CODEX_HETERARCHY = "codex-heterarchy"
    CODEGRAPH_OPERATOR = "codegraph-operator"
    CODEBASE_REFACTOR_GUARDIAN = "codebase-refactor-guardian"


class DefaultDagNodeId(StrEnum):
    """Canonical node identifiers for the 4-node Tri-Polar DAG-ULW++ campaign."""

    N1_TOPOLOGY_SPEC = "n1_topology_spec"
    N2_INTERFACE_CRAFT = "n2_interface_craft"
    N3_RED_TEAM_AUDIT = "n3_red_team_audit"
    N4_VERIFY_FAN_IN = "n4_verify_fan_in"


class SkillScopeKind(StrEnum):
    """Discovery scope classification for registered `$skill` entries."""

    PLUGIN = "plugin"
    GLOBAL = "global"
    HARNESS = "harness"
    BUILTIN = "builtin"


class TuiPermissionMode(StrEnum):
    """OpenAI Codex CLI-compatible sandbox and approval modes for `hcodex tui`."""

    READ_ONLY = "read-only"
    AUTO_EDIT = "auto-edit"
    YOLO = "YOLO mode"


class TuiAnsiStyle(StrEnum):
    """Canonical ANSI SGR color and background escape sequences for `hcodex tui`."""

    CYAN = "\033[38;5;45m"
    TEAL = "\033[38;5;43m"
    EMERALD = "\033[38;5;48m"
    AMBER = "\033[38;5;214m"
    VIOLET = "\033[38;5;141m"
    PINK = "\033[38;5;205m"
    EMBLEM_DIM = "\033[38;5;238m"
    DIM = "\033[2m"
    BOLD = "\033[1m"
    RESET = "\033[0m"
    BG_BAR = "\033[48;5;236m"
    FG_CYAN_SKILL = "\033[48;5;236;1;38;5;117m"
    FG_TEXT = "\033[48;5;236;38;5;255m"
    FG_PLACEHOLDER = "\033[48;5;236;38;5;244m"
    CURSOR_BLOCK = "\033[48;5;255;38;5;236m \033[48;5;236m"


__all__ = [
    "DagAgentPoleSelector",
    "DagEffectListToken",
    "DagHazardCode",
    "DagIgnoredDirectory",
    "DagIsolationMode",
    "DagNodeKind",
    "DagNodeRoleKind",
    "DagRunState",
    "DagSettlementStage",
    "DagVisualEffectStyle",
    "DefaultCoreSkill",
    "DefaultDagNodeId",
    "SkillScopeKind",
    "TuiAnsiStyle",
    "TuiPermissionMode",
    "TuiSlashCommand",
]
