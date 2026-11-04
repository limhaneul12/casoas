//! Offline, single-owner recovery artifacts. Metadata authority stays in Store.
//!
//! Static link/binding checks are not descriptor-relative confinement against
//! hostile concurrent directory swaps. Sync calls are not hardware power-loss proof.
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 1024 * 1024;

pub(crate) struct ChangeArchive {
    root: PathBuf,
    identity: Metadata,
}

impl ChangeArchive {
    pub(crate) fn open(workspace: &Path) -> crate::Result<Self> {
        require_directory(workspace)?;
        let root = workspace.canonicalize()?;
        let identity = fs::symlink_metadata(&root)?;
        Ok(Self { root, identity })
    }

    pub(crate) fn prepare(&self, id: &str, before: &[u8], after: &[u8]) -> crate::Result<()> {
        validate_id(id)?;
        if before.len() > MAX_BYTES || after.len() > MAX_BYTES {
            return Err("recovery artifact exceeds byte limit".into());
        }
        self.validate_root()?;
        let state = self.root.join(".hcodex");
        ensure_directory(&state)?;
        sync_directory(&self.root)?;
        let changes = state.join("changes");
        ensure_directory(&changes)?;
        sync_directory(&state)?;
        let change = changes.join(id);
        match create_directory(&change) {
            Ok(()) => {
                sync_directory(&changes)?;
                self.validate_chain(id)?;
                write_new(&change.join("before"), before)?;
                write_new(&change.join("after"), after)?;
                sync_directory(&change)?;
                self.validate_chain(id)?;
                if self.read(id)? != (before.to_vec(), after.to_vec()) {
                    return Err("recovery artifact publication changed".into());
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A partial existing publication is not silently repaired.
                if self.read(id)? != (before.to_vec(), after.to_vec()) {
                    return Err("recovery artifact identity conflict".into());
                }
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn read(&self, id: &str) -> crate::Result<(Vec<u8>, Vec<u8>)> {
        validate_id(id)?;
        let change = self.validate_chain(id)?;
        let before = read_bounded(&change.join("before"))?;
        let after = read_bounded(&change.join("after"))?;
        self.validate_chain(id)?;
        Ok((before, after))
    }

    fn validate_root(&self) -> crate::Result<()> {
        require_directory(&self.root)?;
        let current = fs::symlink_metadata(&self.root)?;
        if !same_identity(&self.identity, &current) {
            return Err("recovery workspace identity changed".into());
        }
        Ok(())
    }

    fn validate_chain(&self, id: &str) -> crate::Result<PathBuf> {
        self.validate_root()?;
        let mut path = self.root.clone();
        for component in [".hcodex", "changes", id] {
            path.push(component);
            require_directory(&path)?;
        }
        Ok(path)
    }
}

fn validate_id(id: &str) -> crate::Result<()> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("invalid recovery change ID".into());
    }
    Ok(())
}

fn require_directory(path: &Path) -> crate::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err("recovery path is not a real directory".into());
    }
    Ok(())
}

fn create_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn ensure_directory(path: &Path) -> crate::Result<()> {
    match create_directory(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => require_directory(path),
        Err(error) => Err(error.into()),
    }
}

fn require_file(metadata: &Metadata) -> crate::Result<()> {
    if !metadata.file_type().is_file() {
        return Err("artifact is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("hardlinked artifact rejected".into());
        }
    }
    Ok(())
}

fn same_identity(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.is_dir() == b.is_dir() && a.created().ok() == b.created().ok()
    }
}

fn read_bounded(path: &Path) -> crate::Result<Vec<u8>> {
    let before = fs::symlink_metadata(path)?;
    require_file(&before)?;
    let file = File::open(path)?;
    let opened = file.metadata()?;
    require_file(&opened)?;
    if !same_identity(&before, &opened) {
        return Err("artifact identity changed before read".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err("artifact exceeds read limit".into());
    }
    let current = fs::symlink_metadata(path)?;
    require_file(&current)?;
    if !same_identity(&before, &current) {
        return Err("artifact identity changed during read".into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> crate::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> crate::Result<()> {
    File::open(path)?.sync_all().map_err(Into::into)
}

#[cfg(test)]
mod tests;
