use std::fs::{self, OpenOptions};
use std::io::{Error as IoError, ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use toml_edit::{Array, DocumentMut, Item, Table, value};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub(crate) struct CodexMcpStatus {
    pub config_exists: bool,
    pub configured: bool,
    pub enabled: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
}

pub(crate) fn set_enabled(repository: &Path, home: &Path, enabled: bool) -> crate::Result<bool> {
    let path = home.join(".codex/config.toml");
    if !path.is_file() && !enabled {
        return Ok(false);
    }
    let previous = if path.is_file() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut document = parse_document(&previous)?;
    if enabled {
        let binary = resolve_hcodex_binary(repository)?;
        let server = server_table_mut(&mut document)?;
        server["command"] = value(binary.to_string_lossy().into_owned());
        let mut arguments = Array::new();
        arguments.push("mcp");
        server["args"] = value(arguments);
        server["enabled"] = value(true);
        remove_obsolete_python_env(server)?;
    } else if let Some(server) = existing_server_table_mut(&mut document) {
        server["enabled"] = value(false);
    } else {
        return Ok(false);
    }
    let updated = document.to_string();
    if updated == previous {
        return Ok(false);
    }
    write_config(&path, &updated)?;
    Ok(true)
}

pub(crate) fn inspect(home: &Path) -> crate::Result<CodexMcpStatus> {
    let path = home.join(".codex/config.toml");
    if !path.is_file() {
        return Ok(CodexMcpStatus {
            config_exists: false,
            configured: false,
            enabled: false,
            command: None,
            args: Vec::new(),
        });
    }
    let document = parse_document(&fs::read_to_string(path)?)?;
    let server = document
        .get("mcp_servers")
        .and_then(Item::as_table)
        .and_then(|servers| servers.get("heterarchy_codex"))
        .and_then(Item::as_table);
    let command = server
        .and_then(|table| table.get("command"))
        .and_then(Item::as_str)
        .map(ToOwned::to_owned);
    let args = server
        .and_then(|table| table.get("args"))
        .and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|entry| entry.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let enabled = server
        .and_then(|table| table.get("enabled"))
        .and_then(Item::as_bool)
        .unwrap_or(server.is_some());
    Ok(CodexMcpStatus {
        config_exists: true,
        configured: server.is_some(),
        enabled,
        command,
        args,
    })
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

fn parse_document(content: &str) -> crate::Result<DocumentMut> {
    content.parse::<DocumentMut>().map_err(|error| {
        IoError::new(
            ErrorKind::InvalidData,
            format!("invalid Codex config TOML; file was not modified: {error}"),
        )
        .into()
    })
}

fn server_table_mut(document: &mut DocumentMut) -> crate::Result<&mut Table> {
    if !document.contains_key("mcp_servers") {
        document.insert("mcp_servers", Item::Table(Table::new()));
    }
    let servers = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "mcp_servers must be a TOML table"))?;
    if !servers.contains_key("heterarchy_codex") {
        servers.insert("heterarchy_codex", Item::Table(Table::new()));
    }
    servers
        .get_mut("heterarchy_codex")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            IoError::new(
                ErrorKind::InvalidData,
                "heterarchy_codex must be a TOML table",
            )
            .into()
        })
}

fn existing_server_table_mut(document: &mut DocumentMut) -> Option<&mut Table> {
    document
        .get_mut("mcp_servers")?
        .as_table_mut()?
        .get_mut("heterarchy_codex")?
        .as_table_mut()
}

fn remove_obsolete_python_env(server: &mut Table) -> crate::Result<()> {
    const OBSOLETE: [&str; 3] = ["PYTHONPATH", "PYTHONHOME", "VIRTUAL_ENV"];
    let Some(environment) = server.get_mut("env") else {
        return Ok(());
    };
    if let Some(table) = environment.as_table_mut() {
        for key in OBSOLETE {
            let _ = table.remove(key);
        }
        return Ok(());
    }
    if let Some(table) = environment.as_inline_table_mut() {
        for key in OBSOLETE {
            let _ = table.remove(key);
        }
        return Ok(());
    }
    Err(IoError::new(
        ErrorKind::InvalidData,
        "heterarchy_codex.env must be a TOML table or inline table",
    )
    .into())
}

