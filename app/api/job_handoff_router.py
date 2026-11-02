"""MCP tool router for job lifecycle, handoff blackboard tickets, and bridge health."""

from __future__ import annotations

import app.server as _srv
from app.core.collab_enums import (
    HandoffPriorityKind,
    HandoffStatusKind,
    TriadRole,
)


@_srv.mcp.tool()
async def codex_job_status(
    job_id: str = "", idempotency_key: str | None = None, cancel: bool = False
) -> str:
    """Check status, retrieve results, or safely cancel a job by `job_id` or `idempotency_key`.

    Args:
        job_id: Job/request identifier (`cdx-...`) to query.
        idempotency_key: Optional idempotency key fallback when `job_id` is omitted.
        cancel: Whether to request safe cancellation of the target job.

    Returns:
        Pretty-printed JSON string containing the job record and outcome state.
    """
    return _srv._tool_dispatcher.job_status(
        job_store=_srv.job_store,
        job_id=job_id,
        idempotency_key=idempotency_key,
        cancel=cancel,
        cancel_task_fn=_srv._gateway.cancel_background_task,
    )


@_srv.mcp.tool()
async def codex_job_list(limit: int = 15, workspace_dir: str | None = None) -> str:
    """List recent asynchronous Codex collaboration jobs and their current statuses.

    Args:
        limit: Maximum number of recent jobs to return.
        workspace_dir: Optional workspace directory filter.

    Returns:
        Pretty-printed JSON string listing matching job records.
    """
    return _srv._tool_dispatcher.job_list(
        job_store=_srv.job_store,
        limit=limit,
        workspace_dir=workspace_dir,
    )


@_srv.mcp.tool()
async def heterarchy_handoff_create(
    title: str,
    spec_or_payload: str,
    sender: str = TriadRole.CODEX,
    recipient: str = TriadRole.ARGON,
    workspace_dir: str = ".",
    priority: str = HandoffPriorityKind.NORMAL,
) -> str:
    """Create a bidirectional task handoff ticket between GPT/Codex and Antigravity (Argon).

    Note: Ticket creation only places the task in `queued` (`pending`) state.
    It is never completion or consensus evidence until resolved with an output artifact.

    Args:
        title: Concise summary title for the handoff ticket.
        spec_or_payload: Detailed specification, diff, or payload for the recipient.
        sender: Originating pole identifier (`codex`, `argon`, `sonnet`, `opus`).
        recipient: Target pole identifier (`argon`, `codex`, `sonnet`, `opus`).
        workspace_dir: Target workspace directory path.
        priority: Ticket priority level (`low`, `normal`, `high`, `critical`).

    Returns:
        Pretty-printed JSON string describing the newly queued handoff ticket.
    """
    return _srv._tool_dispatcher.handoff_create(
        handoff_store=_srv.handoff_store,
        title=title,
        spec_or_payload=spec_or_payload,
        sender=sender,
        recipient=recipient,
        workspace_dir=workspace_dir,
        priority=priority,
    )


@_srv.mcp.tool()
async def heterarchy_handoff_inbox(
    recipient: str | None = None,
    status: str | None = HandoffStatusKind.PENDING,
    workspace_dir: str | None = None,
    limit: int = 20,
) -> str:
    """Read the shared bidirectional handoff inbox for `'argon'` (Antigravity) or `'codex'` (GPT).

    Pass `status=None` or `status=''` to inspect all tickets regardless of status.
    Pass `workspace_dir` to filter tickets for a specific project workspace.

    Args:
        recipient: Optional recipient pole filter (`argon`, `codex`, `sonnet`, `opus`).
        status: Optional ticket status filter (`pending`, `in_progress`, `completed`, `rejected`).
        workspace_dir: Optional workspace directory path filter.
        limit: Maximum number of tickets to return.

    Returns:
        Pretty-printed JSON string containing matching handoff tickets.
    """
    return _srv._tool_dispatcher.handoff_inbox(
        handoff_store=_srv.handoff_store,
        recipient=recipient,
        status=status,
        workspace_dir=workspace_dir,
        limit=limit,
    )


@_srv.mcp.tool()
async def heterarchy_handoff_resolve(
    ticket_id: str,
    status: str = HandoffStatusKind.COMPLETED,
    resolution_note: str = "",
    alexandria_ref: str = "",
    verdict: str = "",
) -> str:
    """Update or resolve a handoff ticket (`in_progress`, `completed`, or `rejected`).

    Args:
        ticket_id: Identifier of the handoff ticket (`hnd-...`) to update.
        status: New ticket status (`in_progress`, `completed`, or `rejected`).
        resolution_note: Optional summary note explaining the resolution.
        alexandria_ref: Optional Alexandria note or decision reference.
        verdict: Optional structured verdict (`PASS`, `WARN`, or `BLOCK`) for triad consensus.

    Returns:
        Pretty-printed JSON string containing the updated handoff ticket.
    """
    return _srv._tool_dispatcher.handoff_resolve(
        handoff_store=_srv.handoff_store,
        ticket_id=ticket_id,
        status=status,
        resolution_note=resolution_note,
        alexandria_ref=alexandria_ref,
        verdict=verdict,
    )


@_srv.mcp.tool()
async def codex_bridge_health() -> str:
    """Check health, verified routability, and independent auth boundaries of the bridge.

    Returns:
        Pretty-printed JSON string reporting bridge health and routing contracts.
    """
    probes = (_srv.resolve_codex_binary, _srv.discover_agy_endpoint, _srv.list_agy_models)
    return await _srv._tool_dispatcher.check_bridge_health(
        probes=probes,
        handoff_store=_srv.handoff_store,
        active_background_jobs=len(_srv._background_tasks),
        job_store=_srv.job_store,
    )


__all__ = [
    "codex_bridge_health",
    "codex_job_list",
    "codex_job_status",
    "heterarchy_handoff_create",
    "heterarchy_handoff_inbox",
    "heterarchy_handoff_resolve",
]
