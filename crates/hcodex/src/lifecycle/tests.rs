use super::control;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[tokio::test]
async fn identity_status_preserves_legacy_registry_and_source_scope() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let binary = seed_repository(repository.path())?;
    fs::create_dir_all(repository.path().join("src"))?;
    fs::write(
        repository.path().join("src/lib.rs"),
        "const SECRET: &str = \"PRIVATE_SOURCE_BODY\";\n",
    )?;
    for args in [
        vec!["init", "-q"],
        vec!["add", "src/lib.rs"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        let output = std::process::Command::new("git")
            .env_clear()
            .env("PATH", crate::runtime::enriched_command_path())
            .env("HOME", home.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "--no-optional-locks",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.fsmonitor=false",
            ])
            .args(args)
            .current_dir(repository.path())
            .output()?;
        assert!(output.status.success());
    }
    let registry = home.path().join(".cache/hcodex/owned-mcp-processes.json");
    fs::create_dir_all(registry.parent().ok_or("registry parent missing")?)?;
    let legacy = serde_json::to_vec(&serde_json::json!([{
        "pid":std::process::id(), "executable":binary,
        "arguments":["mcp"], "start_identity":"old-or-reused-pid",
        "registered_at_unix_ms":0
    }]))?;
    fs::write(&registry, &legacy)?;
    let status = control("status", repository.path(), home.path()).await?;
    assert_eq!(status["runtime_identity"]["source"]["state"], "observed");
    assert!(status["runtime_identity"].get("registered_mcp").is_none());
    assert_eq!(
        status["runtime_identity"]["capabilities"]["backend"],
        "in_process"
    );
    assert_eq!(
        status["runtime_identity"]["verification"]["source_to_binary"],
        "unverified"
    );
    assert_eq!(fs::read(&registry)?, legacy);
    assert!(!serde_json::to_string(&status)?.contains("PRIVATE_SOURCE_BODY"));
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub const CHANGED: bool = true;\n",
    )?;
    let changed = control("status", repository.path(), home.path()).await?;
    assert_eq!(changed["runtime_identity"]["source"]["dirty"], true);
    assert_ne!(
        status["runtime_identity"]["source"]["manifest_digest"],
        changed["runtime_identity"]["source"]["manifest_digest"]
    );
    assert_eq!(fs::read(&registry)?, legacy);
    Ok(())
}

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
    fs::create_dir_all(binary.parent().ok_or("binary parent is missing")?)?;
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
async fn backend_cutover_setup_never_registers_mcp() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    seed_repository(repository.path())?;
    let result = control("setup", repository.path(), home.path()).await?;
    assert_eq!(result["ok"], true);
    assert!(!home.path().join(".codex/config.toml").exists());
    assert!(
        !home
            .path()
            .join(".gemini/config/plugins/heterarchy-codex")
            .exists()
    );
    assert!(
        home.path()
            .join(".codex/skills/codex-heterarchy/SKILL.md")
            .is_file()
    );
    Ok(())
}

#[tokio::test]
async fn backend_cutover_removes_only_exact_managed_registration() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let binary = seed_repository(repository.path())?;
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().ok_or("config parent missing")?)?;
    fs::write(
        &config,
        format!(
            "# user comment\nmodel = \"gpt-6.1-sol\"\n[mcp_servers.heterarchy_codex]\ncommand = {:?}\nargs = [\"mcp\"]\n[mcp_servers.other]\ncommand = \"other-mcp\"\nenv = {{ API_SENTINEL = \"keep-this\" }}\n",
            binary.to_string_lossy()
        ),
    )?;
    control("sync", repository.path(), home.path()).await?;
    let updated = fs::read_to_string(&config)?;
    assert!(!updated.contains("[mcp_servers.heterarchy_codex]"));
    assert!(updated.contains("# user comment"));
    assert!(updated.contains("API_SENTINEL = \"keep-this\""));
    assert!(updated.contains("[mcp_servers.other]"));
    Ok(())
}

