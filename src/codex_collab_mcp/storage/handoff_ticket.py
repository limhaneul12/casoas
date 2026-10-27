"""Immutable HandoffTicket dataclass and TypedDict/JSON serialization."""

from __future__ import annotations

import json
from dataclasses import dataclass

from codex_collab_mcp.collab_enums import HandoffStatusKind, RoleExecutionState
from codex_collab_mcp.routing.contract_dicts import HandoffTicketDict


def _map_status_to_role_state(status: str) -> str:
    """Map a handoff ticket status string to its corresponding RoleExecutionState.

    Args:
        status: Handoff ticket status string.

    Returns:
        Corresponding RoleExecutionState string value.
    """
    if status == HandoffStatusKind.IN_PROGRESS:
        return str(RoleExecutionState.RUNNING)
    if status == HandoffStatusKind.COMPLETED:
        return str(RoleExecutionState.SUCCEEDED)
    if status == HandoffStatusKind.REJECTED:
        return str(RoleExecutionState.FAILED)
    return str(RoleExecutionState.QUEUED)


@dataclass(frozen=True, slots=True)
class HandoffTicket:
    """Durable handoff ticket exchanged between Argon, Sonnet/Opus, and Codex."""

    ticket_id: str
    sender: str
    recipient: str
    title: str
    spec_or_payload: str
    workspace_dir: str
    priority: str
    status: str
    created_at: float
    updated_at: float
    resolution_note: str | None = None
    alexandria_ref: str | None = None

    def to_dict(self) -> HandoffTicketDict:
        """Serialize the handoff ticket into a TypedDict payload.

        Returns:
            Serialized HandoffTicketDict with derived collaboration role state flags.
        """
        collab_role_state = _map_status_to_role_state(self.status)
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

    @classmethod
    def from_json(cls, raw_json: str) -> HandoffTicket | None:
        """Deserialize a HandoffTicket from a stored JSON string.

        Args:
            raw_json: Serialized JSON string from the `ticket_json` SQLite column.

        Returns:
            Reconstructed HandoffTicket if valid, otherwise None.
        """
        try:
            raw = json.loads(raw_json)
            if not isinstance(raw, dict):
                return None
            res_note = raw.get("resolution_note")
            alex_ref = raw.get("alexandria_ref")
            return cls(
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
                resolution_note=res_note if isinstance(res_note, str) else None,
                alexandria_ref=alex_ref if isinstance(alex_ref, str) else None,
            )
        except json.JSONDecodeError, KeyError, TypeError, ValueError:
            return None
