//! Owned subprocess and local Antigravity Connect-RPC execution boundaries.

mod config;
pub(crate) use config::{ConfiguredCodexDefault, resolve_codex_default};

use crate::routing::{
    Engine, Receipt, ReceiptStatus, Route, RouteErrorCode, RouteRequest, SandboxMode,
    classify_stderr_auth, digest_output, new_receipt_with_context, redact_sensitive_text,
    validate_route,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;

const CONNECT_SERVICE: &str = "exa.language_server_pb.LanguageServerService";
const MAX_REVIEW_INSTRUCTIONS_ARG_BYTES: usize = 96 * 1024;

static CSRF_TOKEN_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"--csrf_token(?:=|\s+)([^\s]+)").ok());
static LISTEN_PORT_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"127\.0\.0\.1:(\d+)\s+\(LISTEN\)").ok());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub receipt: Receipt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub output: String,
    pub structured_output: Option<Value>,
    pub session_id: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    pub workspace_dir: Option<PathBuf>,
    pub sandbox_mode: SandboxMode,
    pub request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub max_input_bytes: usize,
    pub attempt_count: u32,
    /// Trusted preflight alias; actual/resolved model proof remains independent.
    pub requested_model_override: Option<String>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            workspace_dir: None,
            sandbox_mode: SandboxMode::ReadOnly,
            request_id: None,
            idempotency_key: None,
            max_input_bytes: 256_000,
            attempt_count: 1,
            requested_model_override: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexJsonl {
    pub session_id: Option<String>,
    pub messages: Vec<String>,
    pub actual_model: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub context_window: Option<u64>,
    pub current_context_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AntigravityModel {
    pub label: String,
    pub model_id: String,
    pub placeholder: String,
    pub remaining_quota: Option<f64>,
    pub routable: bool,
}

#[derive(Debug, Clone)]
pub struct AntigravityEndpoint {
    pub base_url: String,
    pub csrf_token: String,
    pub pid: u32,
}

#[derive(Default)]
struct AntigravityState {
    cached: Option<(AntigravityEndpoint, Instant)>,
    consecutive_failures: u8,
    open_until: Option<Instant>,
    probe_in_flight: bool,
}

fn antigravity_state() -> &'static Mutex<AntigravityState> {
    static STATE: OnceLock<Mutex<AntigravityState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(AntigravityState::default()))
}

fn antigravity_success(endpoint: Option<AntigravityEndpoint>) -> crate::Result<()> {
    let mut state = antigravity_state()
        .lock()
        .map_err(|_| "Antigravity state lock poisoned")?;
    if let Some(endpoint) = endpoint {
        state.cached = Some((endpoint, Instant::now()));
    }
    state.consecutive_failures = 0;
    state.open_until = None;
    state.probe_in_flight = false;
    Ok(())
}

fn antigravity_failure() -> crate::Result<()> {
    let mut state = antigravity_state()
        .lock()
        .map_err(|_| "Antigravity state lock poisoned")?;
    state.cached = None;
    state.probe_in_flight = false;
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    if state.consecutive_failures >= 3 {
        state.open_until = Some(Instant::now() + Duration::from_secs(30));
    }
    Ok(())
}

#[derive(Clone)]
pub struct AntigravityClient {
    http: reqwest::Client,
}

