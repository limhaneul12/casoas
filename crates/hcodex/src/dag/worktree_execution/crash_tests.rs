//! Process-termination evidence from the real request/Store/Git/gate pipeline.
#![cfg(unix)]
use super::*;
use crate::dag::worktrees::{TestBoundary, TestControl};
use crate::gateway::Gateway;
use crate::routing::{
    ReceiptStatus, close_receipt, digest_output, new_receipt_with_context, validate_route,
};
use crate::runtime::{ExecutionOptions, ExecutionResult};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command as AsyncCommand;

const ORIGIN: &str = "cdx-pipeline-crash-origin";
const SENTINEL: &str = "ANCESTOR_DECISION_RESUME_SENTINEL";
const READY: &str = "HCODEX_PIPELINE_READY ";

fn phase_name(point: TestBoundary) -> &'static str {
    match point {
        TestBoundary::InitialGitAddedBeforeOwnerPin => "initial-git-added",
        TestBoundary::Preparing => "preparing",
        TestBoundary::Staged => "staged",
        TestBoundary::PartialPublication => "partial-publication",
        TestBoundary::CanonicalPublished => "canonical-published",
        TestBoundary::CanonicalGateCompleted => "canonical-gate-completed",
        TestBoundary::CheckpointCommittedBeforeFrontier => "checkpoint-before-frontier",
        TestBoundary::FrontierWrittenBeforePin => "frontier-before-pin",
        TestBoundary::FrontierPinnedBeforeNotify => "frontier-pinned",
        TestBoundary::ExportCommittedBeforeAck => "export-before-ack",
        TestBoundary::CleanupRemovedBeforeAck => "cleanup-before-ack",
    }
}

fn pause(workspace: &Path, state: &Path, calls: &Path, phase: &str) {
    println!(
        "{READY}{}",
        json!({"workspace":workspace,"state":state,"calls":calls,"phase":phase})
    );
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
}

fn arguments(workspace: &Path, id: &str) -> Value {
    let nodes = vec![
        json!({"node_id":"n0","role":"interface_builder","task":"WRITE_STAGE_ONE", "preferred_model":"claude-sonnet-5-5-medium","reasoning_effort":"medium","read_files":["src/lib.rs"],"write_files":["src/lib.rs","created.rs"],"verify_cmd":"cargo test"}),
        json!({"node_id":"n1","role":"interface_builder","task":"WRITE_STAGE_TWO", "preferred_model":"claude-sonnet-5-5-medium","reasoning_effort":"medium","read_files":["src/lib.rs","created.rs"],"write_files":["src/lib.rs","created.rs"],"verify_cmd":"cargo test","depends_on":["n0"]}),
        json!({"node_id":"n2","role":"adversarial_auditor","task":"READ_STAGE_THREE", "preferred_model":"gpt-6.1-sol","read_files":["src/lib.rs","created.rs"],"verify_cmd":"cargo test","depends_on":["n1"]}),
        json!({"node_id":"n3","role":"adversarial_auditor","task":"READ_STAGE_FOUR", "preferred_model":"gpt-6.1-sol","read_files":["src/lib.rs","created.rs"],"verify_cmd":"cargo test","depends_on":["n2"]}),
    ];
    json!({"campaign_goal":"verified cumulative source survives restart", "workspace_dir":workspace,"request_id":id,"plan_only":false,"timeout_seconds":30,"nodes":nodes,
        "spawn_plan":{"plan_summary":"two writers and two readers","hazard_acknowledgment":true,"isolation_mode":"worktree","max_parallel_per_wave":1}})
}

