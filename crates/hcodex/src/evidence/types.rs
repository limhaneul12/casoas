use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodegraphSnapshot {
    pub status: String,
    pub snapshot_id: Option<String>,
    pub workspace_dir: String,
    pub db_mtime_utc: Option<String>,
    pub db_size_bytes: Option<u64>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodegraphSymbolSlice {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
    pub source_excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvidenceDimensions {
    pub static_code_review: String,
    pub test_generation: String,
    pub test_execution: String,
    pub integration_verification: String,
    pub runtime_confirmation: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EvidenceScope {
    pub codegraph: CodegraphSnapshot,
    pub scope_digest: String,
    pub inline_context_bytes: usize,
    pub inline_context_digest: String,
    pub files_inspected_digests: BTreeMap<String, String>,
    pub unverified_or_missing_files: Vec<String>,
    pub symbols_inspected: Vec<String>,
    pub codegraph_symbol_slices: Vec<CodegraphSymbolSlice>,
    pub evidence_dimensions: EvidenceDimensions,
    pub executed_test_commands: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_execution_evidence_origin: Option<String>,
}
