use std::collections::BTreeSet;
use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::config;
use super::types::InstallChanges;

struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
}

const SKILL_PREFIX: &str = "skills/codex-heterarchy/";
const EMBEDDED_SKILLS: &[EmbeddedAsset] = &[
    EmbeddedAsset {
        path: "skills/codex-heterarchy/SKILL.md",
        bytes: include_bytes!("../../assets/heterarchy-codex/skills/codex-heterarchy/SKILL.md"),
    },
    EmbeddedAsset {
        path: "skills/codex-heterarchy/agents/openai.yaml",
        bytes: include_bytes!(
            "../../assets/heterarchy-codex/skills/codex-heterarchy/agents/openai.yaml"
        ),
    },
];
const EMBEDDED_PROVENANCE: &[u8] = include_bytes!("../../assets/PROVENANCE.json");

pub(crate) struct InstallPaths {
    pub gemini_skill: PathBuf,
    pub codex_skill: PathBuf,
}

impl InstallPaths {
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            gemini_skill: home.join(".gemini/config/skills/codex-heterarchy"),
            codex_skill: home.join(".codex/skills/codex-heterarchy"),
        }
    }
}

pub(crate) fn synchronize(repository: &Path, home: &Path) -> crate::Result<InstallChanges> {
    let paths = InstallPaths::new(home);
    let primary = repository.join("plugin/heterarchy-codex/skills/codex-heterarchy");
    verify_embedded_assets()?;
    if primary.is_dir() {
        verify_repository_assets(&primary)?;
    }
    let legacy_mcp = config::retire_managed_mcp(repository, home)?;
    for target in [&paths.gemini_skill, &paths.codex_skill] {
        materialize_skill(&primary, target)?;
    }
    let launcher_migrated = super::launcher::migrate(repository, home)?;
    Ok(InstallChanges {
        legacy_mcp,
        assets_synchronized: true,
        launcher_migrated,
    })
}

fn materialize_skill(repository_skill: &Path, target: &Path) -> crate::Result<()> {
    if target.is_symlink() {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "managed skill directory may not be a symlink",
        )
        .into());
    }
    fs::create_dir_all(target)?;
    for asset in EMBEDDED_SKILLS {
        let relative = asset
            .path
            .strip_prefix(SKILL_PREFIX)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid embedded skill path"))?;
        let destination = target.join(relative);
        if destination.is_symlink() {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                "managed skill file may not be a symlink",
            )
            .into());
        }
        if let Some(parent) = destination.parent() {
            if parent.is_symlink() {
                return Err(IoError::new(
                    ErrorKind::InvalidData,
                    "managed skill parent may not be a symlink",
                )
                .into());
            }
            fs::create_dir_all(parent)?;
        }
        if repository_skill.is_dir() {
            let _bytes = fs::copy(repository_skill.join(relative), destination)?;
        } else {
            fs::write(destination, asset.bytes)?;
        }
    }
    Ok(())
}

fn verify_repository_assets(source: &Path) -> crate::Result<()> {
    verify_embedded_assets()?;
    let expected = EMBEDDED_SKILLS
        .iter()
        .filter_map(|asset| asset.path.strip_prefix(SKILL_PREFIX))
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    let actual = collect_relative_files(source, source)?;
    if actual != expected {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "repository skill inventory differs from embedded provenance",
        )
        .into());
    }
    for asset in EMBEDDED_SKILLS {
        let relative = asset
            .path
            .strip_prefix(SKILL_PREFIX)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid embedded skill path"))?;
        if fs::read(source.join(relative))? != asset.bytes {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                format!("repository skill asset drift detected: {}", asset.path),
            )
            .into());
        }
    }
    Ok(())
}

fn verify_embedded_assets() -> crate::Result<()> {
    let provenance: Value = serde_json::from_slice(EMBEDDED_PROVENANCE)?;
    for asset in EMBEDDED_SKILLS {
        let expected = format!("sha256:{}", sha256(asset.bytes));
        if provenance["files"][asset.path].as_str() != Some(expected.as_str()) {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                format!("embedded skill asset digest mismatch: {}", asset.path),
            )
            .into());
        }
    }
    Ok(())
}

fn collect_relative_files(root: &Path, directory: &Path) -> crate::Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                "managed skill assets may not contain symlinks",
            )
            .into());
        }
        if kind.is_dir() {
            files.extend(collect_relative_files(root, &entry.path())?);
        } else if kind.is_file() {
            files.insert(
                entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(files)
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_asset_content_drift_remains_rejected() -> crate::Result<()> {
        let directory = tempfile::tempdir()?;
        materialize_skill(Path::new("/absent-repository-skill"), directory.path())?;
        verify_repository_assets(directory.path())?;
        fs::write(directory.path().join("SKILL.md"), "unverified replacement")?;
        assert!(verify_repository_assets(directory.path()).is_err());
        Ok(())
    }

    #[test]
    fn repository_assets_equal_embedded_provenance() -> crate::Result<()> {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugin/heterarchy-codex/skills/codex-heterarchy");
        verify_repository_assets(&source)
    }
}
