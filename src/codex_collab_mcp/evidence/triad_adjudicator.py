"""Tri-Polar collaboration vs. consensus adjudication, role artifact inspection, and snapshots."""

from __future__ import annotations

import json
import re

from codex_collab_mcp.collab_enums import (
    AuditVerdict,
    CollaborationState,
    ConsensusState,
    EvidenceDimensionState,
    RoleExecutionState,
    TriadRole,
)
from codex_collab_mcp.evidence.evidence_dicts import (
    EvidenceDimensionsDict,
    RoleAdjudicationSummaryDict,
    TriadAdjudicationDict,
)
from codex_collab_mcp.schemas import JsonObject, JsonValue

VALID_ROLE_STATES: set[str] = {s.value for s in RoleExecutionState}
TRIAD_REQUIRED_ROLES: tuple[str, ...] = (
    str(TriadRole.ARGON),
    str(TriadRole.SONNET),
    str(TriadRole.CODEX),
)
TRIAD_VERDICTS: frozenset[str] = frozenset(v.value for v in AuditVerdict)
TRIAD_RESULT_SCHEMA_VERSION: int = 2
_SHA256_DIGEST_PATTERN = re.compile(r"sha256:[0-9a-f]{64}\Z")


class RoleArtifactInspector:
    """Cohesive inspector for role execution states, review narratives, and verdicts."""

    def map_state(self, res: JsonObject | None) -> str:
        """Map an execution response dictionary to a canonical collaboration role state.

        Args:
            res: Role execution response dictionary or None if not invoked.

        Returns:
            Canonical role state string.
        """
        if res is None:
            return str(RoleExecutionState.NOT_STARTED)
        raw_status = res.get("status")
        status = raw_status.strip().lower() if isinstance(raw_status, str) else ""
        if status in {
            str(RoleExecutionState.TIMED_OUT_UNKNOWN),
            "outcome_unknown",
            "unknown",
        }:
            return str(RoleExecutionState.TIMED_OUT_UNKNOWN)
        if status == RoleExecutionState.RUNNING:
            return str(RoleExecutionState.RUNNING)
        if status == RoleExecutionState.QUEUED:
            return str(RoleExecutionState.QUEUED)
        if status in {str(RoleExecutionState.SUCCEEDED), "completed"}:
            return (
                str(RoleExecutionState.SUCCEEDED)
                if res.get("ok") is True
                else str(RoleExecutionState.FAILED)
            )
        if status:
            return str(RoleExecutionState.FAILED)
        return (
            str(RoleExecutionState.SUCCEEDED)
            if res.get("ok") is True
            else str(RoleExecutionState.FAILED)
        )

    def is_successful(self, res: JsonObject | None) -> bool:
        """Return True only for an explicitly successful, completed role response.

        Args:
            res: Role execution response dictionary or None.

        Returns:
            True when `ok` is True and `status` is `succeeded` or `completed`.
        """
        if res is None or res.get("ok") is not True:
            return False
        raw_status = res.get("status")
        return isinstance(raw_status, str) and raw_status.strip().lower() in {
            str(RoleExecutionState.SUCCEEDED),
            "completed",
        }

    def extract_artifact(self, res: JsonObject | None) -> JsonObject | None:
        """Return the first non-empty structured artifact from a successful role response.

        Args:
            res: Role execution response dictionary or None.

        Returns:
            Structured artifact dictionary (`structured_output` or `data`) or None.
        """
        if res is None or not self.is_successful(res):
            return None
        for key in ("structured_output", "data"):
            obj = res.get(key)
            if isinstance(obj, dict) and obj:
                return obj
        return None

    def has_narrative(self, artifact: JsonObject | None) -> bool:
        """Require a human-readable review summary or critique in the selected artifact.

        Args:
            artifact: Structured review artifact dictionary or None.

        Returns:
            True if `summary` or `executive_critique` is a non-empty string.
        """
        if artifact is None:
            return False
        for key in ("summary", "executive_critique"):
            value = artifact.get(key)
            if isinstance(value, str) and value.strip():
                return True
        return False

    def verdict_from_artifact(self, artifact: JsonObject | None) -> str | None:
        """Return only an explicit valid verdict from one selected structured artifact.

        Args:
            artifact: Structured review artifact dictionary or None.

        Returns:
            Normalized verdict (`PASS`, `WARN`, `BLOCK`) or None.
        """
        if artifact is None:
            return None
        v = artifact.get("verdict")
        if isinstance(v, str):
            normalized = v.strip().upper()
            if normalized in TRIAD_VERDICTS:
                return normalized
        return None

    def is_valid_review(self, res: JsonObject | None) -> bool:
        """Require successful completion, a valid verdict, and non-empty review narrative.

        Args:
            res: Role execution response dictionary or None.

        Returns:
            True if the response contains a valid, narrative-backed review artifact.
        """
        artifact = self.extract_artifact(res)
        return (
            artifact is not None
            and self.verdict_from_artifact(artifact) is not None
            and self.has_narrative(artifact)
        )


