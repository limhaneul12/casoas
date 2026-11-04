//! Restart-time observations over retained lifecycle evidence, never cleanup authority.

use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const MAX_LIFECYCLE_BYTES: u64 = 64 * 1024;
const MAX_NODES: usize = 8;

#[derive(Deserialize)]
struct Lifecycle {
    campaign_id: String,
    original: PathBuf,
    head: String,
    state_path: PathBuf,
    nodes: Vec<RecordedNode>,
}

#[derive(Deserialize)]
struct RecordedNode {
    node_id: String,
    path: PathBuf,
    verified: bool,
    removed: bool,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Observation {
    Observed,
    PresentRegistered,
    ObservedRemoved,
    Unknown,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Presence {
    Present,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Registration {
    Registered,
    Absent,
}

#[derive(Serialize)]
pub(crate) struct Inspection {
    observation: Observation,
    reason: &'static str,
    state_path: Option<PathBuf>,
    nodes: Vec<NodeObservation>,
}

#[derive(Eq, PartialEq, Serialize)]
struct NodeObservation {
    node_id: String,
    path: PathBuf,
    recorded_verified: bool,
    recorded_removed: bool,
    filesystem: Presence,
    git_registration: Registration,
    observation: Observation,
}

struct LifecycleRead {
    lifecycle: Lifecycle,
    bytes: Vec<u8>,
    metadata: Metadata,
}

/// The persisted lifecycle omits inode ownership proofs. Matching observations
/// describe presence/removal only and never authorize cleanup or resume.
pub(crate) async fn inspect(
    state_root: &Path,
    workspace: &Path,
    job_id: &str,
    state_path: &Path,
) -> Inspection {
    let root = state_root.to_path_buf();
    let original = workspace.to_path_buf();
    let campaign = job_id.to_owned();
    let state = state_path.to_path_buf();
    let before =
        tokio::task::spawn_blocking(move || read_lifecycle(&root, &original, &campaign, &state))
            .await;
    let before = match before {
        Ok(Ok(value)) => value,
        Ok(Err(reason)) => return unknown(reason, None),
        Err(_) => return unknown("inspection_unavailable", None),
    };
    let registrations = match registration_snapshot(workspace).await {
        Ok(value) => value,
        Err(()) => return unknown("git_registration_unavailable", Some(state_path)),
    };
    let observed_registrations = registrations.clone();
    let observed = tokio::task::spawn_blocking(move || {
        let nodes = observe_nodes(&before.lifecycle.nodes, &observed_registrations);
        (before, nodes)
    })
    .await;
    let (before, nodes) = match observed {
        Ok(value) => value,
        Err(_) => return unknown("inspection_unavailable", Some(state_path)),
    };
    let root = state_root.to_path_buf();
    let original = workspace.to_path_buf();
    let campaign = job_id.to_owned();
    let state = state_path.to_path_buf();
    let observed_registrations = registrations.clone();
    let after = tokio::task::spawn_blocking(move || {
        let after = read_lifecycle(&root, &original, &campaign, &state)?;
        let stable = before.bytes == after.bytes
            && same_file(&before.metadata, &after.metadata)
            && nodes == observe_nodes(&after.lifecycle.nodes, &observed_registrations);
        Ok::<_, &'static str>((stable, nodes))
    })
    .await;
    let nodes = match after {
        Ok(Ok((true, nodes))) => nodes,
        _ => return unknown("observation_changed", Some(state_path)),
    };
    if registration_snapshot(workspace).await.as_ref() != Ok(&registrations) {
        return unknown("observation_changed", Some(state_path));
    }
    Inspection {
        observation: Observation::Observed,
        reason: "lifecycle_observed",
        state_path: Some(state_path.to_path_buf()),
        nodes,
    }
}

fn observe_nodes(
    nodes: &[RecordedNode],
    registrations: &BTreeSet<PathBuf>,
) -> Vec<NodeObservation> {
    nodes
        .iter()
        .map(|node| {
            let filesystem = match fs::symlink_metadata(&node.path) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Presence::Present,
                Ok(_) => Presence::Unknown,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Presence::Missing,
                Err(_) => Presence::Unknown,
            };
            let registered = registrations.contains(&node.path);
            let observation = match (filesystem, registered, node.removed) {
                (Presence::Present, true, false) => Observation::PresentRegistered,
                (Presence::Missing, false, true) => Observation::ObservedRemoved,
                _ => Observation::Unknown,
            };
            NodeObservation {
                node_id: node.node_id.clone(),
                path: node.path.clone(),
                recorded_verified: node.verified,
                recorded_removed: node.removed,
                filesystem,
                git_registration: if registered {
                    Registration::Registered
                } else {
                    Registration::Absent
                },
                observation,
            }
        })
        .collect()
}

fn unknown(reason: &'static str, state_path: Option<&Path>) -> Inspection {
    Inspection {
        observation: Observation::Unknown,
        reason,
        state_path: state_path.map(Path::to_path_buf),
        nodes: Vec::new(),
    }
}

async fn registration_snapshot(workspace: &Path) -> Result<BTreeSet<PathBuf>, ()> {
    let text = super::git(
        workspace,
        &["worktree", "list", "--porcelain", "-z"],
        Duration::from_secs(5),
    )
    .await
    .map_err(|_| ())?;
    let registrations: BTreeSet<_> = text
        .split('\0')
        .filter_map(|entry| entry.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect();
    if registrations.is_empty() || registrations.len() > 256 {
        return Err(());
    }
    Ok(registrations)
}

fn read_lifecycle(
    state_root: &Path,
    workspace: &Path,
    job_id: &str,
    state_path: &Path,
) -> Result<LifecycleRead, &'static str> {
    let root = state_root
        .canonicalize()
        .map_err(|_| "invalid_state_path")?;
    let parent = root.join("worktrees");
    if !normal_absolute(state_path)
        || state_path.parent() != Some(parent.as_path())
        || !real_directory(&parent)
        || !real_directory(state_path)
    {
        return Err("invalid_state_path");
    }
    let path = state_path.join("lifecycle.json");
    let before = fs::symlink_metadata(&path).map_err(|_| "lifecycle_unavailable")?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > MAX_LIFECYCLE_BYTES {
        return Err("invalid_lifecycle_file");
    }
    let mut file = File::open(&path).map_err(|_| "lifecycle_unavailable")?;
    let opened = file.metadata().map_err(|_| "lifecycle_unavailable")?;
    if !same_file(&before, &opened) {
        return Err("observation_changed");
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_LIFECYCLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "lifecycle_unavailable")?;
    let after = fs::symlink_metadata(&path).map_err(|_| "observation_changed")?;
    if bytes.len() > MAX_LIFECYCLE_BYTES as usize || !same_file(&opened, &after) {
        return Err("observation_changed");
    }
    let lifecycle: Lifecycle = serde_json::from_slice(&bytes).map_err(|_| "invalid_lifecycle")?;
    if lifecycle.campaign_id != job_id
        || lifecycle.original != workspace
        || lifecycle.state_path != state_path
        || lifecycle.head.is_empty()
        || lifecycle.head.len() > 128
        || lifecycle.nodes.len() > MAX_NODES
    {
        return Err("lifecycle_identity_mismatch");
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for node in &lifecycle.nodes {
        if node.node_id.is_empty()
            || node.node_id.len() > 256
            || node.node_id.chars().any(char::is_control)
            || !normal_absolute(&node.path)
            || node.path.parent() != Some(state_path)
            || !ids.insert(&node.node_id)
            || !paths.insert(&node.path)
        {
            return Err("invalid_lifecycle_node");
        }
    }
    Ok(LifecycleRead {
        lifecycle,
        bytes,
        metadata: opened,
    })
}

fn normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.len() == right.len()
            && left.modified().ok() == right.modified().ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        false
    }
}
