use std::fs;

use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

use super::build_scope;

#[tokio::test]
async fn scope_is_deterministic_and_caller_tests_remain_not_run() -> crate::Result<()> {
    let workspace = tempdir()?;
    fs::write(
        workspace.path().join("module.rs"),
        "fn reviewed() -> bool { true }\n",
    )?;
    let args = json!({
        "target_code_or_diff": "fn reviewed() -> bool { true }",
        "files_inspected": ["missing.rs", "module.rs"],
        "symbols_inspected": ["reviewed", "reviewed"],
        "executed_test_commands": [{"command": "cargo test", "passed": true}],
        "bridge_executed_gates": [{"command": "cargo test", "exit_code": 0}],
    });
    let result = json!({
        "ok": true,
        "status": "succeeded",
        "structured_output": {
            "verdict": "PASS",
            "summary": "Reviewed the supplied function.",
            "unit_tests": "#[test] fn reviewed_is_true() {}",
        }
    });
    let first = build_scope(workspace.path(), &args, &result).await?;
    let second = build_scope(workspace.path(), &args, &result).await?;
    assert_eq!(first, second);
    assert!(
        first["scope_digest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        first["files_inspected_digests"]["module.rs"]
            .as_str()
            .is_some()
    );
    assert_eq!(first["unverified_or_missing_files"], json!(["missing.rs"]));
    assert_eq!(first["symbols_inspected"], json!(["reviewed"]));
    assert_eq!(
        first["evidence_dimensions"]["static_code_review"],
        "verified"
    );
    assert_eq!(first["evidence_dimensions"]["test_generation"], "verified");
    assert_eq!(first["evidence_dimensions"]["test_execution"], "not_run");
    assert_eq!(first["test_execution_evidence_origin"], "caller_reported");
    Ok(())
}

#[tokio::test]
async fn outside_and_symlink_escape_paths_are_missing() -> crate::Result<()> {
    let workspace = tempdir()?;
    let outside = tempdir()?;
    let secret = outside.path().join("secret.rs");
    fs::write(&secret, "secret")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, workspace.path().join("escape.rs"))?;
    let args = json!({
        "files_inspected": [secret.to_string_lossy(), "../outside.rs", "escape.rs"],
        "provided_context_or_code": "",
    });
    let scope = build_scope(workspace.path(), &args, &json!({"ok": false})).await?;
    assert!(
        scope["files_inspected_digests"]
            .as_object()
            .is_some_and(|map| map.is_empty())
    );
    let missing = scope["unverified_or_missing_files"]
        .as_array()
        .ok_or("missing files must be an array")?;
    assert_eq!(missing.len(), 3);
    assert_eq!(
        scope["evidence_dimensions"]["static_code_review"],
        "not_run"
    );
    Ok(())
}

#[tokio::test]
async fn codegraph_slices_source_ranges_and_one_hop_edges() -> crate::Result<()> {
    let workspace = tempdir()?;
    fs::write(
        workspace.path().join("lib.rs"),
        "fn caller() {}\nfn target() { callee(); }\nfn callee() {}\n",
    )?;
    let graph_dir = workspace.path().join(".codegraph");
    fs::create_dir_all(&graph_dir)?;
    let connection = Connection::open(graph_dir.join("codegraph.db"))?;
    connection.execute_batch(
        "CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL, file_path TEXT NOT NULL, language TEXT NOT NULL, start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, start_column INTEGER NOT NULL, end_column INTEGER NOT NULL, docstring TEXT, signature TEXT, visibility TEXT, is_exported INTEGER DEFAULT 0, is_async INTEGER DEFAULT 0, is_static INTEGER DEFAULT 0, is_abstract INTEGER DEFAULT 0, decorators TEXT, type_parameters TEXT, return_type TEXT, updated_at INTEGER NOT NULL);\
         CREATE TABLE edges (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL, metadata TEXT, line INTEGER, col INTEGER, provenance TEXT DEFAULT NULL, FOREIGN KEY (source) REFERENCES nodes(id) ON DELETE CASCADE, FOREIGN KEY (target) REFERENCES nodes(id) ON DELETE CASCADE);\
         INSERT INTO nodes (id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES ('function:target', 'function', 'target', 'crate.target', 'lib.rs', 'rust', 2, 2, 1, 2, 1);\
         INSERT INTO nodes (id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES ('function:caller', 'function', 'caller', 'crate.caller', 'lib.rs', 'rust', 1, 1, 1, 2, 1);\
         INSERT INTO nodes (id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES ('function:callee', 'function', 'callee', 'crate.callee', 'lib.rs', 'rust', 3, 3, 1, 2, 1);\
         INSERT INTO edges (source,target,kind) VALUES ('function:caller', 'function:target', 'calls');\
         INSERT INTO edges (source,target,kind) VALUES ('function:target', 'function:callee', 'calls');",
    )?;
    drop(connection);
    let scope = build_scope(
        workspace.path(),
        &json!({"symbols_inspected": ["target"]}),
        &json!({"ok": true, "status": "completed", "data": {"implementation": "done"}}),
    )
    .await?;
    assert_eq!(scope["codegraph"]["status"], "indexed");
    let slice = &scope["codegraph_symbol_slices"][0];
    assert_eq!(slice["start_line"], 2);
    assert_eq!(slice["end_line"], 2);
    assert_eq!(slice["callers"], json!(["caller"]));
    assert_eq!(slice["callees"], json!(["callee"]));
    assert_eq!(slice["source_excerpt"], "fn target() { callee(); }");
    assert_eq!(
        scope["evidence_dimensions"]["static_code_review"],
        "not_run"
    );
    Ok(())
}

#[tokio::test]
async fn only_result_side_bridge_receipts_promote_execution() -> crate::Result<()> {
    let workspace = tempdir()?;
    let result = json!({
        "ok": true,
        "status": "completed",
        "data": {"summary": "Review", "verdict": "WARN"},
        "verification_gates": [
            {"command": "cargo test", "exit_code": 0, "origin": "bridge_executed"}
        ]
    });
    let scope = build_scope(workspace.path(), &json!({}), &result).await?;
    assert_eq!(scope["evidence_dimensions"]["test_execution"], "verified");
    assert_eq!(scope["test_execution_evidence_origin"], "bridge_executed");
    Ok(())
}