class TriadConsensusAdjudicator:
    """Cohesive adjudicator separating role execution state from multi-pole consensus."""

    def adjudicate(
        self,
        *,
        required_roles: list[str],
        role_states: dict[str, str],
        role_verdicts: dict[str, str | None],
        role_outputs: dict[str, JsonValue],
        decision_authority: str = "requires_caller_or_argon_adjudication",
    ) -> TriadAdjudicationDict:
        """Separate per-role execution state from multi-role consensus state.

        Args:
            required_roles: Ordered list of required role identifiers.
            role_states: Mapping of role name to execution state.
            role_verdicts: Mapping of role name to extracted verdict or None.
            role_outputs: Mapping of role name to structured output artifact.
            decision_authority: Authority responsible for final adjudication.

        Returns:
            Typed TriadAdjudicationDict detailing collaboration, consensus, and role breakdown.
        """
        normalized_states: dict[str, str] = {}
        for role in required_roles:
            st = role_states.get(role, str(RoleExecutionState.NOT_STARTED)).strip().lower()
            normalized_states[role] = (
                st if st in VALID_ROLE_STATES else str(RoleExecutionState.FAILED)
            )

        succeeded_roles = [
            r for r in required_roles if normalized_states.get(r) == RoleExecutionState.SUCCEEDED
        ]
        incomplete_roles = [
            r for r in required_roles if normalized_states.get(r) != RoleExecutionState.SUCCEEDED
        ]

        if len(succeeded_roles) == len(required_roles) and len(required_roles) > 0:
            collab_status = str(CollaborationState.COMPLETE)
        elif len(succeeded_roles) > 0:
            collab_status = str(CollaborationState.PARTIAL)
        elif any(
            normalized_states.get(r) in (RoleExecutionState.QUEUED, RoleExecutionState.RUNNING)
            for r in required_roles
        ):
            collab_status = "pending"
        elif any(
            normalized_states.get(r) == RoleExecutionState.TIMED_OUT_UNKNOWN for r in required_roles
        ):
            collab_status = str(RoleExecutionState.TIMED_OUT_UNKNOWN)
        else:
            collab_status = str(CollaborationState.FAILED)

        missing_artifact_roles = [
            role
            for role in required_roles
            if not (isinstance(role_outputs.get(role), dict) and bool(role_outputs.get(role)))
        ]
        normalized_verdicts: dict[str, str | None] = {}
        for role in required_roles:
            verdict = role_verdicts.get(role)
            normalized = verdict.strip().upper() if isinstance(verdict, str) else None
            normalized_verdicts[role] = normalized if normalized in TRIAD_VERDICTS else None

        missing_verdict_roles = [
            role for role in required_roles if normalized_verdicts.get(role) is None
        ]
        missing_review_roles = [
            role
            for role in required_roles
            if (
                normalized_states.get(role) != RoleExecutionState.SUCCEEDED
                or role in missing_artifact_roles
                or role in missing_verdict_roles
            )
        ]
        valid_review_roles = [role for role in required_roles if role not in missing_review_roles]

        active_verdicts: dict[str, str] = {}
        for r in valid_review_roles:
            v = normalized_verdicts.get(r)
            if v is not None:
                active_verdicts[r] = v

        verdict_values = set(active_verdicts.values())
        conflict_detected = len(verdict_values) > 1
        conflict_basis: str | None = None
        if conflict_detected:
            conflict_basis = "Role verdicts diverge across completed roles: " + ", ".join(
                f"{r}={v}" for r, v in sorted(active_verdicts.items())
            )

        if collab_status != CollaborationState.COMPLETE or missing_review_roles:
            consensus_status = str(ConsensusState.INCOMPLETE_NO_CONSENSUS)
            consensus_reached = False
        elif conflict_detected:
            consensus_status = str(ConsensusState.CONFLICT_REQUIRES_ADJUDICATION)
            consensus_reached = False
        elif AuditVerdict.BLOCK in verdict_values:
            consensus_status = str(ConsensusState.BLOCKED_BY_RED_TEAM)
            consensus_reached = False
        else:
            consensus_status = str(ConsensusState.CONSENSUS_VERIFIED)
            consensus_reached = True

        review_approved = consensus_status == ConsensusState.CONSENSUS_VERIFIED and all(
            active_verdicts.get(role) == AuditVerdict.PASS for role in required_roles
        )
        roles_detail: dict[str, RoleAdjudicationSummaryDict] = {
            role: RoleAdjudicationSummaryDict(
                state=normalized_states[role],
                verdict=normalized_verdicts.get(role),
                has_artifact=role not in missing_artifact_roles,
                has_valid_review=role not in missing_review_roles,
            )
            for role in required_roles
        }
        return TriadAdjudicationDict(
            collaboration_status=collab_status,
            consensus_status=consensus_status,
            consensus_reached=consensus_reached,
            required_roles=list(required_roles),
            succeeded_roles=list(succeeded_roles),
            incomplete_roles=list(incomplete_roles),
            role_states=roles_detail,
            missing_artifact_roles=list(missing_artifact_roles),
            missing_verdict_roles=list(missing_verdict_roles),
            missing_review_roles=list(missing_review_roles),
            conflict_detected=conflict_detected,
            conflict_basis=conflict_basis,
            decision_authority=decision_authority,
            review_approved=review_approved,
        )

    def recompute(self, snapshot: JsonObject) -> TriadAdjudicationDict | None:
        """Recompute consensus only from the role evidence stored in one aggregate snapshot.

        Args:
            snapshot: Saved triad collaboration snapshot dictionary.

        Returns:
            Recomputed TriadAdjudicationDict if consistent and uncorrupted, else None.
        """
        raw_roles = snapshot.get("roles")
        saved_adjudication = snapshot.get("adjudication")
        if not isinstance(raw_roles, dict) or not isinstance(saved_adjudication, dict):
            return None
        if set(raw_roles) != set(TRIAD_REQUIRED_ROLES):
            return None

        role_states: dict[str, str] = {}
        role_verdicts: dict[str, str | None] = {}
        role_outputs: dict[str, JsonValue] = {}
        for role in TRIAD_REQUIRED_ROLES:
            role_snapshot = raw_roles.get(role)
            if not isinstance(role_snapshot, dict):
                return None
            saved_state = role_snapshot.get("state")
            response_present = role_snapshot.get("response_present")
            response_status = role_snapshot.get("response_status")
            response_ok = role_snapshot.get("response_ok")
            model_id = role_snapshot.get("model_id")
            if (
                not isinstance(saved_state, str)
                or saved_state not in VALID_ROLE_STATES
                or not isinstance(response_present, bool)
                or (response_status is not None and not isinstance(response_status, str))
                or (response_ok is not None and not isinstance(response_ok, bool))
                or (
                    model_id is not None
                    and (
                        not isinstance(model_id, str)
                        or not model_id.strip()
                        or model_id.strip() != model_id
                        or model_id == "codex-default"
                        or not response_present
                    )
                )
            ):
                return None

            if response_present:
                stub_payload: JsonObject = {"status": response_status, "ok": response_ok}
                computed_state = _ROLE_INSPECTOR.map_state(stub_payload)
            else:
                if response_status is not None or response_ok is not None:
                    return None
                computed_state = _ROLE_INSPECTOR.map_state(None)
            if computed_state != saved_state:
                return None
            completed_success = (
                response_ok is True
                and isinstance(response_status, str)
                and response_status in {str(RoleExecutionState.SUCCEEDED), "completed"}
            )

            artifact_present = role_snapshot.get("review_artifact_present")
            artifact_has_narrative = role_snapshot.get("review_artifact_has_narrative")
            artifact_digest = role_snapshot.get("review_artifact_digest")
            if (
                not isinstance(artifact_present, bool)
                or not isinstance(artifact_has_narrative, bool)
                or (artifact_has_narrative and not artifact_present)
            ):
                return None
            if artifact_present:
                if not completed_success or not isinstance(artifact_digest, str):
                    return None
                if _SHA256_DIGEST_PATTERN.fullmatch(artifact_digest) is None:
                    return None
                role_outputs[role] = {"review_artifact_digest": artifact_digest}
            else:
                if artifact_digest is not None:
                    return None
                role_outputs[role] = None

            verdict_value = role_snapshot.get("verdict")
            if verdict_value is not None and (
                not isinstance(verdict_value, str) or verdict_value not in TRIAD_VERDICTS
            ):
                return None
            if verdict_value is not None and (
                not completed_success or not artifact_present or not artifact_has_narrative
            ):
                return None
            role_verdicts[role] = verdict_value
            role_states[role] = computed_state

            generated_tests_present = role_snapshot.get("generated_tests_present")
            if not isinstance(generated_tests_present, bool):
                return None
            if generated_tests_present and not completed_success:
                return None
            expected_dimensions = EvidenceDimensionsDict(
                static_code_review=(
                    "verified"
                    if completed_success
                    and artifact_present
                    and artifact_has_narrative
                    and verdict_value in TRIAD_VERDICTS
                    else str(EvidenceDimensionState.NOT_RUN)
                ),
                test_generation=(
                    "verified" if generated_tests_present else str(EvidenceDimensionState.NOT_RUN)
                ),
                test_execution=str(EvidenceDimensionState.NOT_RUN),
                integration_verification=str(EvidenceDimensionState.NOT_RUN),
                runtime_confirmation=str(EvidenceDimensionState.NOT_RUN),
            )
            saved_dimensions = role_snapshot.get("evidence_dimensions")
            if not isinstance(saved_dimensions, dict) or saved_dimensions != expected_dimensions:
                return None

        authority = saved_adjudication.get("decision_authority")
        if not isinstance(authority, str) or authority not in {
            "gemini-4-argon",
            "pending_argon_or_user_adjudication",
            "aborted_at_sonnet_stage",
        }:
            return None
        recomputed = self.adjudicate(
            required_roles=list(TRIAD_REQUIRED_ROLES),
            role_states=role_states,
            role_verdicts=role_verdicts,
            role_outputs=role_outputs,
            decision_authority=authority,
        )
        if json.dumps(saved_adjudication, sort_keys=True) != json.dumps(recomputed, sort_keys=True):
            return None
        return recomputed


