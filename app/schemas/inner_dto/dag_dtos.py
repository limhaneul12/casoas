"""Internal frozen dataclass DTOs for Tri-Polar Graph Engineering (`DAG-ULW++`)."""

from __future__ import annotations

from dataclasses import dataclass

from app.core.dag_enums import (
    DagHazardCode,
    DagIsolationMode,
    DagNodeKind,
    DagNodeRoleKind,
    DagRunState,
    DagSettlementStage,
    DagVisualEffectStyle,
)
from app.schemas.boundary_schemas import JsonObject
from app.schemas.inner_dto.dto_dict_types import (
    DagAgentEffectConfigDict,
    DagAgentPoleProfileDict,
    DagCompiledPlanDict,
    DagHazardDict,
    DagNodeSettlementReceiptDict,
    DagSnapshotPayloadDict,
    DagUlwResponseDict,
    DagWaveSpecDict,
    TuiSimulatedExecResultDict,
)


@dataclass(frozen=True, slots=True)
class DagHazard:
    """Internal DTO representing a detected WAW, RAW, symbol, or topology hazard."""

    code: DagHazardCode
    severity: str
    blocking: bool
    node_ids: tuple[str, ...]
    conflicting_paths: tuple[str, ...]
    conflicting_symbols: tuple[str, ...]
    message: str

    def to_dict(self) -> DagHazardDict:
        """Serialize the DAG hazard DTO to a TypedDict.

        Returns:
            Typed `DagHazardDict` mapping.
        """
        return DagHazardDict(
            code=str(self.code),
            severity=self.severity,
            blocking=self.blocking,
            node_ids=list(self.node_ids),
            conflicting_paths=list(self.conflicting_paths),
            conflicting_symbols=list(self.conflicting_symbols),
            message=self.message,
        )


@dataclass(frozen=True, slots=True)
class DagWaveSpec:
    """Internal DTO for a single execution wave in a compiled DAG."""

    wave_index: int
    node_ids: tuple[str, ...]
    isolation_mode: DagIsolationMode
    writer_count: int
    concurrency_limit: int

    def to_dict(self) -> DagWaveSpecDict:
        """Serialize the wave specification to a TypedDict.

        Returns:
            Typed `DagWaveSpecDict` mapping.
        """
        return DagWaveSpecDict(
            wave_index=self.wave_index,
            node_ids=list(self.node_ids),
            isolation_mode=str(self.isolation_mode),
            writer_count=self.writer_count,
            concurrency_limit=self.concurrency_limit,
        )


@dataclass(frozen=True, slots=True)
class DagCompiledPlan:
    """Internal DTO holding the compiled topological waves, hazards, and ASCII graph."""

    dag_id: str
    campaign_goal: str
    plan_digest: str
    total_nodes: int
    total_waves: int
    waves: tuple[DagWaveSpec, ...]
    hazards: tuple[DagHazard, ...]
    blocking_hazards_count: int
    spawn_plan_acknowledged: bool
    ascii_graph: str

    def to_dict(self) -> DagCompiledPlanDict:
        """Serialize the compiled DAG plan to a TypedDict.

        Returns:
            Typed `DagCompiledPlanDict` mapping.
        """
        return DagCompiledPlanDict(
            dag_id=self.dag_id,
            campaign_goal=self.campaign_goal,
            plan_digest=self.plan_digest,
            total_nodes=self.total_nodes,
            total_waves=self.total_waves,
            waves=[w.to_dict() for w in self.waves],
            hazards=[h.to_dict() for h in self.hazards],
            blocking_hazards_count=self.blocking_hazards_count,
            spawn_plan_acknowledged=self.spawn_plan_acknowledged,
            ascii_graph=self.ascii_graph,
        )


@dataclass(frozen=True, slots=True)
class DagNodeSettlementReceipt:
    """Internal DTO recording two-stage settlement (`STOPPED -> SETTLED`) for a DAG node."""

    node_id: str
    role: DagNodeRoleKind
    node_kind: DagNodeKind
    wave_index: int
    settlement_stage: DagSettlementStage
    model_id: str
    duration_seconds: float
    pre_manifest_digest: str
    post_manifest_digest: str
    mutated_files: tuple[str, ...]
    out_of_scope_writes: tuple[str, ...]
    dirty_read_files: tuple[str, ...]
    verify_passed: bool
    reproduction_observed: bool
    output_summary: str
    error: str | None = None

    def to_dict(self) -> DagNodeSettlementReceiptDict:
        """Serialize the node settlement receipt to a TypedDict.

        Returns:
            Typed `DagNodeSettlementReceiptDict` mapping.
        """
        return DagNodeSettlementReceiptDict(
            node_id=self.node_id,
            role=str(self.role),
            node_kind=str(self.node_kind),
            wave_index=self.wave_index,
            settlement_stage=str(self.settlement_stage),
            model_id=self.model_id,
            duration_seconds=self.duration_seconds,
            pre_manifest_digest=self.pre_manifest_digest,
            post_manifest_digest=self.post_manifest_digest,
            mutated_files=list(self.mutated_files),
            out_of_scope_writes=list(self.out_of_scope_writes),
            dirty_read_files=list(self.dirty_read_files),
            verify_passed=self.verify_passed,
            reproduction_observed=self.reproduction_observed,
            output_summary=self.output_summary,
            error=self.error,
        )


