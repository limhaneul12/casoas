use super::*;
use crate::routing::{Engine, ExecutionPhase, ExecutionUsage, UsageScope};
use crate::storage::NewJob;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn job(workspace: &std::path::Path, id: &str) -> NewJob {
    NewJob {
        task_type: "planning_stage".into(),
        workspace_dir: workspace.to_path_buf(),
        model: Some("gemini-4-argon-high".into()),
        metadata: BTreeMap::from([("resolved_model".into(), "gemini-4-argon-high".into())]),
        job_id: Some(id.into()),
        idempotency_key: Some(id.into()),
        provider: Some("antigravity".into()),
        engine: Some("antigravity_cli".into()),
        created_at: None,
    }
}
fn observation(sequence: u64) -> ExecutionProgress {
    ExecutionProgress {
        engine: Engine::AntigravityCli,
        session_id: Some("6f038988-1e77-41b1-9d21-72d7a7ee80bc".into()),
        sequence,
        step_index: Some(sequence.saturating_sub(1)),
        phase: ExecutionPhase::AwaitingInput,
        observed_model: Some("gemini-4-argon-high".into()),
        observed_at: "2026-11-05T04:03:27Z".into(),
        stdout_bytes: 128 * sequence,
        usage: None,
    }
}

#[test]
fn session_is_committed_while_running_and_stale_or_terminal_updates_do_not_clobber_it()
-> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    store.create_job(job(workspace.path(), "running-session"))?;
    let first = observation(1);
    let saved = store
        .checkpoint_execution("running-session", first.clone())?
        .ok_or("missing checkpoint")?;
    assert_eq!(saved.status, JobStatus::Running);
    assert_eq!(saved.session_id, first.session_id);
    assert!(saved.result.is_none());
    let reopened = Store::open(directory.path())?;
    assert_eq!(
        reopened
            .get_job("running-session")?
            .ok_or("missing persisted job")?
            .session_id,
        first.session_id
    );
    let mut second = observation(2);
    second.phase = ExecutionPhase::Running;
    second.usage = Some(ExecutionUsage {
        scope: UsageScope::SessionCumulative,
        input_tokens: 10,
        output_tokens: 4,
        thinking_tokens: 2,
        cache_read_tokens: 0,
        total_tokens: 14,
        turns: Some(1),
    });
    let second_saved = store
        .checkpoint_execution("running-session", second.clone())?
        .ok_or("missing second checkpoint")?;
    assert_eq!(
        store.checkpoint_execution("running-session", first.clone())?,
        Some(second_saved.clone())
    );
    assert_eq!(
        store.checkpoint_execution("running-session", second.clone())?,
        Some(second_saved)
    );
    let mut changed = observation(3);
    changed.session_id = Some("another-session".into());
    assert!(
        store
            .checkpoint_execution("running-session", changed.clone())
            .is_err()
    );
    changed.sequence = 1;
    assert!(
        store
            .checkpoint_execution("running-session", changed)
            .is_err()
    );
    let mut regressed = observation(3);
    regressed.step_index = Some(0);
    assert!(
        store
            .checkpoint_execution("running-session", regressed)
            .is_err()
    );
    assert!(
        store
            .complete_job(
                "running-session",
                json!({"ok":true}),
                Some("wrong-final-id".into()),
                None
            )
            .is_err()
    );
    assert_eq!(
        store
            .get_job("running-session")?
            .ok_or("missing running job")?
            .status,
        JobStatus::Running
    );
    let terminal = store
        .complete_job("running-session", json!({"ok":true}), None, None)?
        .ok_or("missing terminal job")?;
    assert_eq!(terminal.session_id, first.session_id);
    assert_eq!(
        store.checkpoint_execution("running-session", observation(99))?,
        Some(terminal.clone())
    );
    assert_eq!(
        store.heartbeat_execution_owner("running-session")?,
        Some(terminal)
    );
    Ok(())
}

#[test]
fn first_model_mismatch_preserves_observed_session_but_changed_binding_is_rejected()
-> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    store.create_job(job(workspace.path(), "model-mismatch"))?;
    let mut first = observation(1);
    first.observed_model = Some("gemini-4-argon-medium".into());
    let saved = store
        .checkpoint_execution("model-mismatch", first.clone())?
        .ok_or("missing checkpoint")?;
    assert_eq!(saved.session_id, first.session_id);
    assert_eq!(saved.model.as_deref(), Some("gemini-4-argon-high"));
    let mut changed = first;
    changed.sequence = 2;
    changed.observed_model = Some("gemini-4-argon-high".into());
    assert!(
        store
            .checkpoint_execution("model-mismatch", changed)
            .is_err()
    );
    assert_eq!(store.get_job("model-mismatch")?, Some(saved));
    Ok(())
}

