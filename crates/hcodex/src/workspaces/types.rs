use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dag::{DagExecutionEvent, DagNode};
use crate::research::{ModelSpec, ResearchRequest};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Research,
    Planning,
    Building,
}

impl WorkspaceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Planning => "planning",
            Self::Building => "building",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Research => "Research & Search",
            Self::Planning => "Planning",
            Self::Building => "Building",
        }
    }
}

impl std::fmt::Display for WorkspaceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for WorkspaceKind {
    type Err = crate::Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "research" | "research_and_search" | "research-search" => Ok(Self::Research),
            "planning" | "plan" => Ok(Self::Planning),
            "building" | "build" => Ok(Self::Building),
            _ => Err("workspace must be research, planning or building".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRunStatus {
    #[default]
    Idle,
    Reserved,
    Running,
    Completed,
    IncompleteEvidence,
    BuildReady,
    Blocked,
    Stopped,
    Settled,
    Cancelled,
    TimedOutUnknown,
    Failed,
}

impl WorkspaceRunStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::IncompleteEvidence
                | Self::BuildReady
                | Self::Blocked
                | Self::Stopped
                | Self::Settled
                | Self::Cancelled
                | Self::TimedOutUnknown
                | Self::Failed
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub goal: String,
    pub workspace_dir: PathBuf,
    pub stages: Vec<WorkspaceKind>,
    #[serde(default)]
    pub research: Option<ResearchRequest>,
    #[serde(default)]
    pub research_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default = "default_planner")]
    pub planner: ModelSpec,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

impl WorkflowRequest {
    pub fn new(
        goal: impl Into<String>,
        workspace_dir: PathBuf,
        stages: Vec<WorkspaceKind>,
    ) -> Self {
        Self {
            request_id: None,
            goal: goal.into(),
            workspace_dir,
            stages,
            research: None,
            research_id: None,
            plan_id: None,
            planner: default_planner(),
            timeout_seconds: default_timeout(),
        }
    }
}

fn default_planner() -> ModelSpec {
    ModelSpec {
        model: "gpt-6.1-sol".into(),
        effort: "high".into(),
    }
}
const fn default_timeout() -> u64 {
    0
}

#[derive(Debug, Serialize)]
#[serde(tag = "workflow_event", rename_all = "snake_case")]
pub enum WorkflowEvent {
    StageStarted {
        flow_id: String,
        stage: WorkspaceKind,
        child_id: String,
    },
    StageFinished {
        flow_id: String,
        stage: WorkspaceKind,
        status: WorkspaceRunStatus,
        result: Value,
    },
    Dag {
        flow_id: String,
        #[serde(flatten)]
        event: Box<DagExecutionEvent>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningArtifact {
    pub title: String,
    pub objective: String,
    pub requirements: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub scope: Vec<String>,
    pub non_goals: Vec<String>,
    pub decisions: Vec<String>,
    pub risks: Vec<String>,
    pub blocking_questions: Vec<String>,
    pub tasks: Vec<DagNode>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRecord {
    pub schema_version: String,
    pub plan_id: String,
    pub flow_id: String,
    pub workspace_dir: PathBuf,
    pub source_manifest_digest: String,
    pub research_id: Option<String>,
    pub research_digest: Option<String>,
    pub planner_job_id: String,
    pub planner_receipt: Value,
    pub execution_workspace: PathBuf,
    pub artifact: PlanningArtifact,
    pub artifact_digest: String,
    pub status: WorkspaceRunStatus,
    pub readiness_reasons: Vec<String>,
}

impl PlanRecord {
    pub fn mechanically_ready(&self) -> bool {
        self.status == WorkspaceRunStatus::BuildReady && self.readiness_reasons.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_request_and_workspace_names_round_trip() -> crate::Result<()> {
        let request =
            WorkflowRequest::new("feature", "/tmp/repo".into(), vec![WorkspaceKind::Research]);
        let mut value = serde_json::to_value(&request)?;
        assert_eq!(
            serde_json::from_value::<WorkflowRequest>(value.clone())?.stages,
            request.stages
        );
        assert_eq!(WorkspaceKind::from_str("build")?, WorkspaceKind::Building);
        value["new_authority"] = Value::Bool(true);
        assert!(serde_json::from_value::<WorkflowRequest>(value).is_err());
        Ok(())
    }
}
