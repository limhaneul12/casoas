//! Recovery diagnostics through the actual CLI, with isolated durable boundaries.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hcodex::storage::{NewJob, SqlCatalog, Store};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

const SOURCE: &[u8] = b"preserved source\n";
const PRIVATE_PAYLOAD: &str = "fixture-private-provider-payload";

struct Fixture {
    root: TempDir,
    workspace: PathBuf,
    state: PathBuf,
    store: Store,
}

impl Fixture {
    fn new() -> hcodex::Result<Self> {
        let root = tempdir()?;
        let workspace = root.path().join("workspace");
        fs::create_dir(&workspace)?;
        fs::write(workspace.join("source.rs"), SOURCE)?;
        for arguments in [
            vec!["init", "-q"],
            vec!["add", "source.rs"],
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
            git(&workspace, &arguments)?;
        }
        let state = root.path().join("state");
        let store = Store::open(&state)?;
        Ok(Self {
            root,
            workspace,
            state,
            store,
        })
    }
}

fn git(workspace: &Path, arguments: &[&str]) -> hcodex::Result<String> {
    let mut command = Command::new("git");
    command
        .args(["--no-optional-locks", "-c", "core.hooksPath=/dev/null"])
        .args(arguments)
        .current_dir(workspace);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    let output = command.output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn job(workspace: &Path, id: &str, metadata: BTreeMap<String, String>) -> NewJob {
    NewJob {
        task_type: "dag_ulw".into(),
        workspace_dir: workspace.into(),
        model: Some("dag-ulw".into()),
        metadata,
        job_id: Some(id.into()),
        idempotency_key: Some(id.into()),
        provider: Some("heterarchy_triad".into()),
        engine: Some("dag_ulw".into()),
        created_at: Some(1.0),
    }
}

fn diagnose(workspace: &Path, state: &Path, id: &str) -> hcodex::Result<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_hcodex"))
        .arg("--workspace")
        .arg(workspace)
        .arg("--state-root")
        .arg(state)
        .args(["recovery", id])
        .env("HCODEX_HOME", workspace.join("absent-home"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(PRIVATE_PAYLOAD),
        "diagnostics must not disclose raw provider payloads"
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["read_only"], true);
    assert_eq!(report["observation_only"], true);
    assert_eq!(report["resume_authorized"], false);
    Ok(report)
}

fn durable_bytes(state: &Path) -> hcodex::Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        fs::read(state.join("jobs/jobs.db"))?,
        fs::read(state.join("jobs/jobs.db-wal"))?,
    ))
}

fn campaign_state(state: &Path) -> hcodex::Result<PathBuf> {
    let path = state
        .join("worktrees")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn register_campaign(fixture: &Fixture, id: &str, path: &Path) -> hcodex::Result<()> {
    fixture.store.create_job(job(
        &fixture.workspace,
        id,
        BTreeMap::from([(
            "worktree_state_path".into(),
            path.to_str().ok_or("state path is not UTF-8")?.into(),
        )]),
    ))?;
    Ok(())
}

fn lifecycle(workspace: &Path, state: &Path, id: &str, nodes: Vec<Value>) -> hcodex::Result<Value> {
    Ok(json!({
        "campaign_id": id,
        "original": workspace.canonicalize()?,
        "head": git(workspace, &["rev-parse", "HEAD"])?.trim(),
        "state_path": state.canonicalize()?,
        "nodes": nodes,
    }))
}

fn write_lifecycle(state: &Path, body: &Value) -> hcodex::Result<()> {
    fs::write(state.join("lifecycle.json"), serde_json::to_vec(body)?)?;
    Ok(())
}

fn add_worktree(workspace: &Path, path: &Path) -> hcodex::Result<()> {
    git(
        workspace,
        &[
            "worktree",
            "add",
            "--detach",
            path.to_str().ok_or("worktree path is not UTF-8")?,
            "HEAD",
        ],
    )?;
    Ok(())
}

fn node<'a>(report: &'a Value, id: &str) -> hcodex::Result<&'a Value> {
    report["worktrees"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == id))
        .ok_or_else(|| format!("diagnostic node {id} missing: {report}").into())
}