fn mock_gateway(state: &Path, workspace: &Path, calls: &Path, pause_phase: &str) -> Gateway {
    let state_for_provider = state.to_path_buf();
    let original = workspace.to_path_buf();
    let calls = calls.to_path_buf();
    let pause_phase = pause_phase.to_owned();
    Gateway::new(state.to_path_buf()).with_test_provider(move |request, options: &ExecutionOptions| {
        let route = validate_route(request)?;
        let node_root = options.workspace_dir.as_deref().ok_or("fixture node root missing")?;
        let writer = request.model.starts_with("claude-");
        let label = if writer {
            if request.prompt.contains("WRITE_STAGE_TWO") { "n1" } else { "n0" }
        } else if request.prompt.contains("READ_STAGE_FOUR") { "n3" } else { "n2" };
        let mut log = OpenOptions::new().create(true).append(true).open(&calls)?;
        writeln!(log, "{label}")?;
        log.sync_all()?;
        if pause_phase == "provider-admitted" && label == "n0" {
            pause(&original, &state_for_provider, &calls, "provider-admitted");
        }
        let data = if writer {
            let second = label == "n1";
            if second {
                assert_eq!(fs::read_to_string(node_root.join("src/lib.rs"))?, lib(2, false));
                assert_eq!(fs::read_to_string(node_root.join("created.rs"))?, "// from-one\n");
            }
            let manifest = DagSettlementVerifier::capture(node_root)?;
            let new_content = lib(if second { 3 } else { 2 }, false);
            let created = if second { "// from-one\n// from-two\n" } else { "// from-one\n" };
            let patch = json!({"edits":[
                {"path":"src/lib.rs","preimage_sha256":manifest.files[Path::new("src/lib.rs")],"new_content":new_content},
                {"path":"created.rs","preimage_sha256":manifest.files.get(Path::new("created.rs")).map(String::as_str).unwrap_or("sha256:absent"),"new_content":created}
            ]});
            json!({"verdict":"PASS","crafted_code":patch.to_string(),"design_highlights":[],"edge_cases_guarded":[],"handoff_notes_for_codex_audit":"fixture","summary":if second {"second source verified"}else{SENTINEL}})
        } else {
            assert_eq!(fs::read_to_string(node_root.join("src/lib.rs"))?, lib(3, false));
            assert_eq!(fs::read_to_string(node_root.join("created.rs"))?, "// from-one\n// from-two\n");
            assert!(request.prompt.contains(SENTINEL), "resolved ancestor handoff missing from actual provider prompt");
            json!({"verdict":"PASS","risk_score":0,"executive_critique":"Fixture passed.","findings":[],"failing_test_cases":[],"alexandria_decision_note":"fixture"})
        };
        let output = data.to_string();
        let mut receipt = new_receipt_with_context(&route, options.request_id.as_deref(), options.idempotency_key.clone());
        receipt.actual_model = Some(route.resolved_model);
        receipt.identity_source = Some("controlled_provider_transport_fixture".into());
        receipt.status = ReceiptStatus::Completed;
        receipt.outcome_state = "confirmed_success".into();
        receipt.output_digest = Some(digest_output(&output));
        receipt.owned_processes_closed = Some(true);
        close_receipt(&mut receipt);
        Ok(ExecutionResult {receipt,placeholder:None,output,structured_output:Some(data),session_id:Some(format!("fixture-{label}")),exit_code:Some(0)})
    })
}

#[tokio::test]
async fn pipeline_child() -> Result<()> {
    if std::env::var_os("HCODEX_PIPELINE_TEST_ROOT").is_none() {
        return Ok(());
    }
    let phase = std::env::var("HCODEX_PIPELINE_TEST_PHASE")?;
    let f = fixture(false)?;
    let calls = f._root.path().join("provider-calls");
    let gateway = mock_gateway(&f.state, &f.original, &calls, &phase);
    let workspace = f.original.clone();
    let state = f.state.clone();
    let calls_for_hook = calls.clone();
    let phase_for_hook = phase.clone();
    let control = TestControl {
        boundary: Some(Arc::new(move |node, point| {
            let name = phase_name(point);
            if name == phase_for_hook && (node == "n0" || name == "export-before-ack") {
                pause(&workspace, &state, &calls_for_hook, name);
            }
        })),
    };
    let result = crate::dag::request::handle_request_with_worktree_hook(
        &gateway,
        &arguments(&f.original, ORIGIN),
        None,
        control,
    )
    .await;
    gateway.shutdown().await?;
    let result = result?;
    let receipts = result["node_receipts"].as_array().map(|receipts| {
        receipts.iter().map(|receipt| json!({"node":receipt["node_id"],"state":receipt["state"],"error":receipt["error"],"gate":receipt["gate"]})).collect::<Vec<_>>()
    });
    Err(format!("fixture completed without reaching {phase}: {receipts:?}").into())
}

async fn bounded_stderr(reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > 1024 * 1024 {
        return Err(std::io::Error::other("fixture stderr exceeded budget"));
    }
    Ok(bytes)
}

