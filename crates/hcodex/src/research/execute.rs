use crate::execution_policy::ExecutionDeadline;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::{
    Assessment, ModelSpec, ReportSection, ResearchClaim, ResearchPhase, ResearchRequest,
    SourceBundle, StageArtifact,
};
use super::{digest, source_job_id, sources, validate};
use crate::{
    Result,
    gateway::Gateway,
    storage::{JobStatus, NewJob},
};

#[derive(Clone, Serialize)]
struct StageRecord {
    phase: ResearchPhase,
    lane_id: String,
    job_id: String,
    requested_model: String,
    receipt: Value,
    artifact: Option<StageArtifact>,
    error: Option<String>,
    outcome_unknown: bool,
    job_observed: bool,
    job_status: Option<JobStatus>,
}
#[derive(Default)]
struct State {
    bundle: SourceBundle,
    discovery: Option<super::discovery::DiscoveryRecord>,
    discovery_rejections: Vec<super::SourceFailure>,
    claims: Vec<ResearchClaim>,
    sections: Vec<ReportSection>,
    unresolved: Vec<String>,
    stages: Vec<StageRecord>,
    unknown: bool,
    cancelled: bool,
    snapshot_committed: bool,
}

pub(super) async fn run(
    gateway: &Gateway,
    mut request: ResearchRequest,
    parent: &str,
    execution_workspace: &std::path::Path,
    cancellation: watch::Receiver<bool>,
) -> Result<Value> {
    let started = Instant::now();
    let deadline =
        ExecutionDeadline::from_limit(Duration::from_secs(request.budget.timeout_seconds));
    let mut state = State::default();
    if cancelled(gateway, parent, &cancellation).await? {
        let result = json!({"schema_version":"research_report/v1","mode":"research","ok":false,"request_id":parent,"research_status":"partial","cancelled":true,"stages":[],"sources":[],"markdown":"Research cancelled before source admission."});
        let saved = result.clone();
        let id = parent.to_owned();
        gateway
            .database(move |store| {
                store.cancel_job_with_result(
                    &id,
                    "research cancelled before source admission".into(),
                    Some(saved),
                    None,
                )?;
                Ok(())
            })
            .await?;
        return Ok(result);
    }
    let before_workspace = request.workspace_dir.clone();
    let before = tokio::task::spawn_blocking(move || {
        crate::dag::DagSettlementVerifier::capture(&before_workspace)
    })
    .await??;
    let outcome=async {
        if record_cancellation_observation(&mut state,cancelled(gateway,parent,&cancellation).await){return Ok(());}
        state.discovery=super::discovery::discover(gateway,&request,parent,deadline).await?;
        if let Some(discovery)=&state.discovery {
            state.unknown |= discovery.unknown;
            state.cancelled |= discovery.cancelled;
            if let Some(error)=&discovery.error {state.unresolved.push(error.clone());return Ok(());}
            request.sources.extend(discovery.candidate_urls.iter().cloned().map(|url|super::SourceSpec::Web{url}));
        }
        if record_cancellation_observation(&mut state,cancelled(gateway,parent,&cancellation).await){return Ok(());}
        state.bundle=observe_sources_before(gateway,&request,parent,Some(sources::source_io_deadline(deadline))).await?;
        state.snapshot_committed=true;
        for failure in &state.bundle.failures {
            if state.discovery.as_ref().is_some_and(|d|d.candidate_urls.contains(&failure.locator)) {
                state.discovery_rejections.push(failure.clone());
            } else { state.unresolved.push(format!("Source acquisition {}: {}",failure.error_code,failure.locator)); }
        }
        if let Some(super::DiscoveryConfig::Web {min_accepted_sources,..})=request.active_discovery() {
            let accepted=state.bundle.sources.iter().filter(|s|state.discovery.as_ref().is_some_and(|d|d.candidate_urls.contains(s.redirect_chain.first().unwrap_or(&s.locator)))).count();
            if accepted < min_accepted_sources {state.unresolved.push(format!("Only {accepted} discovered sources were captured; minimum is {min_accepted_sources}."));return Ok(());}
        }
        if record_cancellation_observation(&mut state,cancelled(gateway,parent,&cancellation).await){return Ok(());}
        if state.bundle.sources.is_empty(){state.unresolved.push("No observed source excerpts are available; no model was called.".into());return Ok(());}
        if request.as_of.is_some(){state.unresolved.push("Historical as-of evidence is unsupported: current retrieval time is not a historical capture.".into());return Ok(());}
        let mut pending=VecDeque::from(request.investigators.clone());
        let mut tasks=JoinSet::new();
        let mut stop=false;
        while !pending.is_empty()||!tasks.is_empty(){
            if record_cancellation_observation(&mut state,cancelled(gateway,parent,&cancellation).await){stop=true;}
            if deadline.is_elapsed(){state.unresolved.push("Research deadline exhausted.".into());stop=true;}
            while !stop && tasks.len()<request.budget.max_parallel {
                if *cancellation.borrow() || crate::supervisor::is_cancelled(){state.cancelled=true;stop=true;break;}
                let Some(lane)=pending.pop_front()else{break;};
                let owned_gateway=gateway.clone();let owned_request=request.clone();let bundle=state.bundle.clone();let owned_parent=parent.to_owned();let owned_execution_workspace=execution_workspace.to_path_buf();
                tasks.spawn(crate::supervisor::inherit_context(async move {
                    stage(&owned_gateway,&owned_request,&owned_parent,&owned_execution_workspace,&bundle,ResearchPhase::Investigate,&lane.lane_id,&ModelSpec{model:lane.model,effort:lane.effort},&lane.focus,&[],deadline).await
                }));
            }
            let Some(joined)=tasks.join_next().await else{break;};
            match joined {
                Ok(Ok(mut record))=>{
                    observe_outcome(&mut state,&record);
                    if let Some(artifact)=record.artifact.as_ref(){
                        if let Err(error)=validate::validate_stage(ResearchPhase::Investigate,artifact,&state.bundle,&[],&request.budget){record.error=Some(error.to_string());record.artifact=None;stop=true;}
                        else {
                            let claims=artifact.claims.iter().cloned().map(|mut claim|{claim.claim_id=format!("{}/{}",record.lane_id,claim.claim_id);claim});
                            state.claims.extend(claims);state.unresolved.extend(artifact.unresolved.clone());
                            if state.claims.len()>request.budget.max_claims{state.unresolved.push("Aggregate claim budget exhausted.".into());stop=true;}
                        }
                    }
                    if let Some(error)=&record.error{state.unresolved.push(format!("{}: {error}",record.lane_id));stop=true;}
                    state.stages.push(record);
                }
                Ok(Err(error))=>{state.unresolved.push(error.to_string());stop=true;}
                Err(error)=>{state.unresolved.push(format!("Owned investigation task failed: {error}"));state.unknown=true;stop=true;}
            }
        }
        if !pending.is_empty(){state.unresolved.push(format!("{} investigation lanes were not admitted.",pending.len()));}
        if state.cancelled||state.unknown||stop||state.claims.is_empty(){return Ok(());}
        for (phase,lane,model,focus) in [
            (ResearchPhase::Challenge,"challenger",&request.challenger,"Review every known claim for counter-evidence, unsupported inference and missing coverage. Return exactly one challenge for every existing claim ID, retaining the assessment with a bounded reason when no counter-evidence is found. Do not create new claims or upgrade assessments."),
            (ResearchPhase::Synthesize,"synthesizer",&request.synthesizer,"Organize the existing claim ledger into nonempty report_sections with known claim IDs only. Include every supported, contested or refuted claim exactly once. A refuted hypothesis is a valid conclusion; the host will label it as refuted, never as an asserted fact. Do not invent, rewrite or upgrade claims. Unresolved claims belong only in the annex."),
        ]{
            if record_cancellation_observation(&mut state,cancelled(gateway,parent,&cancellation).await){break;}
            if deadline.is_elapsed(){state.unresolved.push("Research deadline exhausted before next phase.".into());break;}
            let mut record=stage(gateway,&request,parent,execution_workspace,&state.bundle,phase,lane,model,focus,&state.claims,deadline).await?;
            observe_outcome(&mut state,&record);
            if let Some(artifact)=record.artifact.as_ref(){
                let valid=validate::validate_stage(phase,artifact,&state.bundle,&state.claims,&request.budget);
                if let Err(error)=valid {record.error=Some(error.to_string());}
                else {
                    state.unresolved.extend(artifact.unresolved.clone());
                    match phase{
                        ResearchPhase::Challenge=>validate::apply_challenges(&mut state.claims,artifact)?,
                        ResearchPhase::Synthesize=>state.sections=artifact.report_sections.clone(),
                        ResearchPhase::Investigate=>return Err("unexpected research phase".into()),
                    }
                }
            }
            let stop=record.error.is_some()||state.unknown||state.cancelled;
            if let Some(error)=&record.error{state.unresolved.push(format!("{}: {error}",record.lane_id));}
            state.stages.push(record);
            if stop{break;}
        }
        Ok::<_,crate::Error>(())
    }.await;
    if let Err(error) = outcome {
        state.unresolved.push(error.to_string());
    }
    record_cancellation_observation(&mut state, cancelled(gateway, parent, &cancellation).await);
    let recheck_failures = match sources::recheck_local_sources(&request, &state.bundle).await {
        Ok(failures) => failures,
        Err(error) => vec![super::SourceFailure {
            kind: super::SourceKind::Repository,
            locator: request.workspace_dir.display().to_string(),
            error_code: "source_recheck_unavailable".into(),
            detail: error.to_string(),
        }],
    };
    if !recheck_failures.is_empty() {
        state
            .unresolved
            .push("Declared source files changed after their captured snapshot.".into());
    }
    let after_workspace = request.workspace_dir.clone();
    let after = tokio::task::spawn_blocking(move || {
        crate::dag::DagSettlementVerifier::capture(&after_workspace)
    })
    .await;
    let workspace_unchanged = match after {
        Ok(Ok(after)) => before == after,
        Ok(Err(error)) => {
            state
                .unresolved
                .push(format!("Final workspace observation unavailable: {error}"));
            false
        }
        Err(error) => {
            state
                .unresolved
                .push(format!("Workspace observer failed: {error}"));
            false
        }
    };
    if !workspace_unchanged {
        state
            .unresolved
            .push("Workspace changed during research; source-backed completion is refused.".into());
    }
    let valid_ledger = state.claims.len() <= request.budget.max_claims;
    if !valid_ledger {
        state.claims.truncate(request.budget.max_claims);
        state.sections.clear();
    }
    let complete = !state.cancelled
        && !state.unknown
        && workspace_unchanged
        && valid_ledger
        && recheck_failures.is_empty()
        && state.bundle.failures.len() == state.discovery_rejections.len()
        && state.unresolved.is_empty()
        && !state.sections.is_empty()
        && state
            .claims
            .iter()
            .all(|c| matches!(c.assessment, Assessment::Supported | Assessment::Refuted))
        && state.stages.iter().all(|s| s.error.is_none());
    let research_status = if complete {
        "complete"
    } else if state.cancelled || state.unknown {
        "partial"
    } else {
        "incomplete_evidence"
    };
    let markdown = validate::render_report(
        &state.claims,
        &state.sections,
        &state.unresolved,
        &state.bundle,
    )?;
    let mut result = json!({
        "schema_version":"research_report/v1","mode":"research","ok":complete,"request_id":parent,
        "research_status":research_status,"question":request.question,"workspace_dir":request.workspace_dir,"audience":request.audience,"output_language":request.output_language,
        "as_of":request.as_of,"sources":state.bundle.sources,"source_failures":state.bundle.failures,
        "source_recheck_failures":recheck_failures,
        "discovery":state.discovery,"discovery_rejections":state.discovery_rejections,
        "source_snapshot_digest":state.snapshot_committed.then_some(&state.bundle.snapshot_digest),
        "source_snapshot_job_id":state.snapshot_committed.then(||source_job_id(parent)),
        "claims":state.claims,"report_sections":state.sections,"unresolved":state.unresolved,"stages":state.stages,
        "markdown":markdown,"harness_sha256":validate::harness_sha256(),
        "citation_integrity":if !state.snapshot_committed||state.claims.is_empty(){"not_observed"}else if valid_ledger{"resolved_for_accepted_claims"}else{"budget_exceeded"},
        "semantic_verification":validate::SEMANTIC_VERIFICATION,"workspace_unchanged":workspace_unchanged,
        "budget":request.budget,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "limitations":["Source and quote integrity is not semantic truth or entailment.","Complete means the declared bounded workflow and citation checks completed; it does not prove exhaustive coverage or factual truth.","Automatic search uses bounded observable native web events. Only independently captured pages are source evidence; crawling, PDF parsing and historical capture proof are unsupported.","Provider completion is not research completeness or code/build/test proof."]
    });
    if serde_json::to_vec(&result)?.len() > 2 * 1024 * 1024 {
        result = json!({"schema_version":"research_report/v1","mode":"research","ok":false,"request_id":parent,"research_status":"partial","source_snapshot_job_id":source_job_id(parent),"error":"research_report_byte_budget_exceeded","markdown":"Research report exceeded its storage budget; inspect original source/stage job IDs.","discovery_job_id":state.discovery.as_ref().map(|d|&d.job_id),"stage_job_ids":state.stages.iter().map(|s|&s.job_id).collect::<Vec<_>>()});
    }
    let save = result.clone();
    let id = parent.to_owned();
    let unknown = state.unknown;
    let cancelled = state.cancelled;
    gateway
        .database(move |store| {
            let record = if cancelled {
                store.cancel_job_with_result(
                    &id,
                    "research cancelled after owned work drained".into(),
                    Some(save),
                    None,
                )?
            } else if unknown {
                store.mark_timed_out_unknown(
                    &id,
                    "research provider outcome unknown; query original stage IDs".into(),
                    Some(save),
                    None,
                )?
            } else {
                store.complete_job(&id, save, None, None)?
            };
            let record = record.ok_or("research parent disappeared before persistence")?;
            if record.result.is_none() {
                return Err("research result was not persisted".into());
            }
            Ok(())
        })
        .await?;
    Ok(result)
}

