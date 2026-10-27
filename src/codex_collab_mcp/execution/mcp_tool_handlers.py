"""MCP tool handlers for job status, handoff blackboard tickets, and bridge health."""

from __future__ import annotations

import asyncio
import json
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime

from codex_collab_mcp.agy_bridge import AgyEndpoint
from codex_collab_mcp.contracts import (
    ANTIGRAVITY_MODEL_SPECS,
    CODEX_SUPPORTED_MODELS,
    get_python_runtime_info,
    redact_sensitive_text,
)
from codex_collab_mcp.domain.service_dicts import (
    AgyRpcBoundaryHealthDict,
    AuthBoundariesHealthDict,
    BridgeHealthResponseDict,
    CodexCliBoundaryHealthDict,
    HandoffCreateResponseDict,
    HandoffInboxResponseDict,
    HandoffResolveResponseDict,
    JobListResponseDict,
    JobStatusResponseDict,
    RoutingContractHealthDict,
    ToolErrorResponseDict,
)
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.schemas import JsonObject

type AsyncEndpointProbeCallable = Callable[[], Coroutine[object, object, AgyEndpoint | None]]
type AsyncModelListCallable = Callable[..., Coroutine[object, object, list[JsonObject]]]
type BridgeHealthProbes = tuple[
    Callable[[], str], AsyncEndpointProbeCallable, AsyncModelListCallable
]