_ROLE_INSPECTOR = RoleArtifactInspector()
_CONSENSUS_ADJUDICATOR = TriadConsensusAdjudicator()

is_successful_role_response = _ROLE_INSPECTOR.is_successful
structured_review_artifact = _ROLE_INSPECTOR.extract_artifact
has_review_narrative = _ROLE_INSPECTOR.has_narrative
verdict_from_artifact = _ROLE_INSPECTOR.verdict_from_artifact
map_response_to_role_state = _ROLE_INSPECTOR.map_state
is_valid_review_response = _ROLE_INSPECTOR.is_valid_review
adjudicate_collaboration_and_consensus = _CONSENSUS_ADJUDICATOR.adjudicate
recompute_saved_triad_adjudication = _CONSENSUS_ADJUDICATOR.recompute


def extract_verdict(res: JsonObject | None) -> str | None:
    """Extract a `PASS | WARN | BLOCK` verdict from a successful role artifact.

    Args:
        res: Role execution response dictionary or None.

    Returns:
        Normalized verdict string if valid and narrative-backed, else None.
    """
    artifact = _ROLE_INSPECTOR.extract_artifact(res)
    if not _ROLE_INSPECTOR.has_narrative(artifact):
        return None
    return _ROLE_INSPECTOR.verdict_from_artifact(artifact)
