"""Pydantic v2 external boundary schemas (`StrictSchemaModel`) for `heterarchy_dag_ulw`."""

from __future__ import annotations

from pydantic import Field

from app.core.dag_enums import DagIsolationMode, DagNodeKind, DagNodeRoleKind
from app.schemas.boundary_schemas import StrictSchemaModel


class DagNodeInputModel(StrictSchemaModel):
    """External I/O specification for a single node in a `heterarchy_dag_ulw` graph."""

    node_id: str = Field(min_length=1, max_length=64)
    role: DagNodeRoleKind = DagNodeRoleKind.DISJOINT_BUILDER
    node_kind: DagNodeKind = DagNodeKind.STANDARD
    task: str = Field(min_length=1)
    deliverable: str = Field(default="Verified implementation or review output")
    read_files: tuple[str, ...] = ()
    write_files: tuple[str, ...] = ()
    symbols: tuple[str, ...] = ()
    depends_on: tuple[str, ...] = ()
    verify_cmd: str = ""
    stop_when: str = "Deliverable and verification criteria are satisfied within scope."
    preferred_model: str | None = None
    reasoning_effort: str | None = None
    reproduction_expected_exit_nonzero: bool = True


class DagSpawnPlanInputModel(StrictSchemaModel):
    """Explicit spawn plan gate acknowledgment required before multi-node DAG execution."""

    plan_summary: str = Field(min_length=1)
    hazard_acknowledgment: bool = True
    isolation_mode: DagIsolationMode = DagIsolationMode.DIRECT
    max_parallel_per_wave: int = Field(default=3, ge=1, le=6)


class DagUlwRequestModel(StrictSchemaModel):
    """External I/O request schema for `heterarchy_dag_ulw`."""

    campaign_goal: str = Field(min_length=1)
    nodes: tuple[DagNodeInputModel, ...] = ()
    workspace_dir: str = "."
    plan_only: bool = True
    spawn_plan: DagSpawnPlanInputModel | None = None
    resume_dag_id: str | None = None
    timeout_seconds: float = Field(default=300.0, gt=0.0, le=3600.0)
    max_input_bytes: int = Field(default=256_000, ge=1024, le=2_000_000)
    request_id: str | None = None
    idempotency_key: str | None = None


__all__ = [
    "DagNodeInputModel",
    "DagSpawnPlanInputModel",
    "DagUlwRequestModel",
]
