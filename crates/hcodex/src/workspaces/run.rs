use crate::execution_policy::ExecutionDeadline;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::types::{WorkflowEvent, WorkflowRequest, WorkspaceKind, WorkspaceRunStatus};
use super::{artifacts, build, planning};
use crate::Result;
use crate::gateway::Gateway;
use crate::storage::NewJob;

const FLOW_SCHEMA: &str = "workspace_flow/v1";

pub async fn run(
    gateway: &Gateway,
    mut request: WorkflowRequest,
    events: Option<mpsc::Sender<WorkflowEvent>>,
) -> Result<Value> {
    validate_request(&request)?;
    request.workspace_dir = request.workspace_dir.canonicalize()?;
    if let Some(research) = &request.research {
        if research.workspace_dir.canonicalize()? != request.workspace_dir {
            return Err("inline research belongs to a different original workspace".into());
        }
        if research.plan_only {
            return Err("a workflow executes selected stages; use a research plan for an effect-free preview".into());
        }
    }
    let flow_id = request
        .request_id
        .clone()
        .unwrap_or_else(|| format!("wsf-{}", uuid::Uuid::new_v4().simple()));
    request.request_id = Some(flow_id.clone());
    // Resolve the external state boundary before even registering a flow.
    artifacts::stage_path(
        &gateway.state_root,
        &request.workspace_dir,
        &flow_id,
        WorkspaceKind::Research,
    )?;
    materialize_child_ids(&mut request, &flow_id);
    let identity = request_identity(&request)?;
    let registration = register(gateway, &request, &flow_id, &identity).await?;
    if !registration.should_dispatch {
        return super::status::status(gateway, &request.workspace_dir, &registration.record.job_id)
            .await;
    }

    let failure_workspace = request.workspace_dir.clone();
    let runner = gateway.clone();
    let owned_id = flow_id.clone();
    let result = gateway
        .supervise(
            flow_id.clone(),
            Box::pin(async move { execute_owned(&runner, request, &owned_id, events).await }),
        )
        .await;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let id = flow_id.clone();
            let mut partial = super::status::status(gateway, &failure_workspace, &flow_id)
                .await
                .unwrap_or_else(|_| json!({"schema_version":FLOW_SCHEMA,"flow_id":flow_id}));
            partial["workflow_status"] = json!("timed_out_unknown");
            partial["error"] = json!(error.to_string());
            let saved = partial.clone();
            gateway
                .database(move |store| {
                    store.mark_timed_out_unknown(
                        &id,
                        "workspace workflow interrupted; inspect original child IDs".into(),
                        Some(saved),
                        None,
                    )?;
                    Ok(())
                })
                .await?;
            Err(error)
        }
    }
}

pub async fn run_with_observer(
    gateway: &Gateway,
    request: WorkflowRequest,
    events: mpsc::Sender<WorkflowEvent>,
) -> Result<Value> {
    run(gateway, request, Some(events)).await
}

pub async fn cancel(gateway: &Gateway, original: &Path, flow_id: &str) -> Result<bool> {
    super::status::cancel(gateway, original, flow_id).await
}

