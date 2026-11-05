//! Bounded native Codex web-search discovery.

use super::{
    CodexCapture, ExecutionOptions, ExecutionResult, append_codex_common, classify_stderr_auth,
    digest_output, enriched_command_path, finish_receipt, mark_cancelled, mark_failure,
    mark_timeout, millis, parse_codex_jsonl, prove_codex_session_identity, receipt,
    resolve_codex_binary, terminate_process_group, verify_input_budget,
};
use crate::routing::{Engine, ReceiptStatus, Route, RouteRequest, SandboxMode, validate_route};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use url::Url;

const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;
const MAX_JSONL_LINE_BYTES: usize = 64 * 1024;
const READ_CHUNK_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchPolicy {
    pub max_search_events: usize,
    pub max_candidates: usize,
}

impl Default for SearchPolicy {
    fn default() -> Self {
        Self {
            max_search_events: 3,
            max_candidates: 6,
        }
    }
}

impl SearchPolicy {
    fn validate(&self) -> crate::Result<()> {
        if !(1..=6).contains(&self.max_search_events) {
            return Err("max_search_events must be between 1 and 6".into());
        }
        if !(1..=12).contains(&self.max_candidates) {
            return Err("max_candidates must be between 1 and 12".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchStopReason {
    Completed,
    TimedOut,
    Cancelled,
    PolicyViolation,
    ProcessFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchEvidence {
    pub schema_version: String,
    pub observed_searches: usize,
    pub completed_searches: usize,
    pub observed_search_ids: Vec<String>,
    pub completed_search_ids: Vec<String>,
    pub matching_event_sha256: Vec<String>,
    pub rejected_candidate_count: usize,
    pub raw_output_sha256: Option<String>,
    pub stop_reason: SearchStopReason,
    pub violation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_workspace: Option<PathBuf>,
}

impl SearchEvidence {
    fn new() -> Self {
        Self {
            schema_version: "codex_web_search_evidence/v1".to_owned(),
            observed_searches: 0,
            completed_searches: 0,
            observed_search_ids: Vec::new(),
            completed_search_ids: Vec::new(),
            matching_event_sha256: Vec::new(),
            rejected_candidate_count: 0,
            raw_output_sha256: None,
            stop_reason: SearchStopReason::ProcessFailure,
            violation: None,
            retained_workspace: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveryOutput {
    candidate_urls: Vec<String>,
}

#[derive(Default)]
struct SearchObserver {
    observed: BTreeSet<String>,
    completed: BTreeSet<String>,
    hashes: Vec<String>,
    result_urls: BTreeSet<String>,
}

impl SearchObserver {
    fn observe(&mut self, line: &[u8], policy: &SearchPolicy) -> Result<(), String> {
        let line = std::str::from_utf8(line).map_err(|_| "invalid_utf8_jsonl".to_owned())?;
        let value: Value = serde_json::from_str(line).map_err(|_| "malformed_jsonl".to_owned())?;
        let Some(item) = value.get("item") else {
            return Ok(());
        };
        let item_type = item
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "item_type_missing".to_owned())?;
        if !matches!(
            item_type,
            "agent_message" | "message" | "reasoning" | "plan" | "web_search"
        ) {
            return Err("forbidden_item_type".to_owned());
        }
        if item_type != "web_search" {
            return Ok(());
        }
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "event_type_missing".to_owned())?;
        if !matches!(event_type, "item.started" | "item.completed") {
            return Err("unsupported_web_search_event".to_owned());
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or_else(|| "web_search_id_invalid".to_owned())?;
        self.observed.insert(id.to_owned());
        if self.observed.len() > policy.max_search_events {
            return Err("search_event_budget_exhausted".to_owned());
        }
        if event_type == "item.completed" {
            if item
                .get("action")
                .and_then(|action| action.get("type"))
                .and_then(Value::as_str)
                != Some("search")
            {
                return Err("completed_web_search_action_invalid".to_owned());
            }
            self.completed.insert(id.to_owned());
            if let Some(results) = item.get("results").and_then(Value::as_array) {
                for result in results {
                    if let Some(url) = result.get("url").and_then(Value::as_str)
                        && let Some(url) = sanitize_candidate(url)
                    {
                        self.result_urls.insert(url);
                    }
                }
            }
        }
        if self.hashes.len() < policy.max_search_events.saturating_mul(2) {
            self.hashes
                .push(format!("sha256:{:x}", Sha256::digest(line.as_bytes())));
        }
        Ok(())
    }

    fn finish(self) -> (SearchEvidence, BTreeSet<String>) {
        let observed_searches = self.observed.len();
        let completed_searches = self.completed.len();
        let evidence = SearchEvidence {
            observed_searches,
            completed_searches,
            observed_search_ids: self.observed.into_iter().collect(),
            completed_search_ids: self.completed.into_iter().collect(),
            matching_event_sha256: self.hashes,
            ..SearchEvidence::new()
        };
        (evidence, self.result_urls)
    }
}

struct ReadCapture {
    bytes: Vec<u8>,
    evidence: Option<SearchEvidence>,
    violation: Option<String>,
    result_urls: BTreeSet<String>,
}

enum OutputCapture {
    Missing,
    Text(String),
    Rejected,
}

pub async fn execute_codex_web_discovery(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
    output_schema: &Value,
    policy: SearchPolicy,
) -> crate::Result<ExecutionResult> {
    let binary = resolve_codex_binary();
    execute_codex_web_discovery_with_binary(
        request,
        limit,
        options,
        output_schema,
        policy,
        &binary,
        None,
    )
    .await
}

async fn execute_codex_web_discovery_with_binary(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
    output_schema: &Value,
    policy: SearchPolicy,
    binary: &Path,
    readiness_marker: Option<&Path>,
) -> crate::Result<ExecutionResult> {
    verify_input_budget(&request, options)?;
    policy.validate()?;
    if options.sandbox_mode != SandboxMode::ReadOnly {
        return Err("web discovery requires the read-only sandbox".into());
    }
    let route = validate_route(&request)?;
    if route.engine != Engine::CodexCli {
        return Err("web discovery requires the Codex CLI route".into());
    }
    let capture = CodexCapture::new(Some(output_schema))?;
    let workspace = capture.directory.join("workspace");
    std::fs::create_dir(&workspace)?;
    let workspace = workspace.canonicalize()?;
    let mut args = vec!["--search".to_owned(), "exec".to_owned()];
    append_codex_common(&mut args, &route, options, &workspace)?;
    args.extend([
        "-c".to_owned(),
        "features.shell_tool=false".to_owned(),
        "-c".to_owned(),
        "features.unified_exec=false".to_owned(),
        "-c".to_owned(),
        "features.multi_agent=false".to_owned(),
    ]);
    capture.append_arguments(&mut args);
    args.push("-".to_owned());
    let mut result = Box::pin(run_search_process(
        &route,
        &request,
        options,
        &workspace,
        &args,
        limit,
        binary,
        &capture.output_file,
        &policy,
        readiness_marker,
    ))
    .await?;
    if result.receipt.owned_processes_closed != Some(true) {
        if let Some(evidence) = result.search_evidence.as_mut() {
            evidence.retained_workspace = Some(capture.directory.clone());
        }
        std::mem::forget(capture);
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
async fn run_search_process(
    route: &Route,
    request: &RouteRequest,
    options: &ExecutionOptions,
    workspace: &Path,
    args: &[String],
    limit: Duration,
    binary: &Path,
    output_file: &Path,
    policy: &SearchPolicy,
    readiness_marker: Option<&Path>,
) -> crate::Result<ExecutionResult> {
    let mut receipt = receipt(route, options);
    let started = Instant::now();
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
        Err(_error) => {
            receipt.duration_ms = millis(started.elapsed());
            receipt.owned_processes_closed = Some(true);
            mark_failure(&mut receipt, "Codex web discovery process could not start");
            let mut evidence = SearchEvidence::new();
            evidence.violation = Some("process_spawn_failed".to_owned());
            return Ok(result_with_evidence(receipt, evidence, None));
        }
    };
    let process_group = child.id();
    let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(mut stdin), Some(stdout), Some(stderr)) = pipes else {
        let cleanup_failed = terminate_process_group(&mut child, process_group)
            .await
            .is_err();
        receipt.duration_ms = millis(started.elapsed());
        receipt.owned_processes_closed = Some(
            !cleanup_failed && crate::verification::owned_process_group_closed(process_group).await,
        );
        mark_failure(
            &mut receipt,
            "Codex web discovery process pipes were unavailable",
        );
        let mut evidence = SearchEvidence::new();
        evidence.violation = Some("process_pipe_unavailable".to_owned());
        return Ok(result_with_evidence(receipt, evidence, None));
    };
    let prompt = request.prompt.clone();
    let stdin_task = tokio::spawn(async move {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await
    });
    let total = Arc::new(AtomicUsize::new(0));
    let (violation_tx, mut violation_rx) = mpsc::channel(1);
    let _violation_guard = violation_tx.clone();
    let stdout_task = tokio::spawn(read_stdout(
        stdout,
        Arc::clone(&total),
        violation_tx.clone(),
        policy.clone(),
    ));
    let stderr_task = tokio::spawn(read_stderr(stderr, total, violation_tx));
    enum WaitOutcome {
        Completed(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
        Violation(String),
    }
    let readiness_outcome = if let Some(marker) = readiness_marker {
        tokio::select! {
            () = crate::supervisor::cancelled() => Some(WaitOutcome::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(30), super::await_child_readiness(&mut child, marker)) => {
                (!matches!(result, Ok(Ok(())))).then(|| WaitOutcome::Violation("fixture_readiness_unconfirmed".to_owned()))
            }
        }
    } else {
        None
    };
    let wait_outcome = match readiness_outcome {
        Some(outcome) => outcome,
        None => tokio::select! {
            status = child.wait() => WaitOutcome::Completed(status),
            () = crate::execution_policy::wait_for_limit(limit) => WaitOutcome::TimedOut,
            () = crate::supervisor::cancelled() => WaitOutcome::Cancelled,
            violation = violation_rx.recv() => WaitOutcome::Violation(
                violation.unwrap_or_else(|| "stream_observer_closed".to_owned())
            ),
        },
    };
    let cleanup_failed = terminate_process_group(&mut child, process_group)
        .await
        .is_err();
    let (stdout, stderr, stdin) = tokio::join!(
        super::cli_io::finish_task(stdout_task),
        super::cli_io::finish_task(stderr_task),
        super::cli_io::finish_task(stdin_task),
    );
    let input_failed = !matches!(stdin, Some(Ok(()))) || stdout.is_none() || stderr.is_none();
    let stdout_capture = stdout.and_then(Result::ok);
    let stderr_capture = stderr.and_then(Result::ok);
    let stdout_capture = stdout_capture.unwrap_or_else(empty_stdout_capture);
    let stderr_capture = stderr_capture.unwrap_or_else(empty_stderr_capture);
    let mut evidence = stdout_capture.evidence.unwrap_or_else(SearchEvidence::new);
    let stream_violation = match wait_outcome {
        WaitOutcome::Violation(ref violation) => Some(violation.clone()),
        _ => stdout_capture.violation.or(stderr_capture.violation),
    };
    receipt.duration_ms = millis(started.elapsed());
    let mut group_closed = crate::verification::owned_process_group_closed(process_group).await;
    if !group_closed {
        let _cleanup = terminate_process_group(&mut child, process_group).await;
        group_closed = crate::verification::owned_process_group_closed(process_group).await;
    }
    receipt.owned_processes_closed = Some(group_closed);
    let stdout = String::from_utf8_lossy(&stdout_capture.bytes).into_owned();
    let mut auth_diagnostics =
        classify_stderr_auth(&String::from_utf8_lossy(&stderr_capture.bytes));
    auth_diagnostics.sanitized_stderr.clear();
    receipt.auth_diagnostics = auth_diagnostics;
    let parsed = parse_codex_jsonl(&stdout);
    let raw_output = match read_bounded_output(output_file) {
        OutputCapture::Text(text) if !text.trim().is_empty() => text.trim().to_owned(),
        OutputCapture::Rejected => {
            evidence.stop_reason = SearchStopReason::PolicyViolation;
            evidence.violation = Some("output_file_limit_exceeded".to_owned());
            mark_failure(
                &mut receipt,
                "Codex web discovery output exceeded its byte limit",
            );
            return Ok(result_with_evidence(receipt, evidence, None));
        }
        OutputCapture::Text(_) | OutputCapture::Missing => parsed
            .messages
            .last()
            .cloned()
            .unwrap_or_else(|| stdout.trim().to_owned()),
    };
    evidence.raw_output_sha256 = Some(digest_output(&raw_output));

    if cleanup_failed || input_failed {
        evidence.stop_reason = SearchStopReason::ProcessFailure;
        evidence.violation = Some("owned_process_io_failed".to_owned());
        mark_failure(&mut receipt, "Codex web discovery process I/O failed");
        return Ok(result_with_evidence(receipt, evidence, None));
    }

    let status = match wait_outcome {
        WaitOutcome::Completed(Ok(status)) => Some(status),
        WaitOutcome::Completed(Err(error)) => {
            evidence.stop_reason = SearchStopReason::ProcessFailure;
            let _wait_error = error;
            mark_failure(&mut receipt, "Codex web discovery process wait failed");
            return Ok(result_with_evidence(receipt, evidence, None));
        }
        WaitOutcome::TimedOut => {
            evidence.stop_reason = SearchStopReason::TimedOut;
            mark_timeout(
                &mut receipt,
                "Codex web discovery timed out after dispatch; inspect the original request before retrying",
            );
            return Ok(result_with_evidence(receipt, evidence, None));
        }
        WaitOutcome::Cancelled => {
            evidence.stop_reason = SearchStopReason::Cancelled;
            mark_cancelled(&mut receipt, "Codex web discovery cancelled by caller");
            return Ok(result_with_evidence(receipt, evidence, None));
        }
        WaitOutcome::Violation(_) => None,
    };
    if let Some(violation) = stream_violation {
        evidence.stop_reason = SearchStopReason::PolicyViolation;
        evidence.violation = Some(violation.clone());
        mark_failure(
            &mut receipt,
            &format!("web discovery policy violation: {violation}"),
        );
        return Ok(result_with_evidence(receipt, evidence, None));
    }
    let Some(status) = status else {
        evidence.stop_reason = SearchStopReason::PolicyViolation;
        evidence.violation = Some("stream_observer_closed".to_owned());
        mark_failure(&mut receipt, "web discovery stream observer closed");
        return Ok(result_with_evidence(receipt, evidence, None));
    };
    if !status.success() {
        evidence.stop_reason = SearchStopReason::ProcessFailure;
        mark_failure(&mut receipt, "Codex web discovery process failed");
        return Ok(result_with_evidence(receipt, evidence, status.code()));
    }
    if evidence.completed_search_ids.is_empty()
        || evidence.completed_search_ids != evidence.observed_search_ids
    {
        evidence.stop_reason = SearchStopReason::PolicyViolation;
        evidence.violation = Some("completed_search_evidence_missing".to_owned());
        mark_failure(
            &mut receipt,
            "web discovery did not prove completion of every observed search event",
        );
        return Ok(result_with_evidence(receipt, evidence, status.code()));
    }

    if prove_identity(&mut receipt, route, &parsed, workspace, &raw_output)
        .await
        .is_err()
    {
        evidence.stop_reason = SearchStopReason::ProcessFailure;
        evidence.violation = Some("model_identity_observation_failed".to_owned());
        mark_failure(
            &mut receipt,
            "Codex web discovery model identity observation failed",
        );
        return Ok(result_with_evidence(receipt, evidence, status.code()));
    }
    if receipt.status != ReceiptStatus::Completed {
        evidence.stop_reason = SearchStopReason::ProcessFailure;
        return Ok(result_with_evidence(receipt, evidence, status.code()));
    }
    let parsed_output: DiscoveryOutput = match serde_json::from_str(&raw_output) {
        Ok(output) => output,
        Err(_error) => {
            evidence.stop_reason = SearchStopReason::PolicyViolation;
            evidence.violation = Some("candidate_output_invalid".to_owned());
            mark_failure(
                &mut receipt,
                "Codex web discovery candidate output was invalid",
            );
            return Ok(result_with_evidence(receipt, evidence, status.code()));
        }
    };
    let (candidate_urls, rejected) = sanitize_candidates(
        parsed_output.candidate_urls,
        &stdout_capture.result_urls,
        policy,
    );
    evidence.rejected_candidate_count = rejected;
    if candidate_urls.is_empty() {
        evidence.stop_reason = SearchStopReason::PolicyViolation;
        evidence.violation = Some("no_safe_candidate_urls".to_owned());
        mark_failure(
            &mut receipt,
            "web discovery returned no safe HTTPS candidate URLs",
        );
        return Ok(result_with_evidence(receipt, evidence, status.code()));
    }
    let structured = serde_json::json!({"candidate_urls": candidate_urls});
    let output = match serde_json::to_string(&structured) {
        Ok(output) => output,
        Err(_error) => {
            evidence.stop_reason = SearchStopReason::ProcessFailure;
            evidence.violation = Some("candidate_serialization_failed".to_owned());
            mark_failure(
                &mut receipt,
                "Codex web discovery candidate serialization failed",
            );
            return Ok(result_with_evidence(receipt, evidence, status.code()));
        }
    };
    receipt.output_digest = Some(digest_output(&output));
    evidence.stop_reason = SearchStopReason::Completed;
    Ok(ExecutionResult {
        receipt,
        placeholder: None,
        output,
        structured_output: Some(structured),
        session_id: parsed.session_id,
        exit_code: status.code(),
        search_evidence: Some(evidence),
    })
}

async fn prove_identity(
    receipt: &mut crate::routing::Receipt,
    route: &Route,
    parsed: &super::CodexJsonl,
    workspace: &Path,
    raw_output: &str,
) -> crate::Result<()> {
    let raw_digest = digest_output(raw_output);
    let session_proof = if let Some(session_id) = parsed.session_id.clone() {
        let workspace = workspace.to_path_buf();
        tokio::task::spawn_blocking(move || {
            prove_codex_session_identity(&session_id, &workspace, &raw_digest)
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
    finish_receipt(
        receipt,
        route,
        actual_model,
        identity_source,
        raw_output,
        parsed.tokens_in,
        parsed.tokens_out,
    );
    receipt.context_window = parsed.context_window.or_else(|| {
        session_proof
            .as_ref()
            .and_then(|proof| proof.context_window)
    });
    receipt.current_context_tokens = parsed.current_context_tokens.or_else(|| {
        session_proof
            .as_ref()
            .and_then(|proof| proof.current_context_tokens)
    });
    Ok(())
}

fn result_with_evidence(
    receipt: crate::routing::Receipt,
    evidence: SearchEvidence,
    exit_code: Option<i32>,
) -> ExecutionResult {
    ExecutionResult {
        receipt,
        placeholder: None,
        output: String::new(),
        structured_output: None,
        session_id: None,
        exit_code,
        search_evidence: Some(evidence),
    }
}

async fn read_stdout<R>(
    reader: R,
    total: Arc<AtomicUsize>,
    violations: mpsc::Sender<String>,
    policy: SearchPolicy,
) -> crate::Result<ReadCapture>
where
    R: AsyncRead + Unpin,
{
    let mut observer = SearchObserver::default();
    let mut bytes = Vec::new();
    let mut pending = Vec::new();
    let violation = read_bounded(reader, &total, &mut bytes, |chunk| {
        pending.extend_from_slice(chunk);
        while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
            if index > MAX_JSONL_LINE_BYTES {
                return Some("jsonl_line_limit_exceeded".to_owned());
            }
            let line = pending.drain(..=index).collect::<Vec<_>>();
            let line = &line[..line.len().saturating_sub(1)];
            if !line.is_empty()
                && let Err(error) = observer.observe(line, &policy)
            {
                return Some(error);
            }
        }
        if pending.len() > MAX_JSONL_LINE_BYTES {
            return Some("jsonl_line_limit_exceeded".to_owned());
        }
        None
    })
    .await?;
    if violation.is_none()
        && !pending.is_empty()
        && let Err(error) = observer.observe(&pending, &policy)
    {
        let _sent = violations.try_send(error.clone());
        let (evidence, result_urls) = observer.finish();
        return Ok(ReadCapture {
            bytes,
            evidence: Some(evidence),
            violation: Some(error),
            result_urls,
        });
    }
    if let Some(error) = &violation {
        let _sent = violations.try_send(error.clone());
    }
    let (evidence, result_urls) = observer.finish();
    Ok(ReadCapture {
        bytes,
        evidence: Some(evidence),
        violation,
        result_urls,
    })
}

async fn read_stderr<R>(
    reader: R,
    total: Arc<AtomicUsize>,
    violations: mpsc::Sender<String>,
) -> crate::Result<ReadCapture>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let violation = read_bounded(reader, &total, &mut bytes, |_| None).await?;
    if let Some(error) = &violation {
        let _sent = violations.try_send(error.clone());
    }
    Ok(ReadCapture {
        bytes,
        evidence: None,
        violation,
        result_urls: BTreeSet::new(),
    })
}

async fn read_bounded<R, F>(
    mut reader: R,
    total: &AtomicUsize,
    bytes: &mut Vec<u8>,
    mut observe: F,
) -> crate::Result<Option<String>>
where
    R: AsyncRead + Unpin,
    F: FnMut(&[u8]) -> Option<String>,
{
    let mut chunk = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        let prior = total.fetch_add(read, Ordering::AcqRel);
        if prior.saturating_add(read) > MAX_STREAM_BYTES {
            return Ok(Some("combined_stream_limit_exceeded".to_owned()));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(violation) = observe(&chunk[..read]) {
            return Ok(Some(violation));
        }
    }
}

fn sanitize_candidates(
    candidates: Vec<String>,
    observed_results: &BTreeSet<String>,
    policy: &SearchPolicy,
) -> (Vec<String>, usize) {
    let input_count = candidates.len();
    let mut accepted = Vec::new();
    let mut seen = BTreeSet::new();
    for candidate in candidates.into_iter().take(policy.max_candidates) {
        if let Some(candidate) = sanitize_candidate(&candidate)
            && observed_results.contains(&candidate)
            && seen.insert(candidate.clone())
        {
            accepted.push(candidate);
        }
    }
    let rejected = input_count.saturating_sub(accepted.len());
    (accepted, rejected)
}

fn empty_stdout_capture() -> ReadCapture {
    ReadCapture {
        bytes: Vec::new(),
        evidence: Some(SearchEvidence::new()),
        violation: Some("stdout_observer_failed".to_owned()),
        result_urls: BTreeSet::new(),
    }
}

fn empty_stderr_capture() -> ReadCapture {
    ReadCapture {
        bytes: Vec::new(),
        evidence: None,
        violation: Some("stderr_observer_failed".to_owned()),
        result_urls: BTreeSet::new(),
    }
}

fn read_bounded_output(path: &Path) -> OutputCapture {
    let Ok(file) = std::fs::File::open(path) else {
        return OutputCapture::Missing;
    };
    let mut bytes = Vec::new();
    let Ok(limit) = u64::try_from(MAX_STREAM_BYTES) else {
        return OutputCapture::Rejected;
    };
    if file
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .is_err()
    {
        return OutputCapture::Rejected;
    }
    if bytes.len() > MAX_STREAM_BYTES {
        return OutputCapture::Rejected;
    }
    match String::from_utf8(bytes) {
        Ok(text) => OutputCapture::Text(text),
        Err(_) => OutputCapture::Rejected,
    }
}

fn sanitize_candidate(candidate: &str) -> Option<String> {
    if candidate.len() > 2_048 {
        return None;
    }
    let mut url = Url::parse(candidate).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query_pairs().any(|(key, _)| sensitive_query_key(&key))
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn sensitive_query_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("x-amz-")
        || key.starts_with("x-goog-")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("signature")
        || key.contains("auth")
        || matches!(
            key.as_str(),
            "token"
                | "access_token"
                | "auth"
                | "api_key"
                | "api-key"
                | "apikey"
                | "password"
                | "secret"
                | "client_secret"
                | "authorization"
                | "signature"
                | "sig"
                | "x-amz-credential"
                | "x-amz-security-token"
                | "x-amz-signature"
                | "x-goog-credential"
                | "x-goog-signature"
        )
}

#[cfg(test)]
mod tests;
