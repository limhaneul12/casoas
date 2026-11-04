//! Read-only organization workbench over existing durable jobs and reviews.

mod recovery;
pub use recovery::recovery;

use std::collections::BTreeSet;
use std::fs::Metadata;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::gateway::Gateway;
use crate::storage::{JobRecord, JobStatus};
use crate::triad::{load_source, recompute};

const MAX_BOARD_ROWS: usize = 100;
const MAX_SCOPED_FILES: usize = 64;
const MAX_SCOPED_BYTES: u64 = 8 * 1024 * 1024;

/// Owned per-role fields read from a council snapshot; the only input a ballot needs.
struct BallotCandidate {
    role: String,
    job_id: Option<String>,
    stored_model: Option<String>,
    verdict: Value,
}

pub async fn board(gateway: &Gateway, workspace: &Path, limit: usize) -> crate::Result<Value> {
    let workspace = workspace.canonicalize()?;
    let limit = limit.clamp(1, MAX_BOARD_ROWS);
    let query = workspace.to_string_lossy().into_owned();
    let query_limit = limit.saturating_mul(8).min(1_000);
    let rows = gateway
        .database(move |store| store.list_jobs(query_limit, Some(&query)))
        .await?;
    let items = rows
        .into_iter()
        .filter(|record| same_workspace(record, &workspace))
        .filter_map(compact_job)
        .take(limit)
        .collect::<Vec<_>>();
    let runtime_identity = crate::lifecycle::runtime_identity(&workspace).await;
    Ok(json!({
        "workspace_dir": workspace,
        "count": items.len(),
        "items": items,
        "read_only": true,
        "runtime_identity": runtime_identity,
    }))
}

pub async fn votes(
    gateway: &Gateway,
    workspace: &Path,
    collaboration_id: &str,
) -> crate::Result<Value> {
    let workspace = workspace.canonicalize()?;
    let source = load_source(gateway, collaboration_id, Some(&workspace), None)
        .await?
        .ok_or("review council source is unavailable or invalid")?;
    let adjudication = recompute(&source)?;
    let roles = source["roles"]
        .as_object()
        .ok_or("review council has no role snapshots")?;
    let candidates = roles
        .iter()
        .map(|(role, value)| BallotCandidate {
            role: role.clone(),
            job_id: value["job_id"].as_str().map(str::to_owned),
            stored_model: value["model_id"].as_str().map(str::to_owned),
            verdict: value["verdict"].clone(),
        })
        .collect::<Vec<_>>();
    let duplicate_receipt = has_duplicate_ids(
        candidates
            .iter()
            .filter_map(|candidate| candidate.job_id.as_deref()),
    );
    let workspace_for_jobs = workspace.clone();
    let ballots = gateway
        .database(move |store| {
            let mut ballots = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let job = match candidate.job_id.as_deref() {
                    Some(job_id) => store.get_job(job_id)?,
                    None => None,
                };
                let workspace_match = job.as_ref().is_some_and(|job| {
                    job.status == JobStatus::Completed && same_workspace(job, &workspace_for_jobs)
                });
                ballots.push(project_ballot(candidate, job.as_ref(), workspace_match));
            }
            Ok(ballots)
        })
        .await?;
    let ballots_valid = !duplicate_receipt
        && ballots.len() == 3
        && ballots.iter().all(|value| value["valid"] == true);
    let freshness_workspace = workspace.clone();
    let freshness_scope = source["evidence_scope"].clone();
    let source_status = tokio::task::spawn_blocking(move || {
        source_freshness(&freshness_workspace, &freshness_scope)
    })
    .await??;
    let historical_review_approved = adjudication.review_approved;
    let current_review_approved =
        historical_review_approved && ballots_valid && source_status["status"] == "current_scoped";
    Ok(json!({
        "collaboration_id": collaboration_id,
        "workspace_dir": workspace,
        "review_kind":"review_council",
        "ballots":ballots,
        "duplicate_role_job":duplicate_receipt,
        "adjudication":adjudication,
        "historical_review_approved":historical_review_approved,
        "source_status":source_status,
        "current_review_approved":current_review_approved,
    }))
}