async fn execute_owned(
    gateway: &Gateway,
    request: WorkflowRequest,
    flow_id: &str,
    events: Option<mpsc::Sender<WorkflowEvent>>,
) -> Result<Value> {
    let deadline = ExecutionDeadline::from_limit(Duration::from_secs(request.timeout_seconds));
    let original = request.workspace_dir.as_path();
    let mut results = Vec::new();
    let (mut current_research, input_research_error) =
        if request.stages.contains(&WorkspaceKind::Research) {
            (None, None)
        } else {
            match load_research(gateway, original, request.research_id.as_deref()).await {
                Ok(value) => (value, None),
                Err(error) => (None, Some(error.to_string())),
            }
        };
    let (mut plan, input_plan_error) = match request.plan_id.as_deref() {
        Some(id) => match artifacts::read_plan(gateway, original, id).await {
            Ok(Some(record)) => (Some(record), None),
            Ok(None) => (
                None,
                Some("referenced planning artifact was not found".to_owned()),
            ),
            Err(error) => (None, Some(error.to_string())),
        },
        None => (None, None),
    };
    let mut overall = WorkspaceRunStatus::Completed;

    for stage in &request.stages {
        if deadline.is_elapsed() {
            overall = WorkspaceRunStatus::TimedOutUnknown;
            break;
        }
        if crate::supervisor::is_cancelled() {
            overall = WorkspaceRunStatus::Cancelled;
            break;
        }
        let child_id = child_id(&request, *stage)?;
        emit(
            &events,
            WorkflowEvent::StageStarted {
                flow_id: flow_id.to_owned(),
                stage: *stage,
                child_id: child_id.clone(),
            },
        )
        .await;
        let stage_task = async {
            match stage {
                WorkspaceKind::Research => {
                    let mut research = request.research.clone().unwrap_or_else(|| {
                        crate::research::ResearchRequest::new(
                            request.goal.clone(),
                            original.to_path_buf(),
                        )
                    });
                    research.workspace_dir = original.to_path_buf();
                    research.request_id = Some(child_id.clone());
                    research.idempotency_key = Some(format!("{flow_id}:research"));
                    if let Some(remaining) = deadline.remaining() {
                        let remaining_seconds = remaining.as_secs().max(1);
                        research.budget.timeout_seconds = if research.budget.timeout_seconds == 0 {
                            remaining_seconds
                        } else {
                            research.budget.timeout_seconds.min(remaining_seconds)
                        };
                    }
                    let execution = artifacts::prepare_stage(
                        &gateway.state_root,
                        original,
                        flow_id,
                        WorkspaceKind::Research,
                    )
                    .await?;
                    let scope = crate::routing::digest_output(&format!(
                        "workspace-research/v1\n{}\n{}",
                        flow_id,
                        execution.display()
                    ));
                    let (_cancel, receiver) = watch::channel(false);
                    let value =
                        crate::research::run_scoped(gateway, research, execution, scope, receiver)
                            .await;
                    value.map(|value| {
                        current_research = Some(value.clone());
                        let status = research_status(&value);
                        (status, value)
                    })
                }
                WorkspaceKind::Planning => {
                    if let Some(error) = &input_research_error {
                        Ok((
                            WorkspaceRunStatus::Failed,
                            json!({"ok":false,"error":error,"research_id":request.research_id}),
                        ))
                    } else {
                        let execution = artifacts::prepare_stage(
                            &gateway.state_root,
                            original,
                            flow_id,
                            WorkspaceKind::Planning,
                        )
                        .await?;
                        let record = planning::execute(
                            gateway,
                            flow_id,
                            original,
                            &execution,
                            &request.goal,
                            current_research.as_ref(),
                            &request.planner,
                            deadline,
                        )
                        .await;
                        record.and_then(|record| {
                            let status = record.status;
                            let value = serde_json::to_value(&record)?;
                            plan = Some(record);
                            Ok((status, value))
                        })
                    }
                }
                WorkspaceKind::Building => {
                    if let Some(plan) = plan.as_ref() {
                        match build::execute(
                            gateway,
                            flow_id,
                            original,
                            plan,
                            deadline,
                            events.clone(),
                        )
                        .await
                        {
                            Ok(value) => {
                                let status = if contains_unknown_outcome(&value) {
                                    WorkspaceRunStatus::TimedOutUnknown
                                } else if value["ok"] == true {
                                    WorkspaceRunStatus::Settled
                                } else {
                                    WorkspaceRunStatus::Stopped
                                };
                                Ok((status, value))
                            }
                            Err(error) => classify_build_error(gateway, &child_id, error).await,
                        }
                    } else {
                        Ok((
                            WorkspaceRunStatus::Blocked,
                            json!({"ok":false,"blocked":true,"error":input_plan_error.as_deref().unwrap_or("Building requires an explicit persisted planning artifact")}),
                        ))
                    }
                }
            }
        };
        let stage_outcome = await_stage(gateway, flow_id, &child_id, deadline, stage_task).await;
        let outcome = match stage_outcome {
            Ok(outcome) => outcome,
            Err(error) => classify_child_error(gateway, &child_id, error).await?,
        };
        results.push(json!({
            "stage":stage,
            "job_id":child_id,
            "status":outcome.0,
            "result":outcome.1,
        }));
        emit(
            &events,
            WorkflowEvent::StageFinished {
                flow_id: flow_id.to_owned(),
                stage: *stage,
                status: outcome.0,
                result: outcome.1,
            },
        )
        .await;
        overall = outcome.0;
        if !can_continue(outcome.0) {
            break;
        }
    }

    let result = json!({
        "schema_version":FLOW_SCHEMA,
        "flow_id":flow_id,
        "workspace_dir":original,
        "selected_stages":request.stages,
        "workflow_status":overall,
        "stages":results,
        "research_id":request.research_id,
        "plan_id":plan.as_ref().map(|record|record.plan_id.clone()),
        "building_id":plan.as_ref().map(|_|build::building_job_id(flow_id)),
    });
    persist_terminal(gateway, flow_id, overall, result.clone()).await?;
    Ok(result)
}

