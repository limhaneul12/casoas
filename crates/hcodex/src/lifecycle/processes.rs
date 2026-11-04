use std::fs;
use std::io::{Error as IoError, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use super::config::resolve_hcodex_binary;
use super::types::OwnedProcessHandle;

#[derive(Debug, Serialize)]
pub(crate) struct ProcessStopResult {
    pub pid: u32,
    pub state: &'static str,
    pub detail: String,
}

pub struct OwnedMcpProcessGuard {
    home: PathBuf,
    pid: u32,
    start_identity: String,
    armed: bool,
}

struct StartupObservation {
    binary_sha256: Option<String>,
    package_version: String,
}

impl OwnedMcpProcessGuard {
    pub fn register(home: &Path, executable: &Path) -> crate::Result<Self> {
        let pid = std::process::id();
        let start_identity = process_start_identity(pid)?.ok_or_else(|| {
            IoError::new(
                ErrorKind::NotFound,
                "could not read current MCP process start identity",
            )
        })?;
        let startup = StartupObservation {
            binary_sha256: super::identity::binary_snapshot(executable)
                .map(|snapshot| snapshot.sha256),
            package_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        register_with_identity(home, pid, executable, &start_identity, Some(startup))?;
        Ok(Self {
            home: home.to_path_buf(),
            pid,
            start_identity,
            armed: true,
        })
    }

    pub fn unregister(mut self) -> crate::Result<()> {
        self.armed = false;
        unregister_owned_mcp_process(&self.home, self.pid, &self.start_identity)
    }
}

impl Drop for OwnedMcpProcessGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ignored = unregister_owned_mcp_process(&self.home, self.pid, &self.start_identity);
        }
    }
}

pub fn register_owned_mcp_process(home: &Path, pid: u32, executable: &Path) -> crate::Result<()> {
    if pid == 0 {
        return Err(IoError::new(ErrorKind::InvalidInput, "PID must be non-zero").into());
    }
    let start_identity = process_start_identity(pid)?.ok_or_else(|| {
        IoError::new(
            ErrorKind::NotFound,
            format!("cannot register PID {pid}: process is not running"),
        )
    })?;
    register_with_identity(home, pid, executable, &start_identity, None)
}

fn register_with_identity(
    home: &Path,
    pid: u32,
    executable: &Path,
    start_identity: &str,
    startup: Option<StartupObservation>,
) -> crate::Result<()> {
    let executable = executable.canonicalize()?;
    let (startup_binary_sha256, registrant_package_version) = startup
        .map(|observation| (observation.binary_sha256, Some(observation.package_version)))
        .unwrap_or_default();
    let mut handles = read_registry(home)?;
    handles.retain(|handle| handle.pid != pid);
    handles.push(OwnedProcessHandle {
        pid,
        executable,
        arguments: vec!["mcp".to_owned()],
        start_identity: start_identity.to_owned(),
        registered_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis()),
        startup_binary_sha256,
        registrant_package_version,
    });
    write_registry(home, &handles)
}

pub fn unregister_owned_mcp_process(
    home: &Path,
    pid: u32,
    start_identity: &str,
) -> crate::Result<()> {
    let mut handles = read_registry(home)?;
    handles.retain(|handle| handle.pid != pid || handle.start_identity.as_str() != start_identity);
    write_registry(home, &handles)
}

pub(crate) fn read_registry(home: &Path) -> crate::Result<Vec<OwnedProcessHandle>> {
    let path = registry_path(home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&fs::read_to_string(path)?).map_err(Into::into)
}

pub(crate) async fn stop_owned_processes(
    repository: &Path,
    home: &Path,
) -> crate::Result<Vec<ProcessStopResult>> {
    let repository = repository.to_path_buf();
    let home = home.to_path_buf();
    let home_for_read = home.clone();
    let (expected_binary, handles) = tokio::task::spawn_blocking(move || {
        Ok::<_, crate::Error>((
            resolve_hcodex_binary(&repository)?,
            read_registry(&home_for_read)?,
        ))
    })
    .await??;
    let mut remaining = Vec::new();
    let mut results = Vec::new();
    for handle in handles {
        if handle.pid == std::process::id() {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "identity_rejected",
                detail: "registry entry points at the lifecycle controller process".to_owned(),
            });
            remaining.push(handle);
            continue;
        }
        let executable = handle.executable.clone();
        let pid = handle.pid;
        let (registered_binary, live_start) = tokio::task::spawn_blocking(move || {
            Ok::<_, crate::Error>((executable.canonicalize().ok(), process_start_identity(pid)?))
        })
        .await??;
        if registered_binary.as_deref() != Some(expected_binary.as_path())
            || handle.arguments != ["mcp"]
        {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "identity_rejected",
                detail: "registered executable or arguments do not match the owned MCP identity"
                    .to_owned(),
            });
            remaining.push(handle);
            continue;
        }
        if live_start.as_deref() != Some(handle.start_identity.as_str()) {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "stale_registry_removed",
                detail: "PID start identity no longer matches the registered process".to_owned(),
            });
            continue;
        }
        let observed = observe_command(handle.pid).await?;
        let Some(command) = observed else {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "stale_registry_removed",
                detail: "process is not running".to_owned(),
            });
            continue;
        };
        if !command_matches(&expected_binary, &command) {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "identity_rejected",
                detail: "live process command does not match the registered Rust MCP".to_owned(),
            });
            remaining.push(handle);
            continue;
        }
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(handle.pid.to_string())
            .status()
            .await?;
        if status.success() {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "termination_requested",
                detail: "SIGTERM sent to verified owned MCP process".to_owned(),
            });
        } else {
            results.push(ProcessStopResult {
                pid: handle.pid,
                state: "termination_failed",
                detail: format!("kill exited with {status}"),
            });
            remaining.push(handle);
        }
    }
    tokio::task::spawn_blocking(move || write_registry(&home, &remaining)).await??;
    Ok(results)
}

