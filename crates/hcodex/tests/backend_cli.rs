//! The CLI reads the shared durable backend without a protocol server.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use hcodex::gateway::Gateway;
use serde_json::{Value, json};
use tokio::process::Command;

async fn read_cli(workspace: &Path, state: &Path, command: &str) -> hcodex::Result<Value> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(env!("CARGO_BIN_EXE_hcodex"))
            .arg("--workspace")
            .arg(workspace)
            .arg("--state-root")
            .arg(state)
            .arg(command)
            .env("HCODEX_HOME", workspace.join("home"))
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    assert!(
        output.status.success(),
        "CLI {command} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[tokio::test]
async fn cli_and_application_share_durable_handoffs_without_server() -> hcodex::Result<()> {
    let workspace = tempfile::tempdir()?;
    let state = workspace.path().join("state");
    let gateway = Gateway::new(state.clone());
    let created = gateway
        .call(
            "heterarchy_handoff_create",
            json!({
                "title":"Backend handoff",
                "spec_or_payload":"Read-only CLI integration fixture",
                "recipient":"codex",
                "workspace_dir":workspace.path()
            }),
        )
        .await?;
    assert_eq!(created["execution_completed"], false);
    let ticket = created["ticket"]["ticket_id"]
        .as_str()
        .ok_or("missing ticket ID")?;
    gateway.shutdown().await?;

    let inbox = read_cli(workspace.path(), &state, "inbox").await?;
    let tickets = inbox["tickets"].as_array().ok_or("missing inbox tickets")?;
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["ticket_id"], ticket);
    assert_eq!(tickets[0]["title"], "Backend handoff");
    let jobs = read_cli(workspace.path(), &state, "jobs").await?;
    assert_eq!(jobs["jobs"], json!([]));
    assert!(!workspace.path().join("home/.codex/config.toml").exists());
    Ok(())
}
