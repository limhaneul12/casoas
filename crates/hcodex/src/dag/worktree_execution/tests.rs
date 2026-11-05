use super::*;
use crate::dag::{NodeKind, NodeRole};
use std::path::PathBuf;
use std::process::Command;
use tempfile::{TempDir, tempdir};

struct Fixture {
    _root: TempDir,
    original: PathBuf,
    state: PathBuf,
}
fn lib(value: u8, isolated_only: bool) -> String {
    format!(
        "pub fn value() -> u8 {{ {value} }}\n#[test]\nfn check() {{ assert_eq!(value(), {value}); {} }}\n",
        if isolated_only {
            "assert!(std::path::Path::new(\".git\").is_file());"
        } else {
            ""
        }
    )
}
fn fixture(isolated_only: bool) -> Result<Fixture> {
    let root = tempdir()?;
    let original = root.path().join("repo");
    fs::create_dir_all(original.join("src"))?;
    let package = format!("wt_fixture_{}", uuid::Uuid::new_v4().simple());
    fs::write(
        original.join("Cargo.toml"),
        format!("[package]\nname=\"{package}\"\nversion=\"0.1.0\"\nedition=\"2024\"\n"),
    )?;
    fs::write(
        original.join("Cargo.lock"),
        format!("version = 4\n[[package]]\nname = \"{package}\"\nversion = \"0.1.0\"\n"),
    )?;
    fs::write(original.join(".gitignore"), "target/\n.hcodex/\n")?;
    fs::write(original.join("src/lib.rs"), lib(1, isolated_only))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    assert!(
        Command::new(cargo)
            .args(["generate-lockfile", "--offline"])
            .current_dir(&original)
            .status()?
            .success()
    );
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(&original)
            .status()?;
        assert!(status.success());
    }
    let original = original.canonicalize()?;
    Ok(Fixture {
        state: root.path().join("state"),
        _root: root,
        original,
    })
}
fn node(id: &str, writes: bool) -> DagNode {
    DagNode {
        id: id.into(),
        title: id.into(),
        task: "controlled provider fixture".into(),
        role: if writes {
            NodeRole::InterfaceBuilder
        } else {
            NodeRole::ContractVerifier
        },
        node_kind: NodeKind::Standard,
        model: if writes {
            "claude-sonnet-5-5-medium"
        } else {
            "gpt-6.1-sol"
        }
        .into(),
        reasoning_effort: "high".into(),
        dependencies: Vec::new(),
        read_files: vec!["src/lib.rs".into()],
        write_files: if writes {
            vec!["src/lib.rs".into(), "created.rs".into()]
        } else {
            Vec::new()
        },
        symbols: Vec::new(),
        verify_command: Some("cargo test".into()),
        reproduction_expected_exit_nonzero: true,
    }
}
fn response(model: &str, code: Option<Value>) -> Value {
    json!({"ok":true,"result":{"receipt":{"actual_model":model,"requested_model":model,
        "status":"succeeded","outcome_state":"confirmed_success","model_mismatch":false,
        "engine":if code.is_some(){"antigravity_cli"}else{"codex_cli"},"owned_processes_closed":true},
        "structured_output":{"crafted_code":code.map(|v|v.to_string()),"summary":"fixture"}}})
}
fn artifact(before: &WorkspaceManifest, source: String) -> Value {
    json!({"edits":[{"path":"src/lib.rs","preimage_sha256":before.files[Path::new("src/lib.rs")],"new_content":source},
        {"path":"created.rs","preimage_sha256":"sha256:absent","new_content":"upstream result\n"}]})
}

