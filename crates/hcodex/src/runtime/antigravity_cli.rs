//! Artifact-only Antigravity CLI execution with durable init-before-prompt ordering.
mod binary_policy;

pub(super) async fn verified_readonly_binary(path: &Path) -> bool {
    binary_policy::verified(path)
        .await
        .is_some_and(|identity| identity.still_matches())
}
use super::cli_io::{self, MAX_RESULT_BYTES};
use super::{
    CodexCapture, ExecutionOptions, ExecutionResult, enriched_command_path, extract_json,
    finish_receipt, mark_cancelled, mark_failure, mark_timeout, millis, receipt,
    terminate_process_group,
};
use crate::routing::{
    Engine, ExecutionPhase, ExecutionProgress, ExecutionUsage, RouteRequest, SandboxMode,
    UsageScope, validate_route,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

const STARTUP_LIMIT: Duration = Duration::from_secs(30);
const EXIT_GRACE: Duration = Duration::from_secs(10);
const PIPE_DRAIN_LIMIT: Duration = Duration::from_secs(2);
pub fn resolve_antigravity_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AGY_CLI_BIN").filter(|value| !value.is_empty()) {
        return regular_binary(Path::new(&path));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join("agy");
            if let Some(path) = regular_binary(&candidate) {
                return Some(path);
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let owned = PathBuf::from(home).join(".local/libexec/hcodex/antigravity");
        return regular_binary(&owned);
    }
    None
}

fn regular_binary(path: &Path) -> Option<PathBuf> {
    let resolved = path.canonicalize().ok()?;
    resolved.is_file().then_some(resolved)
}

pub async fn execute_antigravity_with_options(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
    schema: Option<&Value>,
) -> crate::Result<ExecutionResult> {
    let Some(binary) = resolve_antigravity_binary() else {
        let route = validate_route(&request)?;
        let mut receipt = receipt(&route, options);
        super::mark_failure_code(
            &mut receipt,
            "Antigravity CLI executable not found or not a regular file",
            crate::routing::RouteErrorCode::DependencyUnavailable,
        );
        receipt.owned_processes_closed = Some(true);
        return Ok(super::empty_result(receipt));
    };
    execute_antigravity_with_binary_and_options(request, limit, &binary, options, schema).await
}

