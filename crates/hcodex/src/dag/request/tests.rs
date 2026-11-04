use tempfile::tempdir;

use super::*;
use crate::dag::SettlementStage;

fn checkpoint_receipt() -> SettlementReceipt {
    SettlementReceipt {
        node_id: "node-a".to_owned(),
        state: NodeState::Settled,
        settlement_stage: SettlementStage::Settled,
        role: NodeRole::InterfaceBuilder,
        node_kind: NodeKind::Standard,
        wave_index: 1,
        model_id: "claude-sonnet-5-5-medium".to_owned(),
        duration_seconds: 1.0,
        pre_manifest_digest: "sha256:before".to_owned(),
        stopped_manifest_digest: "sha256:after".to_owned(),
        post_manifest_digest: "sha256:after".to_owned(),
        changed_files: vec![PathBuf::from("a.rs")],
        mutated_files: vec![PathBuf::from("a.rs")],
        out_of_scope_writes: Vec::new(),
        dirty_read_files: Vec::new(),
        verify_passed: true,
        reproduction_observed: true,
        output_summary: "applied".to_owned(),
        error: None,
        gate: super::super::GateProof {
            command: "cargo test".to_owned(),
            executed: true,
            exit_code: Some(0),
            stdout_digest: Some("sha256:gate".to_owned()),
        },
    }
}

#[tokio::test]
async fn rejected_campaign_is_durably_failed_with_its_result() -> Result<()> {
    let workspace = tempdir()?;
    let state = tempdir()?;
    let gateway = Gateway::new(state.path().to_path_buf());
    let response = handle_request(
        &gateway,
        &json!({
            "campaign_goal": "invalid one-node graph",
            "workspace_dir": workspace.path(),
            "request_id": "cdx-rejected-dag",
            "plan_only": true,
            "nodes": [{"node_id": "only", "task": "no dispatch"}],
        }),
    )
    .await?;
    assert_eq!(response["ok"], false);
    let saved = gateway
        .database(|store| store.get_job("cdx-rejected-dag"))
        .await?
        .ok_or("campaign missing")?;
    assert_eq!(saved.status, crate::storage::JobStatus::Failed);
    assert_eq!(saved.result.as_ref(), Some(&response));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_campaign_stops_admission_and_reaps_active_provider_groups() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempdir()?;
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let binary = temporary.path().join("slow-codex");
    let pid_file = temporary.path().join("pids");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\necho $$ >> '{}'\ncat >/dev/null\nsleep 30\n",
            pid_file.display(),
        ),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let gateway = Gateway::new(temporary.path().join("state")).with_codex_binary(binary);
    let execution_gateway = gateway.clone();
    let request = json!({
        "campaign_goal": "controlled cancellation",
        "workspace_dir": workspace,
        "request_id": "cdx-cancel-dag",
        "plan_only": false,
        "timeout_seconds": 30.0,
        "spawn_plan": {"plan_summary": "two active readers", "hazard_acknowledgment": true, "isolation_mode": "direct", "max_parallel_per_wave": 2},
        "nodes": [
            {"node_id":"a", "role":"adversarial_auditor", "task":"slow a", "preferred_model":"gpt-6.1-sol"},
            {"node_id":"z", "role":"adversarial_auditor", "task":"slow z", "preferred_model":"gpt-6.1-sol"},
            {"node_id":"b", "role":"adversarial_auditor", "task":"must not dispatch", "depends_on":["a"], "preferred_model":"gpt-6.1-sol"},
            {"node_id":"final", "role":"adversarial_auditor", "task":"must not dispatch", "depends_on":["b","z"], "preferred_model":"gpt-6.1-sol"}
        ],
    });
    let execution = tokio::spawn(async move { handle_request(&execution_gateway, &request).await });
    let readiness = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let pids = std::fs::read_to_string(&pid_file).unwrap_or_default();
            if pids.lines().count() == 2 {
                return pids;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let cancellation = gateway
        .call(
            "codex_job_status",
            json!({"job_id":"cdx-cancel-dag", "cancel":true}),
        )
        .await?;
    assert_eq!(cancellation["job"]["status"], "cancelled");
    let outcome = tokio::time::timeout(Duration::from_secs(10), execution).await;
    gateway.shutdown().await?;
    assert!(outcome??.is_err());
    for pid in readiness?.lines() {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = tokio::process::Command::new("/bin/kill")
                .args(["-0", pid])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await?;
            if !status.success() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "cancelled DAG provider remained alive"
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
    assert_eq!(std::fs::read_to_string(pid_file)?.lines().count(), 2);
    Ok(())
}

#[tokio::test]
async fn checkpoint_storage_failure_signals_the_owned_campaign() -> Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let temporary = tempdir()?;
    let gateway = Gateway::new(temporary.path().join("state"));
    let mut broken_storage = gateway.clone();
    broken_storage.state_root = temporary.path().join("file-instead-of-state-directory");
    std::fs::write(&broken_storage.state_root, "controlled storage failure")?;
    let (events, receiver) = mpsc::channel(1);
    events
        .send(DagExecutionEvent::NodeSettled {
            receipt: checkpoint_receipt(),
        })
        .await?;
    drop(events);
    let observed = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&observed);
    let owner = gateway.clone();
    let result = gateway
        .supervise("cdx-checkpoint-failure".to_owned(), async move {
            let persisting = persist_node_checkpoints(
                broken_storage,
                receiver,
                "cdx-checkpoint-failure".to_owned(),
                temporary.path().to_path_buf(),
                "plan".to_owned(),
                "config".to_owned(),
                None,
                None,
            );
            let failure =
                cancel_on_checkpoint_failure(&owner, "cdx-checkpoint-failure", persisting);
            let active_work = async {
                crate::supervisor::cancelled().await;
                cancelled.store(true, Ordering::SeqCst);
            };
            let (result, ()) = tokio::join!(failure, active_work);
            result
        })
        .await;
    assert!(result.is_err());
    assert!(observed.load(Ordering::SeqCst));
    gateway.shutdown().await?;
    Ok(())
}

