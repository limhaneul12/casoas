use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::Result;
use crate::gateway::Gateway;
use crate::storage::NewJob;

use super::checkpoint::CheckpointContext;
use super::execution::{DagExecutionEvent, DagExecutionOptions, execute_with_options};
use super::handoff::AgentReport;
use super::{
    DagNode, DagPlan, DagPlanner, DagSettlementVerifier, HazardKind, NodeKind, NodeRole, NodeState,
    SettlementReceipt,
};

mod resume;
pub use resume::{PreparedResume, prepare_resume_request};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RequestRole {
    TopologyPlanner,
    #[serde(alias = "contract_verifier")]
    ReproductionProber,
    #[serde(alias = "boundary_mapper")]
    InterfaceContractor,
    #[default]
    #[serde(alias = "interface_builder")]
    DisjointBuilder,
    #[serde(alias = "security_auditor")]
    AdversarialAuditor,
    #[serde(alias = "integration_fan_in")]
    VerificationFanIn,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RequestNodeKind {
    #[default]
    Standard,
    Reproduction,
    Verification,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NodeRequest {
    node_id: String,
    #[serde(default)]
    role: RequestRole,
    #[serde(default)]
    node_kind: RequestNodeKind,
    task: String,
    #[serde(default = "default_deliverable")]
    deliverable: String,
    #[serde(default)]
    read_files: Vec<PathBuf>,
    #[serde(default)]
    write_files: Vec<PathBuf>,
    #[serde(default)]
    symbols: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    verify_cmd: String,
    #[serde(default = "default_stop_when")]
    stop_when: String,
    preferred_model: Option<String>,
    reasoning_effort: Option<String>,
    #[serde(default = "default_true")]
    reproduction_expected_exit_nonzero: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum IsolationMode {
    #[default]
    Direct,
    Serial,
    Worktree,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SpawnPlanRequest {
    plan_summary: String,
    #[serde(default = "default_true")]
    hazard_acknowledgment: bool,
    #[serde(default)]
    isolation_mode: IsolationMode,
    #[serde(default = "default_parallel")]
    max_parallel_per_wave: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DagRequest {
    campaign_goal: String,
    #[serde(default)]
    nodes: Option<Vec<NodeRequest>>,
    #[serde(default = "default_workspace")]
    workspace_dir: PathBuf,
    #[serde(default = "default_true")]
    plan_only: bool,
    #[serde(default)]
    spawn_plan: Option<SpawnPlanRequest>,
    #[serde(default)]
    resume_dag_id: Option<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: f64,
    #[serde(default = "default_max_input")]
    max_input_bytes: u64,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

pub async fn handle_request(gateway: &Gateway, args: &Value) -> Result<Value> {
    handle_request_with_observer(gateway, args, None).await
}

pub async fn handle_request_with_observer(
    gateway: &Gateway,
    args: &Value,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
) -> Result<Value> {
    handle_request_inner(
        gateway,
        args,
        observer,
        #[cfg(test)]
        None,
    )
    .await
}

#[cfg(test)]
pub(super) async fn handle_request_with_worktree_hook(
    gateway: &Gateway,
    args: &Value,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
    control: super::worktrees::TestControl,
) -> Result<Value> {
    handle_request_inner(gateway, args, observer, Some(control)).await
}

async fn handle_request_inner(
    gateway: &Gateway,
    args: &Value,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
    #[cfg(test)] test_control: Option<super::worktrees::TestControl>,
) -> Result<Value> {
    let request: DagRequest = serde_json::from_value(args.clone())?;
    validate_request(&request)?;
    if !request.plan_only {
        let spawn = request
            .spawn_plan
            .as_ref()
            .ok_or("spawn_plan is required when plan_only is false")?;
        if !spawn.hazard_acknowledgment {
            return Err("spawn_plan.hazard_acknowledgment must be true".into());
        }
    }
    let workspace = request.workspace_dir.canonicalize()?;
    let worktree_mode = request
        .spawn_plan
        .as_ref()
        .is_some_and(|spawn| matches!(spawn.isolation_mode, IsolationMode::Worktree));
    if worktree_mode && !request.plan_only {
        super::worktrees::validate_source_budget(&workspace)?;
    }
    let config_identity = resume::config_identity(&request)?;
    if worktree_mode
        && !request.plan_only
        && let Some(result) =
            resume::early_worktree_readback(gateway, &request, &workspace, &config_identity).await?
    {
        return Ok(result);
    }
    let planning_workspace = workspace.clone();
    let goal = request.campaign_goal.clone();
    let nodes = request
        .nodes
        .as_ref()
        .map(|nodes| nodes.iter().map(convert_node).collect());
    let mut plan = if worktree_mode && request.resume_dag_id.is_some() {
        let (_, plan, _) = resume::load_plan(
            gateway,
            &workspace,
            request
                .resume_dag_id
                .as_deref()
                .ok_or("resume source missing")?,
        )
        .await?;
        plan
    } else {
        tokio::task::spawn_blocking(move || match nodes {
            Some(nodes) => DagPlanner::compile_with_workspace(&goal, nodes, &planning_workspace),
            None => DagPlanner::generate(&goal, &planning_workspace),
        })
        .await??
    };
    let force_serial = request
        .spawn_plan
        .as_ref()
        .is_some_and(|spawn| matches!(spawn.isolation_mode, IsolationMode::Serial));
    if force_serial {
        plan.hazards.retain(|hazard| {
            !matches!(
                hazard.kind,
                HazardKind::Waw | HazardKind::Raw | HazardKind::War
            )
        });
        plan.blocking_hazards_count = plan.hazards.len();
    }
    let plan_identity = digest_json(&plan)?;
    let initial_workspace = workspace.clone();
    let initial_manifest =
        tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&initial_workspace))
            .await??;
    let (retained_worktrees, resumed) = if worktree_mode && request.resume_dag_id.is_some() {
        let (campaign, receipts) =
            resume::reopen_worktrees(gateway, &request, &workspace, &plan, &config_identity)
                .await?;
        (Some(campaign), receipts)
    } else {
        (
            None,
            load_resume(
                gateway,
                request.resume_dag_id.as_deref(),
                &workspace,
                &plan_identity,
                &config_identity,
                &initial_manifest.digest,
            )
            .await?,
        )
    };
    let resumed_nodes: Vec<String> = resumed.keys().cloned().collect();
    let registration = register_campaign(
        gateway,
        &request,
        &workspace,
        &plan_identity,
        &config_identity,
        &initial_manifest.digest,
    )
    .await?;
    if !registration.1 {
        return registration
            .0
            .result
            .ok_or_else(|| "deduplicated DAG campaign has no persisted result".into());
    }
    let dag_id = registration.0.job_id;
    // Validate the durable handoff contract and historical resume input before
    // creating worktrees or admitting any provider effects.
    let checkpoint_context = if !request.plan_only && plan.is_executable() {
        let context = CheckpointContext::new(
            gateway.clone(),
            workspace.clone(),
            dag_id.clone(),
            plan_identity.clone(),
            config_identity.clone(),
            &plan,
            request.resume_dag_id.clone(),
            resumed.keys().cloned().collect(),
        );
        let context = match context {
            Ok(context) => context,
            Err(error) => {
                let id = dag_id.clone();
                let message = error.to_string();
                gateway
                    .database(move |store| {
                        store.reject_before_dispatch(&id, message, None)?;
                        Ok(())
                    })
                    .await?;
                return Err(error);
            }
        };
        if let Err(error) = context.preflight_resumed().await {
            let id = dag_id.clone();
            let message = error.to_string();
            gateway
                .database(move |store| {
                    store.reject_before_dispatch(&id, message, None)?;
                    Ok(())
                })
                .await?;
            return Err(error);
        }
        Some(context)
    } else {
        None
    };
    let execution_gateway = gateway.clone();
    let campaign_id = dag_id.clone();
    let outcome = gateway.supervise(dag_id.clone(), Box::pin(async move {
        let gateway = &execution_gateway;
        let dag_id = campaign_id;
        let worktrees = if worktree_mode && !request.plan_only && plan.is_executable() {
            let mut campaign = if let Some(campaign) = retained_worktrees {
                campaign
            } else {
                let pending = plan.nodes.iter().map(|node|node.id.clone()).collect::<Vec<_>>();
                match super::worktrees::WorktreeCampaign::start_registered(&workspace, &gateway.state_root, &pending,
                    Duration::from_secs_f64(request.timeout_seconds.min(30.0)), &dag_id, Some(gateway), #[cfg(test)] test_control.clone()).await {
                    Ok(campaign) => campaign,
                    Err(error) => {
                        let id=dag_id.clone();let message=error.to_string();
                        gateway.database(move|store|{store.reject_before_dispatch(&id,message,None)?;Ok(())}).await?;
                        return Err(error);
                    }
                }
            };
            #[cfg(test)]
            if let Some(control) = &test_control { campaign.set_test_control(control.clone())?; }
            if request.resume_dag_id.is_some() {
                campaign.activate(gateway, &dag_id).await?;
            } else {
                campaign.initialize_frontier(gateway).await?;
            }
            let campaign = Arc::new(campaign);
            if let Some(context) = &checkpoint_context { context.bind_worktrees(campaign.clone())?; }
            Some(campaign)
        } else { None };
        let mut response = if request.plan_only || !plan.is_executable() {
            json!({
                "ok": plan.is_executable(),
                "dag_id": dag_id,
                "run_state": if plan.is_executable() { "planned" } else { "rejected_preflight" },
                "plan_only": request.plan_only,
                "workspace_dir": workspace,
                "compiled_plan": plan,
                "node_receipts": [],
                "settled_count": 0,
                "resumed_nodes": resumed_nodes,
                "aimd_final_concurrency": if !plan.is_executable() { 0 } else if force_serial { 1 } else { request.spawn_plan.as_ref().map_or(3, |spawn| spawn.max_parallel_per_wave) },
            })
        } else {
            let spawn = request
                .spawn_plan
                .as_ref()
                .ok_or("validated spawn_plan missing")?;
            persist_plan_checkpoint(
                gateway,
                &dag_id,
                &workspace,
                &plan,
                &plan_identity,
                &config_identity,
                &initial_manifest.digest,
                resume::recipe_value(&request)?,
            )
            .await?;
            let (event_tx, event_rx) = mpsc::channel(32);
            let checkpoint_context = checkpoint_context.ok_or("validated checkpoint context missing")?;
            let checkpoint_gateway = gateway.clone();
            let checkpoint_dag_id = dag_id.clone();
            let checkpoint_workspace = workspace.clone();
            let checkpoint_plan_identity = plan_identity.clone();
            let checkpoint_config_identity = config_identity.clone();
                let checkpoint_future = persist_node_checkpoints(
                checkpoint_gateway,
                event_rx,
                checkpoint_dag_id,
                checkpoint_workspace,
                checkpoint_plan_identity,
                checkpoint_config_identity,
                observer,
                Some(checkpoint_context.clone()),
            );
            let failure_context = checkpoint_context.clone();
            let checkpoint_future = async move {
                let result = checkpoint_future.await;
                if let Err(error) = &result { failure_context.fail_pending(&error.to_string()); }
                result
            };
            let checkpoint_future = cancel_on_checkpoint_failure(gateway, &dag_id, checkpoint_future);
            let execution_future = execute_with_options(
                gateway,
                plan.clone(),
                workspace.clone(),
                Duration::from_secs_f64(request.timeout_seconds),
                DagExecutionOptions {
                    max_parallel: spawn.max_parallel_per_wave,
                    max_input_bytes: request.max_input_bytes,
                    force_serial,
                    resumed,
                    worktrees: worktrees.clone(),
                    checkpoints: Some(checkpoint_context),
                },
                Some(event_tx),
            );
            // Keep both futures owned until they finish. In particular, a failed
            // checkpoint write must not drop durable provider waiters mid-dispatch.
            let (execution, checkpoints) = tokio::join!(execution_future, checkpoint_future);
            checkpoints?;
            execution?
        };
        let final_workspace = workspace.clone();
        let final_manifest =
            tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&final_workspace))
                .await??;
        if response["ok"] == true && let Some(campaign) = &worktrees {
            campaign.verify_final(&final_manifest).await?;
        }
        normalize_response(
            &mut response,
            &dag_id,
            &workspace,
            &plan,
            &plan_identity,
            &config_identity,
            &final_manifest.digest,
            &resumed_nodes,
            request.plan_only,
        )?;
        if let Some(campaign) = &worktrees {
            response["worktrees"] = json!({"status":"retained", "report":campaign.report()?});
            if let Some(reference)=campaign.previous_export() {response["resumed_execution_export"]=reference;}
            // Save full execution output/checkpoints outside disposable worktrees
            // before allowing deletion. Final campaign result includes actual cleanup.
            let export_id = format!("ckp-{dag_id}-worktree-export");
            response=campaign.prepare_export(gateway,&export_id,response).await?;
            let export_id = persist_worktree_export(gateway, &dag_id, &workspace, response.clone()).await?;
            #[cfg(test)]
            campaign.test_boundary("", super::worktrees::TestBoundary::ExportCommittedBeforeAck);
            let cleanup = if response["ok"] == true { campaign.cleanup_verified().await }
                else { Err("campaign verification incomplete; worktrees retained".into()) };
            response["worktrees"] = match cleanup {
                Ok(report) => json!({"status":"removed","report":report,"execution_export_id":export_id}),
                Err(error) => json!({"status":if response["ok"]==true {"cleanup_unknown"}else{"retained"},"report":campaign.report()?,"execution_export_id":export_id,"reason":error.to_string()}),
            };
        }
        persist_result(gateway, &dag_id, response.clone()).await?;
        Ok(response)
    })).await;
    match outcome {
        Ok(response) => Ok(response),
        Err(error) => {
            let message =
                format!("DAG interrupted; inspect this campaign and its node checkpoints: {error}");
            let interrupted_id = dag_id.clone();
            gateway
                .database(move |store| {
                    store.mark_timed_out_unknown(&interrupted_id, message, None, None)?;
                    Ok(())
                })
                .await?;
            Err(error)
        }
    }
}

async fn persist_worktree_export(
    gateway: &Gateway,
    dag_id: &str,
    workspace: &Path,
    result: Value,
) -> Result<String> {
    let id = format!("ckp-{dag_id}-worktree-export");
    let key = id.clone();
    let workspace = workspace.to_path_buf();
    gateway
        .database(move |store| {
            let registration = store.resolve_or_register(
                NewJob {
                    task_type: "dag_worktree_export".into(),
                    workspace_dir: workspace,
                    model: Some("dag-export".into()),
                    metadata: BTreeMap::new(),
                    job_id: Some(key.clone()),
                    idempotency_key: Some(key.clone()),
                    provider: Some("heterarchy_triad".into()),
                    engine: Some("dag_ulw".into()),
                    created_at: None,
                },
                false,
            )?;
            let saved = if registration.should_dispatch {
                store
                    .complete_job(&key, result.clone(), None, None)?
                    .ok_or("worktree export record disappeared")?
            } else {
                registration.record
            };
            if saved.status != crate::storage::JobStatus::Completed
                || saved.result.as_ref() != Some(&result)
            {
                return Err("worktree execution export is not durably confirmed".into());
            }
            Ok(())
        })
        .await?;
    Ok(id)
}

async fn cancel_on_checkpoint_failure<F>(
    gateway: &Gateway,
    campaign_id: &str,
    checkpoints: F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    let result = checkpoints.await;
    if result.is_err() {
        // Signal the existing campaign owner without fabricating a user cancel
        // in storage. Its caller records the interruption as outcome unknown.
        gateway.cancel_supervised(campaign_id).await;
    }
    result
}

pub async fn prepare_interactive_request(args: &Value) -> Result<Value> {
    let request: DagRequest = serde_json::from_value(args.clone())?;
    validate_request(&request)?;
    if request.plan_only || request.spawn_plan.is_some() {
        return Ok(args.clone());
    }
    let goal = request.campaign_goal.clone();
    let workspace = request.workspace_dir.canonicalize()?;
    let explicit_nodes = request
        .nodes
        .as_ref()
        .map(|nodes| nodes.iter().map(convert_node).collect::<Vec<_>>());
    let plan = tokio::task::spawn_blocking(move || match explicit_nodes {
        Some(nodes) => DagPlanner::compile_with_workspace(&goal, nodes, &workspace),
        None => DagPlanner::generate(&goal, &workspace),
    })
    .await??;
    let only_serializable_hazards = plan.hazards.iter().all(|hazard| {
        matches!(
            hazard.kind,
            HazardKind::Waw | HazardKind::Raw | HazardKind::War
        )
    });
    if !plan.hazards.is_empty() && !only_serializable_hazards {
        return Err("interactive auto-spawn rejected structural DAG hazards".into());
    }
    let max_parallel = plan
        .waves
        .iter()
        .map(|wave| wave.node_ids.len())
        .max()
        .unwrap_or(1)
        .clamp(1, 6);
    let mut prepared = args.clone();
    let object = prepared
        .as_object_mut()
        .ok_or("interactive DAG arguments must be an object")?;
    object.insert(
        "spawn_plan".to_owned(),
        json!({
            "plan_summary": format!(
                "Validated interactive DAG: {} nodes in {} waves",
                plan.nodes.len(),
                plan.waves.len()
            ),
            "hazard_acknowledgment": true,
            "isolation_mode": if plan.hazards.is_empty() { "direct" } else { "serial" },
            "max_parallel_per_wave": max_parallel,
        }),
    );
    Ok(prepared)
}

fn validate_request(request: &DagRequest) -> Result<()> {
    if !request.plan_only
        && request.resume_dag_id.is_some()
        && request
            .spawn_plan
            .as_ref()
            .is_some_and(|p| matches!(p.isolation_mode, IsolationMode::Worktree))
        && (request.request_id.is_none() || request.request_id == request.resume_dag_id)
    {
        return Err("worktree resume requires a distinct continuation request_id".into());
    }
    if request.campaign_goal.trim().is_empty() {
        return Err("campaign_goal must not be empty".into());
    }
    if !(0.0 < request.timeout_seconds && request.timeout_seconds <= 86_400.0) {
        return Err("timeout_seconds must be in (0, 86400]".into());
    }
    if !(1_024..=2_000_000).contains(&request.max_input_bytes) {
        return Err("max_input_bytes must be between 1024 and 2000000".into());
    }
    if let Some(spawn) = &request.spawn_plan
        && (spawn.plan_summary.trim().is_empty() || !(1..=6).contains(&spawn.max_parallel_per_wave))
    {
        return Err("spawn_plan summary/max_parallel_per_wave is invalid".into());
    }
    if let Some(nodes) = &request.nodes {
        for node in nodes {
            if node.node_id.is_empty() || node.node_id.len() > 64 || node.task.trim().is_empty() {
                return Err("DAG node_id/task is invalid".into());
            }
        }
    }
    Ok(())
}

fn convert_node(node: &NodeRequest) -> DagNode {
    let role = match node.role {
        RequestRole::TopologyPlanner => NodeRole::TopologyPlanner,
        RequestRole::ReproductionProber => NodeRole::ContractVerifier,
        RequestRole::InterfaceContractor => NodeRole::BoundaryMapper,
        RequestRole::DisjointBuilder => NodeRole::InterfaceBuilder,
        RequestRole::AdversarialAuditor => NodeRole::SecurityAuditor,
        RequestRole::VerificationFanIn => NodeRole::IntegrationFanIn,
    };
    let (default_model, default_effort) = defaults_for_role(&role);
    let node_kind = match node.node_kind {
        RequestNodeKind::Standard => NodeKind::Standard,
        RequestNodeKind::Reproduction => NodeKind::Reproduction,
        RequestNodeKind::Verification => NodeKind::Verification,
    };
    DagNode {
        id: node.node_id.clone(),
        title: node.deliverable.clone(),
        task: format!(
            "{}\nDeliverable: {}\nNode kind: {:?}\nStop when: {}\nReproduction expects nonzero: {}",
            node.task,
            node.deliverable,
            node.node_kind,
            node.stop_when,
            node.reproduction_expected_exit_nonzero,
        ),
        role,
        node_kind,
        model: node
            .preferred_model
            .clone()
            .unwrap_or_else(|| default_model.to_owned()),
        reasoning_effort: node
            .reasoning_effort
            .clone()
            .unwrap_or_else(|| default_effort.to_owned()),
        dependencies: node.depends_on.clone(),
        read_files: node.read_files.clone(),
        write_files: node.write_files.clone(),
        symbols: node.symbols.clone(),
        verify_command: (!node.verify_cmd.trim().is_empty()).then(|| node.verify_cmd.clone()),
        reproduction_expected_exit_nonzero: node.reproduction_expected_exit_nonzero,
    }
}

fn defaults_for_role(role: &NodeRole) -> (&'static str, &'static str) {
    match role {
        NodeRole::TopologyPlanner | NodeRole::IntegrationFanIn => ("gemini-4-argon-high", "high"),
        NodeRole::InterfaceBuilder => ("claude-sonnet-5-5-medium", "medium"),
        NodeRole::BoundaryMapper => ("gpt-6-luna", "max"),
        NodeRole::ContractVerifier => ("gpt-6.1-sol", "xhigh"),
        NodeRole::SecurityAuditor => ("gpt-6-astra", "xhigh"),
    }
}

async fn register_campaign(
    gateway: &Gateway,
    request: &DagRequest,
    workspace: &Path,
    plan_identity: &str,
    config_identity: &str,
    source_identity: &str,
) -> Result<(crate::storage::JobRecord, bool)> {
    let mut metadata = BTreeMap::new();
    metadata.insert("plan_identity".to_owned(), plan_identity.to_owned());
    metadata.insert("config_identity".to_owned(), config_identity.to_owned());
    metadata.insert("source_identity".to_owned(), source_identity.to_owned());
    if let Some(resume_id) = &request.resume_dag_id {
        metadata.insert("resume_dag_id".into(), resume_id.clone());
    }
    let new_job = NewJob {
        task_type: "dag_ulw".to_owned(),
        workspace_dir: workspace.to_path_buf(),
        model: Some("triad_dag_ulw".to_owned()),
        metadata,
        job_id: request.request_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        provider: Some("heterarchy_triad".to_owned()),
        engine: Some("dag_ulw".to_owned()),
        created_at: None,
    };
    let worktree_mode = request
        .spawn_plan
        .as_ref()
        .is_some_and(|spawn| matches!(spawn.isolation_mode, IsolationMode::Worktree));
    let registration = gateway
        .database(move |store| {
            if worktree_mode {
                store.register_worktree_campaign(new_job)
            } else {
                store.resolve_or_register(new_job, false)
            }
        })
        .await?;
    Ok((registration.record, registration.should_dispatch))
}

#[allow(clippy::too_many_arguments)]
async fn persist_plan_checkpoint(
    gateway: &Gateway,
    campaign_id: &str,
    workspace: &Path,
    plan: &DagPlan,
    plan_identity: &str,
    config_identity: &str,
    source_identity: &str,
    request_recipe: Value,
) -> Result<()> {
    let mut metadata = checkpoint_metadata(
        campaign_id,
        plan_identity,
        config_identity,
        source_identity,
        "plan",
    );
    metadata.insert("checkpoint_kind".to_owned(), "plan".to_owned());
    let checkpoint_id = format!("ckp-{campaign_id}-plan");
    let source_identity = source_identity.to_owned();
    let new_job = NewJob {
        task_type: "dag_checkpoint".to_owned(),
        workspace_dir: workspace.to_path_buf(),
        model: Some("dag-plan".to_owned()),
        metadata,
        job_id: Some(checkpoint_id.clone()),
        idempotency_key: Some(checkpoint_id.clone()),
        provider: Some("heterarchy_triad".to_owned()),
        engine: Some("dag_ulw".to_owned()),
        created_at: None,
    };
    let plan = serde_json::to_value(plan)?;
    gateway
        .database(move |store| {
            store.complete_checkpoint_once(
                new_job,
                json!({"plan": plan, "source_identity": source_identity,"request_recipe":request_recipe}),
            )?;
            Ok(())
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn persist_node_checkpoints(
    gateway: Gateway,
    mut events: mpsc::Receiver<DagExecutionEvent>,
    campaign_id: String,
    workspace: PathBuf,
    plan_identity: String,
    config_identity: String,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
    context: Option<Arc<CheckpointContext>>,
) -> Result<()> {
    let mut reports = BTreeMap::new();
    while let Some(event) = events.recv().await {
        if let DagExecutionEvent::ProviderFinished { node_id, output } = &event {
            reports.insert(node_id.clone(), AgentReport::from_output("", Some(output)));
        }
        let DagExecutionEvent::NodeSettled { receipt } = &event else {
            if let Some(sender) = &observer {
                let _ignored_observer_backpressure = sender.try_send(event);
            }
            continue;
        };
        let receipt = receipt.clone();
        if let Some(context) = &context {
            let report = reports.remove(&receipt.node_id);
            let report = report.map_or_else(
                || AgentReport::from_output(&receipt.output_summary, None),
                |report| report.with_summary(&receipt.output_summary),
            );
            context.persist(receipt, report).await?;
            if let Some(sender) = &observer {
                // Persistence and commit notification precede UI backpressure;
                // terminal transitions must not silently disappear.
                let _ignored_closed_observer = sender.send(event).await;
            }
            continue;
        }
        let checkpoint_id = format!("ckp-{campaign_id}-{}", receipt.node_id);
        let mut metadata = checkpoint_metadata(
            &campaign_id,
            &plan_identity,
            &config_identity,
            &receipt.stopped_manifest_digest,
            "node",
        );
        metadata.insert("node_id".to_owned(), receipt.node_id.clone());
        let new_job = NewJob {
            task_type: "dag_checkpoint".to_owned(),
            workspace_dir: workspace.clone(),
            model: Some("dag-node".to_owned()),
            metadata,
            job_id: Some(checkpoint_id.clone()),
            idempotency_key: Some(checkpoint_id),
            provider: Some("heterarchy_triad".to_owned()),
            engine: Some("dag_ulw".to_owned()),
            created_at: None,
        };
        gateway
            .database(move |store| {
                store.complete_checkpoint_once(new_job, json!({"receipt": receipt}))?;
                Ok(())
            })
            .await?;
        // A visible settlement is backed by its durable checkpoint. Observer
        // backpressure must not stand between receipt creation and persistence.
        if let Some(sender) = &observer {
            let _ignored_closed_observer = sender.send(event).await;
        }
    }
    Ok(())
}

pub(super) fn checkpoint_metadata(
    campaign_id: &str,
    plan_identity: &str,
    config_identity: &str,
    source_identity: &str,
    kind: &str,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("campaign_id".to_owned(), campaign_id.to_owned()),
        ("plan_identity".to_owned(), plan_identity.to_owned()),
        ("config_identity".to_owned(), config_identity.to_owned()),
        ("source_identity".to_owned(), source_identity.to_owned()),
        ("checkpoint_kind".to_owned(), kind.to_owned()),
    ])
}

async fn load_resume(
    gateway: &Gateway,
    resume_id: Option<&str>,
    workspace: &Path,
    plan_identity: &str,
    config_identity: &str,
    source_identity: &str,
) -> Result<BTreeMap<String, SettlementReceipt>> {
    let Some(resume_id) = resume_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(BTreeMap::new());
    };
    let resume_id = resume_id.to_owned();
    let lookup_id = resume_id.clone();
    let prior = gateway
        .database(move |store| store.get_job(&lookup_id))
        .await?;
    let Some(prior) = prior else {
        return Err("resume_dag_id was not found".into());
    };
    if prior.task_type != "dag_ulw"
        || prior.metadata.get("plan_identity").map(String::as_str) != Some(plan_identity)
        || prior.metadata.get("config_identity").map(String::as_str) != Some(config_identity)
    {
        return Err("resume DAG plan/config identity mismatch".into());
    }
    if matches!(
        prior.status,
        crate::storage::JobStatus::Running | crate::storage::JobStatus::TimedOutUnknown
    ) || prior.result.is_none()
    {
        return Err("direct resume has no confirmed provider frontier; inspect original campaign/provider IDs".into());
    }
    if let Some(result) = &prior.result {
        reject_unresolved_direct_outputs(gateway, result).await?;
    }
    let mut receipts: Vec<SettlementReceipt> = Vec::new();
    let final_source = if let Some(result) = prior.result {
        if result["workspace_dir"] != workspace.to_string_lossy().as_ref() {
            return Err("resume DAG workspace identity mismatch".into());
        }
        receipts = serde_json::from_value(result["node_receipts"].clone())?;
        result["final_manifest_digest"].as_str().map(str::to_owned)
    } else {
        None
    };
    let workspace_filter = workspace.to_string_lossy().into_owned();
    let checkpoints = gateway
        .database(move |store| store.list_jobs(1_000, Some(&workspace_filter)))
        .await?;
    let matching: Vec<_> = checkpoints
        .into_iter()
        .filter(|job| {
            job.task_type == "dag_checkpoint"
                && job.metadata.get("campaign_id") == Some(&resume_id)
                && job.metadata.get("plan_identity").map(String::as_str) == Some(plan_identity)
                && job.metadata.get("config_identity").map(String::as_str) == Some(config_identity)
        })
        .collect();
    let checkpoint_source = matching
        .iter()
        .find(|job| job.metadata.get("checkpoint_kind").map(String::as_str) == Some("node"))
        .and_then(|job| job.metadata.get("source_identity"))
        .cloned();
    if final_source.or(checkpoint_source).as_deref() != Some(source_identity) {
        return Err("resume DAG source identity mismatch".into());
    }
    if receipts.is_empty() {
        for checkpoint in matching {
            if let Some(result) = checkpoint.result
                && let Ok(receipt) = serde_json::from_value(result["receipt"].clone())
            {
                receipts.push(receipt);
            }
        }
    }
    Ok(receipts
        .into_iter()
        .filter(|receipt| receipt.state == NodeState::Settled)
        .map(|receipt| (receipt.node_id.clone(), receipt))
        .collect())
}

async fn reject_unresolved_direct_outputs(gateway: &Gateway, result: &Value) -> Result<()> {
    let mut provider_ids = std::collections::BTreeSet::new();
    if let Some(outputs) = result.get("outputs").and_then(Value::as_object) {
        for output in outputs.values() {
            for pointer in [
                "",
                "/receipt",
                "/result",
                "/result/receipt",
                "/result/result/receipt",
            ] {
                let Some(value) = output.pointer(pointer) else {
                    continue;
                };
                let status = value.get("status").and_then(Value::as_str);
                let outcome = value.get("outcome_state").and_then(Value::as_str);
                if matches!(status, Some("running" | "queued" | "timed_out_unknown"))
                    || matches!(outcome, Some("in_flight" | "outcome_unknown"))
                {
                    return Err("direct resume contains an unresolved provider outcome; inspect its original ID".into());
                }
                for key in ["job_id", "request_id"] {
                    if let Some(id) = value.get(key).and_then(Value::as_str) {
                        provider_ids.insert(id.to_owned());
                    }
                }
            }
        }
    }
    gateway
        .database(move |store| {
            for id in provider_ids {
                if let Some(record) = store.get_job(&id)?
                    && matches!(
                        record.status,
                        crate::storage::JobStatus::Running
                            | crate::storage::JobStatus::TimedOutUnknown
                    )
                {
                    return Err(
                        "direct resume provider record is unresolved; inspect its original ID"
                            .into(),
                    );
                }
            }
            Ok(())
        })
        .await
}

#[allow(clippy::too_many_arguments)]
fn normalize_response(
    response: &mut Value,
    dag_id: &str,
    workspace: &Path,
    plan: &DagPlan,
    plan_identity: &str,
    config_identity: &str,
    final_manifest_digest: &str,
    resumed_nodes: &[String],
    plan_only: bool,
) -> Result<()> {
    let object = response
        .as_object_mut()
        .ok_or("DAG result must be an object")?;
    object.insert("dag_id".to_owned(), Value::String(dag_id.to_owned()));
    object.insert("workspace_dir".to_owned(), json!(workspace));
    object.insert(
        "plan_identity".to_owned(),
        Value::String(plan_identity.to_owned()),
    );
    object.insert(
        "config_identity".to_owned(),
        Value::String(config_identity.to_owned()),
    );
    object.insert(
        "final_manifest_digest".to_owned(),
        Value::String(final_manifest_digest.to_owned()),
    );
    object.insert("resumed_nodes".to_owned(), json!(resumed_nodes));
    object.insert("plan_only".to_owned(), Value::Bool(plan_only));
    object
        .entry("compiled_plan".to_owned())
        .or_insert_with(|| json!(plan));
    object.insert(
        "wave_tree_view".to_owned(),
        Value::String(plan.ascii_graph.clone()),
    );
    if let Some(receipts) = object.get("receipts").cloned() {
        object.insert("node_receipts".to_owned(), receipts.clone());
        let settled = receipts.as_array().map_or(0, |items| {
            items
                .iter()
                .filter(|item| item["state"] == "settled")
                .count()
        });
        object.insert("settled_count".to_owned(), json!(settled));
        object.insert(
            "run_state".to_owned(),
            Value::String(
                if object.get("ok").and_then(Value::as_bool) == Some(true) {
                    "settled"
                } else if settled > 0 {
                    "partial_blocked"
                } else {
                    "failed"
                }
                .to_owned(),
            ),
        );
    }
    Ok(())
}

async fn persist_result(gateway: &Gateway, dag_id: &str, result: Value) -> Result<()> {
    let dag_id = dag_id.to_owned();
    let successful = result.get("ok").and_then(Value::as_bool) == Some(true);
    gateway
        .database(move |store| {
            if successful {
                store.complete_job(&dag_id, result, None, None)?;
            } else {
                store.fail_job_with_result(
                    &dag_id,
                    "DAG campaign did not settle all nodes".to_owned(),
                    Some(result),
                    None,
                )?;
            }
            Ok(())
        })
        .await
}

fn digest_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

fn default_deliverable() -> String {
    "Verified implementation or review output".to_owned()
}
fn default_stop_when() -> String {
    "Deliverable and verification criteria are satisfied within scope.".to_owned()
}
const fn default_true() -> bool {
    true
}
const fn default_parallel() -> usize {
    3
}
const fn default_timeout() -> f64 {
    3_600.0
}
const fn default_max_input() -> u64 {
    256_000
}
fn default_workspace() -> PathBuf {
    PathBuf::from(".")
}

#[cfg(test)]
mod tests;
