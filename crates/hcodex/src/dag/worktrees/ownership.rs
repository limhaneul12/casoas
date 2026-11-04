//! Immutable owner evidence and checkpoint-boundary reopening. A mutable phase
//! alone never proves admission or completion: the Store pins each safe frontier.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Inventory, Node, WorktreeCampaign, git, identity, inventory, private_files};
use crate::Result;
use crate::dag::{DagSettlementVerifier, NodeState, SettlementReceipt};
use crate::gateway::Gateway;
use crate::storage::JobRecord;

const MAX_OWNER_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FRONTIER_BYTES: u64 = 64 * 1024;
pub(super) type FrontierLease = tokio::sync::OwnedMutexGuard<Option<String>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum NodePhase {
    #[default]
    Pending,
    Preparing,
    Admitted,
    Checkpointed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    version: u8,
    origin_campaign_id: String,
    nonce: String,
    original: PathBuf,
    original_identity: (u64, u64),
    original_git_dir: PathBuf,
    original_git_dir_identity: (u64, u64),
    common: PathBuf,
    common_identity: (u64, u64),
    lock_identity: (u64, u64),
    state: PathBuf,
    state_identity: (u64, u64),
    head: String,
    initial_source: Inventory,
    private: Inventory,
    nodes: BTreeMap<String, OwnedNode>,
}

