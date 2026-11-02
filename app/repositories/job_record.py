"""Immutable CodexJobRecord dataclass and TypedDict/JSON serialization."""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, field

from app.core.collab_enums import OutcomeStateKind
from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import (
    CodexJobRecordDict,
    ExecutionReceiptDict,
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
            raw = json.loads(s=raw_json)
            if not isinstance(raw, dict):
                return None
            raw_meta = raw.get("metadata")
            meta_dict = raw_meta if isinstance(raw_meta, dict) else {}
            metadata = {
                k: v for k, v in meta_dict.items() if isinstance(k, str) and isinstance(v, str)
            }
            comp_raw = raw.get("completed_at")
            canc_raw = raw.get("cancelled_at")
            out_raw = raw.get("outcome_state")
            att_raw = raw.get("attempt_count")
            rcp_raw = raw.get("receipt")
            return cls(
                job_id=str(raw["job_id"]),
                task_type=str(raw["task_type"]),
                status=str(raw["status"]),
                created_at=float(raw["created_at"]),
                workspace_dir=str(raw["workspace_dir"]),
                model=val if isinstance(val := raw.get("model"), str) else None,
                session_id=val if isinstance(val := raw.get("session_id"), str) else None,
                completed_at=float(comp_raw) if isinstance(comp_raw, (int, float)) else None,
                result=raw.get("result"),
                error=val if isinstance(val := raw.get("error"), str) else None,
                metadata=metadata,
                outcome_state=out_raw if isinstance(out_raw, str) else OutcomeStateKind.IN_FLIGHT,
                idempotency_key=val if isinstance(val := raw.get("idempotency_key"), str) else None,
                provider=val if isinstance(val := raw.get("provider"), str) else None,
                engine=val if isinstance(val := raw.get("engine"), str) else None,
                attempt_count=att_raw if isinstance(att_raw, int) else 1,
                cancel_requested=bool(raw.get("cancel_requested")),
                cancelled_at=float(canc_raw) if isinstance(canc_raw, (int, float)) else None,
                receipt=rcp_raw if isinstance(rcp_raw, dict) else None,
            )
        except json.JSONDecodeError, KeyError, TypeError, ValueError:
            return None
