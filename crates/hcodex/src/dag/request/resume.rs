//! Exact persisted plans and request recipes, never regenerated from later source.
use super::super::worktrees::WorktreeCampaign;
use super::*;
use crate::storage::{JobRecord, JobStatus};

const MAX_RECIPE_BYTES: usize = 2 * 1024 * 1024;

/// A fresh continuation request and the exact original compiled topology.
pub struct PreparedResume {
    pub request: Value,
    pub plan: DagPlan,
}

pub async fn prepare_resume_request(
    gateway: &Gateway,
    workspace: &Path,
    resume_id: &str,
) -> Result<PreparedResume> {
    let workspace = workspace.canonicalize()?;
    let (_, plan, mut recipe) = load_plan(gateway, &workspace, resume_id).await?;
    recipe.workspace_dir = workspace;
    recipe.resume_dag_id = Some(resume_id.to_owned());
    recipe.request_id = Some(format!("cdx-resume-{}", uuid::Uuid::new_v4()));
    recipe.idempotency_key = None;
    recipe.plan_only = false;
    Ok(PreparedResume {
        request: serde_json::to_value(recipe)?,
        plan,
    })
}

pub(super) fn config_identity(request: &DagRequest) -> Result<String> {
    digest_json(
        &json!({"campaign_goal":request.campaign_goal,"nodes":request.nodes,
        "spawn_plan":request.spawn_plan,"max_input_bytes":request.max_input_bytes}),
    )
}

pub(super) async fn early_worktree_readback(
    gateway: &Gateway,
    request: &DagRequest,
    workspace: &Path,
    config: &str,
) -> Result<Option<Value>> {
    let id = request.request_id.clone();
    let key = request.idempotency_key.clone();
    let record = gateway
        .database(move |store| {
            if let Some(id) = id {
                store.get_job(&id)
            } else if let Some(key) = key {
                store.find_job_by_idempotency_key(&key)
            } else {
                Ok(None)
            }
        })
        .await?;
    let Some(record) = record else {
        return Ok(None);
    };
    if record.task_type != "dag_ulw"
        || Path::new(&record.workspace_dir) != workspace
        || record.metadata.get("config_identity").map(String::as_str) != Some(config)
        || record.idempotency_key != request.idempotency_key
        || request
            .request_id
            .as_ref()
            .is_some_and(|id| id != &record.job_id)
        || record.metadata.get("resume_dag_id") != request.resume_dag_id.as_ref()
    {
        return Err("existing worktree request has different configuration or lineage".into());
    }
    record.result.map(Some).ok_or_else(|| {
        "existing worktree request has no confirmed result; inspect its original ID".into()
    })
}

pub(super) async fn load_plan(
    gateway: &Gateway,
    workspace: &Path,
    resume_id: &str,
) -> Result<(JobRecord, DagPlan, DagRequest)> {
    super::super::handoff::validate_plan_node(resume_id, 0)?;
    let root_id = resume_id.to_owned();
    let checkpoint_id = format!("ckp-{resume_id}-plan");
    let (root, checkpoint) = gateway
        .database(move |store| Ok((store.get_job(&root_id)?, store.get_job(&checkpoint_id)?)))
        .await?;
    let root = root.ok_or("resume DAG was not found")?;
    let checkpoint = checkpoint.ok_or("resume plan checkpoint missing")?;
    if root.task_type != "dag_ulw"
        || Path::new(&root.workspace_dir) != workspace
        || checkpoint.task_type != "dag_checkpoint"
        || checkpoint.status != JobStatus::Completed
        || Path::new(&checkpoint.workspace_dir) != workspace
        || checkpoint.metadata.get("campaign_id").map(String::as_str) != Some(resume_id)
        || checkpoint
            .metadata
            .get("checkpoint_kind")
            .map(String::as_str)
            != Some("plan")
    {
        return Err("resume plan checkpoint scope mismatch".into());
    }
    let result = checkpoint
        .result
        .ok_or("resume plan checkpoint body missing")?;
    let plan: DagPlan = serde_json::from_value(result["plan"].clone())?;
    let recipe_value = result
        .get("request_recipe")
        .ok_or("historical resume request recipe unavailable")?;
    if serde_json::to_vec(recipe_value)?.len() > MAX_RECIPE_BYTES {
        return Err("resume request recipe exceeds byte budget".into());
    }
    let recipe: DagRequest = serde_json::from_value(recipe_value.clone())?;
    let plan_hash = digest_json(&plan)?;
    let config = config_identity(&recipe)?;
    if !plan.is_executable()
        || root.metadata.get("plan_identity") != Some(&plan_hash)
        || root.metadata.get("config_identity") != Some(&config)
        || checkpoint.metadata.get("plan_identity") != Some(&plan_hash)
        || checkpoint.metadata.get("config_identity") != Some(&config)
    {
        return Err("resume persisted plan/config identity mismatch".into());
    }
    Ok((root, plan, recipe))
}

