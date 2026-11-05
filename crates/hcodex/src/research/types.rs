//! Closed research inputs and source-bound artifacts; no write or command authority.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResearchRequest {
    pub question: String,
    pub workspace_dir: PathBuf,
    #[serde(default)]
    pub audience: Audience,
    #[serde(default = "default_language")]
    pub output_language: String,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub sources: Vec<SourceSpec>,
    #[serde(default = "default_investigators")]
    pub investigators: Vec<Investigator>,
    #[serde(default = "default_challenger")]
    pub challenger: ModelSpec,
    #[serde(default = "default_synthesizer")]
    pub synthesizer: ModelSpec,
    #[serde(default)]
    pub budget: ResearchBudget,
    #[serde(default)]
    pub plan_only: bool,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<DiscoveryConfig>,
}

impl ResearchRequest {
    pub fn new(question: impl Into<String>, workspace_dir: PathBuf) -> Self {
        Self {
            question: question.into(),
            workspace_dir,
            audience: Audience::default(),
            output_language: default_language(),
            as_of: None,
            sources: Vec::new(),
            investigators: default_investigators(),
            challenger: default_challenger(),
            synthesizer: default_synthesizer(),
            budget: ResearchBudget::default(),
            plan_only: false,
            request_id: None,
            idempotency_key: None,
            discovery: None,
        }
    }

    /// Resolves the discovery policy without changing the serialized request.
    /// Question-only requests opt into bounded web discovery, while existing
    /// source-seeded requests retain their no-search behavior.
    pub fn active_discovery(&self) -> Option<DiscoveryConfig> {
        match &self.discovery {
            Some(DiscoveryConfig::Disabled) => None,
            Some(config @ DiscoveryConfig::Web { .. }) => Some(config.clone()),
            None if self.sources.is_empty() => Some(DiscoveryConfig::default_web()),
            None => None,
        }
    }
}

pub const MAX_SEARCH_EVENTS: usize = 6;
pub const MAX_DISCOVERY_CANDIDATES: usize = 12;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryConfig {
    Disabled,
    Web {
        #[serde(default = "default_discovery_model")]
        model: String,
        #[serde(default = "default_discovery_effort")]
        effort: String,
        #[serde(default = "default_search_events")]
        max_search_events: usize,
        #[serde(default = "default_discovery_candidates")]
        max_candidates: usize,
        #[serde(default = "default_accepted_sources")]
        min_accepted_sources: usize,
        #[serde(default = "default_discovery_timeout")]
        timeout_seconds: u64,
    },
}

impl DiscoveryConfig {
    pub fn default_web() -> Self {
        Self::Web {
            model: default_discovery_model(),
            effort: default_discovery_effort(),
            max_search_events: default_search_events(),
            max_candidates: default_discovery_candidates(),
            min_accepted_sources: default_accepted_sources(),
            timeout_seconds: default_discovery_timeout(),
        }
    }

    pub fn web_with_model(model: String) -> Self {
        Self::Web {
            model,
            effort: default_discovery_effort(),
            max_search_events: default_search_events(),
            max_candidates: default_discovery_candidates(),
            min_accepted_sources: default_accepted_sources(),
            timeout_seconds: default_discovery_timeout(),
        }
    }
}

fn default_discovery_model() -> String {
    "gpt-6.1-sol".to_owned()
}

fn default_discovery_effort() -> String {
    "high".to_owned()
}

const fn default_search_events() -> usize {
    3
}

const fn default_discovery_candidates() -> usize {
    6
}

const fn default_accepted_sources() -> usize {
    2
}

const fn default_discovery_timeout() -> u64 {
    0
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    #[default]
    Human,
    CodingAgent,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSpec {
    Repository { path: PathBuf },
    TechnicalDocument { path: PathBuf },
    Web { url: String },
}

impl SourceSpec {
    pub fn kind(&self) -> SourceKind {
        match self {
            Self::Repository { .. } => SourceKind::Repository,
            Self::TechnicalDocument { .. } => SourceKind::TechnicalDocument,
            Self::Web { .. } => SourceKind::Web,
        }
    }
    pub fn locator(&self) -> String {
        match self {
            Self::Repository { path } | Self::TechnicalDocument { path } => {
                path.display().to_string()
            }
            Self::Web { url } => url.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Repository,
    TechnicalDocument,
    Web,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Investigator {
    pub lane_id: String,
    pub focus: String,
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResearchBudget {
    pub max_parallel: usize,
    pub max_sources: usize,
    pub max_source_bytes: usize,
    pub max_total_source_bytes: usize,
    pub max_excerpt_bytes: usize,
    pub max_total_excerpt_bytes: usize,
    pub max_claims: usize,
    pub timeout_seconds: u64,
}
impl Default for ResearchBudget {
    fn default() -> Self {
        Self {
            max_parallel: 3,
            max_sources: 8,
            max_source_bytes: 262_144,
            max_total_source_bytes: 1_048_576,
            max_excerpt_bytes: 8_192,
            max_total_excerpt_bytes: 65_536,
            max_claims: 32,
            timeout_seconds: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    pub source_id: String,
    pub kind: SourceKind,
    pub locator: String,
    pub observed_at: String,
    pub body_sha256: String,
    pub body_bytes: u64,
    pub media_type: String,
    pub redirect_chain: Vec<String>,
    pub excerpt: Excerpt,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Excerpt {
    pub excerpt_id: String,
    pub source_id: String,
    pub text: String,
    pub sha256: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceFailure {
    pub kind: SourceKind,
    pub locator: String,
    pub error_code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceBundle {
    pub sources: Vec<SourceRecord>,
    pub failures: Vec<SourceFailure>,
    pub snapshot_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhase {
    Investigate,
    Challenge,
    Synthesize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    Supported,
    Contested,
    Unresolved,
    Refuted,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStance {
    Support,
    Counter,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub source_id: String,
    pub excerpt_id: String,
    pub quote: String,
    pub stance: EvidenceStance,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResearchClaim {
    pub claim_id: String,
    pub text: String,
    pub assessment: Assessment,
    pub inference: bool,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaimChallenge {
    pub claim_id: String,
    pub assessment: Assessment,
    pub reason: String,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReportSection {
    pub heading: String,
    pub claim_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageArtifact {
    pub agent_reported_advisory: String,
    pub claims: Vec<ResearchClaim>,
    pub challenges: Vec<ClaimChallenge>,
    pub report_sections: Vec<ReportSection>,
    pub unresolved: Vec<String>,
}

fn default_language() -> String {
    "ko".to_owned()
}
fn default_investigators() -> Vec<Investigator> {
    vec![Investigator {
        lane_id: "investigator-1".to_owned(),
        focus: "Find source-backed answers and state uncertainties.".to_owned(),
        model: "gemini-4-argon-high".to_owned(),
        effort: "high".to_owned(),
    }]
}
fn default_challenger() -> ModelSpec {
    ModelSpec {
        model: "claude-sonnet-5-5-medium".to_owned(),
        effort: "medium".to_owned(),
    }
}
fn default_synthesizer() -> ModelSpec {
    ModelSpec {
        model: "gpt-6.1-sol".to_owned(),
        effort: "high".to_owned(),
    }
}
