//! Real binary evidence for effect-free research planning and observation.
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn invoke(workspace: &Path, state: &Path, home: &Path, args: &[&str]) -> hcodex::Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_hcodex"))
        .args(["--workspace", workspace.to_str().ok_or("workspace UTF-8")?])
        .args(["--state-root", state.to_str().ok_or("state UTF-8")?])
        .args(args)
        .env("HCODEX_HOME", home)
        .env("NO_COLOR", "1")
        .output()?)
}

#[test]
fn research_plan_no_source_and_status_have_no_runtime_side_effects() -> hcodex::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = directory.path().join("workspace");
    let state = directory.path().join("absent-state");
    let home = directory.path().join("absent-home");
    std::fs::create_dir(&workspace)?;
    std::fs::write(workspace.join("scope.rs"), "pub const LIMIT: usize = 3;\n")?;
    let before = std::fs::read(workspace.join("scope.rs"))?;
    for (args, automatic_search) in [
        (
            vec![
                "research",
                "--plan-only",
                "--file",
                "scope.rs",
                "--",
                "inspect",
                "limits",
            ],
            false,
        ),
        (
            vec!["research", "--no-web", "--", "inspect", "limits"],
            false,
        ),
        (
            vec!["research", "--plan-only", "--", "inspect", "limits"],
            true,
        ),
    ] {
        let output = invoke(&workspace, &state, &home, &args)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(value["mode"], "research");
        assert_eq!(value["plan_only"], true);
        assert_eq!(value["automatic_search"], automatic_search);
        assert_eq!(value["workspace_writes"], false);
        assert!(!state.exists());
        assert!(!home.exists());
    }
    let output = invoke(
        &workspace,
        &state,
        &home,
        &["research-status", "research-missing"],
    )?;
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["read_only"], true);
    assert_eq!(value["request_id"], "research-missing");
    assert_eq!(value["error"], "missing_database");
    assert!(!state.exists());
    assert!(!home.exists());
    assert_eq!(std::fs::read(workspace.join("scope.rs"))?, before);
    Ok(())
}

#[test]
fn named_worktree_research_is_refused_before_workspace_creation() -> hcodex::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = directory.path().join("workspace");
    let state = directory.path().join("absent-state");
    let home = directory.path().join("absent-home");
    std::fs::create_dir(&workspace)?;
    for args in [
        vec![
            "--worktree-name",
            "research-must-not-create",
            "research",
            "--plan-only",
            "--",
            "inspect",
        ],
        vec![
            "--worktree-name",
            "research-must-not-create",
            "research-status",
            "research-missing",
        ],
    ] {
        let output = invoke(&workspace, &state, &home, &args)?;
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("--worktree-name cannot start research")
        );
        assert!(!state.exists());
        assert!(!home.exists());
    }
    Ok(())
}