async fn await_stage<F>(
    gateway: &Gateway,
    flow_id: &str,
    child_id: &str,
    deadline: impl Into<ExecutionDeadline>,
    future: F,
) -> Result<(WorkspaceRunStatus, Value)>
where
    F: std::future::Future<Output = Result<(WorkspaceRunStatus, Value)>>,
{
    let deadline = deadline.into();
    tokio::pin!(future);
    let timed_out = tokio::select! {
        result=&mut future=>return result,
        ()=crate::supervisor::cancelled()=>false,
        ()=deadline.wait()=>true,
    };
    // Latch the parent and signal the exact known child. Existing Supervisor
    // propagates its cancellation to children admitted during this race.
    gateway.cancel_supervised(flow_id).await;
    gateway.cancel_supervised(child_id).await;
    let drained = future.await;
    let unknown = match &drained {
        Ok((status, value)) => {
            *status == WorkspaceRunStatus::TimedOutUnknown || contains_unknown_outcome(value)
        }
        Err(_) => true,
    };
    let status = if timed_out || unknown {
        WorkspaceRunStatus::TimedOutUnknown
    } else {
        WorkspaceRunStatus::Cancelled
    };
    let value = match drained {
        Ok((_, value)) => {
            json!({"interrupted":true,"timed_out":timed_out,"child_job_id":child_id,"observed_result":value})
        }
        Err(error) => {
            json!({"interrupted":true,"timed_out":timed_out,"child_job_id":child_id,"error":error.to_string()})
        }
    };
    Ok((status, value))
}

async fn classify_build_error(
    gateway: &Gateway,
    child_id: &str,
    error: crate::Error,
) -> Result<(WorkspaceRunStatus, Value)> {
    let id = child_id.to_owned();
    let observed = gateway.database(move |store| store.get_job(&id)).await?;
    if observed.is_none() {
        return Ok((
            WorkspaceRunStatus::Blocked,
            json!({"ok":false,"blocked":true,"error":error.to_string()}),
        ));
    }
    classify_child_error(gateway, child_id, error).await
}

fn can_continue(status: WorkspaceRunStatus) -> bool {
    matches!(
        status,
        WorkspaceRunStatus::Completed
            | WorkspaceRunStatus::IncompleteEvidence
            | WorkspaceRunStatus::BuildReady
            | WorkspaceRunStatus::Settled
    )
}

async fn register(
    gateway: &Gateway,
    request: &WorkflowRequest,
    flow_id: &str,
    identity: &str,
) -> Result<crate::storage::JobRegistration> {
    let mut metadata = BTreeMap::from([
        ("request_identity".into(), identity.to_owned()),
        ("workflow_schema".into(), FLOW_SCHEMA.into()),
        (
            "selected_stages".into(),
            serde_json::to_string(&request.stages)?,
        ),
    ]);
    for stage in &request.stages {
        metadata.insert(
            format!("expected_{}_job_id", stage.as_str()),
            child_id(request, *stage)?,
        );
    }
    if request.stages.contains(&WorkspaceKind::Planning) {
        metadata.insert(
            "expected_plan_artifact_id".into(),
            planning::plan_id(flow_id)?,
        );
    }
    if let Some(plan_id) = &request.plan_id {
        metadata.insert("input_plan_id".into(), plan_id.clone());
    }
    if let Some(research_id) = &request.research_id {
        metadata.insert("input_research_id".into(), research_id.clone());
    }
    let job = NewJob {
        task_type: "workspace_flow".into(),
        workspace_dir: request.workspace_dir.clone(),
        model: None,
        metadata,
        job_id: Some(flow_id.to_owned()),
        idempotency_key: Some(flow_id.to_owned()),
        provider: Some("hcodex_workspaces".into()),
        engine: Some("workspace_flow".into()),
        created_at: None,
    };
    gateway
        .database(move |store| store.resolve_or_register(job, false))
        .await
}

async fn persist_terminal(
    gateway: &Gateway,
    flow_id: &str,
    status: WorkspaceRunStatus,
    result: Value,
) -> Result<()> {
    let id = flow_id.to_owned();
    gateway
        .database(move |store| {
            match status {
                WorkspaceRunStatus::Cancelled => {
                    store.cancel_job_with_result(
                        &id,
                        "workspace workflow cancelled after owned work drained".into(),
                        Some(result),
                        None,
                    )?;
                }
                WorkspaceRunStatus::TimedOutUnknown => {
                    store.mark_timed_out_unknown(
                        &id,
                        "workspace workflow outcome unknown".into(),
                        Some(result),
                        None,
                    )?;
                }
                _ => {
                    store.complete_job(&id, result, None, None)?;
                }
            }
            Ok(())
        })
        .await
}

