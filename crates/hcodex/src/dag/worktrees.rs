//! Owned detached node worktrees. Drop deliberately retains all work and evidence.
use super::{DagSettlementVerifier, WorkspaceManifest};
use crate::Result;
use crate::git::run as git;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;
use walkdir::WalkDir;

pub(crate) mod inspection;
mod ownership;
pub(crate) use ownership::export_payload_digest;
#[cfg(test)]
pub(crate) use ownership::{TestBoundary, TestControl};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 4096;
type Inventory = BTreeMap<PathBuf, String>;
mod named;
pub use named::{NamedWorktree, open_named_worktree};
#[derive(Debug, Clone)]
struct Node {
    path: PathBuf,
    initial: Inventory,
    verified: Option<Inventory>,
    verified_digest: Option<String>,
    removed: bool,
    identity: Option<(u64, u64)>,
    git_dir: Option<PathBuf>,
    git_dir_identity: Option<(u64, u64)>,
    phase: ownership::NodePhase,
    checkpoint_id: Option<String>,
}
#[derive(Debug)]
pub(crate) struct WorktreeCampaign {
    campaign_id: String,
    origin_campaign_id: String,
    previous_export: Option<Value>,
    original: PathBuf,
    original_identity: (u64, u64),
    common_identity: (u64, u64),
    lock_identity: (u64, u64),
    state_identity: (u64, u64),
    original_git_dir: PathBuf,
    original_git_dir_identity: (u64, u64),
    common: PathBuf,
    head: String,
    state: PathBuf,
    timeout: Duration,
    private: Inventory,
    initial_source: Inventory,
    nodes: Mutex<BTreeMap<String, Node>>,
    _lock: File,
    export_lock: Arc<Mutex<()>>,
    owner_nonce: String,
    owner_digest: Mutex<Option<String>>,
    source_digest: Mutex<String>,
    generation: u64,
    cleanup_started: Mutex<bool>,
    frontier: Arc<tokio::sync::Mutex<Option<String>>>,
    #[cfg(test)]
    test_control: Mutex<Option<TestControl>>,
}
impl WorktreeCampaign {
    pub(crate) async fn start(
        original: &Path,
        state_root: &Path,
        node_ids: &[String],
        timeout: Duration,
        campaign_id: &str,
    ) -> Result<Self> {
        Self::start_registered(
            original,
            state_root,
            node_ids,
            timeout,
            campaign_id,
            None,
            #[cfg(test)]
            None,
        )
        .await
    }
    pub(crate) async fn start_registered(
        original: &Path,
        state_root: &Path,
        node_ids: &[String],
        timeout: Duration,
        campaign_id: &str,
        gateway: Option<&crate::gateway::Gateway>,
        #[cfg(test)] test_control: Option<TestControl>,
    ) -> Result<Self> {
        let timeout = timeout.min(Duration::from_secs(30));
        let original = original.canonicalize()?;
        if node_ids.is_empty() || node_ids.len() > 8 {
            return Err("worktree node count must be 1..8".into());
        }
        let unique: std::collections::BTreeSet<_> = node_ids.iter().collect();
        if unique.len() != node_ids.len() {
            return Err("duplicate worktree node identity".into());
        }
        let top = PathBuf::from(
            git(&original, &["rev-parse", "--show-toplevel"], timeout)
                .await?
                .trim(),
        )
        .canonicalize()?;
        if top != original {
            return Err("worktree mode requires Git top-level workspace".into());
        }
        let common_raw = git(&original, &["rev-parse", "--git-common-dir"], timeout).await?;
        let common = original.join(common_raw.trim()).canonicalize()?;
        let lock_dir = common.join("hcodex-worktree-locks");
        real_directory(&lock_dir)?;
        let name = format!(
            "{:x}",
            Sha256::digest(original.as_os_str().as_encoded_bytes())
        );
        let lock_path = lock_dir.join(name);
        if fs::symlink_metadata(&lock_path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("invalid worktree lock".into());
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock.try_lock()
            .map_err(|e| format!("workspace worktree campaign already active: {e}"))?;
        let lock_identity = ownership::lock_file_identity(&lock)?;
        if ownership::lock_path_identity(&lock_path)? != lock_identity {
            return Err("worktree lock path replaced during acquisition".into());
        }
        require_clean_original(&original, timeout).await?;
        let head = git(&original, &["rev-parse", "HEAD"], timeout)
            .await?
            .trim()
            .to_owned();
        let tracked = git(&original, &["ls-files", "--stage"], timeout).await?;
        if tracked.lines().any(|l| l.starts_with("160000 ")) {
            return Err("submodules unsupported in worktree mode".into());
        }
        let sparse = git(
            &original,
            &[
                "config",
                "--bool",
                "--default",
                "false",
                "--get",
                "core.sparseCheckout",
            ],
            timeout,
        )
        .await?;
        if sparse.trim() != "false" {
            return Err("sparse checkout unsupported".into());
        }
        validate_source_budget(&original)?;
        let initial_manifest = DagSettlementVerifier::capture(&original)?;
        let source_digest = initial_manifest.digest;
        let initial_source = initial_manifest.files;
        let mut all = initial_source.clone();
        let mut total = 0_u64;
        for path in all.keys() {
            total = total
                .checked_add(fs::metadata(original.join(path))?.len())
                .ok_or("worktree input size overflow")?;
        }
        let names = git(&original, &["ls-files", "-z"], timeout).await?;
        for raw in names.split('\0').filter(|s| !s.is_empty()) {
            let p = PathBuf::from(raw);
            if !safe_relative(&p) {
                return Err("unsafe tracked path".into());
            }
            if !all.contains_key(&p) {
                total = total
                    .checked_add(fs::symlink_metadata(original.join(&p))?.len())
                    .ok_or("worktree input size overflow")?;
                if total > MAX_BYTES as u64 || all.len() >= MAX_FILES {
                    return Err("tracked worktree input budget exceeded".into());
                }
                all.insert(p.clone(), file_digest(&original.join(p))?);
            }
        }
        let private = private_files(&all, &initial_source);
        fs::create_dir_all(state_root)?;
        let state_root = state_root.canonicalize()?;
        if state_root.starts_with(&original) {
            return Err("worktree state must live outside original workspace".into());
        }
        let parent = state_root.join("worktrees");
        real_directory(&parent)?;
        let state = parent.join(Uuid::new_v4().to_string());
        fs::create_dir(&state)?;
        let original_identity = identity(&original)?;
        let common_identity = identity(&common)?;
        let state_identity = identity(&state)?;
        let original_git_dir = PathBuf::from(
            git(&original, &["rev-parse", "--absolute-git-dir"], timeout)
                .await?
                .trim(),
        )
        .canonicalize()?;
        let original_git_dir_identity = identity(&original_git_dir)?;
        let campaign = Self {
            campaign_id: campaign_id.to_owned(),
            origin_campaign_id: campaign_id.to_owned(),
            previous_export: None,
            original_identity,
            common_identity,
            lock_identity,
            state_identity,
            original_git_dir,
            original_git_dir_identity,
            original,
            common,
            head,
            state,
            timeout,
            private,
            initial_source,
            nodes: Mutex::new(BTreeMap::new()),
            _lock: lock,
            export_lock: Arc::new(Mutex::new(())),
            owner_nonce: Uuid::new_v4().to_string(),
            owner_digest: Mutex::new(None),
            source_digest: Mutex::new(source_digest),
            generation: 0,
            cleanup_started: Mutex::new(false),
            frontier: Arc::new(tokio::sync::Mutex::new(None)),
            #[cfg(test)]
            test_control: Mutex::new(test_control),
        };
        campaign.plan_nodes(node_ids)?;
        let transition = campaign.frontier.clone().lock_owned().await;
        campaign.export_frontier(transition).await?;
        if let Some(gateway) = gateway {
            let id = campaign_id.to_owned();
            let state_path = campaign.state_path().to_path_buf();
            gateway
                .database(move |store| store.attach_worktree_state(&id, &state_path))
                .await?;
        }
        if let Err(error) = campaign.create_nodes(node_ids, &all).await {
            return Err(format!(
                "worktree setup failed; inspect retained state {}: {error}",
                campaign.state.display()
            )
            .into());
        }
        // The blocking operation owns the campaign and its actual flock lease
        // until immutable owner serialization/write/fsync completes.
        tokio::task::spawn_blocking(move || {
            campaign.seal_ownership()?;
            campaign.export()?;
            Ok(campaign)
        })
        .await?
    }
    fn plan_nodes(&self, node_ids: &[String]) -> Result<()> {
        let mut nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
        for id in node_ids {
            nodes.insert(
                id.clone(),
                Node {
                    path: self.state.join(Uuid::new_v4().to_string()),
                    initial: BTreeMap::new(),
                    verified: None,
                    verified_digest: None,
                    removed: false,
                    identity: None,
                    git_dir: None,
                    git_dir_identity: None,
                    phase: ownership::NodePhase::Pending,
                    checkpoint_id: None,
                },
            );
        }
        drop(nodes);
        Ok(())
    }
    async fn create_nodes(&self, node_ids: &[String], all: &Inventory) -> Result<()> {
        let timeout = self.timeout;
        for id in node_ids {
            require_clean_original(&self.original, timeout).await?;
            let path = self
                .nodes
                .lock()
                .map_err(|_| "worktree lock poisoned")?
                .get(id)
                .ok_or("planned worktree node missing")?
                .path
                .clone();
            let path_str = path.to_str().ok_or("non UTF8 worktree path")?;
            git(
                &self.original,
                &["worktree", "add", "--detach", path_str, &self.head],
                timeout,
            )
            .await?;
            #[cfg(test)]
            self.test_boundary(id, TestBoundary::InitialGitAddedBeforeOwnerPin);
            let node_identity = identity(&path)?;
            let node_git_dir = PathBuf::from(
                git(&path, &["rev-parse", "--absolute-git-dir"], timeout)
                    .await?
                    .trim(),
            )
            .canonicalize()?;
            {
                let mut nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
                let n = nodes.get_mut(id).ok_or("node disappeared")?;
                n.identity = Some(node_identity);
                n.git_dir_identity = Some(identity(&node_git_dir)?);
                n.git_dir = Some(node_git_dir);
            }
            let initial = inventory(&path)?;
            if &initial != all {
                return Err(
                    "worktree checkout differs from canonical source/private baseline".into(),
                );
            }
            self.nodes
                .lock()
                .map_err(|_| "worktree lock poisoned")?
                .get_mut(id)
                .ok_or("node disappeared")?
                .initial = initial;
        }
        require_clean_original(&self.original, timeout).await?;
        self.verify_identity(&self.original).await?;
        Ok(())
    }
    pub(crate) fn state_path(&self) -> &Path {
        &self.state
    }
    pub(crate) async fn verify_original(&self, expected: &WorkspaceManifest) -> Result<()> {
        if expected.root != self.original {
            return Err("canonical manifest root mismatch".into());
        }
        self.verify_identity(&self.original).await?;
        let status = git(
            &self.original,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            self.timeout,
        )
        .await?;
        for entry in status.split('\0').filter(|v| !v.is_empty()) {
            let raw = entry.get(3..).ok_or("invalid Git status entry")?;
            let path = Path::new(raw);
            if !expected.files.contains_key(path) && !self.initial_source.contains_key(path) {
                return Err("unexpected private/index Git changes".into());
            }
        }
        let original = self.original.clone();
        let expected = expected.files.clone();
        let private = self.private.clone();
        let lease = self._lock.try_clone()?;
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let current = DagSettlementVerifier::capture(&original)?;
            if current.files != expected {
                return Err("unexpected canonical workspace changes".into());
            }
            let mut all = current.files;
            for path in private.keys() {
                all.insert(path.clone(), file_digest(&original.join(path))?);
            }
            if private_files(&all, &expected) != private {
                return Err("protected canonical metadata drift".into());
            }
            Ok(())
        })
        .await?
    }
    pub(crate) async fn prepare_node(
        &self,
        id: &str,
        expected: &WorkspaceManifest,
    ) -> Result<PathBuf> {
        self.verify_original(expected).await?;
        let (path, initial) = {
            let nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let n = nodes.get(id).ok_or("unknown worktree node")?;
            (n.path.clone(), n.initial.clone())
        };
        self.verify_identity(&path).await?;
        if inventory(&path)? != initial {
            return Err("unprepared worktree has unexpected changes".into());
        }
        for relative in self
            .initial_source
            .keys()
            .filter(|p| !expected.files.contains_key(*p))
        {
            fs::remove_file(path.join(relative))?;
        }
        let total = expected.files.keys().try_fold(0u64, |total, path| {
            let size = fs::symlink_metadata(self.original.join(path))?.len();
            total
                .checked_add(size)
                .ok_or_else(|| std::io::Error::other("source copy size overflow"))
        })?;
        if total > MAX_BYTES as u64 || expected.files.len() > MAX_FILES {
            return Err("source copy aggregate budget exceeded".into());
        }
        for (relative, digest) in &expected.files {
            if reserved(relative) || !safe_relative(relative) {
                return Err("reserved/unsafe source path".into());
            }
            if identity(&path)?
                != self
                    .nodes
                    .lock()
                    .map_err(|_| "worktree lock poisoned")?
                    .get(id)
                    .and_then(|n| n.identity)
                    .ok_or("node identity missing")?
            {
                return Err("node root replaced during copy".into());
            }
            let source = self.original.join(relative);
            if fs::symlink_metadata(&source)?.len() > MAX_BYTES as u64 {
                return Err("source copy budget exceeded".into());
            }
            let bytes = fs::read(&source)?;
            if format!("sha256:{:x}", Sha256::digest(&bytes)) != *digest {
                return Err("canonical source changed during copy".into());
            }
            let dest = path.join(relative);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&dest)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::set_permissions(&dest, fs::metadata(source)?.permissions())?;
        }
        self.verify_original(expected).await?;
        if DagSettlementVerifier::capture(&path)?.files != expected.files {
            return Err("node baseline copy failed".into());
        }
        Ok(path)
    }
    pub(crate) async fn mark_verified(
        &self,
        id: &str,
        final_manifest: &WorkspaceManifest,
    ) -> Result<()> {
        let path = {
            self.nodes
                .lock()
                .map_err(|_| "worktree lock poisoned")?
                .get(id)
                .ok_or("unknown worktree node")?
                .path
                .clone()
        };
        self.verify_identity(&path).await?;
        if final_manifest.root != path {
            return Err("node verification manifest root mismatch".into());
        }
        let files = final_manifest.files.clone();
        let private = self.private.clone();
        let lease = self._lock.try_clone()?;
        let (full, hash) = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            if DagSettlementVerifier::capture(&path)?.files != files {
                return Err("node verification manifest drift".into());
            }
            let full = inventory(&path)?;
            if private_files(&full, &files) != private {
                return Err("private worktree state changed".into());
            }
            let hash = ownership::inventory_digest(&full)?;
            Ok::<_, crate::Error>((full, hash))
        })
        .await??;
        let transition = self.frontier.clone().lock_owned().await;
        {
            let mut nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let node = nodes.get_mut(id).ok_or("node disappeared")?;
            node.verified = Some(full);
            node.verified_digest = Some(hash);
        }
        self.export_frontier(transition).await?;
        Ok(())
    }
    pub(crate) fn report(&self) -> Result<Value> {
        self.frontier_report()
    }
    fn frontier_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec_pretty(&self.report()?)?;
        if bytes.len() > 64 * 1024 {
            return Err("worktree frontier exceeds byte budget".into());
        }
        Ok(bytes)
    }
    fn export(&self) -> Result<()> {
        let _export = self
            .export_lock
            .lock()
            .map_err(|_| "worktree export lock poisoned")?;
        ownership::write_frontier(&self.state, self.state_identity, &self.frontier_bytes()?)
            .map(|_| ())
    }
    async fn export_frontier(
        &self,
        transition: ownership::FrontierLease,
    ) -> Result<(ownership::FrontierLease, String)> {
        let bytes = self.frontier_bytes()?;
        let state = self.state.clone();
        let expected = self.state_identity;
        let writer = self.export_lock.clone();
        let lease = self._lock.try_clone()?;
        // Both leases outlive cancellation of this await: no late write may race
        // a new campaign or a newer in-process phase transition.
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let _writer = writer.lock().map_err(|_| "worktree export lock poisoned")?;
            let digest = ownership::write_frontier(&state, expected, &bytes)?;
            Ok((transition, digest))
        })
        .await?
    }
    pub(crate) async fn cleanup_verified(&self) -> Result<Value> {
        let transition = self.frontier.clone().lock_owned().await;
        let original = self.original.clone();
        let current =
            tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&original))
                .await??;
        self.verify_final_locked(&current, &transition).await?;
        self.cleanup_nodes(transition).await
    }

    /// Exercise the filesystem primitive independently from request/Store proof.
    /// Production cleanup always enters through `cleanup_verified` above.
    #[cfg(test)]
    pub(crate) async fn cleanup_fixture_nodes(&self) -> Result<Value> {
        let transition = self.frontier.clone().lock_owned().await;
        self.cleanup_nodes(transition).await
    }

    async fn cleanup_nodes(&self, transition: ownership::FrontierLease) -> Result<Value> {
        self.verify_identity(&self.original).await?;
        let nodes = {
            self.nodes
                .lock()
                .map_err(|_| "worktree lock poisoned")?
                .iter()
                .map(|(id, n)| (id.clone(), n.path.clone(), n.verified.clone(), n.removed))
                .collect::<Vec<_>>()
        };
        // All-or-retain preflight before removing any node.
        for (_, path, verified, removed) in &nodes {
            if *removed {
                continue;
            }
            self.verify_identity(path).await?;
            if verified.as_ref() != Some(&inventory(path)?) {
                return Err("worktree retained: missing verification or unexpected files".into());
            }
        }
        self.mark_cleanup_started()?;
        let (mut transition, _) = self.export_frontier(transition).await?;
        for (id, path, _, removed) in nodes {
            if removed {
                continue;
            }
            let raw = path.to_str().ok_or("nonUTF8 worktree")?;
            git(
                &self.original,
                &["worktree", "remove", "--force", raw],
                self.timeout,
            )
            .await?;
            #[cfg(test)]
            self.test_boundary(&id, TestBoundary::CleanupRemovedBeforeAck);
            self.nodes
                .lock()
                .map_err(|_| "worktree lock poisoned")?
                .get_mut(&id)
                .ok_or("node disappeared")?
                .removed = true;
            let (next, _) = self.export_frontier(transition).await?;
            transition = next;
        }
        drop(transition);
        self.report()
    }
    async fn verify_identity(&self, path: &Path) -> Result<()> {
        self.verify_lock_identity()?;
        if identity(&self.common)? != self.common_identity
            || identity(&self.state)? != self.state_identity
        {
            return Err("worktree owner directory replaced".into());
        }
        let (expected_identity, expected_git_dir, expected_git_identity) = if path == self.original
        {
            (
                self.original_identity,
                self.original_git_dir.clone(),
                self.original_git_dir_identity,
            )
        } else {
            let nodes = self.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let n = nodes
                .values()
                .find(|n| n.path == path)
                .ok_or("unowned worktree path")?;
            (
                n.identity.ok_or("worktree identity missing")?,
                n.git_dir.clone().ok_or("registration missing")?,
                n.git_dir_identity.ok_or("registration identity missing")?,
            )
        };
        if identity(path)? != expected_identity {
            return Err("worktree root replaced".into());
        }
        if path != self.original {
            let marker = fs::symlink_metadata(path.join(".git"))?;
            if !marker.is_file() || marker.file_type().is_symlink() {
                return Err("invalid node Git registration marker".into());
            }
        }
        let git_dir = PathBuf::from(
            git(path, &["rev-parse", "--absolute-git-dir"], self.timeout)
                .await?
                .trim(),
        )
        .canonicalize()?;
        if git_dir != expected_git_dir || identity(&git_dir)? != expected_git_identity {
            return Err("worktree registration replaced".into());
        }
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err("worktree root symlink".into());
        }
        let top = PathBuf::from(
            git(path, &["rev-parse", "--show-toplevel"], self.timeout)
                .await?
                .trim(),
        )
        .canonicalize()?;
        if top != path {
            return Err("worktree root identity changed".into());
        }
        if git(path, &["rev-parse", "HEAD"], self.timeout)
            .await?
            .trim()
            != self.head
        {
            return Err("worktree HEAD/index changed".into());
        }
        // Read-only index comparison: write-tree takes index.lock and races
        // concurrent reader nodes even when the index has not changed.
        git(
            path,
            &["diff", "--cached", "--quiet", &self.head],
            self.timeout,
        )
        .await?;
        let common = git(path, &["rev-parse", "--git-common-dir"], self.timeout).await?;
        if path.join(common.trim()).canonicalize()? != self.common {
            return Err("worktree registration changed".into());
        }
        Ok(())
    }
}
pub(crate) fn validate_source_budget(root: &Path) -> Result<()> {
    let mut total = 0_u64;
    let mut count = 0_usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !matches!(
                    entry.file_name().to_str(),
                    Some(
                        ".git"
                            | ".agents"
                            | ".codex"
                            | ".omx"
                            | ".hcodex"
                            | ".codegraph"
                            | ".venv"
                            | "target"
                            | "node_modules"
                    )
                )
        })
    {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err("worktree source must be regular files".into());
        }
        total = total
            .checked_add(entry.metadata()?.len())
            .ok_or("worktree source size overflow")?;
        count += 1;
        if total > MAX_BYTES as u64 || count > MAX_FILES {
            return Err("worktree source budget exceeded".into());
        }
    }
    Ok(())
}
fn file_digest(path: &Path) -> Result<String> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > MAX_BYTES as u64 {
        return Err("invalid tracked input".into());
    }
    Ok(format!("sha256:{:x}", Sha256::digest(fs::read(path)?)))
}
fn identity(path: &Path) -> Result<(u64, u64)> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err("owner directory invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        Err("worktree identity unsupported on this platform".into())
    }
}
fn safe_relative(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|p| matches!(p, std::path::Component::Normal(_)))
}
fn reserved(path: &Path) -> bool {
    path.components().any(|p| {
        matches!(
            p.as_os_str().to_str(),
            Some(".git" | ".agents" | ".codex" | ".omx" | ".hcodex" | ".codegraph")
        )
    })
}
fn private_files(all: &Inventory, source: &Inventory) -> Inventory {
    all.iter()
        .filter(|(p, _)| !source.contains_key(*p))
        .map(|(p, h)| (p.clone(), h.clone()))
        .collect()
}
fn inventory(root: &Path) -> Result<Inventory> {
    let mut out = BTreeMap::new();
    let mut total = 0usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !matches!(
                    e.file_name().to_str(),
                    Some(".git" | "target" | "node_modules" | ".venv")
                )
        })
    {
        let e = entry?;
        if e.file_type().is_symlink() {
            return Err("source symlink unsupported".into());
        }
        if e.file_type().is_dir() {
            continue;
        }
        if !e.file_type().is_file() {
            return Err("nonregular workspace input".into());
        }
        let metadata = e.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err("hardlinked source unsupported".into());
            }
        }
        total = total
            .checked_add(usize::try_from(metadata.len())?)
            .ok_or("worktree source size overflow")?;
        if total > MAX_BYTES || out.len() >= MAX_FILES {
            return Err("worktree source budget exceeded".into());
        }
        let bytes = fs::read(e.path())?;
        out.insert(
            e.path().strip_prefix(root)?.to_path_buf(),
            format!("sha256:{:x}", Sha256::digest(&bytes)),
        );
    }
    Ok(out)
}
fn real_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err("invalid worktree state directory".into());
    }
    Ok(())
}
pub(crate) async fn require_clean_original(root: &Path, timeout: Duration) -> Result<()> {
    if !git(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        timeout,
    )
    .await?
    .is_empty()
    {
        return Err(
            "worktree mode requires ORIGINAL checkout clean including untracked files".into(),
        );
    }
    Ok(())
}
#[cfg(test)]
async fn clean(root: &Path, timeout: Duration) -> Result<()> {
    require_clean_original(root, timeout).await
}
#[cfg(test)]
mod tests;
