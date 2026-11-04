use std::collections::BTreeSet;
use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::config;
use super::types::InstallChanges;

struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
}

const EMBEDDED_PLUGIN: &[EmbeddedAsset] = &[
    EmbeddedAsset {
        path: "mcp_config.json",
        bytes: include_bytes!("../../assets/heterarchy-codex/mcp_config.json"),
    },
    EmbeddedAsset {
        path: "plugin.json",
        bytes: include_bytes!("../../assets/heterarchy-codex/plugin.json"),
    },
    EmbeddedAsset {
        path: "rules/AGENTS.md",
        bytes: include_bytes!("../../assets/heterarchy-codex/rules/AGENTS.md"),
    },
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

const EMBEDDED_SKILL_PATHS: &[&str] = &[
    "skills/codex-heterarchy/SKILL.md",
    "skills/codex-heterarchy/agents/openai.yaml",
];
const EMBEDDED_PROVENANCE: &[u8] = include_bytes!("../../assets/PROVENANCE.json");

pub(crate) struct InstallPaths {
    pub active_plugin: PathBuf,
    pub disabled_plugin: PathBuf,
    pub gemini_skill: PathBuf,
    pub codex_skill: PathBuf,
}

impl InstallPaths {
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            active_plugin: home.join(".gemini/config/plugins/heterarchy-codex"),
            disabled_plugin: home.join(".gemini/config/plugins-disabled/heterarchy-codex"),
            gemini_skill: home.join(".gemini/config/skills/codex-heterarchy"),
            codex_skill: home.join(".codex/skills/codex-heterarchy"),
        }
    }
}

pub(crate) fn activate(repository: &Path, home: &Path) -> crate::Result<InstallChanges> {
    let paths = InstallPaths::new(home);
    install_plugin(repository, &paths.active_plugin)?;
    remove_managed(&paths.disabled_plugin)?;
    synchronize_skills(repository, &paths, Some(&paths.active_plugin))?;
    let configuration_changed = config::set_enabled(repository, home, true)?;
    let launcher_migrated = super::launcher::migrate(repository, home)?;
    Ok(InstallChanges {
        configuration_changed,
        assets_synchronized: true,
        launcher_migrated,
    })
}

pub(crate) fn synchronize(repository: &Path, home: &Path) -> crate::Result<InstallChanges> {
    let paths = InstallPaths::new(home);
    let plugin_target = if paths.active_plugin.is_dir() {
        install_plugin(repository, &paths.active_plugin)?;
        Some(paths.active_plugin.as_path())
    } else if paths.disabled_plugin.is_dir() {
        install_plugin(repository, &paths.disabled_plugin)?;
        Some(paths.disabled_plugin.as_path())
    } else {
        None
    };
    synchronize_skills(repository, &paths, plugin_target)?;
    let status = config::inspect(home)?;
    let configuration_changed = if status.configured && status.enabled {
        config::set_enabled(repository, home, true)?
    } else {
        false
    };
    let launcher_migrated = super::launcher::migrate(repository, home)?;
    Ok(InstallChanges {
        configuration_changed,
        assets_synchronized: true,
        launcher_migrated,
    })
}

pub(crate) fn disable_plugin(home: &Path) -> crate::Result<bool> {
    let paths = InstallPaths::new(home);
    if !paths.active_plugin.exists() {
        return Ok(false);
    }
    if let Some(parent) = paths.disabled_plugin.parent() {
        fs::create_dir_all(parent)?;
    }
    remove_managed(&paths.disabled_plugin)?;
    fs::rename(paths.active_plugin, paths.disabled_plugin)?;
    Ok(true)
}

fn install_plugin(repository: &Path, target: &Path) -> crate::Result<()> {
    let source = repository.join("plugin/heterarchy-codex");
    remove_managed(target)?;
    if source.is_dir() {
        verify_repository_assets(&source)?;
        copy_tree(&source, target)?;
    } else {
        write_embedded(target, EMBEDDED_PLUGIN)?;
    }
    write_mcp_config(repository, target)?;
    update_plugin_manifest(target)?;
    Ok(())
}

fn synchronize_skills(
    repository: &Path,
    paths: &InstallPaths,
    plugin_target: Option<&Path>,
) -> crate::Result<()> {
    let primary = repository.join("plugin/heterarchy-codex/skills/codex-heterarchy");
    for target in [&paths.gemini_skill, &paths.codex_skill] {
        remove_managed(target)?;
        materialize_skill(&primary, target)?;
    }
    if let Some(plugin) = plugin_target {
        let target = plugin.join("skills/codex-heterarchy");
        remove_managed(&target)?;
        materialize_skill(&primary, &target)?;
    }
    Ok(())
}