struct ReopenInput {
    original: PathBuf,
    common: PathBuf,
    state: PathBuf,
    lock_path: PathBuf,
    lock: File,
    timeout: Duration,
    campaign_id: String,
    metadata: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedNode {
    path: PathBuf,
    initial: Inventory,
    identity: (u64, u64),
    git_dir: PathBuf,
    git_dir_identity: (u64, u64),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResumeFrontier {
    schema_version: u8,
    creation_phase: String,
    pub campaign_id: String,
    pub original: PathBuf,
    head: String,
    state_path: PathBuf,
    origin_campaign_id: String,
    ownership_sha256: Option<String>,
    generation: u64,
    pub source_manifest_digest: String,
    cleanup_started: bool,
    #[serde(default)]
    previous_execution_export: Option<Value>,
    nodes: Vec<FrontierNode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontierNode {
    node_id: String,
    path: PathBuf,
    verified: bool,
    removed: bool,
    phase: NodePhase,
    checkpoint_id: Option<String>,
    verified_inventory_sha256: Option<String>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TestBoundary {
    Preparing,
    InitialGitAddedBeforeOwnerPin,
    ExportCommittedBeforeAck,
    Staged,
    PartialPublication,
    CanonicalPublished,
    CanonicalGateCompleted,
    CheckpointCommittedBeforeFrontier,
    FrontierWrittenBeforePin,
    FrontierPinnedBeforeNotify,
    CleanupRemovedBeforeAck,
}

#[cfg(test)]
type TestBoundaryHook = std::sync::Arc<dyn Fn(&str, TestBoundary) + Send + Sync>;

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct TestControl {
    pub boundary: Option<TestBoundaryHook>,
}
#[cfg(test)]
impl std::fmt::Debug for TestControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestControl").finish_non_exhaustive()
    }
}

impl WorktreeCampaign {
    pub(super) fn seal_ownership(&self) -> Result<()> {
        let nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
        let mut owned = BTreeMap::new();
        for (id, node) in nodes.iter() {
            owned.insert(
                id.clone(),
                OwnedNode {
                    path: node.path.clone(),
                    initial: node.initial.clone(),
                    identity: node.identity.ok_or("node ownership identity missing")?,
                    git_dir: node.git_dir.clone().ok_or("node Git ownership missing")?,
                    git_dir_identity: node
                        .git_dir_identity
                        .ok_or("node Git inode ownership missing")?,
                },
            );
        }
        drop(nodes);
        let owner = Ownership {
            version: 2,
            origin_campaign_id: self.campaign_id.clone(),
            nonce: self.owner_nonce.clone(),
            original: self.original.clone(),
            original_identity: self.original_identity,
            original_git_dir: self.original_git_dir.clone(),
            original_git_dir_identity: self.original_git_dir_identity,
            common: self.common.clone(),
            common_identity: self.common_identity,
            lock_identity: self.lock_identity,
            state: self.state.clone(),
            state_identity: self.state_identity,
            head: self.head.clone(),
            initial_source: self.initial_source.clone(),
            private: self.private.clone(),
            nodes: owned,
        };
        let bytes = serde_json::to_vec(&owner)?;
        if bytes.len() > MAX_OWNER_BYTES as usize {
            return Err("worktree ownership exceeds byte budget".into());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.state.join("ownership.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        File::open(&self.state)?.sync_all()?;
        *self
            .owner_digest
            .lock()
            .map_err(|_| "worktree owner lock poisoned")? = Some(digest(&bytes));
        Ok(())
    }

    pub(crate) fn provider_request_id(&self, node_id: &str) -> String {
        format!(
            "cdx-wt-{:x}",
            Sha256::digest(format!("{}\0{node_id}", self.owner_nonce).as_bytes())
        )
    }

    pub(crate) async fn mark_preparing(&self, node_id: &str) -> Result<()> {
        let transition = self.frontier.clone().lock_owned().await;
        {
            let mut nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let node = nodes.get_mut(node_id).ok_or("unknown worktree node")?;
            if node.phase != NodePhase::Pending {
                return Err("worktree node has already begun admission".into());
            }
            node.phase = NodePhase::Preparing;
        }
        let (_transition, _) = self.export_frontier(transition).await?;
        #[cfg(test)]
        self.test_boundary(node_id, TestBoundary::Preparing);
        Ok(())
    }
    pub(crate) async fn mark_admitted(&self, node_id: &str) -> Result<()> {
        let transition = self.frontier.clone().lock_owned().await;
        {
            let mut nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let node = nodes.get_mut(node_id).ok_or("unknown worktree node")?;
            if node.phase != NodePhase::Preparing {
                return Err("worktree provider admission phase mismatch".into());
            }
            node.phase = NodePhase::Admitted;
        }
        self.export_frontier(transition).await?;
        Ok(())
    }

    pub(crate) async fn initialize_frontier(&self, gateway: &Gateway) -> Result<()> {
        let pinned = self.frontier.clone().lock_owned().await;
        let (mut pinned, next) = self.export_frontier(pinned).await?;
        self.pin(gateway, pinned.as_deref(), &next).await?;
        *pinned = Some(next);
        Ok(())
    }

    pub(crate) async fn checkpoint_committed<F, N>(
        &self,
        gateway: &Gateway,
        receipt: &SettlementReceipt,
        resumed: bool,
        persistence: F,
        notify: N,
    ) -> Result<()>
    where
        F: std::future::Future<Output = Result<()>>,
        N: FnOnce() -> Result<()>,
    {
        let mut pinned = self.frontier.clone().lock_owned().await;
        persistence.await?;
        #[cfg(test)]
        self.test_boundary(
            &receipt.node_id,
            TestBoundary::CheckpointCommittedBeforeFrontier,
        );
        if receipt.state != NodeState::Settled {
            return notify();
        }
        {
            let mut nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let node = nodes
                .get_mut(&receipt.node_id)
                .ok_or("checkpoint worktree node missing")?;
            if node.verified.is_none() || (!resumed && node.phase != NodePhase::Admitted) {
                return Err("checkpoint lacks verified admitted worktree".into());
            }
            node.phase = NodePhase::Checkpointed;
            node.checkpoint_id = Some(format!("ckp-{}-{}", self.campaign_id, receipt.node_id));
        }
        if !resumed {
            *self
                .source_digest
                .lock()
                .map_err(|_| "worktree source lock poisoned")? =
                receipt.post_manifest_digest.clone();
        }
        let (next_lease, next) = self.export_frontier(pinned).await?;
        pinned = next_lease;
        #[cfg(test)]
        self.test_boundary(&receipt.node_id, TestBoundary::FrontierWrittenBeforePin);
        self.pin(gateway, pinned.as_deref(), &next).await?;
        *pinned = Some(next);
        #[cfg(test)]
        self.test_boundary(&receipt.node_id, TestBoundary::FrontierPinnedBeforeNotify);
        notify()
    }

    async fn pin(&self, gateway: &Gateway, previous: Option<&str>, next: &str) -> Result<()> {
        let campaign = self.campaign_id.clone();
        let owner = self.owner_id()?;
        let owner_hash = self
            .owner_digest
            .lock()
            .map_err(|_| "worktree owner lock poisoned")?
            .clone()
            .ok_or("worktree owner is not sealed")?;
        let state = self.state.clone();
        let previous = previous.map(str::to_owned);
        let next = next.to_owned();
        let lease = self._lock.try_clone()?;
        gateway
            .database(move |store| {
                let _lease = lease;
                store.pin_worktree_frontier(
                    &campaign,
                    &owner,
                    &owner_hash,
                    &state,
                    previous.as_deref(),
                    &next,
                )
            })
            .await
    }

    fn owner_id(&self) -> Result<String> {
        Ok(self.origin_campaign_id.clone())
    }

    pub(super) fn frontier_report(&self) -> Result<Value> {
        let nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
        let hash = self
            .owner_digest
            .lock()
            .map_err(|_| "worktree owner lock poisoned")?
            .clone();
        let origin = if hash.is_some() {
            self.owner_id()?
        } else {
            self.campaign_id.clone()
        };
        let source = self
            .source_digest
            .lock()
            .map_err(|_| "worktree source lock poisoned")?
            .clone();
        let cleanup = *self
            .cleanup_started
            .lock()
            .map_err(|_| "worktree cleanup lock poisoned")?;
        let nodes = nodes.iter().map(|(id, n)| -> Result<Value> {
            let verified_hash=n.verified_digest.clone();
            if n.verified.is_some()!=verified_hash.is_some() {return Err("verified inventory digest missing".into());}
            Ok(json!({"node_id":id,"path":n.path,"verified":n.verified.is_some(),"removed":n.removed,
                "phase":n.phase,"checkpoint_id":n.checkpoint_id,"verified_inventory_sha256":verified_hash}))
        }).collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"schema_version":2,"creation_phase":if hash.is_some(){"ready"}else{"precreating"},"campaign_id":self.campaign_id,"original":self.original,"head":self.head,
            "state_path":self.state,"origin_campaign_id":origin,"ownership_sha256":hash,"generation":self.generation,
            "source_manifest_digest":source,"cleanup_started":cleanup,"previous_execution_export":self.previous_export,"nodes":nodes}),
        )
    }

    pub(super) fn mark_cleanup_started(&self) -> Result<()> {
        *self
            .cleanup_started
            .lock()
            .map_err(|_| "worktree cleanup lock poisoned")? = true;
        Ok(())
    }

    async fn frontier_digest(&self) -> Result<String> {
        let path = self.state.join("lifecycle.json");
        tokio::task::spawn_blocking(move || Ok(digest(&read_regular(&path, MAX_FRONTIER_BYTES)?)))
            .await?
    }

    pub(crate) async fn reopen(
        original: &Path,
        state_root: &Path,
        prior: &JobRecord,
        timeout: Duration,
    ) -> Result<(Self, ResumeFrontier)> {
        let original = original.canonicalize()?;
        let root = state_root.canonicalize()?;
        let raw_state = prior
            .metadata
            .get("worktree_state_path")
            .ok_or("worktree resume lacks state locator")?;
        let state = PathBuf::from(raw_state);
        if state.parent() != Some(root.join("worktrees").as_path()) || identity(&state).is_err() {
            return Err("worktree resume state path is not owned".into());
        }
        let common_raw = git(&original, &["rev-parse", "--git-common-dir"], timeout).await?;
        let common = original.join(common_raw.trim()).canonicalize()?;
        let lock_name = format!(
            "{:x}",
            Sha256::digest(original.as_os_str().as_encoded_bytes())
        );
        let lock_path = common.join("hcodex-worktree-locks").join(lock_name);
        if !fs::symlink_metadata(&lock_path)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            return Err("worktree resume lock identity missing".into());
        }
        let lock = OpenOptions::new().read(true).write(true).open(&lock_path)?;
        lock.try_lock()
            .map_err(|_| "worktree campaign is still owned")?;
        let campaign_id = prior.job_id.clone();
        let metadata = prior.metadata.clone();
        let input = ReopenInput {
            original,
            common,
            state,
            lock_path,
            lock,
            timeout,
            campaign_id,
            metadata,
        };
        let (campaign, frontier) =
            tokio::task::spawn_blocking(move || Self::reopen_files(input)).await??;
        campaign.verify_identity(&campaign.original).await?;
        let node_checks = campaign
            .nodes
            .lock()
            .map_err(|_| "worktree lock poisoned")?
            .values()
            .map(|node| node.path.clone())
            .collect::<Vec<_>>();
        for path in node_checks {
            campaign.verify_identity(&path).await?;
        }
        Ok((campaign, frontier))
    }

