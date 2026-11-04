use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoleState {
    NotStarted,
    Queued,
    Running,
    Succeeded,
    Failed,
    TimedOutUnknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Verdict {
    Pass,
    Warn,
    Block,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RoleSnapshot {
    pub role: String,
    pub state: RoleState,
    pub verdict: Option<Verdict>,
    pub response_present: bool,
    pub response_status: Option<String>,
    pub response_ok: Option<bool>,
    pub model_id: Option<String>,
    pub job_id: Option<String>,
    pub handoff_ticket_id: Option<String>,
    pub review_artifact_present: bool,
    pub review_artifact_has_narrative: bool,
    pub review_artifact_digest: Option<String>,
    pub artifact: Option<Value>,
    pub response: Option<Value>,
}

impl RoleSnapshot {
    pub(crate) fn compact(&self) -> Self {
        let mut compact = self.clone();
        compact.artifact = None;
        compact.response = None;
        compact
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RoleSummary {
    pub state: RoleState,
    pub verdict: Option<Verdict>,
    pub has_artifact: bool,
    pub has_valid_review: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Adjudication {
    pub collaboration_status: String,
    pub consensus_status: String,
    pub consensus_reached: bool,
    pub required_roles: Vec<String>,
    pub succeeded_roles: Vec<String>,
    pub incomplete_roles: Vec<String>,
    pub role_states: std::collections::BTreeMap<String, RoleSummary>,
    pub missing_artifact_roles: Vec<String>,
    pub missing_verdict_roles: Vec<String>,
    pub missing_review_roles: Vec<String>,
    pub conflict_detected: bool,
    pub conflict_basis: Option<String>,
    pub decision_authority: String,
    pub review_approved: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GateReceipt {
    pub command: String,
    pub exit_code: i32,
    pub passed: bool,
    pub duration_seconds: f64,
    pub stdout_digest: String,
    pub origin: String,
}
