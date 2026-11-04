//! Actual CLI startup and persistent-name reuse, with planning only (no providers).
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn named_cli_creates_reuses_and_preserves_workspace_without_touching_original() -> hcodex::Result<()>
{
    let root = tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir(&repo)?;
    fs::write(repo.join("a.rs"), "pub fn a() {}\n")?;
    for args in [
        vec!["init", "-q"],
        vec!["add", "a.rs"],
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
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()?
                .success()
        );
    }
    let state = root.path().join("state");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_hcodex"))
            .arg("--workspace")
            .arg(&repo)
            .arg("--state-root")
            .arg(&state)
            .args([
                "--worktree-name",
                "feature-auth",
                "dag-ulw",
                "--plan-only",
                "audit source",
            ])
            .env("HCODEX_HOME", root.path().join("isolated-home"))
            .output()
    };
    let first = run()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: Value = serde_json::from_slice(&first.stdout)?;
    assert_eq!(first["named_worktree"]["created"], true);
    let path = std::path::PathBuf::from(
        first["named_worktree"]["path"]
            .as_str()
            .ok_or("named path missing")?,
    );
    assert_eq!(
        path.file_name().and_then(|s| s.to_str()),
        Some("feature-auth")
    );
    fs::write(path.join("a.rs"), "pub fn named_change() {}\n")?;
    let second = run()?;
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second: Value = serde_json::from_slice(&second.stdout)?;
    assert_eq!(second["named_worktree"]["created"], false);
    assert_eq!(
        first["named_worktree"]["path"],
        second["named_worktree"]["path"]
    );
    assert_eq!(
        fs::read_to_string(path.join("a.rs"))?,
        "pub fn named_change() {}\n"
    );
    assert_eq!(fs::read_to_string(repo.join("a.rs"))?, "pub fn a() {}\n");
    assert!(
        Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(&repo)
            .output()?
            .stdout
            .is_empty()
    );
    Ok(())
}
