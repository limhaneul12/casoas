"""TypedDict return-type contracts for `dag_dtos.py` `.to_dict(self)` methods."""

from __future__ import annotations

from typing import TypedDict

from app.schemas.boundary_schemas import JsonObject


class DagHazardDict(TypedDict):
    """Serialized representation of a `DagHazard` dataclass."""

    code: str
    severity: str
    blocking: bool
    node_ids: list[str]
    conflicting_paths: list[str]
    conflicting_symbols: list[str]
    message: str


class DagWaveSpecDict(TypedDict):
    """Serialized representation of a `DagWaveSpec` dataclass."""

    wave_index: int
    node_ids: list[str]
    isolation_mode: str
    writer_count: int
    concurrency_limit: int


class DagCompiledPlanDict(TypedDict):
    """Serialized representation of a `DagCompiledPlan` dataclass."""

    dag_id: str
    campaign_goal: str
    plan_digest: str
    total_nodes: int
    total_waves: int
    waves: list[DagWaveSpecDict]
    hazards: list[DagHazardDict]
    blocking_hazards_count: int
    spawn_plan_acknowledged: bool
    ascii_graph: str


class DagNodeSettlementReceiptDict(TypedDict):
    """Serialized representation of a `DagNodeSettlementReceipt` dataclass."""

    node_id: str
    role: str
    node_kind: str
    wave_index: int
    settlement_stage: str
    model_id: str
    duration_seconds: float
    pre_manifest_digest: str
    post_manifest_digest: str
    mutated_files: list[str]
    out_of_scope_writes: list[str]
    dirty_read_files: list[str]
    verify_passed: bool
    reproduction_observed: bool
    output_summary: str
    error: str | None


class DagUlwResponseDict(TypedDict):
    """Serialized representation of a `DagUlwResponse` dataclass."""

    ok: bool
    dag_id: str
    run_state: str
    plan_only: bool
    workspace_dir: str
    compiled_plan: DagCompiledPlanDict
    node_receipts: list[DagNodeSettlementReceiptDict]
    settled_count: int
    resumed_nodes: list[str]
    aimd_final_concurrency: int
    wave_tree_view: str
    summary: str


class DagSnapshotPayloadDict(TypedDict):
    """Serialized representation of a `DagSnapshotPayload` dataclass."""

    dag_id: str
    plan_digest: str
    workspace_dir: str
    run_state: str
    node_receipts: list[JsonObject]


class TuiSimulatedExecResultDict(TypedDict):
    """Serialized representation of a `TuiSimulatedExecResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    model_id: str
    model: str
    raw_output: str
    data: str


class DagAgentPoleProfileDict(TypedDict):
    """Serialized representation of a `DagAgentPoleProfile` dataclass."""

    pole: str
    model_id: str
    reasoning_effort: str
    badge_icon: str
    ansi_color: str


class DagAgentEffectConfigDict(TypedDict):
    """Serialized representation of a `DagAgentEffectConfig` dataclass."""

    style: str
    argon: DagAgentPoleProfileDict
    claude: DagAgentPoleProfileDict
    codex: DagAgentPoleProfileDict


__all__ = [
    "DagAgentEffectConfigDict",
    "DagAgentPoleProfileDict",
    "DagCompiledPlanDict",
    "DagHazardDict",
    "DagNodeSettlementReceiptDict",
    "DagSnapshotPayloadDict",
    "DagUlwResponseDict",
    "DagWaveSpecDict",
    "TuiSimulatedExecResultDict",
]
