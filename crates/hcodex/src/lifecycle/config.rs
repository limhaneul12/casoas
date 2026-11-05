//! Exact retirement of managed legacy registrations; no new MCP configuration.

use std::fs::{self, OpenOptions};
use std::io::{Error as IoError, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use toml_edit::{DocumentMut, Item};
use uuid::Uuid;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Default, Serialize)]
pub(crate) struct LegacyMcpCleanup {
    pub removed_registrations: usize,
    pub removed_files: usize,
    pub preserved_custom: Vec<PathBuf>,
}

struct ConfigEdit {
    path: PathBuf,
    original: String,
    replacement: Option<String>,
    registrations: usize,
}

pub(crate) fn retire_managed_mcp(
    repository: &Path,
    home: &Path,
) -> crate::Result<LegacyMcpCleanup> {
    let mut report = LegacyMcpCleanup::default();
    let mut edits = Vec::new();
    plan_codex_retirement(repository, home, &mut report, &mut edits)?;
    for relative in [
        ".gemini/antigravity/mcp_config.json",
        ".gemini/antigravity-cli/mcp_config.json",
        ".gemini/config/plugins/heterarchy-codex/mcp_config.json",
        ".gemini/config/plugins-disabled/heterarchy-codex/mcp_config.json",
    ] {
        let plugin_config = relative.starts_with(".gemini/config/plugins");
        plan_json_retirement(
            repository,
            home,
            relative,
            plugin_config,
            &mut report,
            &mut edits,
        )?;
    }
    // Preflight every candidate before modifying any file. Each commit is an atomic
    // replacement; this is not an atomic transaction across independent clients.
    for edit in edits {
        write_config(&edit.path, &edit.original, edit.replacement.as_deref())?;
        report.removed_registrations += edit.registrations;
        report.removed_files += usize::from(edit.replacement.is_none());
    }
    Ok(report)
}

fn plan_codex_retirement(
    repository: &Path,
    home: &Path,
    report: &mut LegacyMcpCleanup,
    edits: &mut Vec<ConfigEdit>,
) -> crate::Result<()> {
    let path = home.join(".codex/config.toml");
    let Some(original) = read_config(&path, report)? else {
        return Ok(());
    };
    let mut document = original.parse::<DocumentMut>().map_err(|_| {
        IoError::new(
            ErrorKind::InvalidData,
            "invalid Codex config TOML; file was not modified",
        )
    })?;
    let Some(servers) = document.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
        return Ok(());
    };
    let Some(server) = servers.get("heterarchy_codex") else {
        return Ok(());
    };
    let managed = server.as_table().is_some_and(|table| {
        table
            .iter()
            .all(|(key, _)| matches!(key, "command" | "args" | "enabled" | "env"))
            && table
                .get("command")
                .and_then(Item::as_str)
                .is_some_and(|command| managed_command(repository, command, false))
            && table
                .get("args")
                .and_then(Item::as_array)
                .is_some_and(|args| {
                    args.len() == 1 && args.get(0).and_then(toml_edit::Value::as_str) == Some("mcp")
                })
            && table.get("env").is_none_or(managed_toml_env)
    });
    if !managed {
        report.preserved_custom.push(path);
        return Ok(());
    }
    let _removed = servers.remove("heterarchy_codex");
    edits.push(ConfigEdit {
        path,
        original,
        replacement: Some(document.to_string()),
        registrations: 1,
    });
    Ok(())
}

fn managed_toml_env(item: &Item) -> bool {
    let allowed = |key: &str| {
        matches!(
            key,
            "CODEX_CLI_BIN" | "PYTHONPATH" | "PYTHONHOME" | "VIRTUAL_ENV"
        )
    };
    if let Some(table) = item.as_table() {
        return table.iter().all(|(key, _)| allowed(key));
    }
    item.as_inline_table()
        .is_some_and(|table| table.iter().all(|(key, _)| allowed(key)))
}

