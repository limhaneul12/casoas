"""SQLite-backed (WAL) background task, durable receipt, and idempotency tracker."""

from __future__ import annotations

import contextlib
import json
import sqlite3
import threading
import time
import uuid
from collections import OrderedDict
from dataclasses import dataclass, field, replace
from pathlib import Path

from codex_collab_mcp.collab_enums import JobStatusKind, OutcomeStateKind
from codex_collab_mcp.domain.contract_dicts import CodexJobRecordDict
from codex_collab_mcp.schemas import JsonObject, JsonValue

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


@dataclass(frozen=True, slots=True)
class CodexJobRecord:
    """Durable state record for synchronous and asynchronous collaboration requests."""

    job_id: str
    task_type: str
    status: str
    created_at: float
    workspace_dir: str
    model: str | None = None
    session_id: str | None = None
    completed_at: float | None = None
    result: JsonValue = None
    error: str | None = None
    metadata: dict[str, str] = field(default_factory=dict)
    outcome_state: str = OutcomeStateKind.IN_FLIGHT
    idempotency_key: str | None = None
    provider: str | None = None
    engine: str | None = None
    attempt_count: int = 1
    cancel_requested: bool = False
    cancelled_at: float | None = None
    receipt: JsonObject | None = None

    def to_dict(self) -> CodexJobRecordDict:
        """Serialize the job record into a TypedDict payload.

        Returns:
            Serialized CodexJobRecordDict containing status, metadata, and elapsed time.
        """
        meta_json: JsonObject = dict(self.metadata.items())
        return CodexJobRecordDict(
            job_id=self.job_id,
            request_id=self.job_id,
            task_type=self.task_type,
            status=self.status,
            outcome_state=self.outcome_state,
            idempotency_key=self.idempotency_key,
            provider=self.provider,
            engine=self.engine,
            model=self.model,
            attempt_count=self.attempt_count,
            cancel_requested=self.cancel_requested,
            cancelled_at=self.cancelled_at,
            created_at=self.created_at,
            workspace_dir=self.workspace_dir,
            session_id=self.session_id,
            completed_at=self.completed_at,
            result=self.result,
            error=self.error,
            receipt=self.receipt,
            metadata=meta_json,
            elapsed_seconds=round((self.completed_at or time.time()) - self.created_at, 2),
        )


