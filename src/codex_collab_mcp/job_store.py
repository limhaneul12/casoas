"""Background task, durable request receipt, and idempotency tracker for Heterarchy jobs."""

from __future__ import annotations

import json
import os
import threading
import time
import uuid
from collections import OrderedDict
from dataclasses import dataclass, field, replace
from pathlib import Path

from codex_collab_mcp.collab_enums import JobStatusKind, OutcomeStateKind
from codex_collab_mcp.domain.contract_dicts import CodexJobRecordDict
from codex_collab_mcp.schemas import JsonObject, JsonValue


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
    """Low-level LRU cache and atomic disk persistence engine for CodexJobRecord items."""

    def __init__(
        self,
        state_dir: Path | None = None,
        max_cached_jobs: int = 512,
        lock: threading.RLock | None = None,
    ) -> None:
        """Initialize the storage engine without eagerly creating directories on disk.

        Args:
            state_dir: Optional directory path for persisted job JSON files.
            max_cached_jobs: Maximum number of records retained in the in-memory LRU cache.
            lock: Optional shared reentrant lock from a subclass.
        """
        self.state_dir: Path = state_dir or (Path.home() / ".cache" / "codex-collab-mcp" / "jobs")
        self.max_cached_jobs: int = max(1, max_cached_jobs)
        self._lock: threading.RLock = lock if lock is not None else threading.RLock()
        self._jobs: OrderedDict[str, CodexJobRecord] = OrderedDict()
        self._idempotency_index: dict[str, str] = {}
        self._index_loaded: bool = False

    def _ensure_state_dir(self) -> Path:
        """Lazily create and return the backing directory for job persistence.

        Returns:
            Ensured filesystem Path for job record storage.
        """
        self.state_dir.mkdir(parents=True, exist_ok=True)
        return self.state_dir

    def _ensure_index_loaded(self) -> None:
        """Lazily load the persisted `.idempotency_index.json` map from disk into memory."""
        if self._index_loaded:
            return
        self._index_loaded = True
        idx_path = self.state_dir / ".idempotency_index.json"
        if not idx_path.is_file():
            return
        try:
            raw = json.loads(idx_path.read_text(encoding="utf-8"))
            if isinstance(raw, dict):
                for k, v in raw.items():
                    if (
                        isinstance(k, str)
                        and isinstance(v, str)
                        and k not in self._idempotency_index
                    ):
                        self._idempotency_index[k] = v
        except OSError, json.JSONDecodeError, TypeError, ValueError:
            return

    def _cache_put(self, record: CodexJobRecord) -> None:
        """Insert or refresh a job record in the bounded in-memory LRU cache.

        Args:
            record: Job record to cache and index by idempotency key.
        """
        self._jobs[record.job_id] = record
        self._jobs.move_to_end(record.job_id)
        if record.idempotency_key and record.idempotency_key.strip():
            self._idempotency_index[record.idempotency_key.strip()] = record.job_id
        while len(self._jobs) > self.max_cached_jobs:
            self._jobs.popitem(last=False)

    def _persist(self, record: CodexJobRecord) -> None:
        """Atomically write a job record and its idempotency index entry to disk.

        Args:
            record: Job record to serialize and persist.
        """
        with self._lock:
            state_dir = self._ensure_state_dir()
            self._ensure_index_loaded()
            uid = uuid.uuid4().hex[:6]
            tmp_path = state_dir / f".{record.job_id}.{uid}.tmp"
            tmp_path.write_text(
                json.dumps(record.to_dict(), ensure_ascii=False, indent=2), encoding="utf-8"
            )
            os.replace(tmp_path, state_dir / f"{record.job_id}.json")
            if record.idempotency_key and (ikey := record.idempotency_key.strip()):
                self._idempotency_index[ikey] = record.job_id
                idx_tmp = state_dir / f".idempotency_index.{uid}.tmp"
                idx_tmp.write_text(
                    json.dumps(self._idempotency_index, ensure_ascii=False, sort_keys=True),
                    encoding="utf-8",
                )
                os.replace(idx_tmp, state_dir / ".idempotency_index.json")

    def _load_from_disk(self, path: Path) -> CodexJobRecord | None:
        """Deserialize a job record from a JSON file on disk.

        Args:
            path: Filesystem path to the persisted JSON file.

        Returns:
            Reconstructed CodexJobRecord if valid and readable, otherwise None.
        """
        if not path.exists():
            return None
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
            if not isinstance(raw, dict):
                return None
            metadata: dict[str, str] = (
                {k: v for k, v in m.items() if isinstance(k, str) and isinstance(v, str)}
                if isinstance(m := raw.get("metadata"), dict)
                else {}
            )
            comp, canc = raw.get("completed_at"), raw.get("cancelled_at")
            return CodexJobRecord(
                job_id=str(raw["job_id"]),
                task_type=str(raw["task_type"]),
                status=str(raw["status"]),
                created_at=float(raw["created_at"]),
                workspace_dir=str(raw["workspace_dir"]),
                model=v if isinstance(v := raw.get("model"), str) else None,
                session_id=v if isinstance(v := raw.get("session_id"), str) else None,
                completed_at=float(comp) if isinstance(comp, (int, float)) else None,
                result=raw.get("result"),
                error=v if isinstance(v := raw.get("error"), str) else None,
                metadata=metadata,
                outcome_state=(
                    v
                    if isinstance(v := raw.get("outcome_state"), str)
                    else OutcomeStateKind.IN_FLIGHT
                ),
                idempotency_key=v if isinstance(v := raw.get("idempotency_key"), str) else None,
                provider=v if isinstance(v := raw.get("provider"), str) else None,
                engine=v if isinstance(v := raw.get("engine"), str) else None,
                attempt_count=v if isinstance(v := raw.get("attempt_count"), int) else 1,
                cancel_requested=bool(raw.get("cancel_requested")),
                cancelled_at=float(canc) if isinstance(canc, (int, float)) else None,
                receipt=v if isinstance(v := raw.get("receipt"), dict) else None,
            )
        except OSError, json.JSONDecodeError, KeyError, TypeError, ValueError:
            return None

    def get_persisted_job(self, job_id: str) -> CodexJobRecord | None:
        """Read a job record directly from disk without consulting or mutating the cache.

        Args:
            job_id: Unique job identifier to load from disk.

        Returns:
            Persisted CodexJobRecord if present on disk, otherwise None.
        """
        with self._lock:
            return self._load_from_disk(self._ensure_state_dir() / f"{job_id}.json")


