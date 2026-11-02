"""SQLite (WAL) storage engine and LRU query layer for CodexJobRecord items."""

from __future__ import annotations

import contextlib
import json
import os
import sqlite3
import threading
import time
from collections import OrderedDict
from collections.abc import Generator
from dataclasses import replace
from pathlib import Path

from app.core.collab_enums import JobStatusKind, OutcomeStateKind
from app.evidence.workspace_digests import WorkspaceDigestCalculator, matches_workspace
from app.repositories.job_record import CodexJobRecord
from app.repositories.sql_loader import SQL_LOADER
from app.repositories.sql_query_enums import JobSqlQueryId, SqlFeatureFile
from app.schemas.inner_dto.contract_dtos import JobTelemetrySummary
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import (
    CodexJobRecordDict,
    JobTelemetrySummaryDict,
)


def _is_pid_alive(pid: int) -> bool:
    """Return whether an OS process with `pid` is currently alive.

    Args:
        pid: Operating system process identifier to probe.

    Returns:
        True if the process exists, False if it has terminated or is invalid.
    """
    if pid <= 0:
        return False
    if pid == os.getpid():
        return True
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False
    return True


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
                SQL_LOADER.execute_script(
                    conn=conn,
                    feature_file=SqlFeatureFile.JOB_STORE,
                    query_id=JobSqlQueryId.INIT_SCHEMA,
                )
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
            SQL_LOADER.execute_script(
                conn=conn,
                feature_file=SqlFeatureFile.JOB_STORE,
                query_id=JobSqlQueryId.PRAGMA_CONNECT,
            )
            yield conn
        finally:
            conn.close()

    def _cache_put(self, record: CodexJobRecord) -> None:
        """Insert or refresh a job record in the bounded in-memory LRU cache.

        Args:
            record: Job record to cache.
        """
        self._jobs[record.job_id] = record
        self._jobs.move_to_end(key=record.job_id)
        while len(self._jobs) > self.max_cached_jobs:
            self._jobs.popitem(last=False)

    def _persist(self, record: CodexJobRecord) -> None:
        """Atomically upsert a job record into the SQLite WAL database.

        Args:
            record: Job record to serialize and persist.
        """
        with self._lock:
            idempotency_key = (record.idempotency_key or "").strip() or None
            record_json = json.dumps(obj=record.to_dict(), ensure_ascii=False)
            updated_at = record.completed_at or record.cancelled_at or time.time()
            row_values = (
                record.job_id,
                record.task_type,
                record.status,
                record.outcome_state,
                idempotency_key,
                record.workspace_dir,
                record.created_at,
                updated_at,
                record_json,
            )
            try:
                with self._connect() as conn, conn:
                    SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.JOB_STORE,
                        query_id=JobSqlQueryId.UPSERT_JOB,
                        params=row_values,
                    )
                    self._write_count += 1
                    if self._write_count % 64 == 0:
                        evicted = SQL_LOADER.execute_safe(
                            conn=conn,
                            feature_file=SqlFeatureFile.JOB_STORE,
                            query_id=JobSqlQueryId.PRUNE_COMPLETED_JOBS,
                            params=(self.max_retained_jobs,),
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
                        evicted = SQL_LOADER.execute_safe(
                            conn=conn,
                            feature_file=SqlFeatureFile.JOB_STORE,
                            query_id=JobSqlQueryId.PRUNE_COMPLETED_JOBS,
                            params=(keep,),
                        ).fetchall()
                        for row in evicted:
                            self._jobs.pop(str(row[0]), None)
                    if wal_checkpoint:
                        SQL_LOADER.execute_safe(
                            conn=conn,
                            feature_file=SqlFeatureFile.JOB_STORE,
                            query_id=JobSqlQueryId.PRAGMA_WAL_CHECKPOINT,
                        )
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
                    row = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.JOB_STORE,
                        query_id=JobSqlQueryId.SELECT_JOB_BY_ID,
                        params=(job_id,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None or not isinstance(row[0], str):
                return None
            return CodexJobRecord.from_json(raw_json=row[0])


class _JobQueryStore(_JobStorageEngine):
    """Read and lookup operations across cached and SQLite-persisted job records."""

    def _save_job(self, record: CodexJobRecord) -> CodexJobRecord:
        """Persist a job record to SQLite and update the in-memory LRU cache.

        Args:
            record: Job record to persist and cache.

        Returns:
            Persisted and cached CodexJobRecord.
        """
        self._persist(record=record)
        self._cache_put(record=record)
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
                self._jobs.move_to_end(key=job_id)
                return cached
            if (record := self.get_persisted_job(job_id=job_id)) is not None:
                self._cache_put(record=record)
            return record

    def get_verified_persisted_job(self, job_id: str) -> CodexJobRecord | None:
        """Load a job record from SQLite and verify it does not conflict with the in-memory cache.

        Args:
            job_id: Unique job identifier to load and verify.

        Returns:
            Verified CodexJobRecord refreshed in cache, or None if missing or diverged.
        """
        with self._lock:
            in_memory = self._jobs.get(job_id)
            record = self.get_persisted_job(job_id=job_id)
            if record is None or (in_memory is not None and in_memory != record):
                return None
            self._cache_put(record=record)
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
                    if (
                        self.db_path.is_file()
                        and (fresh := self.get_persisted_job(job_id=jid)) is not None
                    ):
                        self._cache_put(record=fresh)
                        return fresh
                    self._jobs.move_to_end(key=jid)
                    return cached
            try:
                with self._connect() as conn:
                    row = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.JOB_STORE,
                        query_id=JobSqlQueryId.SELECT_JOB_BY_IDEMPOTENCY_KEY,
                        params=(key,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None:
                return None
            if (
                isinstance(row[1], str)
                and (rec := CodexJobRecord.from_json(raw_json=row[1])) is not None
            ):
                self._cache_put(record=rec)
                return rec
            return None

    def list_jobs(
        self,
        limit: int = 20,
        workspace_dir: str | None = None,
    ) -> list[CodexJobRecordDict]:
        """List recent job records ordered by creation timestamp descending.

        Args:
            limit: Maximum number of serialized job records to return.
            workspace_dir: Optional workspace directory path filter.

        Returns:
            List of serialized CodexJobRecordDict items.
        """
        ws = (
            WorkspaceDigestCalculator().resolve_dir(workspace_dir=workspace_dir)
            if workspace_dir
            else None
        )
        bounded_limit = max(1, limit)
        with self._lock:
            seen: dict[str, CodexJobRecord] = {
                jid: rec
                for jid, rec in self._jobs.items()
                if ws is None or matches_workspace(record_ws=rec.workspace_dir, ws_filter=ws)
            }
            try:
                with self._connect() as conn:
                    if ws is not None:
                        rows = SQL_LOADER.execute_safe(
                            conn=conn,
                            feature_file=SqlFeatureFile.JOB_STORE,
                            query_id=JobSqlQueryId.SELECT_JOBS_BY_WORKSPACE,
                            params=(ws, bounded_limit),
                        ).fetchall()
                        if len(rows) < bounded_limit:
                            rows.extend(
                                SQL_LOADER.execute_safe(
                                    conn=conn,
                                    feature_file=SqlFeatureFile.JOB_STORE,
                                    query_id=JobSqlQueryId.SELECT_JOBS_EXCLUDING_WORKSPACE,
                                    params=(ws, bounded_limit * 4),
                                ).fetchall()
                            )
                    else:
                        rows = SQL_LOADER.execute_safe(
                            conn=conn,
                            feature_file=SqlFeatureFile.JOB_STORE,
                            query_id=JobSqlQueryId.SELECT_RECENT_JOBS,
                            params=(bounded_limit,),
                        ).fetchall()
                for row in rows:
                    jid, row_ws, raw_json = str(row[0]), str(row[1]), row[2]
                    if jid in seen or not isinstance(raw_json, str):
                        continue
                    if (
                        ws is not None
                        and row_ws != ws
                        and not matches_workspace(record_ws=row_ws, ws_filter=ws)
                    ):
                        continue
                    if (rec := CodexJobRecord.from_json(raw_json=raw_json)) is not None:
                        seen[jid] = rec
            except sqlite3.Error:
                pass
            records = sorted(seen.values(), key=lambda r: r.created_at, reverse=True)
            return [r.to_dict() for r in records[:bounded_limit]]


class _JobTelemetryStore(_JobQueryStore):
    """Telemetry aggregation, orphan running-job reconciliation, and stage tracking."""

    def reconcile_stale_running_jobs(self, stale_age_seconds: float = 3600.0) -> int:
        """Reconcile orphaned `running` jobs whose owner PID is dead into `timed_out_unknown`.

        Args:
            stale_age_seconds: Fallback age threshold for legacy running rows without `owner_pid`.

        Returns:
            Number of orphaned running job records transitioned to `timed_out_unknown`.
        """
        with self._lock:
            if not self.db_path.is_file():
                return 0
            candidates: dict[str, CodexJobRecord] = {
                jid: rec for jid, rec in self._jobs.items() if rec.status == JobStatusKind.RUNNING
            }
            try:
                with self._connect() as conn:
                    rows = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.JOB_STORE,
                        query_id=JobSqlQueryId.SELECT_JOBS_BY_STATUS,
                        params=(str(JobStatusKind.RUNNING),),
                    ).fetchall()
                for row in rows:
                    jid, raw_json = str(row[0]), row[1]
                    if jid not in candidates and isinstance(raw_json, str):
                        if (rec := CodexJobRecord.from_json(raw_json=raw_json)) is not None:
                            candidates[jid] = rec
            except sqlite3.Error:
                return 0

            now = time.time()
            reconciled = 0
            for rec in list(candidates.values()):
                if rec.status != JobStatusKind.RUNNING:
                    continue
                pid_str = rec.metadata.get("owner_pid", "").strip()
                if pid_str.lstrip("-").isdigit():
                    is_orphan = not _is_pid_alive(pid=int(pid_str))
                else:
                    is_orphan = (now - rec.created_at) >= stale_age_seconds
                if not is_orphan:
                    continue
                err_msg = (
                    rec.error
                    or "Orphaned running job from terminated server process reconciled as timed_out_unknown."
                )
                updated = replace(
                    rec,
                    status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                    outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
                    completed_at=now,
                    error=err_msg,
                )
                self._save_job(record=updated)
                reconciled += 1
            return reconciled

    def update_job_stage(self, job_id: str, stage: str) -> CodexJobRecord | None:
        """Update `metadata['current_stage']` on an in-flight job record.

        Args:
            job_id: Unique job identifier to update.
            stage: Current execution stage label.

        Returns:
            Updated CodexJobRecord if found and running, otherwise the current record or None.
        """
        with self._lock:
            if (
                rec := (self.get_persisted_job(job_id=job_id) or self.get_job(job_id=job_id))
            ) is None:
                return None
            if rec.status != JobStatusKind.RUNNING or rec.cancel_requested:
                self._cache_put(record=rec)
                return rec
            meta = dict(rec.metadata)
            meta["current_stage"] = stage
            return self._save_job(record=replace(rec, metadata=meta))

    def summarize_telemetry(self) -> JobTelemetrySummaryDict:
        """Compute aggregated execution telemetry across all persisted SQLite job rows.

        Returns:
            JobTelemetrySummaryDict containing counts by status and average duration.
        """
        with self._lock:
            empty = JobTelemetrySummary(
                total_jobs=0,
                completed_jobs=0,
                failed_jobs=0,
                timed_out_unknown_jobs=0,
                running_jobs=0,
                cancelled_jobs=0,
                avg_duration_seconds=0.0,
            ).to_dict()
            if not self.db_path.is_file():
                return empty
            try:
                with self._connect() as conn:
                    row = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.JOB_STORE,
                        query_id=JobSqlQueryId.SUMMARIZE_JOB_TELEMETRY,
                    ).fetchone()
            except sqlite3.Error:
                row = None
            if row is None:
                return empty
            return JobTelemetrySummary(
                total_jobs=int(row[0] or 0),
                completed_jobs=int(row[1] or 0),
                failed_jobs=int(row[2] or 0),
                timed_out_unknown_jobs=int(row[3] or 0),
                running_jobs=int(row[4] or 0),
                cancelled_jobs=int(row[5] or 0),
                avg_duration_seconds=round(float(row[6] or 0.0), 2),
            ).to_dict()