impl AntigravityClient {
    pub fn new() -> crate::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .build()?,
        })
    }

    pub async fn discover(&self) -> crate::Result<Option<AntigravityEndpoint>> {
        {
            let mut state = antigravity_state()
                .lock()
                .map_err(|_| "Antigravity state lock poisoned")?;
            let now = Instant::now();
            if let Some(until) = state.open_until {
                if until > now {
                    return Err("Antigravity circuit breaker is open".into());
                }
                if state.probe_in_flight {
                    return Err("Antigravity half-open probe is already in flight".into());
                }
                state.probe_in_flight = true;
                state.open_until = None;
            }
            if let Some((endpoint, cached_at)) = &state.cached
                && now.duration_since(*cached_at) < Duration::from_secs(15)
            {
                return Ok(Some(endpoint.clone()));
            }
        }
        let process_output =
            match probe_command(&["ps", "-axo", "pid,command"], Duration::from_secs(5)).await {
                Ok(output) => output,
                Err(error) => {
                    antigravity_failure()?;
                    return Err(error);
                }
            };
        let candidates = endpoint_candidates(
            &process_output,
            std::env::var("ANTIGRAVITY_CSRF_TOKEN").ok().as_deref(),
        )?;
        for (pid, token) in candidates {
            let pid_text = pid.to_string();
            let Ok(ports_output) = probe_command(
                &[
                    "lsof",
                    "-nP",
                    "-a",
                    "-p",
                    &pid_text,
                    "-iTCP",
                    "-sTCP:LISTEN",
                ],
                Duration::from_secs(5),
            )
            .await
            else {
                continue;
            };
            for port in listen_ports(&ports_output)? {
                for scheme in ["https", "http"] {
                    let endpoint = AntigravityEndpoint {
                        base_url: format!("{scheme}://127.0.0.1:{port}"),
                        csrf_token: token.clone(),
                        pid,
                    };
                    if self
                        .catalog(&endpoint, Duration::from_millis(2500))
                        .await
                        .is_ok()
                    {
                        antigravity_success(Some(endpoint.clone()))?;
                        return Ok(Some(endpoint));
                    }
                }
            }
        }
        antigravity_failure()?;
        Ok(None)
    }

    pub async fn catalog(
        &self,
        endpoint: &AntigravityEndpoint,
        limit: Duration,
    ) -> crate::Result<Vec<AntigravityModel>> {
        let payload = self
            .post(endpoint, "GetCascadeModelConfigData", json!({}), limit)
            .await?;
        parse_antigravity_catalog(&payload)
    }

    pub async fn refresh_at(
        &self,
        endpoint: &AntigravityEndpoint,
        limit: Duration,
    ) -> crate::Result<Value> {
        let response = self
            .post(endpoint, "RefreshMcpServers", json!({}), limit)
            .await?;
        let readback = self
            .post(endpoint, "GetCascadeModelConfigData", json!({}), limit)
            .await?;
        antigravity_success(Some(endpoint.clone()))?;
        Ok(json!({
            "ok": true,
            "endpoint": endpoint.base_url,
            "pid": endpoint.pid,
            "csrf_token_redacted": true,
            "refresh_response": redact_value(response, &endpoint.csrf_token),
            "readback": redact_value(readback, &endpoint.csrf_token),
        }))
    }

    pub async fn execute_with_options(
        &self,
        request: RouteRequest,
        limit: Duration,
        options: &ExecutionOptions,
    ) -> crate::Result<ExecutionResult> {
        let route = validate_route(&request)?;
        let mut failure_receipt = receipt(&route, options);
        let endpoint = match self.discover().await {
            Ok(Some(endpoint)) => endpoint,
            Ok(None) => {
                mark_failure_code(
                    &mut failure_receipt,
                    "Antigravity language_server is unavailable",
                    RouteErrorCode::DependencyUnavailable,
                );
                return Ok(empty_result(failure_receipt));
            }
            Err(error) => {
                let message = redact_sensitive_text(&error.to_string());
                let code = antigravity_discovery_error_code(&message);
                mark_failure_code(&mut failure_receipt, &message, code);
                return Ok(empty_result(failure_receipt));
            }
        };
        self.execute_at_with_options(request, limit, &endpoint, options)
            .await
    }

    pub async fn execute_at_with_options(
        &self,
        request: RouteRequest,
        limit: Duration,
        endpoint: &AntigravityEndpoint,
        options: &ExecutionOptions,
    ) -> crate::Result<ExecutionResult> {
        verify_input_budget(&request, options)?;
        let route = validate_route(&request)?;
        if route.engine != Engine::AntigravityRpc {
            return Err("route is not Antigravity RPC".into());
        }
        let mut receipt = receipt(&route, options);
        let started = Instant::now();
        let catalog = match self.catalog(endpoint, Duration::from_secs(5)).await {
            Ok(catalog) => catalog,
            Err(error) => {
                mark_failure_code(
                    &mut receipt,
                    &redact_sensitive_text(&error.to_string()),
                    RouteErrorCode::DependencyUnavailable,
                );
                return Ok(empty_result(receipt));
            }
        };
        let Some(model) = catalog
            .iter()
            .find(|item| item.model_id == route.resolved_model)
        else {
            mark_failure_code(
                &mut receipt,
                &format!(
                    "model is not advertised by Antigravity: {}",
                    route.resolved_model
                ),
                RouteErrorCode::DependencyUnavailable,
            );
            return Ok(empty_result(receipt));
        };
        if !model.routable {
            let code = if model.remaining_quota.is_some_and(|quota| quota <= 0.0) {
                RouteErrorCode::AntigravityQuotaExhausted
            } else {
                RouteErrorCode::DependencyUnavailable
            };
            mark_failure_code(
                &mut receipt,
                &format!(
                    "model is not routable or has no quota: {}",
                    route.resolved_model
                ),
                code,
            );
            return Ok(empty_result(receipt));
        }
        let response = tokio::select! {
            response = self.post(
                endpoint,
                "GetModelResponse",
                json!({"model": model.placeholder, "prompt": request.prompt}),
                limit,
            ) => response,
            () = crate::supervisor::cancelled() => {
                receipt.duration_ms = millis(started.elapsed());
                mark_cancelled(&mut receipt, "Antigravity RPC cancelled by caller; remote outcome may require reconciliation");
                return Ok(empty_result(receipt));
            }
        };
        if let Some(decision) = receipt.routing_decision.as_mut() {
            decision.placeholder = Some(model.placeholder.clone());
        }
        receipt.duration_ms = millis(started.elapsed());
        let response = match response {
            Ok(value) => {
                antigravity_success(Some(endpoint.clone()))?;
                value
            }
            Err(error) if error.to_string().contains("timed out") => {
                antigravity_failure()?;
                mark_timeout(&mut receipt, &error.to_string());
                return Ok(empty_result(receipt));
            }
            Err(error) => {
                antigravity_failure()?;
                mark_failure_code(
                    &mut receipt,
                    &redact_sensitive_text(&error.to_string()),
                    RouteErrorCode::DependencyUnavailable,
                );
                return Ok(empty_result(receipt));
            }
        };
        let provider_reported_model = response
            .get("modelId")
            .or_else(|| response.get("actualModel"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let (actual_model, identity_source) = if let Some(actual) = provider_reported_model {
            (Some(actual), Some("provider_reported".to_owned()))
        } else {
            (
                Some(model.model_id.clone()),
                Some("antigravity_catalog_verified".to_owned()),
            )
        };
        let output = response
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let (mut tokens_in, mut tokens_out) = (0, 0);
        collect_usage(&response, &mut tokens_in, &mut tokens_out);
        finish_receipt(
            &mut receipt,
            &route,
            actual_model,
            identity_source,
            &output,
            tokens_in,
            tokens_out,
        );
        Ok(ExecutionResult {
            receipt,
            placeholder: Some(model.placeholder.clone()),
            structured_output: extract_json(&output),
            output,
            session_id: None,
            exit_code: Some(0),
        })
    }

    async fn post(
        &self,
        endpoint: &AntigravityEndpoint,
        method: &str,
        payload: Value,
        limit: Duration,
    ) -> crate::Result<Value> {
        let url = format!("{}/{CONNECT_SERVICE}/{method}", endpoint.base_url);
        let body = timeout(limit, async {
            let response = self
                .http
                .post(url)
                .header("content-type", "application/json")
                .header("x-codeium-csrf-token", &endpoint.csrf_token)
                .json(&payload)
                .send()
                .await?
                .error_for_status()?;
            response.json::<Value>().await
        })
        .await
        .map_err(|_| "Antigravity RPC timed out after dispatch")??;
        Ok(body)
    }
}

fn antigravity_discovery_error_code(message: &str) -> RouteErrorCode {
    if message.contains("circuit breaker is open") || message.contains("half-open probe") {
        RouteErrorCode::AntigravityCircuitOpen
    } else {
        RouteErrorCode::DependencyUnavailable
    }
}

pub async fn refresh_mcp_servers_and_readback(limit: Duration) -> crate::Result<Value> {
    let client = AntigravityClient::new()?;
    let endpoint = client
        .discover()
        .await?
        .ok_or("Antigravity language_server is unavailable")?;
    client.refresh_at(&endpoint, limit).await
}

fn redact_value(mut value: Value, secret: &str) -> Value {
    match &mut value {
        Value::String(text) => {
            if !secret.is_empty() {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
        Value::Array(items) => {
            for item in items {
                *item = redact_value(item.take(), secret);
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                *item = redact_value(item.take(), secret);
            }
        }
        _ => {}
    }
    value
}

pub fn resolve_codex_binary() -> PathBuf {
    if let Ok(explicit) = std::env::var("CODEX_CLI_BIN")
        && !explicit.trim().is_empty()
    {
        return PathBuf::from(explicit);
    }
    if let Some(paths) = std::env::var_os("PATH")
        && let Some(found) = std::env::split_paths(&paths)
            .map(|dir| dir.join("codex"))
            .find(|path| path.is_file())
    {
        return found;
    }
    for dir in standard_binary_directories() {
        let candidate = dir.join("codex");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("codex")
}

pub(crate) fn enriched_command_path() -> std::ffi::OsString {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .collect();
    for fallback in standard_binary_directories() {
        if !directories.contains(&fallback) && fallback.is_dir() {
            directories.push(fallback);
        }
    }
    std::env::join_paths(directories)
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

fn standard_binary_directories() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    dirs
}

pub async fn execute_codex_with_options(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
) -> crate::Result<ExecutionResult> {
    let binary = resolve_codex_binary();
    execute_codex_with_binary_and_options(request, limit, &binary, options).await
}

pub async fn execute_codex_with_binary_and_options(
    request: RouteRequest,
    limit: Duration,
    binary: &Path,
    options: &ExecutionOptions,
) -> crate::Result<ExecutionResult> {
    execute_codex_with_binary_and_schema(request, limit, binary, options, None, None).await
}

pub async fn execute_codex_structured(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
    output_schema: &Value,
) -> crate::Result<ExecutionResult> {
    let binary = resolve_codex_binary();
    execute_codex_with_binary_and_schema(
        request,
        limit,
        &binary,
        options,
        Some(output_schema),
        None,
    )
    .await
}

async fn execute_codex_with_binary_and_schema(
    request: RouteRequest,
    limit: Duration,
    binary: &Path,
    options: &ExecutionOptions,
    output_schema: Option<&Value>,
    readiness_fifo: Option<&Path>,
) -> crate::Result<ExecutionResult> {
    verify_input_budget(&request, options)?;
    let route = validate_route(&request)?;
    if route.engine != Engine::CodexCli {
        return Err("route is not Codex CLI".into());
    }
    let workspace = resolve_workspace(options.workspace_dir.as_deref())?;
    let capture = CodexCapture::new(output_schema)?;
    let mut args = vec!["exec".to_owned()];
    append_codex_common(&mut args, &route, options, &workspace)?;
    capture.append_arguments(&mut args);
    args.push("-".to_owned());
    run_codex_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        CodexProcessIo {
            binary,
            output_file: Some(&capture.output_file),
            readiness_fifo,
        },
    )
    .await
}

pub async fn health() -> crate::Result<Value> {
    health_with_models().await.map(|(value, _)| value)
}

pub(crate) async fn health_with_models() -> crate::Result<(Value, Vec<AntigravityModel>)> {
    let codex_probe = async {
        let result = timeout(
            Duration::from_secs(5),
            Command::new(resolve_codex_binary())
                .env("PATH", enriched_command_path())
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await;
        match result {
            Ok(Ok(output)) => json!({
                "ready": output.status.success(),
                "version": String::from_utf8_lossy(&output.stdout).trim(),
                "auth_context": "codex_chatgpt_oauth",
                "independent_of_antigravity_rpc": true,
            }),
            Ok(Err(error)) => json!({"ready": false, "error": error.to_string()}),
            Err(_) => json!({"ready": false, "error": "Codex version probe timed out"}),
        }
    };
    let antigravity_probe = async {
        let client = match AntigravityClient::new() {
            Ok(client) => client,
            Err(error) => {
                return (
                    json!({"connected": false, "error": error.to_string()}),
                    Vec::new(),
                );
            }
        };
        let endpoint = match client.discover().await {
            Ok(Some(endpoint)) => endpoint,
            Ok(None) => {
                return (
                    json!({
                        "connected": false,
                        "auth_context": "antigravity_local_csrf",
                        "csrf_token_redacted": true,
                        "independent_of_codex_cli_oauth": true,
                        "routable_models": [],
                    }),
                    Vec::new(),
                );
            }
            Err(error) => {
                return (
                    json!({
                        "connected": false,
                        "auth_context": "antigravity_local_csrf",
                        "csrf_token_redacted": true,
                        "error": redact_stderr(&error.to_string()),
                    }),
                    Vec::new(),
                );
            }
        };
        let models = match client.catalog(&endpoint, Duration::from_secs(5)).await {
            Ok(models) => models,
            Err(error) => {
                return (
                    json!({
                        "connected": true,
                        "endpoint": endpoint.base_url,
                        "auth_context": "antigravity_local_csrf",
                        "csrf_token_redacted": true,
                        "catalog_ready": false,
                        "error": redact_stderr(&error.to_string()),
                    }),
                    Vec::new(),
                );
            }
        };
        let routable = models
            .iter()
            .filter(|model| model.routable)
            .map(|model| model.model_id.clone())
            .collect::<Vec<_>>();
        (
            json!({
                "connected": true,
                "endpoint": endpoint.base_url,
                "auth_context": "antigravity_local_csrf",
                "csrf_token_redacted": true,
                "independent_of_codex_cli_oauth": true,
                "routable_models": routable,
            }),
            models,
        )
    };
    let (mut codex_cli, (mut antigravity_rpc, models)) =
        tokio::join!(codex_probe, antigravity_probe);
    let (circuit_open, consecutive_failures, half_open_probe) = antigravity_circuit_health()?;
    codex_cli["auth_diagnostics"] =
        serde_json::to_value(crate::routing::AuthDiagnostics::default())?;
    antigravity_rpc["auth_diagnostics"] =
        serde_json::to_value(crate::routing::AuthDiagnostics::default())?;
    antigravity_rpc["circuit_breaker_open"] = Value::Bool(circuit_open);
    antigravity_rpc["consecutive_failures"] = json!(consecutive_failures);
    antigravity_rpc["half_open_probe"] = Value::Bool(half_open_probe);
    Ok((
        json!({
            "ok": true,
            "routing_contract": {"silent_fallback_enabled": false, "claude_tier_policy": "medium_only"},
            "auth_boundaries": {"codex_cli_oauth": codex_cli, "antigravity_rpc": antigravity_rpc},
        }),
        models,
    ))
}

fn antigravity_circuit_health() -> crate::Result<(bool, u8, bool)> {
    let state = antigravity_state()
        .lock()
        .map_err(|_| "Antigravity state lock poisoned")?;
    Ok((
        state.open_until.is_some_and(|until| until > Instant::now()),
        state.consecutive_failures,
        state.probe_in_flight,
    ))
}

pub async fn codex_session_followup_with_options(
    request: RouteRequest,
    session_id: &str,
    fork: bool,
    limit: Duration,
    options: &ExecutionOptions,
) -> crate::Result<ExecutionResult> {
    validate_identifier(session_id, "session ID")?;
    let route = validate_route(&request)?;
    let workspace = resolve_workspace(options.workspace_dir.as_deref())?;
    let mut args = vec![
        "exec".to_owned(),
        if fork { "fork" } else { "resume" }.to_owned(),
        session_id.to_owned(),
    ];
    append_codex_common(&mut args, &route, options, &workspace)?;
    args.push("-".to_owned());
    let binary = resolve_codex_binary();
    run_codex_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        CodexProcessIo {
            binary: &binary,
            output_file: None,
            readiness_fifo: None,
        },
    )
    .await
}

#[derive(Debug, Clone, Default)]
pub struct ReviewTarget {
    pub uncommitted: bool,
    pub base_branch: Option<String>,
    pub commit_sha: Option<String>,
}

struct CodexCapture {
    directory: PathBuf,
    output_file: PathBuf,
    schema_file: Option<PathBuf>,
}

impl CodexCapture {
    fn new(schema: Option<&Value>) -> crate::Result<Self> {
        let directory = std::env::temp_dir().join(format!("hcodex-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        let output_file = directory.join("last-message.txt");
        let schema_file = if let Some(value) = schema {
            let path = directory.join("schema.json");
            std::fs::write(&path, serde_json::to_vec_pretty(value)?)?;
            Some(path)
        } else {
            None
        };
        Ok(Self {
            directory,
            output_file,
            schema_file,
        })
    }

    fn append_arguments(&self, args: &mut Vec<String>) {
        args.extend(["-o".to_owned(), self.output_file.display().to_string()]);
        if let Some(schema_file) = &self.schema_file {
            args.extend([
                "--output-schema".to_owned(),
                schema_file.display().to_string(),
            ]);
        }
    }
}

impl Drop for CodexCapture {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.directory);
    }
}

pub async fn codex_git_review_with_options(
    request: RouteRequest,
    target: ReviewTarget,
    limit: Duration,
    options: &ExecutionOptions,
) -> crate::Result<ExecutionResult> {
    let binary = resolve_codex_binary();
    codex_git_review_with_binary(request, target, limit, options, &binary).await
}

async fn codex_git_review_with_binary(
    mut request: RouteRequest,
    target: ReviewTarget,
    limit: Duration,
    options: &ExecutionOptions,
    binary: &Path,
) -> crate::Result<ExecutionResult> {
    verify_input_budget(&request, options)?;
    let route = validate_route(&request)?;
    let workspace = resolve_workspace(options.workspace_dir.as_deref())?;
    // The exec review surface exposes JSONL model/session/usage receipts;
    // plain `codex review` cannot provide the identity proof required here.
    let mut args = vec![
        "exec".to_owned(),
        "review".to_owned(),
        "--json".to_owned(),
        "--ignore-user-config".to_owned(),
        "-m".to_owned(),
        route.resolved_model.clone(),
    ];
    let has_target = if let Some(commit) = target
        .commit_sha
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_identifier(commit, "commit SHA")?;
        args.extend(["--commit".to_owned(), commit.to_owned()]);
        true
    } else if let Some(branch) = target
        .base_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_identifier(branch, "base branch")?;
        args.extend(["--base".to_owned(), branch.to_owned()]);
        true
    } else if target.uncommitted {
        args.push("--uncommitted".to_owned());
        true
    } else {
        false
    };
    args.extend([
        "-c".to_owned(),
        "mcp_servers={}".to_owned(),
        "-c".to_owned(),
        format!(
            "model_reasoning_effort={}",
            serde_json::to_string(&route.effort)?
        ),
        "-c".to_owned(),
        format!(
            "sandbox_mode={}",
            serde_json::to_string(options.sandbox_mode.as_str())?
        ),
    ]);
    if has_target && !request.prompt.trim().is_empty() {
        // Native review rejects target flags combined with positional PROMPT.
        // Preserve the native target and pass supplemental instructions through
        // its supported config key, with a conservative escaped argv limit.
        let instructions = format!(
            "developer_instructions={}",
            serde_json::to_string(&request.prompt)?
        );
        if instructions.len() > MAX_REVIEW_INSTRUCTIONS_ARG_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Native targeted review instructions exceed the 96 KiB escaped argument budget; no review was dispatched",
            )
            .into());
        }
        args.extend(["-c".to_owned(), instructions]);
        request.prompt = String::new();
    } else if !request.prompt.trim().is_empty() {
        args.push("-".to_owned());
    }
    run_codex_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        CodexProcessIo {
            binary,
            output_file: None,
            readiness_fifo: None,
        },
    )
    .await
}

