"""SQLite (WAL) storage engine for HandoffTicket records."""

from __future__ import annotations

import contextlib
import json
import sqlite3
import threading
from collections.abc import Generator
from pathlib import Path

from codex_collab_mcp.storage.handoff_ticket import HandoffTicket

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
CREATE INDEX IF NOT EXISTS idx_handoffs_workspace_updated
ON handoffs(workspace_dir, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_handoffs_updated_at
ON handoffs(updated_at DESC);
"""


class _HandoffStorageBase:
    """Low-level SQLite (WAL) storage helpers for HandoffTicket records."""

    def __init__(
        self,
        state_dir: Path | None = None,
        max_retained_tickets: int = 1000,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the handoff storage base without eagerly creating directories.

        Args:
            state_dir: Optional directory path for storing the SQLite database.
            max_retained_tickets: Maximum resolved tickets retained in SQLite before pruning.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (
            Path.home() / ".cache" / "codex-collab-mcp" / "handoffs"
        )
        self.db_path: Path = self.state_dir / "handoffs.db"
        self.max_retained_tickets: int = max(1, max_retained_tickets)
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()
        self._schema_ready: bool = False
        self._write_count: int = 0

    def ensure_db_initialized(self) -> Path:
        """Lazily create the handoff directory and initialize the SQLite WAL schema.

        Returns:
            Ensured filesystem Path to the initialized `handoffs.db` SQLite database.
        """
        with self._lock:
            if self._schema_ready and self.db_path.is_file():
                return self.db_path
            self.state_dir.mkdir(parents=True, exist_ok=True)
            with contextlib.closing(sqlite3.connect(self.db_path, timeout=10.0)) as conn:
                conn.executescript(_HANDOFFS_SCHEMA_SQL)
            self._schema_ready = True
            return self.db_path

    @contextlib.contextmanager
    def _connect(self) -> Generator[sqlite3.Connection]:
        """Open a configured SQLite WAL connection and deterministically close it on exit.

        Yields:
            Configured `sqlite3.Connection` with `synchronous=NORMAL` and `busy_timeout=5000`.
        """
        db_path = self.ensure_db_initialized()
        conn = sqlite3.connect(db_path, timeout=10.0)
        try:
            conn.execute("PRAGMA synchronous=NORMAL")
            conn.execute("PRAGMA busy_timeout=5000")
            yield conn
        finally:
            conn.close()

    def _persist_ticket(self, ticket: HandoffTicket) -> None:
        """Atomically upsert a handoff ticket into the SQLite WAL database.

        Args:
            ticket: Handoff ticket to serialize and persist.
        """
        with self._lock:
            payload_json = json.dumps(ticket.to_dict(), ensure_ascii=False)
            row = (
                ticket.ticket_id,
                ticket.sender,
                ticket.recipient,
                ticket.status,
                ticket.priority,
                ticket.workspace_dir,
                ticket.created_at,
                ticket.updated_at,
                payload_json,
            )
            try:
                with self._connect() as conn, conn:
                    conn.execute(
                        "INSERT OR REPLACE INTO handoffs VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        row,
                    )
                    self._write_count += 1
                    if self._write_count % 64 == 0:
                        conn.execute(
                            "DELETE FROM handoffs WHERE status IN ('completed', 'rejected') "
                            "AND ticket_id NOT IN ("
                            "SELECT ticket_id FROM handoffs "
                            "WHERE status IN ('completed', 'rejected') "
                            "ORDER BY updated_at DESC LIMIT ?)",
                            (self.max_retained_tickets,),
                        )
            except sqlite3.Error as exc:
                raise OSError(f"SQLite handoff persistence failed: {exc}") from exc

    def prune_resolved_tickets(
        self,
        max_retained: int | None = None,
        wal_checkpoint: bool = False,
    ) -> int:
        """Delete old resolved handoff tickets beyond `max_retained` and optionally truncate WAL.

        Args:
            max_retained: Maximum number of newest tickets to keep (defaults to `max_retained_tickets`).
            wal_checkpoint: Whether to run `PRAGMA wal_checkpoint(TRUNCATE)` after pruning.

        Returns:
            Number of resolved (`completed` or `rejected`) ticket rows deleted from SQLite.
        """
        keep = max(1, max_retained if max_retained is not None else self.max_retained_tickets)
        with self._lock:
            if not self.db_path.is_file():
                return 0
            try:
                with self._connect() as conn:
                    with conn:
                        cur = conn.execute(
                            "DELETE FROM handoffs WHERE status IN ('completed', 'rejected') "
                            "AND ticket_id NOT IN ("
                            "SELECT ticket_id FROM handoffs "
                            "WHERE status IN ('completed', 'rejected') "
                            "ORDER BY updated_at DESC LIMIT ?)",
                            (keep,),
                        )
                        deleted = max(0, cur.rowcount)
                    if wal_checkpoint:
                        conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
                    return deleted
            except sqlite3.Error:
                return 0

    def _load_ticket(self, ticket_id: str) -> HandoffTicket | None:
        """Load and deserialize a handoff ticket from SQLite by ticket ID.

        Args:
            ticket_id: Unique handoff ticket identifier.

        Returns:
            Deserialized HandoffTicket if present and valid, otherwise None.
        """
        with self._lock:
            try:
                with self._connect() as conn:
                    row = conn.execute(
                        "SELECT ticket_json FROM handoffs WHERE ticket_id = ?",
                        (ticket_id,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None or not isinstance(row[0], str):
                return None
            return HandoffTicket.from_json(row[0])
