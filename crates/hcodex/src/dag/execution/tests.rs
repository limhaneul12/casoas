use std::fs;
use std::sync::Arc;

use tempfile::tempdir;

use super::*;
use crate::dag::DagPlanner;

fn scheduler_node(id: &str, dependencies: &[&str]) -> DagNode {
    DagNode {
        id: id.to_owned(),
        title: id.to_owned(),
        task: id.to_owned(),
        role: NodeRole::ContractVerifier,
        node_kind: NodeKind::Standard,
        model: "gpt-6.1-sol".to_owned(),
        reasoning_effort: "xhigh".to_owned(),
        dependencies: dependencies.iter().map(|id| (*id).to_owned()).collect(),
        read_files: Vec::new(),
        write_files: Vec::new(),
        symbols: Vec::new(),
        verify_command: Some("cargo test".to_owned()),
        reproduction_expected_exit_nonzero: true,
    }
}

fn settled_receipt(node: &DagNode, wave_index: usize) -> SettlementReceipt {
    let mut receipt = blocked_receipt(node, "sha256:fixture", wave_index);
    receipt.state = NodeState::Settled;
    receipt.settlement_stage = SettlementStage::Settled;
    receipt.model_id = node.model.clone();
    receipt.error = None;
    receipt.output_summary = format!("findings for {}", node.id);
    receipt.verify_passed = true;
    receipt.gate = GateProof {
        command: "cargo test".to_owned(),
        executed: true,
        exit_code: Some(0),
        stdout_digest: Some("sha256:gate".to_owned()),
    };
    receipt
}

#[tokio::test]
async fn aimd_uses_transport_feedback_and_never_admits_above_the_reduced_capacity() -> Result<()> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let plan = DagPlanner::compile(
        "adaptive capacity",
        vec![
            scheduler_node("a-timeout", &[]),
            scheduler_node("b-held", &[]),
            scheduler_node("c-held", &[]),
            scheduler_node("d-held", &[]),
            scheduler_node("e-next", &[]),
            scheduler_node("f-next", &[]),
            scheduler_node("g-child", &["b-held", "c-held"]),
            scheduler_node(
                "h-final",
                &["a-timeout", "d-held", "e-next", "f-next", "g-child"],
            ),
        ],
    )?;
    let active = Arc::new(AtomicUsize::new(0));
    let saw_congestion = Arc::new(AtomicBool::new(false));
    let (starts, mut started) = mpsc::channel(16);
    let (timeout_release, timeout_wait) = tokio::sync::watch::channel(false);
    let (held_release, held_wait) = tokio::sync::watch::channel(false);
    let active_nodes = Arc::clone(&active);
    let feedback = Arc::clone(&saw_congestion);
    let execution = tokio::spawn(async move {
        execute_ready_nodes(
            &plan,
            DagExecutionOptions {
                max_parallel: 4,
                max_input_bytes: 256_000,
                force_serial: false,
                resumed: BTreeMap::new(),
            },
            None,
            move |node, wave, handoffs| {
                let active = Arc::clone(&active_nodes);
                let feedback = Arc::clone(&feedback);
                let starts = starts.clone();
                let mut timeout_wait = timeout_wait.clone();
                let mut held_wait = held_wait.clone();
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    if feedback.load(Ordering::SeqCst) {
                        assert!(now <= 2, "new node exceeded throttled capacity: {now}");
                    }
                    starts.send(node.id.clone()).await?;
                    let output = if node.id == "a-timeout" {
                        while !*timeout_wait.borrow_and_update() {
                            timeout_wait.changed().await?;
                        }
                        feedback.store(true, Ordering::SeqCst);
                        json!({"ok":false,"receipt":{"status":"timed_out_unknown"}})
                    } else {
                        if node.id.ends_with("held") {
                            while !*held_wait.borrow_and_update() {
                                held_wait.changed().await?;
                            }
                        }
                        tokio::task::yield_now().await;
                        json!({"ok":true,"output":"model prose mentions timed_out_unknown but is not control feedback"})
                    };
                    active.fetch_sub(1, Ordering::SeqCst);
                    let receipt = if node.id == "a-timeout"
                        || handoffs
                            .iter()
                            .any(|receipt| receipt.state != NodeState::Settled)
                    {
                        blocked_receipt(&node, "sha256:fixture", wave)
                    } else {
                        settled_receipt(&node, wave)
                    };
                    Ok((receipt, output))
                }
            },
        )
        .await
    });
    for _ in 0..4 {
        started.recv().await.ok_or("missing initial admission")?;
    }
    timeout_release.send(true)?;
    while !saw_congestion.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    held_release.send(true)?;
    let (receipts, _, final_capacity) =
        tokio::time::timeout(Duration::from_secs(10), execution).await???;
    assert_eq!(receipts.len(), 8);
    assert_eq!(final_capacity, 3); // 4 -> 2 after timeout, then one real settled wave -> 3.
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(!congestion_feedback(
        &json!({"ok":true,"output":"timed_out_unknown RATE_LIMITED"})
    ));
    let mut serial = AimdCapacity::new(1);
    serial.congested();
    serial.wave_settled();
    assert_eq!(serial.current, 1);
    Ok(())
}