class McpToolDispatcher:
    """Cohesive dispatcher for job lifecycle, handoff tickets, and health probe tools."""

    def job_status(
        self,
        job_store: JobStore,
        job_id: str,
        idempotency_key: str | None,
        cancel: bool,
    ) -> str:
        """Query or cancel a job in JobStore and return serialized JSON.

        Args:
            job_store: Active JobStore instance.
            job_id: Job/request identifier (`cdx-...`) to query.
            idempotency_key: Optional idempotency key fallback when `job_id` is omitted.
            cancel: Whether to request safe cancellation of the target job.

        Returns:
            Pretty-printed JSON string containing the job record and outcome state.
        """
        record = job_store.get_job(job_id.strip()) if job_id.strip() else None
        if record is None and idempotency_key and idempotency_key.strip():
            record = job_store.find_by_idempotency_key(idempotency_key.strip())
        if not record:
            lookup_ref = job_id or idempotency_key or "<empty>"
            err = ToolErrorResponseDict(ok=False, error=f"Job '{lookup_ref}' not found.")
            return json.dumps(err, ensure_ascii=False, indent=2)
        if cancel and (updated := job_store.cancel_job(record.job_id)) is not None:
            record = updated
        payload = JobStatusResponseDict(
            ok=True,
            job=record.to_dict(),
            outcome_state=record.outcome_state,
            cancel_requested=record.cancel_requested,
        )
        return json.dumps(payload, ensure_ascii=False, indent=2)

    def job_list(
        self,
        job_store: JobStore,
        limit: int,
        workspace_dir: str | None,
    ) -> str:
        """List recent jobs in JobStore and return serialized JSON.

        Args:
            job_store: Active JobStore instance.
            limit: Maximum number of recent jobs to return.
            workspace_dir: Optional workspace directory filter.

        Returns:
            Pretty-printed JSON string listing matching job records.
        """
        effective_ws = workspace_dir if (workspace_dir and workspace_dir.strip()) else None
        jobs = job_store.list_jobs(limit=limit, workspace_dir=effective_ws)
        payload = JobListResponseDict(ok=True, workspace_filter=effective_ws, jobs=jobs)
        return json.dumps(payload, ensure_ascii=False, indent=2)

    def handoff_create(
        self,
        handoff_store: HandoffStore,
        title: str,
        spec_or_payload: str,
        sender: str,
        recipient: str,
        workspace_dir: str,
        priority: str,
    ) -> str:
        """Create a queued handoff ticket in HandoffStore and return serialized JSON.

        Args:
            handoff_store: Active HandoffStore instance.
            title: Concise summary title for the handoff ticket.
            spec_or_payload: Detailed specification, diff, or payload for the recipient.
            sender: Originating pole identifier (`codex`, `argon`, `sonnet`, `opus`).
            recipient: Target pole identifier (`argon`, `codex`, `sonnet`, `opus`).
            workspace_dir: Target workspace directory path.
            priority: Ticket priority level (`low`, `normal`, `high`, `critical`).

        Returns:
            Pretty-printed JSON string describing the newly queued handoff ticket.
        """
        ticket = handoff_store.create_ticket(
            sender=sender,
            recipient=recipient,
            title=title,
            spec_or_payload=spec_or_payload,
            workspace_dir=workspace_dir,
            priority=priority,
        )
        msg = (
            f"Handoff ticket '{ticket.ticket_id}' queued from '{ticket.sender}' to "
            f"'{ticket.recipient}' (status='pending', collaboration_role_state='queued'). "
            "Do not treat ticket creation as completion."
        )
        payload = HandoffCreateResponseDict(
            ok=True,
            ticket=ticket.to_dict(),
            collaboration_role_state="queued",
            execution_completed=False,
            consensus_eligible=False,
            message=msg,
        )
        return json.dumps(payload, ensure_ascii=False, indent=2)

    def handoff_inbox(
        self,
        handoff_store: HandoffStore,
        recipient: str | None,
        status: str | None,
        workspace_dir: str | None,
        limit: int,
    ) -> str:
        """List handoff tickets in HandoffStore and return serialized JSON.

        Args:
            handoff_store: Active HandoffStore instance.
            recipient: Optional recipient pole filter (`argon`, `codex`, `sonnet`, `opus`).
            status: Optional ticket status filter (`pending`, `in_progress`, `completed`, `rejected`).
            workspace_dir: Optional workspace directory path filter.
            limit: Maximum number of tickets to return.

        Returns:
            Pretty-printed JSON string containing matching handoff tickets.
        """
        effective_status = status if (status and status.strip()) else None
        effective_ws = workspace_dir if (workspace_dir and workspace_dir.strip()) else None
        tickets = handoff_store.list_tickets(
            recipient=recipient,
            status=effective_status,
            workspace_dir=effective_ws,
            limit=limit,
        )
        payload = HandoffInboxResponseDict(
            ok=True,
            recipient_filter=recipient,
            status_filter=effective_status,
            workspace_filter=effective_ws,
            count=len(tickets),
            tickets=tickets,
        )
        return json.dumps(payload, ensure_ascii=False, indent=2)

    def handoff_resolve(
        self,
        handoff_store: HandoffStore,
        ticket_id: str,
        status: str,
        resolution_note: str,
        alexandria_ref: str,
    ) -> str:
        """Update or resolve a handoff ticket in HandoffStore and return serialized JSON.

        Args:
            handoff_store: Active HandoffStore instance.
            ticket_id: Identifier of the handoff ticket (`hnd-...`) to update.
            status: New ticket status (`in_progress`, `completed`, or `rejected`).
            resolution_note: Optional summary note explaining the resolution.
            alexandria_ref: Optional Alexandria note or decision reference.

        Returns:
            Pretty-printed JSON string containing the updated handoff ticket.
        """
        updated = handoff_store.update_ticket_status(
            ticket_id=ticket_id,
            status=status,
            resolution_note=resolution_note or None,
            alexandria_ref=alexandria_ref or None,
        )
        if updated is None:
            err = ToolErrorResponseDict(ok=False, error=f"Handoff ticket '{ticket_id}' not found.")
            return json.dumps(err, ensure_ascii=False, indent=2)
        payload = HandoffResolveResponseDict(ok=True, ticket=updated.to_dict())
        return json.dumps(payload, ensure_ascii=False, indent=2)

    async def check_bridge_health(
        self,
        probes: BridgeHealthProbes,
        handoff_store: HandoffStore,
        active_background_jobs: int,
    ) -> str:
        """Probe Codex CLI binary, Antigravity local RPC endpoint, and handoff queue counts.

        Args:
            probes: Tuple of `(resolve_codex_fn, discover_agy_fn, list_agy_models_fn)`.
            handoff_store: Active HandoffStore instance for pending ticket counts.
            active_background_jobs: Count of currently running background tasks.

        Returns:
            Pretty-printed JSON string reporting bridge health and routing contracts.
        """
        resolve_codex_fn, discover_agy_fn, list_agy_models_fn = probes
        verified_at_utc = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        try:
            codex_bin = resolve_codex_fn()
            proc = await asyncio.create_subprocess_exec(
                codex_bin,
                "--version",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            try:
                stdout_bytes, _ = await asyncio.wait_for(proc.communicate(), timeout=5.0)
            except BaseException:
                proc.kill()
                await proc.wait()
                raise
            version_str = stdout_bytes.decode("utf-8", errors="replace").strip()
            cli_ready = proc.returncode == 0
        except Exception as exc:
            codex_bin = "not_found"
            version_str = redact_sensitive_text(str(exc))
            cli_ready = False

        agy_ep = await discover_agy_fn()
        agy_models = await list_agy_models_fn(endpoint=agy_ep) if agy_ep else []
        routable_agy_models: list[str] = [
            mid
            for m in agy_models
            if bool(m.get("routable")) and isinstance((mid := m.get("model_id")), str)
        ]
        advertised_agy_specs = [spec.to_dict() for spec in ANTIGRAVITY_MODEL_SPECS.values()]
        routable_codex_models: list[str] = sorted(CODEX_SUPPORTED_MODELS) if cli_ready else []
        pending_counts = handoff_store.count_pending_by_recipient(
            ("argon", "sonnet", "opus", "codex")
        )

        routing_contract = RoutingContractHealthDict(
            verified_at_utc=verified_at_utc,
            silent_fallback_enabled=False,
            claude_tier_policy="medium_only",
            advertised_antigravity_models=advertised_agy_specs,
            routable_antigravity_models=routable_agy_models,
            antigravity_routable=agy_ep is not None and len(routable_agy_models) > 0,
            advertised_codex_models=sorted(CODEX_SUPPORTED_MODELS),
            routable_codex_models=routable_codex_models,
            codex_cli_routable=cli_ready,
        )
        agy_auth = AgyRpcBoundaryHealthDict(
            auth_context="antigravity_local_csrf",
            connected=agy_ep is not None,
            endpoint=agy_ep.base_url if agy_ep else None,
            csrf_token_redacted=True,
            independent_of_codex_cli_oauth=True,
        )
        codex_auth = CodexCliBoundaryHealthDict(
            auth_context="codex_chatgpt_oauth",
            cli_binary_ready=cli_ready,
            independent_of_antigravity_rpc=True,
            auto_reauth_enabled=False,
        )
        auth_boundaries = AuthBoundariesHealthDict(
            antigravity_rpc=agy_auth,
            codex_cli_oauth=codex_auth,
        )
        response = BridgeHealthResponseDict(
            ok=cli_ready,
            verified_at_utc=verified_at_utc,
            python_runtime=get_python_runtime_info(),
            codex_binary=codex_bin,
            codex_version=version_str,
            agy_bridge_ready=agy_ep is not None,
            agy_endpoint=agy_ep.base_url if agy_ep else None,
            antigravity_models=agy_models,
            routing_contract=routing_contract,
            auth_boundaries=auth_boundaries,
            active_background_jobs=active_background_jobs,
            pending_argon_handoffs=pending_counts.get("argon", 0),
            pending_sonnet_handoffs=pending_counts.get("sonnet", 0),
            pending_opus_handoffs=pending_counts.get("opus", 0),
            pending_codex_handoffs=pending_counts.get("codex", 0),
        )
        return json.dumps(response, ensure_ascii=False, indent=2)
