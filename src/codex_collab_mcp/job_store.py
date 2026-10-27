"""SQLite-backed (WAL) background task, durable receipt, and idempotency tracker."""

from __future__ import annotations

import threading
import time
import uuid
from dataclasses import replace
from pathlib import Path

from codex_collab_mcp.collab_enums import JobStatusKind, OutcomeStateKind
from codex_collab_mcp.routing.contract_dicts import ExecutionReceiptDict
from codex_collab_mcp.schemas import JsonObject, JsonValue
from codex_collab_mcp.storage.job_record import CodexJobRecord
from codex_collab_mcp.storage.sqlite_job_engine import _JobQueryStore


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
            ts = created_at if created_at is not None else time.time()
            rec = CodexJobRecord(
                job_id=rid,
                task_type=task_type,
                status=str(JobStatusKind.RUNNING),
                created_at=ts,
                workspace_dir=ws or workspace_dir,
                model=model,
                metadata=metadata or {},
                outcome_state=str(OutcomeStateKind.IN_FLIGHT),
                idempotency_key=idempotency_key,
                provider=provider,
                engine=engine,
            )
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
                updated = replace(
                    existing,
                    attempt_count=existing.attempt_count + 1,
                    status=str(JobStatusKind.RUNNING),
                    outcome_state=str(OutcomeStateKind.IN_FLIGHT),
                    error=None,
                    completed_at=None,
                )
                return (self._save_job(updated), True)
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
        receipt: ExecutionReceiptDict | JsonObject | None = None,
    ) -> CodexJobRecord | None:
        """Mark an existing job record as completed with its output result and receipt.

        Args:
            job_id: Unique job identifier to complete.
            result: Structured or raw JSON output value.
            session_id: Optional provider session or thread identifier.
            receipt: Optional execution receipt TypedDict or JSON object.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (rec := self.get_job(job_id)) is None:
                return None
            return self._save_job(
                replace(
                    rec,
                    status=str(JobStatusKind.COMPLETED),
                    outcome_state=str(OutcomeStateKind.CONFIRMED_SUCCESS),
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
        receipt: ExecutionReceiptDict | JsonObject | None = None,
    ) -> CodexJobRecord | None:
        """Mark an existing job record as failed with an error message and receipt.

        Args:
            job_id: Unique job identifier to fail.
            error: Redacted error description string.
            session_id: Optional provider session or thread identifier.
            receipt: Optional execution receipt TypedDict or JSON object.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (rec := self.get_job(job_id)) is None:
                return None
            return self._save_job(
                replace(
                    rec,
                    status=str(JobStatusKind.FAILED),
                    outcome_state=str(OutcomeStateKind.CONFIRMED_FAILURE),
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
        receipt: ExecutionReceiptDict | JsonObject | None = None,
    ) -> CodexJobRecord | None:
        """Persist a provider read/execution timeout as timed_out_unknown (outcome_unknown).

        Args:
            job_id: Unique job identifier that timed out.
            error: Timeout error description string.
            partial_result: Optional partial output captured before timeout.
            session_id: Optional provider session or thread identifier.
            receipt: Optional execution receipt TypedDict or JSON object.

        Returns:
            Updated CodexJobRecord if found, otherwise None.
        """
        with self._lock:
            if (rec := self.get_job(job_id)) is None:
                return None
            return self._save_job(
                replace(
                    rec,
                    status=str(JobStatusKind.TIMED_OUT_UNKNOWN),
                    outcome_state=str(OutcomeStateKind.OUTCOME_UNKNOWN),
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
            ts = time.time()
            updated = replace(record, cancel_requested=True, cancelled_at=ts)
            if record.status in (JobStatusKind.RUNNING, JobStatusKind.TIMED_OUT_UNKNOWN):
                updated = replace(
                    updated,
                    status=str(JobStatusKind.CANCELLED),
                    outcome_state=str(OutcomeStateKind.CANCELLED),
                    completed_at=ts,
                    error=reason,
                )
            return self._save_job(updated)


__all__ = ["CodexJobRecord", "JobStore"]