#[test]
fn missing_state_is_reported_without_creating_a_store() -> hcodex::Result<()> {
    let root = tempdir()?;
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace)?;
    fs::write(workspace.join("source.rs"), b"preserved\n")?;
    let state = root.path().join("missing-state");
    let output = Command::new(env!("CARGO_BIN_EXE_hcodex"))
        .arg("--workspace")
        .arg(&workspace)
        .arg("--state-root")
        .arg(&state)
        .args(["recovery", "cdx-missing"])
        .env("HCODEX_HOME", root.path().join("isolated-home"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["read_only"], true);
    assert!(
        !state.exists(),
        "a diagnostic must not initialize missing state"
    );
    assert_eq!(fs::read(workspace.join("source.rs"))?, b"preserved\n");
    Ok(())
}

#[test]
fn live_wal_and_old_running_unknown_jobs_are_observed_without_reconciliation() -> hcodex::Result<()>
{
    let fixture = Fixture::new()?;
    let main_before_jobs = fs::read(fixture.state.join("jobs/jobs.db"))?;
    for id in ["cdx-old-running", "cdx-outcome-unknown"] {
        fixture
            .store
            .create_job(job(&fixture.workspace, id, BTreeMap::new()))?;
    }
    fixture.store.mark_timed_out_unknown(
        "cdx-outcome-unknown",
        PRIVATE_PAYLOAD.into(),
        Some(json!({"raw_output": PRIVATE_PAYLOAD})),
        None,
    )?;
    assert!(
        main_before_jobs == fs::read(fixture.state.join("jobs/jobs.db"))?,
        "new records must remain in WAL for this visibility fixture"
    );
    let before = durable_bytes(&fixture.state)?;
    for (id, status) in [
        ("cdx-old-running", "running"),
        ("cdx-outcome-unknown", "timed_out_unknown"),
    ] {
        let saved = fixture.store.get_job(id)?;
        let report = diagnose(&fixture.workspace, &fixture.state, id)?;
        assert_eq!(report["observation"], "observed");
        assert_eq!(report["job"]["status"], status);
        assert_eq!(saved, fixture.store.get_job(id)?);
    }
    assert!(
        before == durable_bytes(&fixture.state)?,
        "durable DB/WAL changed"
    );
    assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    Ok(())
}

#[test]
fn closed_wal_store_is_readable_without_changing_the_job_or_schema() -> hcodex::Result<()> {
    let Fixture {
        root: _root,
        workspace,
        state,
        store,
    } = Fixture::new()?;
    store.create_job(job(&workspace, "cdx-closed-store", BTreeMap::new()))?;
    store.checkpoint()?;
    drop(store);
    assert!(
        !state.join("jobs/jobs.db-wal").exists(),
        "fixture must exercise closed WAL storage without a retained sidecar"
    );
    let before = fs::read(state.join("jobs/jobs.db"))?;
    let report = diagnose(&workspace, &state, "cdx-closed-store")?;
    assert_eq!(report["observation"], "observed");
    assert_eq!(report["job"]["status"], "running");
    assert!(
        before == fs::read(state.join("jobs/jobs.db"))?,
        "closed-store diagnostic changed durable DB content"
    );
    assert_eq!(fs::read(workspace.join("source.rs"))?, SOURCE);
    Ok(())
}

#[test]
fn hardening_named_recovery_forms_reject_before_creating_a_worktree() -> hcodex::Result<()> {
    let fixture = Fixture::new()?;
    let before = git(
        &fixture.workspace,
        &["worktree", "list", "--porcelain", "-z"],
    )?;
    let mut violations = Vec::new();
    for (index, arguments) in [
        vec!["recovery", "cdx-missing"],
        vec!["tui", "-c", "/recovery cdx-missing"],
        vec!["tui", "/recovery", "cdx-missing"],
    ]
    .into_iter()
    .enumerate()
    {
        let state = fixture
            .root
            .path()
            .join(format!("absent-named-state-{index}"));
        let output = Command::new(env!("CARGO_BIN_EXE_hcodex"))
            .arg("--workspace")
            .arg(&fixture.workspace)
            .arg("--state-root")
            .arg(&state)
            .args(["--worktree-name", "recovery-probe"])
            .args(&arguments)
            .env("HCODEX_HOME", fixture.root.path().join("absent-home"))
            .output()?;
        if output.status.success() || state.exists() {
            violations.push(format!(
                "{arguments:?}: success={}, state_created={}",
                output.status.success(),
                state.exists()
            ));
        }
    }
    let after = git(
        &fixture.workspace,
        &["worktree", "list", "--porcelain", "-z"],
    )?;
    assert!(
        violations.is_empty() && before == after,
        "recovery must reject before named-worktree effects: {}; registrations_changed={}",
        violations.join("; "),
        before != after
    );
    assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    Ok(())
}

