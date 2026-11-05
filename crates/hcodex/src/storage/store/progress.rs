//! Transactional, same-owner execution observations; provider text is never stored here.
use super::{
    JobRecord, JobStatus, OWNER_HEARTBEAT_AT, Store, execution_owner, now_seconds, owns_execution,
};
use crate::routing::ExecutionProgress;
use chrono::DateTime;

pub(super) const PROGRESS_KEY: &str = "execution_progress";
const MAX_PROGRESS_BYTES: usize = 4 * 1024;

impl JobRecord {
    /// Latest bounded controller observation, not a live provider query.
    pub fn execution_progress(&self) -> Option<ExecutionProgress> {
        let stored = self.metadata.get(PROGRESS_KEY)?;
        if stored.len() > MAX_PROGRESS_BYTES {
            return None;
        }
        let progress: ExecutionProgress = serde_json::from_str(stored).ok()?;
        if progress.session_id != self.session_id {
            return None;
        }
        Some(progress)
    }
}

impl Store {
    /// Commits an in-flight session binding before runtime may send its first
    /// prompt. Duplicate/stale checkpoints and late terminal observations are
    /// no-ops; contradictory immutable bindings are rejected transactionally.
    pub fn checkpoint_execution(
        &self,
        job_id: &str,
        progress: ExecutionProgress,
    ) -> crate::Result<Option<JobRecord>> {
        self.update_owned_execution(job_id, |record, now| {
            let serialized = validate_progress(record, &progress)?;
            if let Some(session_id) = &record.session_id
                && progress.session_id.as_ref() != Some(session_id)
            {
                return Err(
                    "execution checkpoint changed or omitted the committed session ID".into(),
                );
            }
            if let Some(previous) = record.metadata.get(PROGRESS_KEY) {
                let previous: ExecutionProgress = serde_json::from_str(previous)?;
                validate_binding(&previous, &progress)?;
                if progress.sequence < previous.sequence || progress == previous {
                    return Ok(false);
                }
                if progress.sequence == previous.sequence {
                    return Err(
                        "execution checkpoint sequence conflicts with an existing observation"
                            .into(),
                    );
                }
                if progress.step_index < previous.step_index {
                    return Err("execution provider step regressed".into());
                }
                if progress.stdout_bytes < previous.stdout_bytes {
                    return Err("execution output byte count regressed".into());
                }
            }
            if record.session_id.is_none() && progress.session_id.is_some() {
                record.session_id.clone_from(&progress.session_id);
            }
            // observed_model is the CLI's observation, even when it mismatches the
            // requested model. Runtime owns rejection; preserving the first ID is
            // necessary to diagnose that failure without issuing another request.
            record.metadata.insert(PROGRESS_KEY.to_owned(), serialized);
            record
                .metadata
                .insert(OWNER_HEARTBEAT_AT.to_owned(), now.to_string());
            Ok(true)
        })
    }

    /// Controller liveness is deliberately independent of provider activity.
    pub fn heartbeat_execution_owner(&self, job_id: &str) -> crate::Result<Option<JobRecord>> {
        self.update_owned_execution(job_id, |record, now| {
            record
                .metadata
                .insert(OWNER_HEARTBEAT_AT.to_owned(), now.to_string());
            Ok(true)
        })
    }

    fn update_owned_execution(
        &self,
        job_id: &str,
        update: impl FnOnce(&mut JobRecord, f64) -> crate::Result<bool>,
    ) -> crate::Result<Option<JobRecord>> {
        let owner = execution_owner();
        self.with_job_for_update(job_id, |record, connection| {
            if record.status != JobStatus::Running {
                return Ok(());
            }
            if !owns_execution(record, owner) {
                return Err("execution observation is not owned by this process instance".into());
            }
            if update(record, now_seconds())? {
                // Progress and liveness update only a running record. They
                // cannot add completed rows and don't need a retention scan.
                self.write_job_on(connection, record)?;
            }
            Ok(())
        })
    }
}

fn validate_progress(record: &JobRecord, progress: &ExecutionProgress) -> crate::Result<String> {
    if let Some(session_id) = &progress.session_id {
        bounded_atom(session_id, 256, "session ID")?;
    }
    if let Some(model) = &progress.observed_model {
        bounded_atom(model, 128, "observed model")?;
    }
    bounded_atom(&progress.observed_at, 128, "observation timestamp")?;
    DateTime::parse_from_rfc3339(&progress.observed_at)
        .map_err(|_| "execution observation timestamp must be RFC3339")?;
    let engine = serde_json::to_value(progress.engine)?;
    if record
        .engine
        .as_deref()
        .is_some_and(|bound| Some(bound) != engine.as_str())
    {
        return Err("execution checkpoint engine differs from its admitted job".into());
    }
    let serialized = serde_json::to_string(progress)?;
    if serialized.len() > MAX_PROGRESS_BYTES {
        return Err("execution checkpoint exceeds its metadata byte budget".into());
    }
    Ok(serialized)
}

fn validate_binding(previous: &ExecutionProgress, next: &ExecutionProgress) -> crate::Result<()> {
    if previous.engine != next.engine {
        return Err("execution checkpoint changed its engine".into());
    }
    if let Some(session_id) = &previous.session_id
        && next.session_id.as_ref() != Some(session_id)
    {
        return Err("execution checkpoint changed its observed session ID".into());
    }
    if let Some(model) = &previous.observed_model
        && next.observed_model.as_ref() != Some(model)
    {
        return Err("execution checkpoint changed its observed model".into());
    }
    Ok(())
}

fn bounded_atom(value: &str, max: usize, label: &str) -> crate::Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(format!(
            "execution {label} is empty, oversized or contains control characters"
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
