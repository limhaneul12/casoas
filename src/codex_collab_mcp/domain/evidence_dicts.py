"""Explicit TypedDict contracts for evidence scopes, CodeGraph snapshots, and triad adjudication."""

from __future__ import annotations

from typing import TypedDict

from codex_collab_mcp.schemas import JsonObject, JsonValue


class CodegraphIndexSnapshotDict(TypedDict):
    """Serialized representation of a CodegraphIndexSnapshot dataclass."""

    status: str
    snapshot_id: str | None
    workspace_dir: str
    db_mtime_utc: str | None
    db_size_bytes: int | None
    note: str | None


class CodegraphSymbolSliceDict(TypedDict):
    """Serialized representation of a CodegraphSymbolSlice dataclass."""

    name: str
    qualified_name: str
    kind: str
    file_path: str
    start_line: int
    end_line: int
    callers: list[str]
    callees: list[str]
    source_excerpt: str


class EvidenceDimensionsDict(TypedDict):
    """Five verification dimensions for an evidence scope or role snapshot."""

    static_code_review: str
    test_generation: str
    test_execution: str
    integration_verification: str
    runtime_confirmation: str


class EvidenceScopeDict(TypedDict):
    """Full evidence scope dictionary attached to a collaboration response."""

    codegraph: JsonObject
    scope_digest: str
    inline_context_bytes: int
    inline_context_digest: str
    files_inspected_digests: JsonObject
    unverified_or_missing_files: list[JsonValue]
    symbols_inspected: list[JsonValue]
    evidence_dimensions: JsonObject
    executed_test_commands: list[JsonValue]


class RoleAdjudicationSummaryDict(TypedDict):
    """Per-role state and review validity entry inside `adjudicate_collaboration_and_consensus`."""

    state: str
    verdict: str | None
    has_artifact: bool
    has_valid_review: bool


class TriadAdjudicationDict(TypedDict):
    """Multi-role collaboration and consensus adjudication result."""

    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    required_roles: list[str]
    succeeded_roles: list[str]
    incomplete_roles: list[str]
    role_states: JsonObject
    missing_artifact_roles: list[str]
    missing_verdict_roles: list[str]
    missing_review_roles: list[str]
    conflict_detected: bool
    conflict_basis: str | None
    decision_authority: str
    review_approved: bool