#[tokio::test]
async fn dependent_nodes_publish_canonical_results_before_verified_cleanup() -> Result<()> {
    let f = fixture(false)?;
    let manager = WorktreeCampaign::start(
        &f.original,
        &f.state,
        &["writer".into(), "second".into(), "verifier".into()],
        Duration::from_secs(10),
        "fixture",
    )
    .await?;
    let baseline = DagSettlementVerifier::capture(&f.original)?;
    let expected = Mutex::new(baseline.clone());
    let patch = artifact(&baseline, lib(2, false));
    let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
    let paths = seen.clone();
    let original = f.original.clone();
    let (writer, _) = execute_node(
        node("writer", true),
        &f.original,
        Duration::from_secs(30),
        256000,
        0,
        &baseline,
        &[],
        &None,
        &manager,
        &expected,
        move |_, args| async move {
            let path = PathBuf::from(args["workspace_dir"].as_str().ok_or("workspace missing")?);
            assert_ne!(path, original);
            paths.lock().await.push(path);
            Ok(response("claude-sonnet-5-5-medium", Some(patch)))
        },
    )
    .await?;
    assert_eq!(writer.state, NodeState::Settled, "{writer:?}");
    assert_eq!(
        fs::read_to_string(f.original.join("src/lib.rs"))?,
        lib(2, false)
    );
    assert!(manager.cleanup_fixture_nodes().await.is_err());
    let baseline = expected.lock().await.clone();
    let paths = seen.clone();
    let mut second = node("second", true);
    second.write_files = vec!["created.rs".into(), "second.rs".into()];
    second.read_files.push("created.rs".into());
    let patch = json!({"edits":[{"path":"created.rs","preimage_sha256":baseline.files[Path::new("created.rs")],"new_content":"node two result\n"},
        {"path":"second.rs","preimage_sha256":"sha256:absent","new_content":"chain complete\n"}]});
    let (second, _) = execute_node(
        second,
        &f.original,
        Duration::from_secs(30),
        256000,
        1,
        &baseline,
        &[writer],
        &None,
        &manager,
        &expected,
        move |_, args| async move {
            let path = PathBuf::from(args["workspace_dir"].as_str().ok_or("workspace missing")?);
            assert_eq!(
                fs::read_to_string(path.join("created.rs"))?,
                "upstream result\n"
            );
            assert_eq!(fs::read_to_string(path.join("src/lib.rs"))?, lib(2, false));
            paths.lock().await.push(path);
            Ok(response("claude-sonnet-5-5-medium", Some(patch)))
        },
    )
    .await?;
    assert_eq!(second.state, NodeState::Settled);
    let baseline = expected.lock().await.clone();
    let paths = seen.clone();
    let (verifier, _) = execute_node(
        node("verifier", false),
        &f.original,
        Duration::from_secs(30),
        256000,
        2,
        &baseline,
        &[second],
        &None,
        &manager,
        &expected,
        move |_, args| async move {
            let path = PathBuf::from(args["workspace_dir"].as_str().ok_or("workspace missing")?);
            assert_eq!(
                fs::read_to_string(path.join("created.rs"))?,
                "node two result\n"
            );
            assert_eq!(
                fs::read_to_string(path.join("second.rs"))?,
                "chain complete\n"
            );
            assert_eq!(fs::read_to_string(path.join("src/lib.rs"))?, lib(2, false));
            paths.lock().await.push(path);
            Ok(response("gpt-6.1-sol", None))
        },
    )
    .await?;
    assert_eq!(verifier.state, NodeState::Settled);
    let seen = seen.lock().await;
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
    manager
        .verify_original(&expected.lock().await.clone())
        .await?;
    manager.cleanup_fixture_nodes().await?;
    assert!(seen.iter().all(|path| !path.exists()));
    assert!(manager.state_path().join("lifecycle.json").is_file());
    Ok(())
}

#[tokio::test]
async fn canonical_gate_failure_retains_original_and_worktree_evidence() -> Result<()> {
    let f = fixture(true)?;
    let manager = WorktreeCampaign::start(
        &f.original,
        &f.state,
        &["writer".into()],
        Duration::from_secs(10),
        "fixture",
    )
    .await?;
    let baseline = DagSettlementVerifier::capture(&f.original)?;
    let expected = Mutex::new(baseline.clone());
    let patch = artifact(&baseline, lib(2, true));
    let (receipt, output) = execute_node(
        node("writer", true),
        &f.original,
        Duration::from_secs(30),
        256000,
        0,
        &baseline,
        &[],
        &None,
        &manager,
        &expected,
        move |_, _| async move { Ok(response("claude-sonnet-5-5-medium", Some(patch))) },
    )
    .await?;
    assert_ne!(receipt.state, NodeState::Settled);
    assert!(receipt.gate.executed);
    assert_ne!(receipt.gate.exit_code, Some(0));
    assert!(Path::new(output["worktree_path"].as_str().ok_or("path missing")?).exists());
    assert!(manager.cleanup_fixture_nodes().await.is_err());
    assert_eq!(
        fs::read_to_string(f.original.join("src/lib.rs"))?,
        lib(2, true)
    );
    Ok(())
}

