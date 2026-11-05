//! Source-observed research workflow, separate from coding DAG settlement.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::sync::watch;

use crate::Result;
use crate::gateway::Gateway;
use crate::storage::{JobStatus, NewJob};

mod discovery;
mod execute;
mod sources;
mod types;
mod validate;
pub(crate) use discovery::output_schema as discovery_output_schema;
pub use types::*;
use validate::normalized_web_url as normalize_candidate_url;
pub(crate) use validate::stage_output_schema;

pub(crate) fn harness_text() -> &'static str {
    validate::HARNESS_TEXT
}

pub fn plan(request: &ResearchRequest) -> Result<serde_json::Value> {
    validate::validate_request(request)?;
    Ok(serde_json::json!({
        "schema_version":"research_plan/v1", "mode":"research", "plan_only":true,
        "question":request.question, "source_inputs":request.sources,
        "investigators":request.investigators,"challenger":request.challenger,
        "synthesizer":request.synthesizer,"budget":request.budget,
        "harness_sha256":validate::harness_sha256(),
        "missing_inputs":if request.sources.is_empty() && request.active_discovery().is_none(){vec!["Provide --file, --document or HTTPS --url, or enable --web."]}else{Vec::new()},
        "phases":["optional_web_discovery","observe_sources","investigate","challenge","synthesize","validate_and_render"],
        "automatic_search":request.active_discovery().is_some(),"discovery":effective_discovery(request),"workspace_writes":false,
        "semantic_verification":validate::SEMANTIC_VERIFICATION,
        "markdown":format!("# Research plan\n\n{}\n\nSources: {}. Investigation → challenge → host-rendered synthesis.\n{}",request.question,request.sources.len(),if request.active_discovery().is_some(){"Automatic web discovery is planned; no models, network or database were used."}else if request.sources.is_empty(){"Missing source inputs; no models, network or database were used."}else{"Plan only; no models, network or database were used."})
    }))
}

fn effective_discovery(request: &ResearchRequest) -> Option<DiscoveryConfig> {
    let mut config = request.active_discovery()?;
    if let DiscoveryConfig::Web { max_candidates, .. } = &mut config {
        *max_candidates = (*max_candidates).min(
            request
                .budget
                .max_sources
                .saturating_sub(request.sources.len()),
        );
    }
    Some(config)
}

pub async fn run(gateway: &Gateway, request: ResearchRequest) -> Result<serde_json::Value> {
    let (_owner, receiver) = watch::channel(false);
    run_with_cancellation(gateway, request, receiver).await
}

pub async fn run_with_cancellation(
    gateway: &Gateway,
    request: ResearchRequest,
    cancellation: watch::Receiver<bool>,
) -> Result<serde_json::Value> {
    run_with_context(gateway, request, cancellation, None).await
}

#[derive(Clone)]
struct ExecutionScope {
    workspace: PathBuf,
    fingerprint: String,
}

/// Workflow-owned research keeps source provenance on the original workspace
/// while running analysis providers in an external owned directory.
pub(crate) async fn run_scoped(
    gateway: &Gateway,
    request: ResearchRequest,
    execution_workspace: PathBuf,
    scope_fingerprint: String,
    cancellation: watch::Receiver<bool>,
) -> Result<serde_json::Value> {
    let original = request.workspace_dir.canonicalize()?;
    let workspace = execution_workspace.canonicalize()?;
    if workspace.starts_with(&original) || !workspace.is_dir() {
        return Err("scoped research execution workspace must be external".into());
    }
    validate_id(&scope_fingerprint)?;
    let fingerprint = digest(&("research_execution_scope/v1", scope_fingerprint, &workspace))?;
    run_with_context(
        gateway,
        request,
        cancellation,
        Some(ExecutionScope {
            workspace,
            fingerprint,
        }),
    )
    .await
}

