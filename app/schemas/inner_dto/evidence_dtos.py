"""Internal frozen dataclass DTOs for evidence scopes, CodeGraph snapshots, and triad adjudication."""

from __future__ import annotations

from dataclasses import dataclass

from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types import (
    ArgonHandoffRoleResultDict,
    ArgonTicketErrorResultDict,
    CodegraphIndexSnapshotDict,
    CompactedTestCommandEvidenceDict,
    EvidenceDimensionsDict,
    EvidenceScopeDict,
    InlineScopeDigestPayloadDict,
    RoleAdjudicationSummaryDict,
    RoleStatusStubDict,
    TaskHashPayloadDict,
    TriadAdjudicationDict,
    TriadAggregateSnapshotDict,
    TriadCompletedPayloadDict,
    TriadParticipantsDict,
    TriadRoleSnapshotDict,
    TriadSonnetAbortPayloadDict,
)


@dataclass(frozen=True, slots=True)
class EvidenceDimensions:
    """Internal DTO for verification status across each evidence dimension."""

    static_code_review: str
    test_generation: str
    test_execution: str
    integration_verification: str
    runtime_confirmation: str

    def to_dict(self) -> EvidenceDimensionsDict:
        """Serialize the evidence dimensions to a TypedDict.

        Returns:
            Typed `EvidenceDimensionsDict` mapping.
        """
        return EvidenceDimensionsDict(
            static_code_review=self.static_code_review,
            test_generation=self.test_generation,
            test_execution=self.test_execution,
            integration_verification=self.integration_verification,
            runtime_confirmation=self.runtime_confirmation,
        )


@dataclass(frozen=True, slots=True)
class EvidenceScope:
    """Internal DTO for the full evidence scope envelope attached to collaboration outputs."""

    codegraph: CodegraphIndexSnapshotDict
    scope_digest: str
    inline_context_bytes: int
    inline_context_digest: str
    files_inspected_digests: dict[str, str]
    unverified_or_missing_files: list[str]
    symbols_inspected: list[str]
    evidence_dimensions: EvidenceDimensionsDict
    executed_test_commands: list[JsonObject]
    test_execution_evidence_origin: str = ""

    def to_dict(self) -> EvidenceScopeDict:
        """Serialize the evidence scope to a TypedDict.

        Returns:
            Typed `EvidenceScopeDict` mapping.
        """
        if self.test_execution_evidence_origin:
            return EvidenceScopeDict(
                codegraph=self.codegraph,
                scope_digest=self.scope_digest,
                inline_context_bytes=self.inline_context_bytes,
                inline_context_digest=self.inline_context_digest,
                files_inspected_digests=self.files_inspected_digests,
                unverified_or_missing_files=self.unverified_or_missing_files,
                symbols_inspected=self.symbols_inspected,
                evidence_dimensions=self.evidence_dimensions,
                executed_test_commands=self.executed_test_commands,
                test_execution_evidence_origin=self.test_execution_evidence_origin,
            )
        return EvidenceScopeDict(
            codegraph=self.codegraph,
            scope_digest=self.scope_digest,
            inline_context_bytes=self.inline_context_bytes,
            inline_context_digest=self.inline_context_digest,
            files_inspected_digests=self.files_inspected_digests,
            unverified_or_missing_files=self.unverified_or_missing_files,
            symbols_inspected=self.symbols_inspected,
            evidence_dimensions=self.evidence_dimensions,
            executed_test_commands=self.executed_test_commands,
        )


@dataclass(frozen=True, slots=True)
class RoleAdjudicationSummary:
    """Internal DTO for per-role state and review artifact validation summary."""

    state: str
    verdict: str | None
    has_artifact: bool
    has_valid_review: bool

    def to_dict(self) -> RoleAdjudicationSummaryDict:
        """Serialize the role adjudication summary to a TypedDict.

        Returns:
            Typed `RoleAdjudicationSummaryDict` mapping.
        """
        return RoleAdjudicationSummaryDict(
            state=self.state,
            verdict=self.verdict,
            has_artifact=self.has_artifact,
            has_valid_review=self.has_valid_review,
        )


