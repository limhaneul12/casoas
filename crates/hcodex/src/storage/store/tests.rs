use std::collections::BTreeMap;
use std::fs as test_fs;
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::tempdir;

use super::*;
use crate::storage::{HandoffPriority, NewHandoff, NewJob, OutcomeState};

fn new_job(workspace: &Path) -> NewJob {
    NewJob {
        task_type: "audit".to_owned(),
        workspace_dir: workspace.to_path_buf(),
        model: Some("gpt-6.1-sol".to_owned()),
        metadata: BTreeMap::new(),
        job_id: None,
        idempotency_key: Some("stable-key".to_owned()),
        provider: Some("openai_codex".to_owned()),
        engine: Some("codex_cli".to_owned()),
        created_at: None,
    }
}

#[test]
fn job_lifecycle_is_idempotent_and_cancel_is_final() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let store = Store::open(root.path())?;
    let first = store.resolve_or_register(new_job(workspace.path()), false)?;
    assert!(first.should_dispatch);
    let duplicate = store.resolve_or_register(new_job(workspace.path()), false)?;
    assert!(!duplicate.should_dispatch);
    assert_eq!(first.record.job_id, duplicate.record.job_id);

    let timeout = store
        .mark_timed_out_unknown(
            &first.record.job_id,
            "read timed out".to_owned(),
            Some(json!({"partial": true})),
            None,
        )?
        .ok_or_else(|| IoError::other("missing timed-out job"))?;
    assert_eq!(timeout.outcome_state, OutcomeState::OutcomeUnknown);
    let cancelled = store
        .cancel_job(&first.record.job_id, "caller cancelled".to_owned())?
        .ok_or_else(|| IoError::other("missing cancelled job"))?;
    assert_eq!(cancelled.status, JobStatus::Cancelled);
    let completed = store
        .complete_job(&first.record.job_id, json!({"ok": true}), None, None)?
        .ok_or_else(|| IoError::other("missing completed job"))?;
    assert_eq!(completed.status, JobStatus::Cancelled);
    let failed = store
        .fail_job_with_result(
            &first.record.job_id,
            "late failure".to_owned(),
            Some(json!({"late": true})),
            None,
        )?
        .ok_or_else(|| IoError::other("missing cancelled job after failure"))?;
    assert_eq!(failed.status, JobStatus::Cancelled);
    assert_eq!(failed.result, Some(json!({"partial": true})));
    let enriched = store
        .cancel_job_with_result(
            &first.record.job_id,
            "runtime cancellation".to_owned(),
            Some(json!({"replacement": true})),
            Some(json!({"status": "cancelled"})),
        )?
        .ok_or_else(|| IoError::other("missing enriched cancellation"))?;
    assert_eq!(enriched.result, cancelled.result);
    assert_eq!(enriched.cancelled_at, cancelled.cancelled_at);
    assert_eq!(enriched.error, cancelled.error);
    assert_eq!(enriched.receipt, Some(json!({"status": "cancelled"})));
    Ok(())
}

