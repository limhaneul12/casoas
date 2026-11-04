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

fn legacy_handle() -> crate::Result<OwnedProcessHandle> {
    Ok(serde_json::from_value(json!({
        "pid":123,"executable":"/fixture/hcodex","arguments":["mcp"],
        "start_identity":"start-a","registered_at_unix_ms":0
    }))?)
}

#[test]
fn historical_identity_is_not_filled_from_current_installed_file() -> crate::Result<()> {
    let handle = legacy_handle()?;
    let installed = BinarySnapshot {
        path: PathBuf::from("/fixture/hcodex"),
        sha256: format!("sha256:{}", "b".repeat(64)),
    };
    let view = project_process(&handle, true, Some(&installed));
    assert_eq!(view["state"], "unknown");
    assert!(view["recorded_binary_sha256"].is_null());
    assert_eq!(view["installed_comparison"], "unknown");
    Ok(())
}

#[test]
fn startup_disk_mismatch_is_comparable_only_with_live_process_fence() -> crate::Result<()> {
    let mut handle = legacy_handle()?;
    handle.startup_binary_sha256 = Some(format!("sha256:{}", "a".repeat(64)));
    handle.registrant_package_version = Some("0.2.0".into());
    let installed = BinarySnapshot {
        path: PathBuf::from("/fixture/hcodex"),
        sha256: format!("sha256:{}", "b".repeat(64)),
    };
    let live = project_process(&handle, true, Some(&installed));
    assert_eq!(live["installed_comparison"], "different_recorded_disk_hash");
    assert_eq!(live["registrant_package_version"], "0.2.0");
    let stale = project_process(&handle, false, Some(&installed));
    assert_eq!(stale["state"], "unknown");
    assert_eq!(stale["installed_comparison"], "unknown");
    assert_eq!(
        stale["recorded_binary_sha256"],
        live["recorded_binary_sha256"]
    );
    Ok(())
}

#[test]
fn binary_replacement_is_disk_observation_not_loaded_image_proof() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let binary = root.path().join("hcodex");
    fs::write(&binary, b"old-binary")?;
    let before = binary_snapshot(&binary).ok_or("first disk observation missing")?;
    fs::write(&binary, b"new-binary")?;
    let after = binary_snapshot(&binary).ok_or("second disk observation missing")?;
    assert_ne!(before.sha256, after.sha256);
    let mut handle = legacy_handle()?;
    handle.startup_binary_sha256 = Some(before.sha256);
    handle.registrant_package_version = Some("PRIVATE PAYLOAD".into());
    let view = project_process(&handle, true, Some(&after));
    assert_eq!(view["installed_comparison"], "different_recorded_disk_hash");
    assert!(view["registrant_package_version"].is_null());
    assert!(!serde_json::to_string(&view)?.contains("PRIVATE PAYLOAD"));
    Ok(())
}
