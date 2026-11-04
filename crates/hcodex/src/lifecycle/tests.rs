use std::fs;
use std::path::{Path, PathBuf};

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{control, control_with_refresh};

#[cfg(unix)]
#[tokio::test]
async fn setup_migrates_shadowing_python_launcher_and_preserves_custom_commands()
-> crate::Result<()> {
    use std::os::unix::fs::symlink;

    let repository = tempdir()?;
    let home = tempdir()?;
    let binary = seed_repository(repository.path())?;
    let repository_path = repository.path().canonicalize()?;
    let launcher = home.path().join(".local/bin/hcodex");
    fs::create_dir_all(launcher.parent().ok_or("launcher parent missing")?)?;
    fs::write(
        &launcher,
        format!(
            "#!/bin/sh\nexport PYTHONPATH=\"{}\"\nexec \"{}/.venv/bin/python3\" -m app.core.ctl \"$@\"\n",
            repository_path.display(),
            repository_path.display(),
        ),
    )?;
    let setup = control("setup", repository.path(), home.path()).await?;
    assert_eq!(setup["launcher_migrated"], true);
    assert_eq!(fs::read_link(&launcher)?, binary.canonicalize()?);
    let repeat = control("sync", repository.path(), home.path()).await?;
    assert_eq!(repeat["launcher_migrated"], false);

    fs::remove_file(&launcher)?;
    let custom = "#!/bin/sh\nexec /custom/hcodex \"$@\"\n";
    fs::write(&launcher, custom)?;
    control("setup", repository.path(), home.path()).await?;
    assert_eq!(fs::read_to_string(&launcher)?, custom);
    fs::remove_file(&launcher)?;
    let external = home.path().join("custom-command");
    fs::write(&external, custom)?;
    symlink(&external, &launcher)?;
    control("sync", repository.path(), home.path()).await?;
    assert_eq!(fs::read_link(&launcher)?, external);
    assert_eq!(fs::read_to_string(&launcher)?, custom);
    Ok(())
}

fn seed_repository(root: &Path) -> crate::Result<PathBuf> {
    let plugin = root.join("plugin/heterarchy-codex");
    let skill = plugin.join("skills/codex-heterarchy");
    let agents = skill.join("agents");
    let binary = root.join("target/release/hcodex");
    fs::create_dir_all(&agents)?;
    fs::create_dir_all(plugin.join("rules"))?;
    fs::create_dir_all(binary.parent().ok_or("binary parent is missing")?)?;
    fs::write(
        plugin.join("plugin.json"),
        include_bytes!("../../assets/heterarchy-codex/plugin.json"),
    )?;
    fs::write(
        plugin.join("mcp_config.json"),
        include_bytes!("../../assets/heterarchy-codex/mcp_config.json"),
    )?;
    fs::write(
        plugin.join("rules/AGENTS.md"),
        include_bytes!("../../assets/heterarchy-codex/rules/AGENTS.md"),
    )?;
    fs::write(
        skill.join("SKILL.md"),
        include_bytes!("../../assets/heterarchy-codex/skills/codex-heterarchy/SKILL.md"),
    )?;
    fs::write(
        agents.join("openai.yaml"),
        include_bytes!("../../assets/heterarchy-codex/skills/codex-heterarchy/agents/openai.yaml"),
    )?;
    fs::write(&binary, "rust-binary-fixture")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    }
    Ok(binary)
}

fn seed_standalone_binary(root: &Path) -> crate::Result<PathBuf> {
    let binary = root.join("target/release/hcodex");
    fs::create_dir_all(binary.parent().ok_or("binary parent is missing")?)?;
    fs::write(&binary, "standalone-rust-binary-fixture")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    }
    Ok(binary)
}

#[tokio::test]
async fn setup_and_disable_preserve_user_toml_and_use_rust_binary() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let binary = seed_repository(repository.path())?;
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().ok_or("config parent is missing")?)?;
    fs::write(
        &config,
        "# user comment\nmodel = \"gpt-6.1-sol\"\n\n[permissions]\nmode = \"strict\"\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o640))?;
    }

    let setup = control("install", repository.path(), home.path()).await?;
    assert_eq!(setup["ok"], true);
    assert_eq!(setup["process_action"], "none");
    let content = fs::read_to_string(&config)?;
    assert!(content.contains("# user comment"));
    assert!(content.contains("[permissions]"));
    assert!(content.contains("mode = \"strict\""));
    assert!(content.contains(&binary.to_string_lossy().into_owned()));
    assert!(content.contains("args = [\"mcp\"]"));
    assert!(!content.contains("PYTHONPATH"));
    assert!(!content.contains(".venv"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o640);
    }
    let staged_files = fs::read_dir(config.parent().ok_or("config parent is missing")?)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".config.toml.hcodex-")
        })
        .count();
    assert_eq!(staged_files, 0);
    let plugin = home.path().join(".gemini/config/plugins/heterarchy-codex");
    let mcp_json = fs::read_to_string(plugin.join("mcp_config.json"))?;
    assert!(mcp_json.contains(&binary.to_string_lossy().into_owned()));
    assert!(!mcp_json.contains("python"));
    assert!(
        home.path()
            .join(".codex/skills/codex-heterarchy/SKILL.md")
            .is_file()
    );
    assert!(
        home.path()
            .join(".gemini/config/skills/codex-heterarchy/SKILL.md")
            .is_file()
    );

    let disabled = control("stop", repository.path(), home.path()).await?;
    assert_eq!(disabled["ok"], true);
    assert_eq!(disabled["owned_processes"], serde_json::json!([]));
    assert!(!plugin.exists());
    assert!(
        home.path()
            .join(".gemini/config/plugins-disabled/heterarchy-codex")
            .is_dir()
    );
    let disabled_content = fs::read_to_string(config)?;
    assert!(disabled_content.contains("enabled = false"));
    assert!(disabled_content.contains("# user comment"));
    Ok(())
}

