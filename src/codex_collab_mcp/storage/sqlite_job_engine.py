"""SQLite (WAL) storage engine and LRU query layer for CodexJobRecord items."""

from __future__ import annotations

import contextlib
import json
import sqlite3
import threading
import time
from collections import OrderedDict
from collections.abc import Generator
from pathlib import Path

from codex_collab_mcp.evidence.workspace_digests import matches_workspace, resolve_workspace_dir
from codex_collab_mcp.routing.contract_dicts import CodexJobRecordDict, JobTelemetrySummaryDict
from codex_collab_mcp.storage.job_record import CodexJobRecord

_JOBS_SCHEMA_SQL = (
    "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;"
    "CREATE TABLE IF NOT EXISTS jobs ("
    "job_id TEXT PRIMARY KEY, task_type TEXT NOT NULL, status TEXT NOT NULL, "
    "outcome_state TEXT NOT NULL, idempotency_key TEXT, workspace_dir TEXT NOT NULL, "
    "created_at REAL NOT NULL, updated_at REAL NOT NULL, record_json TEXT NOT NULL);"
    "CREATE INDEX IF NOT EXISTS idx_jobs_idempotency_key "
    "ON jobs(idempotency_key) WHERE idempotency_key IS NOT NULL;"
    "CREATE INDEX IF NOT EXISTS idx_jobs_created_at ON jobs(created_at DESC);"
    "CREATE INDEX IF NOT EXISTS idx_jobs_workspace_created ON jobs(workspace_dir, created_at DESC);"
)


