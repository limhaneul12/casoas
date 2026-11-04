//! Rust-native plugin, skill, MCP configuration, and owned-process lifecycle.

mod assets;
mod config;
mod launcher;
mod processes;
mod types;

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

pub use processes::{
    OwnedMcpProcessGuard, register_owned_mcp_process, unregister_owned_mcp_process,
};
pub use types::{ControlCommand, OwnedProcessHandle};

pub async fn control(command: &str, repository: &Path, home: &Path) -> crate::Result<Value> {
    control_with_refresh(command, repository, home, || async {
        crate::runtime::refresh_mcp_servers_and_readback(Duration::from_secs(10)).await
    })
    .await
}

async fn control_with_refresh<F, Fut>(
    command: &str,
    repository: &Path,
    home: &Path,
    refresh: F,
) -> crate::Result<Value>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = crate::Result<Value>>,
{
    let command = ControlCommand::parse(command)?;
    let repository_input = repository.to_path_buf();
    let home_input = home.to_path_buf();
    let (repository, home) =
        blocking(move || Ok((repository_input.canonicalize()?, home_input.canonicalize()?)))
            .await?;
    match command {
        ControlCommand::Status => blocking(move || status(&repository, &home)).await,
        ControlCommand::Enable | ControlCommand::Setup => {
            let changes = blocking(move || assets::activate(&repository, &home)).await?;
            Ok(json!({
                "ok": true,
                "command": command,
                "configuration_changed": changes.configuration_changed,
                "assets_synchronized": changes.assets_synchronized,
                "launcher_migrated": changes.launcher_migrated,
                "process_action": "none",
                "restart_required": true,
            }))
        }
        ControlCommand::Sync => {
            let changes = blocking(move || assets::synchronize(&repository, &home)).await?;
            Ok(json!({
                "ok": true,
                "command": command,
                "configuration_changed": changes.configuration_changed,
                "assets_synchronized": changes.assets_synchronized,
                "launcher_migrated": changes.launcher_migrated,
                "process_action": "none",
                "restart_required": changes.configuration_changed,
            }))
        }
        ControlCommand::Disable => {
            let repository_for_config = repository.clone();
            let home_for_config = home.clone();
            let (configuration_changed, plugin_changed) = blocking(move || {
                Ok((
                    config::set_enabled(&repository_for_config, &home_for_config, false)?,
                    assets::disable_plugin(&home_for_config)?,
                ))
            })
            .await?;
            let stopped = processes::stop_owned_processes(&repository, &home).await?;
            Ok(json!({
                "ok": true,
                "command": command,
                "configuration_changed": configuration_changed,
                "plugin_changed": plugin_changed,
                "owned_processes": stopped,
                "process_action": "verified_owned_stop",
            }))
        }
        ControlCommand::Restart => {
            let stopped = processes::stop_owned_processes(&repository, &home).await?;
            let changes = blocking(move || assets::activate(&repository, &home)).await?;
            let refresh_result = refresh().await;
            let verified = refresh_result.as_ref().is_ok_and(|value| {
                value.get("ok").and_then(Value::as_bool) == Some(true)
                    && value
                        .get("readback")
                        .and_then(|readback| readback.get("clientModelConfigs"))
                        .and_then(Value::as_array)
                        .is_some()
            });
            let refresh_value = refresh_result.ok();
            let sent = refresh_value.as_ref().map(|_| true);
            Ok(json!({
                "ok": verified,
                "command": command,
                "configuration_changed": changes.configuration_changed,
                "assets_synchronized": changes.assets_synchronized,
                "launcher_migrated": changes.launcher_migrated,
                "owned_processes": stopped,
                "process_action": "verified_owned_stop",
                "refresh_request": {
                    "state": if verified { "refresh_rpc_and_catalog_readback_verified" } else { "failed_or_unknown" },
                    "sent": sent,
                    "catalog_readback_verified": verified,
                    "installed_server_readiness": "unverified",
                    "scope": "RefreshMcpServers response and GetCascadeModelConfigData readback only",
                    "result": refresh_value,
                },
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
    let config = config::inspect(home)?;
    let paths = assets::InstallPaths::new(home);
    let registered = processes::read_registry(home)?;
    Ok(json!({
        "ok": true,
        "command": ControlCommand::Status,
        "rust_binary": binary,
        "codex_mcp": config,
        "antigravity_plugin_enabled": paths.active_plugin.is_dir(),
        "antigravity_plugin_disabled": paths.disabled_plugin.is_dir(),
        "gemini_skill_installed": paths.gemini_skill.join("SKILL.md").is_file(),
        "codex_skill_installed": paths.codex_skill.join("SKILL.md").is_file(),
        "registered_owned_processes": registered,
    }))
}

#[cfg(test)]
mod tests;