fn plan_json_retirement(
    repository: &Path,
    home: &Path,
    relative: &str,
    allow_bare: bool,
    report: &mut LegacyMcpCleanup,
    edits: &mut Vec<ConfigEdit>,
) -> crate::Result<()> {
    let path = home.join(relative);
    let Some(original) = read_config(&path, report)? else {
        return Ok(());
    };
    let mut document: Value = serde_json::from_str(&original).map_err(|_| {
        IoError::new(
            ErrorKind::InvalidData,
            "invalid legacy MCP JSON; file was not modified",
        )
    })?;
    let Some(servers) = document
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    let mut removed = 0;
    for key in ["heterarchy-codex", "heterarchy_codex", "hcodex"] {
        let Some(server) = servers.get(key) else {
            continue;
        };
        let managed = server.as_object().is_some_and(|entry| {
            entry
                .keys()
                .all(|key| matches!(key.as_str(), "command" | "args" | "disabled"))
                && entry
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| managed_command(repository, command, allow_bare))
                && entry
                    .get("args")
                    .and_then(Value::as_array)
                    .is_some_and(|args| args.as_slice() == [Value::String("mcp".to_owned())])
        });
        if managed {
            let _removed = servers.remove(key);
            removed += 1;
        } else if !report.preserved_custom.contains(&path) {
            report.preserved_custom.push(path.clone());
        }
    }
    if removed == 0 {
        return Ok(());
    }
    let servers_empty = servers.is_empty();
    let only_servers = document.as_object().is_some_and(|object| object.len() == 1);
    let replacement = if servers_empty && only_servers {
        None
    } else {
        Some(format!("{}\n", serde_json::to_string_pretty(&document)?))
    };
    edits.push(ConfigEdit {
        path,
        original,
        replacement,
        registrations: removed,
    });
    Ok(())
}

fn managed_command(repository: &Path, command: &str, allow_bare: bool) -> bool {
    if allow_bare && command == "hcodex" {
        return true;
    }
    let candidate = Path::new(command);
    if !candidate.is_absolute() {
        return false;
    }
    let mut managed = vec![
        repository.join("target/release/hcodex"),
        repository.join("target/debug/hcodex"),
    ];
    if let Ok(binary) = resolve_hcodex_binary(repository) {
        managed.push(binary);
    }
    managed.into_iter().any(|known| candidate == known || matches!((candidate.canonicalize(), known.canonicalize()), (Ok(actual), Ok(expected)) if actual == expected))
}

fn read_config(path: &Path, report: &mut LegacyMcpCleanup) -> crate::Result<Option<String>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        report.preserved_custom.push(path.to_path_buf());
        return Ok(None);
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "legacy config exceeds retirement size limit",
        )
        .into());
    }
    Ok(Some(read_bounded(path)?))
}

fn read_bounded(path: &Path) -> crate::Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "legacy config exceeds retirement size limit",
        )
        .into());
    }
    String::from_utf8(bytes)
        .map_err(|_| IoError::new(ErrorKind::InvalidData, "legacy config is not UTF-8").into())
}

pub(crate) fn resolve_hcodex_binary(repository: &Path) -> crate::Result<PathBuf> {
    let current = std::env::current_exe()?;
    let is_hcodex = matches!(
        current.file_name().and_then(|name| name.to_str()),
        Some("hcodex" | "hcodex.exe")
    );
    if is_hcodex && is_executable(&current) {
        return current.canonicalize().map_err(Into::into);
    }
    for candidate in [
        repository.join("target/release/hcodex"),
        repository.join("target/debug/hcodex"),
    ] {
        if is_executable(&candidate) {
            return candidate.canonicalize().map_err(Into::into);
        }
    }
    Err(IoError::new(
        ErrorKind::NotFound,
        "Rust hcodex binary not found under target/release, target/debug, or current executable",
    )
    .into())
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn write_config(path: &Path, previous: &str, content: Option<&str>) -> crate::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidInput, "config path has no parent"))?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || read_bounded(path)? != previous {
        return Err(IoError::new(
            ErrorKind::WouldBlock,
            "legacy config changed before retirement; preserved",
        )
        .into());
    }
    let Some(content) = content else {
        fs::remove_file(path)?;
        fs::File::open(parent)?.sync_all()?;
        return Ok(());
    };
    let staged = parent.join(format!(".config.hcodex-{}.tmp", Uuid::new_v4().simple()));
    let result = (|| -> crate::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged)?;
        file.set_permissions(metadata.permissions())?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if !fs::symlink_metadata(path)?.is_file() || read_bounded(path)? != previous {
            return Err(IoError::new(
                ErrorKind::WouldBlock,
                "legacy config changed before retirement; preserved",
            )
            .into());
        }
        fs::rename(&staged, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if staged.exists() {
        fs::remove_file(staged)?;
    }
    result
}

#[cfg(test)]
mod tests;