fn append_codex_common(
    args: &mut Vec<String>,
    route: &Route,
    options: &ExecutionOptions,
    workspace: &Path,
) -> crate::Result<()> {
    args.extend([
        "--ignore-user-config".to_owned(),
        "-c".to_owned(),
        "mcp_servers={}".to_owned(),
        "--cd".to_owned(),
        workspace.display().to_string(),
        "--sandbox".to_owned(),
        options.sandbox_mode.as_str().to_owned(),
        "--skip-git-repo-check".to_owned(),
        "--color".to_owned(),
        "never".to_owned(),
        "--json".to_owned(),
        "-m".to_owned(),
        route.resolved_model.clone(),
        "-c".to_owned(),
        format!(
            "model_reasoning_effort={}",
            serde_json::to_string(&route.effort)?
        ),
    ]);
    Ok(())
}

#[derive(Clone, Copy)]
struct CodexProcessIo<'a> {
    binary: &'a Path,
    output_file: Option<&'a Path>,
    readiness_fifo: Option<&'a Path>,
}

async fn run_codex_process(
    route: &Route,
    request: &RouteRequest,
    options: &ExecutionOptions,
    workspace: &Path,
    args: &[String],
    limit: Duration,
    io: CodexProcessIo<'_>,
) -> crate::Result<ExecutionResult> {
    let mut receipt = receipt(route, options);
    let started = Instant::now();
    let mut command = Command::new(io.binary);
    command
        .env("PATH", enriched_command_path())
        .args(args)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let process_group = child.id();
    let mut stdin_pipe = child.stdin.take();
    if let Some(readiness_fifo) = io.readiness_fifo {
        if let Some(mut stdin) = stdin_pipe.take() {
            stdin.write_all(request.prompt.as_bytes()).await?;
            stdin.shutdown().await?;
        }
        await_child_readiness(&mut child, readiness_fifo).await?;
    }
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or("Codex child stdout was not piped")?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or("Codex child stderr was not piped")?;
    let (mut stdout_bytes, mut stderr_bytes) = (Vec::new(), Vec::new());
    enum WaitOutcome {
        Completed(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }
    let wait_outcome = {
        let stdin_writer = async {
            if let Some(mut stdin) = stdin_pipe.take() {
                stdin.write_all(request.prompt.as_bytes()).await?;
                stdin.shutdown().await?;
            }
            Ok::<_, std::io::Error>(())
        };
        let execution = async {
            let (_, status, _, _) = tokio::try_join!(
                stdin_writer,
                child.wait(),
                stdout_pipe.read_to_end(&mut stdout_bytes),
                stderr_pipe.read_to_end(&mut stderr_bytes),
            )?;
            Ok::<_, std::io::Error>(status)
        };
        tokio::pin!(execution);
        tokio::select! {
            result = &mut execution => WaitOutcome::Completed(result),
            () = tokio::time::sleep(limit) => WaitOutcome::TimedOut,
            () = crate::supervisor::cancelled() => WaitOutcome::Cancelled,
        }
    };
    let status = match wait_outcome {
        WaitOutcome::Completed(Ok(status)) => status,
        WaitOutcome::Completed(Err(error)) => {
            terminate_process_group(&mut child, process_group).await?;
            return Err(error.into());
        }
        WaitOutcome::TimedOut => {
            terminate_process_group(&mut child, process_group).await?;
            receipt.duration_ms = millis(started.elapsed());
            mark_timeout(
                &mut receipt,
                "Codex CLI timed out after dispatch; query this request ID before retrying",
            );
            return Ok(empty_result(receipt));
        }
        WaitOutcome::Cancelled => {
            terminate_process_group(&mut child, process_group).await?;
            receipt.duration_ms = millis(started.elapsed());
            mark_cancelled(&mut receipt, "Codex CLI execution cancelled by caller");
            return Ok(empty_result(receipt));
        }
    };
    receipt.owned_processes_closed =
        Some(crate::verification::owned_process_group_closed(process_group).await);
    receipt.duration_ms = millis(started.elapsed());
    let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
    let auth_diagnostics = classify_stderr_auth(&String::from_utf8_lossy(&stderr_bytes));
    let stderr = auth_diagnostics.sanitized_stderr.clone();
    receipt.auth_diagnostics = auth_diagnostics;
    let parsed = parse_codex_jsonl(&stdout);
    let final_output = io
        .output_file
        .and_then(|path| std::fs::read_to_string(path).ok())
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.trim().to_owned())
        .or_else(|| parsed.messages.last().cloned())
        .unwrap_or_else(|| stdout.trim().to_owned());
    if status.success() {
        let output_digest = digest_output(&final_output);
        let session_proof = if let Some(session_id) = parsed.session_id.clone() {
            let workspace_path = workspace.to_path_buf();
            let digest = output_digest.clone();
            tokio::task::spawn_blocking(move || {
                prove_codex_session_identity(&session_id, &workspace_path, &digest)
            })
            .await?
        } else {
            None
        };
        let (actual_model, identity_source) = if let Some(actual) = parsed.actual_model.clone() {
            (Some(actual), Some("codex_jsonl_reported".to_owned()))
        } else if let Some(proof) = &session_proof {
            (Some(proof.model.clone()), Some(proof.source.clone()))
        } else {
            (None, None)
        };
        let context_window = parsed.context_window.or_else(|| {
            session_proof
                .as_ref()
                .and_then(|proof| proof.context_window)
        });
        let current_context_tokens = parsed.current_context_tokens.or_else(|| {
            session_proof
                .as_ref()
                .and_then(|proof| proof.current_context_tokens)
        });
        finish_receipt(
            &mut receipt,
            route,
            actual_model,
            identity_source,
            &final_output,
            parsed.tokens_in,
            parsed.tokens_out,
        );
        receipt.context_window = context_window;
        receipt.current_context_tokens = current_context_tokens;
    } else {
        mark_failure(
            &mut receipt,
            if stderr.is_empty() {
                &final_output
            } else {
                &stderr
            },
        );
    }
    Ok(ExecutionResult {
        structured_output: extract_json(&final_output),
        receipt,
        placeholder: None,
        output: final_output,
        session_id: parsed.session_id,
        exit_code: status.code(),
    })
}