fn validate_request(request: &WorkflowRequest) -> Result<()> {
    if (request.goal.trim().is_empty() && request.stages != [WorkspaceKind::Building])
        || request.goal.len() > 16_384
    {
        return Err("workflow goal is empty or exceeds its byte budget".into());
    }
    if request.stages.is_empty() || request.stages.len() > 3 {
        return Err("workflow selects one to three stages".into());
    }
    let expected = [
        WorkspaceKind::Research,
        WorkspaceKind::Planning,
        WorkspaceKind::Building,
    ];
    let positions = request
        .stages
        .iter()
        .map(|stage| expected.iter().position(|candidate| candidate == stage))
        .collect::<Option<Vec<_>>>()
        .ok_or("unknown workflow stage")?;
    if positions.windows(2).any(|pair| pair[0] + 1 != pair[1]) {
        return Err("workflow stages must be unique and ordered".into());
    }
    if request.timeout_seconds > 3_600 {
        return Err("workflow timeout exceeds its finite contract".into());
    }
    if request.stages.contains(&WorkspaceKind::Building)
        && !request.stages.contains(&WorkspaceKind::Planning)
        && request.plan_id.is_none()
    {
        return Err("Building-only execution requires --plan".into());
    }
    if request.stages.contains(&WorkspaceKind::Planning) && request.plan_id.is_some() {
        return Err("Planning cannot overwrite or implicitly select an existing plan".into());
    }
    if request.stages.first() == Some(&WorkspaceKind::Planning)
        && request.research_id.is_none()
        && request.plan_id.is_some()
    {
        return Err("an existing plan reference is consumed by Building, not Planning".into());
    }
    if request.stages.contains(&WorkspaceKind::Research) && request.research_id.is_some() {
        return Err("a selected Research stage cannot also import another research ID".into());
    }
    if request.research.is_some() && !request.stages.contains(&WorkspaceKind::Research) {
        return Err("inline research requires an explicit Research stage".into());
    }
    Ok(())
}

fn materialize_child_ids(request: &mut WorkflowRequest, flow_id: &str) {
    if request.stages.contains(&WorkspaceKind::Research) {
        let id = request
            .research
            .as_ref()
            .and_then(|request| request.request_id.clone())
            .unwrap_or_else(|| stable_child_id("wsr", flow_id, "research"));
        request.research_id = Some(id.clone());
        if let Some(research) = request.research.as_mut() {
            research.request_id = Some(id);
        }
    }
}

fn child_id(request: &WorkflowRequest, stage: WorkspaceKind) -> Result<String> {
    match stage {
        WorkspaceKind::Research => request
            .research_id
            .clone()
            .ok_or_else(|| "research stage identity missing".into()),
        WorkspaceKind::Planning => planning::planner_job_id(
            request
                .request_id
                .as_deref()
                .ok_or("flow identity missing")?,
        ),
        WorkspaceKind::Building => Ok(build::building_job_id(
            request
                .request_id
                .as_deref()
                .ok_or("flow identity missing")?,
        )),
    }
}

fn stable_child_id(prefix: &str, flow_id: &str, scope: &str) -> String {
    let digest = crate::routing::digest_output(&format!("workspace-flow/v1\n{flow_id}\n{scope}"));
    format!("{prefix}-{}", &digest.trim_start_matches("sha256:")[..32])
}

fn request_identity(request: &WorkflowRequest) -> Result<String> {
    let mut identity = request.clone();
    identity.request_id = None;
    Ok(crate::routing::digest_output(&serde_json::to_string(&(
        FLOW_SCHEMA,
        identity,
    ))?))
}

async fn load_research(
    gateway: &Gateway,
    original: &Path,
    research_id: Option<&str>,
) -> Result<Option<Value>> {
    match research_id {
        Some(id) => artifacts::read_research(gateway, original, id)
            .await?
            .map(Some)
            .ok_or_else(|| "referenced research campaign is unavailable".into()),
        None => Ok(None),
    }
}

fn research_status(value: &Value) -> WorkspaceRunStatus {
    match value["research_status"].as_str() {
        Some("complete") => WorkspaceRunStatus::Completed,
        Some("incomplete_evidence") => WorkspaceRunStatus::IncompleteEvidence,
        Some("partial") if value["cancelled"] == true => WorkspaceRunStatus::Cancelled,
        Some("partial") => WorkspaceRunStatus::TimedOutUnknown,
        _ => WorkspaceRunStatus::Failed,
    }
}