    fn reopen_files(input: ReopenInput) -> Result<(Self, ResumeFrontier)> {
        let ReopenInput {
            original,
            common,
            state,
            lock_path,
            lock,
            timeout,
            campaign_id,
            metadata,
        } = input;
        let owner_bytes = read_regular(&state.join("ownership.json"), MAX_OWNER_BYTES)?;
        let owner: Ownership = serde_json::from_slice(&owner_bytes)?;
        let owner_hash = digest(&owner_bytes);
        if owner.version != 2
            || owner.original != original
            || owner.state != state
            || owner.common != common
            || metadata.get("worktree_owner_sha256") != Some(&owner_hash)
            || metadata.get("worktree_owner_campaign") != Some(&owner.origin_campaign_id)
            || identity(&original)? != owner.original_identity
            || identity(&common)? != owner.common_identity
            || identity(&state)? != owner.state_identity
            || lock_file_identity(&lock)? != owner.lock_identity
            || lock_path_identity(&lock_path)? != owner.lock_identity
            || owner.nodes.is_empty()
            || owner.nodes.len() > 8
        {
            return Err(
                "worktree immutable ownership mismatch or historical owner unavailable".into(),
            );
        }
        let frontier_bytes = read_regular(&state.join("lifecycle.json"), MAX_FRONTIER_BYTES)?;
        let frontier_hash = digest(&frontier_bytes);
        if metadata.get("worktree_frontier_sha256") != Some(&frontier_hash) {
            return Err("worktree frontier is not durably checkpointed".into());
        }
        let frontier: ResumeFrontier = serde_json::from_slice(&frontier_bytes)?;
        if frontier.schema_version != 2
            || frontier.creation_phase != "ready"
            || frontier.campaign_id != campaign_id
            || frontier.original != original
            || frontier.state_path != state
            || frontier.head != owner.head
            || frontier.cleanup_started
            || frontier.origin_campaign_id != owner.origin_campaign_id
            || frontier.ownership_sha256.as_ref() != Some(&owner_hash)
            || frontier.nodes.len() != owner.nodes.len()
        {
            return Err("worktree resume frontier identity or cleanup state mismatch".into());
        }
        let current = DagSettlementVerifier::capture(&original)?;
        if current.digest != frontier.source_manifest_digest
            || private_files(&inventory(&original)?, &current.files) != owner.private
        {
            return Err("worktree resume canonical source/private state changed".into());
        }
        let mut nodes = BTreeMap::new();
        let mut seen_paths = BTreeSet::new();
        for recorded in &frontier.nodes {
            let owned = owner
                .nodes
                .get(&recorded.node_id)
                .ok_or("resume node absent from immutable owner")?;
            if recorded.path != owned.path
                || recorded.removed
                || !seen_paths.insert(&recorded.path)
                || !matches!(recorded.phase, NodePhase::Pending | NodePhase::Checkpointed)
            {
                return Err("worktree resume node is ambiguous or already admitted".into());
            }
            let actual = inventory(&owned.path)?;
            let verified = match recorded.phase {
                NodePhase::Pending
                    if !recorded.verified
                        && recorded.checkpoint_id.is_none()
                        && actual == owned.initial =>
                {
                    None
                }
                NodePhase::Checkpointed
                    if recorded.verified
                        && recorded.checkpoint_id.is_some()
                        && recorded.verified_inventory_sha256.as_ref()
                            == Some(&inventory_digest(&actual)?) =>
                {
                    Some(actual)
                }
                _ => {
                    return Err("worktree resume node verification or initial state changed".into());
                }
            };
            nodes.insert(
                recorded.node_id.clone(),
                Node {
                    path: owned.path.clone(),
                    initial: owned.initial.clone(),
                    verified,
                    verified_digest: recorded.verified_inventory_sha256.clone(),
                    removed: false,
                    identity: Some(owned.identity),
                    git_dir: Some(owned.git_dir.clone()),
                    git_dir_identity: Some(owned.git_dir_identity),
                    phase: recorded.phase,
                    checkpoint_id: recorded.checkpoint_id.clone(),
                },
            );
        }
        let campaign = Self {
            campaign_id: campaign_id.clone(),
            origin_campaign_id: owner.origin_campaign_id.clone(),
            previous_export: frontier.previous_execution_export.clone(),
            original,
            original_identity: owner.original_identity,
            common_identity: owner.common_identity,
            lock_identity: owner.lock_identity,
            state_identity: owner.state_identity,
            original_git_dir: owner.original_git_dir,
            original_git_dir_identity: owner.original_git_dir_identity,
            common: owner.common,
            head: owner.head,
            state: owner.state,
            timeout,
            private: owner.private,
            initial_source: owner.initial_source,
            nodes: Mutex::new(nodes),
            _lock: lock,
            export_lock: Arc::new(Mutex::new(())),
            owner_nonce: owner.nonce,
            owner_digest: Mutex::new(Some(owner_hash)),
            source_digest: Mutex::new(frontier.source_manifest_digest.clone()),
            generation: frontier.generation,
            frontier: Arc::new(tokio::sync::Mutex::new(Some(frontier_hash))),
            cleanup_started: Mutex::new(false),
            #[cfg(test)]
            test_control: Mutex::new(None),
        };
        Ok((campaign, frontier))
    }