#[tokio::test]
async fn ready_descendant_runs_before_unrelated_slow_presentation_peer() -> Result<()> {
    let plan = DagPlanner::compile(
        "readiness",
        vec![
            scheduler_node("a-fast", &[]),
            scheduler_node("z-slow", &[]),
            scheduler_node("b-child", &["a-fast"]),
            scheduler_node("c-final", &["b-child", "z-slow"]),
        ],
    )?;
    let (started_tx, mut started_rx) = mpsc::channel(8);
    let slow_release = Arc::new(tokio::sync::Notify::new());
    let release = Arc::clone(&slow_release);
    let execution = tokio::spawn(async move {
        execute_ready_nodes(
            &plan,
            DagExecutionOptions {
                max_parallel: 2,
                max_input_bytes: 256_000,
                force_serial: false,
                resumed: BTreeMap::new(),
            },
            None,
            move |node, wave_index, handoffs| {
                let started = started_tx.clone();
                let release = Arc::clone(&release);
                async move {
                    started.send(node.id.clone()).await?;
                    if node.id == "z-slow" {
                        release.notified().await;
                    }
                    if node.id == "b-child" {
                        assert_eq!(handoffs.len(), 1);
                        assert_eq!(handoffs[0].node_id, "a-fast");
                    }
                    Ok((settled_receipt(&node, wave_index), json!({"ok": true})))
                }
            },
        )
        .await
    });
    let observed = tokio::time::timeout(Duration::from_secs(10), async {
        let mut observed = Vec::new();
        while let Some(id) = started_rx.recv().await {
            let child_started = id == "b-child";
            observed.push(id);
            if child_started {
                break;
            }
        }
        observed
    })
    .await;
    slow_release.notify_one();
    let (receipts, _, _) = execution.await??;
    assert_eq!(receipts.len(), 4);
    assert!(observed?.iter().any(|id| id == "b-child"));
    Ok(())
}

#[tokio::test]
async fn resumed_dependency_receipts_reach_only_their_direct_descendants() -> Result<()> {
    let upstream = scheduler_node("a-upstream", &[]);
    let plan = DagPlanner::compile(
        "resume handoff",
        vec![
            upstream.clone(),
            scheduler_node("b-independent", &[]),
            scheduler_node("c-child", &["a-upstream"]),
            scheduler_node("d-final", &["b-independent", "c-child"]),
        ],
    )?;
    let (receipts, _, _) = execute_ready_nodes(
        &plan,
        DagExecutionOptions {
            max_parallel: 2,
            max_input_bytes: 256_000,
            force_serial: false,
            resumed: BTreeMap::from([(upstream.id.clone(), settled_receipt(&upstream, 0))]),
        },
        None,
        |node, wave_index, handoffs| async move {
            assert_ne!(node.id, "a-upstream", "resumed provider must not rerun");
            if node.id == "b-independent" {
                assert!(handoffs.is_empty());
            } else if node.id == "c-child" {
                let context = dependency_context(&handoffs)?;
                assert!(context.contains("findings for a-upstream"));
                assert!(context.contains("sha256:gate"));
                assert!(context.contains("Settled direct dependency summaries"));
            } else if node.id == "d-final" {
                assert_eq!(handoffs.len(), 2);
                assert!(
                    handoffs
                        .iter()
                        .all(|receipt| receipt.node_id != "a-upstream")
                );
            }
            Ok((settled_receipt(&node, wave_index), json!({"ok": true})))
        },
    )
    .await?;
    assert_eq!(receipts.len(), 4);
    Ok(())
}