@dataclass(frozen=True, slots=True)
class TriadAdjudication:
    """Internal DTO separating role execution state from consensus state."""

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

    def to_dict(self) -> TriadAdjudicationDict:
        """Serialize the triad adjudication result to a TypedDict.

        Returns:
            Typed `TriadAdjudicationDict` mapping.
        """
        return TriadAdjudicationDict(
            collaboration_status=self.collaboration_status,
            consensus_status=self.consensus_status,
            consensus_reached=self.consensus_reached,
            required_roles=self.required_roles,
            succeeded_roles=self.succeeded_roles,
            incomplete_roles=self.incomplete_roles,
            role_states=self.role_states,
            missing_artifact_roles=self.missing_artifact_roles,
            missing_verdict_roles=self.missing_verdict_roles,
            missing_review_roles=self.missing_review_roles,
            conflict_detected=self.conflict_detected,
            conflict_basis=self.conflict_basis,
            decision_authority=self.decision_authority,
            review_approved=self.review_approved,
        )


@dataclass(frozen=True, slots=True)
class TaskHashPayload:
    """Internal DTO for the canonical task input tuple hashed to produce `source_task_digest`."""

    task_spec: str
    target_code_or_context: str
    invariants_and_rules: str
    language: str
    mode: str

    def to_dict(self) -> TaskHashPayloadDict:
        """Serialize the task hash payload to a TypedDict.

        Returns:
            Typed `TaskHashPayloadDict` mapping.
        """
        return TaskHashPayloadDict(
            task_spec=self.task_spec,
            target_code_or_context=self.target_code_or_context,
            invariants_and_rules=self.invariants_and_rules,
            language=self.language,
            mode=self.mode,
        )


@dataclass(frozen=True, slots=True)
class InlineScopeDigestPayload:
    """Internal DTO for the canonical inline context and file/symbol digest payload."""

    inline_context_sha256: str
    file_hashes: dict[str, str]
    symbols: list[str]

    def to_dict(self) -> InlineScopeDigestPayloadDict:
        """Serialize the inline scope digest payload to a TypedDict.

        Returns:
            Typed `InlineScopeDigestPayloadDict` mapping.
        """
        return InlineScopeDigestPayloadDict(
            inline_context_sha256=self.inline_context_sha256,
            file_hashes=self.file_hashes,
            symbols=self.symbols,
        )


@dataclass(frozen=True, slots=True)
class RoleStatusStub:
    """Internal DTO for recomputing role execution state from persisted snapshot fields."""

    status: str | None
    ok: bool | None

    def to_dict(self) -> RoleStatusStubDict:
        """Serialize the role status stub to a TypedDict.

        Returns:
            Typed `RoleStatusStubDict` mapping.
        """
        return RoleStatusStubDict(status=self.status, ok=self.ok)


@dataclass(frozen=True, slots=True)
class CompactedTestCommandEvidence:
    """Internal DTO for a compacted test command evidence item."""

    item_digest: str
    passed_is_boolean: bool

    def to_dict(self) -> CompactedTestCommandEvidenceDict:
        """Serialize the compacted test command evidence to a TypedDict.

        Returns:
            Typed `CompactedTestCommandEvidenceDict` mapping.
        """
        return CompactedTestCommandEvidenceDict(
            item_digest=self.item_digest,
            passed_is_boolean=self.passed_is_boolean,
        )


@dataclass(frozen=True, slots=True)
class TriadRoleSnapshot:
    """Internal DTO for a single role's execution and review evidence snapshot."""

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

    def to_dict(self) -> TriadRoleSnapshotDict:
        """Serialize the role snapshot to a TypedDict.

        Returns:
            Typed `TriadRoleSnapshotDict` mapping.
        """
        return TriadRoleSnapshotDict(
            state=self.state,
            response_present=self.response_present,
            response_status=self.response_status,
            response_ok=self.response_ok,
            verdict=self.verdict,
            review_artifact_present=self.review_artifact_present,
            review_artifact_has_narrative=self.review_artifact_has_narrative,
            review_artifact_digest=self.review_artifact_digest,
            generated_tests_present=self.generated_tests_present,
            evidence_dimensions=self.evidence_dimensions,
        )


@dataclass(frozen=True, slots=True)
class TriadAggregateSnapshot:
    """Internal DTO for an aggregate triad collaboration snapshot."""

    schema_version: int
    task_type: str
    collaboration_id: str
    orchestration_mode: str
    source_task_digest: str
    workspace_dir: str
    evidence_scope: JsonObject
    roles: JsonObject
    adjudication: JsonObject

    def to_dict(self) -> TriadAggregateSnapshotDict:
        """Serialize the aggregate triad snapshot to a TypedDict.

        Returns:
            Typed `TriadAggregateSnapshotDict` mapping.
        """
        return TriadAggregateSnapshotDict(
            schema_version=self.schema_version,
            task_type=self.task_type,
            collaboration_id=self.collaboration_id,
            orchestration_mode=self.orchestration_mode,
            source_task_digest=self.source_task_digest,
            workspace_dir=self.workspace_dir,
            evidence_scope=self.evidence_scope,
            roles=self.roles,
            adjudication=self.adjudication,
        )


