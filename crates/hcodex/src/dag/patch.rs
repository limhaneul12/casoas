use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Result;

use super::WorkspaceManifest;

const ABSENT_PREIMAGE: &str = "sha256:absent";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PatchArtifact {
    pub edits: Vec<PatchEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PatchEdit {
    pub path: PathBuf,
    pub preimage_sha256: String,
    pub new_content: String,
}

struct ValidatedEdit {
    path: PathBuf,
    absolute: PathBuf,
    new_content: Vec<u8>,
    original: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
}

pub(crate) struct AppliedScopedPatch {
    pub changed_paths: Vec<PathBuf>,
    root: PathBuf,
    edits: Vec<ValidatedEdit>,
}

impl AppliedScopedPatch {
    pub(crate) fn rollback(self) -> Result<()> {
        let committed: Vec<usize> = (0..self.edits.len()).collect();
        rollback_committed(&self.root, &self.edits, &committed)
    }
}

pub fn apply_scoped_patch(
    root: &Path,
    declared_write_files: &[PathBuf],
    pre_manifest: &WorkspaceManifest,
    artifact: &PatchArtifact,
) -> Result<Vec<PathBuf>> {
    Ok(
        apply_scoped_patch_with_rollback(root, declared_write_files, pre_manifest, artifact)?
            .changed_paths,
    )
}

pub(crate) fn apply_scoped_patch_with_rollback(
    root: &Path,
    declared_write_files: &[PathBuf],
    pre_manifest: &WorkspaceManifest,
    artifact: &PatchArtifact,
) -> Result<AppliedScopedPatch> {
    apply_scoped_patch_inner(
        root,
        declared_write_files,
        pre_manifest,
        artifact,
        #[cfg(test)]
        None,
    )
}

#[cfg(test)]
pub(crate) fn apply_scoped_patch_observed(
    root: &Path,
    declared_write_files: &[PathBuf],
    pre_manifest: &WorkspaceManifest,
    artifact: &PatchArtifact,
    observer: &dyn Fn(&'static str),
) -> Result<AppliedScopedPatch> {
    apply_scoped_patch_inner(
        root,
        declared_write_files,
        pre_manifest,
        artifact,
        Some(observer),
    )
}

fn apply_scoped_patch_inner(
    root: &Path,
    declared_write_files: &[PathBuf],
    pre_manifest: &WorkspaceManifest,
    artifact: &PatchArtifact,
    #[cfg(test)] observer: Option<&dyn Fn(&'static str)>,
) -> Result<AppliedScopedPatch> {
    if artifact.edits.is_empty() {
        return Err("patch artifact contains no edits".into());
    }
    let root = root.canonicalize()?;
    if root != pre_manifest.root {
        return Err("patch workspace does not match preimage manifest".into());
    }
    let declared: BTreeSet<PathBuf> = declared_write_files
        .iter()
        .map(|path| normalize_relative(path))
        .collect::<Result<_>>()?;
    let mut seen = BTreeSet::new();
    let mut validated = Vec::new();
    for edit in &artifact.edits {
        let relative = normalize_relative(&edit.path)?;
        if !declared.contains(&relative) {
            return Err(format!(
                "patch path is outside declared write scope: {}",
                relative.display()
            )
            .into());
        }
        if !seen.insert(relative.clone()) {
            return Err(format!("patch contains duplicate path: {}", relative.display()).into());
        }
        let absolute = validate_destination(&root, &relative)?;
        let expected = pre_manifest
            .files
            .get(&relative)
            .map(String::as_str)
            .unwrap_or(ABSENT_PREIMAGE);
        if edit.preimage_sha256 != expected {
            return Err(format!("stale patch preimage for {}", relative.display()).into());
        }
        let metadata = fs::symlink_metadata(&absolute).ok();
        if metadata
            .as_ref()
            .is_some_and(|value| value.file_type().is_symlink())
        {
            return Err(format!("patch destination is a symlink: {}", relative.display()).into());
        }
        let original = if metadata.as_ref().is_some_and(fs::Metadata::is_file) {
            let bytes = fs::read(&absolute)?;
            let actual = format!("sha256:{:x}", Sha256::digest(&bytes));
            if actual != expected {
                return Err(format!(
                    "patch preimage changed before apply: {}",
                    relative.display()
                )
                .into());
            }
            Some(bytes)
        } else if expected == ABSENT_PREIMAGE {
            None
        } else {
            return Err(format!("patch destination disappeared: {}", relative.display()).into());
        };
        validated.push(ValidatedEdit {
            path: relative,
            absolute,
            new_content: edit.new_content.as_bytes().to_vec(),
            original,
            permissions: metadata.map(|value| value.permissions()),
        });
    }
    apply_validated(
        &root,
        validated,
        #[cfg(test)]
        observer,
    )
}

fn apply_validated(
    root: &Path,
    edits: Vec<ValidatedEdit>,
    #[cfg(test)] observer: Option<&dyn Fn(&'static str)>,
) -> Result<AppliedScopedPatch> {
    let mut staged = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let temporary = temporary_path(root, &edit.path, index, "apply")?;
        if let Err(error) = write_new_file(&temporary, &edit.new_content, edit.permissions.clone())
        {
            cleanup_paths(&staged);
            return Err(error);
        }
        staged.push(temporary);
    }
    #[cfg(test)]
    if let Some(observer) = observer {
        observer("staged");
    }
    let mut committed = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        if let Err(error) = fs::rename(&staged[index], &edit.absolute) {
            let rollback = rollback_committed(root, &edits, &committed);
            cleanup_paths(&staged[index..]);
            return match rollback {
                Ok(()) => Err(error.into()),
                Err(rollback_error) => Err(format!(
                    "patch commit failed ({error}); rollback also failed ({rollback_error})"
                )
                .into()),
            };
        }
        committed.push(index);
        #[cfg(test)]
        if committed.len() < edits.len()
            && let Some(observer) = observer
        {
            observer("partial_publication");
        }
    }
    let changed_paths = edits.iter().map(|edit| edit.path.clone()).collect();
    Ok(AppliedScopedPatch {
        changed_paths,
        root: root.to_path_buf(),
        edits,
    })
}