pub fn parse_codex_jsonl(stdout: &str) -> CodexJsonl {
    let mut parsed = CodexJsonl::default();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
    {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        find_string(
            &value,
            &["thread_id", "session_id", "conversation_id"],
            &mut parsed.session_id,
        );
        find_string(
            &value,
            &["model", "model_id", "actual_model"],
            &mut parsed.actual_model,
        );
        collect_usage(&value, &mut parsed.tokens_in, &mut parsed.tokens_out);
        collect_context_usage(
            &value,
            &mut parsed.context_window,
            &mut parsed.current_context_tokens,
        );
        if let Some(item) = value.get("item").and_then(Value::as_object)
            && matches!(
                item.get("type").and_then(Value::as_str),
                Some("agent_message" | "message")
            )
            && let Some(message) = item
                .get("text")
                .or_else(|| item.get("content"))
                .and_then(Value::as_str)
        {
            parsed.messages.push(message.trim().to_owned());
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            parsed.messages.push(message.trim().to_owned());
        }
    }
    parsed
}

struct SessionIdentityProof {
    model: String,
    source: String,
    context_window: Option<u64>,
    current_context_tokens: Option<u64>,
}

fn prove_codex_session_identity(
    session_id: &str,
    workspace: &Path,
    output_digest: &str,
) -> Option<SessionIdentityProof> {
    let sessions = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))?
        .join("sessions");
    prove_codex_session_identity_at(&sessions, session_id, workspace, output_digest)
}