async fn cancelled(
    gateway: &Gateway,
    parent: &str,
    cancel: &watch::Receiver<bool>,
) -> Result<bool> {
    if *cancel.borrow() || crate::supervisor::is_cancelled() {
        return Ok(true);
    }
    let id = parent.to_owned();
    gateway
        .database(move |store| Ok(store.get_job(&id)?.is_some_and(|job| job.cancel_requested)))
        .await
}
fn observe_outcome(state: &mut State, record: &StageRecord) {
    state.unknown |= record.outcome_unknown
        || record.job_status == Some(JobStatus::TimedOutUnknown)
        || record.receipt["status"] == "timed_out_unknown";
    state.cancelled |=
        record.job_status == Some(JobStatus::Cancelled) || record.receipt["status"] == "cancelled";
}

fn record_cancellation_observation(state: &mut State, observation: Result<bool>) -> bool {
    match observation {
        Ok(true) => {
            state.cancelled = true;
            true
        }
        Ok(false) => false,
        Err(error) => {
            state.unknown = true;
            state
                .unresolved
                .push(format!("Cancellation observation unavailable: {error}"));
            true
        }
    }
}

async fn persist_sources(
    gateway: &Gateway,
    request: &ResearchRequest,
    parent: &str,
    bundle: &SourceBundle,
) -> Result<()> {
    let id = source_job_id(parent);
    let snapshot = serde_json::to_value(bundle)?;
    let new = NewJob {
        task_type: "research_sources".into(),
        workspace_dir: request.workspace_dir.clone(),
        model: None,
        metadata: BTreeMap::from([
            (
                "request_identity".into(),
                digest(&(parent, &bundle.snapshot_digest))?,
            ),
            ("parent_job_id".into(), parent.to_owned()),
        ]),
        job_id: Some(id.clone()),
        idempotency_key: Some(id.clone()),
        provider: Some("hcodex_research".into()),
        engine: Some("observed_source_snapshot".into()),
        created_at: None,
    };
    gateway
        .database(move |store| {
            let registered = store.resolve_or_register(new, false)?;
            if !registered.should_dispatch {
                return if registered.record.status == JobStatus::Completed
                    && registered.record.result.as_ref() == Some(&snapshot)
                {
                    Ok(())
                } else {
                    Err("source snapshot identity conflict".into())
                };
            }
            let saved = store
                .complete_job(&id, snapshot, None, None)?
                .ok_or("source snapshot disappeared")?;
            if saved.status != JobStatus::Completed {
                return Err("source snapshot was not committed".into());
            }
            Ok(())
        })
        .await
}

