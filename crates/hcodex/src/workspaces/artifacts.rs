//! Owned external stage directories and immutable planning snapshots.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::gateway::Gateway;
use crate::routing::digest_output;
use crate::storage::{JobRecord, JobStatus, NewJob};
use crate::{Error, Result};

use super::{PlanRecord, WorkspaceKind};

const PLAN_TASK: &str = "workspace_plan";

pub fn stage_path(
    state_root: &Path,
    original: &Path,
    flow_id: &str,
    kind: WorkspaceKind,
) -> Result<PathBuf> {
    validate_id(flow_id)?;
    let original = original.canonicalize()?;
    let state = canonical_future_path(state_root)?;
    if state.starts_with(&original) {
        return Err("workflow state root must be outside the original workspace".into());
    }
    let original_key = digest_output(&original.to_string_lossy()).replace("sha256:", "");
    let flow_key = digest_output(flow_id).replace("sha256:", "");
    Ok(state
        .join("workspaces")
        .join(&original_key[..32])
        .join(&flow_key[..32])
        .join(kind.as_str()))
}

#[derive(Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct StageOwner {
    schema_version: u8,
    original: PathBuf,
    flow_id: String,
    stage: WorkspaceKind,
}

pub async fn prepare_stage(
    state_root: &Path,
    original: &Path,
    flow_id: &str,
    kind: WorkspaceKind,
) -> Result<PathBuf> {
    let expected = stage_path(state_root, original, flow_id, kind)?;
    let owner = StageOwner {
        schema_version: 1,
        original: original.canonicalize()?,
        flow_id: flow_id.into(),
        stage: kind,
    };
    tokio::task::spawn_blocking(move || {
        let mut cursor = PathBuf::new();
        let mut created_stage = false;
        for component in expected.components() {
            cursor.push(component.as_os_str());
            match fs::symlink_metadata(&cursor) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(
                        "workflow directory path contains a symlink or non-directory".into(),
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut builder = fs::DirBuilder::new();
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::DirBuilderExt;
                        builder.mode(0o700);
                    }
                    builder.create(&cursor)?;
                    if cursor == expected {
                        created_stage = true;
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let marker = expected.join(".hcodex-stage-owner.json");
        if created_stage {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&marker)?;
            file.write_all(&serde_json::to_vec(&owner)?)?;
            file.sync_all()?;
        } else {
            let metadata = fs::symlink_metadata(&marker)
                .map_err(|_| "existing workflow stage has no ownership proof")?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
                return Err("workflow stage ownership marker is unsafe".into());
            }
            let file = fs::File::open(&marker)?;
            let mut bytes = Vec::new();
            file.take(4097).read_to_end(&mut bytes)?;
            let stored: StageOwner = serde_json::from_slice(&bytes)?;
            if stored != owner {
                return Err("workflow stage belongs to a different owner".into());
            }
        }
        if expected.canonicalize()? != expected {
            return Err("workflow stage identity changed during admission".into());
        }
        Ok::<_, Error>(expected)
    })
    .await?
}

pub async fn persist_plan(gateway: &Gateway, record: &PlanRecord) -> Result<()> {
    if record.plan_id != super::planning::plan_id(&record.flow_id)?
        || record.artifact_digest != digest_output(&serde_json::to_string(&record.artifact)?)
    {
        return Err("planning artifact identity or digest is invalid".into());
    }
    let id = record.plan_id.clone();
    let snapshot = serde_json::to_value(record)?;
    let new = NewJob {
        task_type: PLAN_TASK.to_owned(),
        workspace_dir: record.workspace_dir.clone(),
        model: None,
        metadata: BTreeMap::from([
            (
                "snapshot_digest".into(),
                digest_output(&snapshot.to_string()),
            ),
            (
                "request_identity".into(),
                digest_output(&snapshot.to_string()),
            ),
            ("flow_id".into(), record.flow_id.clone()),
            ("artifact_digest".into(), record.artifact_digest.clone()),
            (
                "source_manifest_digest".into(),
                record.source_manifest_digest.clone(),
            ),
        ]),
        job_id: Some(id.clone()),
        idempotency_key: Some(id.clone()),
        provider: Some("hcodex_workspaces".into()),
        engine: Some("planning_artifact".into()),
        created_at: None,
    };
    gateway
        .database(move |store| {
            let registration = store.resolve_or_register(new, false)?;
            if !registration.should_dispatch {
                return if registration.record.status == JobStatus::Completed
                    && registration.record.result.as_ref() == Some(&snapshot)
                {
                    Ok(())
                } else {
                    Err("planning artifact identity conflict".into())
                };
            }
            store
                .complete_job(&id, snapshot, None, None)?
                .ok_or("planning artifact disappeared before commit")?;
            Ok(())
        })
        .await
}

