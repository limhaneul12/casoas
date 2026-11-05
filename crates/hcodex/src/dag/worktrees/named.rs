//! Persistent named session workspace; node/campaign worktrees remain disposable.
use super::*;
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Debug, Serialize)]
pub struct NamedWorktree {
    pub name: String,
    pub original: PathBuf,
    pub path: PathBuf,
    pub base_head: String,
    pub current_head: String,
    pub created: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedRecord {
    version: u32,
    name: String,
    original: PathBuf,
    path: PathBuf,
    head: String,
    common: PathBuf,
    git_dir: PathBuf,
    original_identity: (u64, u64),
    common_identity: (u64, u64),
    node_identity: (u64, u64),
}

/// Create from a clean original, or reopen the same owned workspace without
/// resetting its user changes. Named sessions are never automatically removed.
pub async fn open_named_worktree(
    original: &Path,
    state_root: &Path,
    name: &str,
) -> Result<NamedWorktree> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("worktree name must contain 1..64 ASCII letters, digits, '-' or '_'".into());
    }
    let original = original.canonicalize()?;
    let timeout = Duration::from_secs(30);
    require_clean_original(&original, timeout).await?;
    let top = PathBuf::from(
        git(&original, &["rev-parse", "--show-toplevel"], timeout)
            .await?
            .trim(),
    )
    .canonicalize()?;
    if top != original {
        return Err("named worktree requires original Git top-level workspace".into());
    }
    let state_root = resolve_state_root(state_root)?;
    if state_root.starts_with(&original) {
        return Err("named worktree state must live outside original workspace".into());
    }
    fs::create_dir_all(&state_root)?;
    let parent = state_root.join("named-worktrees");
    real_directory(&parent)?;
    let namespace = parent.join(format!(
        "{:x}",
        Sha256::digest(original.as_os_str().as_encoded_bytes())
    ));
    real_directory(&namespace)?;
    let path = namespace.join(name);
    let record_path = namespace.join(format!("{name}.json"));
    let metadata = match fs::symlink_metadata(&record_path) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(m) = metadata {
        if !m.is_file() || m.file_type().is_symlink() {
            return Err("invalid named worktree record".into());
        }
        let mut bytes = Vec::new();
        File::open(&record_path)?
            .take(16385)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16384 {
            return Err("named worktree record exceeds budget".into());
        }
        let r: NamedRecord = serde_json::from_slice(&bytes)?;
        if r.version != 1
            || r.name != name
            || r.original != original
            || r.path != path
            || identity(&original)? != r.original_identity
            || identity(&r.common)? != r.common_identity
            || identity(&path)? != r.node_identity
        {
            return Err("named worktree ownership identity changed".into());
        }
        let marker = fs::symlink_metadata(path.join(".git"))?;
        if !marker.is_file() || marker.file_type().is_symlink() {
            return Err("invalid named worktree Git marker".into());
        }
        let common = PathBuf::from(
            git(&original, &["rev-parse", "--git-common-dir"], timeout)
                .await?
                .trim(),
        );
        let node_common = PathBuf::from(
            git(&path, &["rev-parse", "--git-common-dir"], timeout)
                .await?
                .trim(),
        );
        let git_dir = PathBuf::from(
            git(&path, &["rev-parse", "--absolute-git-dir"], timeout)
                .await?
                .trim(),
        )
        .canonicalize()?;
        if original.join(common).canonicalize()? != r.common
            || path.join(node_common).canonicalize()? != r.common
            || git_dir != r.git_dir
        {
            return Err("named worktree Git registration changed".into());
        }
        let current_head = git(&path, &["rev-parse", "HEAD"], timeout)
            .await?
            .trim()
            .to_owned();
        return Ok(NamedWorktree {
            name: r.name,
            original: r.original,
            path: r.path,
            base_head: r.head,
            current_head,
            created: false,
        });
    }
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            return Err(format!("unregistered named worktree exists: {}", path.display()).into());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let id = format!("named-{}", Uuid::new_v4());
    let session =
        WorktreeCampaign::start(&original, &state_root, &[name.to_owned()], timeout, &id).await?;
    let setup = async {
        let (old, node_identity, git_dir) = {
            let nodes = session.nodes.lock().map_err(|_| "worktree lock poisoned")?;
            let n = nodes.get(name).ok_or("named node missing")?;
            (
                n.path.clone(),
                n.identity.ok_or("named identity missing")?,
                n.git_dir.clone().ok_or("named registration missing")?,
            )
        };
        let old_text = old.to_str().ok_or("nonUTF8 worktree path")?;
        let new_text = path.to_str().ok_or("nonUTF8 named worktree path")?;
        git(
            &original,
            &["worktree", "move", old_text, new_text],
            timeout,
        )
        .await?;
        session
            .nodes
            .lock()
            .map_err(|_| "worktree lock poisoned")?
            .get_mut(name)
            .ok_or("named node missing")?
            .path = path.clone();
        session.verify_identity(&path).await?;
        session.export()?;
        let record = NamedRecord {
            version: 1,
            name: name.into(),
            original: original.clone(),
            path: path.clone(),
            head: session.head.clone(),
            common: session.common.clone(),
            git_dir,
            original_identity: session.original_identity,
            common_identity: session.common_identity,
            node_identity,
        };
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&record_path)?;
        file.write_all(&serde_json::to_vec(&record)?)?;
        file.sync_all()?;
        File::open(&namespace)?.sync_all()?;
        Ok::<_, crate::Error>(NamedWorktree {
            name: name.into(),
            original,
            path,
            base_head: session.head.clone(),
            current_head: session.head.clone(),
            created: true,
        })
    }
    .await;
    setup.map_err(|error| {
        format!(
            "named worktree setup incomplete; inspect {}: {error}",
            session.state.display()
        )
        .into()
    })
}

fn resolve_state_root(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute()
        || root.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("named worktree state root must be an absolute normalized path".into());
    }
    let mut existing = root;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or("state root has no existing ancestor")?;
    }
    Ok(existing.canonicalize()?.join(root.strip_prefix(existing)?))
}

#[cfg(test)]
mod tests;
