//! Evidence scopes grounded in workspace files, CodeGraph, and explicit execution receipts.

mod builder;
mod types;

use std::path::Path;

use serde_json::Value;

pub use types::{CodegraphSnapshot, CodegraphSymbolSlice, EvidenceDimensions, EvidenceScope};

pub async fn build_scope(workspace: &Path, args: &Value, result: &Value) -> crate::Result<Value> {
    let workspace = workspace.to_path_buf();
    let args = args.clone();
    let result = result.clone();
    tokio::task::spawn_blocking(move || builder::build(&workspace, &args, &result)).await?
}

#[cfg(test)]
mod tests;