pub(super) async fn reopen_worktrees(
    gateway: &Gateway,
    request: &DagRequest,
    workspace: &Path,
    plan: &DagPlan,
    config: &str,
) -> Result<(WorktreeCampaign, BTreeMap<String, SettlementReceipt>)> {
    let resume_id = request
        .resume_dag_id
        .as_deref()
        .ok_or("worktree resume source missing")?;
    let (prior, saved_plan, _) = load_plan(gateway, workspace, resume_id).await?;
    if prior.metadata.get("config_identity").map(String::as_str) != Some(config)
        || digest_json(plan)? != digest_json(&saved_plan)?
    {
        return Err("resume DAG plan/config identity mismatch".into());
    }
    let (mut campaign, frontier) = WorktreeCampaign::reopen(
        workspace,
        &gateway.state_root,
        &prior,
        Duration::from_secs_f64(request.timeout_seconds.min(30.0)),
    )
    .await?;
    let pending = frontier.pending_ids();
    let pending_ids = pending
        .iter()
        .map(|id| campaign.provider_request_id(id))
        .collect::<Vec<_>>();
    let checkpoints = frontier.checkpoint_ids();
    let plan_ids = plan
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if pending.len() + checkpoints.len() != plan_ids.len()
        || pending
            .iter()
            .chain(checkpoints.keys())
            .any(|id| !plan_ids.contains(id))
    {
        return Err("resume worktree topology mismatch".into());
    }
    let scope = resume_id.to_owned();
    let plan_identity = digest_json(plan)?;
    let config = config.to_owned();
    let workspace = workspace.to_path_buf();
    let export_plan_identity = plan_identity.clone();
    let export_config = config.clone();
    let export_workspace = workspace.clone();
    let receipts=gateway.database(move|store|{
        for id in pending_ids {if store.get_job(&id)?.is_some(){return Err("resume pending provider request was already registered; inspect its original ID".into());}}
        let mut receipts=BTreeMap::new();
        for (node,id) in checkpoints {
            if id!=format!("ckp-{scope}-{node}") {return Err("resume checkpoint reference mismatch".into());}
            let checkpoint=store.get_job(&id)?.ok_or("resume settled checkpoint missing")?;
            if checkpoint.task_type!="dag_checkpoint" || checkpoint.status!=JobStatus::Completed || Path::new(&checkpoint.workspace_dir)!=workspace
                || checkpoint.metadata.get("campaign_id")!=Some(&scope) || checkpoint.metadata.get("plan_identity")!=Some(&plan_identity)
                || checkpoint.metadata.get("config_identity")!=Some(&config) || checkpoint.metadata.get("node_id")!=Some(&node) {
                return Err("resume settled checkpoint scope mismatch".into());
            }
            let result=checkpoint.result.ok_or("resume checkpoint body missing")?;
            let receipt:SettlementReceipt=serde_json::from_value(result["receipt"].clone())?;
            if receipt.node_id!=node || receipt.state!=NodeState::Settled || !receipt.verify_passed
                || !receipt.gate.executed || receipt.gate.stdout_digest.is_none() || !receipt.out_of_scope_writes.is_empty() {
                return Err("resume checkpoint is not verified settlement".into());
            }
            receipts.insert(node,receipt);
        }
        Ok(receipts)
    }).await?;
    let prior_export_id = format!("ckp-{resume_id}-worktree-export");
    let export_lookup = prior_export_id.clone();
    let prior_export = gateway
        .database(move |store| store.get_job(&export_lookup))
        .await?;
    let reference = if let Some(export) = prior_export {
        if export.status == JobStatus::Completed {
            let body = export
                .result
                .ok_or("completed prior export payload missing")?;
            let report = campaign.report()?;
            if export.task_type != "dag_worktree_export"
                || Path::new(&export.workspace_dir) != export_workspace
                || body["dag_id"] != resume_id
                || body["plan_identity"] != export_plan_identity
                || body["config_identity"] != export_config
                || body["final_manifest_digest"] != frontier.source_manifest_digest
                || body["worktrees"]["report"] != report
            {
                return Err("prior worktree export payload scope conflict".into());
            }
            let hash = tokio::task::spawn_blocking(move || {
                super::super::worktrees::export_payload_digest(&body)
            })
            .await??;
            if prior.metadata.get("worktree_export_id") != Some(&prior_export_id)
                || prior.metadata.get("worktree_export_sha256") != Some(&hash)
                || prior.metadata.get("worktree_export_frontier_sha256")
                    != prior.metadata.get("worktree_frontier_sha256")
            {
                return Err(
                    "prior worktree export exact intent binding missing or conflicting".into(),
                );
            }
            Some(json!({"job_id":prior_export_id,"payload_sha256":hash}))
        } else if export.result.is_some() {
            return Err("unconfirmed prior worktree export contains conflicting payload".into());
        } else {
            None
        }
    } else {
        None
    };
    campaign.inherit_export(reference);
    Ok((campaign, receipts))
}

pub(super) fn recipe_value(request: &DagRequest) -> Result<Value> {
    let value = serde_json::to_value(request)?;
    if serde_json::to_vec(&value)?.len() > MAX_RECIPE_BYTES {
        return Err("DAG request recipe exceeds byte budget".into());
    }
    Ok(value)
}