@dataclass(frozen=True, slots=True)
class DagUlwResponse:
    """Internal DTO for the complete `heterarchy_dag_ulw` response."""

    ok: bool
    dag_id: str
    run_state: DagRunState
    plan_only: bool
    workspace_dir: str
    compiled_plan: DagCompiledPlan
    node_receipts: tuple[DagNodeSettlementReceipt, ...]
    settled_count: int
    resumed_nodes: tuple[str, ...]
    aimd_final_concurrency: int
    wave_tree_view: str
    summary: str

    def to_dict(self) -> DagUlwResponseDict:
        """Serialize the DAG-ULW++ campaign response to a TypedDict.

        Returns:
            Typed `DagUlwResponseDict` mapping.
        """
        return DagUlwResponseDict(
            ok=self.ok,
            dag_id=self.dag_id,
            run_state=str(self.run_state),
            plan_only=self.plan_only,
            workspace_dir=self.workspace_dir,
            compiled_plan=self.compiled_plan.to_dict(),
            node_receipts=[r.to_dict() for r in self.node_receipts],
            settled_count=self.settled_count,
            resumed_nodes=list(self.resumed_nodes),
            aimd_final_concurrency=self.aimd_final_concurrency,
            wave_tree_view=self.wave_tree_view,
            summary=self.summary,
        )


@dataclass(frozen=True, slots=True)
class DagSnapshotPayload:
    """Internal DTO for a persisted JobStore DAG snapshot payload."""

    dag_id: str
    plan_digest: str
    workspace_dir: str
    run_state: str
    node_receipts: list[JsonObject]

    def to_dict(self) -> DagSnapshotPayloadDict:
        """Serialize the DAG snapshot payload to a TypedDict.

        Returns:
            Typed `DagSnapshotPayloadDict` mapping.
        """
        return DagSnapshotPayloadDict(
            dag_id=self.dag_id,
            plan_digest=self.plan_digest,
            workspace_dir=self.workspace_dir,
            run_state=self.run_state,
            node_receipts=self.node_receipts,
        )


@dataclass(frozen=True, slots=True)
class TuiSimulatedExecResult:
    """Internal DTO for a simulated execution result in offline TUI preview mode."""

    ok: bool
    status: str
    outcome_state: str
    model_id: str
    model: str
    raw_output: str
    data: str

    def to_dict(self) -> TuiSimulatedExecResultDict:
        """Serialize the simulated TUI execution result to a TypedDict.

        Returns:
            Typed `TuiSimulatedExecResultDict` mapping.
        """
        return TuiSimulatedExecResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            model_id=self.model_id,
            model=self.model,
            raw_output=self.raw_output,
            data=self.data,
        )


@dataclass(frozen=True, slots=True)
class DagAgentPoleProfile:
    """Internal DTO defining model, reasoning effort, and visual effect badge for a DAG agent pole."""

    pole: str
    model_id: str
    reasoning_effort: str
    badge_icon: str
    ansi_color: str

    def to_dict(self) -> DagAgentPoleProfileDict:
        """Serialize the agent pole profile to a TypedDict.

        Returns:
            Typed `DagAgentPoleProfileDict` mapping.
        """
        return DagAgentPoleProfileDict(
            pole=self.pole,
            model_id=self.model_id,
            reasoning_effort=self.reasoning_effort,
            badge_icon=self.badge_icon,
            ansi_color=self.ansi_color,
        )


@dataclass(frozen=True, slots=True)
class DagAgentEffectConfig:
    """Internal DTO storing the active DAG visual effect theme and 3-pole agent configurations."""

    style: DagVisualEffectStyle
    argon: DagAgentPoleProfile
    claude: DagAgentPoleProfile
    codex: DagAgentPoleProfile

    def to_dict(self) -> DagAgentEffectConfigDict:
        """Serialize the DAG agent effect configuration to a TypedDict.

        Returns:
            Typed `DagAgentEffectConfigDict` mapping.
        """
        return DagAgentEffectConfigDict(
            style=str(self.style),
            argon=self.argon.to_dict(),
            claude=self.claude.to_dict(),
            codex=self.codex.to_dict(),
        )


__all__ = [
    "DagAgentEffectConfig",
    "DagAgentPoleProfile",
    "DagCompiledPlan",
    "DagHazard",
    "DagNodeSettlementReceipt",
    "DagSnapshotPayload",
    "DagUlwResponse",
    "DagWaveSpec",
    "TuiSimulatedExecResult",
]