#[test]
fn strict_request_rejects_unknown_fields_and_maps_all_node_fields() -> Result<()> {
    let bad = json!({"campaign_goal":"x","unknown":true});
    assert!(serde_json::from_value::<DagRequest>(bad).is_err());
    let request: DagRequest = serde_json::from_value(json!({
        "campaign_goal":"x",
        "nodes":[
            {"node_id":"repro","role":"reproduction_prober","node_kind":"reproduction","task":"reproduce","deliverable":"trace","read_files":["a.rs"],"write_files":[],"symbols":["run"],"depends_on":[],"verify_cmd":"cargo test","stop_when":"observed","preferred_model":"gpt-6.1-sol","reasoning_effort":"xhigh","reproduction_expected_exit_nonzero":true},
            {"node_id":"mapper","role":"interface_contractor","node_kind":"standard","task":"map","deliverable":"map","read_files":["a.rs"],"write_files":[],"symbols":[],"depends_on":[],"verify_cmd":"cargo test","stop_when":"mapped","preferred_model":"gpt-6-luna","reasoning_effort":"max","reproduction_expected_exit_nonzero":false}
        ]
    }))?;
    let nodes = request.nodes.ok_or("nodes")?;
    let node = convert_node(&nodes[0]);
    assert_eq!(node.role, NodeRole::ContractVerifier);
    assert!(node.task.contains("Node kind: Reproduction"));
    assert_eq!(node.verify_command.as_deref(), Some("cargo test"));
    let mapper = convert_node(&nodes[1]);
    assert_eq!(mapper.role, NodeRole::BoundaryMapper);
    assert_eq!(mapper.model, "gpt-6-luna");
    Ok(())
}

#[tokio::test]
async fn resume_requires_matching_plan_config_and_source_identity() -> Result<()> {
    let workspace = tempdir()?;
    let state = tempdir()?;
    std::fs::write(workspace.path().join("a.rs"), "fn a() {}")?;
    let gateway = Gateway::new(state.path().to_path_buf());
    let args =
        json!({"campaign_goal":"refactor","workspace_dir":workspace.path(),"plan_only":true});
    let first = handle_request(&gateway, &args).await?;
    std::fs::write(workspace.path().join("a.rs"), "fn changed() {}")?;
    let resumed = json!({"campaign_goal":"refactor","workspace_dir":workspace.path(),"plan_only":true,"resume_dag_id":first["dag_id"]});
    assert!(handle_request(&gateway, &resumed).await.is_err());
    Ok(())
}