/// This owned boundary commits every completed source observation before the
/// campaign observes cancellation or admits a model request.
#[cfg(test)]
pub(super) async fn observe_sources(
    gateway: &Gateway,
    request: &ResearchRequest,
    parent: &str,
) -> Result<SourceBundle> {
    observe_sources_before(gateway, request, parent, None).await
}

async fn observe_sources_before(
    gateway: &Gateway,
    request: &ResearchRequest,
    parent: &str,
    deadline: Option<Instant>,
) -> Result<SourceBundle> {
    #[cfg(test)]
    let fixture = gateway
        .test_research_sources
        .as_ref()
        .map(|collect| collect(request))
        .transpose()?;
    #[cfg(not(test))]
    let fixture: Option<SourceBundle> = None;
    let bundle = match fixture {
        Some(bundle) => bundle,
        None => match deadline {
            Some(deadline) => sources::collect_before(request, deadline).await?,
            None => sources::collect(request).await?,
        },
    };
    validate::validate_sources(request, &bundle)?;
    persist_sources(gateway, request, parent, &bundle).await?;
    Ok(bundle)
}

#[allow(clippy::too_many_arguments)]
async fn stage(
    gateway: &Gateway,
    request: &ResearchRequest,
    parent: &str,
    execution_workspace: &std::path::Path,
    bundle: &SourceBundle,
    phase: ResearchPhase,
    lane: &str,
    model: &ModelSpec,
    focus: &str,
    claims: &[ResearchClaim],
    deadline: ExecutionDeadline,
) -> Result<StageRecord> {
    let key = digest(&(parent, &bundle.snapshot_digest, phase, lane))?;
    let id = format!("rst-{}", &key.trim_start_matches("sha256:")[..32]);
    let mut record = StageRecord {
        phase,
        lane_id: lane.to_owned(),
        job_id: id.clone(),
        requested_model: model.model.clone(),
        receipt: Value::Null,
        artifact: None,
        error: None,
        outcome_unknown: false,
        job_observed: false,
        job_status: None,
    };
    let Some(remaining) = deadline.remaining_seconds() else {
        record.error = Some("research deadline exhausted before provider admission".into());
        return Ok(record);
    };
    let task = json!({"question":request.question,"audience":request.audience,"output_language":request.output_language,"phase":phase,"lane":lane,"focus":focus,"sources":bundle.sources,"prior_claims":claims,"source_snapshot_digest":bundle.snapshot_digest});
    let args = json!({"task_spec":serde_json::to_string(&task)?,"phase":phase,"source_snapshot_digest":bundle.snapshot_digest,"workspace_dir":execution_workspace,"model":model.model,"reasoning_effort":model.effort,"engine":super::execution_engine(&model.model),"timeout_seconds":remaining,"request_id":id,"idempotency_key":key,"max_input_bytes":2_000_000,"sandbox_mode":"read-only"});
    let response = gateway.call("research_stage", args).await;
    let lookup = id.clone();
    let stored = match gateway.database(move |store| store.get_job(&lookup)).await {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            record.outcome_unknown = true;
            record.error = Some(
                "Research stage has no durable execution observation; inspect its original ID."
                    .into(),
            );
            return Ok(record);
        }
        Err(error) => {
            record.outcome_unknown = true;
            record.error = Some(format!("Research stage readback unavailable: {error}"));
            return Ok(record);
        }
    };
    record.job_observed = true;
    record.job_status = Some(stored.status);
    record.receipt = stored.receipt.clone().unwrap_or(Value::Null);
    if stored.status != JobStatus::Completed
        || record.receipt["actual_model"] != record.receipt["resolved_model"]
        || !record.receipt["actual_model"].is_string()
        || record.receipt["requested_model"] != model.model
        || record.receipt["model_mismatch"] == true
    {
        record.error = Some(format!(
            "Research stage did not produce matching confirmed evidence: {}",
            stored.status.as_str()
        ));
        return Ok(record);
    }
    if matches!(
        record.receipt["engine"].as_str(),
        Some("codex_cli" | "antigravity_cli")
    ) && record.receipt["owned_processes_closed"] != true
    {
        record.outcome_unknown = true;
        record.error = Some(
            "Local CLI process-group closure is unproved; inspect this original stage ID.".into(),
        );
        return Ok(record);
    }
    let artifact = response
        .as_ref()
        .ok()
        .and_then(|value| value.pointer("/result/structured_output"))
        .or_else(|| {
            stored
                .result
                .as_ref()
                .and_then(|v| v.pointer("/result/structured_output"))
        });
    let Some(artifact) = artifact else {
        record.error = Some("Research stage has no structured artifact.".into());
        return Ok(record);
    };
    if serde_json::to_vec(artifact)?.len() > 262_144 {
        record.error = Some("research stage artifact exceeds byte budget".into());
        return Ok(record);
    }
    match serde_json::from_value::<StageArtifact>(artifact.clone()) {
        Ok(value) => record.artifact = Some(value),
        Err(error) => record.error = Some(format!("Invalid research artifact: {error}")),
    }
    Ok(record)
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn cancellation_observation_failure_retains_collected_stage_identity_and_marks_unknown() {
        let mut state = State::default();
        state.stages.push(StageRecord {
            phase: ResearchPhase::Investigate,
            lane_id: "observed".into(),
            job_id: "original-stage-id".into(),
            requested_model: "gemini-4-argon-high".into(),
            receipt: Value::Null,
            artifact: None,
            error: None,
            outcome_unknown: false,
            job_observed: true,
            job_status: Some(JobStatus::Completed),
        });
        assert!(record_cancellation_observation(
            &mut state,
            Err("SQLite read observation failed".into())
        ));
        assert!(state.unknown);
        assert!(!state.cancelled);
        assert_eq!(state.stages[0].job_id, "original-stage-id");
        assert!(state.unresolved[0].contains("observation unavailable"));
    }
}