    pub(super) fn verify_lock_identity(&self) -> Result<()> {
        let name = format!(
            "{:x}",
            Sha256::digest(self.original.as_os_str().as_encoded_bytes())
        );
        let path = self.common.join("hcodex-worktree-locks").join(name);
        if lock_file_identity(&self._lock)? != self.lock_identity
            || lock_path_identity(&path)? != self.lock_identity
        {
            return Err("worktree flock ownership identity changed".into());
        }
        Ok(())
    }

    pub(crate) async fn verify_final(
        &self,
        manifest: &crate::dag::WorkspaceManifest,
    ) -> Result<()> {
        let pinned = self.frontier.clone().lock_owned().await;
        self.verify_final_locked(manifest, &pinned).await
    }

    pub(super) async fn verify_final_locked(
        &self,
        manifest: &crate::dag::WorkspaceManifest,
        pinned: &Option<String>,
    ) -> Result<()> {
        let expected = self
            .source_digest
            .lock()
            .map_err(|_| "worktree source lock poisoned")?
            .clone();
        if manifest.digest != expected || pinned.as_ref() != Some(&self.frontier_digest().await?) {
            return Err("worktree final source is not the pinned verified frontier".into());
        }
        self.verify_original(manifest).await
    }

    pub(crate) fn inherit_export(&mut self, reference: Option<Value>) {
        if reference.is_some() {
            self.previous_export = reference;
        }
    }