#[tokio::test]
async fn unknown_or_unclosed_provider_never_publishes_or_permits_cleanup() -> Result<()> {
    let f = fixture(false)?;
    let manager = WorktreeCampaign::start(
        &f.original,
        &f.state,
        &["unknown".into(), "unclosed".into()],
        Duration::from_secs(10),
        "fixture",
    )
    .await?;
    let baseline = DagSettlementVerifier::capture(&f.original)?;
    let expected = Mutex::new(baseline.clone());
    for id in ["unknown", "unclosed"] {
        let mut out = response("gpt-6.1-sol", None);
        if id == "unknown" {
            out["ok"] = json!(false);
            out["result"]["receipt"]["status"] = json!("timed_out_unknown");
        } else {
            out["result"]["receipt"]["owned_processes_closed"] = json!(false);
        }
        let (receipt, _) = execute_node(
            node(id, false),
            &f.original,
            Duration::from_secs(30),
            256000,
            0,
            &baseline,
            &[],
            &None,
            &manager,
            &expected,
            move |_, _| async move { Ok(out) },
        )
        .await?;
        assert_ne!(receipt.state, NodeState::Settled);
    }
    assert_eq!(DagSettlementVerifier::capture(&f.original)?, baseline);
    assert!(manager.cleanup_fixture_nodes().await.is_err());
    Ok(())
}

#[cfg(unix)]
async fn request_fixture(block_export: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture(false)?;
    let binary = f._root.path().join("fixture-codex");
    let audit = json!({"verdict":"PASS","risk_score":0,"executive_critique":"Fixture passed.","findings":[],"failing_test_cases":[],"alexandria_decision_note":"fixture"});
    let event =
        json!({"type":"item.completed","item":{"type":"agent_message","text":audit.to_string()}});
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"thr_fixture_worktree\",\"model\":\"gpt-6.1-sol\"}}' '{event}'\n"
        ),
    )?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))?;
    let gateway = crate::gateway::Gateway::new(f.state.clone()).with_codex_binary(binary);
    if block_export {
        gateway.database(|_| Ok(())).await?;
        rusqlite::Connection::open(f.state.join("jobs/jobs.db"))?
            .execute_batch(include_str!("../../../sql/worktree_export_fixture.sql"))?;
    }
    let nodes:Vec<Value>=(0..4).map(|i|json!({"node_id":format!("n{i}"),"role":"adversarial_auditor","task":"read-only fixture review","read_files":["src/lib.rs"],"verify_cmd":"cargo test","preferred_model":"gpt-6.1-sol", "depends_on":if i>=2 {vec!["n0","n1"]}else{vec![]}})).collect();
    let args = json!({"campaign_goal":"worktree fixture","workspace_dir":f.original,"request_id":"cdx-worktree-fixture","plan_only":false,"timeout_seconds":30,
        "spawn_plan":{"plan_summary":"four isolated readers","hazard_acknowledgment":true,"isolation_mode":"worktree","max_parallel_per_wave":2},"nodes":nodes});
    let attempt = super::super::request::handle_request(&gateway, &args).await;
    if block_export {
        assert!(attempt.is_err());
        let saved = gateway
            .database(|store| store.get_job("cdx-worktree-fixture"))
            .await?
            .ok_or("campaign missing")?;
        let state = PathBuf::from(
            saved
                .metadata
                .get("worktree_state_path")
                .ok_or("retention path missing")?,
        );
        let report: Value = serde_json::from_slice(&fs::read(state.join("lifecycle.json"))?)?;
        assert_eq!(report["campaign_id"], "cdx-worktree-fixture");
        for n in report["nodes"].as_array().ok_or("nodes missing")? {
            assert_eq!(n["removed"], false);
            assert_eq!(n["verified"], true);
            assert!(Path::new(n["path"].as_str().ok_or("path missing")?).exists());
        }
        gateway.shutdown().await?;
        return Ok(());
    }
    let result = attempt?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["worktrees"]["status"], "removed");
    let exported = result["worktrees"]["execution_export_id"]
        .as_str()
        .ok_or("export missing")?
        .to_owned();
    let saved = gateway
        .database(move |store| store.get_job(&exported))
        .await?
        .ok_or("export absent")?;
    assert_eq!(saved.status, crate::storage::JobStatus::Completed);
    assert_eq!(
        saved.result.as_ref().ok_or("export empty")?["settled_count"],
        4
    );
    for n in result["worktrees"]["report"]["nodes"]
        .as_array()
        .ok_or("nodes absent")?
    {
        assert!(!Path::new(n["path"].as_str().ok_or("path absent")?).exists());
    }
    fs::write(
        f.original.join("src/lib.rs"),
        "external edit after completion\n",
    )?;
    assert_eq!(
        super::super::request::handle_request(&gateway, &args).await?,
        result
    );
    fs::write(f.original.join("src/lib.rs"), lib(1, false))?;
    let other = fixture(false)?;
    let mut wrong = args.clone();
    wrong["workspace_dir"] = json!(other.original);
    assert!(
        super::super::request::handle_request(&gateway, &wrong)
            .await
            .is_err()
    );
    assert!(
        Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(&f.original)
            .output()?
            .stdout
            .is_empty()
    );
    gateway.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn real_request_persists_checkpoints_and_result_export_before_cleanup() -> Result<()> {
    request_fixture(false).await
}