#[test]
fn hardening_receipt_labels_cannot_echo_short_private_payloads() -> hcodex::Result<()> {
    let fixture = Fixture::new()?;
    let valid = json!({
        "status":"succeeded",
        "outcome_state":"confirmed_success",
        "requested_model":"gpt-6.1-sol",
        "actual_model":"gpt-6.1-sol",
        "model_mismatch":false,
        "owned_processes_closed":true,
    });
    let mut violations = Vec::new();
    for (index, malformed_field) in [
        None,
        Some("status"),
        Some("outcome_state"),
        Some("requested_model"),
        Some("actual_model"),
        Some("job_model"),
        Some("task_type"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("cdx-private-label-{index}");
        let secret = format!("provider failed: prompt=PRIVATE_PROMPT_{index}");
        let mut receipt = valid.clone();
        let mut request = job(&fixture.workspace, &id, BTreeMap::new());
        request.model = Some("gpt-6.1-sol".into());
        if let Some(field) = malformed_field {
            if field == "job_model" {
                request.model = Some(secret.clone());
            } else if field == "task_type" {
                request.task_type = secret.clone();
            } else {
                receipt[field] = json!(secret);
            }
        }
        fixture.store.create_job(request)?;
        fixture
            .store
            .complete_job(&id, json!({}), None, Some(receipt))?;
        let report = diagnose(&fixture.workspace, &fixture.state, &id)?;
        if malformed_field.is_none() {
            assert_eq!(report["job"]["recorded_provider_evidence"], valid);
            assert_eq!(report["job"]["model"], "gpt-6.1-sol");
        }
        if serde_json::to_string(&report)?.contains("PRIVATE_PROMPT") {
            violations.push(malformed_field.unwrap_or("valid_fixture"));
        }
    }
    assert!(
        violations.is_empty(),
        "malformed bounded fields disclosed private text: {violations:?}"
    );
    Ok(())
}

#[test]
fn foreign_workspace_and_unreadable_records_are_not_exposed_as_observed_jobs() -> hcodex::Result<()>
{
    let fixture = Fixture::new()?;
    let foreign = fixture.root.path().join("foreign-workspace");
    fs::create_dir(&foreign)?;
    fixture
        .store
        .create_job(job(&foreign, "cdx-foreign-workspace", BTreeMap::new()))?;
    fixture.store.create_job(job(
        &fixture.workspace,
        "cdx-oversized-record",
        BTreeMap::new(),
    ))?;
    fixture.store.complete_job(
        "cdx-oversized-record",
        json!({"raw_output": "x".repeat(4 * 1024 * 1024 + 1)}),
        None,
        None,
    )?;
    fixture.store.create_job(job(
        &fixture.workspace,
        "cdx-malformed-record",
        BTreeMap::new(),
    ))?;
    let catalog = SqlCatalog::parse(include_str!("../sql/recovery_diagnostics_fixture.sql"))?;
    let writer = Connection::open(fixture.state.join("jobs/jobs.db"))?;
    assert_eq!(
        writer.execute(
            catalog.query("replace_record_json", 2)?,
            params!["{interrupted", "cdx-malformed-record"],
        )?,
        1
    );
    let before = durable_bytes(&fixture.state)?;
    for id in [
        "cdx-foreign-workspace",
        "cdx-oversized-record",
        "cdx-malformed-record",
    ] {
        let report = diagnose(&fixture.workspace, &fixture.state, id)?;
        assert_ne!(report["observation"], "observed");
        assert!(report["job"].is_null());
    }
    assert!(
        before == durable_bytes(&fixture.state)?,
        "durable DB/WAL changed"
    );
    assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_database_components_are_not_followed() -> hcodex::Result<()> {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    fixture.store.create_job(job(
        &fixture.workspace,
        "cdx-linked-database",
        BTreeMap::new(),
    ))?;
    let directory_link = fixture.root.path().join("directory-link-state");
    fs::create_dir(&directory_link)?;
    symlink(fixture.state.join("jobs"), directory_link.join("jobs"))?;
    let file_link = fixture.root.path().join("file-link-state");
    fs::create_dir_all(file_link.join("jobs"))?;
    symlink(
        fixture.state.join("jobs/jobs.db"),
        file_link.join("jobs/jobs.db"),
    )?;
    let mut linked_states = vec![directory_link, file_link];
    for suffix in ["-wal", "-shm"] {
        let state = fixture.root.path().join(format!("sidecar{suffix}-state"));
        fs::create_dir_all(state.join("jobs"))?;
        for name in ["jobs.db", "jobs.db-wal", "jobs.db-shm"] {
            let source = fixture.state.join("jobs").join(name);
            let destination = state.join("jobs").join(name);
            if name.ends_with(suffix) {
                symlink(source, destination)?;
            } else {
                fs::copy(source, destination)?;
            }
        }
        linked_states.push(state);
    }
    let before = durable_bytes(&fixture.state)?;
    for state in &linked_states {
        let report = diagnose(&fixture.workspace, state, "cdx-linked-database")?;
        assert_ne!(report["observation"], "observed");
        assert!(report["job"].is_null());
        assert!(!state.join("handoffs").exists());
    }
    assert!(
        before == durable_bytes(&fixture.state)?,
        "durable DB/WAL changed"
    );
    assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    Ok(())
}

#[test]
fn lifecycle_claims_are_separate_from_current_git_and_path_observations() -> hcodex::Result<()> {
    let fixture = Fixture::new()?;
    let state = campaign_state(&fixture.state)?;
    let present = state.join("present with space");
    let removed = state.join("removed");
    let contradictory = state.join("claimed-removed-but-present");
    let registration_only = state.join("registered-but-missing");
    for path in [&present, &removed, &contradictory, &registration_only] {
        add_worktree(&fixture.workspace, path)?;
    }
    git(
        &fixture.workspace,
        &[
            "worktree",
            "remove",
            removed.to_str().ok_or("removed path is not UTF-8")?,
        ],
    )?;
    fs::remove_dir_all(&registration_only)?;
    register_campaign(&fixture, "cdx-lifecycle-observations", &state)?;
    write_lifecycle(
        &state,
        &lifecycle(
            &fixture.workspace,
            &state,
            "cdx-lifecycle-observations",
            vec![
                json!({"node_id":"present","path":present,"verified":true,"removed":false}),
                json!({"node_id":"removed","path":removed,"verified":true,"removed":true}),
                json!({"node_id":"contradictory","path":contradictory,"verified":true,"removed":true}),
                json!({"node_id":"registration_only","path":registration_only,"verified":false,"removed":false}),
            ],
        )?,
    )?;
    let before = durable_bytes(&fixture.state)?;
    let saved = fixture.store.get_job("cdx-lifecycle-observations")?;
    let report = diagnose(
        &fixture.workspace,
        &fixture.state,
        "cdx-lifecycle-observations",
    )?;
    assert_eq!(
        node(&report, "present")?["observation"],
        "present_registered"
    );
    assert_eq!(node(&report, "removed")?["observation"], "observed_removed");
    assert_eq!(node(&report, "contradictory")?["observation"], "unknown");
    assert_eq!(
        node(&report, "registration_only")?["observation"],
        "unknown"
    );
    assert_eq!(node(&report, "registration_only")?["filesystem"], "missing");
    assert_eq!(
        node(&report, "registration_only")?["git_registration"],
        "registered"
    );
    assert_eq!(saved, fixture.store.get_job("cdx-lifecycle-observations")?);
    assert!(
        before == durable_bytes(&fixture.state)?,
        "durable DB/WAL changed"
    );
    assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    assert_eq!(fs::read(present.join("source.rs"))?, SOURCE);
    assert_eq!(fs::read(contradictory.join("source.rs"))?, SOURCE);
    Ok(())
}

#[test]
fn foreign_malformed_and_oversized_lifecycle_never_becomes_current_proof() -> hcodex::Result<()> {
    let fixture = Fixture::new()?;
    let state = campaign_state(&fixture.state)?;
    register_campaign(&fixture, "cdx-invalid-lifecycle", &state)?;
    let valid = lifecycle(&fixture.workspace, &state, "cdx-invalid-lifecycle", vec![])?;
    let mut foreign_campaign = valid.clone();
    foreign_campaign["campaign_id"] = json!("cdx-another-campaign");
    let mut foreign_workspace = valid.clone();
    foreign_workspace["original"] = json!(fixture.root.path());
    let mut foreign_state = valid;
    foreign_state["state_path"] = json!(fixture.state);
    let mut bodies = vec![
        serde_json::to_vec(&foreign_campaign)?,
        serde_json::to_vec(&foreign_workspace)?,
        serde_json::to_vec(&foreign_state)?,
        b"{interrupted".to_vec(),
        vec![b'x'; 65_537],
    ];
    let saved = fixture.store.get_job("cdx-invalid-lifecycle")?;
    for body in bodies.drain(..) {
        fs::write(state.join("lifecycle.json"), &body)?;
        let report = diagnose(&fixture.workspace, &fixture.state, "cdx-invalid-lifecycle")?;
        assert_eq!(report["observation"], "observed");
        assert_eq!(report["worktrees"]["observation"], "unknown");
        assert_eq!(fs::read(state.join("lifecycle.json"))?, body);
        assert_eq!(saved, fixture.store.get_job("cdx-invalid-lifecycle")?);
    }
    assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    Ok(())
}

#[test]
fn interrupted_campaign_child() -> hcodex::Result<()> {
    let Some(root) = std::env::var_os("HCODEX_DIAGNOSTIC_TEST_ROOT") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    let workspace = root.join("workspace");
    let store = Store::open(&root.join("state"))?;
    let state = campaign_state(&root.join("state"))?;
    let path = state.join("owned-node");
    add_worktree(&workspace, &path)?;
    store.create_job(job(
        &workspace,
        "cdx-interrupted-diagnostic",
        BTreeMap::from([(
            "worktree_state_path".into(),
            state.to_str().ok_or("state path is not UTF-8")?.into(),
        )]),
    ))?;
    write_lifecycle(
        &state,
        &lifecycle(
            &workspace,
            &state,
            "cdx-interrupted-diagnostic",
            vec![json!({"node_id":"n1","path":path,"verified":true,"removed":false})],
        )?,
    )?;
    fs::File::open(state.join("lifecycle.json"))?.sync_all()?;
    fs::File::open(&state)?.sync_all()?;
    if std::env::var("HCODEX_DIAGNOSTIC_TEST_PHASE")? == "removed_before_ack" {
        git(
            &workspace,
            &[
                "worktree",
                "remove",
                path.to_str().ok_or("node path is not UTF-8")?,
            ],
        )?;
    }
    println!("HCODEX_DIAGNOSTIC_READY");
    std::io::stdout().flush()?;
    loop {
        std::thread::park();
    }
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn terminate_campaign(root: &Path, phase: &str) -> hcodex::Result<()> {
    let mut child = OwnedChild(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "interrupted_campaign_child", "--nocapture"])
            .env("HCODEX_DIAGNOSTIC_TEST_ROOT", root)
            .env("HCODEX_DIAGNOSTIC_TEST_PHASE", phase)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let stdout = child.0.stdout.take().ok_or("child stdout missing")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.is_ok_and(|line| line == "HCODEX_DIAGNOSTIC_READY") {
                let _ = sender.send(());
                break;
            }
        }
    });
    let ready = receiver.recv_timeout(Duration::from_secs(15));
    let killed = child.0.kill();
    let waited = child.0.wait();
    reader.join().map_err(|_| "child output reader panicked")?;
    ready.map_err(|_| "child did not reach durable diagnostic boundary")?;
    killed?;
    assert!(!waited?.success());
    Ok(())
}

