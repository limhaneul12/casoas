use super::*;

#[test]
fn health_workspace_scope_is_optional_and_remains_closed() -> crate::Result<()> {
    let legacy = crate::catalog::arguments("codex_bridge_health", json!({}))?;
    assert!(legacy["workspace_dir"].is_null());
    let scoped =
        crate::catalog::arguments("codex_bridge_health", json!({"workspace_dir":"/repo"}))?;
    assert_eq!(scoped["workspace_dir"], "/repo");
    assert!(crate::catalog::arguments("codex_bridge_health", json!({"workspace_dir":7})).is_err());
    assert!(crate::catalog::arguments("codex_bridge_health", json!({"unexpected":true})).is_err());
    Ok(())
}

#[test]
fn binary_replacement_remains_a_disk_observation() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let binary = root.path().join("hcodex");
    fs::write(&binary, b"old-binary")?;
    let before = binary_snapshot(&binary).ok_or("first disk observation missing")?;
    fs::write(&binary, b"new-binary")?;
    let after = binary_snapshot(&binary).ok_or("second disk observation missing")?;
    assert_ne!(before.sha256, after.sha256);
    assert_eq!(binary_value(Some(&after))["scope"], "current_disk_file");
    Ok(())
}
