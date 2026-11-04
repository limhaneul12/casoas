use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::Result;

use super::{
    DagNode, GateProof, NodeKind, NodeState, SettlementReceipt, SettlementStage, WorkspaceManifest,
};

const IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    ".codegraph",
    ".venv",
    ".agents",
    ".omx",
    ".codex",
    ".hcodex",
    "target",
    "node_modules",
];

#[derive(Clone, Debug, Default)]
pub struct DagSettlementVerifier;

#[derive(Clone, Debug, Default)]
pub(crate) struct SettlementEvidence {
    pub wave_index: usize,
    pub duration_seconds: f64,
    pub dirty_read_files: Vec<PathBuf>,
    pub reproduction_observed: bool,
    pub output_summary: String,
    pub error: Option<String>,
}

impl DagSettlementVerifier {
    pub fn capture(root: &Path) -> Result<WorkspaceManifest> {
        let root = root.canonicalize()?;
        let mut files = BTreeMap::new();
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !ignored(entry.path()))
        {
            let entry = entry?;
            if entry.file_type().is_dir() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(&root)?.to_path_buf();
            let digest = if entry.file_type().is_symlink() {
                let target = fs::read_link(path)?;
                format!(
                    "symlink:sha256:{}",
                    hex_digest(target.as_os_str().as_encoded_bytes())
                )
            } else if entry.file_type().is_file() {
                format!("sha256:{}", hex_digest(&fs::read(path)?))
            } else {
                continue;
            };
            files.insert(relative, digest);
        }
        let digest = manifest_digest(&files);
        Ok(WorkspaceManifest {
            root,
            files,
            digest,
        })
    }

    pub fn settle(
        node: &DagNode,
        pre: &WorkspaceManifest,
        stopped: &WorkspaceManifest,
        gate: GateProof,
    ) -> SettlementReceipt {
        Self::settle_with_evidence(
            node,
            pre,
            stopped,
            gate,
            SettlementEvidence {
                reproduction_observed: node.node_kind != NodeKind::Reproduction,
                ..SettlementEvidence::default()
            },
        )
    }

    pub(crate) fn settle_with_evidence(
        node: &DagNode,
        pre: &WorkspaceManifest,
        stopped: &WorkspaceManifest,
        gate: GateProof,
        evidence: SettlementEvidence,
    ) -> SettlementReceipt {
        let changed_files = changed_files(&pre.files, &stopped.files);
        let allowed: BTreeSet<PathBuf> = node
            .write_files
            .iter()
            .filter(|path| safe_relative(path))
            .cloned()
            .collect();
        let out_of_scope_writes: Vec<PathBuf> = changed_files
            .iter()
            .filter(|path| !allowed.contains(*path))
            .cloned()
            .collect();
        let implemented_declared_write =
            node.write_files.is_empty() || changed_files.iter().any(|path| allowed.contains(path));
        let verification_accepted = if node.node_kind == NodeKind::Reproduction
            && node.reproduction_expected_exit_nonzero
        {
            evidence.reproduction_observed
        } else {
            gate.passed()
        };
        let state = if !out_of_scope_writes.is_empty() {
            NodeState::RejectedOutOfScopeWrite
        } else if !evidence.dirty_read_files.is_empty() {
            NodeState::RejectedDirtyRead
        } else if node.node_kind == NodeKind::Reproduction && !evidence.reproduction_observed {
            NodeState::RejectedReproductionNotObserved
        } else if !implemented_declared_write || !verification_accepted {
            NodeState::RejectedVerification
        } else {
            NodeState::Settled
        };
        let settlement_stage = settlement_stage(&state);
        let changed_files_copy = changed_files.clone();
        SettlementReceipt {
            node_id: node.id.clone(),
            state,
            settlement_stage,
            role: node.role.clone(),
            node_kind: node.node_kind,
            wave_index: evidence.wave_index,
            model_id: node.model.clone(),
            duration_seconds: evidence.duration_seconds,
            pre_manifest_digest: pre.digest.clone(),
            stopped_manifest_digest: stopped.digest.clone(),
            post_manifest_digest: stopped.digest.clone(),
            changed_files,
            mutated_files: changed_files_copy,
            out_of_scope_writes,
            dirty_read_files: evidence.dirty_read_files,
            verify_passed: gate.passed(),
            reproduction_observed: evidence.reproduction_observed,
            output_summary: evidence.output_summary,
            error: evidence.error,
            gate,
        }
    }
}

fn settlement_stage(state: &NodeState) -> SettlementStage {
    match state {
        NodeState::Settled => SettlementStage::Settled,
        NodeState::RejectedOutOfScopeWrite => SettlementStage::RejectedOutOfScopeWrite,
        NodeState::RejectedDirtyRead => SettlementStage::RejectedDirtyRead,
        NodeState::RejectedVerification => SettlementStage::RejectedVerifyFailed,
        NodeState::RejectedReproductionNotObserved => {
            SettlementStage::RejectedReproductionNotObserved
        }
        NodeState::BlockedUpstream => SettlementStage::BlockedUpstream,
        NodeState::FailedExecution => SettlementStage::FailedExecution,
        NodeState::Planned => SettlementStage::Queued,
        NodeState::Running | NodeState::Stopped => SettlementStage::Running,
    }
}

fn ignored(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| IGNORED_DIRECTORIES.contains(&name))
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn changed_files(
    pre: &BTreeMap<PathBuf, String>,
    post: &BTreeMap<PathBuf, String>,
) -> Vec<PathBuf> {
    let paths: BTreeSet<&PathBuf> = pre.keys().chain(post.keys()).collect();
    paths
        .into_iter()
        .filter(|path| pre.get(*path) != post.get(*path))
        .cloned()
        .collect()
}

fn manifest_digest(files: &BTreeMap<PathBuf, String>) -> String {
    let mut hasher = Sha256::new();
    for (path, digest) in files {
        let path_bytes = path.as_os_str().as_encoded_bytes();
        hasher.update((path_bytes.len() as u64).to_be_bytes());
        hasher.update(path_bytes);
        hasher.update((digest.len() as u64).to_be_bytes());
        hasher.update(digest.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests;