fn rollback_committed(root: &Path, edits: &[ValidatedEdit], committed: &[usize]) -> Result<()> {
    for index in committed.iter().rev().copied() {
        let edit = &edits[index];
        if let Some(original) = &edit.original {
            let rollback = temporary_path(root, &edit.path, index, "rollback")?;
            write_new_file(&rollback, original, edit.permissions.clone())?;
            fs::rename(rollback, &edit.absolute)?;
        } else if edit.absolute.exists() {
            fs::remove_file(&edit.absolute)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_destination(root: &Path, relative: &Path) -> Result<PathBuf> {
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let absolute_parent = root.join(parent).canonicalize()?;
    if !absolute_parent.starts_with(root) {
        return Err(format!("patch path escapes workspace: {}", relative.display()).into());
    }
    let mut cursor = root.to_path_buf();
    for component in parent.components() {
        cursor.push(component.as_os_str());
        if fs::symlink_metadata(&cursor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(format!("patch path traverses symlink: {}", relative.display()).into());
        }
    }
    Ok(root.join(relative))
}

pub(crate) fn normalize_relative(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || text.contains('\0')
        || text.contains('\\')
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            ) || component.as_os_str() == ".hcodex"
        })
    {
        return Err(format!("unsafe patch path: {}", path.display()).into());
    }
    Ok(path.components().collect())
}

fn temporary_path(root: &Path, relative: &Path, index: usize, purpose: &str) -> Result<PathBuf> {
    let file_name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("patch file name is not valid UTF-8")?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    Ok(root.join(parent).join(format!(
        ".{file_name}.hcodex-{purpose}-{}-{index}",
        std::process::id()
    )))
}

