//! Dynamic DAG planning, hazard analysis, and two-stage settlement.

mod checkpoint;
mod execution;
mod handoff;
mod patch;
mod planner;
mod request;
mod settlement;
mod text_patch;
mod types;
mod worktree_execution;
mod worktrees;

pub use execution::{DagExecutionEvent, execute, execute_with_observer};
pub(crate) use execution::{default_gate_command, gate_command_supported, validate_node_route};
pub(crate) use patch::normalize_relative;
pub use patch::{PatchArtifact, PatchEdit, apply_scoped_patch};
pub use planner::DagPlanner;
pub use request::{
    PreparedResume, handle_request, handle_request_with_observer, prepare_interactive_request,
    prepare_resume_request,
};
pub use settlement::DagSettlementVerifier;
pub use text_patch::{CompiledTextPatch, HunkGeometry, TextPatch, TextPatchError, TextPatchHunk};
pub use types::{
    DagNode, DagPlan, DagWave, GateProof, Hazard, HazardKind, NodeKind, NodeRole, NodeState,
    SettlementReceipt, SettlementStage, WorkspaceManifest,
};
pub(crate) use worktrees::inspection::inspect as inspect_worktree_recovery;
pub(crate) use worktrees::require_clean_original;
pub use worktrees::{NamedWorktree, open_named_worktree};