#[test]
fn foreign_process_instance_cannot_checkpoint_or_heartbeat_and_clone_shares_live_connections()
-> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    let clone = store.clone();
    assert!(Arc::ptr_eq(&store.jobs, &clone.jobs));
    assert!(Arc::ptr_eq(&store.handoffs, &clone.handoffs));
    assert!(Arc::ptr_eq(&store.job_sql, &clone.job_sql));
    let mut untrusted = job(workspace.path(), "foreign-owner");
    untrusted
        .metadata
        .insert("owner_pid".into(), "not-the-host".into());
    untrusted.metadata.insert(
        super::super::OWNER_INSTANCE_ID.into(),
        "forged-instance".into(),
    );
    untrusted
        .metadata
        .insert(OWNER_HEARTBEAT_AT.into(), "999999999999999".into());
    let admitted = store.create_job(untrusted)?;
    assert_eq!(
        admitted.metadata.get("owner_pid"),
        Some(&super::super::execution_owner().pid)
    );
    assert_eq!(
        admitted.metadata.get(super::super::OWNER_INSTANCE_ID),
        Some(&super::super::execution_owner().instance)
    );
    store.transition_job("foreign-owner", |record| {
        record.metadata.insert(
            super::super::OWNER_INSTANCE_ID.into(),
            "older-process-instance".into(),
        );
    })?;
    let before = clone.get_job("foreign-owner")?;
    assert!(
        clone
            .checkpoint_execution("foreign-owner", observation(1))
            .is_err()
    );
    assert!(clone.heartbeat_execution_owner("foreign-owner").is_err());
    assert_eq!(clone.get_job("foreign-owner")?, before);
    Ok(())
}

#[test]
fn owner_heartbeat_keeps_old_live_execution_running_but_lost_owner_is_unknown_without_retry()
-> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    let mut request = job(workspace.path(), "long-lived");
    request.created_at = Some(1.0);
    store.create_job(request.clone())?;
    store.checkpoint_execution("long-lived", observation(1))?;
    let provider_observation = store
        .get_job("long-lived")?
        .ok_or("missing job")?
        .metadata
        .get(PROGRESS_KEY)
        .cloned();
    store.heartbeat_execution_owner("long-lived")?;
    assert_eq!(
        store
            .get_job("long-lived")?
            .ok_or("missing job")?
            .metadata
            .get(PROGRESS_KEY)
            .cloned(),
        provider_observation
    );
    store.transition_job("long-lived", |record| {
        record
            .metadata
            .insert("owner_pid".into(), "4294967294".into());
    })?;
    let other_writer = Store::open(directory.path())?;
    assert_eq!(
        other_writer
            .get_job("long-lived")?
            .ok_or("missing long job")?
            .status,
        JobStatus::Running
    );
    store.transition_job("long-lived", |record| {
        record
            .metadata
            .insert(OWNER_HEARTBEAT_AT.into(), "1".into());
    })?;
    assert_eq!(other_writer.reconcile_stale_running_jobs(3_600.0)?, 1);
    let unknown = other_writer
        .get_job("long-lived")?
        .ok_or("missing unknown job")?;
    assert_eq!(unknown.status, JobStatus::TimedOutUnknown);
    assert_eq!(unknown.session_id, observation(1).session_id);
    assert!(
        !other_writer
            .resolve_or_register(request, true)?
            .should_dispatch
    );
    assert_eq!(
        other_writer.checkpoint_execution("long-lived", observation(2))?,
        Some(unknown)
    );
    Ok(())
}

#[test]
fn malformed_or_conflicting_progress_rolls_back_and_legacy_rows_remain_readable()
-> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    store.create_job(job(workspace.path(), "invalid-progress"))?;
    let before = store.get_job("invalid-progress")?;
    for kind in 0..4 {
        let mut invalid = observation(1);
        match kind {
            0 => invalid.session_id = Some("bad\0id".into()),
            1 => invalid.engine = Engine::CodexCli,
            2 => invalid.observed_at = "not-a-timestamp".into(),
            _ => invalid.observed_model = Some("x".repeat(129)),
        }
        assert!(
            store
                .checkpoint_execution("invalid-progress", invalid)
                .is_err()
        );
        assert_eq!(store.get_job("invalid-progress")?, before);
    }
    let mut legacy = serde_json::to_value(before.ok_or("missing legacy job")?)?;
    legacy["engine"] = json!("antigravity_rpc");
    legacy["metadata"]
        .as_object_mut()
        .ok_or("missing metadata")?
        .remove(super::super::OWNER_INSTANCE_ID);
    legacy["metadata"]
        .as_object_mut()
        .ok_or("missing metadata")?
        .remove(OWNER_HEARTBEAT_AT);
    let decoded: JobRecord = serde_json::from_value(legacy)?;
    assert_eq!(decoded.engine.as_deref(), Some("antigravity_rpc"));
    assert!(decoded.session_id.is_none());
    Ok(())
}
