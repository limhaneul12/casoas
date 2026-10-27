"""Explicit TypedDict contracts for evidence scopes, CodeGraph snapshots, and triad adjudication."""

from __future__ import annotations

from typing import NotRequired, TypedDict

from codex_collab_mcp.schemas import JsonObject


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
    """Verification status across each evidence dimension."""

    static_code_review: str
    test_generation: str
    test_execution: str
    integration_verification: str
    runtime_confirmation: str


class EvidenceScopeDict(TypedDict):
    """Full evidence scope envelope attached to collaboration outputs."""

    codegraph: CodegraphIndexSnapshotDict
    scope_digest: str
    inline_context_bytes: int
    inline_context_digest: str
    files_inspected_digests: dict[str, str]
    unverified_or_missing_files: list[str]
    symbols_inspected: list[str]
    evidence_dimensions: EvidenceDimensionsDict
    executed_test_commands: list[JsonObject]
    test_execution_evidence_origin: NotRequired[str]


class RoleAdjudicationSummaryDict(TypedDict):
    """Per-role state and review artifact validation summary."""

    state: str
    verdict: str | None
    has_artifact: bool
    has_valid_review: bool


class TriadAdjudicationDict(TypedDict):
    """Full adjudication result separating role execution state from consensus state."""

    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    required_roles: list[str]
    succeeded_roles: list[str]
    incomplete_roles: list[str]
    role_states: dict[str, RoleAdjudicationSummaryDict]
    missing_artifact_roles: list[str]
    missing_verdict_roles: list[str]
    missing_review_roles: list[str]
    conflict_detected: bool
    conflict_basis: str | None
    decision_authority: str
    review_approved: bool


class TaskHashPayloadDict(TypedDict):
    """Canonical task input tuple hashed to produce `source_task_digest`."""

    task_spec: str
    target_code_or_context: str
    invariants_and_rules: str
    language: str
    mode: str


class InlineScopeDigestPayloadDict(TypedDict):
    """Canonical inline context and file/symbol digest payload for `scope_digest`."""

    inline_context_sha256: str
    file_hashes: dict[str, str]
    symbols: list[str]
