use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::types::{CodegraphSnapshot, CodegraphSymbolSlice, EvidenceDimensions, EvidenceScope};
use crate::storage::{CodegraphNode, CodegraphReader};

const NOT_RUN: &str = "not_run";
const VERIFIED: &str = "verified";
const EXECUTED_FAIL: &str = "executed_fail";
const SOURCE_BUDGET: usize = 48_000;

#[derive(Serialize)]
struct ScopeDigestPayload<'a> {
    inline_context_sha256: &'a str,
    file_hashes: &'a BTreeMap<String, String>,
    symbols: &'a [String],
}

pub(crate) fn build(workspace: &Path, args: &Value, result: &Value) -> crate::Result<Value> {
    let workspace = workspace.canonicalize()?;
    if !workspace.is_dir() {
        return Err(IoError::new(ErrorKind::InvalidInput, "workspace must be a directory").into());
    }
    let files = string_array(args, "files_inspected");
    let symbols = sorted_unique(string_array(args, "symbols_inspected"));
    let inline_context = inline_context(args);
    let inline_sha = sha256(inline_context.as_bytes());
    let (file_hashes, missing) = hash_files(&workspace, &files);
    let mut codegraph = inspect_codegraph(&workspace)?;
    let slices = symbol_slices(&workspace, &symbols, &mut codegraph);
    let (commands, execution, origin) = execution_evidence(args, result);
    let dimensions = EvidenceDimensions {
        static_code_review: if explicit_review(result) {
            VERIFIED.to_owned()
        } else {
            NOT_RUN.to_owned()
        },
        test_generation: if successful(result) && generated_tests(result) {
            VERIFIED.to_owned()
        } else {
            NOT_RUN.to_owned()
        },
        test_execution: execution,
        integration_verification: NOT_RUN.to_owned(),
        runtime_confirmation: NOT_RUN.to_owned(),
    };
    let digest_payload = ScopeDigestPayload {
        inline_context_sha256: &inline_sha,
        file_hashes: &file_hashes,
        symbols: &symbols,
    };
    let digest_bytes = serde_json::to_vec(&digest_payload)?;
    let scope = EvidenceScope {
        codegraph,
        scope_digest: format!("sha256:{}", sha256(&digest_bytes)),
        inline_context_bytes: inline_context.len(),
        inline_context_digest: format!("sha256:{inline_sha}"),
        files_inspected_digests: file_hashes,
        unverified_or_missing_files: missing,
        symbols_inspected: symbols,
        codegraph_symbol_slices: slices,
        evidence_dimensions: dimensions,
        executed_test_commands: commands,
        test_execution_evidence_origin: origin,
    };
    serde_json::to_value(scope).map_err(Into::into)
}

fn hash_files(workspace: &Path, files: &[String]) -> (BTreeMap<String, String>, Vec<String>) {
    let mut hashes = BTreeMap::new();
    let mut missing = Vec::new();
    for raw in sorted_unique(files.to_vec()) {
        let candidate = Path::new(&raw);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            workspace.join(candidate)
        };
        let resolved = joined.canonicalize();
        let valid = resolved
            .as_ref()
            .is_ok_and(|path| path.starts_with(workspace) && path.is_file());
        if !valid {
            missing.push(raw);
            continue;
        }
        match resolved.and_then(fs::read) {
            Ok(bytes) => {
                hashes.insert(raw, format!("sha256:{}", sha256(&bytes)));
            }
            Err(_) => missing.push(raw),
        }
    }
    (hashes, missing)
}