fn prove_codex_session_identity_at(
    sessions: &Path,
    session_id: &str,
    workspace: &Path,
    output_digest: &str,
) -> Option<SessionIdentityProof> {
    if session_id.is_empty() || !sessions.is_dir() {
        return None;
    }
    let artifact = walkdir::WalkDir::new(sessions)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .find(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(&format!("{session_id}.jsonl")))
        })?;
    let workspace = workspace.canonicalize().ok()?;
    let source = std::fs::read_to_string(artifact.path()).ok()?;
    let mut session_metadata_matches = false;
    let mut current_model: Option<String> = None;
    let mut current_cwd_matches = false;
    let mut settings_model: Option<String> = None;
    let mut settings_cwd_matches = false;
    let mut verified_models = BTreeSet::new();
    let mut turn_generation = 0_u64;
    let mut matched_generation = None;
    let mut context_window = None;
    let mut current_context_tokens = None;
    for line in source.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match event["type"].as_str() {
            Some("session_meta") => {
                session_metadata_matches = event["payload"]["id"].as_str() == Some(session_id)
                    && event["payload"]["cwd"]
                        .as_str()
                        .is_some_and(|cwd| same_canonical_path(Path::new(cwd), &workspace));
            }
            Some("turn_context") => {
                turn_generation = turn_generation.saturating_add(1);
                current_model = event["payload"]["model"].as_str().map(str::to_owned);
                current_cwd_matches = event["payload"]["cwd"]
                    .as_str()
                    .is_some_and(|cwd| same_canonical_path(Path::new(cwd), &workspace));
            }
            Some("event_msg")
                if event["payload"]["type"].as_str() == Some("thread_settings_applied") =>
            {
                let settings = &event["payload"]["thread_settings"];
                settings_model = settings["model"].as_str().map(str::to_owned);
                settings_cwd_matches = settings["cwd"]
                    .as_str()
                    .is_some_and(|cwd| same_canonical_path(Path::new(cwd), &workspace));
            }
            Some("response_item")
                if event["payload"]["type"].as_str() == Some("message")
                    && event["payload"]["role"].as_str() == Some("assistant") =>
            {
                let content = event["payload"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["type"].as_str() == Some("output_text"))
                    .filter_map(|item| item["text"].as_str())
                    .collect::<String>();
                if digest_output(content.trim()) == output_digest
                    && session_metadata_matches
                    && current_cwd_matches
                    && settings_cwd_matches
                    && current_model == settings_model
                    && let Some(model) = &current_model
                {
                    verified_models.insert(model.clone());
                    matched_generation = Some(turn_generation);
                }
            }
            Some("event_msg")
                if event["payload"]["type"].as_str() == Some("token_count")
                    && matched_generation == Some(turn_generation) =>
            {
                collect_context_usage(
                    &event["payload"],
                    &mut context_window,
                    &mut current_context_tokens,
                );
            }
            _ => {}
        }
    }
    if verified_models.len() != 1 {
        return None;
    }
    Some(SessionIdentityProof {
        model: verified_models.into_iter().next()?,
        source: "codex_turn_context_verified".to_owned(),
        context_window,
        current_context_tokens,
    })
}