pub async fn read_plan(
    gateway: &Gateway,
    original: &Path,
    plan_id: &str,
) -> Result<Option<PlanRecord>> {
    validate_id(plan_id)?;
    let original = original.canonicalize()?;
    let id = plan_id.to_owned();
    gateway
        .database(move |store| {
            let Some(record) = store.get_job(&id)? else {
                return Ok(None);
            };
            plan_from_record(&record, &original).map(Some)
        })
        .await
}

/// One pure validator for both normal imports and read-only status projection.
pub(super) fn plan_from_record(record: &JobRecord, original: &Path) -> Result<PlanRecord> {
    if record.task_type != PLAN_TASK || Path::new(&record.workspace_dir) != original {
        return Err("planning artifact does not belong to this workspace".into());
    }
    if record.status != JobStatus::Completed {
        return Err("planning artifact is not durably completed".into());
    }
    let value = record
        .result
        .as_ref()
        .ok_or("planning artifact has no stored snapshot")?;
    if serde_json::to_vec(value)?.len() > 512 * 1024 {
        return Err("planning snapshot exceeds its byte budget".into());
    }
    if record.metadata.get("snapshot_digest") != Some(&digest_output(&value.to_string())) {
        return Err("planning snapshot digest is missing or inconsistent".into());
    }
    let plan: PlanRecord = serde_json::from_value(value.clone())?;
    if plan.workspace_dir != original
        || record.metadata.get("flow_id") != Some(&plan.flow_id)
        || record.metadata.get("source_manifest_digest") != Some(&plan.source_manifest_digest)
        || record.metadata.get("artifact_digest") != Some(&plan.artifact_digest)
        || plan.plan_id != record.job_id
        || plan.plan_id != super::planning::plan_id(&plan.flow_id)?
        || plan.artifact_digest != digest_output(&serde_json::to_string(&plan.artifact)?)
    {
        return Err("planning artifact readback failed identity validation".into());
    }
    Ok(plan)
}

/// Returns only the immutable report body, never status/readback decorations.
pub async fn read_research(
    gateway: &Gateway,
    original: &Path,
    research_id: &str,
) -> Result<Option<Value>> {
    if research_id.is_empty()
        || research_id.len() > 256
        || research_id.chars().any(char::is_control)
    {
        return Err("invalid research artifact ID".into());
    }
    let state = gateway.state_root.clone();
    let original = original.canonicalize()?;
    let id = research_id.to_owned();
    tokio::task::spawn_blocking(move || {
        use crate::storage::diagnostics::{ReadObservation, read};
        let record = match read(&state, &id) {
            ReadObservation::Observed(snapshot) => match snapshot.job {
                Some(job) => job,
                None => return Ok(None),
            },
            ReadObservation::Unavailable(_) => return Ok(None),
            ReadObservation::Unknown(issue) => {
                return Err(format!(
                    "research evidence observation unavailable: {}",
                    issue.code()
                )
                .into());
            }
        };
        if record.task_type != "research_campaign" || Path::new(&record.workspace_dir) != original {
            return Err("research evidence belongs to a different workspace or task".into());
        }
        if !matches!(record.status, JobStatus::Completed | JobStatus::Succeeded) {
            return Err("research evidence is not a known completed outcome".into());
        }
        let report = record
            .result
            .ok_or("research evidence has no immutable report")?;
        if report["request_id"] != id
            || !matches!(
                report["research_status"].as_str(),
                Some("complete" | "incomplete_evidence")
            )
        {
            return Err("research evidence is partial, unknown or has an inconsistent ID".into());
        }
        if let Some(workspace) = report["workspace_dir"].as_str()
            && Path::new(workspace) != original
        {
            return Err("research report workspace binding mismatch".into());
        }
        if serde_json::to_vec(&report)?.len() > 1_048_576 {
            return Err("research report exceeds the handoff byte budget".into());
        }
        Ok::<_, Error>(Some(report))
    })
    .await?
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    {
        return Err("invalid workflow identifier".into());
    }
    Ok(())
}