class _JobStorageEngine:
    """Low-level LRU cache and SQLite (WAL) persistence engine for CodexJobRecord items."""

    def __init__(
        self,
        state_dir: Path | None = None,
        max_cached_jobs: int = 512,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the storage engine without eagerly creating directories on disk.

        Args:
            state_dir: Optional directory path for the SQLite database file.
            max_cached_jobs: Maximum number of records retained in the in-memory LRU cache.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (Path.home() / ".cache" / "codex-collab-mcp" / "jobs")
        self.db_path: Path = self.state_dir / "jobs.db"
        self.max_cached_jobs: int = max(1, max_cached_jobs)
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()
        self._jobs: OrderedDict[str, CodexJobRecord] = OrderedDict()
        self._schema_ready: bool = False

    def ensure_db_initialized(self) -> Path:
        """Lazily create the backing directory and initialize the SQLite WAL schema.

        Returns:
            Ensured filesystem Path to the initialized `jobs.db` SQLite database.
        """
        with self._lock:
            self.state_dir.mkdir(parents=True, exist_ok=True)
            if not self._schema_ready or not self.db_path.is_file():
                with contextlib.closing(sqlite3.connect(self.db_path, timeout=10.0)) as conn:
                    conn.executescript(_JOBS_SCHEMA_SQL)
                self._schema_ready = True
            return self.db_path

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
            db_path = self.ensure_db_initialized()
            r, ik = record, (record.idempotency_key or "").strip() or None
            rj = json.dumps(r.to_dict(), ensure_ascii=False)
            ua = r.completed_at or r.cancelled_at or time.time()
            p1 = (r.job_id, r.task_type, r.status, r.outcome_state, ik)
            p2 = (r.workspace_dir, r.created_at, ua, rj)
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn, conn:
                    conn.execute("PRAGMA busy_timeout=5000")
                    conn.execute("INSERT OR REPLACE INTO jobs VALUES (?,?,?,?,?,?,?,?,?)", p1 + p2)
            except sqlite3.Error as exc:
                raise OSError(f"SQLite job persistence failed: {exc}") from exc

    def _deserialize_record(self, raw_json: str) -> CodexJobRecord | None:
        """Deserialize a CodexJobRecord from a stored JSON string.

        Args:
            raw_json: Serialized JSON string from the `record_json` column.

        Returns:
            Reconstructed CodexJobRecord if valid, otherwise None.
        """
        try:
            raw = json.loads(raw_json)
            if not isinstance(raw, dict):
                return None
            m = raw.get("metadata")
            md = m if isinstance(m, dict) else {}
            meta = {k: v for k, v in md.items() if isinstance(k, str) and isinstance(v, str)}
            c0, x0, o0 = raw.get("completed_at"), raw.get("cancelled_at"), raw.get("outcome_state")
            ca = float(c0) if isinstance(c0, (int, float)) else None
            xa = float(x0) if isinstance(x0, (int, float)) else None
            ost = o0 if isinstance(o0, str) else OutcomeStateKind.IN_FLIGHT
            m0, s0, e0 = raw.get("model"), raw.get("session_id"), raw.get("error")
            k0, p0, g0 = raw.get("idempotency_key"), raw.get("provider"), raw.get("engine")
            a0, r0 = raw.get("attempt_count"), raw.get("receipt")
            mod = m0 if isinstance(m0, str) else None
            sid = s0 if isinstance(s0, str) else None
            err, ik = (e0 if isinstance(e0, str) else None), (k0 if isinstance(k0, str) else None)
            prv, eng = (p0 if isinstance(p0, str) else None), (g0 if isinstance(g0, str) else None)
            att, rcp = (a0 if isinstance(a0, int) else 1), (r0 if isinstance(r0, dict) else None)
            j, t, s = str(raw["job_id"]), str(raw["task_type"]), str(raw["status"])
            b = CodexJobRecord(j, t, s, float(raw["created_at"]), str(raw["workspace_dir"]), mod)
            b = replace(b, session_id=sid, completed_at=ca, result=raw.get("result"), error=err)
            b = replace(b, metadata=meta, outcome_state=ost, idempotency_key=ik, provider=prv)
            cr = bool(raw.get("cancel_requested"))
            return replace(
                b, engine=eng, attempt_count=att, cancel_requested=cr, cancelled_at=xa, receipt=rcp
            )
        except json.JSONDecodeError, KeyError, TypeError, ValueError:
            return None

    def get_persisted_job(self, job_id: str) -> CodexJobRecord | None:
        """Read a job record directly from SQLite without consulting or mutating the cache.

        Args:
            job_id: Unique job identifier to load from SQLite.

        Returns:
            Persisted CodexJobRecord if present in SQLite, otherwise None.
        """
        with self._lock:
            db_path = self.ensure_db_initialized()
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn:
                    row = conn.execute(
                        "SELECT record_json FROM jobs WHERE job_id = ?",
                        (job_id,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None or not isinstance(row[0], str):
                return None
            return self._deserialize_record(row[0])


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
        """Find an existing job record by its idempotency_key via indexed SQLite lookup.

        Args:
            idempotency_key: Caller-supplied idempotency key string.

        Returns:
            Matching CodexJobRecord if found, otherwise None.
        """
        if not (key := idempotency_key.strip()):
            return None
        with self._lock:
            db_path = self.ensure_db_initialized()
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn:
                    row = conn.execute(
                        "SELECT job_id, record_json FROM jobs "
                        "WHERE idempotency_key = ? ORDER BY created_at DESC LIMIT 1",
                        (key,),
                    ).fetchone()
            except sqlite3.Error:
                return None
            if row is None:
                return None
            job_id = str(row[0])
            if (cached := self._jobs.get(job_id)) is not None and cached.idempotency_key == key:
                self._jobs.move_to_end(job_id)
                return cached
            if isinstance(row[1], str) and (rec := self._deserialize_record(row[1])) is not None:
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
        ws = str(Path(w).expanduser().resolve()) if (w := (workspace_dir or "").strip()) else None
        bounded_limit = max(1, limit)
        with self._lock:
            db_path = self.ensure_db_initialized()
            seen: dict[str, CodexJobRecord] = dict(self._jobs)
            try:
                with contextlib.closing(sqlite3.connect(db_path, timeout=10.0)) as conn:
                    rows = conn.execute(
                        "SELECT job_id, record_json FROM jobs ORDER BY created_at DESC LIMIT ?",
                        (bounded_limit * 4 if ws else bounded_limit * 2,),
                    ).fetchall()
                for row in rows:
                    jid = str(row[0])
                    if jid not in seen and isinstance(row[1], str):
                        if (rec := self._deserialize_record(row[1])) is not None:
                            seen[jid] = rec
            except sqlite3.Error:
                pass
            records = sorted(seen.values(), key=lambda r: r.created_at, reverse=True)
            if ws:
                records = [r for r in records if self._matches_workspace(r.workspace_dir, ws)]
            return [r.to_dict() for r in records[:bounded_limit]]

    @staticmethod
    def _matches_workspace(record_ws: str, ws_filter: str) -> bool:
        """Check whether a record's workspace directory matches the resolved filter path.

        Args:
            record_ws: Workspace directory string stored on the job record.
            ws_filter: Resolved absolute workspace directory filter string.

        Returns:
            True if the workspace paths match, otherwise False.
        """
        if record_ws == ws_filter:
            return True
        if not record_ws.strip() or Path(record_ws).is_absolute():
            return False
        return str(Path(record_ws).expanduser().resolve()) == ws_filter


class JobStore(_JobQueryStore):
    """Thread-safe in-memory and SQLite-backed job store for durable requests and timeouts."""

    def __init__(self, state_dir: Path | None = None, max_cached_jobs: int = 512) -> None:
        """Initialize the job store with an explicit reentrant lock and LRU cache limit.

        Args:
            state_dir: Optional directory path for the SQLite `jobs.db` file.
            max_cached_jobs: Maximum number of records retained in the in-memory LRU cache.
        """
        super().__init__(state_dir, max_cached_jobs, lock=threading.RLock())

    def create_job(
        self,
        task_type: str,
        workspace_dir: str,
        model: str | None = None,
        metadata: dict[str, str] | None = None,
        *,
        job_id: str | None = None,
        idempotency_key: str | None = None,
        provider: str | None = None,
        engine: str | None = None,
        created_at: float | None = None,
    ) -> CodexJobRecord:
        """Create, persist, and cache a new running job record.

        Args:
            task_type: Collaboration task category name.
            workspace_dir: Target workspace directory path.
            model: Optional requested model identifier.
            metadata: Optional string key-value metadata map.
            job_id: Optional explicit request/job identifier.
            idempotency_key: Optional caller idempotency key.
            provider: Optional provider identifier.
            engine: Optional execution engine identifier.
            created_at: Optional explicit job creation UNIX timestamp.

        Returns:
            Newly created and persisted CodexJobRecord.
        """
        with self._lock:
            rid = (job_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            ws = str(Path(w).expanduser().resolve()) if (w := workspace_dir.strip()) else ""
            ts, st = (created_at if created_at is not None else time.time()), JobStatusKind.RUNNING
            b = CodexJobRecord(rid, task_type, st, ts, ws or workspace_dir, model)
            md, ik = metadata or {}, idempotency_key
            rec = replace(b, metadata=md, idempotency_key=ik, provider=provider, engine=engine)
            return self._save_job(rec)

    def resolve_or_register_request(
        self,
        *,
        task_type: str,
        workspace_dir: str,
        model: str | None = None,
        provider: str | None = None,
        engine: str | None = None,
        request_id: str | None = None,
        idempotency_key: str | None = None,
        retry_same_request: bool = False,
        metadata: dict[str, str] | None = None,
    ) -> tuple[CodexJobRecord, bool]:
        """Resolve an existing request or create a new one without duplicate dispatch.

        Args:
            task_type: Collaboration task category name.
            workspace_dir: Target workspace directory path.
            model: Optional requested model identifier.
            provider: Optional provider identifier.
            engine: Optional execution engine identifier.
            request_id: Optional explicit request/job identifier.
            idempotency_key: Optional caller idempotency key.
            retry_same_request: Whether to re-dispatch an existing non-running request.
            metadata: Optional string key-value metadata map.

        Returns:
            Tuple of (CodexJobRecord, should_dispatch).
        """
        with self._lock:
            existing = self.get_job(rid) if (rid := (request_id or "").strip()) else None
            if existing is None and (ikey := (idempotency_key or "").strip()):
                existing = self.find_by_idempotency_key(ikey)
            if existing is not None:
                if existing.status == JobStatusKind.RUNNING or existing.cancel_requested:
                    return (existing, False)
                done = (JobStatusKind.TIMED_OUT_UNKNOWN, JobStatusKind.COMPLETED)
                if not retry_same_request and existing.status in done:
                    return (existing, False)
                att = existing.attempt_count + 1
                st, ost = JobStatusKind.RUNNING, OutcomeStateKind.IN_FLIGHT
                u = replace(existing, attempt_count=att, status=st, outcome_state=ost)
                return (self._save_job(replace(u, error=None, completed_at=None)), True)
            rec = self.create_job(
                task_type,
                workspace_dir,
                model,
                metadata,
                job_id=request_id,
                idempotency_key=idempotency_key,
                provider=provider,
                engine=engine,
            )
            return (rec, True)

    def complete_job(
        self,
        job_id: str,
        result: JsonValue,
        session_id: str | None = None,
        receipt: JsonObject | None = None,
    ) -> CodexJobRecord | None:
        """Mark an existing job record as completed with its output result and receipt.

        Args:
            job_id: Unique job identifier to complete.
            result: Structured or raw JSON output value.
            session_id: Optional provider session or thread identifier.
            receipt: Optional execution receipt dictionary.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (rec := self.get_job(job_id)) is None:
                return None
            sid = session_id or rec.session_id
            rcpt = receipt if receipt is not None else rec.receipt
            st, ost = JobStatusKind.COMPLETED, OutcomeStateKind.CONFIRMED_SUCCESS
            r1 = replace(rec, status=st, outcome_state=ost, completed_at=time.time())
            return self._save_job(replace(r1, result=result, session_id=sid, receipt=rcpt))

    def fail_job(
        self,
        job_id: str,
        error: str,
        session_id: str | None = None,
        receipt: JsonObject | None = None,
    ) -> CodexJobRecord | None:
        """Mark an existing job record as failed with an error message and receipt.

        Args:
            job_id: Unique job identifier to fail.
            error: Redacted error description string.
            session_id: Optional provider session or thread identifier.
            receipt: Optional execution receipt dictionary.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (rec := self.get_job(job_id)) is None:
                return None
            sid = session_id or rec.session_id
            rcpt = receipt if receipt is not None else rec.receipt
            st, ost = JobStatusKind.FAILED, OutcomeStateKind.CONFIRMED_FAILURE
            r1 = replace(rec, status=st, outcome_state=ost, completed_at=time.time())
            return self._save_job(replace(r1, error=error, session_id=sid, receipt=rcpt))

    def mark_timed_out_unknown(
        self,
        job_id: str,
        error: str,
        partial_result: JsonValue = None,
        session_id: str | None = None,
        receipt: JsonObject | None = None,
    ) -> CodexJobRecord | None:
        """Persist a provider read/execution timeout as timed_out_unknown (outcome_unknown).

        Args:
            job_id: Unique job identifier that timed out.
            error: Timeout error description string.
            partial_result: Optional partial output captured before timeout.
            session_id: Optional provider session or thread identifier.
            receipt: Optional execution receipt dictionary.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (rec := self.get_job(job_id)) is None:
                return None
            sid = session_id or rec.session_id
            rcpt = receipt if receipt is not None else rec.receipt
            res_val = partial_result if partial_result is not None else rec.result
            st, ost = JobStatusKind.TIMED_OUT_UNKNOWN, OutcomeStateKind.OUTCOME_UNKNOWN
            r1 = replace(rec, status=st, outcome_state=ost, completed_at=time.time())
            return self._save_job(
                replace(r1, error=error, result=res_val, session_id=sid, receipt=rcpt)
            )

    def cancel_job(
        self,
        job_id: str,
        reason: str = "Cancelled by caller; further retries blocked.",
    ) -> CodexJobRecord | None:
        """Safely mark an in-flight or timed_out_unknown job as cancelled.

        Args:
            job_id: Unique job identifier to cancel.
            reason: Cancellation reason recorded on the job error field.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (record := self.get_job(job_id)) is None:
                return None
            ts = time.time()
            updated = replace(record, cancel_requested=True, cancelled_at=ts)
            if record.status in (JobStatusKind.RUNNING, JobStatusKind.TIMED_OUT_UNKNOWN):
                st, ost = JobStatusKind.CANCELLED, OutcomeStateKind.CANCELLED
                updated = replace(
                    updated, status=st, outcome_state=ost, completed_at=ts, error=reason
                )
            return self._save_job(updated)
