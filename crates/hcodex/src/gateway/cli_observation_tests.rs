use super::*;
use crate::routing::{ExecutionPhase, ExecutionProgress};

fn job(workspace: &Path, id: &str) -> NewJob {
    NewJob {
        task_type: "claude_code_craft".into(),
        workspace_dir: workspace.into(),
        model: Some("gemini-4-argon-high".into()),
        metadata: BTreeMap::new(),
        job_id: Some(id.into()),
        idempotency_key: None,
        provider: Some("antigravity".into()),
        engine: Some("antigravity_cli".into()),
        created_at: None,
    }
}

#[tokio::test]
async fn live_cli_session_admission_commits_before_ack_and_rejects_a_cancelled_job()
-> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let gateway = Gateway::new(directory.path().into());
    let first = job(directory.path(), "live-observer");
    let cancelled = job(directory.path(), "cancelled-observer");
    gateway
        .database(move |store| {
            store.create_job(first)?;
            store.create_job(cancelled)?;
            store.cancel_job("cancelled-observer", "fixture cancellation".into())?;
            Ok(())
        })
        .await?;

    let progress = ExecutionProgress {
        engine: Engine::AntigravityCli,
        phase: ExecutionPhase::AwaitingInput,
        session_id: Some("owned-cli-session".into()),
        observed_model: Some("gemini-4-argon-high".into()),
        sequence: 1,
        step_index: None,
        observed_at: verified_at_utc(),
        stdout_bytes: 50,
        usage: None,
    };
    gateway.execution_observer("live-observer".into())(progress.clone()).await?;
    let record = Store::open(directory.path())?
        .get_job("live-observer")?
        .ok_or("live job not persisted")?;
    assert_eq!(record.status, JobStatus::Running);
    assert_eq!(record.session_id, progress.session_id);
    assert!(record.result.is_none());
    assert!(
        gateway.execution_observer("cancelled-observer".into())(progress)
            .await
            .is_err()
    );

    let foreign = Gateway::new(directory.path().into());
    foreign
        .call(
            "codex_job_status",
            json!({"job_id":"live-observer","cancel":true}),
        )
        .await?;
    assert_eq!(
        Store::open(directory.path())?
            .get_job("live-observer")?
            .ok_or("live job missing after foreign cancellation")?
            .status,
        JobStatus::Running
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn cached_connections_reject_replacing_a_database_at_the_same_path() -> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let gateway = Gateway::new(directory.path().into());
    gateway.database(|_| Ok(())).await?;

    let jobs_db = directory.path().join("jobs/jobs.db");
    std::fs::rename(&jobs_db, directory.path().join("replaced.db"))?;
    std::fs::File::create(jobs_db)?;
    let error = gateway
        .clone()
        .database(|_| Ok(()))
        .await
        .err()
        .ok_or("cached database replacement unexpectedly succeeded")?;
    assert!(error.to_string().contains("database identity changed"));
    Ok(())
}

#[tokio::test]
async fn background_wrapper_does_not_settle_an_existing_provider_record() -> crate::Result<()> {
    let directory = tempfile::tempdir()?;
    let gateway = Gateway::new(directory.path().into());
    let initial = job(directory.path(), "existing-cli-job");
    gateway
        .database(move |store| store.create_job(initial))
        .await?;
    for response in [
        Ok(json!({"job_id":"existing-cli-job","deduplicated_existing_request":true})),
        Ok(
            json!({"job_id":"existing-cli-job","result_persisted":false,"outcome_state":"outcome_unknown"}),
        ),
        Err("different request identity".into()),
    ] {
        gateway
            .persist_background_outcome(
                "existing-cli-job",
                "claude_code_craft",
                &json!({}),
                response,
            )
            .await?;
    }
    let saved = gateway
        .database(|store| store.get_job("existing-cli-job"))
        .await?
        .ok_or("provider job missing")?;
    assert_eq!(saved.status, JobStatus::Running);
    assert!(saved.result.is_none());
    Ok(())
}