fn canonical_future_path(path: &Path) -> Result<PathBuf> {
    let mut current = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    for component in path.components() {
        match component {
            Component::RootDir => current.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(part) => current.push(part),
            Component::ParentDir | Component::Prefix(_) => {
                return Err("workflow state path contains unsupported components".into());
            }
        }
        if current.exists() {
            current = current.canonicalize()?;
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stage_is_external_and_confined() -> Result<()> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        let state = root.path().join("state");
        fs::create_dir(&original)?;
        let stage = prepare_stage(&state, &original, "flow-one", WorkspaceKind::Planning).await?;
        assert!(stage.starts_with(&state));
        assert!(!stage.starts_with(&original));
        assert!(
            stage_path(
                &original.join("state"),
                &original,
                "flow-one",
                WorkspaceKind::Planning
            )
            .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn foreign_stage_and_symlink_ancestors_are_rejected_before_writes() -> Result<()> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        let state = root.path().join("state");
        fs::create_dir(&original)?;
        let path = stage_path(&state, &original, "foreign-flow", WorkspaceKind::Planning)?;
        fs::create_dir_all(&path)?;
        fs::write(path.join("keep.txt"), "foreign data")?;
        assert!(
            prepare_stage(&state, &original, "foreign-flow", WorkspaceKind::Planning)
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(path.join("keep.txt"))?, "foreign data");
        assert!(!path.join(".hcodex-stage-owner.json").exists());
        #[cfg(unix)]
        {
            let other_state = root.path().join("other-state");
            let outside = root.path().join("outside");
            fs::create_dir(&other_state)?;
            fs::create_dir(&outside)?;
            std::os::unix::fs::symlink(&outside, other_state.join("workspaces"))?;
            assert!(
                prepare_stage(
                    &other_state,
                    &original,
                    "guarded-flow",
                    WorkspaceKind::Research
                )
                .await
                .is_err()
            );
            assert_eq!(fs::read_dir(&outside)?.count(), 0);
        }
        let owned = prepare_stage(&state, &original, "owned-flow", WorkspaceKind::Planning).await?;
        assert_eq!(
            prepare_stage(&state, &original, "owned-flow", WorkspaceKind::Planning).await?,
            owned
        );
        Ok(())
    }

    async fn report_fixture(
        gateway: &Gateway,
        original: &Path,
        id: &str,
        status: JobStatus,
        report_status: &str,
    ) -> Result<Value> {
        let value = serde_json::json!({"request_id":id,"workspace_dir":original,"research_status":report_status,"sources":[{"excerpt":{"text":"actual captured evidence"}}],"claims":[],"unresolved":[]});
        let new = NewJob {
            task_type: "research_campaign".into(),
            workspace_dir: original.into(),
            model: None,
            metadata: BTreeMap::new(),
            job_id: Some(id.into()),
            idempotency_key: None,
            provider: None,
            engine: None,
            created_at: None,
        };
        let owned_id = id.to_owned();
        let body = value.clone();
        gateway
            .database(move |store| {
                store.create_job(new)?;
                match status {
                    JobStatus::Completed | JobStatus::Succeeded => {
                        store.complete_job(&owned_id, body, None, None)?;
                    }
                    JobStatus::Cancelled => {
                        store.cancel_job_with_result(
                            &owned_id,
                            "fixture cancelled".into(),
                            Some(body),
                            None,
                        )?;
                    }
                    JobStatus::TimedOutUnknown => {
                        store.mark_timed_out_unknown(
                            &owned_id,
                            "fixture unknown".into(),
                            Some(body),
                            None,
                        )?;
                    }
                    JobStatus::Running => {}
                    _ => return Err("unsupported fixture status".into()),
                }
                Ok(())
            })
            .await?;
        Ok(value)
    }

    #[tokio::test]
    async fn research_handoff_is_canonical_completed_scoped_and_read_only() -> Result<()> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        let foreign = root.path().join("foreign");
        fs::create_dir(&original)?;
        fs::create_dir(&foreign)?;
        let gateway = Gateway::new(root.path().join("state"));
        let report = report_fixture(
            &gateway,
            &original,
            "research-known",
            JobStatus::Completed,
            "incomplete_evidence",
        )
        .await?;
        assert_eq!(
            read_research(&gateway, &original, "research-known").await?,
            Some(report)
        );
        assert!(
            read_research(&gateway, &foreign, "research-known")
                .await
                .is_err()
        );
        for (id, status, report_status) in [
            ("research-running", JobStatus::Running, "complete"),
            ("research-unknown", JobStatus::TimedOutUnknown, "partial"),
            ("research-cancelled", JobStatus::Cancelled, "partial"),
            ("research-partial", JobStatus::Completed, "partial"),
        ] {
            report_fixture(&gateway, &original, id, status, report_status).await?;
            assert!(read_research(&gateway, &original, id).await.is_err());
        }
        let absent = root.path().join("absent");
        let observer = Gateway::new(absent.clone());
        assert!(
            read_research(&observer, &original, "missing")
                .await?
                .is_none()
        );
        assert!(!absent.exists());
        gateway.shutdown().await?;
        Ok(())
    }
}