fn inspect_codegraph(workspace: &Path) -> crate::Result<CodegraphSnapshot> {
    let database = workspace.join(".codegraph/codegraph.db");
    if !database.is_file() {
        return Ok(CodegraphSnapshot {
            status: "unindexed".to_owned(),
            snapshot_id: None,
            workspace_dir: workspace.to_string_lossy().into_owned(),
            db_mtime_utc: None,
            db_size_bytes: None,
            note: Some(
                "No .codegraph/codegraph.db found in workspace; static graph unindexed.".to_owned(),
            ),
        });
    }
    let metadata = fs::metadata(&database)?;
    let modified = metadata.modified()?;
    let seconds = modified.duration_since(UNIX_EPOCH)?.as_secs();
    let timestamp: DateTime<Utc> = modified.into();
    let seed = format!("{}::{}::{seconds}", workspace.display(), metadata.len());
    Ok(CodegraphSnapshot {
        status: "indexed".to_owned(),
        snapshot_id: Some(format!("cg-{}", &sha256(seed.as_bytes())[..12])),
        workspace_dir: workspace.to_string_lossy().into_owned(),
        db_mtime_utc: Some(timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        db_size_bytes: Some(metadata.len()),
        note: None,
    })
}

fn symbol_slices(
    workspace: &Path,
    symbols: &[String],
    snapshot: &mut CodegraphSnapshot,
) -> Vec<CodegraphSymbolSlice> {
    if symbols.is_empty() || snapshot.status != "indexed" {
        return Vec::new();
    }
    let database = workspace.join(".codegraph/codegraph.db");
    let reader = match CodegraphReader::open(&database) {
        Ok(reader) => reader,
        Err(error) => {
            snapshot.note = Some(format!("CodeGraph symbol lookup unavailable: {error}"));
            return Vec::new();
        }
    };
    let mut seen = BTreeSet::new();
    let mut slices = Vec::new();
    let mut remaining = SOURCE_BUDGET;
    for symbol in symbols {
        let nodes = match reader.find_symbol(symbol) {
            Ok(nodes) => nodes,
            Err(error) => {
                snapshot.note = Some(format!("CodeGraph symbol lookup incomplete: {error}"));
                continue;
            }
        };
        for node in nodes {
            if !seen.insert(node.id.clone()) {
                continue;
            }
            let excerpt = read_excerpt(workspace, &node, remaining);
            remaining = remaining.saturating_sub(excerpt.len());
            slices.push(to_slice(node, excerpt));
        }
    }
    slices
}

fn read_excerpt(workspace: &Path, node: &CodegraphNode, budget: usize) -> String {
    if budget == 0 || node.start_line <= 0 || node.end_line < node.start_line {
        return String::new();
    }
    let Ok(path) = workspace.join(&node.file_path).canonicalize() else {
        return String::new();
    };
    if !path.starts_with(workspace) || !path.is_file() {
        return String::new();
    }
    let Ok(bytes) = fs::read(path) else {
        return String::new();
    };
    let content = String::from_utf8_lossy(&bytes);
    let start = usize::try_from(node.start_line.saturating_sub(1)).unwrap_or_default();
    let count = usize::try_from(node.end_line - node.start_line + 1).unwrap_or_default();
    let excerpt = content
        .lines()
        .skip(start)
        .take(count)
        .collect::<Vec<_>>()
        .join("\n");
    truncate_utf8(&excerpt, budget)
}

fn to_slice(node: CodegraphNode, source_excerpt: String) -> CodegraphSymbolSlice {
    let mut callers = node.callers;
    callers.sort();
    callers.dedup();
    let mut callees = node.callees;
    callees.sort();
    callees.dedup();
    CodegraphSymbolSlice {
        name: node.name,
        qualified_name: node.qualified_name,
        kind: node.kind,
        file_path: node.file_path,
        start_line: node.start_line,
        end_line: node.end_line,
        callers,
        callees,
        source_excerpt,
    }
}

fn execution_evidence(args: &Value, result: &Value) -> (Vec<Value>, String, Option<String>) {
    if let Some(gates) = authoritative_gates(result) {
        let passed = gates
            .iter()
            .all(|gate| gate.get("exit_code").and_then(Value::as_i64) == Some(0));
        return (
            gates,
            if passed { VERIFIED } else { EXECUTED_FAIL }.to_owned(),
            Some("bridge_executed".to_owned()),
        );
    }
    let mut commands = value_array(args, "executed_test_commands");
    commands.extend(value_array(args, "bridge_executed_gates"));
    let origin = (!commands.is_empty()).then(|| "caller_reported".to_owned());
    (commands, NOT_RUN.to_owned(), origin)
}

fn authoritative_gates(result: &Value) -> Option<Vec<Value>> {
    for key in ["bridge_executed_gates", "verification_gates"] {
        let gates = value_array(result, key);
        if !gates.is_empty()
            && gates.iter().all(|gate| {
                gate.get("origin").and_then(Value::as_str) == Some("bridge_executed")
                    && gate.get("exit_code").and_then(Value::as_i64).is_some()
            })
        {
            return Some(gates);
        }
    }
    None
}

fn explicit_review(result: &Value) -> bool {
    let Some(artifact) = successful_artifact(result) else {
        return false;
    };
    let verdict = artifact
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_uppercase);
    let narrative = ["summary", "executive_critique"].iter().any(|key| {
        artifact
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    });
    narrative && matches!(verdict.as_deref(), Some("PASS" | "WARN" | "BLOCK"))
}

fn generated_tests(result: &Value) -> bool {
    let Some(artifact) = successful_artifact(result) else {
        return false;
    };
    artifact
        .get("failing_test_cases")
        .and_then(Value::as_array)
        .is_some_and(|tests| !tests.is_empty())
        || artifact
            .get("unit_tests")
            .and_then(Value::as_str)
            .is_some_and(|tests| !tests.trim().is_empty())
}

fn successful_artifact(result: &Value) -> Option<&Map<String, Value>> {
    if !successful(result) {
        return None;
    }
    ["structured_output", "data"]
        .iter()
        .filter_map(|key| result.get(*key).and_then(Value::as_object))
        .find(|object| !object.is_empty())
}

fn successful(result: &Value) -> bool {
    result.get("ok").and_then(Value::as_bool) == Some(true)
        && matches!(
            result.get("status").and_then(Value::as_str),
            Some("succeeded" | "completed")
        )
}

fn inline_context(args: &Value) -> String {
    if let Some(explicit) = args.get("provided_context_or_code").and_then(Value::as_str) {
        return explicit.to_owned();
    }
    [
        "target_code_or_diff",
        "target_code_or_context",
        "code_or_context",
        "signature",
        "contract_and_constraints",
        "failure_trace",
        "task_spec",
        "campaign_goal",
    ]
    .iter()
    .filter_map(|key| args.get(*key).and_then(Value::as_str))
    .filter(|text| !text.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value_array(value, key)
        .into_iter()
        .filter_map(|item| item.as_str().map(ToOwned::to_owned))
        .collect()
}

fn value_array(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
