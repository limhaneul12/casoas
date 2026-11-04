//! One-file recovery observation; never applies, restores or retries source work.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::recovery_artifacts::ChangeArchive;
use crate::storage::{RecoveryRecord, Store};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryObservation {
    BeforeImageObserved,
    AfterImageObservedUnverified,
    Conflict,
}

pub struct RecoveryJournal<'a> {
    store: &'a Store,
    workspace: PathBuf,
    archive: ChangeArchive,
}

impl<'a> RecoveryJournal<'a> {
    /// Open the existing workspace without creating private artifact directories.
    pub fn open(workspace: &Path, store: &'a Store) -> crate::Result<Self> {
        let workspace = workspace.canonicalize()?;
        let archive = ChangeArchive::open(&workspace)?;
        Ok(Self {
            store,
            workspace,
            archive,
        })
    }

    /// Persist original/intended bytes, not a source-write authorization.
    pub fn prepare(
        &self,
        change_id: &str,
        relative_path: &Path,
        before: &[u8],
        after: &[u8],
    ) -> crate::Result<RecoveryRecord> {
        if before.len() > 1024 * 1024 || after.len() > 1024 * 1024 {
            return Err("recovery payload exceeds limit".into());
        }
        let relative_path = crate::dag::normalize_relative(relative_path)?;
        let record = RecoveryRecord {
            change_id: change_id.to_owned(),
            workspace: self.workspace.clone(),
            relative_path,
            before_sha256: hash(before),
            after_sha256: hash(after),
            before_len: u64::try_from(before.len())?,
            after_len: u64::try_from(after.len())?,
        };
        record.validate()?;
        if self.store.get_recovery(change_id)?.is_none()
            && read_source(&self.workspace, &record.relative_path)? != before
        {
            return Err("recovery preimage differs from current source".into());
        }
        let registered = self.store.register_recovery(&record)?;
        self.archive.prepare(change_id, before, after)?;
        Ok(registered)
    }

    /// Read and compare exact bytes. No source or journal mutation occurs.
    pub fn inspect(&self, change_id: &str) -> crate::Result<RecoveryObservation> {
        let record = self
            .store
            .get_recovery(change_id)?
            .ok_or("recovery record missing")?;
        if record.workspace != self.workspace {
            return Err("recovery record belongs to another workspace".into());
        }
        let (before, after) = self.archive.read(change_id)?;
        if hash(&before) != record.before_sha256
            || hash(&after) != record.after_sha256
            || u64::try_from(before.len())? != record.before_len
            || u64::try_from(after.len())? != record.after_len
        {
            return Err("recovery artifact integrity mismatch".into());
        }
        let current = match read_source(&self.workspace, &record.relative_path) {
            Ok(bytes) => bytes,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(RecoveryObservation::Conflict);
            }
            Err(error) => return Err(error),
        };
        Ok(if current == before {
            RecoveryObservation::BeforeImageObserved
        } else if current == after {
            RecoveryObservation::AfterImageObservedUnverified
        } else {
            RecoveryObservation::Conflict
        })
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn read_source(workspace: &Path, relative: &Path) -> crate::Result<Vec<u8>> {
    let relative = crate::dag::normalize_relative(relative)?;
    let mut path = workspace.to_path_buf();
    for component in relative.components() {
        path.push(component);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err("recovery source path is a symlink".into());
        }
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("recovery source must be a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("recovery source is hardlinked".into());
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err("recovery source grew beyond limit".into());
    }
    Ok(bytes)
}