fn materialize_skill(repository_skill: &Path, target: &Path) -> crate::Result<()> {
    if repository_skill.is_dir() {
        let plugin_root = repository_skill
            .ancestors()
            .nth(2)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid plugin skill path"))?;
        verify_repository_assets(plugin_root)?;
        copy_tree(repository_skill, target)
    } else {
        let skill_assets = EMBEDDED_PLUGIN
            .iter()
            .filter(|asset| EMBEDDED_SKILL_PATHS.contains(&asset.path))
            .map(|asset| EmbeddedAsset {
                path: asset
                    .path
                    .strip_prefix("skills/codex-heterarchy/")
                    .unwrap_or(asset.path),
                bytes: asset.bytes,
            })
            .collect::<Vec<_>>();
        write_embedded(target, &skill_assets)
    }
}

fn verify_repository_assets(source: &Path) -> crate::Result<()> {
    verify_embedded_assets()?;
    let expected = EMBEDDED_PLUGIN
        .iter()
        .map(|asset| asset.path.to_owned())
        .collect::<BTreeSet<_>>();
    let actual = collect_relative_files(source, source)?;
    if actual != expected {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "repository plugin asset inventory differs from embedded provenance",
        )
        .into());
    }
    for asset in EMBEDDED_PLUGIN {
        let bytes = fs::read(source.join(asset.path))?;
        if bytes != asset.bytes {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                format!("repository plugin asset drift detected: {}", asset.path),
            )
            .into());
        }
    }
    Ok(())
}

fn verify_embedded_assets() -> crate::Result<()> {
    let provenance: Value = serde_json::from_slice(EMBEDDED_PROVENANCE)?;
    for asset in EMBEDDED_PLUGIN {
        let declared = provenance["files"][asset.path].as_str();
        let expected = format!("sha256:{}", sha256(asset.bytes));
        if declared != Some(expected.as_str()) {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                format!("embedded plugin asset digest mismatch: {}", asset.path),
            )
            .into());
        }
    }
    Ok(())
}

fn write_embedded(target: &Path, assets: &[EmbeddedAsset]) -> crate::Result<()> {
    verify_embedded_assets()?;
    for asset in assets {
        let destination = target.join(asset.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, asset.bytes)?;
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
                format!(
                    "managed assets may not contain symlinks: {}",
                    entry.path().display()
                ),
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

fn copy_tree(source: &Path, target: &Path) -> crate::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_symlink() {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                format!(
                    "managed assets may not contain symlinks: {}",
                    entry.path().display()
                ),
            )
            .into());
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if kind.is_file() {
            let _ = fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn remove_managed(path: &Path) -> crate::Result<()> {
    if path.is_symlink() || path.is_file() {
        fs::remove_file(path)?;
    } else if path.is_dir() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct McpConfig<'a> {
    #[serde(rename = "mcpServers")]
    servers: BTreeServers<'a>,
}

#[derive(Serialize)]
struct BTreeServers<'a> {
    #[serde(rename = "heterarchy-codex")]
    heterarchy_codex: McpEntry<'a>,
}

#[derive(Serialize)]
struct McpEntry<'a> {
    command: &'a str,
    args: [&'static str; 1],
}

fn write_mcp_config(repository: &Path, plugin: &Path) -> crate::Result<()> {
    let binary = config::resolve_hcodex_binary(repository)?;
    let command = binary.to_string_lossy();
    let payload = McpConfig {
        servers: BTreeServers {
            heterarchy_codex: McpEntry {
                command: &command,
                args: ["mcp"],
            },
        },
    };
    let mut encoded = serde_json::to_string_pretty(&payload)?;
    encoded.push('\n');
    fs::write(plugin.join("mcp_config.json"), encoded)?;
    Ok(())
}

fn update_plugin_manifest(plugin: &Path) -> crate::Result<()> {
    let path = plugin.join("plugin.json");
    let mut object = if path.is_file() {
        serde_json::from_str::<Map<String, Value>>(&fs::read_to_string(&path)?)?
    } else {
        Map::new()
    };
    object.insert(
        "name".to_owned(),
        Value::String("heterarchy-codex".to_owned()),
    );
    object.insert(
        "description".to_owned(),
        Value::String("Rust-native receipt-verified Tri-Polar Heterarchy bridge.".to_owned()),
    );
    let mut encoded = serde_json::to_string_pretty(&object)?;
    encoded.push('\n');
    fs::write(path, encoded)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_asset_content_drift_remains_rejected() -> crate::Result<()> {
        let directory = tempfile::tempdir()?;
        write_embedded(directory.path(), EMBEDDED_PLUGIN)?;
        verify_repository_assets(directory.path())?;
        fs::write(
            directory.path().join("skills/codex-heterarchy/SKILL.md"),
            "unverified replacement",
        )?;
        assert!(verify_repository_assets(directory.path()).is_err());
        Ok(())
    }
    #[test]
    fn repository_assets_equal_embedded_provenance() -> crate::Result<()> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        verify_repository_assets(&repository.join("plugin/heterarchy-codex"))?;
        verify_embedded_assets()
    }
}
