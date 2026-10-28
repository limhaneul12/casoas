"""SQLite-backed (WAL) Handoff Store for Gemini 4 Argon ↔ Sonnet/Opus ↔ Codex collaboration."""

from __future__ import annotations

import sqlite3
import threading
import time
import uuid
from dataclasses import replace
from pathlib import Path

from codex_collab_mcp.collab_enums import HandoffPriorityKind, HandoffStatusKind, TriadRole
from codex_collab_mcp.evidence.workspace_digests import matches_workspace, resolve_workspace_dir
from codex_collab_mcp.routing.contract_dicts import HandoffTicketDict
from codex_collab_mcp.storage.handoff_ticket import HandoffTicket
from codex_collab_mcp.storage.sqlite_handoff_engine import _HandoffStorageBase


class HandoffStore(_HandoffStorageBase):
    """Thread-safe SQLite-backed blackboard store for cross-agent handoff tickets."""

    def __init__(self, state_dir: Path | None = None) -> None:
        """Initialize the handoff store with an explicit reentrant lock.

        Args:
            state_dir: Optional directory path for the SQLite `handoffs.db` file.
        """
        super().__init__(state_dir, lock=threading.RLock())

    def create_ticket(
        self,
        sender: str,
        recipient: str,
        title: str,
        spec_or_payload: str,
        workspace_dir: str = ".",
        priority: str = HandoffPriorityKind.NORMAL,
        alexandria_ref: str | None = None,
    ) -> HandoffTicket:
        """Create and persist a new pending handoff ticket in SQLite.

        Args:
            sender: Originating role identifier.
            recipient: Target role identifier.
            title: Short summary title of the handoff task.
            spec_or_payload: Detailed specification or payload text.
            workspace_dir: Target workspace directory path.
            priority: Ticket priority level (`low`, `normal`, `high`, `critical`).
            alexandria_ref: Optional Alexandria note or compact reference.

        Returns:
            Newly created and persisted HandoffTicket.
        """
        with self._lock:
            now = time.time()
            ticket_id = f"hnd-{uuid.uuid4().hex[:8]}"
            norm_priority = priority.strip().lower()
            valid_priorities = (
                HandoffPriorityKind.LOW,
                HandoffPriorityKind.NORMAL,
                HandoffPriorityKind.HIGH,
                HandoffPriorityKind.CRITICAL,
            )
            if norm_priority not in valid_priorities:
                norm_priority = str(HandoffPriorityKind.NORMAL)

            resolved_ws = resolve_workspace_dir(workspace_dir) or workspace_dir
            ticket = HandoffTicket(
                ticket_id=ticket_id,
                sender=sender.strip().lower(),
                recipient=recipient.strip().lower(),
                title=title.strip(),
                spec_or_payload=spec_or_payload,
                workspace_dir=resolved_ws,
                priority=norm_priority,
                status=str(HandoffStatusKind.PENDING),
                created_at=now,
                updated_at=now,
                alexandria_ref=alexandria_ref,
            )
            self._persist_ticket(ticket)
            return ticket

    def get_ticket(self, ticket_id: str) -> HandoffTicket | None:
        """Retrieve a handoff ticket by its identifier from SQLite.

        Args:
            ticket_id: Unique handoff ticket identifier.

        Returns:
            Matching HandoffTicket if found, otherwise None.
        """
        with self._lock:
            return self._load_ticket(ticket_id.strip())

    def update_ticket_status(
        self,
        ticket_id: str,
        status: str = HandoffStatusKind.COMPLETED,
        resolution_note: str | None = None,
        alexandria_ref: str | None = None,
    ) -> HandoffTicket | None:
        """Update the status, resolution note, and Alexandria reference of an existing ticket.

        Args:
            ticket_id: Unique handoff ticket identifier.
            status: Target status (`in_progress`, `completed`, `rejected`, `pending`).
            resolution_note: Optional resolution summary or deliverable note.
            alexandria_ref: Optional updated Alexandria reference.

        Returns:
            Updated HandoffTicket if found, otherwise None.
        """
        with self._lock:
            existing = self._load_ticket(ticket_id.strip())
            if existing is None:
                return None

            norm_status = status.strip().lower()
            valid_statuses = (
                HandoffStatusKind.IN_PROGRESS,
                HandoffStatusKind.COMPLETED,
                HandoffStatusKind.REJECTED,
                HandoffStatusKind.PENDING,
            )
            if norm_status not in valid_statuses:
                norm_status = str(HandoffStatusKind.COMPLETED)

            updated = replace(
                existing,
                status=norm_status,
                updated_at=time.time(),
                resolution_note=(
                    resolution_note if resolution_note is not None else existing.resolution_note
                ),
                alexandria_ref=(
                    alexandria_ref if alexandria_ref is not None else existing.alexandria_ref
                ),
            )
            self._persist_ticket(updated)
            return updated

    def list_tickets(
        self,
        recipient: str | None = None,
        status: str | None = HandoffStatusKind.PENDING,
        workspace_dir: str | None = None,
        limit: int = 20,
    ) -> list[HandoffTicketDict]:
        """List handoff tickets matching recipient, status, and workspace filters via SQLite.

        Args:
            recipient: Optional recipient role filter (`argon`, `sonnet`, `opus`, `codex`).
            status: Optional status filter (`pending`, `in_progress`, `completed`, `rejected`).
            workspace_dir: Optional workspace directory path filter.
            limit: Maximum number of serialized tickets to return.

        Returns:
            List of serialized HandoffTicketDict items sorted by updated_at descending.
        """
        recipient_filter = recipient.strip().lower() if recipient else None
        status_filter = status.strip().lower() if status else None
        ws_filter = resolve_workspace_dir(workspace_dir) if workspace_dir else None
        bounded_limit = max(1, limit)

        with self._lock:
            tickets: list[HandoffTicket] = []
            try:
                with self._connect() as conn:
                    if ws_filter is None:
                        cursor = conn.execute(
                            "SELECT ticket_json FROM handoffs "
                            "WHERE (? IS NULL OR recipient = ?) AND (? IS NULL OR status = ?) "
                            "ORDER BY updated_at DESC LIMIT ?",
                            (
                                recipient_filter,
                                recipient_filter,
                                status_filter,
                                status_filter,
                                bounded_limit,
                            ),
                        )
                    else:
                        cursor = conn.execute(
                            "SELECT ticket_json FROM handoffs "
                            "WHERE (? IS NULL OR recipient = ?) AND (? IS NULL OR status = ?) "
                            "ORDER BY updated_at DESC",
                            (recipient_filter, recipient_filter, status_filter, status_filter),
                        )
                    for row in cursor:
                        if not isinstance(row[0], str):
                            continue
                        ticket = HandoffTicket.from_json(row[0])
                        if ticket is None:
                            continue
                        if ws_filter and not matches_workspace(ticket.workspace_dir, ws_filter):
                            continue
                        tickets.append(ticket)
                        if len(tickets) >= bounded_limit:
                            break
            except sqlite3.Error:
                return []
            return [t.to_dict() for t in tickets[:bounded_limit]]

    def count_pending_by_recipient(
        self,
        recipients: tuple[str, ...] = (
            TriadRole.ARGON,
            TriadRole.SONNET,
            TriadRole.OPUS,
            TriadRole.CODEX,
        ),
    ) -> dict[str, int]:
        """Count pending handoff tickets per recipient role using a single indexed SQLite query.

        Args:
            recipients: Tuple of recipient role identifiers to count.

        Returns:
            Dictionary mapping each recipient role identifier to its pending ticket count.
        """
        normalized_recipients = tuple(r.strip().lower() for r in recipients if r.strip())
        counts: dict[str, int] = {r: 0 for r in normalized_recipients}
        if not normalized_recipients:
            return counts
        with self._lock:
            try:
                with self._connect() as conn:
                    rows = conn.execute(
                        "SELECT recipient, COUNT(*) FROM handoffs "
                        "WHERE status = ? GROUP BY recipient",
                        (str(HandoffStatusKind.PENDING),),
                    ).fetchall()
                for row in rows:
                    rec_name = str(row[0]).strip().lower()
                    if rec_name in counts and isinstance(row[1], int):
                        counts[rec_name] = row[1]
            except sqlite3.Error:
                return counts
            return counts


__all__ = ["HandoffStore"]