@dataclass(frozen=True, slots=True)
class ArgonTicketErrorResult:
    """Internal DTO for a missing Argon handoff ticket error."""

    ok: bool
    status: str
    error: str

    def to_dict(self) -> ArgonTicketErrorResultDict:
        """Serialize the Argon ticket error result to a TypedDict.

        Returns:
            Typed `ArgonTicketErrorResultDict` mapping.
        """
        return ArgonTicketErrorResultDict(ok=self.ok, status=self.status, error=self.error)


@dataclass(frozen=True, slots=True)
class ArgonHandoffRoleResult:
    """Internal DTO for an Argon role response resolved from a handoff ticket."""

    ok: bool
    status: str
    outcome_state: str
    handoff_ticket: JsonObject

    def to_dict(self) -> ArgonHandoffRoleResultDict:
        """Serialize the Argon handoff role result to a TypedDict.

        Returns:
            Typed `ArgonHandoffRoleResultDict` mapping.
        """
        return ArgonHandoffRoleResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            handoff_ticket=self.handoff_ticket,
        )


@dataclass(frozen=True, slots=True)
class TriadParticipants:
    """Internal DTO for 3-pole participant model identifiers."""

    macro_architect: str
    code_craftsman: JsonValue
    red_team_auditor: JsonValue

    def to_dict(self) -> TriadParticipantsDict:
        """Serialize participant model identifiers to a TypedDict.

        Returns:
            Typed `TriadParticipantsDict` mapping.
        """
        return TriadParticipantsDict(
            macro_architect=self.macro_architect,
            code_craftsman=self.code_craftsman,
            red_team_auditor=self.red_team_auditor,
        )


@dataclass(frozen=True, slots=True)
class TriadSonnetAbortPayload:
    """Internal DTO for an aborted pipeline payload at the Sonnet stage."""

    ok: bool
    collaboration_status: str
    consensus_status: str
    consensus_reached: bool
    triad_consensus_ready: bool
    orchestration_mode: str
    failed_stage: str
    adjudication: JsonObject
    sonnet_craft: JsonObject

    def to_dict(self) -> TriadSonnetAbortPayloadDict:
        """Serialize the aborted pipeline payload to a TypedDict.

        Returns:
            Typed `TriadSonnetAbortPayloadDict` mapping.
        """
        return TriadSonnetAbortPayloadDict(
            ok=self.ok,
            collaboration_status=self.collaboration_status,
            consensus_status=self.consensus_status,
            consensus_reached=self.consensus_reached,
            triad_consensus_ready=self.triad_consensus_ready,
            orchestration_mode=self.orchestration_mode,
            failed_stage=self.failed_stage,
            adjudication=self.adjudication,
            sonnet_craft=self.sonnet_craft,
        )


@dataclass(frozen=True, slots=True)
class TriadCompletedPayload:
    """Internal DTO for a completed 3-pole collaboration response payload."""

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

    def to_dict(self) -> TriadCompletedPayloadDict:
        """Serialize the completed 3-pole payload to a TypedDict.

        Returns:
            Typed `TriadCompletedPayloadDict` mapping.
        """
        return TriadCompletedPayloadDict(
            ok=self.ok,
            collaboration_status=self.collaboration_status,
            consensus_status=self.consensus_status,
            consensus_reached=self.consensus_reached,
            triad_consensus_ready=self.triad_consensus_ready,
            orchestration_mode=self.orchestration_mode,
            participants=self.participants,
            adjudication=self.adjudication,
            argon_review=self.argon_review,
            sonnet_craft=self.sonnet_craft,
            codex_audit=self.codex_audit,
            codex_verdict=self.codex_verdict,
        )


__all__ = [
    "ArgonHandoffRoleResult",
    "ArgonTicketErrorResult",
    "CompactedTestCommandEvidence",
    "EvidenceDimensions",
    "EvidenceScope",
    "InlineScopeDigestPayload",
    "RoleAdjudicationSummary",
    "RoleStatusStub",
    "TaskHashPayload",
    "TriadAdjudication",
    "TriadAggregateSnapshot",
    "TriadCompletedPayload",
    "TriadParticipants",
    "TriadRoleSnapshot",
    "TriadSonnetAbortPayload",
]