fn same_canonical_path(candidate: &Path, expected: &Path) -> bool {
    candidate
        .canonicalize()
        .is_ok_and(|candidate| candidate == expected)
}

fn find_string(value: &Value, keys: &[&str], destination: &mut Option<String>) {
    if let Some(object) = value.as_object() {
        for key in keys {
            if let Some(found) = object
                .get(*key)
                .and_then(Value::as_str)
                .filter(|v| !v.trim().is_empty())
            {
                *destination = Some(found.trim().to_owned());
            }
        }
        for child in object.values() {
            find_string(child, keys, destination);
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            find_string(child, keys, destination);
        }
    }
}

fn collect_usage(value: &Value, input: &mut u64, output: &mut u64) {
    if let Some(object) = value.as_object() {
        for (key, item) in object {
            if matches!(key.as_str(), "input_tokens" | "tokens_in") {
                *input = (*input).max(item.as_u64().unwrap_or(0));
            }
            if matches!(key.as_str(), "output_tokens" | "tokens_out") {
                *output = (*output).max(item.as_u64().unwrap_or(0));
            }
            collect_usage(item, input, output);
        }
    } else if let Some(array) = value.as_array() {
        for item in array {
            collect_usage(item, input, output);
        }
    }
}

fn collect_context_usage(
    value: &Value,
    context_window: &mut Option<u64>,
    current_context_tokens: &mut Option<u64>,
) {
    if let Some(object) = value.as_object() {
        if let Some(window) = object.get("model_context_window").and_then(Value::as_u64) {
            *context_window = Some(window);
            *current_context_tokens = object
                .get("last_token_usage")
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64);
        }
        for child in object.values() {
            collect_context_usage(child, context_window, current_context_tokens);
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            collect_context_usage(child, context_window, current_context_tokens);
        }
    }
}