class _JobQueryStore(_JobStorageEngine):
    """Read and lookup operations across cached and persisted job records."""

    def _save_job(self, record: CodexJobRecord) -> CodexJobRecord:
        """Persist a job record to disk and update the in-memory LRU cache.

        Args:
            record: Job record to persist and cache.

        Returns:
            Persisted and cached CodexJobRecord.
        """
        self._persist(record)
        self._cache_put(record)
        return record

    def get_job(self, job_id: str) -> CodexJobRecord | None:
        """Retrieve a job record by ID from the LRU cache or disk fallback.

        Args:
            job_id: Unique job identifier.

        Returns:
            Matching CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (cached := self._jobs.get(job_id)) is not None:
                self._jobs.move_to_end(job_id)
                return cached
            if (
                record := self._load_from_disk(self._ensure_state_dir() / f"{job_id}.json")
            ) is not None:
                self._cache_put(record)
            return record

    def find_by_idempotency_key(self, idempotency_key: str) -> CodexJobRecord | None:
        """Find an existing job record by its idempotency_key across memory and disk.

        Args:
            idempotency_key: Caller-supplied idempotency key string.

        Returns:
            Matching CodexJobRecord if found, otherwise None.
        """
        if not (key := idempotency_key.strip()):
            return None
        with self._lock:
            self._ensure_index_loaded()
            if (mapped_id := self._idempotency_index.get(key)) is not None:
                if (rec := self.get_job(mapped_id)) is not None and rec.idempotency_key == key:
                    return rec
            paths = (
                p for p in self._ensure_state_dir().glob("*.json") if not p.name.startswith(".")
            )
            for path in sorted(paths, key=lambda p: p.stat().st_mtime, reverse=True):
                if path.stem not in self._jobs and (rec := self.get_job(path.stem)) is not None:
                    if rec.idempotency_key == key:
                        self._idempotency_index[key] = rec.job_id
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
        with self._lock:
            seen: dict[str, CodexJobRecord] = dict(self._jobs)
            paths = (
                p for p in self._ensure_state_dir().glob("*.json") if not p.name.startswith(".")
            )
            for path in sorted(paths, key=lambda p: p.stat().st_mtime, reverse=True)[: limit * 2]:
                if path.stem not in seen and (rec := self.get_job(path.stem)) is not None:
                    seen[path.stem] = rec
            records = sorted(seen.values(), key=lambda r: r.created_at, reverse=True)
            if ws:
                records = [r for r in records if self._matches_workspace(r.workspace_dir, ws)]
            return [r.to_dict() for r in records[:limit]]

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
    """Thread-safe in-memory and disk-backed job store for durable requests and timeouts."""

    def __init__(self, state_dir: Path | None = None, max_cached_jobs: int = 512) -> None:
        """Initialize the job store with an explicit reentrant lock and LRU cache limit.

        Args:
            state_dir: Optional directory path for persisted job JSON files.
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
            resolved_id = (job_id or "").strip() or f"cdx-{uuid.uuid4().hex[:10]}"
            ws = (
                str(Path(w).expanduser().resolve())
                if (w := workspace_dir.strip())
                else workspace_dir
            )
            return self._save_job(
                CodexJobRecord(
                    job_id=resolved_id,
                    task_type=task_type,
                    status=JobStatusKind.RUNNING,
                    created_at=created_at if created_at is not None else time.time(),
                    workspace_dir=ws,
                    model=model,
                    metadata=metadata or {},
                    outcome_state=OutcomeStateKind.IN_FLIGHT,
                    idempotency_key=idempotency_key,
                    provider=provider,
                    engine=engine,
                )
            )

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
                if not retry_same_request and existing.status in (
                    JobStatusKind.TIMED_OUT_UNKNOWN,
                    JobStatusKind.COMPLETED,
                ):
                    return (existing, False)
                updated = replace(
                    existing,
                    attempt_count=existing.attempt_count + 1,
                    status=JobStatusKind.RUNNING,
                    outcome_state=OutcomeStateKind.IN_FLIGHT,
                    error=None,
                    completed_at=None,
                )
                return (self._save_job(updated), True)
            record = self.create_job(
                task_type,
                workspace_dir,
                model,
                metadata,
                job_id=request_id,
                idempotency_key=idempotency_key,
                provider=provider,
                engine=engine,
            )
            return (record, True)

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
            return self._save_job(
                replace(
                    rec,
                    status=JobStatusKind.COMPLETED,
                    outcome_state=OutcomeStateKind.CONFIRMED_SUCCESS,
                    completed_at=time.time(),
                    result=result,
                    session_id=session_id or rec.session_id,
                    receipt=receipt if receipt is not None else rec.receipt,
                )
            )

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
            return self._save_job(
                replace(
                    rec,
                    status=JobStatusKind.FAILED,
                    outcome_state=OutcomeStateKind.CONFIRMED_FAILURE,
                    completed_at=time.time(),
                    error=error,
                    session_id=session_id or rec.session_id,
                    receipt=receipt if receipt is not None else rec.receipt,
                )
            )

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
            return self._save_job(
                replace(
                    rec,
                    status=JobStatusKind.TIMED_OUT_UNKNOWN,
                    outcome_state=OutcomeStateKind.OUTCOME_UNKNOWN,
                    completed_at=time.time(),
                    error=error,
                    result=partial_result if partial_result is not None else rec.result,
                    session_id=session_id or rec.session_id,
                    receipt=receipt if receipt is not None else rec.receipt,
                )
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
            cancelled_at = time.time()
            if record.status in (JobStatusKind.RUNNING, JobStatusKind.TIMED_OUT_UNKNOWN):
                updated = replace(
                    record,
                    cancel_requested=True,
                    cancelled_at=cancelled_at,
                    status=JobStatusKind.CANCELLED,
                    outcome_state=OutcomeStateKind.CANCELLED,
                    completed_at=cancelled_at,
                    error=reason,
                )
            else:
                updated = replace(record, cancel_requested=True, cancelled_at=cancelled_at)
            return self._save_job(updated)
