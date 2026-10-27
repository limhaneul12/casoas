"""SQLite (WAL) storage engine for HandoffTicket records."""

from __future__ import annotations

import contextlib
import json
import sqlite3
import threading
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
CREATE INDEX IF NOT EXISTS idx_handoffs_updated_at
ON handoffs(updated_at DESC);
"""


class _HandoffStorageBase:
    """Low-level SQLite (WAL) storage helpers for HandoffTicket records."""

    def __init__(
        self,
        state_dir: Path | None = None,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the handoff storage base without eagerly creating directories.

        Args:
            state_dir: Optional directory path for storing the SQLite database.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (
            Path.home() / ".cache" / "codex-collab-mcp" / "handoffs"
        )
        self.db_path: Path = self.state_dir / "handoffs.db"
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()
        self._schema_ready: bool = False

    def ensure_db_initialized(self) -> Path:
        """Lazily create the handoff directory and initialize the SQLite WAL schema.

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

    def _persist_ticket(self, ticket: HandoffTicket) -> None:
        """Atomically upsert a handoff ticket into the SQLite WAL database.

        Args:
            ticket: Handoff ticket to serialize and persist.
        """
        with self._lock:
            db_path = self.ensure_db_initialized()
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
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn, conn:
                    conn.execute("PRAGMA busy_timeout=5000")
                    conn.execute(
                        "INSERT OR REPLACE INTO handoffs VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        row,
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
        return HandoffTicket.from_json(raw_json)

    def _load_ticket(self, ticket_id: str) -> HandoffTicket | None:
        """Load and deserialize a handoff ticket from SQLite by ticket ID.

        Args:
            ticket_id: Unique handoff ticket identifier.

        Returns:
            Deserialized HandoffTicket if present and valid, otherwise None.
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
