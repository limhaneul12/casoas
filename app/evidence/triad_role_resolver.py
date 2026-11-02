"""Argon role resolution and triad aggregate evidence persistence helpers."""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Coroutine
from typing import TYPE_CHECKING

from app.clients.agy_rpc_transport import AntigravityModelCatalog
from app.core.collab_enums import (
    AuditVerdict,
    CanonicalModelId,
    CollaborationState,
    EvidenceDimensionState,
    JobStatusKind,
    OrchestrationMode,
    OutcomeStateKind,
    RoleExecutionState,
    SchemaName,
    TriadRole,
    TriadStageKind,
)
from app.core.model_contract_enums import (
    CollaborationPersistenceState,
    DecisionAuthorityKind,
    TaskTypeKind,
)
from app.evidence.scope_builder import EvidenceScopeBuilder
from app.evidence.triad_adjudicator import (
    TRIAD_REQUIRED_ROLES,
    TRIAD_RESULT_SCHEMA_VERSION,
)
from app.evidence.workspace_digests import WorkspaceDigestCalculator
from app.routing.auth_redaction import redact_sensitive_text
from app.schemas.boundary_schemas import (
    ArgonReviewOutput,
    JsonObject,
    as_json_object,
    validate_structured_output,
)
from app.schemas.inner_dto.dto_dict_types.evidence_dict_types import (
    EvidenceDimensionsDict,
    TriadAdjudicationDict,
)
from app.schemas.inner_dto.evidence_dtos import (
    ArgonHandoffRoleResult,
    ArgonTicketErrorResult,
    TriadAggregateSnapshot,
    TriadCompletedPayload,
    TriadParticipants,
    TriadSonnetAbortPayload,
)

if TYPE_CHECKING:
    from app.repositories.handoff_store import HandoffStore
    from app.repositories.job_store import JobStore

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]

_COLLABORATION_ID_PATTERN = re.compile(pattern=r"cdx-[0-9a-f]{10}\Z")


