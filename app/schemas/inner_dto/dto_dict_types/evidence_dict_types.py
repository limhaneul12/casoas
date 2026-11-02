"""TypedDict return-type contracts for `evidence_dtos.py` `.to_dict(self)` methods."""

from __future__ import annotations

from typing import NotRequired, TypedDict

from app.schemas.boundary_schemas import JsonObject, JsonValue


class CodegraphIndexSnapshotDict(TypedDict):
    """Serialized representation of a `CodegraphIndexSnapshot` dataclass."""

    status: str
    snapshot_id: str | None
    workspace_dir: str
    db_mtime_utc: str | None
    db_size_bytes: int | None
    note: str | None


class EvidenceDimensionsDict(TypedDict):
    """Serialized representation of an `EvidenceDimensions` dataclass."""

    static_code_review: str
    test_generation: str
    test_execution: str
    integration_verification: str
    runtime_confirmation: str


class EvidenceScopeDict(TypedDict):
    """Serialized representation of an `EvidenceScope` dataclass."""

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
    """Serialized representation of a `RoleAdjudicationSummary` dataclass."""

    state: str
    verdict: str | None
    has_artifact: bool
    has_valid_review: bool


class TriadAdjudicationDict(TypedDict):
    """Serialized representation of a `TriadAdjudication` dataclass."""

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
    """Serialized representation of a `TaskHashPayload` dataclass."""

    task_spec: str
    target_code_or_context: str
    invariants_and_rules: str
    language: str
    mode: str


class InlineScopeDigestPayloadDict(TypedDict):
    """Serialized representation of an `InlineScopeDigestPayload` dataclass."""

    inline_context_sha256: str
    file_hashes: dict[str, str]
    symbols: list[str]


class RoleStatusStubDict(TypedDict):
    """Serialized representation of a `RoleStatusStub` dataclass."""

    status: str | None
    ok: bool | None


class CompactedTestCommandEvidenceDict(TypedDict):
    """Serialized representation of a `CompactedTestCommandEvidence` dataclass."""

    item_digest: str
    passed_is_boolean: bool


class TriadRoleSnapshotDict(TypedDict):
    """Serialized representation of a `TriadRoleSnapshot` dataclass."""

    state: str
    response_present: bool
    response_status: str | None
    response_ok: bool | None
    verdict: str | None
    review_artifact_present: bool
    review_artifact_has_narrative: bool
    review_artifact_digest: str | None
    generated_tests_present: bool
    evidence_dimensions: JsonObject


class TriadAggregateSnapshotDict(TypedDict):
    """Serialized representation of a `TriadAggregateSnapshot` dataclass."""

    schema_version: int
    task_type: str
    collaboration_id: str
    orchestration_mode: str
    source_task_digest: str
    workspace_dir: str
    evidence_scope: JsonObject
    roles: JsonObject
    adjudication: JsonObject


class ArgonTicketErrorResultDict(TypedDict):
    """Serialized representation of an `ArgonTicketErrorResult` dataclass."""

    ok: bool
    status: str
    error: str


class ArgonHandoffRoleResultDict(TypedDict):
    """Serialized representation of an `ArgonHandoffRoleResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    handoff_ticket: JsonObject


class TriadParticipantsDict(TypedDict):
    """Serialized representation of a `TriadParticipants` dataclass."""

    macro_architect: str
    code_craftsman: JsonValue
    red_team_auditor: JsonValue


class TriadSonnetAbortPayloadDict(TypedDict):
    """Serialized representation of a `TriadSonnetAbortPayload` dataclass."""

    ok: bool
    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    triad_consensus_ready: bool
    orchestration_mode: str
    failed_stage: str
    adjudication: JsonObject
    sonnet_craft: JsonObject


class TriadCompletedPayloadDict(TypedDict):
    """Serialized representation of a `TriadCompletedPayload` dataclass."""

    ok: bool
    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    triad_consensus_ready: bool
    orchestration_mode: str
    participants: JsonObject
    adjudication: JsonObject
    argon_review: JsonObject | None
    sonnet_craft: JsonObject
    codex_audit: JsonObject
    codex_verdict: str | None


__all__ = [
    "ArgonHandoffRoleResultDict",
    "ArgonTicketErrorResultDict",
    "CodegraphIndexSnapshotDict",
    "CompactedTestCommandEvidenceDict",
    "EvidenceDimensionsDict",
    "EvidenceScopeDict",
    "InlineScopeDigestPayloadDict",
    "RoleAdjudicationSummaryDict",
    "RoleStatusStubDict",
    "TaskHashPayloadDict",
    "TriadAdjudicationDict",
    "TriadAggregateSnapshotDict",
    "TriadCompletedPayloadDict",
    "TriadParticipantsDict",
    "TriadRoleSnapshotDict",
    "TriadSonnetAbortPayloadDict",
]
