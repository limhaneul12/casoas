"""Alexandria decision pack synthesis and scoped triad aggregate validation service."""

from __future__ import annotations

import json
import re

from codex_collab_mcp.collab_enums import (
    ConsensusState,
    JobStatusKind,
    OrchestrationMode,
    OutcomeStateKind,
)
from codex_collab_mcp.evidence.evidence_dicts import TriadAdjudicationDict
from codex_collab_mcp.evidence.scope_builder import canonical_sha256, resolve_workspace_dir
from codex_collab_mcp.evidence.triad_adjudicator import (
    TRIAD_REQUIRED_ROLES,
    TRIAD_RESULT_SCHEMA_VERSION,
    recompute_saved_triad_adjudication,
)
from codex_collab_mcp.job_store import CodexJobRecord, JobStore
from codex_collab_mcp.orchestration.service_dicts import (
    DecisionAuthoritativeEvidenceDict,
    DecisionCallerContentDict,
    DecisionDigestPayloadDict,
    DecisionPackRequest,
    DecisionPackResponseDict,
)
from codex_collab_mcp.schemas import JsonObject

_COLLABORATION_ID_PATTERN = re.compile(r"cdx-[0-9a-f]{10}\Z")
_SHA256_PATTERN = re.compile(r"sha256:[0-9a-f]{64}\Z")

type DecisionFencesTuple = tuple[str | None, str | None, str | None]