class TriadRoleResolver:
    """Resolve Gemini 4 Argon role execution and persist triad evidence snapshots."""

    def attach_triad_evidence_and_persist(
        self,
        payload: JsonObject,
        workspace_dir: str,
        provided_context_or_code: str,
        files_inspected: list[str] | None,
        symbols_inspected: list[str] | None,
        executed_test_commands: list[JsonObject] | None,
        orchestration_mode: str,
        source_task_digest: str,
        role_snapshots: dict[str, JsonObject],
        role_evidence_dimensions: dict[str, EvidenceDimensionsDict],
        adjudication: TriadAdjudicationDict,
        job_store: JobStore,
        bridge_executed_gates: list[JsonObject] | None = None,
        started_at: float | None = None,
        collaboration_id_override: str | None = None,
        request_id: str | None = None,
        idempotency_key: str | None = None,
    ) -> None:
        """Attach aggregate review evidence and persist one compact same-run snapshot.

        Args:
            payload: Mutable orchestration response payload to enrich in place.
            workspace_dir: Target workspace directory path.
            provided_context_or_code: Inline source context or crafted code under review.
            files_inspected: Optional relative file paths inspected for evidence scope.
            symbols_inspected: Optional symbol names inspected for evidence scope.
            executed_test_commands: Optional caller-reported test command records.
            orchestration_mode: Normalized mode (`pipeline` or `council`).
            source_task_digest: SHA-256 digest of the canonical task inputs.
            role_snapshots: Compact per-role execution snapshots.
            role_evidence_dimensions: Per-role static review and test generation dimensions.
            adjudication: Typed TriadAdjudicationDict summary.
            job_store: Active JobStore instance used to persist the triad aggregate.
            bridge_executed_gates: Optional bridge-executed verification gate records.
            started_at: Optional orchestration start timestamp for accurate duration tracking.
            collaboration_id_override: Optional pre-registered background job ID (`cdx-...`).
            request_id: Optional caller request ID (`cdx-...`) for the aggregate job.
            idempotency_key: Optional caller idempotency key for the aggregate job.
        """
        digest_calc = WorkspaceDigestCalculator()
        scope_builder = EvidenceScopeBuilder()
        resolved_workspace = digest_calc.resolve_dir(workspace_dir=workspace_dir)
        if resolved_workspace is None:
            raise ValueError("A valid workspace directory is required for triad evidence.")

        evidence_scope_value = scope_builder.attach_scope(
            result=payload,
            workspace_dir=resolved_workspace,
            provided_context_or_code=provided_context_or_code,
            files_inspected=files_inspected,
            symbols_inspected=symbols_inspected,
            executed_test_commands=executed_test_commands,
            bridge_executed_gates=bridge_executed_gates,
        )
        not_run, verified = (
            str(EvidenceDimensionState.NOT_RUN),
            str(EvidenceDimensionState.VERIFIED),
        )
        successful_reviews = sum(1 for r in TRIAD_REQUIRED_ROLES if role_evidence_dimensions[r]["static_code_review"] == verified)
        aggregate_review = verified if successful_reviews == len(TRIAD_REQUIRED_ROLES) else str(EvidenceDimensionState.PARTIAL) if successful_reviews > 0 else not_run
        has_test_gen = any(role_evidence_dimensions[r]["test_generation"] == verified for r in TRIAD_REQUIRED_ROLES)
        evidence_dimensions = evidence_scope_value["evidence_dimensions"]
        evidence_dimensions["static_code_review"] = aggregate_review
        evidence_dimensions["test_generation"] = verified if has_test_gen else not_run
        evidence_dimensions["runtime_confirmation"] = not_run

        payload["role_evidence_dimensions"] = {r: as_json_object(record=role_evidence_dimensions[r]) for r in TRIAD_REQUIRED_ROLES}
        payload["review_approved"] = adjudication["review_approved"] is True
        payload["source_task_digest"] = source_task_digest
        payload["workspace_dir"] = resolved_workspace

        compact_scope = scope_builder.compact_scope(evidence_scope=evidence_scope_value)
        role_snapshots_json: JsonObject = {r: role_snapshots[r] for r in TRIAD_REQUIRED_ROLES}
        model_parts = [m for r in TRIAD_REQUIRED_ROLES if isinstance(m := role_snapshots[r].get("model_id"), str) and m]
        triad_model = "+".join(model_parts) if model_parts else None
        meta_fields: dict[str, str] = dict(
            (
                ("schema_version", str(TRIAD_RESULT_SCHEMA_VERSION)),
                ("source_task_digest", source_task_digest),
                ("current_stage", str(TriadStageKind.COMPLETED)),
            )
        )
        try:
            if collaboration_id_override and _COLLABORATION_ID_PATTERN.fullmatch(collaboration_id_override) is not None and (existing_agg := job_store.get_persisted_job(job_id=collaboration_id_override) or job_store.get_job(job_id=collaboration_id_override)):
                if existing_agg.status == JobStatusKind.CANCELLED or existing_agg.cancel_requested:
                    payload["collaboration_persistence"] = str(CollaborationPersistenceState.CANCELLED)
                    return
                aggregate = existing_agg
            else:
                eff_job_id = request_id if request_id and _COLLABORATION_ID_PATTERN.fullmatch(request_id.strip()) else None
                aggregate = job_store.create_job(
                    task_type=str(TaskTypeKind.TRIAD_ORCHESTRATION),
                    workspace_dir=resolved_workspace,
                    model=triad_model,
                    metadata=meta_fields,
                    job_id=eff_job_id,
                    idempotency_key=idempotency_key,
                    provider="heterarchy_triad",
                    engine=orchestration_mode,
                    created_at=started_at,
                )
        except OSError as exc:
            payload["collaboration_persistence"] = str(CollaborationPersistenceState.FAILED)
            payload["collaboration_persistence_error"] = redact_sensitive_text(text=str(exc))
            return

        collaboration_id = aggregate.job_id
        if _COLLABORATION_ID_PATTERN.fullmatch(collaboration_id) is None:
            raise RuntimeError("JobStore returned an invalid triad collaboration identifier.")

        snap_dto = TriadAggregateSnapshot(
            schema_version=TRIAD_RESULT_SCHEMA_VERSION,
            task_type=str(TaskTypeKind.TRIAD_ORCHESTRATION),
            collaboration_id=collaboration_id,
            orchestration_mode=orchestration_mode,
            source_task_digest=source_task_digest,
            workspace_dir=resolved_workspace,
            evidence_scope=compact_scope,
            roles=role_snapshots_json,
            adjudication=as_json_object(record=adjudication),
        )
        snapshot = as_json_object(record=snap_dto.to_dict())
        snapshot["snapshot_digest"] = digest_calc.canonical_hash(value=snapshot)
        try:
            completed = job_store.complete_job(
                job_id=aggregate.job_id,
                result=snapshot,
                model=triad_model,
                metadata_updates=meta_fields,
            )
        except OSError as exc:
            payload["collaboration_persistence"] = str(CollaborationPersistenceState.FAILED)
            payload["unconfirmed_collaboration_id"] = collaboration_id
            payload["collaboration_persistence_error"] = redact_sensitive_text(text=str(exc))
            return
        if completed is not None and completed.status == JobStatusKind.CANCELLED:
            payload["collaboration_persistence"] = str(CollaborationPersistenceState.CANCELLED)
            return
        if completed is None or completed.status != JobStatusKind.COMPLETED:
            raise RuntimeError("Triad aggregate was not completed in JobStore.")
        payload["collaboration_id"] = collaboration_id
        payload["collaboration_persistence"] = str(CollaborationPersistenceState.RECORDED)

    async def resolve_argon_role(
        self,
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
            ticket = handoff_store.get_ticket(ticket_id=argon_handoff_ticket_id.strip())
            if ticket is None:
                err_dto = ArgonTicketErrorResult(
                    ok=False,
                    status=str(JobStatusKind.FAILED),
                    error=f"Argon handoff ticket '{argon_handoff_ticket_id}' not found.",
                )
                return as_json_object(record=err_dto.to_dict())
            t_dict = ticket.to_dict()
            role_state = t_dict["collaboration_role_state"] or str(RoleExecutionState.QUEUED)
            is_done = t_dict["execution_completed"] is True
            h_dto = ArgonHandoffRoleResult(
                ok=is_done,
                status=str(JobStatusKind.SUCCEEDED) if is_done else role_state,
                outcome_state=str(OutcomeStateKind.CONFIRMED_SUCCESS if is_done else OutcomeStateKind.IN_FLIGHT),
                handoff_ticket=as_json_object(record=t_dict),
            )
            out_res = as_json_object(record=h_dto.to_dict())
            if is_done:
                note_str = (ticket.resolution_note or "").strip()
                parsed_json = AntigravityModelCatalog.extract_json_from_model_text(raw_text=note_str)
                valid_verdicts = (AuditVerdict.PASS, AuditVerdict.WARN, AuditVerdict.BLOCK)
                risks: list[str] = []
                if isinstance(parsed_json, dict) and parsed_json:
                    v_raw = str(parsed_json.get("verdict") or "").strip().upper()
                    verdict_enum = AuditVerdict(v_raw) if v_raw in valid_verdicts else AuditVerdict.PASS
                    sum_str = str(parsed_json.get("summary") or note_str).strip() or note_str
                    risks_raw = parsed_json.get("architectural_risks")
                    risks = [str(r) for r in risks_raw] if isinstance(risks_raw, list) else []
                    rec_str = str(parsed_json.get("final_recommendation") or sum_str)
                else:
                    v_match = re.search(pattern=r"\b(PASS|WARN|BLOCK)\b", string=note_str.upper())
                    verdict_enum = AuditVerdict(v_match.group(1)) if v_match else AuditVerdict.PASS
                    sum_str, rec_str = note_str, note_str
                rev_model = ArgonReviewOutput(
                    verdict=verdict_enum,
                    summary=sum_str,
                    architectural_risks=[r for r in risks],
                    final_recommendation=rec_str,
                )
                struct_out = as_json_object(record=rev_model.model_dump(mode="json"))
                is_valid, norm_out, _ = validate_structured_output(schema_name=str(SchemaName.ARGON_REVIEW), payload=struct_out)
                out_res["model_id"] = argon_model
                out_res["schema_valid"] = is_valid
                out_res["structured_output"] = norm_out if (is_valid and norm_out is not None) else struct_out
            return out_res
        if not invoke_argon_rpc:
            return None
        eff_prompt = f"{argon_prompt}\n\n{extra_context}" if extra_context else argon_prompt
        rpc_res = await run_agy_fn(
            prompt=eff_prompt,
            model=argon_model,
            expect_json=True,
            timeout_seconds=timeout_seconds,
            max_input_bytes=max_input_bytes,
            workspace_dir=workspace_dir,
            job_store=job_store,
        )
        if isinstance(rpc_res, dict) and isinstance(st := rpc_res.get("structured_output"), dict):
            rpc_res = dict(rpc_res)
            is_valid, _, _ = validate_structured_output(schema_name=str(SchemaName.ARGON_REVIEW), payload=st)
            rpc_res["schema_valid"] = is_valid
        return rpc_res

    def build_sonnet_abort_payload(
        self,
        sonnet_res: JsonObject,
        workspace_dir: str,
        provided_context_or_code: str,
        files_inspected: list[str] | None,
        symbols_inspected: list[str] | None,
        executed_test_commands: list[JsonObject] | None,
        source_task_digest: str,
        job_store: JobStore,
        started_at: float | None = None,
        collaboration_id_override: str | None = None,
        request_id: str | None = None,
        idempotency_key: str | None = None,
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
            collaboration_id_override: Optional pre-registered background job ID (`cdx-...`).
            request_id: Optional caller request ID (`cdx-...`) for the aggregate job.
            idempotency_key: Optional caller idempotency key for the aggregate job.

        Returns:
            Pretty-printed JSON string representing the aborted pipeline outcome.
        """
        role_map: dict[str, JsonObject | None] = dict(
            (
                (str(TriadRole.ARGON), None),
                (str(TriadRole.SONNET), sonnet_res),
                (str(TriadRole.CODEX), None),
            )
        )
        adjudication, role_snapshots, role_dims = EvidenceScopeBuilder().prepare_triad_evidence(
            role_responses=role_map,
            decision_authority=str(DecisionAuthorityKind.ABORTED_AT_SONNET_STAGE),
        )
        abort_dto = TriadSonnetAbortPayload(
            ok=False,
            collaboration_status=adjudication["collaboration_status"],
            consensus_status=adjudication["consensus_status"],
            consensus_reached=False,
            triad_consensus_ready=False,
            orchestration_mode=str(OrchestrationMode.PIPELINE),
            failed_stage=str(TriadStageKind.SONNET_CRAFT),
            adjudication=as_json_object(record=adjudication),
            sonnet_craft=sonnet_res,
        )
        fail_payload = as_json_object(record=abort_dto.to_dict())
        self.attach_triad_evidence_and_persist(
            payload=fail_payload,
            workspace_dir=workspace_dir,
            provided_context_or_code=provided_context_or_code,
            files_inspected=files_inspected,
            symbols_inspected=symbols_inspected,
            executed_test_commands=executed_test_commands,
            orchestration_mode=str(OrchestrationMode.PIPELINE),
            source_task_digest=source_task_digest,
            role_snapshots=role_snapshots,
            role_evidence_dimensions=role_dims,
            adjudication=adjudication,
            job_store=job_store,
            started_at=started_at,
            collaboration_id_override=collaboration_id_override,
            request_id=request_id,
            idempotency_key=idempotency_key,
        )
        return json.dumps(obj=fail_payload, ensure_ascii=False, indent=2)

    def build_completed_triad_payload(
        self,
        mode: str,
        workspace_dir: str,
        provided_context_or_code: str,
        source_task_digest: str,
        argon_model: str,
        claude_model: str,
        codex_model: str | None,
        argon_res: JsonObject | None,
        sonnet_res: JsonObject,
        codex_res: JsonObject,
        codex_verdict: str | None,
        files_inspected: list[str] | None,
        symbols_inspected: list[str] | None,
        executed_test_commands: list[JsonObject] | None,
        job_store: JobStore,
        resumed_roles: list[str] | None = None,
        resume_collaboration_id: str | None = None,
        repair_rounds_executed: int | None = None,
        bridge_executed_gates: list[JsonObject] | None = None,
        started_at: float | None = None,
        collaboration_id_override: str | None = None,
        request_id: str | None = None,
        idempotency_key: str | None = None,
    ) -> str:
        """Assemble the 3-pole response payload, attach evidence scope, and serialize JSON.

        Args:
            mode: Normalized orchestration mode (`council` or `pipeline`).
            workspace_dir: Target workspace directory path.
            provided_context_or_code: Inline code or context string to hash for evidence scope.
            source_task_digest: SHA-256 digest of the canonical task inputs.
            argon_model: Requested Gemini 4 Argon model identifier.
            claude_model: Requested Claude Sonnet/Opus model identifier.
            codex_model: Requested OpenAI Codex model identifier, or None.
            argon_res: Gemini 4 Argon role response dictionary, or None.
            sonnet_res: Claude Sonnet/Opus role response dictionary.
            codex_res: OpenAI Codex role response dictionary.
            codex_verdict: Extracted Codex verdict (`PASS`, `WARN`, `BLOCK`, or None).
            files_inspected: Optional relative file paths inspected for evidence scope.
            symbols_inspected: Optional symbol names inspected for evidence scope.
            executed_test_commands: Optional caller-reported test command records.
            job_store: Active JobStore instance used to persist the triad aggregate.
            resumed_roles: Optional list of resumed role names.
            resume_collaboration_id: Optional source collaboration ID for checkpoint resume.
            repair_rounds_executed: Optional count of auto-repair or adjudication rounds run.
            bridge_executed_gates: Optional bridge-executed verification gate records.
            started_at: Optional orchestration start timestamp for accurate duration tracking.
            collaboration_id_override: Optional pre-registered background job ID (`cdx-...`).
            request_id: Optional caller request ID (`cdx-...`) for the aggregate job.
            idempotency_key: Optional caller idempotency key for the aggregate job.

        Returns:
            Pretty-printed JSON string representing the completed triad payload.
        """
        role_map: dict[str, JsonObject | None] = dict(
            (
                (str(TriadRole.ARGON), argon_res),
                (str(TriadRole.SONNET), sonnet_res),
                (str(TriadRole.CODEX), codex_res),
            )
        )
        adjudication, role_snapshots, role_dims = EvidenceScopeBuilder().prepare_triad_evidence(role_responses=role_map)
        consensus_reached = adjudication["consensus_reached"]
        collab_status = adjudication["collaboration_status"]
        c_model = codex_res.get("model", codex_model or CanonicalModelId.CODEX_DEFAULT)
        parts_dto = TriadParticipants(
            macro_architect=argon_model,
            code_craftsman=sonnet_res.get("model_id", claude_model),
            red_team_auditor=c_model,
        )
        comp_dto = TriadCompletedPayload(
            ok=collab_status == CollaborationState.COMPLETE and consensus_reached,
            collaboration_status=collab_status,
            consensus_status=adjudication["consensus_status"],
            consensus_reached=consensus_reached,
            triad_consensus_ready=consensus_reached,
            orchestration_mode=mode,
            participants=as_json_object(record=parts_dto.to_dict()),
            adjudication=as_json_object(record=adjudication),
            argon_review=argon_res,
            sonnet_craft=sonnet_res,
            codex_audit=codex_res,
            codex_verdict=codex_verdict,
        )
        payload = as_json_object(record=comp_dto.to_dict())
        if resumed_roles:
            payload["resumed_roles"] = list(resumed_roles)
            payload["resumed_from_collaboration_id"] = resume_collaboration_id
        if repair_rounds_executed is not None:
            payload["repair_rounds_executed"] = repair_rounds_executed
        self.attach_triad_evidence_and_persist(
            payload=payload,
            workspace_dir=workspace_dir,
            provided_context_or_code=provided_context_or_code,
            files_inspected=files_inspected,
            symbols_inspected=symbols_inspected,
            executed_test_commands=executed_test_commands,
            orchestration_mode=mode,
            source_task_digest=source_task_digest,
            role_snapshots=role_snapshots,
            role_evidence_dimensions=role_dims,
            adjudication=adjudication,
            job_store=job_store,
            bridge_executed_gates=bridge_executed_gates,
            started_at=started_at,
            collaboration_id_override=collaboration_id_override,
            request_id=request_id,
            idempotency_key=idempotency_key,
        )
        return json.dumps(obj=payload, ensure_ascii=False, indent=2)
