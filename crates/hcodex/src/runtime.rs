//! Owned subprocess and local Antigravity Connect-RPC execution boundaries.

mod antigravity_cli;
mod cli_io;
mod config;
mod search;
pub use antigravity_cli::{
    execute_antigravity_with_binary_and_options, execute_antigravity_with_options,
    resolve_antigravity_binary,
};
pub(crate) use config::{ConfiguredCodexDefault, resolve_codex_default};
pub use search::{SearchEvidence, SearchPolicy, SearchStopReason, execute_codex_web_discovery};

use crate::routing::{
    Engine, Receipt, ReceiptStatus, Route, RouteErrorCode, RouteRequest, SandboxMode,
    classify_stderr_auth, digest_output, new_receipt_with_context, redact_sensitive_text,
    validate_route,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const MAX_REVIEW_INSTRUCTIONS_ARG_BYTES: usize = 96 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub receipt: Receipt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub output: String,
    pub structured_output: Option<Value>,
    pub session_id: Option<String>,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_evidence: Option<SearchEvidence>,
}

pub type ProgressObserver = std::sync::Arc<
    dyn Fn(
            crate::routing::ExecutionProgress,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct ExecutionOptions {
    pub workspace_dir: Option<PathBuf>,
    pub sandbox_mode: SandboxMode,
    pub request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub max_input_bytes: usize,
    pub attempt_count: u32,
    /// Trusted preflight alias; actual/resolved model proof remains independent.
    pub requested_model_override: Option<String>,
    pub progress: Option<ProgressObserver>,
}

impl std::fmt::Debug for ExecutionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionOptions")
            .field("workspace_dir", &self.workspace_dir)
            .field("sandbox_mode", &self.sandbox_mode)
            .field("request_id", &self.request_id)
            .field("attempt_count", &self.attempt_count)
            .field("max_input_bytes", &self.max_input_bytes)
            .field("requested_model_override", &self.requested_model_override)
            .field("progress_observer", &self.progress.is_some())
            .finish()
    }
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
            progress: None,
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
    readiness_marker: Option<&Path>,
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
    Box::pin(run_codex_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        CodexProcessIo {
            binary,
            output_file: Some(&capture.output_file),
            readiness_marker,
        },
    ))
    .await
}

pub async fn health() -> crate::Result<Value> {
    health_with_models().await.map(|(value, _)| value)
}