fn finish_receipt(
    receipt: &mut Receipt,
    route: &Route,
    actual_model: Option<String>,
    identity_source: Option<String>,
    output: &str,
    tokens_in: u64,
    tokens_out: u64,
) {
    crate::routing::close_receipt(receipt);
    receipt.actual_model = actual_model;
    receipt.identity_source = identity_source;
    receipt.tokens_in = tokens_in;
    receipt.tokens_out = tokens_out;
    receipt.output_digest = Some(digest_output(output));
    receipt.model_mismatch = receipt.actual_model.as_deref() != Some(route.resolved_model.as_str());
    if receipt.model_mismatch {
        mark_failure_code(
            receipt,
            "provider did not prove the requested model identity",
            RouteErrorCode::ActualModelMismatch,
        );
    } else {
        receipt.status = ReceiptStatus::Completed;
        receipt.outcome_state = "confirmed_success".to_owned();
    }
}

fn receipt(route: &Route, options: &ExecutionOptions) -> Receipt {
    let mut receipt = new_receipt_with_context(
        route,
        options.request_id.as_deref(),
        options.idempotency_key.clone(),
    );
    receipt.attempt_count = options.attempt_count.max(1);
    apply_requested_model_override(&mut receipt, options.requested_model_override.as_deref());
    receipt
}

pub(crate) fn apply_requested_model_override(receipt: &mut Receipt, requested: Option<&str>) {
    if let Some(requested) = requested {
        receipt.requested_model = requested.to_owned();
        if let Some(decision) = receipt.routing_decision.as_mut() {
            decision.requested_model = requested.to_owned();
        }
    }
}

fn verify_input_budget(request: &RouteRequest, options: &ExecutionOptions) -> crate::Result<()> {
    if request.prompt.len() > options.max_input_bytes {
        return Err("prompt exceeds max_input_bytes".into());
    }
    Ok(())
}

