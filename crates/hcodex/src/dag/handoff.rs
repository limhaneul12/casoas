use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{GateProof, NodeKind, NodeState, SettlementReceipt, SettlementStage};

const SCHEMA_VERSION: u8 = 1;
const MAX_AGENT_REPORT_BYTES: usize = 2 * 1024;
const MAX_AGENT_REPORT_ITEMS: usize = 32;
const MAX_PARENT_REFS: usize = 8;
const MAX_RECORD_BYTES: usize = 32 * 1024;
const MAX_ID_BYTES: usize = 256;
const TRUNCATION_MARKER: &str = "...[truncated]";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NodeHandoff {
    pub(super) schema_version: u8,
    pub(super) campaign_id: String,
    pub(super) node_id: String,
    pub(super) plan_identity: String,
    pub(super) config_identity: String,
    pub(super) checkpoint_id: String,
    pub(super) system_facts: HandoffFacts,
    pub(super) agent_report: AgentReport,
    pub(super) parents: Vec<HandoffRef>,
    pub(super) content_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandoffFacts {
    pub(super) actual_model: String,
    pub(super) changed_files: Vec<PathBuf>,
    pub(super) pre_manifest_digest: String,
    pub(super) post_manifest_digest: String,
    pub(super) gate: GateProof,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentReport {
    pub(super) summary: String,
    pub(super) decisions: Vec<String>,
    pub(super) caveats: Vec<String>,
    pub(super) truncated: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandoffRef {
    pub(super) campaign_id: String,
    pub(super) node_id: String,
    pub(super) checkpoint_id: String,
    pub(super) content_digest: String,
}

impl NodeHandoff {
    #[cfg(test)]
    pub(super) fn create(
        campaign_id: &str,
        plan_identity: &str,
        config_identity: &str,
        receipt: &SettlementReceipt,
        parents: Vec<HandoffRef>,
        provider_output: Option<&Value>,
    ) -> crate::Result<Self> {
        Self::create_with_report(
            campaign_id,
            plan_identity,
            config_identity,
            receipt,
            parents,
            AgentReport::from_output(&receipt.output_summary, provider_output),
        )
    }

    pub(super) fn create_with_report(
        campaign_id: &str,
        plan_identity: &str,
        config_identity: &str,
        receipt: &SettlementReceipt,
        mut parents: Vec<HandoffRef>,
        agent_report: AgentReport,
    ) -> crate::Result<Self> {
        validate_plan_node(campaign_id, 0)?;
        validate_plan_node(&receipt.node_id, parents.len())?;
        validate_digest(plan_identity, "plan identity")?;
        validate_digest(config_identity, "config identity")?;
        validate_settled_receipt(receipt)?;
        parents.sort();
        validate_parent_refs(&receipt.node_id, &parents)?;

        let mut changed_files = receipt.changed_files.clone();
        changed_files.sort();
        validate_changed_files(&changed_files)?;

        let checkpoint_id = checkpoint_id(campaign_id, &receipt.node_id);
        let mut handoff = Self {
            schema_version: SCHEMA_VERSION,
            campaign_id: campaign_id.to_owned(),
            node_id: receipt.node_id.clone(),
            plan_identity: plan_identity.to_owned(),
            config_identity: config_identity.to_owned(),
            checkpoint_id,
            system_facts: HandoffFacts {
                actual_model: receipt.model_id.clone(),
                changed_files,
                pre_manifest_digest: receipt.pre_manifest_digest.clone(),
                post_manifest_digest: receipt.post_manifest_digest.clone(),
                gate: receipt.gate.clone(),
            },
            agent_report,
            parents,
            content_digest: String::new(),
        };
        handoff.content_digest = handoff.calculate_digest()?;
        handoff.verify()?;
        Ok(handoff)
    }

    pub(super) fn verify(&self) -> crate::Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!("unsupported node handoff schema: {}", self.schema_version).into());
        }
        validate_plan_node(&self.campaign_id, 0)?;
        validate_plan_node(&self.node_id, self.parents.len())?;
        validate_digest(&self.plan_identity, "plan identity")?;
        validate_digest(&self.config_identity, "config identity")?;
        validate_digest(&self.content_digest, "handoff content digest")?;
        if self.checkpoint_id != checkpoint_id(&self.campaign_id, &self.node_id) {
            return Err("node handoff checkpoint identity mismatch".into());
        }
        validate_system_facts(&self.system_facts)?;
        validate_agent_report(&self.agent_report)?;
        validate_parent_refs(&self.node_id, &self.parents)?;
        if self.content_digest != self.calculate_digest()? {
            return Err("node handoff content digest mismatch".into());
        }
        if serde_json::to_vec(self)?.len() > MAX_RECORD_BYTES {
            return Err("node handoff exceeds record byte budget".into());
        }
        Ok(())
    }

    pub(super) fn reference(&self) -> HandoffRef {
        HandoffRef {
            campaign_id: self.campaign_id.clone(),
            node_id: self.node_id.clone(),
            checkpoint_id: self.checkpoint_id.clone(),
            content_digest: self.content_digest.clone(),
        }
    }

    fn calculate_digest(&self) -> crate::Result<String> {
        let mut digest_input = self.clone();
        digest_input.content_digest.clear();
        let bytes = serde_json::to_vec(&digest_input)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

impl AgentReport {
    pub(super) fn from_output(summary: &str, provider_output: Option<&Value>) -> Self {
        let summary = if summary.is_empty() {
            provider_output
                .and_then(selected_summary)
                .unwrap_or(summary)
        } else {
            summary
        };
        let decisions = provider_output
            .map(|output| selected_array_strings(output, "design_highlights"))
            .unwrap_or_default();
        let mut caveats = provider_output
            .map(|output| selected_array_strings(output, "edge_cases_guarded"))
            .unwrap_or_default();
        if let Some(note) = provider_output
            .and_then(|output| selected_string(output, "handoff_notes_for_codex_audit"))
            .filter(|note| !note.is_empty())
        {
            caveats.push(note.to_owned());
        }

        Self::bounded(summary, decisions, caveats, false)
    }

    pub(super) fn with_summary(self, summary: &str) -> Self {
        Self::bounded(summary, self.decisions, self.caveats, self.truncated)
    }

    fn bounded(
        summary: &str,
        decisions: Vec<String>,
        caveats: Vec<String>,
        mut truncated: bool,
    ) -> Self {
        let mut remaining = MAX_AGENT_REPORT_BYTES;
        let mut remaining_items = MAX_AGENT_REPORT_ITEMS;
        let summary = take_bounded(summary, &mut remaining, &mut truncated).unwrap_or_default();
        let decisions = take_bounded_list(
            decisions,
            &mut remaining,
            &mut remaining_items,
            &mut truncated,
        );
        let caveats = take_bounded_list(
            caveats,
            &mut remaining,
            &mut remaining_items,
            &mut truncated,
        );
        Self {
            summary,
            decisions,
            caveats,
            truncated,
        }
    }

    fn content_bytes(&self) -> usize {
        self.summary.len()
            + self.decisions.iter().map(String::len).sum::<usize>()
            + self.caveats.iter().map(String::len).sum::<usize>()
    }
}

fn validate_settled_receipt(receipt: &SettlementReceipt) -> crate::Result<()> {
    validate_id(&receipt.node_id, "node ID")?;
    validate_id(&receipt.model_id, "actual model")?;
    if receipt.state != NodeState::Settled || receipt.settlement_stage != SettlementStage::Settled {
        return Err("node handoff requires a settled receipt".into());
    }
    validate_digest(&receipt.pre_manifest_digest, "pre-manifest digest")?;
    validate_digest(&receipt.post_manifest_digest, "post-manifest digest")?;
    if !receipt.gate.executed
        || receipt.gate.exit_code.is_none()
        || receipt.gate.stdout_digest.is_none()
        || (!receipt.gate.passed()
            && !(receipt.node_kind == NodeKind::Reproduction && receipt.reproduction_observed))
    {
        return Err("settled receipt lacks terminal verification evidence".into());
    }
    if receipt.verify_passed != receipt.gate.passed() {
        return Err("settled receipt verification flag conflicts with gate proof".into());
    }
    Ok(())
}

fn validate_system_facts(facts: &HandoffFacts) -> crate::Result<()> {
    validate_id(&facts.actual_model, "actual model")?;
    validate_changed_files(&facts.changed_files)?;
    validate_digest(&facts.pre_manifest_digest, "pre-manifest digest")?;
    validate_digest(&facts.post_manifest_digest, "post-manifest digest")?;
    if !facts.gate.executed || facts.gate.exit_code.is_none() || facts.gate.stdout_digest.is_none()
    {
        return Err("node handoff gate proof is incomplete".into());
    }
    Ok(())
}

fn validate_agent_report(report: &AgentReport) -> crate::Result<()> {
    if report.content_bytes() > MAX_AGENT_REPORT_BYTES {
        return Err("node handoff agent report exceeds byte budget".into());
    }
    Ok(())
}

fn validate_parent_refs(node_id: &str, parents: &[HandoffRef]) -> crate::Result<()> {
    if parents.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("node handoff parent references are not canonical".into());
    }
    let mut identities = BTreeSet::new();
    for parent in parents {
        validate_id(&parent.campaign_id, "parent campaign ID")?;
        validate_id(&parent.node_id, "parent node ID")?;
        validate_digest(&parent.content_digest, "parent handoff digest")?;
        if parent.node_id == node_id {
            return Err("node handoff cannot reference itself".into());
        }
        if parent.checkpoint_id != checkpoint_id(&parent.campaign_id, &parent.node_id) {
            return Err("node handoff parent checkpoint identity mismatch".into());
        }
        if !identities.insert((&parent.campaign_id, &parent.node_id)) {
            return Err("node handoff contains duplicate parent identity".into());
        }
    }
    Ok(())
}

pub(super) fn validate_plan_node(identity: &str, dependency_count: usize) -> crate::Result<()> {
    validate_id(identity, "plan node or campaign ID")?;
    if dependency_count > MAX_PARENT_REFS {
        return Err("plan node has too many direct dependencies".into());
    }
    Ok(())
}

fn validate_changed_files(paths: &[PathBuf]) -> crate::Result<()> {
    if paths.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("node handoff changed files are not canonical".into());
    }
    let mut seen = BTreeSet::new();
    for path in paths {
        if !safe_relative(path) {
            return Err(format!("unsafe changed file in node handoff: {}", path.display()).into());
        }
        if !seen.insert(path) {
            return Err(
                format!("duplicate changed file in node handoff: {}", path.display()).into(),
            );
        }
    }
    Ok(())
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn validate_id(value: &str, label: &str) -> crate::Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("invalid {label}").into());
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> crate::Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(format!("invalid {label}").into());
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid {label}").into());
    }
    Ok(())
}