#[cfg(unix)]
#[tokio::test]
async fn export_failure_retains_verified_worktrees_with_durable_campaign_link() -> Result<()> {
    request_fixture(true).await
}

#[tokio::test]
async fn missing_worktree_resume_origin_is_rejected_without_new_dispatch() -> Result<()> {
    let root = tempdir()?;
    let gateway = crate::gateway::Gateway::new(root.path().join("state"));
    let args = json!({"campaign_goal":"do not replay","workspace_dir":root.path(),"request_id":"cdx-new-resume",
        "resume_dag_id":"cdx-prior-unknown","plan_only":false,
        "spawn_plan":{"plan_summary":"reject unsafe replay","hazard_acknowledgment":true,"isolation_mode":"worktree"}});
    let error = super::super::request::handle_request(&gateway, &args)
        .await
        .err()
        .ok_or("resume should reject")?;
    assert!(error.to_string().contains("resume DAG was not found"));
    assert!(
        gateway
            .database(|store| store.get_job("cdx-new-resume"))
            .await?
            .is_none()
    );
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn failed_worktree_id_is_readback_only_even_after_original_becomes_clean() -> Result<()> {
    let f = fixture(false)?;
    fs::write(f.original.join("untracked.tmp"), "dirty")?;
    let gateway = crate::gateway::Gateway::new(f.state.clone());
    let nodes:Vec<_>=(0..4).map(|i|json!({"node_id":format!("n{i}"),"role":"adversarial_auditor","task":"must not dispatch",
        "preferred_model":"invalid-model","depends_on":if i>=2 {vec!["n0","n1"]}else{vec![]}})).collect();
    let args = json!({"campaign_goal":"reject dirty source","workspace_dir":f.original,"request_id":"cdx-dirty-worktree",
        "plan_only":false,"nodes":nodes,"spawn_plan":{"plan_summary":"no dispatch","hazard_acknowledgment":true,"isolation_mode":"worktree"}});
    assert!(
        super::super::request::handle_request(&gateway, &args)
            .await
            .is_err()
    );
    let first = gateway
        .database(|store| store.get_job("cdx-dirty-worktree"))
        .await?
        .ok_or("rejection record missing")?;
    assert_eq!(first.status, crate::storage::JobStatus::RejectedPreDispatch);
    fs::remove_file(f.original.join("untracked.tmp"))?;
    assert!(
        super::super::request::handle_request(&gateway, &args)
            .await
            .is_err()
    );
    let second = gateway
        .database(|store| store.get_job("cdx-dirty-worktree"))
        .await?
        .ok_or("rejection disappeared")?;
    assert_eq!(second.status, first.status);
    assert_eq!(second.attempt_count, first.attempt_count);
    assert!(!f.state.join("worktrees").exists());
    gateway.shutdown().await?;
    Ok(())
}

#[path = "crash_tests.rs"]
mod crash_tests;