#[tokio::test]
async fn backend_cutover_preserves_custom_same_name_registration() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    seed_repository(repository.path())?;
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().ok_or("config parent missing")?)?;
    let original = "# custom integration\n[mcp_servers.heterarchy_codex]\ncommand = \"/custom/hcodex\"\nargs = [\"mcp\"]\nenv = { API_SENTINEL = \"preserve-me\" }\n";
    fs::write(&config, original)?;
    control("setup", repository.path(), home.path()).await?;
    assert_eq!(fs::read_to_string(config)?, original);
    Ok(())
}

#[tokio::test]
async fn standalone_setup_is_idempotent_and_preserves_user_skill_extras() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    seed_standalone_binary(repository.path())?;
    let first = control("setup", repository.path(), home.path()).await?;
    assert_eq!(first["ok"], true);
    let skill = home.path().join(".codex/skills/codex-heterarchy");
    fs::write(skill.join("user-notes.md"), "keep me")?;
    fs::write(skill.join("SKILL.md"), "stale installed skill")?;
    let synced = control("sync", repository.path(), home.path()).await?;
    assert_eq!(synced["ok"], true);
    assert_eq!(synced["configuration_changed"], false);
    assert!(fs::read_to_string(skill.join("SKILL.md"))?.starts_with("---\nname: codex-heterarchy"));
    assert_eq!(fs::read_to_string(skill.join("user-notes.md"))?, "keep me");
    assert!(!home.path().join(".codex/config.toml").exists());
    assert!(
        !home
            .path()
            .join(".gemini/config/plugins/heterarchy-codex")
            .exists()
    );
    for retired in ["on", "off", "restart", "enable", "disable", "reload"] {
        assert!(
            control(retired, repository.path(), home.path())
                .await
                .is_err()
        );
    }
    Ok(())
}

#[tokio::test]
async fn managed_plugin_registration_is_removed_but_custom_files_remain() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    seed_repository(repository.path())?;
    let plugin = home.path().join(".gemini/config/plugins/heterarchy-codex");
    fs::create_dir_all(&plugin)?;
    fs::write(
        plugin.join("mcp_config.json"),
        r#"{"mcpServers":{"heterarchy-codex":{"command":"hcodex","args":["mcp"]}}}"#,
    )?;
    fs::write(plugin.join("user-notes.md"), "preserve custom plugin note")?;
    let result = control("setup", repository.path(), home.path()).await?;
    assert_eq!(result["legacy_mcp"]["removed_registrations"], 1);
    assert_eq!(result["legacy_mcp"]["removed_files"], 1);
    assert!(!plugin.join("mcp_config.json").exists());
    assert_eq!(
        fs::read_to_string(plugin.join("user-notes.md"))?,
        "preserve custom plugin note"
    );
    Ok(())
}

#[tokio::test]
async fn malformed_config_is_left_untouched_before_skill_installation() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    seed_repository(repository.path())?;
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().ok_or("parent missing")?)?;
    let invalid = "[broken\n# preserve me\n";
    fs::write(&config, invalid)?;
    assert!(
        control("setup", repository.path(), home.path())
            .await
            .is_err()
    );
    assert_eq!(fs::read_to_string(config)?, invalid);
    assert!(!home.path().join(".codex/skills/codex-heterarchy").exists());
    Ok(())
}

#[tokio::test]
async fn custom_plugin_registration_is_reported_without_mutation() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    seed_repository(repository.path())?;
    let plugin = home.path().join(".gemini/config/plugins/heterarchy-codex");
    fs::create_dir_all(&plugin)?;
    let config = plugin.join("mcp_config.json");
    let original = r#"{"mcpServers":{"heterarchy-codex":{"command":"/custom/hcodex","args":["mcp"],"env":{"API_SENTINEL":"keep-me"}}}}"#;
    fs::write(&config, original)?;
    let result = control("setup", repository.path(), home.path()).await?;
    assert_eq!(result["ok"], false);
    assert_eq!(result["legacy_mcp"]["removed_registrations"], 0);
    assert_eq!(
        result["legacy_mcp"]["preserved_custom"][0],
        config.to_string_lossy().as_ref()
    );
    assert_eq!(fs::read_to_string(config)?, original);
    Ok(())
}
