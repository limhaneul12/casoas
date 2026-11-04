//! Dynamic DAG planning, hazard analysis, and two-stage settlement.

mod execution;
mod patch;
mod planner;
mod request;
mod settlement;
mod text_patch;
mod types;

pub use execution::{DagExecutionEvent, execute, execute_with_observer};
pub(crate) use patch::normalize_relative;
pub use patch::{PatchArtifact, PatchEdit, apply_scoped_patch};
pub use planner::DagPlanner;
pub use request::{handle_request, handle_request_with_observer, prepare_interactive_request};
pub use settlement::DagSettlementVerifier;
pub use text_patch::{CompiledTextPatch, HunkGeometry, TextPatch, TextPatchError, TextPatchHunk};
pub use types::{
    DagNode, DagPlan, DagWave, GateProof, Hazard, HazardKind, NodeKind, NodeRole, NodeState,
    SettlementReceipt, SettlementStage, WorkspaceManifest,
};