#[test]
fn pre_dispatch_rejection_is_queryable_and_never_clobbers_terminal_state() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let store = Store::open(root.path())?;

    let rejected = store.create_job(NewJob {
        idempotency_key: Some("reject-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let receipt = json!({"status": "rejected_pre_dispatch", "error_code": "BAD_ROUTE"});
    let transitioned = store
        .reject_before_dispatch(
            &rejected.job_id,
            "route rejected".to_owned(),
            Some(receipt.clone()),
        )?
        .ok_or_else(|| IoError::other("missing rejected job"))?;
    assert_eq!(transitioned.status, JobStatus::RejectedPreDispatch);
    assert_eq!(transitioned.outcome_state, OutcomeState::ConfirmedFailure);
    assert_eq!(transitioned.receipt, Some(receipt));
    let queried = store
        .find_job_by_idempotency_key("reject-key")?
        .ok_or_else(|| IoError::other("rejected job is not queryable"))?;
    assert_eq!(queried.status, JobStatus::RejectedPreDispatch);

    let completed = store.create_job(NewJob {
        idempotency_key: Some("complete-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let _ = store.complete_job(&completed.job_id, json!({"ok": true}), None, None)?;
    let preserved = store
        .reject_before_dispatch(&completed.job_id, "late rejection".to_owned(), None)?
        .ok_or_else(|| IoError::other("missing completed job"))?;
    assert_eq!(preserved.status, JobStatus::Completed);

    let cancelled = store.create_job(NewJob {
        idempotency_key: Some("cancel-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let _ = store.cancel_job(&cancelled.job_id, "cancelled".to_owned())?;
    let preserved = store
        .reject_before_dispatch(&cancelled.job_id, "late rejection".to_owned(), None)?
        .ok_or_else(|| IoError::other("missing cancelled job"))?;
    assert_eq!(preserved.status, JobStatus::Cancelled);

    let budget = store.create_job(NewJob {
        idempotency_key: Some("budget-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let exceeded = store
        .mark_input_budget_exceeded(
            &budget.job_id,
            "input too large".to_owned(),
            Some(json!({"status": "input_budget_exceeded"})),
        )?
        .ok_or_else(|| IoError::other("missing budget job"))?;
    assert_eq!(exceeded.status, JobStatus::InputBudgetExceeded);
    let preserved = store
        .complete_job(&budget.job_id, json!({"late": true}), None, None)?
        .ok_or_else(|| IoError::other("missing budget terminal job"))?;
    assert_eq!(preserved.status, JobStatus::InputBudgetExceeded);

    let detailed = store.create_job(NewJob {
        idempotency_key: Some("details-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let mut metadata = BTreeMap::new();
    metadata.insert("actual_model".to_owned(), "gpt-6.1-sol".to_owned());
    let detailed = store
        .complete_job_with_details(
            &detailed.job_id,
            json!({"ok": true}),
            Some("thr_test".to_owned()),
            Some(json!({"actual_model": "gpt-6.1-sol"})),
            Some("gpt-6.1-sol".to_owned()),
            metadata,
        )?
        .ok_or_else(|| IoError::other("missing detailed completion"))?;
    assert_eq!(detailed.session_id.as_deref(), Some("thr_test"));
    assert_eq!(detailed.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(
        detailed.metadata.get("actual_model").map(String::as_str),
        Some("gpt-6.1-sol")
    );

    let failed = store.create_job(NewJob {
        idempotency_key: Some("failed-result-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let failed = store
        .fail_job_with_result(
            &failed.job_id,
            "triad persistence failed".to_owned(),
            Some(json!({"completed_peers": ["codex", "sonnet"]})),
            Some(json!({"status": "failed"})),
        )?
        .ok_or_else(|| IoError::other("missing failed result job"))?;
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(
        failed.result,
        Some(json!({"completed_peers": ["codex", "sonnet"]}))
    );
    Ok(())
}

#[test]
fn reads_python_compatible_json_and_filters_handoffs() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let store = Store::open(root.path())?;
    let ticket = store.create_handoff(NewHandoff {
        sender: TriadRole::Codex,
        recipient: TriadRole::Argon,
        title: "Review migration".to_owned(),
        spec_or_payload: "Inspect all contracts".to_owned(),
        workspace_dir: workspace.path().to_path_buf(),
        priority: HandoffPriority::High,
        alexandria_ref: None,
    })?;
    let listed = store.list_handoffs(HandoffFilter {
        recipient: Some(TriadRole::Argon),
        status: Some(HandoffStatus::Pending),
        workspace_dir: Some(workspace.path().to_path_buf()),
        limit: 20,
    })?;
    assert_eq!(listed, vec![ticket.clone()]);
    let updated = store
        .update_handoff(
            &ticket.ticket_id,
            HandoffStatus::Completed,
            Some("PASS: reviewed".to_owned()),
            Some("alex://decision".to_owned()),
        )?
        .ok_or_else(|| IoError::other("missing handoff"))?;
    assert_eq!(updated.status, HandoffStatus::Completed);
    assert!(store.pending_handoff_counts()?.is_empty());
    store.checkpoint()?;
    Ok(())
}

#[test]
fn rejects_nul_before_sql_binding() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let store = Store::open(root.path())?;
    let mut request = new_job(workspace.path());
    request.task_type = "bad\0task".to_owned();
    assert!(store.create_job(request).is_err());
    Ok(())
}

#[test]
fn concurrent_idempotent_registration_has_one_identity() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let stores = [
        Arc::new(Store::open(root.path())?),
        Arc::new(Store::open(root.path())?),
    ];
    let barrier = Arc::new(Barrier::new(12));
    let request = new_job(workspace.path());
    let mut handles = Vec::new();
    for index in 0..12 {
        let shared = Arc::clone(&stores[index % stores.len()]);
        let start = Arc::clone(&barrier);
        let cloned = request.clone();
        handles.push(thread::spawn(move || {
            start.wait();
            shared.resolve_or_register(cloned, false)
        }));
    }
    let mut registrations = Vec::new();
    for handle in handles {
        registrations.push(
            handle
                .join()
                .map_err(|_| IoError::other("registration thread panicked"))??,
        );
    }
    let identity = &registrations[0].record.job_id;
    assert!(
        registrations
            .iter()
            .all(|item| item.record.job_id == *identity)
    );
    assert_eq!(
        registrations
            .iter()
            .filter(|item| item.should_dispatch)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn cross_process_idempotency_has_one_dispatch_and_identity() -> crate::Result<()> {
    const CHILD_FLAG: &str = "HCODEX_IDEMPOTENCY_CHILD";
    if std::env::var_os(CHILD_FLAG).is_some() {
        let root = PathBuf::from(std::env::var("HCODEX_IDEMPOTENCY_ROOT")?);
        let ready = PathBuf::from(std::env::var("HCODEX_IDEMPOTENCY_READY")?);
        let output = PathBuf::from(std::env::var("HCODEX_IDEMPOTENCY_OUTPUT")?);
        test_fs::write(&ready, b"ready")?;
        let go = root.join("go");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !go.is_file() {
            if Instant::now() >= deadline {
                return Err(IoError::other("process barrier timed out").into());
            }
            thread::yield_now();
        }
        let workspace = root.join("workspace");
        test_fs::create_dir_all(&workspace)?;
        let registration = Store::open(&root)?.resolve_or_register(
            NewJob {
                idempotency_key: Some("process-key".to_owned()),
                ..new_job(&workspace)
            },
            false,
        )?;
        test_fs::write(
            output,
            serde_json::to_vec(&serde_json::json!({
                "job_id": registration.record.job_id,
                "dispatch": registration.should_dispatch,
            }))?,
        )?;
        return Ok(());
    }

    let root = tempdir()?;
    let executable = std::env::current_exe()?;
    let test_name =
        "storage::store::tests::cross_process_idempotency_has_one_dispatch_and_identity";
    let mut children = Vec::new();
    for index in 0..2 {
        let ready = root.path().join(format!("ready-{index}"));
        let output = root.path().join(format!("output-{index}.json"));
        let child = Command::new(&executable)
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD_FLAG, "1")
            .env("HCODEX_IDEMPOTENCY_ROOT", root.path())
            .env("HCODEX_IDEMPOTENCY_READY", &ready)
            .env("HCODEX_IDEMPOTENCY_OUTPUT", &output)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        children.push((child, ready, output));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while children.iter().any(|(_, ready, _)| !ready.is_file()) {
        if Instant::now() >= deadline {
            return Err(IoError::other("child readiness timed out").into());
        }
        thread::yield_now();
    }
    test_fs::write(root.path().join("go"), b"go")?;
    let mut outcomes = Vec::new();
    for (child, _, output) in children {
        let child_output = child.wait_with_output()?;
        if !child_output.status.success() {
            return Err(IoError::other(format!(
                "child failed with {}\nstdout:\n{}\nstderr:\n{}",
                child_output.status,
                String::from_utf8_lossy(&child_output.stdout),
                String::from_utf8_lossy(&child_output.stderr),
            ))
            .into());
        }
        outcomes.push(serde_json::from_slice::<Value>(&test_fs::read(output)?)?);
    }
    assert_eq!(outcomes[0]["job_id"], outcomes[1]["job_id"]);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["dispatch"] == true)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn independent_store_cancel_completion_race_is_terminally_coherent() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let creator = Store::open(root.path())?;
    let job = creator.create_job(NewJob {
        idempotency_key: Some("race-key".to_owned()),
        ..new_job(workspace.path())
    })?;
    let [cancel_store, complete_store] = [Store::open(root.path())?, Store::open(root.path())?];
    let barrier = Arc::new(Barrier::new(2));
    let job_id = job.job_id.clone();
    let cancel_start = Arc::clone(&barrier);
    let cancel_id = job_id.clone();
    let cancel = thread::spawn(move || {
        cancel_start.wait();
        cancel_store.cancel_job(&cancel_id, "race cancel".to_owned())
    });
    let complete_start = Arc::clone(&barrier);
    let complete = thread::spawn(move || {
        complete_start.wait();
        complete_store.complete_job(&job_id, json!({"completed": true}), None, None)
    });
    let _ = cancel
        .join()
        .map_err(|_| IoError::other("cancel thread panicked"))??;
    let _ = complete
        .join()
        .map_err(|_| IoError::other("complete thread panicked"))??;
    let final_record = Store::open(root.path())?
        .get_job(&job.job_id)?
        .ok_or_else(|| IoError::other("race job missing"))?;
    match final_record.status {
        JobStatus::Cancelled => {
            assert!(final_record.cancel_requested);
            assert!(final_record.result.is_none());
        }
        JobStatus::Completed => {
            assert!(!final_record.cancel_requested);
            assert_eq!(final_record.result, Some(json!({"completed": true})));
        }
        other => {
            return Err(IoError::other(format!("incoherent race status: {other:?}")).into());
        }
    }
    Ok(())
}

#[test]
fn independent_store_retry_reuses_id_and_increments_once() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let setup = Store::open(root.path())?;
    let first = setup.resolve_or_register(
        NewJob {
            idempotency_key: Some("retry-key".to_owned()),
            ..new_job(workspace.path())
        },
        false,
    )?;
    let _ = setup.mark_timed_out_unknown(&first.record.job_id, "timeout".to_owned(), None, None)?;
    let stores = [Store::open(root.path())?, Store::open(root.path())?];
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for store in stores {
        let start = Arc::clone(&barrier);
        let request = NewJob {
            idempotency_key: Some("retry-key".to_owned()),
            ..new_job(workspace.path())
        };
        handles.push(thread::spawn(move || {
            start.wait();
            store.resolve_or_register(request, true)
        }));
    }
    let mut registrations = Vec::new();
    for handle in handles {
        registrations.push(
            handle
                .join()
                .map_err(|_| IoError::other("retry thread panicked"))??,
        );
    }
    assert!(
        registrations
            .iter()
            .all(|registration| registration.record.job_id == first.record.job_id)
    );
    assert!(
        registrations
            .iter()
            .all(|registration| registration.record.attempt_count == 2)
    );
    assert_eq!(
        registrations
            .iter()
            .filter(|registration| registration.should_dispatch)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn historical_duplicate_migration_preserves_rows_and_selects_one_owner() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let jobs_dir = root.path().join("jobs");
    test_fs::create_dir_all(&jobs_dir)?;
    let database = jobs_dir.join("jobs.db");
    let connection = Connection::open(&database)?;
    connection.execute_batch(
        "CREATE TABLE jobs (job_id TEXT PRIMARY KEY, task_type TEXT NOT NULL, status TEXT NOT NULL, outcome_state TEXT NOT NULL, idempotency_key TEXT, workspace_dir TEXT NOT NULL, created_at REAL NOT NULL, updated_at REAL NOT NULL, record_json TEXT NOT NULL);",
    )?;
    for (job_id, created_at) in [("cdx-owner-old", 10.0), ("cdx-owner-new", 20.0)] {
        let mut record = build_job_record(NewJob {
            job_id: Some(job_id.to_owned()),
            idempotency_key: Some("historical-duplicate".to_owned()),
            created_at: Some(created_at),
            ..new_job(workspace.path())
        })?;
        record.status = JobStatus::Completed;
        record.outcome_state = OutcomeState::ConfirmedSuccess;
        record.completed_at = Some(created_at + 1.0);
        connection.execute(
            "INSERT INTO jobs (job_id,task_type,status,outcome_state,idempotency_key,workspace_dir,created_at,updated_at,record_json) VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                record.job_id,
                record.task_type,
                record.status.as_str(),
                record.outcome_state.as_str(),
                record.idempotency_key,
                record.workspace_dir,
                record.created_at,
                record.completed_at,
                serde_json::to_string(&record)?,
            ],
        )?;
    }
    drop(connection);

    let store = Store::open(root.path())?;
    let owner = store
        .find_job_by_idempotency_key("historical-duplicate")?
        .ok_or_else(|| IoError::other("migrated owner missing"))?;
    assert_eq!(owner.job_id, "cdx-owner-old");
    let connection = Connection::open(&database)?;
    let (rows, owners): (i64, i64) = connection.query_row(
        "SELECT COUNT(*), SUM(idempotency_owner) FROM jobs WHERE idempotency_key = 'historical-duplicate'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!((rows, owners), (2, 1));
    let unique_index: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pragma_index_list('jobs') WHERE name = 'uq_jobs_idempotency_owner' AND \"unique\" = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(unique_index, 1);
    let duplicate = store.create_job(NewJob {
        job_id: Some("cdx-new-duplicate".to_owned()),
        idempotency_key: Some("historical-duplicate".to_owned()),
        ..new_job(workspace.path())
    });
    assert!(duplicate.is_err());
    let rows_after: i64 = connection.query_row(
        "SELECT COUNT(*) FROM jobs WHERE idempotency_key = 'historical-duplicate'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(rows_after, 2);
    Ok(())
}

#[test]
fn python_json_extra_derived_fields_are_compatible() -> crate::Result<()> {
    let raw = json!({
        "job_id": "cdx-python01",
        "request_id": "cdx-python01",
        "task_type": "audit",
        "status": "completed",
        "created_at": 100.0,
        "workspace_dir": "/tmp/project",
        "metadata": {},
        "outcome_state": "confirmed_success",
        "attempt_count": 1,
        "cancel_requested": false,
        "elapsed_seconds": 1.25
    })
    .to_string();
    let record: JobRecord = deserialize(&raw, "Python job record")?;
    assert_eq!(record.job_id, "cdx-python01");
    assert_eq!(record.status, JobStatus::Completed);
    Ok(())
}

#[test]
fn reads_fixture_created_by_python_store_with_nonstring_metadata() -> crate::Result<()> {
    let root = tempdir()?;
    copy_python_fixture(root.path())?;
    let store = Store::open(root.path())?;
    let job = store
        .get_job("cdx-pythonfx")?
        .ok_or_else(|| IoError::other("missing Python-created job"))?;
    assert_eq!(job.metadata.get("text").map(String::as_str), Some("kept"));
    assert!(!job.metadata.contains_key("numeric"));
    let handoffs = store.list_handoffs(HandoffFilter {
        recipient: Some(TriadRole::Argon),
        status: Some(HandoffStatus::Pending),
        workspace_dir: None,
        limit: 10,
    })?;
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0].spec_or_payload, "keep");
    let resolved = store.list_handoffs(HandoffFilter {
        recipient: Some(TriadRole::Argon),
        status: Some(HandoffStatus::Completed),
        workspace_dir: None,
        limit: 10,
    })?;
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].spec_or_payload, "python-owned");
    assert_eq!(
        resolved[0].verdict(),
        Some(crate::storage::AuditVerdict::Warn)
    );
    Ok(())
}

fn copy_python_fixture(target: &Path) -> crate::Result<()> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/storage/fixtures/python-state");
    for (directory, database) in [("jobs", "jobs.db"), ("handoffs", "handoffs.db")] {
        let destination = target.join(directory);
        test_fs::create_dir_all(&destination)?;
        let copied = test_fs::copy(
            fixture.join(directory).join(database),
            destination.join(database),
        )?;
        if copied == 0 {
            return Err(IoError::other("Python fixture copy was empty").into());
        }
    }
    Ok(())
}

#[test]
fn verdict_and_pruning_preserve_active_records() -> crate::Result<()> {
    let root = tempdir()?;
    let workspace = tempdir()?;
    let store = Store::open(root.path())?;
    let active = store.create_job(NewJob {
        idempotency_key: Some("active".to_owned()),
        ..new_job(workspace.path())
    })?;
    for index in 0..5 {
        let job = store.create_job(NewJob {
            job_id: Some(format!("cdx-terminal{index}")),
            idempotency_key: Some(format!("terminal-{index}")),
            ..new_job(workspace.path())
        })?;
        let completed = store.complete_job(&job.job_id, json!({"index": index}), None, None)?;
        assert!(completed.is_some());
    }
    assert_eq!(store.prune_completed_jobs(2, true)?, 3);
    assert!(store.get_job(&active.job_id)?.is_some());

    let pending = store.create_handoff(NewHandoff {
        sender: TriadRole::Codex,
        recipient: TriadRole::Argon,
        title: "pending".to_owned(),
        spec_or_payload: "keep".to_owned(),
        workspace_dir: workspace.path().to_path_buf(),
        priority: HandoffPriority::Normal,
        alexandria_ref: None,
    })?;
    let mut resolved_ids = Vec::new();
    for index in 0..4 {
        let ticket = store.create_handoff(NewHandoff {
            sender: TriadRole::Codex,
            recipient: TriadRole::Argon,
            title: format!("resolved-{index}"),
            spec_or_payload: "review".to_owned(),
            workspace_dir: workspace.path().to_path_buf(),
            priority: HandoffPriority::High,
            alexandria_ref: None,
        })?;
        let resolved = store
            .resolve_handoff(
                &ticket.ticket_id,
                HandoffStatus::Completed,
                format!("reviewed {index}"),
                crate::storage::AuditVerdict::Pass,
                None,
            )?
            .ok_or_else(|| IoError::other("missing resolved handoff"))?;
        assert_eq!(resolved.verdict(), Some(crate::storage::AuditVerdict::Pass));
        resolved_ids.push(resolved.ticket_id);
    }
    assert_eq!(store.prune_resolved_handoffs(2, true)?, 2);
    assert!(store.get_handoff(&pending.ticket_id)?.is_some());
    let mut retained = 0;
    for ticket_id in resolved_ids {
        if store.get_handoff(&ticket_id)?.is_some() {
            retained += 1;
        }
    }
    assert_eq!(retained, 2);
    Ok(())
}