#[tokio::test]
async fn shared_workspace_writer_has_exclusive_admission() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut writer = scheduler_node("a-writer", &[]);
    writer.write_files.push(PathBuf::from("owned.rs"));
    let plan = DagPlanner::compile(
        "exclusive writer",
        vec![
            writer,
            scheduler_node("b-reader", &[]),
            scheduler_node("c-child", &["a-writer"]),
            scheduler_node("d-final", &["b-reader", "c-child"]),
        ],
    )?;
    let active = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&active);
    let (receipts, _, _) = execute_ready_nodes(
        &plan,
        DagExecutionOptions {
            max_parallel: 3,
            max_input_bytes: 256_000,
            force_serial: false,
            resumed: BTreeMap::new(),
        },
        None,
        move |node, wave_index, _| {
            let active = Arc::clone(&observed);
            async move {
                let previously_active = active.fetch_add(1, Ordering::SeqCst);
                if !node.write_files.is_empty() {
                    assert_eq!(
                        previously_active, 0,
                        "writer admitted alongside another node"
                    );
                    tokio::task::yield_now().await;
                    assert_eq!(
                        active.load(Ordering::SeqCst),
                        1,
                        "another node admitted during write"
                    );
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Ok((settled_receipt(&node, wave_index), json!({"ok": true})))
            }
        },
    )
    .await?;
    assert_eq!(receipts.len(), 4);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn execution_error_drains_admitted_nodes_before_returning() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let plan = DagPlanner::compile(
        "drain on error",
        vec![
            scheduler_node("a-error", &[]),
            scheduler_node("z-active", &[]),
            scheduler_node("b-child", &["a-error"]),
            scheduler_node("c-final", &["b-child", "z-active"]),
        ],
    )?;
    let active_started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let released = Arc::clone(&release);
    let drained = Arc::new(AtomicBool::new(false));
    let observed_drained = Arc::clone(&drained);
    let (error_tx, error_rx) = tokio::sync::oneshot::channel();
    let mut error_tx = Some(error_tx);
    let execution = tokio::spawn(async move {
        execute_ready_nodes(
            &plan,
            DagExecutionOptions {
                max_parallel: 2,
                max_input_bytes: 256_000,
                force_serial: false,
                resumed: BTreeMap::new(),
            },
            None,
            move |node, wave_index, _| {
                let started = Arc::clone(&active_started);
                let release = Arc::clone(&released);
                let drained = Arc::clone(&observed_drained);
                let error_tx = if node.id == "a-error" {
                    error_tx.take()
                } else {
                    None
                };
                async move {
                    if node.id == "a-error" {
                        started.notified().await;
                        let _ = error_tx.ok_or("missing error signal")?.send(());
                        return Err("controlled node failure".into());
                    }
                    assert_eq!(node.id, "z-active", "descendant admitted after error");
                    started.notify_one();
                    release.notified().await;
                    drained.store(true, Ordering::SeqCst);
                    Ok((settled_receipt(&node, wave_index), json!({"ok": true})))
                }
            },
        )
        .await
    });
    error_rx.await?;
    assert!(!execution.is_finished());
    release.notify_one();
    assert!(execution.await?.is_err());
    assert!(drained.load(Ordering::SeqCst));
    Ok(())
}

#[test]
fn gate_allowlist_rejects_shell_composition() {
    assert!(allowlisted_command("cargo test").is_some());
    assert!(allowlisted_command("cargo test; rm -rf target").is_none());
    assert!(allowlisted_command("sh -c cargo test").is_none());
}