#[test]
fn killed_campaign_is_inspected_without_replay_or_invented_cleanup() -> hcodex::Result<()> {
    for (phase, expected) in [
        ("lifecycle_before_result", "present_registered"),
        ("removed_before_ack", "unknown"),
    ] {
        let fixture = Fixture::new()?;
        terminate_campaign(fixture.root.path(), phase)?;
        let saved = fixture
            .store
            .get_job("cdx-interrupted-diagnostic")?
            .ok_or("interrupted campaign missing")?;
        assert!(
            saved.result.is_none(),
            "fixture must die before final result"
        );
        let state = PathBuf::from(
            saved
                .metadata
                .get("worktree_state_path")
                .ok_or("state link missing")?,
        );
        let before_lifecycle = fs::read(state.join("lifecycle.json"))?;
        let before_db = durable_bytes(&fixture.state)?;
        let report = diagnose(
            &fixture.workspace,
            &fixture.state,
            "cdx-interrupted-diagnostic",
        )?;
        assert_eq!(report["observation"], "observed");
        assert_eq!(report["job"]["status"], "running");
        assert_eq!(node(&report, "n1")?["observation"], expected);
        assert_eq!(
            fixture.store.get_job("cdx-interrupted-diagnostic")?,
            Some(saved)
        );
        assert!(
            before_db == durable_bytes(&fixture.state)?,
            "durable DB/WAL changed"
        );
        assert_eq!(fs::read(state.join("lifecycle.json"))?, before_lifecycle);
        assert_eq!(fs::read(fixture.workspace.join("source.rs"))?, SOURCE);
    }
    Ok(())
}