#[tokio::test]
async fn serial_spawn_plan_explicitly_isolates_declared_file_hazards() -> Result<()> {
    let workspace = tempdir()?;
    let state = tempdir()?;
    std::fs::write(workspace.path().join("same.rs"), "fn same() {}")?;
    let gateway = Gateway::new(state.path().to_path_buf());
    let nodes = json!([
        {"node_id":"a","task":"write a","write_files":["same.rs"]},
        {"node_id":"b","task":"write b","write_files":["same.rs"]},
        {"node_id":"c","task":"check","depends_on":["a","b"]},
        {"node_id":"d","task":"fan in","depends_on":["c"]}
    ]);
    let direct = handle_request(
        &gateway,
        &json!({"campaign_goal":"conflict","workspace_dir":workspace.path(),"nodes":nodes.clone(),"plan_only":true}),
    )
    .await?;
    assert_eq!(direct["ok"], false);
    let serial = handle_request(
        &gateway,
        &json!({"campaign_goal":"conflict","workspace_dir":workspace.path(),"nodes":nodes,"plan_only":true,"spawn_plan":{"plan_summary":"serialize writers","isolation_mode":"serial"}}),
    )
    .await?;
    assert_eq!(serial["ok"], true);
    assert_eq!(serial["compiled_plan"]["blocking_hazards_count"], 0);
    assert_eq!(serial["aimd_final_concurrency"], 1);
    Ok(())
}

#[tokio::test]
async fn settled_node_events_are_persisted_as_campaign_checkpoints() -> Result<()> {
    let workspace = tempdir()?;
    let state = tempdir()?;
    let gateway = Gateway::new(state.path().to_path_buf());
    let receipt = checkpoint_receipt();
    let (sender, receiver) = mpsc::channel(2);
    sender
        .send(DagExecutionEvent::NodeSettled {
            receipt: receipt.clone(),
        })
        .await?;
    drop(sender);
    let (observer, mut observed) = mpsc::channel(1);
    observer
        .send(DagExecutionEvent::WaveFinished { wave_index: 99 })
        .await?;
    let persisting = tokio::spawn(persist_node_checkpoints(
        gateway.clone(),
        receiver,
        "campaign-a".to_owned(),
        workspace.path().to_path_buf(),
        "plan-a".to_owned(),
        "config-a".to_owned(),
        Some(observer),
        None,
    ));
    // The deliberately full observer must not prevent durable settlement.
    let persistence = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let saved = gateway.database(|store| store.list_jobs(10, None)).await?;
            if !saved.is_empty() {
                return Ok::<(), crate::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    let _previous = observed.recv().await;
    persisting.await??;
    persistence??;
    let delivered = observed
        .recv()
        .await
        .ok_or("settlement notification lost")?;
    assert!(matches!(delivered, DagExecutionEvent::NodeSettled { .. }));
    let jobs = gateway
        .database(move |store| store.list_jobs(10, None))
        .await?;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].task_type, "dag_checkpoint");
    assert_eq!(
        jobs[0].metadata.get("campaign_id").map(String::as_str),
        Some("campaign-a")
    );
    let stored: SettlementReceipt = serde_json::from_value(
        jobs[0].result.as_ref().ok_or("checkpoint result")?["receipt"].clone(),
    )?;
    assert_eq!(stored, receipt);
    Ok(())
}

#[tokio::test]
async fn interactive_auto_spawn_is_derived_from_validated_plan() -> Result<()> {
    let workspace = tempdir()?;
    std::fs::write(workspace.path().join("a.rs"), "fn a() {}")?;
    std::fs::write(workspace.path().join("b.rs"), "fn b() {}")?;
    let prepared = prepare_interactive_request(&json!({
        "campaign_goal":"refactor both",
        "workspace_dir":workspace.path(),
        "plan_only":false
    }))
    .await?;
    assert_eq!(prepared["spawn_plan"]["hazard_acknowledgment"], true);
    assert_eq!(prepared["spawn_plan"]["isolation_mode"], "direct");
    assert_eq!(prepared["spawn_plan"]["max_parallel_per_wave"], 2);
    assert!(
        prepared["spawn_plan"]["plan_summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("5 nodes in 3 waves"))
    );
    Ok(())
}
