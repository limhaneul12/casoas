"""Shared disk-backed blackboard store for bidirectional Argon <-> Codex task handoffs."""

from __future__ import annotations

import json
import os
import threading
import time
import uuid
from dataclasses import dataclass, replace
from pathlib import Path

from codex_collab_mcp.collab_enums import (
    HandoffPriorityKind,
    HandoffStatusKind,
    RoleExecutionState,
    TriadRole,
)
from codex_collab_mcp.domain.contract_dicts import HandoffTicketDict


@dataclass(frozen=True, slots=True)
class HandoffTicket:
    """Durable handoff ticket exchanged between Argon, Sonnet/Opus, and Codex."""

    ticket_id: str
    sender: str  # "codex" | "argon"
    recipient: str  # "argon" | "codex"
    title: str
    spec_or_payload: str
    workspace_dir: str
    priority: str  # "low" | "normal" | "high" | "critical"
    status: str  # "pending" | "in_progress" | "completed" | "rejected"
    created_at: float
    updated_at: float
    resolution_note: str | None = None
    alexandria_ref: str | None = None

    def to_dict(self) -> HandoffTicketDict:
        """Serialize the handoff ticket into a TypedDict payload.

        Returns:
            Serialized HandoffTicketDict with derived collaboration role state flags.
        """
        role_state_map: dict[str, str] = {
            HandoffStatusKind.PENDING: RoleExecutionState.QUEUED,
            HandoffStatusKind.IN_PROGRESS: RoleExecutionState.RUNNING,
            HandoffStatusKind.COMPLETED: RoleExecutionState.SUCCEEDED,
            HandoffStatusKind.REJECTED: RoleExecutionState.FAILED,
        }
        collab_role_state = role_state_map.get(self.status, RoleExecutionState.QUEUED)
        is_completed = self.status == HandoffStatusKind.COMPLETED and bool(
            (self.resolution_note or "").strip()
        )
        return HandoffTicketDict(
            ticket_id=self.ticket_id,
            sender=self.sender,
            recipient=self.recipient,
            title=self.title,
            spec_or_payload=self.spec_or_payload,
            workspace_dir=self.workspace_dir,
            priority=self.priority,
            status=self.status,
            collaboration_role_state=collab_role_state,
            execution_completed=is_completed,
            consensus_eligible=is_completed,
            created_at=self.created_at,
            updated_at=self.updated_at,
            resolution_note=self.resolution_note,
            alexandria_ref=self.alexandria_ref,
        )