fn mark_timeout(receipt: &mut Receipt, error: &str) {
    crate::routing::close_receipt(receipt);
    receipt.status = ReceiptStatus::TimedOutUnknown;
    receipt.outcome_state = "outcome_unknown".to_owned();
    receipt.error = Some(error.to_owned());
    receipt.failure_reason = Some(error.to_owned());
}
fn mark_failure(receipt: &mut Receipt, error: &str) {
    crate::routing::close_receipt(receipt);
    receipt.status = ReceiptStatus::Failed;
    receipt.outcome_state = "confirmed_failure".to_owned();
    receipt.error = Some(error.to_owned());
    receipt.failure_reason = Some(error.to_owned());
}
fn mark_failure_code(receipt: &mut Receipt, error: &str, error_code: RouteErrorCode) {
    mark_failure(receipt, error);
    receipt.error_code = Some(error_code);
}
fn mark_cancelled(receipt: &mut Receipt, error: &str) {
    crate::routing::close_receipt(receipt);
    receipt.status = ReceiptStatus::Cancelled;
    receipt.outcome_state = "cancelled".to_owned();
    receipt.error = Some(error.to_owned());
    receipt.failure_reason = Some(error.to_owned());
}
fn empty_result(receipt: Receipt) -> ExecutionResult {
    ExecutionResult {
        receipt,
        placeholder: None,
        output: String::new(),
        structured_output: None,
        session_id: None,
        exit_code: None,
    }
}

fn extract_json(text: &str) -> Option<Value> {
    // Bound decoder work while accepting the legacy first-object contract.
    if text.len() > 4 * 1024 * 1024 {
        return None;
    }
    if let Ok(value) = serde_json::from_str(text.trim()) {
        return Some(value);
    }
    for (offset, _) in text
        .char_indices()
        .filter(|(_, character)| *character == '{')
        .take(256)
    {
        if let Some(Ok(value @ Value::Object(_))) =
            serde_json::Deserializer::from_str(&text[offset..])
                .into_iter::<Value>()
                .next()
        {
            return Some(value);
        }
    }
    None
}

fn parse_antigravity_catalog(value: &Value) -> crate::Result<Vec<AntigravityModel>> {
    let entries = value
        .get("clientModelConfigs")
        .and_then(Value::as_array)
        .ok_or("missing clientModelConfigs")?;
    Ok(entries
        .iter()
        .filter_map(|item| {
            let model_id = item.get("modelId")?.as_str()?.to_owned();
            let placeholder = item.get("modelOrAlias")?.get("model")?.as_str()?.to_owned();
            let remaining_quota = item
                .get("quotaInfo")
                .and_then(|q| q.get("remainingFraction"))
                .and_then(Value::as_f64);
            let claude_wrong_tier =
                model_id.starts_with("claude-") && !model_id.ends_with("-medium");
            Some(AntigravityModel {
                label: item
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                model_id,
                routable: !placeholder.is_empty()
                    && remaining_quota.is_none_or(|quota| quota > 0.0)
                    && !claude_wrong_tier,
                placeholder,
                remaining_quota,
            })
        })
        .collect())
}

fn endpoint_candidates(
    ps_output: &str,
    fallback: Option<&str>,
) -> crate::Result<Vec<(u32, String)>> {
    let token_re = CSRF_TOKEN_RE
        .as_ref()
        .ok_or("CSRF token regex unavailable")?;
    Ok(ps_output
        .lines()
        .filter_map(|line| {
            if !line.contains("language_server") {
                return None;
            }
            let mut pieces = line.trim().splitn(2, char::is_whitespace);
            let pid = pieces.next()?.parse().ok()?;
            let command = pieces.next()?;
            let token = token_re
                .captures(command)
                .and_then(|cap| cap.get(1))
                .map(|m| m.as_str())
                .or(fallback)?;
            Some((pid, token.to_owned()))
        })
        .collect())
}

fn listen_ports(output: &str) -> crate::Result<Vec<u16>> {
    let port_re = LISTEN_PORT_RE
        .as_ref()
        .ok_or("listen port regex unavailable")?;
    Ok(port_re
        .captures_iter(output)
        .filter_map(|cap| cap.get(1)?.as_str().parse().ok())
        .collect())
}

async fn probe_command(args: &[&str], limit: Duration) -> crate::Result<String> {
    let (program, command_args) = args.split_first().ok_or("empty probe command")?;
    let output = timeout(
        limit,
        Command::new(program)
            .args(command_args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "probe command timed out")??;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn terminate_process_group(
    child: &mut tokio::process::Child,
    group: Option<u32>,
) -> crate::Result<()> {
    #[cfg(unix)]
    if let Some(pid) = group {
        let group = format!("-{pid}");
        let _kill_status = Command::new("/bin/kill")
            .args(["-KILL", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await?;
    }
    #[cfg(not(unix))]
    let _unused_group = group;
    if let Err(error) = child.start_kill()
        && !matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
        )
    {
        return Err(error.into());
    }
    let _status = child.wait().await?;
    Ok(())
}

async fn await_child_readiness(
    child: &mut tokio::process::Child,
    readiness_fifo: &Path,
) -> crate::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    let open = options.open(readiness_fifo);
    tokio::pin!(open);
    let mut reader = tokio::select! {
        result = &mut open => result?,
        status = child.wait() => {
            return Err(format!("child exited before readiness handshake: {}", status?).into());
        }
    };
    let mut ready = [0_u8; 1];
    reader.read_exact(&mut ready).await?;
    if ready != *b"R" {
        return Err("child readiness handshake returned an invalid marker".into());
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> crate::Result<()> {
    if value.trim().is_empty() || value.starts_with('-') || value.contains('\0') {
        return Err(
            format!("{label} must be non-empty and cannot start with '-' or contain NUL").into(),
        );
    }
    Ok(())
}
fn resolve_workspace(path: Option<&Path>) -> crate::Result<PathBuf> {
    Ok(path
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir()?)
        .canonicalize()?)
}
fn redact_stderr(stderr: &str) -> String {
    redact_sensitive_text(stderr)
}
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
