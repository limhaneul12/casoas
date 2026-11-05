//! One durable native-search proposal stage. Only host acquisition creates sources.
use crate::execution_policy::ExecutionDeadline;
use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{DiscoveryConfig, ResearchRequest, SourceSpec, digest, normalize_candidate_url};
use crate::{Result, gateway::Gateway, storage::JobStatus};

pub(super) const CONTRACT: &str = "research_native_discovery/v1";

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct DiscoveryRecord {
    pub job_id: String,
    pub job_status: Option<JobStatus>,
    pub receipt: Value,
    pub search_evidence: Value,
    pub candidate_urls: Vec<String>,
    pub error: Option<String>,
    pub unknown: bool,
    pub cancelled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidates {
    candidate_urls: Vec<String>,
}

pub(crate) fn output_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{"candidate_urls":{"type":"array","maxItems":12,"items":{"type":"string","maxLength":2048}}},"required":["candidate_urls"]})
}

pub(super) fn job_id(parent: &str, config: &DiscoveryConfig) -> Result<String> {
    let hash = digest(&(CONTRACT, parent, config))?;
    Ok(format!("rsd-{}", &hash.trim_start_matches("sha256:")[..32]))
}

pub(super) async fn discover(
    gateway: &Gateway,
    request: &ResearchRequest,
    parent: &str,
    deadline: ExecutionDeadline,
) -> Result<Option<DiscoveryRecord>> {
    let Some(config @ DiscoveryConfig::Web { .. }) = request.active_discovery() else {
        return Ok(None);
    };
    let DiscoveryConfig::Web {
        ref model,
        ref effort,
        max_search_events,
        max_candidates,
        timeout_seconds,
        ..
    } = config
    else {
        return Ok(None);
    };
    let id = job_id(parent, &config)?;
    let mut record = DiscoveryRecord {
        job_id: id.clone(),
        ..Default::default()
    };
    let discovery_deadline = ExecutionDeadline::from_limit(Duration::from_secs(timeout_seconds));
    let remaining = match (
        deadline.remaining_seconds(),
        discovery_deadline.remaining_seconds(),
    ) {
        (Some(0.0), other) | (other, Some(0.0)) => other,
        (Some(a), Some(b)) => Some(a.min(b)),
        _ => None,
    };
    let Some(remaining) = remaining.filter(|_| !crate::supervisor::is_cancelled()) else {
        record.error = Some("discovery not admitted after cancellation/deadline".into());
        record.cancelled = crate::supervisor::is_cancelled();
        return Ok(Some(record));
    };
    let task = json!({"question":request.question,"output_language":request.output_language,"max_search_events":max_search_events,"max_candidates":max_candidates,"already_declared_urls":request.sources.iter().filter_map(|s| match s { SourceSpec::Web {url}=>Some(url), _=>None }).collect::<Vec<_>>()});
    let args = json!({"task_spec":serde_json::to_string(&task)?,"workspace_dir":request.workspace_dir,"model":model,"reasoning_effort":effort,"engine":"cli","timeout_seconds":remaining,"request_id":id,"idempotency_key":digest(&(CONTRACT,parent,&config))?,"max_input_bytes":64000,"sandbox_mode":"read-only","max_search_events":max_search_events,"max_candidates":max_candidates});
    // Gateway persists the sanitized result before returning. Parent retries are
    // readback only, even when a response is lost or the outcome is unknown.
    let response = gateway.call("research_discover", args).await;
    let lookup = id.clone();
    let stored = match gateway.database(move |store| store.get_job(&lookup)).await {
        Ok(Some(job)) => job,
        Ok(None) => {
            record.unknown = true;
            record.error = Some("discovery has no durable observation; inspect original ID".into());
            return Ok(Some(record));
        }
        Err(_) => {
            record.unknown = true;
            record.error = Some("discovery readback unavailable; inspect original ID".into());
            return Ok(Some(record));
        }
    };
    record.job_status = Some(stored.status);
    record.receipt = stored.receipt.clone().unwrap_or(Value::Null);
    record.unknown = outcome_unknown(
        stored.status,
        &record.receipt,
        stored
            .result
            .as_ref()
            .and_then(|v| v.pointer("/result/search_evidence")),
    );
    record.cancelled = stored.status == JobStatus::Cancelled;
    let result = stored.result.as_ref().and_then(|v| v.get("result"));
    record.search_evidence = result
        .and_then(|v| v.get("search_evidence"))
        .cloned()
        .unwrap_or(Value::Null);
    if stored.status != JobStatus::Completed
        || record.receipt["actual_model"] != record.receipt["resolved_model"]
        || !record.receipt["actual_model"].is_string()
        || record.receipt["requested_model"] != model.as_str()
        || record.receipt["model_mismatch"] == true
        || record.unknown
    {
        record.error = Some(format!(
            "native discovery did not produce matching closed execution evidence: {}",
            stored.status.as_str()
        ));
        return Ok(Some(record));
    }
    if !confirmed_search(&record.search_evidence, max_search_events) {
        record.error = Some(
            "no valid completed native web-search evidence; URL proposals are unverified".into(),
        );
        return Ok(Some(record));
    }
    let artifact = result.and_then(|v| v.get("structured_output")).or_else(|| {
        response
            .as_ref()
            .ok()
            .and_then(|v| v.pointer("/result/structured_output"))
    });
    let Some(artifact) = artifact else {
        record.error = Some("discovery structured output missing".into());
        return Ok(Some(record));
    };
    let candidates: Candidates = match serde_json::from_value(artifact.clone()) {
        Ok(candidates) => candidates,
        Err(_) => {
            record.error =
                Some("discovery structured output violates closed candidate contract".into());
            return Ok(Some(record));
        }
    };
    if candidates.candidate_urls.len() > max_candidates {
        record.error = Some("discovery candidate budget exceeded".into());
        return Ok(Some(record));
    }
    let mut seen = request
        .sources
        .iter()
        .filter_map(|source| match source {
            SourceSpec::Web { url } => normalize_candidate_url(url).ok(),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let available = request
        .budget
        .max_sources
        .saturating_sub(request.sources.len());
    for candidate in candidates.candidate_urls {
        if let Ok(url) = normalize_candidate_url(&candidate)
            && seen.insert(url.clone())
            && record.candidate_urls.len() < available
        {
            record.candidate_urls.push(url);
        }
    }
    if record.candidate_urls.is_empty() {
        record.error = Some("native search returned no new admissible HTTPS candidates".into());
    }
    Ok(Some(record))
}

// Versioned observed CLI events are required, independently of model output.
// Kept here as a host gate even when transport validates the same invariant.
pub(super) fn confirmed_search(evidence: &Value, limit: usize) -> bool {
    let Ok(evidence) = serde_json::from_value::<crate::runtime::SearchEvidence>(evidence.clone())
    else {
        return false;
    };
    let observed = evidence.observed_search_ids.iter().collect::<BTreeSet<_>>();
    let completed = evidence
        .completed_search_ids
        .iter()
        .collect::<BTreeSet<_>>();
    evidence.schema_version == "codex_web_search_evidence/v1"
        && evidence.stop_reason == crate::runtime::SearchStopReason::Completed
        && !observed.is_empty()
        && observed.len() <= limit
        && observed.len() == evidence.observed_search_ids.len()
        && completed == observed
        && completed.len() == evidence.completed_search_ids.len()
        && evidence.observed_searches == observed.len()
        && evidence.completed_searches == completed.len()
        && !evidence.matching_event_sha256.is_empty()
        && evidence.violation.is_none()
}

fn outcome_unknown(status: JobStatus, receipt: &Value, evidence: Option<&Value>) -> bool {
    matches!(status, JobStatus::Running | JobStatus::TimedOutUnknown)
        || receipt["outcome_state"] == "outcome_unknown"
        || ((status == JobStatus::Completed
            || evidence.is_some_and(|e| !e.is_null())
            || receipt["actual_model"].is_string())
            && receipt["owned_processes_closed"] != true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmed_pre_dispatch_failures_are_distinct_from_unknown_owned_execution() {
        for status in [
            JobStatus::Failed,
            JobStatus::RejectedPreDispatch,
            JobStatus::InputBudgetExceeded,
        ] {
            assert!(!outcome_unknown(status, &Value::Null, None));
        }
        assert!(outcome_unknown(JobStatus::Running, &Value::Null, None));
        assert!(outcome_unknown(
            JobStatus::TimedOutUnknown,
            &Value::Null,
            None
        ));
        assert!(outcome_unknown(
            JobStatus::Failed,
            &json!({"owned_processes_closed":false}),
            Some(&json!({"observed_searches":1}))
        ));
        assert!(!outcome_unknown(
            JobStatus::Failed,
            &json!({"owned_processes_closed":true}),
            Some(&json!({"observed_searches":1}))
        ));
        assert!(outcome_unknown(JobStatus::Completed, &Value::Null, None));
    }
}