    pub(crate) fn previous_export(&self) -> Option<Value> {
        self.previous_export.clone()
    }

    pub(crate) async fn prepare_export(
        &self,
        gateway: &Gateway,
        export_id: &str,
        payload: Value,
    ) -> Result<Value> {
        let pinned = self.frontier.clone().lock_owned().await;
        let frontier = pinned
            .as_ref()
            .ok_or("worktree export frontier is unpinned")?
            .clone();
        let campaign = self.campaign_id.clone();
        let id = export_id.to_owned();
        let hash_lease = self._lock.try_clone()?;
        let (payload, hash) = tokio::task::spawn_blocking(move || {
            let _lease = hash_lease;
            let hash = export_payload_digest(&payload)?;
            Ok::<_, crate::Error>((payload, hash))
        })
        .await??;
        let lease = self._lock.try_clone()?;
        gateway
            .database(move |store| {
                let _lease = lease;
                store.prepare_worktree_export(&campaign, &frontier, &id, &hash)
            })
            .await?;
        Ok(payload)
    }

    pub(crate) async fn activate(&mut self, gateway: &Gateway, campaign_id: &str) -> Result<()> {
        let original = self.original.clone();
        let lease = self._lock.try_clone()?;
        let current = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            DagSettlementVerifier::capture(&original)
        })
        .await??;
        self.verify_final(&current).await?;
        self.campaign_id = campaign_id.to_owned();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("worktree generation overflow")?;
        *self.frontier.clone().lock_owned().await = None;
        self.initialize_frontier(gateway).await
    }

    #[cfg(test)]
    pub(crate) fn set_test_control(&self, control: TestControl) -> Result<()> {
        *self
            .test_control
            .lock()
            .map_err(|_| "test control lock poisoned")? = Some(control);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn test_boundary(&self, node: &str, boundary: TestBoundary) {
        let hook = self
            .test_control
            .lock()
            .ok()
            .and_then(|control| control.as_ref().and_then(|c| c.boundary.clone()));
        if let Some(hook) = hook {
            hook(node, boundary);
        }
    }
}

