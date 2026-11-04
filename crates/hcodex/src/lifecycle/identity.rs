//! Read-only, scoped observations of source, installation and MCP process identity.

use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use super::{OwnedProcessHandle, config, processes};

const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_FILES: usize = 4096;
const MAX_REGISTRY_BYTES: u64 = 64 * 1024;
const MAX_REGISTERED: usize = 32;
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
    let (installed, current, handles, registry_state) = match disk {
        Ok(value) => value,
        Err(_) => (None, None, Vec::new(), "unknown"),
    };
    let mut registered = Vec::new();
    let process_deadline = Instant::now() + Duration::from_secs(5);
    for handle in handles {
        if Instant::now() >= process_deadline {
            let mut view = project_process(&handle, false, installed.as_ref());
            view["reason"] = json!("process_probe_budget_exceeded");
            registered.push(view);
        } else {
            registered
                .push(process_observation(&handle, installed.as_ref(), process_deadline).await);
        }
    }
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
        "registered_mcp":registered,
        "registry_observation":registry_state,
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
) -> (
    Option<BinarySnapshot>,
    Option<BinarySnapshot>,
    Vec<OwnedProcessHandle>,
    &'static str,
) {
    let selected = match home.map(config::inspect) {
        Some(Ok(config)) => match config.command {
            Some(command) => resolve_command(&command),
            None => config::resolve_hcodex_binary(repository).ok(),
        },
        Some(Err(_)) => None,
        None => config::resolve_hcodex_binary(repository).ok(),
    };
    let installed = selected.as_deref().and_then(binary_snapshot);
    let current_path = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    let current = match (&installed, current_path.as_deref()) {
        (Some(installed), Some(path)) if installed.path == path => Some(installed.clone()),
        (_, Some(path)) => binary_snapshot(path),
        _ => None,
    };
    let (handles, state) = match home.map(read_handles) {
        Some(Ok(handles)) => (handles, "observed"),
        Some(Err(())) => (Vec::new(), "unknown"),
        None => (Vec::new(), "unavailable"),
    };
    (installed, current, handles, state)
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    if command.is_empty() || command.chars().any(char::is_control) {
        return None;
    }
    let path = Path::new(command);
    if path.is_absolute() {
        return path.canonicalize().ok();
    }
    if path.components().count() != 1 {
        return None;
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(command))
            .find_map(|candidate| candidate.canonicalize().ok())
    })
}

fn read_handles(home: &Path) -> Result<Vec<OwnedProcessHandle>, ()> {
    let path = processes::registry_path(home);
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(()),
    };
    if !before.is_file() || before.file_type().is_symlink() || before.len() > MAX_REGISTRY_BYTES {
        return Err(());
    }
    let file = File::open(&path).map_err(|_| ())?;
    if !same_file(&before, &file.metadata().map_err(|_| ())?) {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(MAX_REGISTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > MAX_REGISTRY_BYTES as usize
        || !same_file(&before, &fs::symlink_metadata(path).map_err(|_| ())?)
    {
        return Err(());
    }
    let handles: Vec<OwnedProcessHandle> = serde_json::from_slice(&bytes).map_err(|_| ())?;
    if handles.len() > MAX_REGISTERED {
        return Err(());
    }
    Ok(handles)
}

async fn process_observation(
    handle: &OwnedProcessHandle,
    installed: Option<&BinarySnapshot>,
    deadline: Instant,
) -> Value {
    let pid = handle.pid;
    let live_start = processes::observe_start_identity(pid).await.ok().flatten();
    if handle.start_identity.is_empty()
        || live_start.as_deref() != Some(handle.start_identity.as_str())
    {
        return project_process(handle, false, installed);
    }
    if Instant::now() >= deadline {
        return project_process(handle, false, installed);
    }
    let command = processes::observe_command(pid).await.ok().flatten();
    if Instant::now() >= deadline {
        return project_process(handle, false, installed);
    }
    let after = processes::observe_start_identity(pid).await.ok().flatten();
    let live = !handle.start_identity.is_empty()
        && live_start.as_deref() == Some(handle.start_identity.as_str())
        && after == live_start
        && handle.arguments == ["mcp"]
        && command
            .as_deref()
            .is_some_and(|command| processes::command_matches(&handle.executable, command));
    project_process(handle, live, installed)
}

fn project_process(
    handle: &OwnedProcessHandle,
    live: bool,
    installed: Option<&BinarySnapshot>,
) -> Value {
    let recorded = handle
        .startup_binary_sha256
        .as_deref()
        .filter(|value| valid_digest(value));
    let comparison = match (live, recorded, installed) {
        (true, Some(recorded), Some(installed)) if recorded == installed.sha256 => {
            "same_recorded_disk_hash"
        }
        (true, Some(_), Some(_)) => "different_recorded_disk_hash",
        _ => "unknown",
    };
    json!({
        "pid":handle.pid,
        "state":if live && recorded.is_some() {"observed"} else {"unknown"},
        "process_observation":if live {"pid_start_and_command_match"} else {"unknown"},
        "reason":if !live {"process_identity_unproven"} else if recorded.is_none() {"historical_identity_unavailable"} else {"startup_disk_observation_available"},
        "recorded_binary_sha256":recorded,
        "registrant_package_version":handle.registrant_package_version.as_deref().filter(|value| valid_version(value)),
        "installed_comparison":comparison,
        "scope":"recorded_startup_disk_observation_not_loaded_image",
    })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_version(value: &str) -> bool {
    if value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte))
    {
        return false;
    }
    let core = value.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
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
