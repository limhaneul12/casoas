"""SQLite-backed (WAL) shared blackboard store for Argon <-> Claude <-> Codex handoffs."""

from __future__ import annotations

import contextlib
import json
import sqlite3
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

_HANDOFFS_SCHEMA_SQL = """
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;
CREATE TABLE IF NOT EXISTS handoffs (
    ticket_id TEXT PRIMARY KEY,
    sender TEXT NOT NULL,
    recipient TEXT NOT NULL,
    status TEXT NOT NULL,
    priority TEXT NOT NULL,
    workspace_dir TEXT NOT NULL,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    ticket_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_handoffs_recipient_status
    ON handoffs(recipient, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_handoffs_updated_at
    ON handoffs(updated_at DESC);
"""


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
    """Low-level SQLite (WAL) storage helpers for HandoffTicket records."""

    def __init__(
        self,
        state_dir: Path | None = None,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the handoff storage base without eagerly creating directories.

        Args:
            state_dir: Optional directory path for the SQLite `handoffs.db` file.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (
            Path.home() / ".cache" / "codex-collab-mcp" / "handoffs"
        )
        self.db_path: Path = self.state_dir / "handoffs.db"
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()
        self._schema_ready: bool = False

    def ensure_db_initialized(self) -> Path:
        """Lazily create the backing directory and initialize the SQLite WAL schema.

        Returns:
            Ensured filesystem Path to the initialized `handoffs.db` SQLite database.
        """
        with self._lock:
            self.state_dir.mkdir(parents=True, exist_ok=True)
            if not self._schema_ready or not self.db_path.is_file():
                with contextlib.closing(sqlite3.connect(self.db_path, timeout=10.0)) as conn:
                    conn.executescript(_HANDOFFS_SCHEMA_SQL)
                self._schema_ready = True
            return self.db_path

    def _persist(self, ticket: HandoffTicket) -> None:
        """Atomically upsert a handoff ticket into the SQLite WAL database.

        Args:
            ticket: HandoffTicket instance to serialize and persist.
        """
        with self._lock:
            db_path = self.ensure_db_initialized()
            ticket_json = json.dumps(ticket.to_dict(), ensure_ascii=False)
            try:
                with (
                    contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn,
                    conn,
                ):
                    conn.execute("PRAGMA busy_timeout=5000")
                    conn.execute(
                        "INSERT OR REPLACE INTO handoffs ("
                        "ticket_id, sender, recipient, status, priority, "
                        "workspace_dir, created_at, updated_at, ticket_json"
                        ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        (
                            ticket.ticket_id,
                            ticket.sender,
                            ticket.recipient,
                            ticket.status,
                            ticket.priority,
                            ticket.workspace_dir,
                            ticket.created_at,
                            ticket.updated_at,
                            ticket_json,
                        ),
                    )
            except sqlite3.Error as exc:
                raise OSError(f"SQLite handoff persistence failed: {exc}") from exc

    def _deserialize_ticket(self, raw_json: str) -> HandoffTicket | None:
        """Deserialize a HandoffTicket from a stored JSON string.

        Args:
            raw_json: Serialized JSON string from the `ticket_json` column.

        Returns:
            Reconstructed HandoffTicket if valid, otherwise None.
        """
        try:
            raw = json.loads(raw_json)
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
        except json.JSONDecodeError, KeyError, TypeError, ValueError:
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
    """Thread-safe SQLite-backed blackboard for Argon, Sonnet/Opus, and Codex handoffs."""

    def __init__(self, state_dir: Path | None = None) -> None:
        """Initialize the handoff store with an explicit reentrant lock.

        Args:
            state_dir: Optional directory path for the SQLite `handoffs.db` file.
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
        """Load a handoff ticket by its identifier from SQLite.

        Args:
            ticket_id: Unique handoff ticket identifier.

        Returns:
            Matching HandoffTicket if found in SQLite, otherwise None.
        """
        with self._lock:
            db_path = self.ensure_db_initialized()
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn:
                    row = conn.execute(
                        "SELECT ticket_json FROM handoffs WHERE ticket_id = ?",
                        (ticket_id,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None or not isinstance(row[0], str):
                return None
            return self._deserialize_ticket(row[0])

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
        bounded_limit = max(1, limit)

        with self._lock:
            db_path = self.ensure_db_initialized()
            query_sql = (
                "SELECT ticket_json FROM handoffs "
                "WHERE (? IS NULL OR recipient = ?) AND (? IS NULL OR status = ?) "
                "ORDER BY updated_at DESC"
            )
            query_params = (recipient_filter, recipient_filter, status_filter, status_filter)
            tickets: list[HandoffTicket] = []
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn:
                    rows = conn.execute(query_sql, query_params).fetchall()
                for row in rows:
                    if not isinstance(row[0], str):
                        continue
                    ticket = self._deserialize_ticket(row[0])
                    if ticket is None:
                        continue
                    if ws_filter and not self._matches_workspace(ticket.workspace_dir, ws_filter):
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
        """Count pending tickets per recipient via an indexed SQLite GROUP BY query.

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
            db_path = self.ensure_db_initialized()
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn:
                    rows = conn.execute(
                        "SELECT recipient, COUNT(*) FROM handoffs "
                        "WHERE status = ? GROUP BY recipient",
                        (str(HandoffStatusKind.PENDING),),
                    ).fetchall()
                for row in rows:
                    rec_name = str(row[0])
                    if rec_name in counts and isinstance(row[1], int):
                        counts[rec_name] = row[1]
            except sqlite3.Error:
                return counts
        return counts