pub(crate) async fn health_with_models() -> crate::Result<(Value, Vec<AntigravityModel>)> {
    let codex_probe = async {
        match cli_io::probe(&resolve_codex_binary(), &["--version"]).await {
            Ok(version) => json!({
                "ready": true,
                "version": version.trim(),
                "auth_context": "codex_chatgpt_oauth",
                "independent_of_antigravity_rpc": true,
            }),
            Err(error) => json!({"ready": false, "error": redact_stderr(&error.to_string())}),
        }
    };
    let antigravity_probe = async {
        let Some(binary) = resolve_antigravity_binary() else {
            return (
                json!({"connected":false,"ready":false,"transport":"cli","error":"Antigravity CLI executable not found or not a regular file"}),
                Vec::new(),
            );
        };
        let version = match cli_io::probe(&binary, &["--version"]).await {
            Ok(version) => version.trim().to_owned(),
            Err(error) => {
                return (
                    json!({"connected":false,"ready":false,"transport":"cli","error":redact_stderr(&error.to_string())}),
                    Vec::new(),
                );
            }
        };
        let readonly_admission_verified = antigravity_cli::verified_readonly_binary(&binary).await;
        let catalog = match cli_io::probe(&binary, &["models"]).await {
            Ok(catalog) => catalog,
            Err(error) => {
                return (
                    json!({"connected":true,"ready":true,"transport":"cli","version":version,"catalog_ready":false,"readonly_admission_verified":readonly_admission_verified,"error":redact_stderr(&error.to_string())}),
                    Vec::new(),
                );
            }
        };
        let mut ids = BTreeSet::new();
        for token in catalog.split(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '.'))
        {
            if let Some(model) = crate::routing::normalize_model(token)
                && (model.starts_with("gemini-") || model.starts_with("claude-"))
            {
                ids.insert(model);
            }
        }
        let models = ids
            .iter()
            .map(|id| AntigravityModel {
                label: id.clone(),
                model_id: id.clone(),
                placeholder: id.clone(),
                remaining_quota: None,
                routable: id.rsplit_once('-').is_some_and(|(_, effort)| {
                    validate_route(&RouteRequest {
                        provider: crate::routing::Provider::Antigravity,
                        engine: Engine::AntigravityCli,
                        model: id.clone(),
                        effort: effort.to_owned(),
                        prompt: "catalog preflight".to_owned(),
                    })
                    .is_ok()
                }),
            })
            .collect::<Vec<_>>();
        let routable = models
            .iter()
            .filter(|entry| entry.routable)
            .map(|entry| entry.model_id.as_str())
            .collect::<Vec<_>>();
        (
            json!({"connected":true,"ready":true,"transport":"cli","version":version,"auth_context":"antigravity_cli_cached_credentials","catalog_ready":!models.is_empty(),"readonly_admission_verified":readonly_admission_verified,"catalog_scope":"CLI advertised model slugs; live inference and quota require separate receipts","routable_models":routable,"independent_of_codex_cli_oauth":true}),
            models,
        )
    };
    let (mut codex_cli, (mut antigravity_cli, models)) =
        tokio::join!(codex_probe, antigravity_probe);
    codex_cli["auth_diagnostics"] =
        serde_json::to_value(crate::routing::AuthDiagnostics::default())?;
    antigravity_cli["auth_diagnostics"] =
        serde_json::to_value(crate::routing::AuthDiagnostics::default())?;
    Ok((
        json!({"ok":true,"routing_contract":{"silent_fallback_enabled":false,"claude_tier_policy":"medium_only"},"auth_boundaries":{"codex_cli_oauth":codex_cli,"antigravity_cli":antigravity_cli,"antigravity_rpc":{"connected":false,"retired":true}}}),
        models,
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
    Box::pin(run_codex_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        CodexProcessIo {
            binary: &binary,
            output_file: None,
            readiness_marker: None,
        },
    ))
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
    Box::pin(run_codex_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        CodexProcessIo {
            binary,
            output_file: None,
            readiness_marker: None,
        },
    ))
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
    readiness_marker: Option<&'a Path>,
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
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            receipt.owned_processes_closed = Some(true);
            mark_failure(&mut receipt, "Codex CLI process could not start");
            return Ok(empty_result(receipt));
        }
    };
    let group = child.id();
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _cleanup = terminate_process_group(&mut child, group).await;
        receipt.owned_processes_closed =
            Some(crate::verification::owned_process_group_closed(group).await);
        mark_failure(&mut receipt, "Codex CLI pipes unavailable");
        return Ok(empty_result(receipt));
    };
    let prompt = request.prompt.clone();
    let writer = tokio::spawn(async move {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await
    });
    let (sender, mut lines) = tokio::sync::mpsc::channel(2);
    let reader = tokio::spawn(cli_io::read_lines(stdout, sender));
    let errors = tokio::spawn(cli_io::read_stderr(stderr));
    let mut failure: Option<&'static str> = None;
    if let Some(marker) = io.readiness_marker {
        let ready = tokio::select! {
            value=tokio::time::timeout(Duration::from_secs(30),await_child_readiness(&mut child,marker))=>matches!(value,Ok(Ok(()))),
            ()=crate::supervisor::cancelled()=>false,
        };
        if !ready {
            failure = Some("cli_readiness_unconfirmed");
        }
    }
    let deadline = crate::execution_policy::ExecutionDeadline::from_limit(limit);
    let mut parsed = CodexJsonl::default();
    let mut sequence = 0_u64;
    let mut stdout_bytes = 0_u64;
    let mut status = None;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut natural_group_closed = true;
    let mut exit_drain: Option<tokio::time::Instant> = None;
    let mut stdout_open = true;
    while failure.is_none() && (stdout_open || status.is_none()) {
        tokio::select! {
            ()=deadline.wait()=>{timed_out=true;break;}
            ()=crate::supervisor::cancelled()=>{cancelled=true;break;}
            ()=async {match exit_drain {Some(at)=>tokio::time::sleep_until(at).await,None=>std::future::pending::<()>().await}}=>{failure=Some("cli_pipe_drain_unconfirmed");break;}
            value=child.wait(),if status.is_none()=>{
                match value {Ok(value)=>status=Some(value),Err(_)=>{failure=Some("cli_wait_failed");break;}}
                natural_group_closed=crate::verification::owned_process_group_closed(group).await;
                if !natural_group_closed {let _cleanup=terminate_process_group(&mut child,group).await;}
                exit_drain=Some(tokio::time::Instant::now()+Duration::from_secs(2));
            }
            line=lines.recv(),if stdout_open=>{
                let Some(line)=line else {stdout_open=false;continue;};
                let line=match line {Ok(line)=>line,Err(code)=>{failure=Some(code);break;}};
                stdout_bytes=stdout_bytes.saturating_add(line.len() as u64);
                let Ok(text)=std::str::from_utf8(&line)else{failure=Some("cli_stdout_invalid_utf8");break;};
                if text.len()>cli_io::MAX_RESULT_BYTES {failure=Some("cli_result_byte_budget_exceeded");break;}
                let next=parse_codex_jsonl(text);
                if let Some(session)=next.session_id {
                    if parsed.session_id.as_ref().is_some_and(|old|old!=&session){failure=Some("cli_session_changed");break;}
                    parsed.session_id=Some(session);
                }
                if let Some(model)=next.actual_model {
                    if parsed.actual_model.as_ref().is_some_and(|old|old!=&model){failure=Some("cli_model_changed");break;}
                    parsed.actual_model=Some(model);
                }
                parsed.tokens_in=parsed.tokens_in.max(next.tokens_in);parsed.tokens_out=parsed.tokens_out.max(next.tokens_out);
                parsed.context_window=next.context_window.or(parsed.context_window);
                parsed.current_context_tokens=next.current_context_tokens.or(parsed.current_context_tokens);
                if let Some(message)=next.messages.last() {
                    if message.len()>cli_io::MAX_RESULT_BYTES {failure=Some("cli_result_byte_budget_exceeded");break;}
                    parsed.messages.clear();parsed.messages.push(message.clone());
                }
                sequence=sequence.saturating_add(1);
                if let Some(observer)=&options.progress {
                    let terminal=serde_json::from_str::<Value>(text).ok().is_some_and(|v|v["type"]=="turn.completed");
                    let progress=crate::routing::ExecutionProgress{
                        engine:Engine::CodexCli,session_id:parsed.session_id.clone(),sequence,step_index:None,
                        phase:if terminal {crate::routing::ExecutionPhase::ResultObserved}else{crate::routing::ExecutionPhase::Running},
                        observed_model:parsed.actual_model.clone(),observed_at:chrono::Utc::now().to_rfc3339(),stdout_bytes,
                        usage:terminal.then(||crate::routing::ExecutionUsage{scope:crate::routing::UsageScope::CurrentTurn,input_tokens:parsed.tokens_in,output_tokens:parsed.tokens_out,thinking_tokens:0,cache_read_tokens:0,total_tokens:parsed.tokens_in.saturating_add(parsed.tokens_out),turns:Some(1)}),
                    };
                    let notify=observer(progress);tokio::pin!(notify);
                    let committed=tokio::select! {
                        value=tokio::time::timeout(Duration::from_secs(30),&mut notify)=>matches!(value,Ok(Ok(()))),
                        ()=deadline.wait()=>{timed_out=true;false},
                        ()=crate::supervisor::cancelled()=>{cancelled=true;false},
                    };
                    if !committed {failure=Some("cli_progress_commit_failed");break;}
                }
            }
        }
    }
    let known_closed = status.is_some() && natural_group_closed;
    let cleanup = known_closed || terminate_process_group(&mut child, group).await.is_ok();
    drop(lines);
    let (stdout_done, stderr, input_done) = tokio::join!(
        cli_io::finish_task(reader),
        cli_io::finish_task(errors),
        cli_io::finish_task(writer),
    );
    if stdout_done.is_none()
        || !matches!(stderr, Some(Ok(_)))
        || !matches!(input_done, Some(Ok(())))
    {
        failure.get_or_insert("cli_owned_io_unconfirmed");
    }
    let stderr = stderr.and_then(Result::ok).map(|stderr| stderr.bytes);
    receipt.owned_processes_closed = Some(
        known_closed || (cleanup && crate::verification::owned_process_group_closed(group).await),
    );
    receipt.duration_ms = millis(started.elapsed());
    if let Some(stderr) = stderr {
        receipt.auth_diagnostics = classify_stderr_auth(&String::from_utf8_lossy(&stderr));
    }
    let final_output = if let Some(path) = io
        .output_file
        .filter(|_| receipt.owned_processes_closed == Some(true))
    {
        let path = path.to_path_buf();
        match tokio::task::spawn_blocking(move || cli_io::read_result_file(&path)).await {
            Ok(Ok(Some(text))) if !text.trim().is_empty() => text.trim().to_owned(),
            Ok(Ok(_)) => parsed.messages.pop().unwrap_or_default(),
            _ => {
                failure.get_or_insert("cli_result_file_unavailable_or_oversized");
                String::new()
            }
        }
    } else {
        parsed.messages.pop().unwrap_or_default()
    };
    if cancelled {
        mark_cancelled(
            &mut receipt,
            "Codex CLI cancelled; inspect the original session before any new request",
        );
    } else if timed_out {
        mark_timeout(
            &mut receipt,
            "Codex CLI caller deadline elapsed; inspect the original session before any new request",
        );
    } else if let Some(code) = failure {
        mark_failure(&mut receipt, code);
    } else if status
        .as_ref()
        .is_some_and(std::process::ExitStatus::success)
    {
        let digest = digest_output(&final_output);
        let proof = if let Some(session) = parsed.session_id.clone() {
            let workspace = workspace.to_path_buf();
            tokio::task::spawn_blocking(move || {
                prove_codex_session_identity(&session, &workspace, &digest)
            })
            .await
            .ok()
            .flatten()
        } else {
            None
        };
        let (actual, source) = if let Some(actual) = parsed.actual_model.clone() {
            (Some(actual), Some("codex_jsonl_reported".into()))
        } else if let Some(proof) = &proof {
            (Some(proof.model.clone()), Some(proof.source.clone()))
        } else {
            (None, None)
        };
        finish_receipt(
            &mut receipt,
            route,
            actual,
            source,
            &final_output,
            parsed.tokens_in,
            parsed.tokens_out,
        );
        receipt.context_window = parsed
            .context_window
            .or_else(|| proof.as_ref().and_then(|p| p.context_window));
        receipt.current_context_tokens = parsed
            .current_context_tokens
            .or_else(|| proof.as_ref().and_then(|p| p.current_context_tokens));
        if !natural_group_closed {
            mark_timeout(
                &mut receipt,
                "CLI leader exited with active descendants; forced cleanup does not confirm job completion",
            );
        }
    } else {
        let diagnostics = receipt.auth_diagnostics.sanitized_stderr.clone();
        mark_failure(&mut receipt, &diagnostics);
    }
    Ok(ExecutionResult {
        structured_output: extract_json(&final_output),
        receipt,
        placeholder: None,
        output: final_output,
        session_id: parsed.session_id,
        exit_code: status.and_then(|s| s.code()),
        search_evidence: None,
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
    use std::io::{BufRead, Read};
    let mut source = std::io::BufReader::new(std::fs::File::open(artifact.path()).ok()?);
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
    loop {
        let mut line = Vec::new();
        let size = source
            .by_ref()
            .take(cli_io::MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .ok()?;
        if size == 0 {
            break;
        }
        if line.len() > cli_io::MAX_LINE_BYTES {
            return None;
        }
        let Ok(event) = serde_json::from_slice::<Value>(&line) else {
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
        search_evidence: None,
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
    readiness_marker: &Path,
) -> crate::Result<()> {
    let marker = async {
        loop {
            match tokio::fs::read(readiness_marker).await {
                Ok(bytes) if bytes == b"R" => return Ok(()),
                Ok(bytes) if bytes.is_empty() => {}
                Ok(_) => return Err("child readiness handshake returned an invalid marker".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::select! {
        result = marker => result,
        status = child.wait() => Err(format!("child exited before readiness handshake: {}", status?).into()),
    }
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