async fn terminate_at(phase: &str, root: &Path) -> Result<Value> {
    let mut child = AsyncCommand::new(std::env::current_exe()?)
        .args([
            "--exact",
            "dag::worktree_execution::tests::crash_tests::pipeline_child",
            "--nocapture",
        ])
        .env("HCODEX_PIPELINE_TEST_ROOT", root)
        .env("HCODEX_PIPELINE_TEST_PHASE", phase)
        .env("TMPDIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().ok_or("fixture stdout missing")?;
    let stderr = child.stderr.take().ok_or("fixture stderr missing")?;
    let stderr_reader = tokio::spawn(bounded_stderr(stderr));
    let ready = tokio::time::timeout(Duration::from_secs(90), async {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            if let Some((_, body)) = line.split_once(READY) {
                return serde_json::from_str::<Value>(body).map_err(std::io::Error::other);
            }
        }
        Err(std::io::Error::other(
            "child exited without reaching requested boundary",
        ))
    })
    .await;
    if child.try_wait()?.is_none() {
        child.kill().await?;
    }
    let status = child.wait().await?;
    let stderr = stderr_reader
        .await
        .map_err(|_| "fixture stderr reader panicked")??;
    let ready = ready
        .map_err(|_| {
            format!(
                "timed out before {phase}: {}",
                String::from_utf8_lossy(&stderr)
            )
        })?
        .map_err(|error| format!("{phase}: {error}: {}", String::from_utf8_lossy(&stderr)))?;
    assert_eq!(
        status.signal(),
        Some(9),
        "fixture was not killed at {phase}: {status}"
    );
    assert_eq!(ready["phase"], phase);
    Ok(ready)
}

fn call_count(path: &Path) -> Result<usize> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value.lines().count()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

async fn resume_through_tui(gateway: &Gateway, workspace: &Path) -> Result<Value> {
    use crate::tui::{ActionHandler, LaneState, TuiAction, TuiUpdate};
    let handler =
        crate::terminal_adapter::AppHandler::new(gateway.clone(), workspace.to_path_buf())?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
    let action = handler.handle(TuiAction::Resume(Some(ORIGIN.into())), sender);
    let drain = async {
        let mut updates = Vec::new();
        while let Some(update) = receiver.recv().await {
            assert!(updates.len() < 512, "fixture UI update bound exceeded");
            updates.push(update);
        }
        updates
    };
    let (outcome, updates) = tokio::join!(action, drain);
    outcome?;
    assert!(
        updates
            .iter()
            .any(|update| matches!(update,TuiUpdate::LaneState{node_id,..} if node_id=="n0")),
        "reused settlement was not delivered to the real UI"
    );
    assert!(!updates.iter().any(|update| matches!(update,TuiUpdate::LaneState{node_id,state:LaneState::Running|LaneState::Stopped,..} if node_id=="n0")), "reused node acquired fabricated provider progress");
    let roots = gateway.database(|store| store.list_jobs(100, None)).await?;
    roots
        .into_iter()
        .find(|record| {
            record.task_type == "dag_ulw"
                && record.metadata.get("resume_dag_id").map(String::as_str) == Some(ORIGIN)
        })
        .and_then(|record| record.result)
        .ok_or_else(|| "UI resume did not persist a continuation result".into())
}

