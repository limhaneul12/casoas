//! UI-only projection for the three durable workflow spaces.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{DagLaneView, TranscriptEntry, ViewState};
use crate::workspaces::{WorkspaceKind, WorkspaceRunStatus};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceArtifactView {
    pub artifact_id: String,
    pub digest: String,
    pub label: String,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum WorkspaceUpdate {
    Status {
        status: WorkspaceRunStatus,
        detail: String,
    },
    Transcript(TranscriptEntry),
    View {
        title: String,
        body: Vec<String>,
    },
    Lanes(Vec<DagLaneView>),
    Progress {
        done: usize,
        total: usize,
    },
    Artifact(WorkspaceArtifactView),
}

#[derive(Debug)]
pub struct WorkspacePane {
    pub(super) current_run_id: Option<String>,
    pub(super) status: WorkspaceRunStatus,
    pub(super) detail: String,
    pub(super) objective: String,
    pub(super) artifact: Option<WorkspaceArtifactView>,
    pub(super) input: String,
    pub(super) cursor: usize,
    pub(super) transcript: VecDeque<TranscriptEntry>,
    pub(super) lanes: Vec<DagLaneView>,
    pub(super) lane_clocks: BTreeMap<String, (Instant, Duration)>,
    pub(super) done: usize,
    pub(super) total: usize,
    pub(super) current_view: Option<ViewState>,
    pub(super) view_history: Vec<ViewState>,
    pub(super) ignored_updates: usize,
}

impl WorkspacePane {
    pub(super) fn new(kind: WorkspaceKind) -> Self {
        let ready = match kind {
            WorkspaceKind::Building => "Heterarchy Codex Rust workbench ready".to_owned(),
            _ => format!("{} workspace ready", kind.label()),
        };
        Self {
            current_run_id: None,
            status: WorkspaceRunStatus::Idle,
            detail: "Ready".to_owned(),
            objective: String::new(),
            artifact: None,
            input: String::new(),
            cursor: 0,
            transcript: VecDeque::from([TranscriptEntry::root(ready)]),
            lanes: Vec::new(),
            lane_clocks: BTreeMap::new(),
            done: 0,
            total: 0,
            current_view: None,
            view_history: Vec::new(),
            ignored_updates: 0,
        }
    }

    pub fn current_run_id(&self) -> Option<&str> {
        self.current_run_id.as_deref()
    }

    pub fn status(&self) -> WorkspaceRunStatus {
        self.status
    }

    pub fn status_label(&self) -> &'static str {
        status_label(self.status)
    }

    pub fn status_glyph(&self) -> &'static str {
        status_glyph(self.status)
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn objective(&self) -> &str {
        &self.objective
    }

    pub fn artifact(&self) -> Option<&WorkspaceArtifactView> {
        self.artifact.as_ref()
    }

    pub fn transcript(&self) -> &VecDeque<TranscriptEntry> {
        &self.transcript
    }

    pub fn lanes(&self) -> &[DagLaneView] {
        &self.lanes
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.done, self.total)
    }

    pub fn ignored_updates(&self) -> usize {
        self.ignored_updates
    }
}

pub(super) fn panes() -> BTreeMap<WorkspaceKind, WorkspacePane> {
    [
        WorkspaceKind::Research,
        WorkspaceKind::Planning,
        WorkspaceKind::Building,
    ]
    .into_iter()
    .map(|kind| (kind, WorkspacePane::new(kind)))
    .collect()
}

pub(super) fn status_label(status: WorkspaceRunStatus) -> &'static str {
    match status {
        WorkspaceRunStatus::Idle => "IDLE",
        WorkspaceRunStatus::Reserved => "RESERVED",
        WorkspaceRunStatus::Running => "RUNNING",
        WorkspaceRunStatus::Completed => "COMPLETED",
        WorkspaceRunStatus::IncompleteEvidence => "INCOMPLETE",
        WorkspaceRunStatus::BuildReady => "BUILD READY",
        WorkspaceRunStatus::Blocked => "BLOCKED",
        WorkspaceRunStatus::Stopped => "STOPPED",
        WorkspaceRunStatus::Settled => "SETTLED",
        WorkspaceRunStatus::Cancelled => "CANCELLED",
        WorkspaceRunStatus::TimedOutUnknown => "UNKNOWN",
        WorkspaceRunStatus::Failed => "FAILED",
    }
}

pub(super) fn status_glyph(status: WorkspaceRunStatus) -> &'static str {
    match status {
        WorkspaceRunStatus::Running => "●",
        WorkspaceRunStatus::Reserved | WorkspaceRunStatus::Stopped => "◐",
        WorkspaceRunStatus::Completed
        | WorkspaceRunStatus::BuildReady
        | WorkspaceRunStatus::Settled => "✓",
        WorkspaceRunStatus::IncompleteEvidence
        | WorkspaceRunStatus::Blocked
        | WorkspaceRunStatus::Cancelled
        | WorkspaceRunStatus::TimedOutUnknown
        | WorkspaceRunStatus::Failed => "!",
        WorkspaceRunStatus::Idle => "○",
    }
}

pub(super) fn terminal(status: WorkspaceRunStatus) -> bool {
    status == WorkspaceRunStatus::Idle || status.is_terminal()
}