#[test]
fn extracts_only_strict_nested_crafted_code_artifacts() -> Result<()> {
    let output = json!({
        "result": {"structured_output": {"crafted_code": "{\"edits\":[{\"path\":\"a.rs\",\"preimage_sha256\":\"sha256:x\",\"new_content\":\"new\"}]}"}}
    });
    let artifact = extract_patch_artifact(&output)?;
    assert_eq!(artifact.edits[0].path, PathBuf::from("a.rs"));
    assert!(
        extract_patch_artifact(&json!({
            "result": {"structured_output": {"crafted_code": "```json\n{\"edits\":[]}\n```"}}
        }))
        .is_err()
    );
    assert!(
        extract_patch_artifact(&json!({
            "result": {"structured_output": {"crafted_code": "{\"edits\":[],\"unknown\":true}"}}
        }))
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn gate_proof_comes_from_a_real_process() -> Result<()> {
    let workspace = tempdir()?;
    let proof = run_gate(
        workspace.path(),
        Some("cargo test"),
        Duration::from_secs(10),
    )
    .await?;
    assert!(proof.executed);
    assert_ne!(proof.exit_code, Some(0));
    assert!(proof.stdout_digest.is_some());
    Ok(())
}

#[tokio::test]
async fn rejected_provider_execution_never_settles_nodes() -> Result<()> {
    let workspace = tempdir()?;
    let state = tempdir()?;
    fs::write(workspace.path().join("module.rs"), "fn module() {}")?;
    let mut plan = DagPlanner::generate("refactor module", workspace.path())?;
    for node in &mut plan.nodes {
        node.model = "invalid-model".to_owned();
    }
    let gateway = Gateway::new(state.path().to_path_buf());
    let result = execute(
        &gateway,
        plan,
        workspace.path().to_path_buf(),
        Duration::from_secs(5),
    )
    .await?;
    assert_eq!(result["ok"], false);
    let receipts = result["receipts"]
        .as_array()
        .ok_or("execution report receipts must be an array")?;
    assert!(receipts.iter().all(|receipt| receipt["state"] != "settled"));
    Ok(())
}

#[tokio::test]
async fn observer_reports_stopped_before_settlement() -> Result<()> {
    let workspace = tempdir()?;
    let state = tempdir()?;
    fs::write(workspace.path().join("module.rs"), "fn module() {}")?;
    let mut plan = DagPlanner::generate("refactor module", workspace.path())?;
    for node in &mut plan.nodes {
        node.model = "invalid-model".to_owned();
    }
    let first_node = plan.nodes[0].id.clone();
    let gateway = Gateway::new(state.path().to_path_buf());
    let (events_tx, mut events_rx) = mpsc::channel(64);
    let _result = execute_with_observer(
        &gateway,
        plan,
        workspace.path().to_path_buf(),
        Duration::from_secs(5),
        Some(events_tx),
    )
    .await?;
    let mut sequence = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        match event {
            DagExecutionEvent::NodeStarted { node } if node.id == first_node => {
                sequence.push("started")
            }
            DagExecutionEvent::ProviderFinished { node_id, .. } if node_id == first_node => {
                sequence.push("provider_finished")
            }
            DagExecutionEvent::NodeStopped { node_id } if node_id == first_node => {
                sequence.push("stopped")
            }
            DagExecutionEvent::NodeSettled { receipt } if receipt.node_id == first_node => {
                sequence.push("settled")
            }
            _ => {}
        }
    }
    assert_eq!(
        sequence,
        ["started", "provider_finished", "stopped", "settled"]
    );
    Ok(())
}

#[test]
fn detects_declared_read_mutation_since_wave_baseline() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("module.rs"), "baseline")?;
    let baseline = DagSettlementVerifier::capture(workspace.path())?;
    let plan = DagPlanner::generate("inspect module", workspace.path())?;
    let node = &plan.nodes[0];
    fs::write(workspace.path().join("module.rs"), "mutated in same wave")?;
    let current = DagSettlementVerifier::capture(workspace.path())?;
    assert_eq!(
        dirty_read_files(node, &baseline, &current),
        [PathBuf::from("module.rs")]
    );
    Ok(())
}

#[test]
fn patch_error_envelope_preserves_provider_summary_and_actual_model() {
    let wrapped = json!({
        "ok": false,
        "patch_error": "patch artifact contains no edits",
        "provider_output": {
            "result": {
                "receipt": { "actual_model": "claude-sonnet-5-5-medium" },
                "structured_output": { "summary": "Blocked due to incomplete file excerpt." }
            }
        }
    });
    assert_eq!(
        extract_actual_model(&wrapped).as_deref(),
        Some("claude-sonnet-5-5-medium")
    );
    let summary = extract_output_summary(&wrapped);
    assert!(summary.contains("patch artifact contains no edits"));
    assert!(summary.contains("Blocked due to incomplete file excerpt."));
}