fn checkpoint_id(campaign_id: &str, node_id: &str) -> String {
    format!("ckp-{campaign_id}-{node_id}")
}

fn selected_array_strings(output: &Value, key: &str) -> Vec<String> {
    selected_values(output, key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn selected_string<'a>(output: &'a Value, key: &str) -> Option<&'a str> {
    selected_values(output, key).and_then(Value::as_str)
}

fn selected_summary(output: &Value) -> Option<&str> {
    [
        "/result/structured_output/summary",
        "/result/structured_output/executive_critique",
        "/structured_output/summary",
        "/result/output",
        "/provider_output/result/structured_output/summary",
        "/provider_output/result/structured_output/executive_critique",
        "/provider_output/structured_output/summary",
        "/provider_output/result/output",
    ]
    .iter()
    .find_map(|pointer| output.pointer(pointer).and_then(Value::as_str))
}

fn selected_values<'a>(output: &'a Value, key: &str) -> Option<&'a Value> {
    [
        format!("/result/data/{key}"),
        format!("/data/{key}"),
        format!("/result/structured_output/{key}"),
        format!("/structured_output/{key}"),
    ]
    .iter()
    .find_map(|pointer| output.pointer(pointer))
}

fn take_bounded_list(
    values: Vec<String>,
    remaining: &mut usize,
    remaining_items: &mut usize,
    truncated: &mut bool,
) -> Vec<String> {
    let mut bounded = Vec::new();
    for value in values {
        if *remaining_items == 0 {
            *truncated = true;
            break;
        }
        let Some(value) = take_bounded(&value, remaining, truncated) else {
            *truncated = true;
            break;
        };
        bounded.push(value);
        *remaining_items -= 1;
        if *remaining == 0 {
            break;
        }
    }
    bounded
}

fn take_bounded(value: &str, remaining: &mut usize, truncated: &mut bool) -> Option<String> {
    if value.is_empty() {
        return Some(String::new());
    }
    if value.len() <= *remaining {
        *remaining -= value.len();
        return Some(value.to_owned());
    }
    *truncated = true;
    if *remaining == 0 {
        return None;
    }
    let marker = if *remaining >= TRUNCATION_MARKER.len() {
        TRUNCATION_MARKER
    } else {
        ""
    };
    let prefix_budget = remaining.saturating_sub(marker.len());
    let mut end = prefix_budget.min(value.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut result = value[..end].to_owned();
    result.push_str(marker);
    *remaining = 0;
    Some(result)
}

#[cfg(test)]
mod tests;
