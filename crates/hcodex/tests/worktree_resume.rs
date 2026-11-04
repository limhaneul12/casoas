//! Restart acceptance through the public DAG boundary and real isolated Git/Cargo.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use hcodex::gateway::Gateway;
use hcodex::storage::SqlCatalog;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

fn git(root: &Path, args: &[&str]) -> hcodex::Result<()> {
    let result = std::process::Command::new("git")
        .args(["--no-optional-locks", "-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(root)
        .output()?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

fn repository(root: &Path) -> hcodex::Result<PathBuf> {
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src"))?;
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname=\"resume_fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )?;
    fs::write(repo.join(".gitignore"), "target/\n.hcodex/\n")?;
    fs::write(
        repo.join("src/lib.rs"),
        "pub fn value() -> u8 { 1 }\n#[test] fn value_is_one() { assert_eq!(value(), 1); }\n",
    )?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    assert!(
        std::process::Command::new(cargo)
            .args(["generate-lockfile", "--offline"])
            .current_dir(&repo)
            .status()?
            .success()
    );
    git(&repo, &["init", "-q"])?;
    git(&repo, &["add", "."])?;
    git(
        &repo,
        &[
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
    )?;
    Ok(repo.canonicalize()?)
}

fn request(repo: &Path, id: &str) -> Value {
    let nodes: Vec<Value> = (0..4).map(|i| json!({
        "node_id":format!("n{i}"), "role":"adversarial_auditor", "task":"read-only fixture review",
        "read_files":["src/lib.rs"], "verify_cmd":"cargo test", "preferred_model":"gpt-6.1-sol",
        "depends_on":if i == 0 { vec![] } else { vec![format!("n{}",i-1)] }
    })).collect();
    json!({"campaign_goal":"resume retained verified worktrees", "workspace_dir":repo, "request_id":id,
        "plan_only":false, "timeout_seconds":30, "nodes":nodes,
        "spawn_plan":{"plan_summary":"four serial readers", "hazard_acknowledgment":true,
            "isolation_mode":"worktree", "max_parallel_per_wave":1}})
}

async fn rejects_without_replay(
    gateway: &Gateway,
    repo: &Path,
    calls: &Path,
    request: &Value,
) -> hcodex::Result<()> {
    let source_before = fs::read(repo.join("src/lib.rs"))?;
    let registrations_before = std::process::Command::new("git")
        .args([
            "--no-optional-locks",
            "worktree",
            "list",
            "--porcelain",
            "-z",
        ])
        .current_dir(repo)
        .output()?;
    assert!(registrations_before.status.success());
    let origin_before = gateway
        .database(|store| store.get_job("cdx-resume-export-origin"))
        .await?;
    assert!(hcodex::dag::handle_request(gateway, request).await.is_err());
    assert_eq!(fs::read_to_string(calls)?.lines().count(), 4);
    assert_eq!(fs::read(repo.join("src/lib.rs"))?, source_before);
    assert_eq!(
        gateway
            .database(|store| store.get_job("cdx-resume-export-origin"))
            .await?,
        origin_before
    );
    let registrations_after = std::process::Command::new("git")
        .args([
            "--no-optional-locks",
            "worktree",
            "list",
            "--porcelain",
            "-z",
        ])
        .current_dir(repo)
        .output()?;
    assert!(registrations_after.status.success());
    assert_eq!(registrations_after.stdout, registrations_before.stdout);
    Ok(())
}

async fn exercise_export_restart(root: &Path) -> hcodex::Result<()> {
    let repo = repository(root)?;
    let state = root.join("state");
    let calls = root.join("provider-calls");
    let gateway = Gateway::new(state.clone());
    gateway.database(|_| Ok(())).await?;
    rusqlite::Connection::open(state.join("jobs/jobs.db"))?
        .execute_batch(include_str!("../sql/worktree_export_fixture.sql"))?;
    let original = request(&repo, "cdx-resume-export-origin");
    let failed = hcodex::dag::handle_request(&gateway, &original).await;
    assert!(
        failed.is_err(),
        "fixture must interrupt durable result export"
    );
    let saved = gateway
        .database(|store| store.get_job("cdx-resume-export-origin"))
        .await?
        .ok_or("origin campaign missing")?;
    let retained = PathBuf::from(
        saved
            .metadata
            .get("worktree_state_path")
            .ok_or("retained state missing")?,
    );
    let lifecycle: Value = serde_json::from_slice(&fs::read(retained.join("lifecycle.json"))?)?;
    let nodes = lifecycle["nodes"]
        .as_array()
        .ok_or("retained nodes missing")?;
    assert_eq!(nodes.len(), 4);
    let records = gateway.database(|store| store.list_jobs(100, None)).await?;
    let diagnostics: Vec<_> = records
        .iter()
        .filter(|record| record.metadata.get("checkpoint_kind").map(String::as_str) == Some("node"))
        .map(|record| json!({"job_id":record.job_id,"state":record.result.as_ref().map(|v|&v["receipt"]["state"]),"error":record.result.as_ref().map(|v|&v["receipt"]["error"])}))
        .collect();
    assert!(
        nodes
            .iter()
            .all(|n| n["verified"] == true && n["removed"] == false),
        "retained nodes were not all settled: {diagnostics:?}"
    );
    assert_eq!(fs::read_to_string(&calls)?.lines().count(), 4);
    gateway.shutdown().await?;
    drop(gateway);

    let fixture_sql = SqlCatalog::parse(include_str!("../sql/worktree_resume_fixture.sql"))?;
    rusqlite::Connection::open(state.join("jobs/jobs.db"))?
        .execute(fixture_sql.query("release_export_block", 0)?, [])?;
    let gateway = Gateway::new(state);
    let mut resume = request(&repo, "cdx-resume-export-continuation");
    resume["resume_dag_id"] = json!("cdx-resume-export-origin");

    let mut changed_plan = resume.clone();
    changed_plan["request_id"] = json!("cdx-refused-plan");
    changed_plan["campaign_goal"] = json!("foreign plan");
    rejects_without_replay(&gateway, &repo, &calls, &changed_plan).await?;
    let mut changed_config = resume.clone();
    changed_config["request_id"] = json!("cdx-refused-config");
    changed_config["spawn_plan"]["max_parallel_per_wave"] = json!(2);
    rejects_without_replay(&gateway, &repo, &calls, &changed_config).await?;

    let locks: Vec<_> = fs::read_dir(repo.join(".git/hcodex-worktree-locks"))?
        .collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(locks.len(), 1);
    let lock_path = locks[0].path();
    let original_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)?;
    original_lock.try_lock()?;
    let parked_lock = root.join("parked-worktree-lock");
    fs::rename(&lock_path, &parked_lock)?;
    fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    let mut replaced_lock = resume.clone();
    replaced_lock["request_id"] = json!("cdx-refused-replaced-lock");
    rejects_without_replay(&gateway, &repo, &calls, &replaced_lock).await?;
    original_lock.unlock()?;
    fs::remove_file(&lock_path)?;
    fs::rename(parked_lock, lock_path)?;

    let source = repo.join("src/lib.rs");
    let original_source = fs::read(&source)?;
    fs::write(&source, "external source edit\n")?;
    let mut changed_source = resume.clone();
    changed_source["request_id"] = json!("cdx-refused-source");
    rejects_without_replay(&gateway, &repo, &calls, &changed_source).await?;
    fs::write(&source, &original_source)?;

    let first_node = PathBuf::from(nodes[0]["path"].as_str().ok_or("node path missing")?);
    let private = first_node.join("unexpected-private-file");
    fs::write(&private, "unrelated work must survive\n")?;
    let mut changed_private = resume.clone();
    changed_private["request_id"] = json!("cdx-refused-private");
    rejects_without_replay(&gateway, &repo, &calls, &changed_private).await?;
    assert_eq!(
        fs::read_to_string(&private)?,
        "unrelated work must survive\n"
    );
    fs::remove_file(private)?;

    let parked = root.join("parked-owned-node");
    fs::rename(&first_node, &parked)?;
    fs::create_dir(&first_node)?;
    let mut changed_inode = resume.clone();
    changed_inode["request_id"] = json!("cdx-refused-node-identity");
    rejects_without_replay(&gateway, &repo, &calls, &changed_inode).await?;
    fs::remove_dir(&first_node)?;
    fs::rename(parked, &first_node)?;

    let owner_path = retained.join("ownership.json");
    let original_owner = fs::read(&owner_path)?;
    let mut altered_owner = original_owner.clone();
    altered_owner.push(b'\n');
    fs::write(&owner_path, altered_owner)?;
    let mut changed_owner = resume.clone();
    changed_owner["request_id"] = json!("cdx-refused-owner-body");
    rejects_without_replay(&gateway, &repo, &calls, &changed_owner).await?;
    fs::write(&owner_path, original_owner)?;
    let parked_owner = root.join("parked-ownership.json");
    fs::rename(&owner_path, &parked_owner)?;
    let mut missing_owner = resume.clone();
    missing_owner["request_id"] = json!("cdx-refused-missing-owner");
    rejects_without_replay(&gateway, &repo, &calls, &missing_owner).await?;
    fs::rename(parked_owner, owner_path)?;

    let result = hcodex::dag::handle_request(&gateway, &resume).await;
    gateway.shutdown().await?;
    let result = result?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["settled_count"], 4);
    assert_eq!(
        result["resumed_nodes"]
            .as_array()
            .ok_or("reused nodes missing")?
            .len(),
        4
    );
    assert_eq!(result["worktrees"]["status"], "removed");
    assert_eq!(
        fs::read_to_string(&calls)?.lines().count(),
        4,
        "settled providers must not run again"
    );
    let reopened = Gateway::new(root.join("state"));
    let replay = hcodex::dag::handle_request(&reopened, &resume).await;
    reopened.shutdown().await?;
    assert_eq!(
        replay?, result,
        "same-ID exact readback must not reopen removed worktrees"
    );
    assert_eq!(fs::read_to_string(&calls)?.lines().count(), 4);
    for node in nodes {
        assert!(!Path::new(node["path"].as_str().ok_or("node path missing")?).exists());
    }
    assert_eq!(
        fs::read_to_string(repo.join("src/lib.rs"))?,
        "pub fn value() -> u8 { 1 }\n#[test] fn value_is_one() { assert_eq!(value(), 1); }\n"
    );
    Ok(())
}

#[tokio::test]
async fn resume_export_fixture_child() -> hcodex::Result<()> {
    let Some(root) = std::env::var_os("HCODEX_RESUME_TEST_ROOT") else {
        return Ok(());
    };
    exercise_export_restart(&PathBuf::from(root)).await
}

async fn capture(reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > 1024 * 1024 {
        return Err(std::io::Error::other("fixture output exceeded bound"));
    }
    Ok(bytes)
}

#[tokio::test]
async fn retained_settled_worktrees_resume_after_export_failure_without_provider_replay()
-> hcodex::Result<()> {
    let root = tempdir()?;
    let binary = root.path().join("fixture-codex");
    let calls = root.path().join("provider-calls");
    let audit = json!({"verdict":"PASS", "risk_score":0, "executive_critique":"Fixture passed.",
        "findings":[], "failing_test_cases":[], "alexandria_decision_note":"fixture"});
    let event =
        json!({"type":"item.completed", "item":{"type":"agent_message", "text":audit.to_string()}});
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nif [ \"$1\" != exec ]; then printf '%s\\n' 'codex-cli 0.166.0'; exit 0; fi\nprintf '%s\\n' called >> '{}'\ncat >/dev/null\nprintf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"thr_fixture_resume\",\"model\":\"gpt-6.1-sol\"}}' '{event}'\n",
            calls.display()
        ),
    )?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "resume_export_fixture_child", "--nocapture"])
        .env("HCODEX_RESUME_TEST_ROOT", root.path())
        .env("CODEX_CLI_BIN", &binary)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().ok_or("fixture stdout missing")?;
    let stderr = child.stderr.take().ok_or("fixture stderr missing")?;
    let completed = {
        let operation = async { tokio::try_join!(child.wait(), capture(stdout), capture(stderr)) };
        tokio::time::timeout(Duration::from_secs(180), operation).await
    };
    let (status, stdout, stderr) = match completed {
        Ok(result) => result?,
        Err(error) => {
            child.kill().await?;
            child.wait().await?;
            return Err(error.into());
        }
    };
    assert!(
        status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    Ok(())
}