/// Deterministic ballot projection and receipt validation for one council role.
///
/// Performs no I/O: the caller resolves the stored job and workspace membership.
/// Receipt precedence is presence-based (`job.receipt`, then `result.receipt`,
/// then `result.result.receipt`); malformed preferred data is never skipped.
fn project_ballot(
    candidate: BallotCandidate,
    job: Option<&JobRecord>,
    workspace_match: bool,
) -> Value {
    let BallotCandidate {
        role,
        job_id,
        stored_model,
        verdict,
    } = candidate;
    let Some(job_id) = job_id else {
        return json!({"role":role,"valid":false,"reason":"missing_role_job","verdict":verdict});
    };
    let Some(job) = job else {
        return json!({"role":role,"job_id":job_id,"valid":false,"reason":"role_job_missing","verdict":verdict});
    };
    let selected = job
        .receipt
        .as_ref()
        .or_else(|| job.result.as_ref().and_then(receipt));
    let requested = selected
        .and_then(|value| value.get("requested_model"))
        .and_then(Value::as_str);
    let actual = selected
        .and_then(|value| value.get("actual_model"))
        .and_then(Value::as_str);
    let receipt_status = selected
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str);
    let model_mismatch = selected
        .and_then(|value| value.get("model_mismatch"))
        .and_then(Value::as_bool);
    let valid = job.status == JobStatus::Completed
        && workspace_match
        && requested.is_some_and(|model| actual == Some(model))
        && actual == stored_model.as_deref()
        && actual == job.model.as_deref()
        && matches!(receipt_status, Some("succeeded" | "completed"))
        && model_mismatch == Some(false);
    json!({
        "role":role,"job_id":job_id,"requested_model":requested,
        "actual_model":actual,"verdict":verdict,"valid":valid,
        "reason":if valid {"verified_role_receipt"} else {"invalid_role_receipt"},
    })
}

pub fn council_request(task: &str, workspace: &Path) -> Value {
    json!({
        "task_spec":task,
        "workspace_dir":workspace,
        "mode":"council",
        "argon_model":"gemini-4-argon-high",
        "claude_model":"claude-sonnet-5-5-medium",
        "codex_model":"gpt-6.1-sol",
        "codex_reasoning_effort":"high",
        "invoke_argon_rpc":true,
        "max_rounds":1,
        "run_verification_gates":false,
        "async_background":false,
    })
}

pub fn council_request_with_files(
    task: &str,
    workspace: &Path,
    files: &[PathBuf],
) -> crate::Result<Value> {
    let workspace = workspace.canonicalize()?;
    let mut inspected = BTreeSet::new();
    for file in files {
        if file.is_absolute()
            || file.as_os_str().is_empty()
            || file
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("council file must be a safe relative path".into());
        }
        let text = file
            .to_str()
            .ok_or("council file path must be valid UTF-8")?;
        let absolute = workspace.join(file);
        let metadata = std::fs::symlink_metadata(&absolute)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || absolute.canonicalize()? != absolute
        {
            return Err("council file must be a regular file inside the workspace".into());
        }
        inspected.insert(text.to_owned());
    }
    if inspected.len() > MAX_SCOPED_FILES {
        return Err("council file scope exceeds 64 files".into());
    }
    let mut request = council_request(task, &workspace);
    request["files_inspected"] = serde_json::to_value(inspected)?;
    Ok(request)
}

/// Parse `/council --file relative [--file relative ...] -- proposal`.
/// File paths with whitespace must be supplied by the native CLI API instead.
pub fn parse_council_request(input: &str, workspace: &Path) -> crate::Result<Value> {
    let body = input
        .trim()
        .strip_prefix("/council")
        .unwrap_or(input.trim())
        .trim();
    if !body.starts_with("--file") {
        if body.is_empty() {
            return Err("usage: /council [--file relative ... --] <proposal>".into());
        }
        return Ok(council_request(body, workspace));
    }
    let tokens = body.split_whitespace().collect::<Vec<_>>();
    let mut files = Vec::new();
    let mut cursor = 0;
    while cursor < tokens.len() && tokens[cursor] == "--file" {
        let file = tokens
            .get(cursor + 1)
            .ok_or("--file requires a relative path")?;
        if *file == "--" || file.starts_with("--file") {
            return Err("--file requires a relative path".into());
        }
        files.push(PathBuf::from(file));
        cursor += 2;
    }
    if tokens.get(cursor) != Some(&"--") {
        return Err("file-scoped council requires `--` before the proposal; whitespace in file paths is unsupported in TUI commands".into());
    }
    let proposal = tokens[cursor + 1..].join(" ");
    if proposal.is_empty() {
        return Err("council proposal is empty".into());
    }
    council_request_with_files(&proposal, workspace, &files)
}