pub async fn execute_antigravity_with_binary_and_options(
    mut request: RouteRequest,
    limit: Duration,
    binary: &Path,
    options: &ExecutionOptions,
    schema: Option<&Value>,
) -> crate::Result<ExecutionResult> {
    let route = validate_route(&request)?;
    if route.engine != Engine::AntigravityCli {
        return Err("route is not Antigravity CLI".into());
    }
    if options.sandbox_mode != SandboxMode::ReadOnly {
        return Err("artifact CLI execution requires read-only admission".into());
    }
    if let Some(schema) = schema {
        // Native --json-schema introduces a finish tool step. This adapter
        // requires a tool-free response and the gateway validates the JSON.
        request.prompt.push_str(
            "\n\nRespond with only one JSON object matching this schema. Return it as the final text without invoking tools.\n",
        );
        request.prompt.push_str(&serde_json::to_string(schema)?);
    }
    super::verify_input_budget(&request, options)?;
    let capture = CodexCapture::new(None)?;
    let workspace = capture.directory.join("workspace");
    std::fs::create_dir(&workspace)?;
    let workspace = workspace.canonicalize()?;
    let policy = json!({"toolPermission":"strict","allowNonWorkspaceAccess":false,"permissions":{"allow":[],"ask":[],"deny":["read_file(*)","write_file(*)","read_url(*)","execute_url(*)","command(*)","unsandboxed(*)","mcp(*)"]},"statusLine":{"enabled":false},"title":{"enabled":false},"useG1Credits":false});
    let args = vec![
        "--mode".into(),
        "plan".into(),
        "--sandbox".into(),
        "--model".into(),
        route.resolved_model.clone(),
        "--effort".into(),
        route.effort.clone(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--print-timeout".into(),
        "0".into(),
        "--disable-slash-commands".into(),
        "--settings".into(),
        serde_json::to_string(&policy)?,
        "--log-file".into(),
        "/dev/null".into(),
    ];
    let resolved_binary =
        regular_binary(binary).ok_or("Antigravity CLI path is not a regular file")?;
    let verified_binary = binary_policy::verified(&resolved_binary).await;
    let mut result = Box::pin(run(
        &route,
        &request,
        &resolved_binary,
        &args,
        &workspace,
        limit,
        options,
        verified_binary,
    ))
    .await?;
    if result.receipt.owned_processes_closed != Some(true) {
        result.receipt.failure_reason = Some(format!(
            "Owned CLI closure unproved; retained execution directory {}",
            capture.directory.display()
        ));
        std::mem::forget(capture);
    }
    Ok(result)
}

#[derive(Default)]
struct AgyState {
    session: Option<String>,
    model: Option<String>,
    result: Option<Value>,
    sequence: u64,
    step: Option<u64>,
    stdout_bytes: u64,
    initialized: bool,
}
impl AgyState {
    fn observe(
        &mut self,
        line: &[u8],
        workspace: &Path,
        verified_native_policy: bool,
    ) -> Result<ExecutionPhase, &'static str> {
        self.stdout_bytes = self.stdout_bytes.saturating_add(line.len() as u64);
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("event_counter_overflow")?;
        if line.is_empty() {
            return Err("empty_cli_event");
        }
        let value: Value = serde_json::from_slice(line).map_err(|_| "malformed_cli_json")?;
        match value["event"].as_str() {
            Some("init") => {
                if self.initialized {
                    return Err("duplicate_cli_init");
                }
                let id = value["conversation_id"]
                    .as_str()
                    .filter(|id| valid_id(id))
                    .ok_or("missing_cli_session")?
                    .to_owned();
                self.session = Some(id);
                self.initialized = true;
                self.model = value
                    .pointer("/init/model")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let info = value.get("init").ok_or("missing_cli_init")?;
                let expected = workspace.to_str().ok_or("execution_path_not_utf8")?;
                if info["cwd"] != expected {
                    return Err("cli_execution_workspace_mismatch");
                }
                if info["permission_mode"] != "strict" {
                    return Err("cli_strict_policy_unconfirmed");
                }
                let tools = info["tools"]
                    .as_array()
                    .ok_or("cli_tool_policy_unconfirmed")?;
                // Native init lists capabilities rather than effective permissions.
                // Non-empty inventories are only admitted on the pinned, tested
                // binary under this adapter's exact sandbox and strict deny settings.
                if !tools.is_empty() && !verified_native_policy {
                    return Err("cli_tool_policy_unconfirmed");
                }
                Ok(ExecutionPhase::AwaitingInput)
            }
            Some("step_update") => {
                if !self.initialized || self.result.is_some() {
                    return Err("cli_event_order_invalid");
                }
                let step = &value["step_update"];
                self.require_session(step["conversation_id"].as_str())?;
                if step.get("tool_name").is_some()
                    || step.get("tool_info").is_some()
                    || step.get("subagent_info").is_some()
                {
                    return Err("cli_forbidden_tool_or_subagent");
                }
                if !matches!(
                    step["step_type"].as_str(),
                    Some("user_input" | "agent_response" | "thinking" | "plan" | "checkpoint")
                ) {
                    return Err("cli_step_type_unsupported");
                }
                let index = step["step_index"]
                    .as_u64()
                    .ok_or("cli_step_index_missing")?;
                if self.step.is_some_and(|old| index < old) {
                    return Err("cli_step_index_regressed");
                }
                self.step = Some(index);
                match step["state"].as_str() {
                    Some("ACTIVE" | "DONE" | "PENDING") => {}
                    Some("ERROR") => return Err("cli_step_error"),
                    _ => return Err("cli_step_state_unsupported"),
                }
                Ok(ExecutionPhase::Running)
            }
            Some("result") => {
                if !self.initialized || self.result.is_some() {
                    return Err("cli_result_order_invalid");
                }
                let result = value.get("result").ok_or("missing_cli_result")?;
                self.require_session(result["conversation_id"].as_str())?;
                if result["response"]
                    .as_str()
                    .is_some_and(|text| text.len() > MAX_RESULT_BYTES)
                {
                    return Err("cli_result_byte_budget_exceeded");
                }
                if serde_json::to_vec(result)
                    .map_err(|_| "cli_result_encoding_failed")?
                    .len()
                    > MAX_RESULT_BYTES
                {
                    return Err("cli_result_byte_budget_exceeded");
                }
                if !matches!(
                    result["status"].as_str(),
                    Some("SUCCESS" | "ERROR" | "CANCELLED")
                ) {
                    return Err("cli_terminal_status_unsupported");
                }
                self.result = Some(result.clone());
                Ok(ExecutionPhase::ResultObserved)
            }
            _ => Err("cli_event_type_unsupported"),
        }
    }
    fn require_session(&self, id: Option<&str>) -> Result<(), &'static str> {
        if id == self.session.as_deref() {
            Ok(())
        } else {
            Err("cli_session_changed")
        }
    }
    fn progress(&self, phase: ExecutionPhase) -> ExecutionProgress {
        ExecutionProgress {
            engine: Engine::AntigravityCli,
            session_id: self.session.clone(),
            sequence: self.sequence,
            step_index: self.step,
            phase,
            observed_model: self.model.clone(),
            observed_at: chrono::Utc::now().to_rfc3339(),
            stdout_bytes: self.stdout_bytes,
            usage: self.result.as_ref().and_then(result_usage),
        }
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn result_usage(result: &Value) -> Option<ExecutionUsage> {
    let usage = result.get("usage")?;
    Some(ExecutionUsage {
        scope: UsageScope::SessionCumulative,
        input_tokens: usage["input_tokens"].as_u64()?,
        output_tokens: usage["output_tokens"].as_u64()?,
        thinking_tokens: usage["thinking_tokens"].as_u64().unwrap_or(0),
        cache_read_tokens: usage["cache_read_tokens"].as_u64().unwrap_or(0),
        total_tokens: usage["total_tokens"].as_u64().unwrap_or(0),
        turns: result["num_turns"].as_u64(),
    })
}

async fn notify(
    options: &ExecutionOptions,
    state: &AgyState,
    phase: ExecutionPhase,
) -> crate::Result<()> {
    if let Some(observer) = &options.progress {
        observer(state.progress(phase)).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run(
    route: &crate::routing::Route,
    request: &RouteRequest,
    binary: &Path,
    args: &[String],
    workspace: &Path,
    limit: Duration,
    options: &ExecutionOptions,
    verified_binary: Option<binary_policy::VerifiedBinary>,
) -> crate::Result<ExecutionResult> {
    let started = Instant::now();
    let mut receipt = receipt(route, options);
    let mut state = AgyState::default();
    let mut command = Command::new(binary);
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
            mark_failure(&mut receipt, "Antigravity CLI process could not start");
            return Ok(super::empty_result(receipt));
        }
    };
    let group = child.id();
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _cleanup = terminate_process_group(&mut child, group).await;
        receipt.owned_processes_closed =
            Some(crate::verification::owned_process_group_closed(group).await);
        mark_failure(&mut receipt, "Antigravity CLI pipes unavailable");
        return Ok(super::empty_result(receipt));
    };
    let (sender, mut lines) = mpsc::channel(2);
    let reader = tokio::spawn(cli_io::read_lines(stdout, sender));
    let errors = tokio::spawn(cli_io::read_stderr(stderr));
    let deadline = crate::execution_policy::ExecutionDeadline::from_limit(limit);
    let startup = crate::execution_policy::ExecutionDeadline::from_limit(STARTUP_LIMIT);
    let mut stdin = Some(stdin);
    let mut drain_until = None;
    let mut input_started = false;
    let mut terminal = false;
    let mut process_status = None;
    let mut natural_group_closed = false;
    let mut failure: Option<&'static str> = None;
    let mut timed_out = false;
    let mut cancelled = false;
    loop {
        tokio::select! {
            ()=crate::supervisor::cancelled()=>{cancelled=true;break;}
            ()=deadline.wait()=>{timed_out=true;break;}
            ()=startup.wait(),if !input_started=>{failure=Some("cli_startup_or_init_commit_timeout");break;}
            ()=async {match drain_until {Some(at)=>tokio::time::sleep_until(at).await,None=>std::future::pending::<()>().await}}=>{failure=Some("cli_terminal_drain_timeout");break;}
            status=child.wait(),if process_status.is_none()=>{
                match status {Ok(status)=>process_status=Some(status),Err(_)=>{failure=Some("cli_wait_failed");break;}}
                natural_group_closed=crate::verification::owned_process_group_closed(group).await;
                if !natural_group_closed {let _cleanup=terminate_process_group(&mut child,group).await;}
                drain_until=Some(tokio::time::Instant::now()+PIPE_DRAIN_LIMIT);
            }
            line=lines.recv()=>{
                let Some(line)=line else {break;};
                let line=match line {Ok(line)=>line,Err(code)=>{failure=Some(code);break;}};
                let verified_native_policy = !state.initialized && verified_binary.as_ref().is_some_and(binary_policy::VerifiedBinary::still_matches);
                let observed=state.observe(&line,workspace,verified_native_policy);
                // Even a rejected init retains its observed handle before no-prompt teardown.
                let phase=observed.as_ref().copied().unwrap_or(ExecutionPhase::Stopped);
                let committed=tokio::select! {
                    value=tokio::time::timeout(STARTUP_LIMIT,notify(options,&state,phase))=>matches!(value,Ok(Ok(()))),
                    ()=crate::supervisor::cancelled()=>{cancelled=true;false},
                    ()=startup.wait(),if !input_started=>false,
                    ()=deadline.wait()=>{timed_out=true;false},
                };
                if !committed {failure=Some("cli_progress_commit_failed");break;}
                if let Err(code)=observed {failure=Some(code);break;}
                if state.model.as_deref()!=Some(route.resolved_model.as_str()){failure=Some("cli_selected_model_mismatch");break;}
                if !input_started {
                    if process_status.is_some(){failure=Some("cli_exited_before_input");break;}
                    if crate::supervisor::is_cancelled(){cancelled=true;break;}
                    if deadline.is_elapsed(){timed_out=true;break;}
                    if startup.is_elapsed(){failure=Some("cli_startup_or_init_commit_timeout");break;}
                    let input=match serde_json::to_vec(&json!({"event":"user","message":{"content":request.prompt}})){Ok(mut bytes)=>{bytes.push(b'\n');bytes},Err(_)=>{failure=Some("cli_prompt_encoding_failed");break;}};
                    let Some(pipe)=stdin.as_mut() else {failure=Some("cli_prompt_pipe_missing");break;};
                    input_started=true;
                    let write=pipe.write_all(&input);tokio::pin!(write);
                    let sent=tokio::select! {
                        biased;
                        ()=crate::supervisor::cancelled()=>{cancelled=true;false},
                        ()=deadline.wait()=>{timed_out=true;false},
                        value=tokio::time::timeout(STARTUP_LIMIT,&mut write)=>matches!(value,Ok(Ok(()))),
                    };
                    if !sent {failure=Some("cli_prompt_pipe_failed");break;}
                    state.sequence=state.sequence.saturating_add(1);
                    let committed=tokio::select! {
                        value=tokio::time::timeout(STARTUP_LIMIT,notify(options,&state,ExecutionPhase::InputSent))=>matches!(value,Ok(Ok(()))),
                        ()=crate::supervisor::cancelled()=>{cancelled=true;false},
                        ()=deadline.wait()=>{timed_out=true;false},
                    };
                    if !committed {failure=Some("cli_input_progress_commit_failed");break;}
                }
                if state.result.is_some(){terminal=true;drop(stdin.take());drain_until=Some(tokio::time::Instant::now()+EXIT_GRACE);}
            }
        }
    }
    // A terminal event ends input; process exit/pipe closure is separately bounded.
    if process_status.is_none() && failure.is_none() && !timed_out && !cancelled {
        match tokio::time::timeout(EXIT_GRACE, child.wait()).await {
            Ok(Ok(status)) => {
                process_status = Some(status);
                natural_group_closed = crate::verification::owned_process_group_closed(group).await;
            }
            _ => failure = Some("cli_terminal_process_did_not_exit"),
        }
    }
    drop(stdin);
    let cleanup = natural_group_closed || terminate_process_group(&mut child, group).await.is_ok();
    drop(lines);
    let (stdout_done, stderr) =
        tokio::join!(cli_io::finish_task(reader), cli_io::finish_task(errors));
    if stdout_done.is_none() || !matches!(stderr, Some(Ok(_))) {
        failure.get_or_insert("cli_pipe_drain_unconfirmed");
    }
    let stderr = stderr.and_then(Result::ok);
    receipt.owned_processes_closed = Some(
        natural_group_closed
            || (cleanup && crate::verification::owned_process_group_closed(group).await),
    );
    receipt.duration_ms = millis(started.elapsed());
    if let Some(stderr) = stderr {
        receipt.auth_diagnostics =
            super::classify_stderr_auth(&String::from_utf8_lossy(&stderr.bytes));
        if stderr.permission_denied {
            failure.get_or_insert("cli_permission_denied");
        }
    }
    let mut output = String::new();
    let mut structured = None;
    if cancelled {
        mark_cancelled(
            &mut receipt,
            "Antigravity CLI cancelled; inspect the saved conversation before any new request",
        );
    } else if timed_out {
        mark_timeout(
            &mut receipt,
            "Antigravity CLI caller deadline elapsed; inspect the saved conversation before any new request",
        );
    } else if input_started
        && (!terminal || !natural_group_closed)
        && failure != Some("cli_permission_denied")
    {
        mark_timeout(
            &mut receipt,
            failure.unwrap_or("cli_terminal_result_or_natural_closure_unproved"),
        );
    } else if let Some(code) = failure {
        mark_failure(&mut receipt, code);
    } else if !terminal {
        mark_failure(&mut receipt, "cli_terminal_result_not_observed");
    } else if receipt.owned_processes_closed != Some(true) {
        mark_timeout(
            &mut receipt,
            "Antigravity CLI process-group closure is unproved",
        );
    } else if !process_status
        .as_ref()
        .is_some_and(std::process::ExitStatus::success)
    {
        mark_failure(
            &mut receipt,
            "Antigravity CLI process did not exit successfully",
        );
    } else if let Some(result) = state.result.as_ref() {
        if result["status"] != "SUCCESS" {
            mark_failure(
                &mut receipt,
                "Antigravity CLI returned a non-success terminal result",
            );
        } else if result["num_turns"].as_u64() != Some(1) {
            mark_failure(&mut receipt, "cli_fresh_job_turn_count_unproved");
        } else {
            output = result["response"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_owned();
            structured = result
                .get("structured_output")
                .cloned()
                .or_else(|| extract_json(&output));
            let usage = result_usage(result);
            finish_receipt(
                &mut receipt,
                route,
                state.model.clone(),
                Some("antigravity_cli_init_configuration".into()),
                &output,
                usage.as_ref().map_or(0, |u| u.input_tokens),
                usage.as_ref().map_or(0, |u| u.output_tokens),
            );
        }
    }
    Ok(ExecutionResult {
        receipt,
        output,
        structured_output: structured,
        placeholder: None,
        session_id: state.session,
        exit_code: process_status.and_then(|s| s.code()),
        search_evidence: None,
    })
}

#[cfg(all(test, unix))]
mod tests;
