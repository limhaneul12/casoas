//! Real owned-child termination against isolated fixture workspaces, never user files.
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hcodex::recovery::{RecoveryJournal, RecoveryObservation};
use hcodex::storage::{RecoveryRecord, Store};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const BEFORE: &[u8] = b"original\n";
const AFTER: &[u8] = b"intended\n";

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn record(workspace: &Path) -> hcodex::Result<RecoveryRecord> {
    Ok(RecoveryRecord {
        change_id: "crash-test".into(),
        workspace: workspace.canonicalize()?,
        relative_path: PathBuf::from("source.rs"),
        before_sha256: hash(BEFORE),
        after_sha256: hash(AFTER),
        before_len: u64::try_from(BEFORE.len())?,
        after_len: u64::try_from(AFTER.len())?,
    })
}

#[test]
fn recovery_child() -> hcodex::Result<()> {
    let Some(root) = std::env::var_os("HCODEX_RECOVERY_TEST_ROOT") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    let workspace = root.join("workspace");
    let store = Store::open(&root.join("state"))?;
    let journal = RecoveryJournal::open(&workspace, &store)?;
    let phase = std::env::var("HCODEX_RECOVERY_TEST_PHASE")?;
    if phase == "intent" {
        store.register_recovery(&record(&workspace)?)?;
    } else {
        journal.prepare("crash-test", Path::new("source.rs"), BEFORE, AFTER)?;
        if phase == "source" {
            // Controlled fixture actor, not a new production source-apply path.
            let mut file = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(workspace.join("source.rs"))?;
            file.write_all(AFTER)?;
            file.sync_all()?;
        }
    }
    println!("HCODEX_RECOVERY_READY");
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

fn terminate_at(root: &Path, phase: &str) -> hcodex::Result<()> {
    let mut child = OwnedChild(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "recovery_child", "--nocapture"])
            .env("HCODEX_RECOVERY_TEST_ROOT", root)
            .env("HCODEX_RECOVERY_TEST_PHASE", phase)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let stdout = child.0.stdout.take().ok_or("child stdout missing")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.is_ok_and(|line| line == "HCODEX_RECOVERY_READY") {
                let _ = sender.send(());
                break;
            }
        }
    });
    let ready = receiver.recv_timeout(Duration::from_secs(15));
    let killed = child.0.kill();
    let waited = child.0.wait();
    reader.join().map_err(|_| "child output reader panicked")?;
    ready.map_err(|_| "child did not reach requested durable boundary")?;
    killed?;
    assert!(!waited?.success());
    Ok(())
}

#[test]
fn terminated_process_reopens_exact_intent_without_replaying_source() -> hcodex::Result<()> {
    for phase in ["intent", "archive", "source"] {
        let root = tempdir()?;
        let workspace = root.path().join("workspace");
        fs::create_dir(&workspace)?;
        fs::write(workspace.join("source.rs"), BEFORE)?;
        terminate_at(root.path(), phase)?;
        let store = Store::open(&root.path().join("state"))?;
        assert_eq!(store.get_recovery("crash-test")?, Some(record(&workspace)?));
        let journal = RecoveryJournal::open(&workspace, &store)?;
        if phase == "intent" {
            assert!(journal.inspect("crash-test").is_err());
            assert!(!workspace.join(".hcodex").exists());
            assert_eq!(fs::read(workspace.join("source.rs"))?, BEFORE);
        } else {
            let expected = if phase == "source" {
                RecoveryObservation::AfterImageObservedUnverified
            } else {
                RecoveryObservation::BeforeImageObserved
            };
            assert_eq!(journal.inspect("crash-test")?, expected);
            let original = fs::read(workspace.join("source.rs"))?;
            assert_eq!(original, if phase == "source" { AFTER } else { BEFORE });
            fs::write(workspace.join("source.rs"), b"external change\n")?;
            assert_eq!(
                journal.inspect("crash-test")?,
                RecoveryObservation::Conflict
            );
            assert_eq!(fs::read(workspace.join("source.rs"))?, b"external change\n");
            fs::write(
                workspace.join(".hcodex/changes/crash-test/before"),
                b"tampered",
            )?;
            assert!(journal.inspect("crash-test").is_err());
        }
    }
    Ok(())
}

#[test]
fn private_recovery_state_is_excluded_and_cannot_be_patched() -> hcodex::Result<()> {
    use hcodex::dag::{DagSettlementVerifier, PatchArtifact, PatchEdit, apply_scoped_patch};
    let root = tempdir()?;
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace)?;
    fs::write(workspace.join("source.rs"), BEFORE)?;
    let pre = DagSettlementVerifier::capture(&workspace)?;
    let store = Store::open(&root.path().join("state"))?;
    let journal = RecoveryJournal::open(&workspace, &store)?;
    assert!(journal.inspect("missing").is_err());
    assert!(!workspace.join(".hcodex").exists());
    let prepared = journal.prepare("one", Path::new("source.rs"), BEFORE, AFTER)?;
    assert_eq!(
        journal.prepare("one", Path::new("source.rs"), BEFORE, AFTER)?,
        prepared
    );
    assert!(
        journal
            .prepare("one", Path::new("source.rs"), BEFORE, b"different")
            .is_err()
    );
    assert_eq!(pre, DagSettlementVerifier::capture(&workspace)?);
    let private = PathBuf::from(".hcodex/changes/one/before");
    assert!(
        apply_scoped_patch(
            &workspace,
            std::slice::from_ref(&private),
            &pre,
            &PatchArtifact {
                edits: vec![PatchEdit {
                    path: private.clone(),
                    preimage_sha256: "sha256:absent".into(),
                    new_content: "overwrite".into()
                }],
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(workspace.join(private))?, BEFORE);
    let contexts = hcodex::context::discover_relevance(&workspace, "source audit", 20)?;
    assert!(contexts.files.iter().all(|p| !p.starts_with(".hcodex")));
    Ok(())
}