pub(super) async fn observe_command(pid: u32) -> crate::Result<Option<String>> {
    observe_ps_field(pid, "command=").await
}

pub(super) async fn observe_start_identity(pid: u32) -> crate::Result<Option<String>> {
    observe_ps_field(pid, "lstart=").await
}

/// Fixed metadata probes share the existing process-group cleanup authority.
async fn observe_ps_field(pid: u32, field: &str) -> crate::Result<Option<String>> {
    let mut command = Command::new("/bin/ps");
    command
        .args(["-p", &pid.to_string(), "-o", field])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let group = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or("process observation stdout missing")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("process observation stderr missing")?;
    let operation = async {
        tokio::try_join!(
            child.wait(),
            bounded_ps_output(stdout),
            bounded_ps_output(stderr)
        )
    };
    let observed = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(2), operation) => result.ok().and_then(Result::ok),
        () = crate::supervisor::cancelled() => None,
    };
    match observed {
        Some((status, stdout, _)) => {
            if !crate::verification::owned_process_group_closed(group).await {
                crate::verification::terminate_and_reap(&mut child, group).await?;
                return Err("process observation closure unproven".into());
            }
            if !status.success() {
                return Ok(None);
            }
            let text = String::from_utf8(stdout)?.trim().to_owned();
            Ok((!text.is_empty()).then_some(text))
        }
        None => {
            crate::verification::terminate_and_reap(&mut child, group).await?;
            Err("process observation unavailable".into())
        }
    }
}

async fn bounded_ps_output<R: AsyncRead + Unpin>(reader: R) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(65_537).read_to_end(&mut bytes).await?;
    if bytes.len() > 65_536 {
        return Err(IoError::other("process observation exceeds byte limit"));
    }
    Ok(bytes)
}

pub(super) fn process_start_identity(pid: u32) -> crate::Result<Option<String>> {
    let output = StdCommand::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .env("LC_ALL", "C")
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!identity.is_empty()).then_some(identity))
}

pub(super) fn command_matches(expected_binary: &Path, command: &str) -> bool {
    let binary = expected_binary.to_string_lossy();
    command
        .strip_prefix(binary.as_ref())
        .is_some_and(|remainder| {
            remainder.starts_with(char::is_whitespace) && remainder.split_whitespace().eq(["mcp"])
        })
}

pub(super) fn registry_path(home: &Path) -> PathBuf {
    home.join(".cache/hcodex/owned-mcp-processes.json")
}

fn write_registry(home: &Path, handles: &[OwnedProcessHandle]) -> crate::Result<()> {
    let path = registry_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut content = serde_json::to_string_pretty(handles)?;
    content.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_process_observation_matches_registration_start_identity() -> crate::Result<()>
    {
        let pid = std::process::id();
        let recorded = process_start_identity(pid)?;
        assert!(recorded.is_some());
        assert_eq!(observe_start_identity(pid).await?, recorded);
        assert!(observe_command(pid).await?.is_some());
        Ok(())
    }

    #[test]
    fn command_identity_requires_exact_binary_and_mcp_argument() {
        let binary = Path::new("/tmp/hcodex");
        assert!(command_matches(binary, "/tmp/hcodex mcp"));
        assert!(!command_matches(binary, "/tmp/hcodex tui"));
        assert!(!command_matches(binary, "/tmp/hcodex-evil mcp"));
        assert!(!command_matches(binary, "/bin/sh /tmp/hcodex mcp"));
        assert!(!command_matches(binary, "/tmp/other mcp"));
        for ambiguous in [
            "/tmp/hcodex tui -c mcp",
            "/tmp/hcodex --worktree-name mcp tui",
            "/tmp/hcodex --workspace /repo mcp",
            "/tmp/hcodex mcp --workspace /repo",
            "/tmp/hcodex mcp extra",
            "/tmp/hcodex",
            "/tmp/hcodexmcp",
        ] {
            assert!(!command_matches(binary, ambiguous), "{ambiguous}");
        }
    }

    #[test]
    fn guard_registration_is_removed_on_drop() -> crate::Result<()> {
        let home = tempfile::tempdir()?;
        let executable = std::env::current_exe()?;
        let guard = OwnedMcpProcessGuard::register(home.path(), &executable)?;
        let registered = read_registry(home.path())?;
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].pid, std::process::id());
        assert!(!registered[0].start_identity.is_empty());
        assert!(registered[0].startup_binary_sha256.is_some());
        assert_eq!(
            registered[0].registrant_package_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        drop(guard);
        assert!(read_registry(home.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn late_public_registration_after_replacement_has_no_startup_witness() -> crate::Result<()> {
        let home = tempfile::tempdir()?;
        let executable = home.path().join("hcodex");
        fs::write(&executable, b"original-startup-image")?;
        fs::write(&executable, b"later-installed-replacement")?;
        // Even the current PID through the public external-registration API is
        // not the owned guard's initialization call site.
        register_owned_mcp_process(home.path(), std::process::id(), &executable)?;
        let registered = read_registry(home.path())?;
        assert_eq!(registered.len(), 1);
        assert!(registered[0].startup_binary_sha256.is_none());
        assert!(registered[0].registrant_package_version.is_none());
        Ok(())
    }
}