class _JobStorageEngine:
    """Low-level LRU cache and SQLite (WAL) persistence engine for CodexJobRecord items."""

    def __init__(
        self,
        state_dir: Path | None = None,
        max_cached_jobs: int = 512,
        max_retained_jobs: int = 1000,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the storage engine without eagerly creating directories on disk.

        Args:
            state_dir: Optional directory path for the SQLite database file.
            max_cached_jobs: Maximum number of records retained in the in-memory LRU cache.
            max_retained_jobs: Maximum terminal job rows retained in SQLite before pruning.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (Path.home() / ".cache" / "codex-collab-mcp" / "jobs")
        self.db_path: Path = self.state_dir / "jobs.db"
        self.max_cached_jobs: int = max(1, max_cached_jobs)
        self.max_retained_jobs: int = max(1, max_retained_jobs)
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()
        self._jobs: OrderedDict[str, CodexJobRecord] = OrderedDict()
        self._schema_ready: bool = False
        self._write_count: int = 0

    def ensure_db_initialized(self) -> Path:
        """Lazily create the backing directory and initialize the SQLite WAL schema.

        Returns:
            Ensured filesystem Path to the initialized `jobs.db` SQLite database.
        """
        with self._lock:
            if self._schema_ready and self.db_path.is_file():
                return self.db_path
            self.state_dir.mkdir(parents=True, exist_ok=True)
            with contextlib.closing(sqlite3.connect(self.db_path, timeout=10.0)) as conn:
                conn.executescript(_JOBS_SCHEMA_SQL)
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

    def _cache_put(self, record: CodexJobRecord) -> None:
        """Insert or refresh a job record in the bounded in-memory LRU cache.

        Args:
            record: Job record to cache.
        """
        self._jobs[record.job_id] = record
        self._jobs.move_to_end(record.job_id)
        while len(self._jobs) > self.max_cached_jobs:
            self._jobs.popitem(last=False)

    def _persist(self, record: CodexJobRecord) -> None:
        """Atomically upsert a job record into the SQLite WAL database.

        Args:
            record: Job record to serialize and persist.
        """
        with self._lock:
            r, ik = record, (record.idempotency_key or "").strip() or None
            rj = json.dumps(r.to_dict(), ensure_ascii=False)
            ua = r.completed_at or r.cancelled_at or time.time()
            p1 = (r.job_id, r.task_type, r.status, r.outcome_state, ik)
            p2 = (r.workspace_dir, r.created_at, ua, rj)
            try:
                with self._connect() as conn, conn:
                    conn.execute("INSERT OR REPLACE INTO jobs VALUES (?,?,?,?,?,?,?,?,?)", p1 + p2)
                    self._write_count += 1
                    if self._write_count % 64 == 0:
                        evicted = conn.execute(
                            "DELETE FROM jobs WHERE status != 'running' AND job_id NOT IN ("
                            "SELECT job_id FROM jobs WHERE status != 'running' "
                            "ORDER BY created_at DESC LIMIT ?) RETURNING job_id",
                            (self.max_retained_jobs,),
                        ).fetchall()
                        for row in evicted:
                            self._jobs.pop(str(row[0]), None)
            except sqlite3.Error as exc:
                raise OSError(f"SQLite job persistence failed: {exc}") from exc

    def prune_completed_jobs(
        self,
        max_retained: int | None = None,
        wal_checkpoint: bool = False,
    ) -> int:
        """Delete old terminal job records beyond `max_retained` and optionally truncate WAL.

        Args:
            max_retained: Maximum number of newest jobs to keep (defaults to `max_retained_jobs`).
            wal_checkpoint: Whether to run `PRAGMA wal_checkpoint(TRUNCATE)` after pruning.

        Returns:
            Number of terminal job rows deleted from SQLite.
        """
        keep = max(1, max_retained if max_retained is not None else self.max_retained_jobs)
        with self._lock:
            if not self.db_path.is_file():
                return 0
            try:
                with self._connect() as conn:
                    with conn:
                        evicted = conn.execute(
                            "DELETE FROM jobs WHERE status != 'running' AND job_id NOT IN ("
                            "SELECT job_id FROM jobs WHERE status != 'running' "
                            "ORDER BY created_at DESC LIMIT ?) RETURNING job_id",
                            (keep,),
                        ).fetchall()
                        for row in evicted:
                            self._jobs.pop(str(row[0]), None)
                    if wal_checkpoint:
                        conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
                    return len(evicted)
            except sqlite3.Error:
                return 0

    def get_persisted_job(self, job_id: str) -> CodexJobRecord | None:
        """Read a job record directly from SQLite without consulting or mutating the cache.

        Args:
            job_id: Unique job identifier to load from SQLite.

        Returns:
            Persisted CodexJobRecord if present in SQLite, otherwise None.
        """
        with self._lock:
            try:
                with self._connect() as conn:
                    row = conn.execute(
                        "SELECT record_json FROM jobs WHERE job_id = ?",
                        (job_id,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None or not isinstance(row[0], str):
                return None
            return CodexJobRecord.from_json(row[0])


class _JobQueryStore(_JobStorageEngine):
    """Read and lookup operations across cached and SQLite-persisted job records."""

    def _save_job(self, record: CodexJobRecord) -> CodexJobRecord:
        """Persist a job record to SQLite and update the in-memory LRU cache.

        Args:
            record: Job record to persist and cache.

        Returns:
            Persisted and cached CodexJobRecord.
        """
        self._persist(record)
        self._cache_put(record)
        return record

    def get_job(self, job_id: str) -> CodexJobRecord | None:
        """Retrieve a job record by ID from the LRU cache or SQLite fallback.

        Args:
            job_id: Unique job identifier.

        Returns:
            Matching CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (cached := self._jobs.get(job_id)) is not None:
                self._jobs.move_to_end(job_id)
                return cached
            if (record := self.get_persisted_job(job_id)) is not None:
                self._cache_put(record)
            return record

    def find_by_idempotency_key(self, idempotency_key: str) -> CodexJobRecord | None:
        """Find an existing job record by its idempotency_key via LRU cache or indexed SQLite lookup.

        Args:
            idempotency_key: Caller-supplied idempotency key string.

        Returns:
            Matching CodexJobRecord if found, otherwise None.
        """
        if not (key := idempotency_key.strip()):
            return None
        with self._lock:
            for jid in reversed(self._jobs):
                cached = self._jobs[jid]
                if cached.idempotency_key == key:
                    self._jobs.move_to_end(jid)
                    return cached
            try:
                with self._connect() as conn:
                    row = conn.execute(
                        "SELECT job_id, record_json FROM jobs "
                        "WHERE idempotency_key = ? ORDER BY created_at DESC LIMIT 1",
                        (key,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None:
                return None
            if isinstance(row[1], str) and (rec := CodexJobRecord.from_json(row[1])) is not None:
                self._cache_put(rec)
                return rec
            return None

    def list_jobs(
        self, limit: int = 20, workspace_dir: str | None = None
    ) -> list[CodexJobRecordDict]:
        """List recent job records ordered by creation timestamp descending.

        Args:
            limit: Maximum number of serialized job records to return.
            workspace_dir: Optional workspace directory path filter.

        Returns:
            List of serialized CodexJobRecordDict items.
        """
        ws = resolve_workspace_dir(workspace_dir) if workspace_dir else None
        bounded_limit = max(1, limit)
        with self._lock:
            seen: dict[str, CodexJobRecord] = {
                jid: rec
                for jid, rec in self._jobs.items()
                if ws is None or matches_workspace(rec.workspace_dir, ws)
            }
            try:
                with self._connect() as conn:
                    if ws is not None:
                        rows = conn.execute(
                            "SELECT job_id, workspace_dir, record_json FROM jobs "
                            "WHERE workspace_dir = ? ORDER BY created_at DESC LIMIT ?",
                            (ws, bounded_limit),
                        ).fetchall()
                        if len(rows) < bounded_limit:
                            rows.extend(
                                conn.execute(
                                    "SELECT job_id, workspace_dir, record_json FROM jobs "
                                    "WHERE workspace_dir != ? ORDER BY created_at DESC LIMIT ?",
                                    (ws, bounded_limit * 4),
                                ).fetchall()
                            )
                    else:
                        rows = conn.execute(
                            "SELECT job_id, workspace_dir, record_json FROM jobs "
                            "ORDER BY created_at DESC LIMIT ?",
                            (bounded_limit,),
                        ).fetchall()
                for row in rows:
                    jid, row_ws, raw_json = str(row[0]), str(row[1]), row[2]
                    if jid in seen or not isinstance(raw_json, str):
                        continue
                    if ws is not None and row_ws != ws and not matches_workspace(row_ws, ws):
                        continue
                    if (rec := CodexJobRecord.from_json(raw_json)) is not None:
                        seen[jid] = rec
            except sqlite3.Error:
                pass
            records = sorted(seen.values(), key=lambda r: r.created_at, reverse=True)
            return [r.to_dict() for r in records[:bounded_limit]]

    def summarize_telemetry(self) -> JobTelemetrySummaryDict:
        """Compute aggregated execution telemetry across all persisted SQLite job rows.

        Returns:
            JobTelemetrySummaryDict containing counts by status and average duration.
        """
        with self._lock:
            if not self.db_path.is_file():
                return JobTelemetrySummaryDict(
                    total_jobs=0,
                    completed_jobs=0,
                    failed_jobs=0,
                    timed_out_unknown_jobs=0,
                    running_jobs=0,
                    cancelled_jobs=0,
                    avg_duration_seconds=0.0,
                )
            try:
                with self._connect() as conn:
                    row = conn.execute(
                        "SELECT COUNT(*), "
                        "COALESCE(SUM(CASE WHEN status IN ('completed', 'succeeded') THEN 1 ELSE 0 END), 0), "
                        "COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0), "
                        "COALESCE(SUM(CASE WHEN status = 'timed_out_unknown' THEN 1 ELSE 0 END), 0), "
                        "COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0), "
                        "COALESCE(SUM(CASE WHEN status = 'cancelled' THEN 1 ELSE 0 END), 0), "
                        "COALESCE(AVG(CASE WHEN status != 'running' AND updated_at >= created_at "
                        "THEN updated_at - created_at END), 0.0) FROM jobs"
                    ).fetchone()
            except sqlite3.Error:
                row = None
            if row is None:
                return JobTelemetrySummaryDict(
                    total_jobs=0,
                    completed_jobs=0,
                    failed_jobs=0,
                    timed_out_unknown_jobs=0,
                    running_jobs=0,
                    cancelled_jobs=0,
                    avg_duration_seconds=0.0,
                )
            return JobTelemetrySummaryDict(
                total_jobs=int(row[0] or 0),
                completed_jobs=int(row[1] or 0),
                failed_jobs=int(row[2] or 0),
                timed_out_unknown_jobs=int(row[3] or 0),
                running_jobs=int(row[4] or 0),
                cancelled_jobs=int(row[5] or 0),
                avg_duration_seconds=round(float(row[6] or 0.0), 2),
            )