#[tokio::test]
async fn sync_and_restart_report_process_and_refresh_boundaries() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let _ = seed_repository(repository.path())?;
    let enabled = control("on", repository.path(), home.path()).await?;
    assert_eq!(enabled["restart_required"], true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let config = home.path().join(".codex/config.toml");
        assert_eq!(fs::metadata(config)?.permissions().mode() & 0o777, 0o600);
    }
    let installed = home.path().join(".codex/skills/codex-heterarchy/SKILL.md");
    fs::write(&installed, "stale")?;
    let synced = control("refresh", repository.path(), home.path()).await?;
    assert_eq!(synced["assets_synchronized"], true);
    assert!(fs::read_to_string(installed)?.contains("# Rust Heterarchy Codex"));
    let restarted = control_with_refresh("reload", repository.path(), home.path(), || async {
        Err("fixture refresh unavailable".into())
    })
    .await?;
    assert_eq!(restarted["ok"], false);
    assert!(restarted["refresh_request"]["sent"].is_null());
    assert_eq!(restarted["refresh_request"]["state"], "failed_or_unknown");
    let status = control("st", repository.path(), home.path()).await?;
    assert_eq!(status["codex_mcp"]["enabled"], true);
    assert_eq!(status["antigravity_plugin_enabled"], true);
    Ok(())
}

#[tokio::test]
async fn restart_calls_real_refresh_rpc_and_verifies_catalog_readback() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let _ = seed_repository(repository.path())?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (method, body) in [
            ("RefreshMcpServers", r#"{"refreshed":true}"#),
            ("GetCascadeModelConfigData", r#"{"clientModelConfigs":[]}"#),
        ] {
            let (mut stream, _) = listener.accept().await?;
            let mut buffer = vec![0_u8; 8192];
            let count = stream.read(&mut buffer).await?;
            let request = String::from_utf8_lossy(&buffer[..count]).into_owned();
            if !request.contains(method)
                || !request
                    .to_ascii_lowercase()
                    .contains("x-codeium-csrf-token: fixture-token")
            {
                return Err::<Vec<String>, std::io::Error>(std::io::Error::other(
                    "unexpected refresh RPC request",
                ));
            }
            requests.push(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
            stream.shutdown().await?;
        }
        Ok::<_, std::io::Error>(requests)
    });
    let endpoint = crate::runtime::AntigravityEndpoint {
        base_url: format!("http://{address}"),
        csrf_token: "fixture-token".to_owned(),
        pid: std::process::id(),
    };
    let result = control_with_refresh("restart", repository.path(), home.path(), || async move {
        crate::runtime::AntigravityClient::new()?
            .refresh_at(&endpoint, std::time::Duration::from_secs(2))
            .await
    })
    .await?;
    assert_eq!(result["ok"], true);
    assert_eq!(result["refresh_request"]["sent"], true);
    assert_eq!(result["refresh_request"]["catalog_readback_verified"], true);
    assert_eq!(
        result["refresh_request"]["installed_server_readiness"],
        "unverified"
    );
    assert_eq!(
        result["refresh_request"]["result"]["csrf_token_redacted"],
        true
    );
    assert!(!serde_json::to_string(&result)?.contains("fixture-token"));
    assert_eq!(server.await??.len(), 2);
    Ok(())
}

#[tokio::test]
async fn invalid_toml_is_left_untouched() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let _ = seed_repository(repository.path())?;
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().ok_or("config parent is missing")?)?;
    let invalid = "[broken\n# preserve me\n";
    fs::write(&config, invalid)?;
    assert!(control("on", repository.path(), home.path()).await.is_err());
    assert_eq!(fs::read_to_string(config)?, invalid);
    Ok(())
}

#[tokio::test]
async fn standalone_setup_uses_embedded_assets_without_original_checkout() -> crate::Result<()> {
    let standalone_project = tempdir()?;
    let home = tempdir()?;
    let binary = seed_standalone_binary(standalone_project.path())?;
    assert!(!standalone_project.path().join("plugin").exists());
    let result = control("setup", standalone_project.path(), home.path()).await?;
    assert_eq!(result["ok"], true);
    let plugin = home.path().join(".gemini/config/plugins/heterarchy-codex");
    for path in [
        "mcp_config.json",
        "plugin.json",
        "rules/AGENTS.md",
        "skills/codex-heterarchy/SKILL.md",
        "skills/codex-heterarchy/agents/openai.yaml",
    ] {
        assert!(plugin.join(path).is_file(), "missing embedded asset {path}");
    }
    let config = fs::read_to_string(home.path().join(".codex/config.toml"))?;
    assert!(config.contains(&binary.to_string_lossy().into_owned()));
    assert!(!config.contains("python"));
    let skill = home.path().join(".codex/skills/codex-heterarchy/SKILL.md");
    fs::write(&skill, "stale")?;
    let sync = control("sync", standalone_project.path(), home.path()).await?;
    assert_eq!(sync["assets_synchronized"], true);
    assert!(fs::read_to_string(skill)?.contains("# Rust Heterarchy Codex"));
    Ok(())
}
