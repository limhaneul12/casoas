"""Immutable CodexJobRecord dataclass and TypedDict/JSON serialization."""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, field

from codex_collab_mcp.collab_enums import OutcomeStateKind
from codex_collab_mcp.routing.contract_dicts import CodexJobRecordDict, ExecutionReceiptDict
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
    receipt: ExecutionReceiptDict | JsonObject | None = None

    def to_dict(self) -> CodexJobRecordDict:
        """Serialize the job record into a TypedDict payload.

        Returns:
            Serialized CodexJobRecordDict containing status, metadata, and elapsed time.
        """
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
            metadata=dict(self.metadata),
            elapsed_seconds=round((self.completed_at or time.time()) - self.created_at, 2),
        )

    @classmethod
    def from_json(cls, raw_json: str) -> CodexJobRecord | None:
        """Deserialize a CodexJobRecord from a stored JSON string.

        Args:
            raw_json: Serialized JSON string from the `record_json` SQLite column.

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
            err = e0 if isinstance(e0, str) else None
            ik = k0 if isinstance(k0, str) else None
            prv = p0 if isinstance(p0, str) else None
            eng = g0 if isinstance(g0, str) else None
            att = a0 if isinstance(a0, int) else 1
            rcp = r0 if isinstance(r0, dict) else None
            return cls(
                job_id=str(raw["job_id"]),
                task_type=str(raw["task_type"]),
                status=str(raw["status"]),
                created_at=float(raw["created_at"]),
                workspace_dir=str(raw["workspace_dir"]),
                model=mod,
                session_id=sid,
                completed_at=ca,
                result=raw.get("result"),
                error=err,
                metadata=meta,
                outcome_state=ost,
                idempotency_key=ik,
                provider=prv,
                engine=eng,
                attempt_count=att,
                cancel_requested=bool(raw.get("cancel_requested")),
                cancelled_at=xa,
                receipt=rcp,
            )
        except json.JSONDecodeError, KeyError, TypeError, ValueError:
            return None
