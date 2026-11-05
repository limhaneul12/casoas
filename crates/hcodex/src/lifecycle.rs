//! Rust-native CLI skill installation and read-only backend identity observations.

mod assets;
mod config;
mod identity;
mod launcher;
mod types;

use std::path::Path;

use serde_json::{Value, json};

pub use types::ControlCommand;

/// Scoped source/disk/process observations; never a build or CI approval.
pub async fn runtime_identity(workspace: &Path) -> Value {
    let home = std::env::var_os("HCODEX_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from);
    identity::observe(workspace, home.as_deref()).await
}

pub async fn control(command: &str, repository: &Path, home: &Path) -> crate::Result<Value> {
    let command = ControlCommand::parse(command)?;
    let repository_input = repository.to_path_buf();
    let home_input = home.to_path_buf();
    let (repository, home) =
        blocking(move || Ok((repository_input.canonicalize()?, home_input.canonicalize()?)))
            .await?;
    match command {
        ControlCommand::Status => {
            let source = repository.clone();
            let identity_home = home.clone();
            let mut status = blocking(move || status(&repository, &home)).await?;
            status["runtime_identity"] = identity::observe(&source, Some(&identity_home)).await;
            Ok(status)
        }
        ControlCommand::Setup | ControlCommand::Sync => {
            let changes = blocking(move || assets::synchronize(&repository, &home)).await?;
            Ok(json!({
                "ok": changes.legacy_mcp.preserved_custom.is_empty(),
                "command": command,
                "configuration_changed": changes.legacy_mcp.removed_registrations > 0,
                "legacy_mcp": changes.legacy_mcp,
                "assets_synchronized": changes.assets_synchronized,
                "launcher_migrated": changes.launcher_migrated,
                "process_action": "none",
                "backend": "in_process",
            }))
        }
    }
}

async fn blocking<T, F>(operation: F) -> crate::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> crate::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation).await?
}

fn status(repository: &Path, home: &Path) -> crate::Result<Value> {
    let binary = config::resolve_hcodex_binary(repository)?;
    let paths = assets::InstallPaths::new(home);
    Ok(json!({
        "ok": true,
        "command": ControlCommand::Status,
        "rust_binary": binary,
        "backend": "in_process",
        "mcp_server": false,
        "gemini_skill_installed": paths.gemini_skill.join("SKILL.md").is_file(),
        "codex_skill_installed": paths.codex_skill.join("SKILL.md").is_file(),
    }))
}

#[cfg(test)]
mod tests;
