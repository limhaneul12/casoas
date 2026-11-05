use super::*;
use tempfile::tempdir;

#[test]
fn staged_config_retirement_preserves_mode_and_rejects_changed_source() -> crate::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("config.toml");
    fs::write(&path, "before")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
    }
    write_config(&path, "before", Some("after"))?;
    assert_eq!(fs::read_to_string(&path)?, "after");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o640);
    }
    assert!(write_config(&path, "before", Some("overwrite")).is_err());
    assert_eq!(fs::read_to_string(&path)?, "after");
    assert_eq!(fs::read_dir(directory.path())?.count(), 1);
    Ok(())
}

#[test]
fn unknown_env_credentials_prevent_automatic_registration_removal() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let path = home.path().join(".codex/config.toml");
    fs::create_dir_all(path.parent().ok_or("parent missing")?)?;
    let original = format!(
        "[mcp_servers.heterarchy_codex]\ncommand = {:?}\nargs = [\"mcp\"]\nenv = {{ API_SENTINEL = \"preserve\" }}\n",
        repository
            .path()
            .join("target/release/hcodex")
            .to_string_lossy()
    );
    fs::write(&path, &original)?;
    let report = retire_managed_mcp(repository.path(), home.path())?;
    assert_eq!(report.removed_registrations, 0);
    assert_eq!(report.preserved_custom, vec![path.clone()]);
    assert_eq!(fs::read_to_string(path)?, original);
    Ok(())
}

#[test]
fn mixed_json_preserves_unrelated_entries_and_user_fields() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let path = home.path().join(".gemini/antigravity/mcp_config.json");
    fs::create_dir_all(path.parent().ok_or("parent missing")?)?;
    let binary = repository.path().join("target/release/hcodex");
    let original = serde_json::json!({"userSetting":true,"mcpServers":{"heterarchy-codex":{"command":binary,"args":["mcp"]},"other":{"command":"other","env":{"API_SENTINEL":"preserve"}}}});
    fs::write(&path, serde_json::to_string(&original)?)?;
    let report = retire_managed_mcp(repository.path(), home.path())?;
    assert_eq!(report.removed_registrations, 1);
    assert_eq!(report.removed_files, 0);
    let updated: Value = serde_json::from_slice(&fs::read(path)?)?;
    assert!(updated["mcpServers"].get("heterarchy-codex").is_none());
    assert_eq!(
        updated["mcpServers"]["other"],
        original["mcpServers"]["other"]
    );
    assert_eq!(updated["userSetting"], true);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_config_is_preserved_without_following_or_replacing_it() -> crate::Result<()> {
    use std::os::unix::fs::symlink;
    let repository = tempdir()?;
    let home = tempdir()?;
    let target = home.path().join("custom-config");
    let path = home.path().join(".codex/config.toml");
    fs::create_dir_all(path.parent().ok_or("parent missing")?)?;
    fs::write(&target, "private settings")?;
    symlink(&target, &path)?;
    let report = retire_managed_mcp(repository.path(), home.path())?;
    assert_eq!(report.preserved_custom, vec![path.clone()]);
    assert_eq!(fs::read_link(path)?, target);
    assert_eq!(fs::read_to_string(target)?, "private settings");
    Ok(())
}

#[test]
fn oversized_config_fails_before_any_candidate_is_modified() -> crate::Result<()> {
    let repository = tempdir()?;
    let home = tempdir()?;
    let config = home.path().join(".codex/config.toml");
    fs::create_dir_all(config.parent().ok_or("parent missing")?)?;
    let original = format!(
        "[mcp_servers.heterarchy_codex]\ncommand = {:?}\nargs = [\"mcp\"]\n",
        repository
            .path()
            .join("target/release/hcodex")
            .to_string_lossy()
    );
    fs::write(&config, &original)?;
    let large = home.path().join(".gemini/antigravity/mcp_config.json");
    fs::create_dir_all(large.parent().ok_or("parent missing")?)?;
    fs::write(&large, vec![b' '; MAX_CONFIG_BYTES as usize + 1])?;
    assert!(retire_managed_mcp(repository.path(), home.path()).is_err());
    assert_eq!(fs::read_to_string(config)?, original);
    assert_eq!(fs::metadata(large)?.len(), MAX_CONFIG_BYTES + 1);
    Ok(())
}