class _HandoffStorageBase:
    """Low-level atomic disk storage helpers for HandoffTicket files."""

    def __init__(
        self,
        state_dir: Path | None = None,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the handoff storage base without eagerly creating directories.

        Args:
            state_dir: Optional directory path for persisted handoff ticket JSON files.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (
            Path.home() / ".cache" / "codex-collab-mcp" / "handoffs"
        )
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()

    def _ensure_state_dir(self) -> Path:
        """Lazily create and return the backing directory for handoff tickets.

        Returns:
            Ensured filesystem Path for ticket storage.
        """
        self.state_dir.mkdir(parents=True, exist_ok=True)
        return self.state_dir

    def _ticket_path(self, ticket_id: str) -> Path:
        """Build the canonical JSON file path for a handoff ticket ID.

        Args:
            ticket_id: Unique handoff ticket identifier.

        Returns:
            Filesystem Path to the ticket's JSON file.
        """
        return self._ensure_state_dir() / f"{ticket_id}.json"

    def _persist(self, ticket: HandoffTicket) -> None:
        """Atomically write a handoff ticket to disk via temporary file replacement.

        Args:
            ticket: HandoffTicket instance to serialize and persist.
        """
        with self._lock:
            state_dir = self._ensure_state_dir()
            target_path = state_dir / f"{ticket.ticket_id}.json"
            tmp_path = state_dir / f".{ticket.ticket_id}.{uuid.uuid4().hex[:6]}.tmp"
            tmp_path.write_text(
                json.dumps(ticket.to_dict(), ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            os.replace(tmp_path, target_path)

    def _read_ticket_file(self, path: Path) -> HandoffTicket | None:
        """Deserialize a handoff ticket from a JSON file on disk.

        Args:
            path: Filesystem path to the persisted ticket JSON file.

        Returns:
            Reconstructed HandoffTicket if valid and readable, otherwise None.
        """
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
            if not isinstance(raw, dict):
                return None
            raw_res = raw.get("resolution_note")
            raw_alex = raw.get("alexandria_ref")
            return HandoffTicket(
                ticket_id=str(raw["ticket_id"]),
                sender=str(raw["sender"]),
                recipient=str(raw["recipient"]),
                title=str(raw["title"]),
                spec_or_payload=str(raw["spec_or_payload"]),
                workspace_dir=str(raw["workspace_dir"]),
                priority=str(raw["priority"]),
                status=str(raw["status"]),
                created_at=float(raw["created_at"]),
                updated_at=float(raw["updated_at"]),
                resolution_note=raw_res if isinstance(raw_res, str) else None,
                alexandria_ref=raw_alex if isinstance(raw_alex, str) else None,
            )
        except OSError, json.JSONDecodeError, KeyError, TypeError, ValueError:
            return None

    @staticmethod
    def _matches_workspace(ticket_ws: str, ws_filter: str) -> bool:
        """Check whether a ticket's workspace directory matches the resolved filter path.

        Args:
            ticket_ws: Workspace directory string stored on the ticket.
            ws_filter: Resolved absolute workspace directory filter string.

        Returns:
            True if the workspace paths match, otherwise False.
        """
        if ticket_ws == ws_filter:
            return True
        if not ticket_ws.strip() or Path(ticket_ws).is_absolute():
            return False
        return str(Path(ticket_ws).expanduser().resolve()) == ws_filter


class HandoffStore(_HandoffStorageBase):
    """Thread-safe atomic disk-backed blackboard for Argon, Sonnet/Opus, and Codex handoffs."""

    def __init__(self, state_dir: Path | None = None) -> None:
        """Initialize the handoff store with an explicit reentrant lock.

        Args:
            state_dir: Optional directory path for persisted handoff ticket JSON files.
        """
        super().__init__(state_dir=state_dir, lock=threading.RLock())

    def create_ticket(
        self,
        sender: str,
        recipient: str,
        title: str,
        spec_or_payload: str,
        workspace_dir: str = ".",
        priority: str = HandoffPriorityKind.NORMAL,
    ) -> HandoffTicket:
        """Create and atomically persist a new pending handoff ticket.

        Args:
            sender: Role identifier of the ticket creator.
            recipient: Role identifier of the target recipient.
            title: Short summary title for the handoff task.
            spec_or_payload: Detailed specification or payload text.
            workspace_dir: Target workspace directory path.
            priority: Priority level string ('low', 'normal', 'high', 'critical').

        Returns:
            Newly created and persisted HandoffTicket.
        """
        with self._lock:
            now = time.time()
            ticket_id = f"hnd-{uuid.uuid4().hex[:10]}"
            resolved_ws = str(Path(workspace_dir).expanduser().resolve())
            ticket = HandoffTicket(
                ticket_id=ticket_id,
                sender=sender.strip().lower(),
                recipient=recipient.strip().lower(),
                title=title.strip(),
                spec_or_payload=spec_or_payload,
                workspace_dir=resolved_ws,
                priority=priority.strip().lower(),
                status=HandoffStatusKind.PENDING,
                created_at=now,
                updated_at=now,
            )
            self._persist(ticket)
            return ticket

    def get_ticket(self, ticket_id: str) -> HandoffTicket | None:
        """Load a handoff ticket by its identifier from disk.

        Args:
            ticket_id: Unique handoff ticket identifier.

        Returns:
            Matching HandoffTicket if found on disk, otherwise None.
        """
        with self._lock:
            path = self._ticket_path(ticket_id)
            if not path.is_file():
                return None
            return self._read_ticket_file(path)

    def update_ticket_status(
        self,
        ticket_id: str,
        status: str,
        resolution_note: str | None = None,
        alexandria_ref: str | None = None,
    ) -> HandoffTicket | None:
        """Update a handoff ticket's status, resolution note, and Alexandria reference.

        Args:
            ticket_id: Unique handoff ticket identifier.
            status: New status string ('in_progress', 'completed', or 'rejected').
            resolution_note: Optional resolution summary note.
            alexandria_ref: Optional Alexandria durable memory reference.

        Returns:
            Updated HandoffTicket if found, otherwise None.
        """
        with self._lock:
            ticket = self.get_ticket(ticket_id)
            if ticket is None:
                return None
            updated = replace(
                ticket,
                status=status.strip().lower(),
                updated_at=time.time(),
                resolution_note=(
                    resolution_note if resolution_note is not None else ticket.resolution_note
                ),
                alexandria_ref=(
                    alexandria_ref if alexandria_ref is not None else ticket.alexandria_ref
                ),
            )
            self._persist(updated)
            return updated

    def list_tickets(
        self,
        recipient: str | None = None,
        status: str | None = None,
        workspace_dir: str | None = None,
        limit: int = 20,
    ) -> list[HandoffTicketDict]:
        """List handoff tickets matching optional recipient, status, and workspace filters.

        Args:
            recipient: Optional recipient role filter.
            status: Optional ticket status filter.
            workspace_dir: Optional workspace directory path filter.
            limit: Maximum number of serialized tickets to return.

        Returns:
            List of serialized HandoffTicketDict items sorted by updated_at descending.
        """
        recipient_filter = recipient.strip().lower() if recipient else None
        status_filter = status.strip().lower() if status else None
        ws_filter = (
            str(Path(workspace_dir).expanduser().resolve())
            if workspace_dir and workspace_dir.strip()
            else None
        )

        with self._lock:
            state_dir = self._ensure_state_dir()
            tickets: list[HandoffTicket] = []
            for path in state_dir.glob("hnd-*.json"):
                ticket = self._read_ticket_file(path)
                if ticket is None:
                    continue
                if recipient_filter and ticket.recipient != recipient_filter:
                    continue
                if status_filter and ticket.status != status_filter:
                    continue
                if ws_filter and not self._matches_workspace(ticket.workspace_dir, ws_filter):
                    continue
                tickets.append(ticket)

            tickets.sort(key=lambda t: t.updated_at, reverse=True)
            return [t.to_dict() for t in tickets[:limit]]

    def count_pending_by_recipient(
        self,
        recipients: tuple[str, ...] = (
            TriadRole.ARGON,
            TriadRole.SONNET,
            TriadRole.OPUS,
            TriadRole.CODEX,
        ),
    ) -> dict[str, int]:
        """Count pending tickets per recipient in a single directory scan.

        Args:
            recipients: Tuple of recipient role names to tally.

        Returns:
            Dictionary mapping each normalized recipient name to its pending ticket count.
        """
        normalized_recipients = tuple(r.strip().lower() for r in recipients if r.strip())
        counts: dict[str, int] = {r: 0 for r in normalized_recipients}
        if not counts:
            return counts
        with self._lock:
            state_dir = self._ensure_state_dir()
            for path in state_dir.glob("hnd-*.json"):
                ticket = self._read_ticket_file(path)
                if ticket is None or ticket.status != HandoffStatusKind.PENDING:
                    continue
                if ticket.recipient in counts:
                    counts[ticket.recipient] += 1
        return counts