async fn run_with_context(
    gateway: &Gateway,
    mut request: ResearchRequest,
    mut cancellation: watch::Receiver<bool>,
    execution: Option<ExecutionScope>,
) -> Result<serde_json::Value> {
    validate::validate_request(&request)?;
    request.workspace_dir = request.workspace_dir.canonicalize()?;
    for lane in &request.investigators {
        validate_route(&lane.model, &lane.effort)?;
    }
    validate_route(&request.challenger.model, &request.challenger.effort)?;
    validate_route(&request.synthesizer.model, &request.synthesizer.effort)?;
    if request.plan_only || (request.sources.is_empty() && request.active_discovery().is_none()) {
        return plan(&request);
    }
    // Freeze effective defaults only for discovery. Historical seeded requests
    // with discovery=None retain their byte-for-byte serialized identity.
    if let Some(config) = effective_discovery(&request) {
        if let DiscoveryConfig::Web {
            ref model,
            ref effort,
            ..
        } = config
        {
            validate_route(model, effort)?;
        }
        request.discovery = Some(config);
    }
    let state = gateway.state_root.clone();
    let workspace = request.workspace_dir.clone();
    tokio::task::spawn_blocking(move || {
        if canonical_future_path(&state)?.starts_with(&workspace) {
            return Err("research state root must be outside the observed workspace".into());
        }
        Ok::<_, crate::Error>(())
    })
    .await??;
    if *cancellation.borrow() {
        return Ok(
            serde_json::json!({"mode":"research","research_status":"partial","cancelled":true,"registered":false,"markdown":"Research cancelled before admission."}),
        );
    }
    let mut identity = request.clone();
    identity.request_id = None;
    identity.idempotency_key = None;
    let request_identity = if request.active_discovery().is_some() {
        digest(&(discovery::CONTRACT, identity))?
    } else {
        digest(&identity)?
    };
    let identity = match &execution {
        Some(scope) => digest(&("research_scoped/v1", request_identity, &scope.fingerprint))?,
        None => request_identity,
    };
    let parent_id = request
        .request_id
        .clone()
        .unwrap_or_else(|| format!("rsh-{}", uuid::Uuid::new_v4().simple()));
    let snapshot_id = source_job_id(&parent_id);
    let mut metadata = BTreeMap::from([
        ("request_identity".to_owned(), identity),
        ("harness_sha256".to_owned(), validate::harness_sha256()),
        ("source_snapshot_job_id".to_owned(), snapshot_id),
    ]);
    if let Some(config) = request.active_discovery() {
        metadata.insert(
            "discovery_job_id".to_owned(),
            discovery::job_id(&parent_id, &config)?,
        );
    }
    if let Some(scope) = &execution {
        metadata.insert(
            "execution_scope_fingerprint".into(),
            scope.fingerprint.clone(),
        );
        metadata.insert(
            "execution_workspace".into(),
            scope.workspace.to_string_lossy().into_owned(),
        );
    }
    let registration = NewJob {
        task_type: "research_campaign".to_owned(),
        workspace_dir: request.workspace_dir.clone(),
        model: None,
        metadata,
        job_id: Some(parent_id.clone()),
        idempotency_key: request.idempotency_key.clone(),
        provider: Some("hcodex_research".to_owned()),
        engine: Some("research".to_owned()),
        created_at: None,
    };
    let registration = gateway
        .database(move |store| store.resolve_or_register(registration, false))
        .await?;
    if !registration.should_dispatch {
        return Ok(readback_value(registration.record));
    }
    let parent_id = registration.record.job_id;
    request.request_id = Some(parent_id.clone());
    let runner = gateway.clone();
    let owned_id = parent_id.clone();
    let owned_cancel = cancellation.clone();
    let execution_workspace = execution
        .map(|scope| scope.workspace)
        .unwrap_or_else(|| request.workspace_dir.clone());
    let future = gateway.supervise(
        parent_id.clone(),
        Box::pin(async move {
            execute::run(
                &runner,
                request,
                &owned_id,
                &execution_workspace,
                owned_cancel,
            )
            .await
        }),
    );
    tokio::pin!(future);
    let result = tokio::select! {
        result=&mut future => result,
        ()=wait_cancel(&mut cancellation) => {
            // The receiver remains latched if cancellation races registration or
            // supervisor admission. The owner checks it before any source effect.
            gateway.cancel_supervised(&parent_id).await;
            future.await
        }
    };
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let id = parent_id.clone();
            let message = format!("Research interrupted; inspect original request {id}: {error}");
            gateway
                .database(move |store| {
                    store.mark_timed_out_unknown(&id, message, None, None)?;
                    Ok(())
                })
                .await?;
            Err(error)
        }
    }
}