fn write_config(path: &Path, content: &str) -> crate::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidInput, "Codex config path has no parent"))?;
    fs::create_dir_all(parent)?;
    let existing_permissions = path.metadata().ok().map(|metadata| metadata.permissions());
    let staged = parent.join(format!(
        ".config.toml.hcodex-{}.tmp",
        Uuid::new_v4().simple()
    ));
    let write_result = (|| -> crate::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged)?;
        if let Some(permissions) = existing_permissions {
            file.set_permissions(permissions)?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
        }
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staged, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() && staged.exists() {
        let _ignored = fs::remove_file(staged);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn seed_binary(repository: &Path) -> crate::Result<()> {
        let binary = repository.join("target/release/hcodex");
        fs::create_dir_all(binary.parent().ok_or("binary parent missing")?)?;
        fs::write(&binary, "rust-binary")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(binary, fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    #[test]
    fn managed_env_table_preserves_non_python_keys_comments_and_mode() -> crate::Result<()> {
        let repository = tempdir()?;
        let home = tempdir()?;
        seed_binary(repository.path())?;
        let path = home.path().join(".codex/config.toml");
        fs::create_dir_all(path.parent().ok_or("config parent missing")?)?;
        fs::write(
            &path,
            "# root comment\n[mcp_servers.heterarchy_codex]\ncommand = \"legacy\"\nargs = []\n\n[mcp_servers.heterarchy_codex.env]\n# provider env comment\nCODEX_CLI_BIN = \"/custom/codex\"\nAPI_SENTINEL = \"synthetic-api-value\"\nPYTHONPATH = \"/obsolete\"\nPYTHONHOME = \"/obsolete-home\"\nVIRTUAL_ENV = \"/obsolete-venv\"\n",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
        }
        assert!(set_enabled(repository.path(), home.path(), true)?);
        let updated = fs::read_to_string(&path)?;
        assert!(updated.contains("# root comment"));
        assert!(updated.contains("# provider env comment"));
        let document = parse_document(&updated)?;
        let environment = document["mcp_servers"]["heterarchy_codex"]["env"]
            .as_table()
            .ok_or("env table missing")?;
        assert!(
            environment["CODEX_CLI_BIN"]
                .as_str()
                .is_some_and(|value| value == "/custom/codex")
        );
        assert!(
            environment["API_SENTINEL"]
                .as_str()
                .is_some_and(|value| value == "synthetic-api-value")
        );
        assert!(environment.get("PYTHONPATH").is_none());
        assert!(environment.get("PYTHONHOME").is_none());
        assert!(environment.get("VIRTUAL_ENV").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o640);
        }
        Ok(())
    }

    #[test]
    fn managed_inline_env_preserves_non_python_keys() -> crate::Result<()> {
        let repository = tempdir()?;
        let home = tempdir()?;
        seed_binary(repository.path())?;
        let path = home.path().join(".codex/config.toml");
        fs::create_dir_all(path.parent().ok_or("config parent missing")?)?;
        fs::write(
            &path,
            "[mcp_servers.heterarchy_codex]\ncommand = \"legacy\"\nargs = []\nenv = { CODEX_CLI_BIN = \"/inline/codex\", API_SENTINEL = \"inline-api-value\", PYTHONPATH = \"/obsolete\", VIRTUAL_ENV = \"/obsolete-venv\" } # inline comment\n",
        )?;
        assert!(set_enabled(repository.path(), home.path(), true)?);
        let updated = fs::read_to_string(path)?;
        assert!(updated.contains("# inline comment"));
        let document = parse_document(&updated)?;
        let environment = document["mcp_servers"]["heterarchy_codex"]["env"]
            .as_inline_table()
            .ok_or("inline env missing")?;
        assert!(
            environment["CODEX_CLI_BIN"]
                .as_str()
                .is_some_and(|value| value == "/inline/codex")
        );
        assert!(
            environment["API_SENTINEL"]
                .as_str()
                .is_some_and(|value| value == "inline-api-value")
        );
        assert!(environment.get("PYTHONPATH").is_none());
        assert!(environment.get("VIRTUAL_ENV").is_none());
        Ok(())
    }
}
