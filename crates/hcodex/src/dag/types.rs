use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    TopologyPlanner,
    BoundaryMapper,
    InterfaceBuilder,
    ContractVerifier,
    SecurityAuditor,
    IntegrationFanIn,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    #[default]
    Standard,
    Reproduction,
    Verification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Planned,
    Running,
    Stopped,
    Settled,
    RejectedOutOfScopeWrite,
    RejectedDirtyRead,
    RejectedVerification,
    RejectedReproductionNotObserved,
    BlockedUpstream,
    FailedExecution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStage {
    Queued,
    Running,
    Settled,
    RejectedOutOfScopeWrite,
    RejectedDirtyRead,
    RejectedVerifyFailed,
    RejectedReproductionNotObserved,
    BlockedUpstream,
    FailedExecution,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagNode {
    pub id: String,
    pub title: String,
    pub task: String,
    pub role: NodeRole,
    #[serde(default)]
    pub node_kind: NodeKind,
    pub model: String,
    pub reasoning_effort: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub read_files: Vec<PathBuf>,
    #[serde(default)]
    pub write_files: Vec<PathBuf>,
    #[serde(default)]
    pub symbols: Vec<String>,
    pub verify_command: Option<String>,
    #[serde(default = "default_true")]
    pub reproduction_expected_exit_nonzero: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagWave {
    pub index: usize,
    pub node_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HazardKind {
    DuplicateNode,
    UnknownDependency,
    Cycle,
    UnsafePath,
    WaveCountOutOfRange,
    NodeCountOutOfRange,
    Waw,
    Raw,
    War,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hazard {
    pub code: String,
    pub kind: HazardKind,
    pub node_ids: Vec<String>,
    pub paths: Vec<PathBuf>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagPlan {
    pub goal: String,
    pub nodes: Vec<DagNode>,
    pub waves: Vec<DagWave>,
    pub hazards: Vec<Hazard>,
    pub total_nodes: usize,
    pub total_waves: usize,
    pub blocking_hazards_count: usize,
    pub ascii_graph: String,
}

impl DagPlan {
    pub fn is_executable(&self) -> bool {
        self.hazards.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceManifest {
    pub root: PathBuf,
    pub files: BTreeMap<PathBuf, String>,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateProof {
    pub command: String,
    pub executed: bool,
    pub exit_code: Option<i32>,
    pub stdout_digest: Option<String>,
}

impl GateProof {
    pub fn passed(&self) -> bool {
        self.executed && self.exit_code == Some(0) && self.stdout_digest.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementReceipt {
    pub node_id: String,
    pub state: NodeState,
    pub settlement_stage: SettlementStage,
    pub role: NodeRole,
    pub node_kind: NodeKind,
    pub wave_index: usize,
    pub model_id: String,
    pub duration_seconds: f64,
    pub pre_manifest_digest: String,
    pub stopped_manifest_digest: String,
    pub post_manifest_digest: String,
    pub changed_files: Vec<PathBuf>,
    pub mutated_files: Vec<PathBuf>,
    pub out_of_scope_writes: Vec<PathBuf>,
    pub dirty_read_files: Vec<PathBuf>,
    pub verify_passed: bool,
    pub reproduction_observed: bool,
    pub output_summary: String,
    pub error: Option<String>,
    pub gate: GateProof,
}

const fn default_true() -> bool {
    true
}
