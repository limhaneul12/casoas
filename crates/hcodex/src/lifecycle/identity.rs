//! Read-only, scoped observations of source, installation and current process identity.

use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use super::config;

const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_FILES: usize = 4096;
static OBSERVATIONS: Semaphore = Semaphore::const_new(2);

#[derive(Clone)]
pub(super) struct BinarySnapshot {
    pub path: PathBuf,
    pub sha256: String,
}

/// The disk file is observed, not the OS image already mapped by a process.
pub(super) fn binary_snapshot(path: &Path) -> Option<BinarySnapshot> {
    let path = path.canonicalize().ok()?;
    let before = fs::symlink_metadata(&path).ok()?;
    if !before.is_file() || before.len() > MAX_BINARY_BYTES {
        return None;
    }
    let mut file = File::open(&path).ok()?;
    if !same_file(&before, &file.metadata().ok()?) {
        return None;
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let started = Instant::now();
    let mut count = 0_u64;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        count = count.checked_add(u64::try_from(read).ok()?)?;
        if count > MAX_BINARY_BYTES || started.elapsed() > Duration::from_secs(5) {
            return None;
        }
        hash.update(&buffer[..read]);
    }
    if !same_file(&before, &file.metadata().ok()?)
        || !same_file(&before, &fs::symlink_metadata(&path).ok()?)
    {
        return None;
    }
    Some(BinarySnapshot {
        path,
        sha256: format!("sha256:{:x}", hash.finalize()),
    })
}

pub(crate) async fn observe(workspace: &Path, home: Option<&Path>) -> Value {
    let Ok(_permit) = OBSERVATIONS.acquire().await else {
        return json!({"observation_only":true,"state":"unknown","reason":"identity_unavailable"});
    };
    let source = source_observation(workspace).await;
    let repository = workspace.to_path_buf();
    let home = home.map(Path::to_path_buf);
    let disk =
        tokio::task::spawn_blocking(move || disk_observations(&repository, home.as_deref())).await;
    let (installed, current) = disk.unwrap_or((None, None));
    json!({
        "observation_only":true,
        "source":source,
        "installed_cli":binary_value(installed.as_ref()),
        "current_process":{
            "pid":std::process::id(),
            "package_version":env!("CARGO_PKG_VERSION"),
            "executable_on_disk":binary_value(current.as_ref()),
            "loaded_image_identity":"unverified",
        },
        "capabilities":{"backend":"in_process","mcp_server":false,"realtime_projection":"bounded_soft_realtime"},
        "verification":{"source_to_binary":"unverified","ci":"not_supplied"},
    })
}

fn binary_value(snapshot: Option<&BinarySnapshot>) -> Value {
    match snapshot {
        Some(snapshot) => json!({
            "state":"observed", "path":snapshot.path, "sha256":snapshot.sha256,
            "package_version":null,"scope":"current_disk_file",
        }),
        None => json!({"state":"unknown","path":null,"sha256":null,"package_version":null}),
    }
}

fn disk_observations(
    repository: &Path,
    home: Option<&Path>,
) -> (Option<BinarySnapshot>, Option<BinarySnapshot>) {
    let installed_path = home
        .map(|home| home.join(".cargo/bin/hcodex"))
        .filter(|path| path.is_file())
        .or_else(|| config::resolve_hcodex_binary(repository).ok());
    let installed = installed_path.as_deref().and_then(binary_snapshot);
    let current_path = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    let current = match (&installed, current_path.as_deref()) {
        (Some(installed), Some(path)) if installed.path == path => Some(installed.clone()),
        (_, Some(path)) => binary_snapshot(path),
        _ => None,
    };
    (installed, current)
}

async fn source_observation(workspace: &Path) -> Value {
    let Ok(workspace) = workspace.canonicalize() else {
        return source_unknown("workspace_unavailable");
    };
    let limit = Duration::from_secs(5);
    let head = match crate::git::run(&workspace, &["rev-parse", "HEAD"], limit).await {
        Ok(head)
            if matches!(head.trim().len(), 40 | 64)
                && head.trim().bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            head.trim().to_owned()
        }
        _ => return source_unknown("git_head_unavailable"),
    };
    let status_args = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
    let status = match crate::git::run(&workspace, &status_args, limit).await {
        Ok(status) => status,
        Err(_) => return source_unknown("git_status_unavailable"),
    };
    let paths = match crate::git::run(
        &workspace,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
            "*.sql",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
        ],
        limit,
    )
    .await
    {
        Ok(paths) => paths,
        Err(_) => return source_unknown("source_paths_unavailable"),
    };
    let source_root = workspace.clone();
    let digest = tokio::task::spawn_blocking(move || source_digest(&source_root, &paths)).await;
    let (digest, count) = match digest {
        Ok(Ok(value)) => value,
        _ => return source_unknown("source_snapshot_unavailable"),
    };
    let after_head = crate::git::run(&workspace, &["rev-parse", "HEAD"], limit).await;
    let after_status = crate::git::run(&workspace, &status_args, limit).await;
    if !matches!(after_head.as_ref(), Ok(value) if value.trim()==head)
        || !matches!(after_status.as_ref(), Ok(value) if value==&status)
    {
        return source_unknown("source_observation_changed");
    }
    json!({"state":"observed","head":head,"dirty":!status.is_empty(),"manifest_digest":digest,"file_count":count,"scope":"git_listed_rust_sql_cargo_toolchain"})
}

fn source_unknown(reason: &'static str) -> Value {
    json!({"state":"unknown","reason":reason,"head":null,"dirty":null,"manifest_digest":null,"scope":"git_listed_rust_sql_cargo_toolchain"})
}

fn source_digest(root: &Path, paths: &str) -> Result<(String, usize), ()> {
    let paths: BTreeSet<_> = paths.split('\0').filter(|path| !path.is_empty()).collect();
    if paths.is_empty() || paths.len() > MAX_SOURCE_FILES {
        return Err(());
    }
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let started = Instant::now();
    let mut buffer = [0; 64 * 1024];
    for relative in &paths {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(());
        }
        let path = root.join(relative_path);
        let before = fs::symlink_metadata(&path).map_err(|_| ())?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || !path.canonicalize().map_err(|_| ())?.starts_with(root)
        {
            return Err(());
        }
        total = total.checked_add(before.len()).ok_or(())?;
        if total > MAX_SOURCE_BYTES || started.elapsed() > Duration::from_secs(5) {
            return Err(());
        }
        let mut file = File::open(&path).map_err(|_| ())?;
        if !same_file(&before, &file.metadata().map_err(|_| ())?) {
            return Err(());
        }
        hash.update(u64::try_from(relative.len()).map_err(|_| ())?.to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update(before.len().to_le_bytes());
        let mut bytes = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(|_| ())?;
            if read == 0 {
                break;
            }
            bytes = bytes
                .checked_add(u64::try_from(read).map_err(|_| ())?)
                .ok_or(())?;
            if bytes > before.len() || started.elapsed() > Duration::from_secs(5) {
                return Err(());
            }
            hash.update(&buffer[..read]);
        }
        if bytes != before.len()
            || !same_file(&before, &file.metadata().map_err(|_| ())?)
            || !same_file(&before, &fs::symlink_metadata(&path).map_err(|_| ())?)
        {
            return Err(());
        }
    }
    Ok((format!("sha256:{:x}", hash.finalize()), paths.len()))
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
        left.len() == right.len() && left.modified().ok() == right.modified().ok()
    }
}

#[cfg(test)]
mod tests;