fn write_new_file(path: &Path, content: &[u8], permissions: Option<fs::Permissions>) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    if let Some(permissions) = permissions {
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn cleanup_paths(paths: &[PathBuf]) {
    for path in paths {
        let _ignored_cleanup_error = fs::remove_file(path);
    }
}

pub(crate) fn preimage_contract(
    manifest: &WorkspaceManifest,
    declared_write_files: &[PathBuf],
) -> Result<BTreeMap<String, String>> {
    declared_write_files
        .iter()
        .map(|path| {
            let relative = normalize_relative(path)?;
            let digest = manifest
                .files
                .get(&relative)
                .cloned()
                .unwrap_or_else(|| ABSENT_PREIMAGE.to_owned());
            Ok((relative.to_string_lossy().into_owned(), digest))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::dag::DagSettlementVerifier;

    #[test]
    fn applies_exact_preimage_scoped_artifact() -> Result<()> {
        let workspace = tempdir()?;
        fs::write(workspace.path().join("a.rs"), "old")?;
        let manifest = DagSettlementVerifier::capture(workspace.path())?;
        let artifact = PatchArtifact {
            edits: vec![PatchEdit {
                path: PathBuf::from("a.rs"),
                preimage_sha256: manifest.files[Path::new("a.rs")].clone(),
                new_content: "new".to_owned(),
            }],
        };
        let changed = apply_scoped_patch(
            workspace.path(),
            &[PathBuf::from("a.rs")],
            &manifest,
            &artifact,
        )?;
        assert_eq!(changed, [PathBuf::from("a.rs")]);
        assert_eq!(fs::read_to_string(workspace.path().join("a.rs"))?, "new");
        Ok(())
    }

    #[test]
    fn rejects_noop_stale_out_of_scope_and_traversal_before_writing() -> Result<()> {
        let workspace = tempdir()?;
        fs::write(workspace.path().join("a.rs"), "old-a")?;
        fs::write(workspace.path().join("b.rs"), "old-b")?;
        let manifest = DagSettlementVerifier::capture(workspace.path())?;
        assert!(
            apply_scoped_patch(
                workspace.path(),
                &[PathBuf::from("a.rs")],
                &manifest,
                &PatchArtifact { edits: Vec::new() },
            )
            .is_err()
        );
        let two_edits = PatchArtifact {
            edits: vec![
                PatchEdit {
                    path: PathBuf::from("a.rs"),
                    preimage_sha256: manifest.files[Path::new("a.rs")].clone(),
                    new_content: "new-a".to_owned(),
                },
                PatchEdit {
                    path: PathBuf::from("b.rs"),
                    preimage_sha256: "sha256:stale".to_owned(),
                    new_content: "new-b".to_owned(),
                },
            ],
        };
        assert!(
            apply_scoped_patch(
                workspace.path(),
                &[PathBuf::from("a.rs"), PathBuf::from("b.rs")],
                &manifest,
                &two_edits,
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(workspace.path().join("a.rs"))?, "old-a");
        for unsafe_path in [PathBuf::from("b.rs"), PathBuf::from("../escape.rs")] {
            let artifact = PatchArtifact {
                edits: vec![PatchEdit {
                    path: unsafe_path,
                    preimage_sha256: manifest.files[Path::new("b.rs")].clone(),
                    new_content: "bad".to_owned(),
                }],
            };
            assert!(
                apply_scoped_patch(
                    workspace.path(),
                    &[PathBuf::from("a.rs")],
                    &manifest,
                    &artifact
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_file_changed_after_manifest_capture() -> Result<()> {
        let workspace = tempdir()?;
        fs::write(workspace.path().join("a.rs"), "captured")?;
        let manifest = DagSettlementVerifier::capture(workspace.path())?;
        let captured_digest = manifest.files[Path::new("a.rs")].clone();
        fs::write(workspace.path().join("a.rs"), "concurrent-change")?;
        let artifact = PatchArtifact {
            edits: vec![PatchEdit {
                path: PathBuf::from("a.rs"),
                preimage_sha256: captured_digest,
                new_content: "would-clobber".to_owned(),
            }],
        };
        assert!(
            apply_scoped_patch(
                workspace.path(),
                &[PathBuf::from("a.rs")],
                &manifest,
                &artifact,
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("a.rs"))?,
            "concurrent-change"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_destination() -> Result<()> {
        use std::os::unix::fs::symlink;

        let workspace = tempdir()?;
        let outside = tempdir()?;
        fs::write(outside.path().join("secret"), "secret")?;
        symlink(
            outside.path().join("secret"),
            workspace.path().join("link.rs"),
        )?;
        let manifest = DagSettlementVerifier::capture(workspace.path())?;
        let artifact = PatchArtifact {
            edits: vec![PatchEdit {
                path: PathBuf::from("link.rs"),
                preimage_sha256: manifest.files[Path::new("link.rs")].clone(),
                new_content: "bad".to_owned(),
            }],
        };
        assert!(
            apply_scoped_patch(
                workspace.path(),
                &[PathBuf::from("link.rs")],
                &manifest,
                &artifact
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(outside.path().join("secret"))?, "secret");
        Ok(())
    }

    #[test]
    fn applied_scoped_patch_rolls_back_modified_and_created_files() -> Result<()> {
        let workspace = tempdir()?;
        fs::write(workspace.path().join("existing.rs"), "original-bytes")?;
        let manifest = DagSettlementVerifier::capture(workspace.path())?;
        let artifact = PatchArtifact {
            edits: vec![
                PatchEdit {
                    path: PathBuf::from("existing.rs"),
                    preimage_sha256: manifest.files[Path::new("existing.rs")].clone(),
                    new_content: "broken-bytes".to_owned(),
                },
                PatchEdit {
                    path: PathBuf::from("created.rs"),
                    preimage_sha256: "sha256:absent".to_owned(),
                    new_content: "new-file".to_owned(),
                },
            ],
        };
        let applied = apply_scoped_patch_with_rollback(
            workspace.path(),
            &[PathBuf::from("existing.rs"), PathBuf::from("created.rs")],
            &manifest,
            &artifact,
        )?;
        assert_eq!(
            fs::read_to_string(workspace.path().join("existing.rs"))?,
            "broken-bytes"
        );
        assert!(workspace.path().join("created.rs").is_file());
        applied.rollback()?;
        assert_eq!(
            fs::read_to_string(workspace.path().join("existing.rs"))?,
            "original-bytes"
        );
        assert!(!workspace.path().join("created.rs").exists());
        Ok(())
    }
}