pub async fn status(
    gateway: &Gateway,
    workspace: &Path,
    request_id: &str,
) -> Result<serde_json::Value> {
    validate_id(request_id)?;
    let workspace = workspace.canonicalize()?;
    let state = gateway.state_root.clone();
    let id = request_id.to_owned();
    tokio::task::spawn_blocking(move || {
        use crate::storage::diagnostics::{ReadObservation,read};
        match read(&state,&id) {
            ReadObservation::Observed(snapshot)=>{
                let Some(job)=snapshot.job else{return Ok(serde_json::json!({"ok":false,"read_only":true,"request_id":id,"error":"research_request_not_found"}));};
                if job.task_type!="research_campaign" || Path::new(&job.workspace_dir)!=workspace {
                    return Err("research request does not belong to this workspace".into());
                }
                let mut value=readback_value(job);
                value["read_only"]=serde_json::json!(true);
                Ok(value)
            }
            ReadObservation::Unavailable(issue)|ReadObservation::Unknown(issue)=>Ok(serde_json::json!({"ok":false,"read_only":true,"request_id":id,"error":issue.code()})),
        }
    }).await?
}

/// Requests cancellation only in the process that owns this campaign. A separate
/// CLI status invocation cannot stop a supervisor in another process.
pub async fn cancel(gateway: &Gateway, workspace: &Path, request_id: &str) -> Result<bool> {
    validate_id(request_id)?;
    let workspace = workspace.canonicalize()?;
    let id = request_id.to_owned();
    let accepted = gateway
        .database(move |store| {
            let Some(record) = store.get_job(&id)? else {
                return Ok(false);
            };
            if record.task_type != "research_campaign"
                || Path::new(&record.workspace_dir) != workspace
                || record.metadata.get("owner_pid") != Some(&std::process::id().to_string())
            {
                return Ok(false);
            }
            if matches!(
                record.status,
                JobStatus::Running | JobStatus::TimedOutUnknown
            ) {
                store.cancel_job(&id, "research cancellation requested".to_owned())?;
                return Ok(true);
            }
            Ok(record.status == JobStatus::Cancelled)
        })
        .await?;
    if accepted {
        gateway.cancel_supervised(request_id).await;
    }
    Ok(accepted)
}

fn readback_value(record: crate::storage::JobRecord) -> serde_json::Value {
    let mut value=record.result.unwrap_or_else(||serde_json::json!({"mode":"research","research_status":"partial","markdown":"Research has no final report; inspect original stage jobs and source snapshot."}));
    value["request_id"] = serde_json::json!(record.job_id);
    value["job_status"] = serde_json::json!(record.status);
    value["workspace_dir"] = serde_json::json!(record.workspace_dir);
    value["deduplicated_existing_request"] = serde_json::json!(true);
    value["expected_source_snapshot_job_id"] =
        serde_json::json!(record.metadata.get("source_snapshot_job_id"));
    value["expected_discovery_job_id"] = serde_json::json!(record.metadata.get("discovery_job_id"));
    value
}
fn digest(value: &impl serde::Serialize) -> Result<String> {
    Ok(crate::routing::digest_output(&serde_json::to_string(
        value,
    )?))
}
fn source_job_id(parent: &str) -> String {
    format!(
        "rss-{}",
        &crate::routing::digest_output(parent).trim_start_matches("sha256:")[..32]
    )
}
fn validate_id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err("invalid research request ID".into());
    }
    Ok(())
}
async fn wait_cancel(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
fn execution_engine(model: &str) -> &'static str {
    if model.starts_with("gpt-") && !model.starts_with("gpt-oss") {
        "cli"
    } else {
        "antigravity_cli"
    }
}
fn validate_route(model: &str, effort: &str) -> Result<()> {
    let cli = execution_engine(model) == "cli";
    crate::routing::validate_route(&crate::routing::RouteRequest {
        provider: if cli {
            crate::routing::Provider::OpenaiCodex
        } else {
            crate::routing::Provider::Antigravity
        },
        engine: if cli {
            crate::routing::Engine::CodexCli
        } else {
            crate::routing::Engine::AntigravityCli
        },
        model: model.to_owned(),
        effort: effort.to_owned(),
        prompt: String::new(),
    })?;
    Ok(())
}
fn canonical_future_path(path: &Path) -> Result<PathBuf> {
    let mut path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut suffix = Vec::new();
    while !path.exists() {
        suffix.push(path.file_name().ok_or("invalid state root")?.to_os_string());
        if !path.pop() {
            return Err("invalid state root".into());
        }
    }
    let mut root = path.canonicalize()?;
    for component in suffix.iter().rev() {
        if component == ".." {
            root.pop();
        } else if component != "." {
            root.push(component);
        }
    }
    Ok(root)
}

#[cfg(test)]
mod tests;
