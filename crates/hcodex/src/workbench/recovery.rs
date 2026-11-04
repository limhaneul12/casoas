//! Bounded recovery observations over the existing storage and worktree owners.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::routing::normalize_model;
use crate::storage::diagnostics::{self, ReadObservation, Snapshot};
use crate::storage::{JobRecord, JobStatus, OutcomeState};

/// Read existing evidence without dispatch, reconciliation, cleanup or resume.
pub async fn recovery(state_root: &Path, workspace: &Path, job_id: &str) -> crate::Result<Value> {
    if job_id.is_empty() || job_id.len() > 256 || job_id.chars().any(char::is_control) {
        return Err("recovery requires one bounded non-control job ID".into());
    }
    let workspace = workspace.to_path_buf();
    let (workspace, before) = read(state_root.to_path_buf(), workspace, job_id.to_owned()).await?;
    let snapshot = match before {
        ReadObservation::Observed(snapshot) => snapshot,
        ReadObservation::Unavailable(issue) => {
            return Ok(report(
                &workspace,
                job_id,
                "unavailable",
                issue.code(),
                None,
                None,
            ));
        }
        ReadObservation::Unknown(issue) => {
            return Ok(report(
                &workspace,
                job_id,
                "unknown",
                issue.code(),
                None,
                None,
            ));
        }
    };
    let Some(job) = &snapshot.job else {
        return Ok(report(
            &workspace,
            job_id,
            "unavailable",
            "missing_job",
            None,
            None,
        ));
    };
    if let Some(reason) = identity_issue(&snapshot, &workspace) {
        return Ok(report(&workspace, job_id, "unknown", reason, None, None));
    }
    let worktrees = if let Some(path) = job.metadata.get("worktree_state_path") {
        if job.task_type != "dag_ulw" {
            return Ok(report(
                &workspace,
                job_id,
                "unknown",
                "invalid_campaign_type",
                None,
                None,
            ));
        }
        Some(serde_json::to_value(
            crate::dag::inspect_worktree_recovery(state_root, &workspace, job_id, Path::new(path))
                .await,
        )?)
    } else {
        None
    };
    let (workspace_after, after) = read(
        state_root.to_path_buf(),
        workspace.clone(),
        job_id.to_owned(),
    )
    .await?;
    if workspace_after != workspace
        || !matches!(after, ReadObservation::Observed(ref after) if after.fingerprint == snapshot.fingerprint)
    {
        return Ok(report(
            &workspace,
            job_id,
            "unknown",
            "observation_changed",
            None,
            None,
        ));
    }
    Ok(report(
        &workspace,
        job_id,
        "observed",
        "stored_evidence_observed",
        Some(project_job(job, snapshot.export.as_ref())),
        worktrees,
    ))
}

async fn read(
    state_root: PathBuf,
    workspace: PathBuf,
    job_id: String,
) -> crate::Result<(PathBuf, ReadObservation)> {
    tokio::task::spawn_blocking(move || {
        let workspace = workspace.canonicalize()?;
        Ok((workspace, diagnostics::read(&state_root, &job_id)))
    })
    .await?
}

fn identity_issue(snapshot: &Snapshot, workspace: &Path) -> Option<&'static str> {
    let job = snapshot.job.as_ref()?;
    if Path::new(&job.workspace_dir) != workspace {
        return Some("foreign_workspace");
    }
    if let Some(export) = &snapshot.export
        && (export.task_type != "dag_worktree_export"
            || export.status != JobStatus::Completed
            || Path::new(&export.workspace_dir) != workspace)
    {
        return Some("export_identity_mismatch");
    }
    None
}

fn project_job(job: &JobRecord, export: Option<&JobRecord>) -> Value {
    let receipt = job.receipt.as_ref().or_else(|| {
        job.result.as_ref().and_then(|result| {
            result
                .get("receipt")
                .or_else(|| result.pointer("/result/receipt"))
        })
    });
    let recorded_provider_evidence = receipt.map(|receipt| {
        let mut fields = serde_json::Map::new();
        for name in [
            "status",
            "outcome_state",
            "requested_model",
            "actual_model",
            "model_mismatch",
            "owned_processes_closed",
        ] {
            if let Some(value) = receipt.get(name)
                && valid_receipt_field(name, value)
            {
                fields.insert(name.to_owned(), value.clone());
            }
        }
        Value::Object(fields)
    });
    json!({
        "status":job.status,
        "outcome_state":job.outcome_state,
        "model":job.model.as_deref().and_then(bounded_label).and_then(normalize_model),
        "attempt_count":job.attempt_count,
        "cancel_requested":job.cancel_requested,
        "recorded_provider_evidence":recorded_provider_evidence,
        "execution_export_id":export.map(|record|&record.job_id),
    })
}

fn valid_receipt_field(name: &str, value: &Value) -> bool {
    if matches!(name, "model_mismatch" | "owned_processes_closed") {
        return value.is_boolean();
    }
    let Some(text) = value.as_str().and_then(bounded_label) else {
        return false;
    };
    match name {
        "status" => serde_json::from_value::<JobStatus>(value.clone()).is_ok(),
        "outcome_state" => serde_json::from_value::<OutcomeState>(value.clone()).is_ok(),
        "requested_model" | "actual_model" => normalize_model(text).is_some(),
        _ => false,
    }
}

fn bounded_label(value: &str) -> Option<&str> {
    (value.len() <= 128 && !value.chars().any(char::is_control)).then_some(value)
}

fn report(
    workspace: &Path,
    job_id: &str,
    observation: &'static str,
    reason: &'static str,
    job: Option<Value>,
    worktrees: Option<Value>,
) -> Value {
    json!({
        "read_only":true,
        "read_only_scope":"persistent_domain_state",
        "observation_only":true,
        "resume_authorized":false,
        "job_id":job_id,
        "workspace_dir":workspace,
        "observation":observation,
        "reason":reason,
        "job":job,
        "worktrees":worktrees,
    })
}