fn compact_job(record: JobRecord) -> Option<Value> {
    let result = record.result.as_ref();
    let detail = match record.task_type.as_str() {
        "dag_checkpoint" => json!({
            "kind":record.metadata.get("checkpoint_kind"),
            "node_id":record.metadata.get("node_id"),
            "handoff_status":if result.and_then(|value| value.get("handoff")).is_some() {"available"}
                else if result.and_then(|value| value.get("resumed_handoff")).is_some() {"referenced"}
                else {"unavailable_legacy"},
            "handoff":result.and_then(|value| value.get("handoff")).map(|handoff| json!({
                "content_digest":handoff["content_digest"],
                "summary":handoff["agent_report"]["summary"],
                "decisions":handoff["agent_report"]["decisions"],
                "caveats":handoff["agent_report"]["caveats"],
                "parents":handoff["parents"],
            })),
        }),
        "dag_ulw" => json!({
            "kind":"campaign",
            "settled_count":result.map(|value| value["settled_count"].clone()),
            "worktrees":result.map(|value| value["worktrees"]["status"].clone()),
        }),
        "triad_orchestration" => json!({
            "kind":"review_council",
            "collaboration_status":result.map(|value| value["adjudication"]["collaboration_status"].clone()),
            "consensus_status":result.map(|value| value["adjudication"]["consensus_status"].clone()),
            "review_approved":result.map(|value| value["adjudication"]["review_approved"].clone()),
        }),
        _ => return None,
    };
    Some(json!({
        "job_id":record.job_id,"task_type":record.task_type,"status":record.status,
        "outcome_state":record.outcome_state,"created_at":record.created_at,
        "completed_at":record.completed_at,"detail":detail,
    }))
}

fn source_freshness(workspace: &Path, scope: &Value) -> crate::Result<Value> {
    let Some(scope_workspace) = scope["workspace_dir"].as_str() else {
        return Ok(json!({"status":"unknown","reason":"missing_scope_workspace"}));
    };
    if PathBuf::from(scope_workspace).canonicalize()? != workspace {
        return Err("review source scope belongs to another workspace".into());
    }
    let Some(files) = scope["files_inspected_digests"].as_object() else {
        return Ok(json!({"status":"unknown","reason":"missing_file_scope"}));
    };
    if files.is_empty() {
        return Ok(json!({"status":"unknown","reason":"empty_file_scope"}));
    }
    if files.len() > MAX_SCOPED_FILES {
        return Ok(json!({"status":"unknown","reason":"file_scope_limit_exceeded"}));
    }
    if scope["unverified_or_missing_files"]
        .as_array()
        .is_some_and(|values| !values.is_empty())
    {
        return Ok(json!({"status":"unknown","reason":"source_scope_was_incomplete"}));
    }
    let mut total = 0_u64;
    let mut stale = Vec::new();
    for (relative, expected) in files {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("review source path escapes workspace".into());
        }
        let file = workspace.join(relative_path);
        let expected_path = file.clone();
        let metadata = match std::fs::symlink_metadata(&file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                stale.push(relative.clone());
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let canonical = file.canonicalize()?;
        if canonical != expected_path {
            return Err("review source path traverses a symlink".into());
        }
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("review source is not a regular confined file".into());
        }
        let Some(expected) = expected.as_str().filter(|value| valid_sha256(value)) else {
            return Ok(json!({"status":"unknown","reason":"malformed_source_digest"}));
        };
        let remaining = MAX_SCOPED_BYTES.saturating_sub(total);
        let mut handle = std::fs::File::open(&file)?;
        let opened = handle.metadata()?;
        if !same_file_snapshot(&metadata, &opened) {
            return Ok(json!({"status":"unknown","reason":"source_changed_before_read"}));
        }
        let mut bytes = Vec::new();
        (&mut handle)
            .take(remaining.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len())? > remaining {
            return Ok(json!({"status":"unknown","reason":"source_byte_limit_exceeded"}));
        }
        total = total
            .checked_add(u64::try_from(bytes.len())?)
            .ok_or("review source size overflow")?;
        let after_read = handle.metadata()?;
        let after_path = std::fs::symlink_metadata(&file)?;
        if !same_file_snapshot(&opened, &after_read)
            || !same_file_snapshot(&after_read, &after_path)
        {
            return Ok(json!({"status":"unknown","reason":"source_changed_during_read"}));
        }
        let actual = format!("sha256:{:x}", Sha256::digest(bytes));
        if expected != actual {
            stale.push(relative.clone());
        }
    }
    Ok(json!({
        "status":if stale.is_empty() {"current_scoped"} else {"stale"},
        "checked_files":files.len(),"checked_bytes":total,"stale_files":stale,
    }))
}

fn same_workspace(record: &JobRecord, workspace: &Path) -> bool {
    PathBuf::from(&record.workspace_dir)
        .canonicalize()
        .is_ok_and(|path| path == workspace)
}

fn receipt(result: &Value) -> Option<&Value> {
    result
        .get("receipt")
        .or_else(|| result.pointer("/result/receipt"))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Reports whether any present role job ID repeats; absent IDs are never passed in.
fn has_duplicate_ids<'a>(job_ids: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    job_ids.into_iter().any(|job_id| !seen.insert(job_id))
}

fn same_file_snapshot(left: &Metadata, right: &Metadata) -> bool {
    let (Ok(left_modified), Ok(right_modified)) = (left.modified(), right.modified()) else {
        return false;
    };
    if !left.is_file()
        || !right.is_file()
        || left.len() != right.len()
        || left_modified != right_modified
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests;