#[tokio::test]
async fn crash_boundaries_preserve_evidence_and_only_checkpoint_frontiers_resume() -> Result<()> {
    for phase in [
        "initial-git-added",
        "preparing",
        "provider-admitted",
        "staged",
        "partial-publication",
        "canonical-published",
        "canonical-gate-completed",
        "checkpoint-before-frontier",
        "frontier-before-pin",
        "frontier-pinned",
        "export-before-ack",
        "cleanup-before-ack",
    ] {
        let root = tempdir()?;
        let ready = terminate_at(phase, root.path()).await?;
        let workspace = PathBuf::from(ready["workspace"].as_str().ok_or("workspace missing")?);
        let state = PathBuf::from(ready["state"].as_str().ok_or("state missing")?);
        let calls = PathBuf::from(ready["calls"].as_str().ok_or("calls missing")?);
        assert!(workspace.starts_with(root.path()) && state.starts_with(root.path()));
        let gateway = mock_gateway(&state, &workspace, &calls, "");
        let source_before = fs::read(workspace.join("src/lib.rs"))?;
        let count_before = call_count(&calls)?;
        let expected_calls = match phase {
            "initial-git-added" | "preparing" => 0,
            "export-before-ack" | "cleanup-before-ack" => 4,
            _ => 1,
        };
        assert_eq!(
            count_before, expected_calls,
            "cutpoint did not observe the intended dispatch frontier: {phase}"
        );
        let expected_value = match phase {
            "initial-git-added" | "preparing" | "provider-admitted" | "staged" => 1,
            "export-before-ack" | "cleanup-before-ack" => 3,
            _ => 2,
        };
        assert_eq!(
            source_before,
            lib(expected_value, false).as_bytes(),
            "cutpoint did not observe the intended source effect: {phase}"
        );
        if matches!(
            phase,
            "initial-git-added"
                | "preparing"
                | "provider-admitted"
                | "staged"
                | "partial-publication"
        ) {
            assert!(
                !workspace.join("created.rs").exists(),
                "second publication happened before {phase}"
            );
        }
        let origin_before = gateway
            .database(|store| store.get_job(ORIGIN))
            .await?
            .ok_or("origin must be durable before the crash")?;
        let locator = PathBuf::from(
            origin_before
                .metadata
                .get("worktree_state_path")
                .ok_or("durable locator missing")?,
        );
        assert!(
            locator.is_dir(),
            "owned work must remain discoverable at {phase}"
        );
        let frontier_before = fs::read(locator.join("lifecycle.json"))?;
        let frontier: Value = serde_json::from_slice(&frontier_before)?;
        let mut observed_paths = vec![workspace.clone()];
        for node in frontier["nodes"]
            .as_array()
            .ok_or("frontier nodes missing")?
        {
            observed_paths.push(PathBuf::from(
                node["path"].as_str().ok_or("node path missing")?,
            ));
        }
        let source_snapshots = observed_paths
            .iter()
            .map(|path| {
                if path.is_dir() {
                    Ok(Some(DagSettlementVerifier::capture(path)?.files))
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let export_id = format!("ckp-{ORIGIN}-worktree-export");
        let export_lookup = export_id.clone();
        let export_before = gateway
            .database(move |store| store.get_job(&export_lookup))
            .await?;
        let mut resume = arguments(&workspace, &format!("cdx-crash-resume-{phase}"));
        resume["resume_dag_id"] = json!(ORIGIN);
        let resumed = if phase == "frontier-pinned" {
            resume_through_tui(&gateway, &workspace).await
        } else {
            crate::dag::request::handle_request(&gateway, &resume).await
        };
        if matches!(phase, "frontier-pinned" | "export-before-ack") {
            let resumed = resumed?;
            assert_eq!(resumed["ok"], true, "{phase}: {resumed}");
            assert_eq!(resumed["settled_count"], 4);
            assert_eq!(
                resumed["resumed_nodes"]
                    .as_array()
                    .ok_or("resumed nodes missing")?
                    .len(),
                if phase == "frontier-pinned" { 1 } else { 4 }
            );
            assert_eq!(resumed["worktrees"]["status"], "removed");
            assert_eq!(
                fs::read_to_string(workspace.join("src/lib.rs"))?,
                lib(3, false)
            );
            assert_eq!(
                fs::read_to_string(workspace.join("created.rs"))?,
                "// from-one\n// from-two\n"
            );
            let log = fs::read_to_string(&calls)?;
            assert_eq!(
                log.lines().collect::<Vec<_>>(),
                vec!["n0", "n1", "n2", "n3"]
            );
        } else {
            assert!(resumed.is_err(), "ambiguous {phase} must not resume");
            assert_eq!(
                call_count(&calls)?,
                count_before,
                "provider repeated after {phase}"
            );
            assert_eq!(
                fs::read(workspace.join("src/lib.rs"))?,
                source_before,
                "source overwritten after {phase}"
            );
            assert_eq!(
                gateway.database(|store| store.get_job(ORIGIN)).await?,
                Some(origin_before)
            );
            assert!(locator.is_dir(), "retained state removed after {phase}");
            assert_eq!(fs::read(locator.join("lifecycle.json"))?, frontier_before);
            for (path, before) in observed_paths.iter().zip(&source_snapshots) {
                let after = if path.is_dir() {
                    Some(DagSettlementVerifier::capture(path)?.files)
                } else {
                    None
                };
                assert_eq!(
                    &after,
                    before,
                    "source fixture scope changed after {phase}: {}",
                    path.display()
                );
            }
        }
        if phase == "export-before-ack" {
            assert!(
                export_before.is_some(),
                "export commit hook must follow actual SQL commit"
            );
            assert_eq!(
                gateway
                    .database(move |store| store.get_job(&export_id))
                    .await?,
                export_before
            );
        }
        gateway.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn all_settled_resume_rejects_source_drift_after_last_checkpoint_pin() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempdir()?;
    let ready = terminate_at("export-before-ack", root.path()).await?;
    let workspace = PathBuf::from(ready["workspace"].as_str().ok_or("workspace missing")?);
    let state = PathBuf::from(ready["state"].as_str().ok_or("state missing")?);
    let calls = PathBuf::from(ready["calls"].as_str().ok_or("calls missing")?);
    assert_eq!(call_count(&calls)?, 4);
    let changed = Arc::new(AtomicBool::new(false));
    let changed_for_hook = changed.clone();
    let source = workspace.join("src/lib.rs");
    let source_for_hook = source.clone();
    let control = TestControl {
        boundary: Some(Arc::new(move |node, point| {
            if node == "n3" && point == TestBoundary::FrontierPinnedBeforeNotify {
                assert!(
                    fs::write(&source_for_hook, "external change after final checkpoint\n").is_ok()
                );
                changed_for_hook.store(true, Ordering::SeqCst);
            }
        })),
    };
    let gateway = mock_gateway(&state, &workspace, &calls, "");
    let mut resume = arguments(&workspace, "cdx-all-settled-source-drift");
    resume["resume_dag_id"] = json!(ORIGIN);
    let result =
        crate::dag::request::handle_request_with_worktree_hook(&gateway, &resume, None, control)
            .await;
    assert!(
        changed.load(Ordering::SeqCst),
        "late source mutation boundary was not reached"
    );
    assert!(
        !matches!(result,Ok(ref result) if result["ok"]==true),
        "changed final source was accepted: {result:?}"
    );
    assert_eq!(
        fs::read_to_string(&source)?,
        "external change after final checkpoint\n"
    );
    assert_eq!(call_count(&calls)?, 4, "reused providers must not repeat");
    let continuation = gateway
        .database(|store| store.get_job("cdx-all-settled-source-drift"))
        .await?
        .ok_or("continuation missing after final pin")?;
    assert_ne!(
        continuation.outcome_state,
        crate::storage::OutcomeState::ConfirmedSuccess
    );
    let origin = gateway
        .database(|store| store.get_job(ORIGIN))
        .await?
        .ok_or("origin missing")?;
    let locator = PathBuf::from(
        origin
            .metadata
            .get("worktree_state_path")
            .ok_or("locator missing")?,
    );
    let lifecycle: Value = serde_json::from_slice(&fs::read(locator.join("lifecycle.json"))?)?;
    assert!(
        lifecycle["nodes"]
            .as_array()
            .ok_or("nodes missing")?
            .iter()
            .all(|node| node["removed"] == false
                && node["path"]
                    .as_str()
                    .is_some_and(|path| Path::new(path).is_dir()))
    );
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn pending_node_with_existing_provider_identity_is_never_dispatched() -> Result<()> {
    use crate::storage::{JobStatus, NewJob, SqlCatalog};
    let root = tempdir()?;
    let ready = terminate_at("frontier-pinned", root.path()).await?;
    let workspace = PathBuf::from(ready["workspace"].as_str().ok_or("workspace missing")?);
    let state = PathBuf::from(ready["state"].as_str().ok_or("state missing")?);
    let calls = PathBuf::from(ready["calls"].as_str().ok_or("calls missing")?);
    let gateway = mock_gateway(&state, &workspace, &calls, "");
    let origin = gateway
        .database(|store| store.get_job(ORIGIN))
        .await?
        .ok_or("origin missing")?;
    let (campaign, _) =
        WorktreeCampaign::reopen(&workspace, &state, &origin, Duration::from_secs(5)).await?;
    let pending_id = campaign.provider_request_id("n1");
    drop(campaign);
    let locator = PathBuf::from(
        origin
            .metadata
            .get("worktree_state_path")
            .ok_or("locator missing")?,
    );
    let lifecycle_before = fs::read(locator.join("lifecycle.json"))?;
    let lifecycle: Value = serde_json::from_slice(&lifecycle_before)?;
    let pending = PathBuf::from(
        lifecycle["nodes"]
            .as_array()
            .ok_or("nodes missing")?
            .iter()
            .find(|node| node["node_id"] == "n1")
            .and_then(|node| node["path"].as_str())
            .ok_or("pending path missing")?,
    );
    let source_before = fs::read(workspace.join("src/lib.rs"))?;
    let fixture_sql = SqlCatalog::parse(include_str!("../../../sql/worktree_resume_fixture.sql"))?;
    for (suffix, status) in [
        ("running", JobStatus::Running),
        ("unknown", JobStatus::TimedOutUnknown),
        ("completed", JobStatus::Completed),
    ] {
        let id = pending_id.clone();
        let node_root = pending.clone();
        gateway
            .database(move |store| {
                store.create_job(NewJob {
                    task_type: "claude_code_craft".into(),
                    workspace_dir: node_root,
                    model: Some("claude-sonnet-5-5-medium".into()),
                    metadata: Default::default(),
                    job_id: Some(id.clone()),
                    idempotency_key: Some(id.clone()),
                    provider: Some("antigravity".into()),
                    engine: Some("antigravity_local_rpc".into()),
                    created_at: None,
                })?;
                match status {
                    JobStatus::Running => {}
                    JobStatus::TimedOutUnknown => {
                        store.mark_timed_out_unknown(
                            &id,
                            "controlled prior unknown".into(),
                            None,
                            None,
                        )?;
                    }
                    JobStatus::Completed => {
                        store.complete_job(
                            &id,
                            json!({"fixture":"already completed"}),
                            None,
                            None,
                        )?;
                    }
                    _ => return Err("unexpected collision fixture state".into()),
                }
                Ok(())
            })
            .await?;
        let mut resume = arguments(&workspace, &format!("cdx-pending-collision-{suffix}"));
        resume["resume_dag_id"] = json!(ORIGIN);
        assert!(
            crate::dag::request::handle_request(&gateway, &resume)
                .await
                .is_err(),
            "existing {suffix} provider ID was admitted again"
        );
        assert_eq!(
            call_count(&calls)?,
            1,
            "provider called despite existing {suffix} identity"
        );
        assert_eq!(fs::read(workspace.join("src/lib.rs"))?, source_before);
        assert_eq!(fs::read(locator.join("lifecycle.json"))?, lifecycle_before);
        assert_eq!(
            gateway.database(|store| store.get_job(ORIGIN)).await?,
            Some(origin.clone())
        );
        // Remove only the synthetic collision row introduced by this test actor.
        assert_eq!(
            rusqlite::Connection::open(state.join("jobs/jobs.db"))?.execute(
                fixture_sql.query("delete_collision_fixture_job", 1)?,
                [&pending_id]
            )?,
            1
        );
    }
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cleanup_refuses_source_change_after_durable_execution_export() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = fixture(false)?;
    let calls = f._root.path().join("provider-calls");
    let gateway = mock_gateway(&f.state, &f.original, &calls, "");
    let source = f.original.join("src/lib.rs");
    let source_for_hook = source.clone();
    let changed = Arc::new(AtomicBool::new(false));
    let changed_for_hook = changed.clone();
    let control = TestControl {
        boundary: Some(Arc::new(move |_, point| {
            if point == TestBoundary::ExportCommittedBeforeAck {
                assert!(
                    fs::write(&source_for_hook, "external change after durable export\n").is_ok()
                );
                changed_for_hook.store(true, Ordering::SeqCst);
            }
        })),
    };
    let result = crate::dag::request::handle_request_with_worktree_hook(
        &gateway,
        &arguments(&f.original, "cdx-export-source-drift"),
        None,
        control,
    )
    .await;
    assert!(
        changed.load(Ordering::SeqCst),
        "export-to-cleanup mutation boundary was not reached"
    );
    assert_eq!(call_count(&calls)?, 4);
    assert_eq!(
        fs::read_to_string(&source)?,
        "external change after durable export\n"
    );
    if let Ok(result) = &result {
        assert_ne!(
            result["worktrees"]["status"], "removed",
            "cleanup accepted changed source after durable export"
        );
    }
    let saved = gateway
        .database(|store| store.get_job("cdx-export-source-drift"))
        .await?
        .ok_or("campaign missing after export")?;
    let locator = PathBuf::from(
        saved
            .metadata
            .get("worktree_state_path")
            .ok_or("durable locator missing")?,
    );
    let lifecycle: Value = serde_json::from_slice(&fs::read(locator.join("lifecycle.json"))?)?;
    let nodes = lifecycle["nodes"]
        .as_array()
        .ok_or("retained nodes missing")?;
    assert_eq!(nodes.len(), 4);
    assert!(
        nodes.iter().all(|node| node["removed"] == false
            && node["path"]
                .as_str()
                .is_some_and(|path| Path::new(path).is_dir())),
        "verified worktrees were deleted despite changed canonical source"
    );
    assert!(
        gateway
            .database(|store| store.get_job("ckp-cdx-export-source-drift-worktree-export"))
            .await?
            .is_some(),
        "boundary did not follow actual export commit"
    );
    gateway.shutdown().await?;
    Ok(())
}
