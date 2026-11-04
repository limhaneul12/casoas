//! Application service shared by MCP, CLI, and terminal adapters.
mod contracts;
mod grounding;

use crate::routing::{Engine, Provider, ReceiptStatus, RouteRequest};
use crate::storage::{AuditVerdict, HandoffFilter, JobStatus, NewHandoff, NewJob, Store};
use crate::supervisor::Supervisor;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use contracts::{
    BudgetEnvelope, background_response, budget_response, deduplicated_response, job_list_response,
    job_status_response, model_execution_response, rejection_response, verified_at_utc,
};
use grounding::{
    boundary_schema, build_route_request, grounded_prompt, normalize_boundary, structured_prompt,
};

#[cfg(test)]
type TestProvider = std::sync::Arc<
    dyn Fn(
            &RouteRequest,
            &crate::runtime::ExecutionOptions,
        ) -> crate::Result<crate::runtime::ExecutionResult>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct Gateway {
    pub state_root: PathBuf,
    supervisor: Supervisor,
    codex_binary: Option<PathBuf>,
    sandbox_mode: crate::routing::SandboxMode,
    #[cfg(test)]
    test_provider: Option<TestProvider>,
}

struct ProviderInvocation {
    name: String,
    args: Value,
    request: RouteRequest,
    engine: Engine,
    duration: Duration,
    options: crate::runtime::ExecutionOptions,
    schema: Option<Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthArguments {
    #[serde(default)]
    workspace_dir: Option<PathBuf>,
}
impl Gateway {
    pub fn new(state_root: PathBuf) -> Self {
        Self {
            state_root,
            supervisor: Supervisor::new(8),
            codex_binary: None,
            sandbox_mode: crate::routing::SandboxMode::ReadOnly,
            #[cfg(test)]
            test_provider: None,
        }
    }
    pub fn with_sandbox(mut self, mode: crate::routing::SandboxMode) -> Self {
        self.sandbox_mode = mode;
        self
    }
    #[cfg(test)]
    pub(crate) fn with_codex_binary(mut self, binary: PathBuf) -> Self {
        self.codex_binary = Some(binary);
        self
    }
    /// Test-only replacement of the external provider transport. Routing,
    /// grounding, reservation and persistence remain the real application path.
    #[cfg(test)]
    pub(crate) fn with_test_provider<F>(mut self, provider: F) -> Self
    where
        F: Fn(
                &RouteRequest,
                &crate::runtime::ExecutionOptions,
            ) -> crate::Result<crate::runtime::ExecutionResult>
            + Send
            + Sync
            + 'static,
    {
        self.test_provider = Some(std::sync::Arc::new(provider));
        self
    }
    pub async fn shutdown(&self) -> crate::Result<()> {
        self.supervisor.shutdown(Duration::from_secs(5)).await
    }
    pub async fn active_background_jobs(&self) -> usize {
        self.supervisor.active().await
    }
    pub(crate) async fn cancel_supervised(&self, job_id: &str) -> bool {
        self.supervisor.cancel(job_id).await
    }
    /// Owns a whole campaign so cancellation reaches nested providers and gates.
    pub(crate) async fn supervise<T, F>(&self, job_id: String, future: F) -> crate::Result<T>
    where
        T: Send + 'static,
        F: Future<Output = crate::Result<T>> + Send + 'static,
    {
        if crate::supervisor::current_job_id().as_deref() == Some(job_id.as_str()) {
            return future.await;
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.supervisor
            .spawn(job_id, async move {
                let _receiver_closed = sender.send(future.await);
            })
            .await?;
        receiver.await?
    }
    async fn bridge_health(&self, args: HealthArguments) -> crate::Result<Value> {
        let (mut health, antigravity_models) = crate::runtime::health_with_models().await?;
        let (telemetry, pending) = self
            .database(|store| Ok((store.job_telemetry()?, store.pending_handoff_counts()?)))
            .await?;
        health["active_background_jobs"] = json!(self.active_background_jobs().await);
        health["job_telemetry"] = json!({
            "total_jobs": telemetry.total_jobs,
            "completed_jobs": telemetry.completed_jobs,
            "failed_jobs": telemetry.failed_jobs,
            "timed_out_unknown_jobs": telemetry.unknown_jobs,
            "running_jobs": telemetry.running_jobs,
            "cancelled_jobs": telemetry.cancelled_jobs,
            "avg_duration_seconds": telemetry.average_duration_seconds,
        });
        health["pending_handoffs"] = serde_json::to_value(pending)?;
        health["verified_at_utc"] = Value::String(verified_at_utc());
        let codex_boundary = health["auth_boundaries"]["codex_cli_oauth"].clone();
        health["auth_boundaries"]["codex_cli_oauth"]["cli_binary_ready"] = codex_boundary
            .get("ready")
            .cloned()
            .unwrap_or(Value::Bool(false));
        health["auth_boundaries"]["codex_cli_oauth"]["auto_reauth_enabled"] = Value::Bool(false);
        health["codex_binary"] = Value::String(
            crate::runtime::resolve_codex_binary()
                .to_string_lossy()
                .into_owned(),
        );
        health["codex_version"] = codex_boundary
            .get("version")
            .cloned()
            .unwrap_or_else(|| Value::String("unavailable".to_owned()));
        let agy_boundary = health["auth_boundaries"]["antigravity_rpc"].clone();
        health["agy_bridge_ready"] = agy_boundary
            .get("connected")
            .cloned()
            .unwrap_or(Value::Bool(false));
        health["agy_endpoint"] = agy_boundary.get("endpoint").cloned().unwrap_or(Value::Null);
        health["antigravity_models"] = Value::Array(
            antigravity_models
                .iter()
                .map(|model| {
                    let mut value = serde_json::to_value(model)?;
                    value["provider"] = Value::String("antigravity".to_owned());
                    value["engine"] = Value::String("antigravity_local_rpc".to_owned());
                    Ok(value)
                })
                .collect::<crate::Result<Vec<_>>>()?,
        );
        health["routing_contract"]["routable_antigravity_models"] = json!(
            antigravity_models
                .iter()
                .filter(|model| model.routable)
                .map(|model| model.model_id.clone())
                .collect::<Vec<_>>()
        );
        health["routing_contract"]["antigravity_routable"] =
            Value::Bool(antigravity_models.iter().any(|model| model.routable));
        health["routing_contract"]["advertised_antigravity_models"] =
            health["antigravity_models"].clone();
        health["pending_argon_handoffs"] = pending_count(&health, "argon");
        health["pending_sonnet_handoffs"] = pending_count(&health, "sonnet");
        health["pending_opus_handoffs"] = pending_count(&health, "opus");
        health["pending_codex_handoffs"] = pending_count(&health, "codex");
        health["routing_contract"]["advertised_codex_models"] = json!([
            "gpt-6-terra",
            "gpt-6-luna",
            "gpt-6-sol",
            "gpt-6-astra",
            "gpt-6.1-terra",
            "gpt-6.1-sol"
        ]);
        let codex_ready = health["auth_boundaries"]["codex_cli_oauth"]["ready"]
            .as_bool()
            .unwrap_or(false);
        health["routing_contract"]["routable_codex_models"] = if codex_ready {
            health["routing_contract"]["advertised_codex_models"].clone()
        } else {
            json!([])
        };
        health["routing_contract"]["verified_at_utc"] = health["verified_at_utc"].clone();
        health["routing_contract"]["codex_cli_routable"] = Value::Bool(codex_ready);
        health["ok"] = Value::Bool(codex_ready);
        let workspace = args
            .workspace_dir
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        health["runtime_identity"] = crate::lifecycle::runtime_identity(&workspace).await;
        Ok(health)
    }
    pub async fn database<T, F>(&self, operation: F) -> crate::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Store) -> crate::Result<T> + Send + 'static,
    {
        let root = self.state_root.clone();
        tokio::task::spawn_blocking(move || operation(Store::open(&root)?)).await?
    }
    pub fn call<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = crate::Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let args = crate::catalog::arguments(name, input)?;
            if args["async_background"].as_bool().unwrap_or(false) {
                return self.spawn_background(name, args).await;
            }
            self.call_foreground(name, args).await
        })
    }

    async fn call_foreground(&self, name: &str, args: Value) -> crate::Result<Value> {
        match name {
            "codex_job_status" => {
                let id = text(&args, "job_id");
                let key = optional(&args, "idempotency_key");
                let lookup = if id.is_empty() {
                    key.clone().unwrap_or_else(|| "<empty>".to_owned())
                } else {
                    id.clone()
                };
                let cancel = args["cancel"].as_bool().unwrap_or(false);
                let (record, cancel_id) = self
                    .database(move |store| {
                        let record = if id.is_empty() {
                            key.as_deref()
                                .map(|key| store.find_job_by_idempotency_key(key))
                                .transpose()?
                                .flatten()
                        } else {
                            store.get_job(&id)?
                        };
                        let record = match record {
                            Some(record)
                                if cancel
                                    && matches!(
                                        record.status,
                                        JobStatus::Running | JobStatus::TimedOutUnknown
                                    ) =>
                            {
                                store.cancel_job(
                                    &record.job_id,
                                    "user requested cancellation".into(),
                                )?
                            }
                            other => other,
                        };
                        let cancel_id = record
                            .as_ref()
                            .filter(|record| cancel && record.status == JobStatus::Cancelled)
                            .map(|record| record.job_id.clone());
                        Ok((record, cancel_id))
                    })
                    .await?;
                if let Some(cancel_id) = cancel_id {
                    self.supervisor.cancel(&cancel_id).await;
                }
                job_status_response(record, &lookup)
            }
            "codex_job_list" => {
                let limit = limit(&args);
                let workspace = optional(&args, "workspace_dir");
                self.database(move |store| {
                    job_list_response(store.list_jobs(limit, workspace.as_deref())?, workspace)
                })
                .await
            }
            "heterarchy_handoff_create" => {
                let request = NewHandoff {
                    sender: field(&args, "sender")?,
                    recipient: field(&args, "recipient")?,
                    title: text(&args, "title"),
                    spec_or_payload: text(&args, "spec_or_payload"),
                    workspace_dir: workspace(&args),
                    priority: field(&args, "priority")?,
                    alexandria_ref: None,
                };
                self.database(move |store| {
                    let ticket = serde_json::to_value(store.create_handoff(request)?)?;
                    let message=format!("Handoff ticket '{}' queued from '{}' to '{}' (status='pending', collaboration_role_state='queued'). Do not treat ticket creation as completion.",ticket["ticket_id"].as_str().unwrap_or_default(),ticket["sender"].as_str().unwrap_or_default(),ticket["recipient"].as_str().unwrap_or_default());
                    Ok(json!({"ok":true,"ticket":enrich_handoff(ticket),"collaboration_role_state":"queued","execution_completed":false,"consensus_eligible":false,"message":message}))
                })
                .await
            }
            "heterarchy_handoff_inbox" => {
                let status = optional(&args, "status")
                    .filter(|value| !value.is_empty())
                    .map(|value| serde_json::from_value(Value::String(value)))
                    .transpose()?;
                let request = HandoffFilter {
                    recipient: optional(&args, "recipient")
                        .map(|value| serde_json::from_value(Value::String(value)))
                        .transpose()?,
                    status,
                    workspace_dir: optional(&args, "workspace_dir").map(PathBuf::from),
                    limit: limit(&args),
                };
                self.database(move |store| {
                    let tickets = store
                        .list_handoffs(request)?
                        .into_iter()
                        .map(serde_json::to_value)
                        .collect::<std::result::Result<Vec<_>, _>>()?
                        .into_iter()
                        .map(enrich_handoff)
                        .collect::<Vec<_>>();
                    Ok(json!({"ok":true,"tickets":tickets}))
                })
                .await
            }
            "heterarchy_handoff_resolve" => {
                let id = text(&args, "ticket_id");
                let status = field(&args, "status")?;
                let note = optional(&args, "resolution_note");
                let reference = optional(&args, "alexandria_ref");
                let verdict = optional(&args, "verdict")
                    .map(|value| serde_json::from_value::<AuditVerdict>(Value::String(value)))
                    .transpose()?;
                self.database(move |store| {
                    let ticket = if let Some(verdict) = verdict {
                        store.resolve_handoff(
                            &id,
                            status,
                            note.unwrap_or_else(|| "Resolved by heterarchy reviewer.".to_owned()),
                            verdict,
                            reference,
                        )?
                    } else {
                        store.update_handoff(&id, status, note, reference)?
                    };
                    let ticket = ticket
                        .map(serde_json::to_value)
                        .transpose()?
                        .map(enrich_handoff);
                    Ok(json!({"ok":true,"ticket":ticket}))
                })
                .await
            }
            "heterarchy_dag_ulw" => crate::dag::handle_request(self, &args).await,
            "codex_bridge_health" => self.bridge_health(serde_json::from_value(args)?).await,
            "heterarchy_triad_orchestrate" => crate::triad::orchestrate(self, &args).await,
            "codex_alexandria_decision_pack" => crate::triad::decision_pack(self, &args).await,
            _ => self.execute_tool(name, &args).await,
        }
    }
    pub async fn spawn_background(&self, name: &str, mut args: Value) -> crate::Result<Value> {
        let idempotency_key = optional(&args, "idempotency_key");
        let request_id = optional(&args, "request_id")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("cdx-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]));
        args["request_id"] = Value::String(request_id.clone());
        if args.get("async_background").is_some() {
            args["async_background"] = Value::Bool(false);
        }
        if matches!(name, "heterarchy_dag_ulw" | "heterarchy_triad_orchestrate") {
            let task_type = if name == "heterarchy_dag_ulw" {
                "dag_ulw"
            } else {
                "triad_orchestration"
            };
            let mode = if name == "heterarchy_triad_orchestrate" {
                triad_mode(&args)
            } else {
                optional(&args, "mode").unwrap_or_else(|| "pipeline".to_owned())
            };
            let source_task_digest = crate::routing::digest_output(
                args.get("task_spec")
                    .and_then(Value::as_str)
                    .or_else(|| args.get("campaign_goal").and_then(Value::as_str))
                    .unwrap_or_default(),
            );
            let model = if name == "heterarchy_dag_ulw" {
                "dag-ulw".to_owned()
            } else {
                triad_model_set(&args)
            };
            let mut metadata = BTreeMap::from([
                ("current_stage".to_owned(), "queued".to_owned()),
                ("mode".to_owned(), mode.clone()),
                ("schema_version".to_owned(), "2".to_owned()),
                ("source_task_digest".to_owned(), source_task_digest),
            ]);
            if name == "heterarchy_triad_orchestrate" {
                metadata.insert(
                    "request_identity".to_owned(),
                    triad_request_identity(&args)?,
                );
            }
            let request = NewJob {
                task_type: task_type.to_owned(),
                workspace_dir: workspace(&args),
                model: Some(model),
                metadata,
                job_id: Some(request_id.clone()),
                idempotency_key: idempotency_key.clone(),
                provider: Some("heterarchy_triad".to_owned()),
                engine: Some(mode),
                created_at: None,
            };
            let retry = args["retry_same_request"].as_bool().unwrap_or(false);
            let registration = self
                .database(move |store| store.resolve_or_register(request, retry))
                .await?;
            if !registration.should_dispatch {
                return deduplicated_response(&registration.record);
            }
            if name == "heterarchy_triad_orchestrate" {
                args["_background_reserved"] = Value::Bool(true);
            }
        }
        let mut configured_default = None;
        if matches!(
            name,
            "codex_adversarial_audit" | "codex_rescue_sandbox" | "claude_code_craft"
        ) {
            let mut route_request = build_route_request(name, &args, String::new())?;
            configured_default = self
                .configured_codex_default(&route_request, &workspace(&args))
                .await?;
            if let Some(configured) = &configured_default {
                route_request.model.clone_from(&configured.model);
            }
            if let Err(error) = crate::routing::validate_route(&route_request) {
                let sandbox_mode = args
                    .get("sandbox_mode")
                    .filter(|value| !value.is_null())
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()?
                    .unwrap_or(self.sandbox_mode);
                let duration = Duration::from_secs_f64(
                    args["timeout_seconds"]
                        .as_f64()
                        .unwrap_or(3600.0)
                        .clamp(0.01, 86_400.0),
                );
                let max_input_bytes =
                    usize::try_from(args["max_input_bytes"].as_u64().unwrap_or(256_000))?;
                let stored_model = if configured_default.is_some() {
                    "codex-default".to_owned()
                } else {
                    route_request.model.clone()
                };
                let request_identity = provider_request_identity(
                    name,
                    &semantic_request_arguments(&args),
                    &route_request,
                    &stored_model,
                    sandbox_mode,
                    duration,
                    max_input_bytes,
                )?;
                let registration_request = NewJob {
                    task_type: name.to_owned(),
                    workspace_dir: workspace(&args),
                    model: Some(stored_model),
                    metadata: BTreeMap::from([("request_identity".to_owned(), request_identity)]),
                    job_id: Some(request_id.clone()),
                    idempotency_key: idempotency_key.clone(),
                    provider: Some(
                        serde_json::to_value(route_request.provider)?
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                    engine: Some(
                        serde_json::to_value(route_request.engine)?
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                    created_at: None,
                };
                let retry = args["retry_same_request"].as_bool().unwrap_or(false);
                let registration = self
                    .database(move |store| store.resolve_or_register(registration_request, retry))
                    .await?;
                if !registration.should_dispatch {
                    return deduplicated_response(&registration.record);
                }
                let rejection = crate::routing::route_validation_error(error.as_ref())
                    .ok_or("route validation failed without a typed routing error")?;
                let mut receipt = crate::routing::rejected_receipt(
                    rejection,
                    Some(&registration.record.job_id),
                    registration.record.idempotency_key.clone(),
                );
                receipt.attempt_count = registration.record.attempt_count;
                crate::runtime::apply_requested_model_override(
                    &mut receipt,
                    configured_default.as_ref().map(|_| "codex-default"),
                );
                let receipt_value = serde_json::to_value(&receipt)?;
                let persisted_receipt = receipt_value.clone();
                let rejected_id = registration.record.job_id.clone();
                let message = rejection.message.clone();
                let persisted_message = message.clone();
                self.database(move |store| {
                    store.reject_before_dispatch(
                        &rejected_id,
                        persisted_message,
                        Some(persisted_receipt),
                    )?;
                    Ok(())
                })
                .await?;
                let mut response = rejection_response(&receipt, rejection.error_code, &message)?;
                response["async_dispatched"] = Value::Bool(false);
                return Ok(response);
            }
        }
        let worker = self.clone();
        let task_name = name.to_owned();
        let task_args = args.clone();
        let task_id = request_id.clone();
        self.supervisor
            .spawn(request_id.clone(), async move {
                let result = if configured_default.is_some() {
                    Box::pin(worker.execute_tool_with_default(
                        &task_name,
                        &task_args,
                        configured_default,
                    ))
                    .await
                } else {
                    Box::pin(worker.call_foreground(&task_name, task_args.clone())).await
                };
                let _persisted = worker
                    .persist_background_outcome(&task_id, &task_name, &task_args, result)
                    .await;
            })
            .await?;
        for _attempt in 0..200 {
            let lookup = request_id.clone();
            let lookup_key = idempotency_key.clone();
            if let Some(record) = self
                .database(move |store| {
                    let by_id = store.get_job(&lookup)?;
                    if by_id.is_some() {
                        return Ok(by_id);
                    }
                    lookup_key
                        .as_deref()
                        .map(|key| store.find_job_by_idempotency_key(key))
                        .transpose()
                        .map(Option::flatten)
                })
                .await?
            {
                return Ok(background_response(&record, record.job_id != request_id));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.supervisor.cancel(&request_id).await;
        Err("background job did not persist registration before admission deadline".into())
    }

    async fn persist_background_outcome(
        &self,
        job_id: &str,
        name: &str,
        args: &Value,
        result: crate::Result<Value>,
    ) -> crate::Result<()> {
        if let Ok(value) = &result {
            let returned_id = value
                .get("job_id")
                .or_else(|| value.get("collaboration_id"))
                .and_then(Value::as_str)
                .or_else(|| {
                    value
                        .get("job")
                        .and_then(|job| job.get("job_id"))
                        .and_then(Value::as_str)
                });
            if returned_id.is_some_and(|returned_id| returned_id != job_id) {
                return Ok(());
            }
        }
        let lookup = job_id.to_owned();
        let existing = self.database(move |store| store.get_job(&lookup)).await?;
        let record = if let Some(record) = existing {
            record
        } else {
            let request = NewJob {
                task_type: name.to_owned(),
                workspace_dir: workspace(args),
                model: optional(args, "model").or_else(|| optional(args, "claude_model")),
                metadata: BTreeMap::new(),
                job_id: Some(job_id.to_owned()),
                idempotency_key: optional(args, "idempotency_key"),
                provider: Some("heterarchy".to_owned()),
                engine: Some("supervisor".to_owned()),
                created_at: None,
            };
            self.database(move |store| store.create_job(request))
                .await?
        };
        if record.status != JobStatus::Running {
            return Ok(());
        }
        let final_id = job_id.to_owned();
        match result {
            Ok(value) => {
                if value["collaboration_persistence"] == "failed" {
                    let error = value["persistence_error"]
                        .as_str()
                        .unwrap_or("Triad collaboration persistence failed after role execution.")
                        .to_owned();
                    return self
                        .database(move |store| {
                            store.fail_job_with_result(&final_id, error, Some(value), None)?;
                            Ok(())
                        })
                        .await;
                }
                self.database(move |store| {
                    store.complete_job(&final_id, value, None, None)?;
                    Ok(())
                })
                .await
            }
            Err(error) => {
                let message = error.to_string();
                self.database(move |store| {
                    store.fail_job(&final_id, message, None)?;
                    Ok(())
                })
                .await
            }
        }
    }

    async fn dispatch_provider(
        &self,
        invocation: ProviderInvocation,
    ) -> crate::Result<crate::runtime::ExecutionResult> {
        #[cfg(test)]
        if let Some(provider) = &self.test_provider {
            return provider(&invocation.request, &invocation.options);
        }
        let ProviderInvocation {
            name,
            args,
            request,
            engine,
            duration,
            options,
            schema,
        } = invocation;
        match name.as_str() {
            "codex_git_review" => {
                let target = crate::runtime::ReviewTarget {
                    uncommitted: args["uncommitted"].as_bool().unwrap_or(true),
                    base_branch: optional(&args, "base_branch"),
                    commit_sha: optional(&args, "commit_sha"),
                };
                crate::runtime::codex_git_review_with_options(request, target, duration, &options)
                    .await
            }
            "codex_session_followup" => {
                crate::runtime::codex_session_followup_with_options(
                    request,
                    &text(&args, "session_id"),
                    args["fork"].as_bool().unwrap_or(false),
                    duration,
                    &options,
                )
                .await
            }
            _ => match engine {
                Engine::CodexCli => {
                    if let Some(binary) = self.codex_binary.as_deref() {
                        crate::runtime::execute_codex_with_binary_and_options(
                            request, duration, binary, &options,
                        )
                        .await
                    } else if let Some(schema) = schema.as_ref() {
                        crate::runtime::execute_codex_structured(
                            request, duration, &options, schema,
                        )
                        .await
                    } else {
                        crate::runtime::execute_codex_with_options(request, duration, &options)
                            .await
                    }
                }
                Engine::AntigravityRpc => {
                    let mut request = request;
                    if let Some(schema) = schema.as_ref() {
                        request.prompt = structured_prompt(&request.prompt, schema)?;
                    }
                    crate::runtime::AntigravityClient::new()?
                        .execute_with_options(request, duration, &options)
                        .await
                }
                Engine::Http => {
                    crate::http_runtime::execute(request, duration, &options, schema.as_ref()).await
                }
            },
        }
    }

    pub async fn execute_tool(&self, name: &str, args: &Value) -> crate::Result<Value> {
        self.execute_tool_with_default(name, args, None).await
    }

    async fn configured_codex_default(
        &self,
        request: &RouteRequest,
        workspace: &Path,
    ) -> crate::Result<Option<crate::runtime::ConfiguredCodexDefault>> {
        if request.engine != Engine::CodexCli
            || request.provider != Provider::OpenaiCodex
            || request.model != "codex-default"
        {
            return Ok(None);
        }
        Ok(Some(self.configured_default_model(workspace).await?))
    }

    pub(crate) async fn configured_default_model(
        &self,
        workspace: &Path,
    ) -> crate::Result<crate::runtime::ConfiguredCodexDefault> {
        let binary = self
            .codex_binary
            .clone()
            .unwrap_or_else(crate::runtime::resolve_codex_binary);
        crate::runtime::resolve_codex_default(&binary, workspace, Duration::from_secs(5)).await
    }

    async fn execute_tool_with_default(
        &self,
        name: &str,
        args: &Value,
        configured: Option<crate::runtime::ConfiguredCodexDefault>,
    ) -> crate::Result<Value> {
        let sandbox_mode = args
            .get("sandbox_mode")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?
            .unwrap_or(self.sandbox_mode);
        let duration = Duration::from_secs_f64(
            args["timeout_seconds"]
                .as_f64()
                .unwrap_or(3600.0)
                .clamp(0.01, 86_400.0),
        );
        let max_input_bytes = usize::try_from(args["max_input_bytes"].as_u64().unwrap_or(256_000))?;
        let root = workspace(args);
        let request_id = optional(args, "request_id");
        let key = optional(args, "idempotency_key");
        let retry = args["retry_same_request"].as_bool().unwrap_or(false);
        let identity_args = semantic_request_arguments(args);
        let context_args = identity_args.clone();
        let tool = name.to_owned();
        let (normalized_args, prompt) =
            tokio::task::spawn_blocking(move || grounded_prompt(&tool, &context_args, &root))
                .await??;
        let args = &normalized_args;
        let mut request = build_route_request(name, args, prompt)?;
        let configured = match configured {
            Some(configured) => Some(configured),
            None => {
                self.configured_codex_default(&request, &workspace(args))
                    .await?
            }
        };
        let requested_model_override = configured.as_ref().map(|_| "codex-default".to_owned());
        if let Some(configured) = &configured {
            request.model.clone_from(&configured.model);
        }
        let engine = request.engine;
        let provider = request.provider;
        let model = requested_model_override
            .clone()
            .unwrap_or_else(|| request.model.clone());
        let route_validation = crate::routing::validate_route(&request);
        let mut metadata = BTreeMap::new();
        if let Some(configured) = &configured {
            metadata.insert("codex_default_model".to_owned(), configured.model.clone());
            if let Some(origin) = &configured.origin {
                metadata.insert(
                    "codex_default_origin".to_owned(),
                    serde_json::to_string(origin)?,
                );
            }
        }
        metadata.insert(
            "request_identity".to_owned(),
            provider_request_identity(
                name,
                &identity_args,
                &request,
                &model,
                sandbox_mode,
                duration,
                max_input_bytes,
            )?,
        );
        let new_job = NewJob {
            task_type: name.into(),
            workspace_dir: workspace(args),
            model: Some(model),
            metadata,
            job_id: request_id,
            idempotency_key: key,
            provider: Some(
                serde_json::to_value(provider)?
                    .as_str()
                    .unwrap_or_default()
                    .into(),
            ),
            engine: Some(
                serde_json::to_value(engine)?
                    .as_str()
                    .unwrap_or_default()
                    .into(),
            ),
            created_at: None,
        };
        let registration = self
            .database(move |store| store.resolve_or_register(new_job, retry))
            .await?;
        if !registration.should_dispatch {
            return deduplicated_response(&registration.record);
        }
        let job_id = registration.record.job_id;
        if let Err(error) = route_validation {
            let rejection = crate::routing::route_validation_error(error.as_ref())
                .ok_or("route validation failed without a typed routing error")?;
            let mut receipt = crate::routing::rejected_receipt(
                rejection,
                Some(&job_id),
                registration.record.idempotency_key.clone(),
            );
            receipt.attempt_count = registration.record.attempt_count;
            crate::runtime::apply_requested_model_override(
                &mut receipt,
                requested_model_override.as_deref(),
            );
            let value = serde_json::to_value(&receipt)?;
            let saved = value.clone();
            let rejected_id = job_id.clone();
            let message = error.to_string();
            let persisted_message = message.clone();
            self.database(move |store| {
                store.reject_before_dispatch(&rejected_id, persisted_message, Some(saved))?;
                Ok(())
            })
            .await?;
            return rejection_response(&receipt, rejection.error_code, &message);
        }
        let budget =
            BudgetEnvelope::evaluate(&request.prompt, max_input_bytes, duration.as_secs_f64());
        if budget.input_budget_exceeded {
            let route = crate::routing::validate_route(&request)?;
            let mut receipt = crate::routing::new_receipt_with_context(
                &route,
                Some(&job_id),
                registration.record.idempotency_key.clone(),
            );
            receipt.attempt_count = registration.record.attempt_count;
            crate::runtime::apply_requested_model_override(
                &mut receipt,
                requested_model_override.as_deref(),
            );
            receipt.status = ReceiptStatus::Failed;
            receipt.outcome_state = "confirmed_failure".to_owned();
            receipt.call_path = "gateway::input_budget".to_owned();
            receipt.ended_at = Some(verified_at_utc());
            receipt.error_code = Some(crate::routing::RouteErrorCode::InputBudgetExceeded);
            let error = format!(
                "Prompt input size ({} bytes) exceeds max_input_bytes ({} bytes).",
                budget.actual_input_bytes, budget.max_input_bytes
            );
            receipt.error = Some(error.clone());
            receipt.failure_reason = Some(error.clone());
            let stored_receipt = serde_json::to_value(&receipt)?;
            let budget_id = job_id.clone();
            let budget_error = error.clone();
            let persisted_receipt = stored_receipt.clone();
            self.database(move |store| {
                store.mark_input_budget_exceeded(
                    &budget_id,
                    budget_error,
                    Some(persisted_receipt),
                )?;
                Ok(())
            })
            .await?;
            return budget_response(&receipt, &budget, &error);
        }
        let options = crate::runtime::ExecutionOptions {
            workspace_dir: Some(workspace(args)),
            request_id: Some(job_id.clone()),
            idempotency_key: registration.record.idempotency_key,
            attempt_count: registration.record.attempt_count,
            max_input_bytes,
            sandbox_mode,
            requested_model_override,
        };
        let schema = boundary_schema(name);
        let runner = self.clone();
        let provider_name = name.to_owned();
        let provider_args = args.clone();
        let provider_schema = schema.clone();
        let provider = async move {
            runner
                .dispatch_provider(ProviderInvocation {
                    name: provider_name,
                    args: provider_args,
                    request,
                    engine,
                    duration,
                    options,
                    schema: provider_schema,
                })
                .await
        };
        let result = if crate::supervisor::current_job_id().is_some() {
            provider.await
        } else {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            self.supervisor
                .spawn(job_id.clone(), async move {
                    let _sent = sender.send(Box::pin(provider).await);
                })
                .await?;
            receiver.await?
        };
        match result {
            Ok(mut execution) => {
                if execution.receipt.status == ReceiptStatus::Completed
                    && let Some(schema) = schema.as_ref()
                {
                    let _request_schema = schema;
                    let validated = execution
                        .structured_output
                        .as_mut()
                        .ok_or_else(|| -> crate::Error {
                            "provider returned no structured output".into()
                        })
                        .and_then(|output| normalize_boundary(name, output));
                    if let Err(error) = validated {
                        execution.receipt.status = ReceiptStatus::Failed;
                        execution.receipt.outcome_state = "confirmed_failure".into();
                        execution.receipt.error =
                            Some(format!("structured output validation failed: {error}"));
                    }
                }
                let status = execution.receipt.status;
                let mut value = serde_json::to_value(&execution)?;
                if schema.is_some() {
                    value["schema_valid"] = Value::Bool(status == ReceiptStatus::Completed);
                }
                let scope = crate::evidence::build_scope(&workspace(args), args, &value).await?;
                value["evidence_scope"] = scope;
                let schema_valid = value.get("schema_valid").and_then(Value::as_bool);
                let mut final_budget = budget.clone();
                final_budget.elapsed_seconds = execution.receipt.duration_ms as f64 / 1000.0;
                final_budget.time_budget_exceeded = status == ReceiptStatus::TimedOutUnknown;
                final_budget.termination_reason = match status {
                    ReceiptStatus::Completed => "completed",
                    ReceiptStatus::TimedOutUnknown => "timed_out_unknown",
                    ReceiptStatus::Cancelled => "cancelled",
                    _ => "failed",
                }
                .to_owned();
                let workspace_dir = workspace(args).to_string_lossy().into_owned();
                let mut response = model_execution_response(
                    &execution,
                    &workspace_dir,
                    &final_budget,
                    schema_valid,
                    value["evidence_scope"].clone(),
                )?;
                response["result"] = value;
                let receipt = Some(serde_json::to_value(&execution.receipt)?);
                let saved = response.clone();
                let saved_id = job_id.clone();
                let session_id = execution.session_id.clone();
                let actual_model = execution.receipt.actual_model.clone();
                let mut metadata_updates = BTreeMap::new();
                if let Some(valid) = schema_valid {
                    metadata_updates.insert("schema_valid".to_owned(), valid.to_string());
                }
                if let Some(source) = execution.receipt.identity_source.as_ref() {
                    metadata_updates.insert("identity_source".to_owned(), source.clone());
                }
                self.database(move |store| {
                    match status {
                        ReceiptStatus::TimedOutUnknown => {
                            store.mark_timed_out_unknown(
                                &saved_id,
                                "outcome unknown after timeout".into(),
                                Some(saved),
                                receipt,
                            )?;
                        }
                        ReceiptStatus::Completed => {
                            store.complete_job_with_details(
                                &saved_id,
                                saved,
                                session_id,
                                receipt,
                                actual_model,
                                metadata_updates,
                            )?;
                        }
                        ReceiptStatus::Cancelled => {
                            let reason = execution_failure_message(&saved);
                            store.cancel_job_with_result(
                                &saved_id,
                                reason,
                                Some(saved),
                                receipt,
                            )?;
                        }
                        _ => {
                            let failure = execution_failure_message(&saved);
                            store.fail_job_with_result(&saved_id, failure, Some(saved), receipt)?;
                        }
                    }
                    Ok(())
                })
                .await?;
                Ok(response)
            }
            Err(error) => {
                let message = error.to_string();
                self.database(move |store| {
                    store.fail_job(&job_id, message, None)?;
                    Ok(())
                })
                .await?;
                Err(error)
            }
        }
    }
}

const TRANSPORT_ARGUMENTS: [&str; 5] = [
    "_background_reserved",
    "async_background",
    "idempotency_key",
    "request_id",
    "retry_same_request",
];

const PROVIDER_NORMALIZED_ARGUMENTS: [&str; 9] = [
    "engine",
    "max_input_bytes",
    "model",
    "claude_model",
    "provider",
    "reasoning_effort",
    "sandbox_mode",
    "timeout_seconds",
    "workspace_dir",
];

const TRIAD_NORMALIZED_ARGUMENTS: [&str; 11] = [
    "argon_model",
    "claude_model",
    "codex_model",
    "codex_reasoning_effort",
    "invoke_argon_rpc",
    "max_input_bytes",
    "max_rounds",
    "mode",
    "run_verification_gates",
    "timeout_seconds",
    "workspace_dir",
];

fn semantic_request_arguments(args: &Value) -> Value {
    let mut semantic = args.clone();
    if let Some(object) = semantic.as_object_mut() {
        for key in TRANSPORT_ARGUMENTS {
            object.remove(key);
        }
    }
    semantic
}

fn without_normalized_arguments(args: &Value, normalized: &[&str]) -> Value {
    let mut semantic = semantic_request_arguments(args);
    if let Some(object) = semantic.as_object_mut() {
        for key in normalized {
            object.remove(*key);
        }
    }
    semantic
}

fn request_identity_digest(value: &Value) -> crate::Result<String> {
    let encoded = serde_json::to_vec(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn provider_request_identity(
    name: &str,
    args: &Value,
    request: &RouteRequest,
    stored_model: &str,
    sandbox_mode: crate::routing::SandboxMode,
    duration: Duration,
    max_input_bytes: usize,
) -> crate::Result<String> {
    request_identity_digest(&json!({
        "tool": name,
        "arguments": without_normalized_arguments(args, &PROVIDER_NORMALIZED_ARGUMENTS),
        "route": {
            "provider": request.provider,
            "engine": request.engine,
            "resolved_model": request.model,
            "stored_model": stored_model,
            "reasoning_effort": request.effort,
        },
        "execution": {
            "sandbox_mode": sandbox_mode,
            "timeout_seconds": duration.as_secs_f64(),
            "max_input_bytes": max_input_bytes,
        },
    }))
}

pub(crate) fn triad_mode(args: &Value) -> String {
    let mode = optional(args, "mode")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if mode.is_empty() {
        "pipeline".to_owned()
    } else {
        mode
    }
}

pub(crate) fn triad_model_set(args: &Value) -> String {
    format!(
        "{}+{}+{}",
        optional(args, "argon_model").unwrap_or_else(|| "gemini-4-argon-high".to_owned()),
        optional(args, "claude_model").unwrap_or_else(|| "claude-sonnet-5-5-medium".to_owned()),
        optional(args, "codex_model").unwrap_or_else(|| "gpt-6-astra".to_owned()),
    )
}

pub(crate) fn triad_request_identity(args: &Value) -> crate::Result<String> {
    request_identity_digest(&json!({
        "tool": "heterarchy_triad_orchestrate",
        "arguments": without_normalized_arguments(args, &TRIAD_NORMALIZED_ARGUMENTS),
        "orchestration": {
            "mode": triad_mode(args),
            "model_set": triad_model_set(args),
            "codex_reasoning_effort": optional(args, "codex_reasoning_effort")
                .unwrap_or_else(|| "xhigh".to_owned()),
            "invoke_argon_rpc": args["invoke_argon_rpc"].as_bool().unwrap_or(true),
            "timeout_seconds": args["timeout_seconds"].as_f64().unwrap_or(3600.0),
            "max_input_bytes": args["max_input_bytes"].as_u64().unwrap_or(256_000),
            "max_rounds": args["max_rounds"].as_u64().unwrap_or(1).clamp(1, 3),
            "run_verification_gates": args["run_verification_gates"].as_bool().unwrap_or(false),
        },
    }))
}

fn pending_count(health: &Value, role: &str) -> Value {
    health["pending_handoffs"]
        .get(role)
        .cloned()
        .unwrap_or_else(|| json!(0))
}

fn execution_failure_message(value: &Value) -> String {
    value["receipt"]["failure_reason"]
        .as_str()
        .or_else(|| value["receipt"]["error"].as_str())
        .or_else(|| value["error"].as_str())
        .unwrap_or("Provider execution failed.")
        .to_owned()
}

fn enrich_handoff(mut ticket: Value) -> Value {
    let status = ticket["status"].as_str().unwrap_or("pending").to_owned();
    let role_state = match status.as_str() {
        "pending" => "queued",
        "in_progress" => "running",
        "completed" => "succeeded",
        "rejected" => "failed",
        _ => "unknown",
    };
    let verdict = ticket["resolution_note"]
        .as_str()
        .and_then(|note| serde_json::from_str::<Value>(note).ok())
        .and_then(|note| note["verdict"].as_str().map(str::to_owned));
    ticket["collaboration_role_state"] = Value::String(role_state.to_owned());
    ticket["execution_completed"] =
        Value::Bool(matches!(status.as_str(), "completed" | "rejected"));
    ticket["consensus_eligible"] =
        Value::Bool(status == "completed" && verdict.as_deref() == Some("PASS"));
    ticket
}

pub(crate) fn text(args: &Value, key: &str) -> String {
    args[key].as_str().unwrap_or_default().into()
}
pub(crate) fn optional(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::to_owned)
}
pub(crate) fn workspace(args: &Value) -> PathBuf {
    optional(args, "workspace_dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
fn limit(args: &Value) -> usize {
    usize::try_from(args["limit"].as_u64().unwrap_or(20).min(1000)).unwrap_or(1000)
}
fn field<T: DeserializeOwned>(args: &Value, key: &str) -> crate::Result<T> {
    Ok(serde_json::from_value(args[key].clone())?)
}

#[cfg(test)]
mod tests;
