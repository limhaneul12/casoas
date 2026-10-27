"""Argon role resolution and triad aggregate evidence persistence helpers."""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Coroutine

from codex_collab_mcp.collab_enums import (
    EvidenceDimensionState,
    JobStatusKind,
    OrchestrationMode,
    OutcomeStateKind,
    RoleExecutionState,
    TriadRole,
)
from codex_collab_mcp.evidence.evidence_dicts import (
    EvidenceDimensionsDict,
    TriadAdjudicationDict,
)
from codex_collab_mcp.evidence.scope_builder import (
    attach_evidence_scope,
    compact_evidence_scope,
    prepare_triad_role_evidence,
)
from codex_collab_mcp.evidence.triad_adjudicator import (
    TRIAD_REQUIRED_ROLES,
    TRIAD_RESULT_SCHEMA_VERSION,
)
from codex_collab_mcp.evidence.workspace_digests import (
    canonical_sha256,
    resolve_workspace_dir,
)
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.routing.auth_redaction import redact_sensitive_text
from codex_collab_mcp.schemas import JsonObject, as_json_object

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]

_COLLABORATION_ID_PATTERN = re.compile(r"cdx-[0-9a-f]{10}\Z")


class TriadRoleResolver:
    """Resolve Gemini 4 Argon role execution and persist triad evidence snapshots."""

    def attach_triad_evidence_and_persist(
        self,
        payload: JsonObject,
        *,
        workspace_dir: str,
        provided_context_or_code: str,
        files_inspected: list[str] | None,
        symbols_inspected: list[str] | None,
        executed_test_commands: list[JsonObject] | None,
        bridge_executed_gates: list[JsonObject] | None = None,
        orchestration_mode: str,
        source_task_digest: str,
        role_snapshots: dict[str, JsonObject],
        role_evidence_dimensions: dict[str, EvidenceDimensionsDict],
        adjudication: TriadAdjudicationDict,
        job_store: JobStore,
        started_at: float | None = None,
    ) -> None:
        """Attach aggregate review evidence and persist one compact same-run snapshot.

        Args:
            payload: Mutable orchestration response payload to enrich in place.
            workspace_dir: Target workspace directory path.
            provided_context_or_code: Inline source context or crafted code under review.
            files_inspected: Optional relative file paths inspected for evidence scope.
            symbols_inspected: Optional symbol names inspected for evidence scope.
            executed_test_commands: Optional caller-reported test command records.
            bridge_executed_gates: Optional bridge-executed verification gate records.
            orchestration_mode: Normalized mode (`pipeline` or `council`).
            source_task_digest: SHA-256 digest of the canonical task inputs.
            role_snapshots: Compact per-role execution snapshots.
            role_evidence_dimensions: Per-role static review and test generation dimensions.
            adjudication: Typed TriadAdjudicationDict summary.
            job_store: Active JobStore instance used to persist the triad aggregate.
            started_at: Optional orchestration start timestamp for accurate duration tracking.
        """
        resolved_workspace = resolve_workspace_dir(workspace_dir)
        if resolved_workspace is None:
            raise ValueError("A valid workspace directory is required for triad evidence.")

        attach_evidence_scope(
            payload,
            workspace_dir=resolved_workspace,
            provided_context_or_code=provided_context_or_code,
            files_inspected=files_inspected,
            symbols_inspected=symbols_inspected,
            executed_test_commands=executed_test_commands,
            bridge_executed_gates=bridge_executed_gates,
        )
        not_run = str(EvidenceDimensionState.NOT_RUN)
        successful_reviews = sum(
            1
            for role in TRIAD_REQUIRED_ROLES
            if role_evidence_dimensions[role]["static_code_review"] == "verified"
        )
        aggregate_review = (
            "verified"
            if successful_reviews == len(TRIAD_REQUIRED_ROLES)
            else "partial"
            if successful_reviews > 0
            else not_run
        )
        aggregate_test_generation = (
            "verified"
            if any(
                role_evidence_dimensions[role]["test_generation"] == "verified"
                for role in TRIAD_REQUIRED_ROLES
            )
            else not_run
        )
        evidence_scope_value = payload.get("evidence_scope")
        if not isinstance(evidence_scope_value, dict):
            raise RuntimeError("Triad evidence scope was not built.")
        evidence_dimensions = evidence_scope_value.get("evidence_dimensions")
        if not isinstance(evidence_dimensions, dict):
            raise RuntimeError("Triad evidence dimensions were not built.")
        evidence_dimensions["static_code_review"] = aggregate_review
        evidence_dimensions["test_generation"] = aggregate_test_generation
        evidence_dimensions["runtime_confirmation"] = not_run

        payload["role_evidence_dimensions"] = {
            role: as_json_object(role_evidence_dimensions[role]) for role in TRIAD_REQUIRED_ROLES
        }
        payload["review_approved"] = adjudication["review_approved"] is True
        payload["source_task_digest"] = source_task_digest
        payload["workspace_dir"] = resolved_workspace

        compact_scope = compact_evidence_scope(evidence_scope_value)
        role_snapshots_json: JsonObject = {
            role: role_snapshots[role] for role in TRIAD_REQUIRED_ROLES
        }
        model_parts = [
            m
            for role in TRIAD_REQUIRED_ROLES
            if isinstance(m := role_snapshots[role].get("model_id"), str) and m
        ]
        triad_model = "+".join(model_parts) if model_parts else None
        try:
            aggregate = job_store.create_job(
                task_type="triad_orchestration",
                workspace_dir=resolved_workspace,
                model=triad_model,
                provider="heterarchy_triad",
                engine=orchestration_mode,
                created_at=started_at,
                metadata={
                    "schema_version": str(TRIAD_RESULT_SCHEMA_VERSION),
                    "source_task_digest": source_task_digest,
                },
            )
        except OSError as exc:
            payload["collaboration_persistence"] = "failed"
            payload["collaboration_persistence_error"] = redact_sensitive_text(str(exc))
            return

        collaboration_id = aggregate.job_id
        if _COLLABORATION_ID_PATTERN.fullmatch(collaboration_id) is None:
            raise RuntimeError("JobStore returned an invalid triad collaboration identifier.")

        snapshot: JsonObject = {
            "schema_version": TRIAD_RESULT_SCHEMA_VERSION,
            "task_type": "triad_orchestration",
            "collaboration_id": collaboration_id,
            "orchestration_mode": orchestration_mode,
            "source_task_digest": source_task_digest,
            "workspace_dir": resolved_workspace,
            "evidence_scope": compact_scope,
            "roles": role_snapshots_json,
            "adjudication": as_json_object(adjudication),
        }
        snapshot["snapshot_digest"] = canonical_sha256(snapshot)
        try:
            completed = job_store.complete_job(aggregate.job_id, result=snapshot)
        except OSError as exc:
            payload["collaboration_persistence"] = "failed"
            payload["unconfirmed_collaboration_id"] = collaboration_id
            payload["collaboration_persistence_error"] = redact_sensitive_text(str(exc))
            return
        if completed is None or completed.status != JobStatusKind.COMPLETED:
            raise RuntimeError("Triad aggregate was not completed in JobStore.")
        payload["collaboration_id"] = collaboration_id
        payload["collaboration_persistence"] = "recorded"

    async def resolve_argon_role(
        self,
        *,
        argon_model: str,
        argon_prompt: str,
        extra_context: str,
        argon_handoff_ticket_id: str | None,
        invoke_argon_rpc: bool,
        timeout_seconds: float,
        max_input_bytes: int,
        workspace_dir: str,
        job_store: JobStore,
        handoff_store: HandoffStore,
        run_agy_fn: AsyncEngineCallable,
    ) -> JsonObject | None:
        """Resolve Gemini 4 Argon's role response via handoff ticket or local RPC.

        Args:
            argon_model: Requested Gemini 4 Argon model identifier.
            argon_prompt: Base architectural adjudication prompt for Argon.
            extra_context: Additional downstream role context to append when present.
            argon_handoff_ticket_id: Optional handoff ticket ID representing Argon's role.
            invoke_argon_rpc: Whether to invoke Argon via local Antigravity RPC.
            timeout_seconds: RPC execution timeout in seconds.
            max_input_bytes: Maximum UTF-8 prompt byte budget.
            workspace_dir: Resolved workspace directory path.
            job_store: Active JobStore instance.
            handoff_store: Active HandoffStore instance.
            run_agy_fn: Antigravity local RPC prompt execution callable.

        Returns:
            Argon role response dictionary, or None when Argon is not started.
        """
        if argon_handoff_ticket_id and argon_handoff_ticket_id.strip():
            ticket = handoff_store.get_ticket(argon_handoff_ticket_id.strip())
            if ticket is None:
                return {
                    "ok": False,
                    "status": str(JobStatusKind.FAILED),
                    "error": f"Argon handoff ticket '{argon_handoff_ticket_id}' not found.",
                }
            t_dict = ticket.to_dict()
            role_state = t_dict["collaboration_role_state"] or str(RoleExecutionState.QUEUED)
            is_done = t_dict["execution_completed"] is True
            return {
                "ok": is_done,
                "status": str(JobStatusKind.SUCCEEDED) if is_done else role_state,
                "outcome_state": (
                    str(OutcomeStateKind.CONFIRMED_SUCCESS)
                    if is_done
                    else str(OutcomeStateKind.IN_FLIGHT)
                ),
                "handoff_ticket": as_json_object(t_dict),
            }
        if not invoke_argon_rpc:
            return None
        effective_argon_prompt = (
            f"{argon_prompt}\n\n{extra_context}" if extra_context else argon_prompt
        )
        return await run_agy_fn(
            prompt=effective_argon_prompt,
            model=argon_model,
            expect_json=True,
            timeout_seconds=timeout_seconds,
            max_input_bytes=max_input_bytes,
            workspace_dir=workspace_dir,
            job_store=job_store,
        )

    def build_sonnet_abort_payload(
        self,
        sonnet_res: JsonObject,
        *,
        workspace_dir: str,
        provided_context_or_code: str,
        files_inspected: list[str] | None,
        symbols_inspected: list[str] | None,
        executed_test_commands: list[JsonObject] | None,
        source_task_digest: str,
        job_store: JobStore,
        started_at: float | None = None,
    ) -> str:
        """Build, persist, and serialize an aborted pipeline payload when Sonnet fails.

        Args:
            sonnet_res: Non-succeeded Claude Sonnet/Opus role response dictionary.
            workspace_dir: Target workspace directory path.
            provided_context_or_code: Inline source context or task specification.
            files_inspected: Optional relative file paths inspected for evidence scope.
            symbols_inspected: Optional symbol names inspected for evidence scope.
            executed_test_commands: Optional caller-reported test command records.
            source_task_digest: SHA-256 digest of the canonical task inputs.
            job_store: Active JobStore instance used to persist the triad aggregate.
            started_at: Optional orchestration start timestamp for accurate duration tracking.

        Returns:
            Pretty-printed JSON string representing the aborted pipeline outcome.
        """
        adjudication, role_snapshots, role_evidence_dimensions = prepare_triad_role_evidence(
            {TriadRole.ARGON: None, TriadRole.SONNET: sonnet_res, TriadRole.CODEX: None},
            decision_authority="aborted_at_sonnet_stage",
        )
        fail_payload: JsonObject = {
            "ok": False,
            "collaboration_status": adjudication["collaboration_status"],
            "consensus_status": adjudication["consensus_status"],
            "consensus_reached": False,
            "triad_consensus_ready": False,
            "orchestration_mode": OrchestrationMode.PIPELINE,
            "failed_stage": "sonnet_craft",
            "adjudication": as_json_object(adjudication),
            "sonnet_craft": sonnet_res,
        }
        self.attach_triad_evidence_and_persist(
            fail_payload,
            workspace_dir=workspace_dir,
            provided_context_or_code=provided_context_or_code,
            files_inspected=files_inspected,
            symbols_inspected=symbols_inspected,
            executed_test_commands=executed_test_commands,
            orchestration_mode=OrchestrationMode.PIPELINE,
            source_task_digest=source_task_digest,
            role_snapshots=role_snapshots,
            role_evidence_dimensions=role_evidence_dimensions,
            adjudication=adjudication,
            job_store=job_store,
            started_at=started_at,
        )
        return json.dumps(fail_payload, ensure_ascii=False, indent=2)