class AlexandriaDecisionPackService:
    """Validate persisted triad aggregates and package Alexandria decision bundles."""

    def load_scoped_triad_aggregate(
        self,
        fences: DecisionFencesTuple,
        job_store: JobStore,
    ) -> tuple[JsonObject, TriadAdjudicationDict] | None:
        """Load one completed triad aggregate after validating its ID and scope fence.

        Args:
            fences: Tuple of `(collaboration_id, expected_workspace, expected_scope_digest)`.
            job_store: Active JobStore instance used to read cached and persisted records.

        Returns:
            Tuple of `(snapshot_dict, adjudication_dict)` when verified, else None.
        """
        collaboration_id, expected_workspace, expected_scope_digest = fences
        if (
            collaboration_id is None
            or _COLLABORATION_ID_PATTERN.fullmatch(collaboration_id) is None
            or expected_workspace is None
            or expected_scope_digest is None
            or _SHA256_PATTERN.fullmatch(expected_scope_digest) is None
        ):
            return None

        cached_record = job_store.get_job(collaboration_id)
        record = job_store.get_persisted_job(collaboration_id)
        if cached_record is None or record is None or cached_record != record:
            return None
        if (
            record.job_id != collaboration_id
            or record.task_type != "triad_orchestration"
            or record.status != JobStatusKind.COMPLETED
            or record.outcome_state != OutcomeStateKind.CONFIRMED_SUCCESS
            or record.completed_at is None
            or record.cancel_requested
            or record.metadata.get("schema_version") != str(TRIAD_RESULT_SCHEMA_VERSION)
        ):
            return None
        if (
            record.workspace_dir != expected_workspace
            and resolve_workspace_dir(record.workspace_dir) != expected_workspace
        ):
            return None

        return self._validate_persisted_snapshot(
            record=record,
            collaboration_id=collaboration_id,
            expected_workspace=expected_workspace,
            expected_scope_digest=expected_scope_digest,
        )

    def _validate_persisted_snapshot(
        self,
        *,
        record: CodexJobRecord,
        collaboration_id: str,
        expected_workspace: str,
        expected_scope_digest: str,
    ) -> tuple[JsonObject, TriadAdjudicationDict] | None:
        """Verify cryptographic digests, scope fences, and recomputed adjudication.

        Args:
            record: Persisted CodexJobRecord loaded from disk.
            collaboration_id: Validated collaboration identifier.
            expected_workspace: Canonicalized expected workspace directory path.
            expected_scope_digest: Validated expected SHA-256 scope digest.

        Returns:
            Tuple of `(snapshot_dict, adjudication_dict)` when valid, else None.
        """
        result = record.result
        if not isinstance(result, dict):
            return None
        stored_snapshot_digest = result.get("snapshot_digest")
        if not isinstance(stored_snapshot_digest, str):
            return None
        unsigned_snapshot: JsonObject = {
            key: value for key, value in result.items() if key != "snapshot_digest"
        }
        if (
            _SHA256_PATTERN.fullmatch(stored_snapshot_digest) is None
            or canonical_sha256(unsigned_snapshot) != stored_snapshot_digest
        ):
            return None
        if (
            result.get("schema_version") != TRIAD_RESULT_SCHEMA_VERSION
            or result.get("task_type") != "triad_orchestration"
            or result.get("collaboration_id") != collaboration_id
            or result.get("workspace_dir") != expected_workspace
        ):
            return None
        source_task_digest = result.get("source_task_digest")
        if (
            not isinstance(source_task_digest, str)
            or _SHA256_PATTERN.fullmatch(source_task_digest) is None
            or record.metadata.get("source_task_digest") != source_task_digest
        ):
            return None
        orchestration_mode = result.get("orchestration_mode")
        if not isinstance(orchestration_mode, str) or orchestration_mode not in {
            OrchestrationMode.COUNCIL,
            OrchestrationMode.PIPELINE,
        }:
            return None

        evidence_scope = result.get("evidence_scope")
        if not isinstance(evidence_scope, dict):
            return None
        stored_scope_digest = evidence_scope.get("scope_digest")
        if (
            not isinstance(stored_scope_digest, str)
            or _SHA256_PATTERN.fullmatch(stored_scope_digest) is None
            or stored_scope_digest != expected_scope_digest
        ):
            return None
        codegraph = evidence_scope.get("codegraph")
        if isinstance(codegraph, dict) and codegraph.get("workspace_dir") != expected_workspace:
            return None

        adjudication = recompute_saved_triad_adjudication(result)
        if adjudication is None:
            return None
        return result, adjudication

    def _reconcile_live_role_evidence(
        self,
        source_roles: JsonObject | None,
    ) -> tuple[JsonObject, list[str]]:
        """Extract per-role evidence dimensions and markdown summary lines from a snapshot.

        Args:
            source_roles: Validated `roles` dictionary from a triad snapshot, or None.

        Returns:
            Tuple of `(role_dimensions, role_markdown_lines)`.
        """
        role_dimensions: JsonObject = {}
        role_lines: list[str] = []
        if source_roles is not None:
            for role in TRIAD_REQUIRED_ROLES:
                role_snapshot = source_roles.get(role)
                if isinstance(role_snapshot, dict):
                    dimensions = role_snapshot.get("evidence_dimensions")
                    if isinstance(dimensions, dict):
                        role_dimensions[role] = dimensions
                    state = role_snapshot.get("state")
                    verdict = role_snapshot.get("verdict")
                    artifact_digest = role_snapshot.get("review_artifact_digest")
                    role_lines.append(
                        f"- `{role}`: state `{state}`, verdict `{verdict}`, "
                        f"review artifact digest `{artifact_digest}`"
                    )
        return role_dimensions, role_lines

    def _render_decision_markdown(
        self,
        *,
        title_prefix: str,
        topic_title: str,
        project_scope: str,
        evidence_status: str,
        collaboration_markdown: str,
        task_digest_markdown: str,
        workspace_markdown: str,
        scope_digest_markdown: str,
        collaboration_status: str,
        consensus_status: str,
        consensus_reached: bool,
        review_approved: bool,
        idempotency_key: str,
        source_roles_markdown: str,
        argon_architecture_intent: str,
        sonnet_craft_or_review_summary: str,
        codex_audit_or_synthesis_summary: str,
        final_resolution: str,
        verification_evidence: str,
    ) -> str:
        """Render the deterministic Markdown body for an Alexandria decision bundle.

        Args:
            title_prefix: Header tag (`Heterarchy Consensus` or `Heterarchy Decision`).
            topic_title: Short descriptive title of the decision topic.
            project_scope: Target Alexandria project scope.
            evidence_status: Verification state (`verified` or `unverified`).
            collaboration_markdown: Collaboration ID or `unverified`.
            task_digest_markdown: Task SHA-256 digest or `unverified`.
            workspace_markdown: Canonical workspace directory or `unverified`.
            scope_digest_markdown: Evidence scope SHA-256 digest or `unverified`.
            collaboration_status: Collaboration status string.
            consensus_status: Consensus status string.
            consensus_reached: Whether 3-pole consensus was verified.
            review_approved: Whether all 3 roles approved the review.
            idempotency_key: Deterministic Alexandria upsert idempotency key.
            source_roles_markdown: Formatted bullet lines of same-run role evidence.
            argon_architecture_intent: Caller-supplied Argon architectural notes.
            sonnet_craft_or_review_summary: Caller-supplied Claude craft/review notes.
            codex_audit_or_synthesis_summary: Caller-supplied Codex audit/synthesis notes.
            final_resolution: Caller-supplied final trade-off resolution.
            verification_evidence: Caller-supplied verification notes.

        Returns:
            Formatted Markdown string ready for Alexandria persistence.
        """
        return f"""# [{title_prefix}] {topic_title}

- **Project Scope**: `{project_scope}`
- **Evidence Status**: `{evidence_status}`
- **Source Collaboration ID**: `{collaboration_markdown}`
- **Source Task SHA-256**: `{task_digest_markdown}`
- **Source Workspace**: `{workspace_markdown}`
- **Source Evidence Scope Digest**: `{scope_digest_markdown}`
- **Collaboration Status**: `{collaboration_status}`
- **Consensus Status**: `{consensus_status}`
- **Consensus Reached**: `{str(consensus_reached).lower()}`
- **Review Approved**: `{str(review_approved).lower()}`
- **Resolution Verified**: `false`
- **Idempotency Key**: `{idempotency_key}`

## Source Role Evidence
{source_roles_markdown}

## Caller-Provided Review Notes
The following prose was supplied by the caller and was not independently matched to the
stored role artifacts. The cited run verifies review status only.

### Caller-Provided Argon Architecture Notes
{argon_architecture_intent}

### Caller-Provided Claude Craft or Review Notes
{sonnet_craft_or_review_summary or "No notes supplied."}

### Caller-Provided Codex Audit or Synthesis Notes
{codex_audit_or_synthesis_summary}

## Resolution & Trade-off Decision
Caller-provided resolution; it has not been verified against the cited run.

{final_resolution}

## Caller-Provided Verification Notes
{verification_evidence or "No verification notes supplied."}
"""

    def build_decision_pack(
        self,
        req: DecisionPackRequest,
        job_store: JobStore,
    ) -> str:
        """Package a Tri-Polar collaboration outcome into a deterministic Alexandria bundle.

        Args:
            req: Validated request bundle containing caller notes and scope fences.
            job_store: Active JobStore instance.

        Returns:
            Pretty-printed JSON string containing the Alexandria decision bundle.
        """
        expected_ws = (
            resolve_workspace_dir(req.workspace_dir) if req.workspace_dir is not None else None
        )

        validated_source = self.load_scoped_triad_aggregate(
            (req.collaboration_id, expected_ws, req.expected_scope_digest), job_store
        )
        source_snapshot: JsonObject | None = None
        adjudication: TriadAdjudicationDict | None = None
        if validated_source is not None:
            source_snapshot, adjudication = validated_source

        source_evidence_scope_value = (
            source_snapshot.get("evidence_scope") if source_snapshot is not None else None
        )
        source_evidence_scope = (
            source_evidence_scope_value if isinstance(source_evidence_scope_value, dict) else None
        )
        source_roles_value = source_snapshot.get("roles") if source_snapshot is not None else None
        source_roles = source_roles_value if isinstance(source_roles_value, dict) else None
        source_task_digest_value = (
            source_snapshot.get("source_task_digest") if source_snapshot is not None else None
        )
        source_task_digest = (
            source_task_digest_value if isinstance(source_task_digest_value, str) else None
        )
        source_workspace_value = (
            source_snapshot.get("workspace_dir") if source_snapshot is not None else None
        )
        source_workspace = (
            source_workspace_value if isinstance(source_workspace_value, str) else None
        )
        source_scope_value = (
            source_evidence_scope.get("scope_digest") if source_evidence_scope is not None else None
        )
        source_scope_digest = source_scope_value if isinstance(source_scope_value, str) else None
        snapshot_digest_value = (
            source_snapshot.get("snapshot_digest") if source_snapshot is not None else None
        )
        snapshot_digest = snapshot_digest_value if isinstance(snapshot_digest_value, str) else None
        evidence_status = "verified" if source_snapshot is not None else "unverified"
        collaboration_status = (
            adjudication["collaboration_status"] if adjudication is not None else "unverified"
        )
        consensus_status = (
            adjudication["consensus_status"] if adjudication is not None else "unverified"
        )
        consensus_reached = adjudication is not None and adjudication["consensus_reached"]
        review_approved = adjudication is not None and adjudication["review_approved"]
        resolution_verified = False

        role_dimensions, role_lines = self._reconcile_live_role_evidence(source_roles)

        caller_content = DecisionCallerContentDict(
            project_scope=req.project_scope,
            topic_title=req.topic_title,
            argon_architecture_intent=req.argon_architecture_intent,
            sonnet_craft_or_review_summary=req.sonnet_craft_or_review_summary,
            codex_audit_or_synthesis_summary=req.codex_audit_or_synthesis_summary,
            final_resolution=req.final_resolution,
            verification_evidence=req.verification_evidence,
            requested_collaboration_id=req.collaboration_id,
            requested_workspace_dir=req.workspace_dir,
            expected_scope_digest=req.expected_scope_digest,
        )
        collab_val = source_snapshot.get("collaboration_id") if source_snapshot else None
        authoritative_evidence = DecisionAuthoritativeEvidenceDict(
            evidence_status=evidence_status,
            collaboration_id=collab_val,
            source_task_digest=source_task_digest,
            workspace_dir=source_workspace,
            scope_digest=source_scope_digest,
            snapshot_digest=snapshot_digest,
            collaboration_status=collaboration_status,
            consensus_status=consensus_status,
            consensus_reached=consensus_reached,
            review_approved=review_approved,
            resolution_verified=resolution_verified,
        )
        digest_payload = DecisionDigestPayloadDict(
            caller_content=caller_content,
            authoritative_evidence=authoritative_evidence,
        )
        digest = canonical_sha256(digest_payload).removeprefix("sha256:")[:16]
        idempotency_key = f"heterarchy-argon-codex-{digest}"
        title_prefix = (
            "Heterarchy Consensus"
            if consensus_status == ConsensusState.CONSENSUS_VERIFIED and consensus_reached
            else "Heterarchy Decision"
        )
        source_roles_markdown = "\n".join(role_lines) or "- No validated same-run role evidence."
        scope_digest_markdown = source_scope_digest or "unverified"
        task_digest_markdown = source_task_digest or "unverified"
        workspace_markdown = source_workspace or "unverified"
        collaboration_markdown = str(collab_val) if source_snapshot is not None else "unverified"

        markdown_body = self._render_decision_markdown(
            title_prefix=title_prefix,
            topic_title=req.topic_title,
            project_scope=req.project_scope,
            evidence_status=evidence_status,
            collaboration_markdown=collaboration_markdown,
            task_digest_markdown=task_digest_markdown,
            workspace_markdown=workspace_markdown,
            scope_digest_markdown=scope_digest_markdown,
            collaboration_status=collaboration_status,
            consensus_status=consensus_status,
            consensus_reached=consensus_reached,
            review_approved=review_approved,
            idempotency_key=idempotency_key,
            source_roles_markdown=source_roles_markdown,
            argon_architecture_intent=req.argon_architecture_intent,
            sonnet_craft_or_review_summary=req.sonnet_craft_or_review_summary,
            codex_audit_or_synthesis_summary=req.codex_audit_or_synthesis_summary,
            final_resolution=req.final_resolution,
            verification_evidence=req.verification_evidence,
        )
        next_step = (
            "Call `alexandria_verified_upsert` with the generated `idempotency_key` "
            "and `markdown_content` to persist this decision record into Heterarchy Alexandria."
        )
        response = DecisionPackResponseDict(
            ok=True,
            project_scope=req.project_scope,
            requested_collaboration_id=req.collaboration_id,
            collaboration_id=collab_val,
            evidence_status=evidence_status,
            source_task_digest=source_task_digest,
            workspace_dir=source_workspace,
            expected_workspace_dir=expected_ws,
            expected_scope_digest=req.expected_scope_digest,
            evidence_scope=source_evidence_scope,
            role_evidence_dimensions=role_dimensions,
            collaboration_status=collaboration_status,
            consensus_status=consensus_status,
            consensus_reached=consensus_reached,
            review_approved=review_approved,
            resolution_verified=resolution_verified,
            idempotency_key=idempotency_key,
            logical_identity=f"{req.project_scope}/decisions/{digest}",
            markdown_content=markdown_body,
            recommended_next_step=next_step,
        )
        return json.dumps(response, ensure_ascii=False, indent=2)