impl ResumeFrontier {
    pub(crate) fn checkpoint_ids(&self) -> BTreeMap<String, String> {
        self.nodes
            .iter()
            .filter_map(|node| {
                node.checkpoint_id
                    .as_ref()
                    .map(|id| (node.node_id.clone(), id.clone()))
            })
            .collect()
    }
    pub(crate) fn pending_ids(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|node| node.phase == NodePhase::Pending)
            .map(|node| node.node_id.clone())
            .collect()
    }
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > limit {
        return Err("invalid bounded worktree evidence file".into());
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err("worktree evidence replaced during open".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit as usize {
        return Err("worktree evidence byte budget exceeded".into());
    }
    Ok(bytes)
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
pub(super) fn inventory_digest(value: &Inventory) -> Result<String> {
    Ok(digest(&serde_json::to_vec(value)?))
}

pub(super) fn lock_file_identity(file: &File) -> Result<(u64, u64)> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err("worktree flock is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Err("worktree flock ownership unsupported".into())
    }
}
pub(super) fn lock_path_identity(path: &Path) -> Result<(u64, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("worktree flock path is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Err("worktree flock ownership unsupported".into())
    }
}

pub(super) fn write_frontier(state: &Path, expected: (u64, u64), bytes: &[u8]) -> Result<String> {
    if identity(state)? != expected {
        return Err("state directory replaced before export".into());
    }
    let temp = state.join(format!("lifecycle-{}.pending", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temp, state.join("lifecycle.json"))?;
    File::open(state)?.sync_all()?;
    Ok(digest(bytes))
}

/// Streaming identity for the bounded durable execution-export payload only.
pub(crate) fn export_payload_digest(payload: &Value) -> Result<String> {
    struct ExportHash {
        hash: Sha256,
        bytes: usize,
    }
    impl Write for ExportHash {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let total = self
                .bytes
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("export byte count overflow"))?;
            if total > 32 * 1024 * 1024 {
                return Err(std::io::Error::other(
                    "execution export exceeds byte budget",
                ));
            }
            self.hash.update(bytes);
            self.bytes = total;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = ExportHash {
        hash: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut sink, payload)?;
    Ok(format!("sha256:{:x}", sink.hash.finalize()))
}

#[cfg(test)]
mod digest_tests {
    use super::*;
    #[test]
    fn export_stream_identity_matches_existing_canonical_json_bytes() -> Result<()> {
        let payload = json!({"unicode":"한글\n\u{001b}","float":1.622453292,"null":null,"nested":[true,{"x":7}]});
        assert_eq!(
            export_payload_digest(&payload)?,
            digest(&serde_json::to_vec(&payload)?)
        );
        Ok(())
    }
}
