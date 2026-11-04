use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Succeeded,
    Failed,
    TimedOutUnknown,
    Cancelled,
    RejectedPreDispatch,
    InputBudgetExceeded,
}

impl JobStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOutUnknown => "timed_out_unknown",
            Self::Cancelled => "cancelled",
            Self::RejectedPreDispatch => "rejected_pre_dispatch",
            Self::InputBudgetExceeded => "input_budget_exceeded",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    InFlight,
    ConfirmedSuccess,
    ConfirmedFailure,
    OutcomeUnknown,
    Cancelled,
}

impl OutcomeState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InFlight => "in_flight",
            Self::ConfirmedSuccess => "confirmed_success",
            Self::ConfirmedFailure => "confirmed_failure",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JobRecord {
    pub job_id: String,
    pub task_type: String,
    pub status: JobStatus,
    pub created_at: f64,
    pub workspace_dir: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub completed_at: Option<f64>,
    pub result: Option<Value>,
    pub error: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_map")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default = "default_outcome")]
    pub outcome_state: OutcomeState,
    pub idempotency_key: Option<String>,
    pub provider: Option<String>,
    pub engine: Option<String>,
    #[serde(default = "default_attempt")]
    pub attempt_count: u32,
    #[serde(default)]
    pub cancel_requested: bool,
    pub cancelled_at: Option<f64>,
    pub receipt: Option<Value>,
}

const fn default_outcome() -> OutcomeState {
    OutcomeState::InFlight
}

const fn default_attempt() -> u32 {
    1
}

#[derive(Clone, Debug)]
pub struct NewJob {
    pub task_type: String,
    pub workspace_dir: PathBuf,
    pub model: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub job_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub provider: Option<String>,
    pub engine: Option<String>,
    pub created_at: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JobRegistration {
    pub record: JobRecord,
    pub should_dispatch: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct JobTelemetry {
    pub total_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub unknown_jobs: u64,
    pub running_jobs: u64,
    pub cancelled_jobs: u64,
    pub average_duration_seconds: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriadRole {
    Argon,
    Sonnet,
    Opus,
    Codex,
}

impl TriadRole {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Argon => "argon",
            Self::Sonnet => "sonnet",
            Self::Opus => "opus",
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Pending,
    InProgress,
    Completed,
    Rejected,
}

impl HandoffStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl HandoffPriority {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HandoffTicket {
    pub ticket_id: String,
    pub sender: TriadRole,
    pub recipient: TriadRole,
    pub title: String,
    pub spec_or_payload: String,
    pub workspace_dir: String,
    pub priority: HandoffPriority,
    pub status: HandoffStatus,
    pub created_at: f64,
    pub updated_at: f64,
    pub resolution_note: Option<String>,
    pub alexandria_ref: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewHandoff {
    pub sender: TriadRole,
    pub recipient: TriadRole,
    pub title: String,
    pub spec_or_payload: String,
    pub workspace_dir: PathBuf,
    pub priority: HandoffPriority,
    pub alexandria_ref: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct HandoffFilter {
    pub recipient: Option<TriadRole>,
    pub status: Option<HandoffStatus>,
    pub workspace_dir: Option<PathBuf>,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AuditVerdict {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "WARN")]
    Warn,
    #[serde(rename = "BLOCK")]
    Block,
}

impl HandoffTicket {
    pub fn verdict(&self) -> Option<AuditVerdict> {
        let note = self.resolution_note.as_deref()?;
        serde_json::from_str::<VerdictNote>(note)
            .ok()
            .map(|parsed| parsed.verdict)
    }
}

#[derive(Deserialize)]
struct VerdictNote {
    verdict: AuditVerdict,
}

fn deserialize_string_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = BTreeMap::<String, Value>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_owned())))
        .collect())
}