async fn classify_child_error(
    gateway: &Gateway,
    child_id: &str,
    error: crate::Error,
) -> Result<(WorkspaceRunStatus, Value)> {
    let id = child_id.to_owned();
    let observed = gateway.database(move |store| store.get_job(&id)).await?;
    let status = if observed.as_ref().is_some_and(|record| {
        record
            .receipt
            .as_ref()
            .is_some_and(contains_unknown_outcome)
    }) {
        WorkspaceRunStatus::TimedOutUnknown
    } else {
        match observed.as_ref().map(|record| record.status) {
            Some(crate::storage::JobStatus::Cancelled) => WorkspaceRunStatus::Cancelled,
            Some(crate::storage::JobStatus::TimedOutUnknown)
            | Some(crate::storage::JobStatus::Running) => WorkspaceRunStatus::TimedOutUnknown,
            _ => WorkspaceRunStatus::Failed,
        }
    };
    Ok((
        status,
        json!({
            "ok":false,
            "error":error.to_string(),
            "job_id":child_id,
            "job_status":observed.map(|record|record.status),
        }),
    ))
}

fn contains_unknown_outcome(value: &Value) -> bool {
    let mut remaining = 512;
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        if remaining == 0 {
            return true;
        }
        remaining -= 1;
        match value {
            Value::Object(map) => {
                if map.get("outcome_state").and_then(Value::as_str) == Some("outcome_unknown")
                    || map.get("status").and_then(Value::as_str) == Some("timed_out_unknown")
                    || map.get("owned_processes_closed") == Some(&Value::Bool(false))
                {
                    return true;
                }
                pending.extend(map.values().filter(|v| v.is_object() || v.is_array()));
            }
            Value::Array(items) => {
                pending.extend(items.iter().filter(|v| v.is_object() || v.is_array()))
            }
            _ => {}
        }
    }
    false
}

async fn emit(events: &Option<mpsc::Sender<WorkflowEvent>>, event: WorkflowEvent) {
    if let Some(events) = events {
        let _delivery = events.try_send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_ordered_and_build_never_appears_implicitly() -> Result<()> {
        let root = std::env::current_dir()?;
        for stages in [
            vec![WorkspaceKind::Research],
            vec![WorkspaceKind::Planning],
            vec![WorkspaceKind::Research, WorkspaceKind::Planning],
        ] {
            let request = WorkflowRequest::new("goal", root.clone(), stages);
            validate_request(&request)?;
            assert!(!request.stages.contains(&WorkspaceKind::Building));
        }
        let invalid = WorkflowRequest::new(
            "goal",
            root,
            vec![WorkspaceKind::Building, WorkspaceKind::Planning],
        );
        assert!(validate_request(&invalid).is_err());
        let mut building = WorkflowRequest::new(
            "goal",
            std::env::current_dir()?,
            vec![WorkspaceKind::Building],
        );
        building.plan_id = Some("wpa-existing".into());
        validate_request(&building)?;
        let planning_building = WorkflowRequest::new(
            "goal",
            std::env::current_dir()?,
            vec![WorkspaceKind::Planning, WorkspaceKind::Building],
        );
        validate_request(&planning_building)?;
        let all = WorkflowRequest::new(
            "goal",
            std::env::current_dir()?,
            vec![
                WorkspaceKind::Research,
                WorkspaceKind::Planning,
                WorkspaceKind::Building,
            ],
        );
        validate_request(&all)?;
        Ok(())
    }

    #[tokio::test]
    async fn missing_build_plan_is_durable_blocked_and_same_id_is_readback_only() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let original = temp.path().join("repo");
        std::fs::create_dir(&original)?;
        let gateway = Gateway::new(temp.path().join("state"));
        let mut request = WorkflowRequest::new(
            "build an existing plan",
            original,
            vec![WorkspaceKind::Building],
        );
        request.request_id = Some("flow-missing-plan".into());
        request.plan_id = Some("wpa-missing".into());
        let first = run(&gateway, request.clone(), None).await?;
        assert_eq!(first["workflow_status"], "blocked");
        let before = gateway
            .database(|store| Ok(store.job_telemetry()?.total_jobs))
            .await?;
        let replay = run(&gateway, request, None).await?;
        let after = gateway
            .database(|store| Ok(store.job_telemetry()?.total_jobs))
            .await?;
        assert_eq!(replay["deduplicated_existing_request"], true);
        assert_eq!(before, after);
        gateway.shutdown().await?;
        Ok(())
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod journeys;
