//! Bounded-output verification processes owned through completion or cancellation.
//!
//! Callers own command admission. They must await this future through cooperative
//! cancellation; aborting/dropping it is not a completed cleanup operation.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GateOutcome {
    Completed,
    OwnedProcessesRemain,
    TimedOut,
    Cancelled,
}

#[derive(Debug)]
pub(crate) struct ProcessProof {
    pub owned_processes_closed: bool,
    pub outcome: GateOutcome,
    pub exit_code: Option<i32>,
    pub duration_seconds: f64,
    pub stdout_digest: String,
    pub stderr_digest: String,
}

pub(crate) async fn run(
    workspace: &Path,
    limit: Duration,
    program: &str,
    arguments: &[&str],
) -> crate::Result<ProcessProof> {
    run_profile(workspace, limit, program, arguments, false).await
}

pub(crate) async fn run_isolated(
    workspace: &Path,
    limit: Duration,
    program: &str,
    arguments: &[&str],
) -> crate::Result<ProcessProof> {
    run_profile(workspace, limit, program, arguments, true).await
}

async fn run_profile(
    workspace: &Path,
    limit: Duration,
    program: &str,
    arguments: &[&str],
    isolated_build: bool,
) -> crate::Result<ProcessProof> {
    let mut process = Command::new(program);
    if isolated_build {
        process.env("CARGO_TARGET_DIR", workspace.join("target"));
    }
    process
        .env("PATH", crate::runtime::enriched_command_path())
        .args(arguments)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let started = Instant::now();
    let mut child = process.spawn()?;
    // Retain group identity even if the direct child exits before descendants
    // release inherited pipes. Child::id becomes None after wait completes.
    let process_group = child.id();
    let mut stdout = child
        .stdout
        .take()
        .ok_or("verification stdout pipe missing")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("verification stderr pipe missing")?;
    let (mut stdout_hash, mut stderr_hash) = (Sha256::new(), Sha256::new());
    enum WaitResult {
        Completed(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }
    let result = {
        let operation = async {
            let (status, (), ()) = tokio::try_join!(
                child.wait(),
                hash_stream(&mut stdout, &mut stdout_hash),
                hash_stream(&mut stderr, &mut stderr_hash),
            )?;
            Ok::<_, std::io::Error>(status)
        };
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => WaitResult::Completed(result),
            () = tokio::time::sleep(limit) => WaitResult::TimedOut,
            () = crate::supervisor::cancelled() => WaitResult::Cancelled,
        }
    };
    let (outcome, exit_code) = match result {
        WaitResult::Completed(Ok(status)) => (GateOutcome::Completed, status.code()),
        WaitResult::Completed(Err(error)) => {
            terminate_and_reap(&mut child, process_group).await?;
            return Err(error.into());
        }
        WaitResult::TimedOut | WaitResult::Cancelled => {
            terminate_and_reap(&mut child, process_group).await?;
            (
                if matches!(result, WaitResult::Cancelled) {
                    GateOutcome::Cancelled
                } else {
                    GateOutcome::TimedOut
                },
                None,
            )
        }
    };
    let mut proof = proof(outcome, exit_code, started, stdout_hash, stderr_hash);
    proof.owned_processes_closed = owned_process_group_closed(process_group).await;
    if proof.outcome == GateOutcome::Completed && !proof.owned_processes_closed {
        proof.outcome = GateOutcome::OwnedProcessesRemain;
    }
    proof.duration_seconds = started.elapsed().as_secs_f64();
    Ok(proof)
}

/// Positive absence observation for the exact group created by this process.
/// Errors, unsupported platforms and truncated process inventories are unproven.
pub(crate) async fn owned_process_group_closed(group: Option<u32>) -> bool {
    #[cfg(not(unix))]
    {
        let _ = group;
        false
    }
    #[cfg(unix)]
    {
        let Some(group) = group else {
            return false;
        };
        let mut command = Command::new("/bin/ps");
        command
            .args(["-axo", "pgid="])
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let Ok(mut child) = command.spawn() else {
            return false;
        };
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return false;
        };
        const MAX_OUTPUT: u64 = 256 * 1024;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut stdout = stdout.take(MAX_OUTPUT + 1);
        let mut stderr = stderr.take(4097);
        let operation = async {
            let (status, _, _) = tokio::try_join!(
                child.wait(),
                stdout.read_to_end(&mut out),
                stderr.read_to_end(&mut err),
            )?;
            Ok::<_, std::io::Error>(status)
        };
        let observed = tokio::time::timeout(Duration::from_secs(5), operation).await;
        if !matches!(observed, Ok(Ok(status)) if status.success()) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return false;
        }
        if !err.is_empty() {
            return false;
        }
        inventory_proves_group_absent(&out, group)
    }
}

#[cfg(unix)]
fn inventory_proves_group_absent(output: &[u8], group: u32) -> bool {
    if output.len() > 256 * 1024 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(output) else {
        return false;
    };
    let mut rows = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(found) = line.trim().parse::<u32>() else {
            return false;
        };
        rows += 1;
        if found == group {
            return false;
        }
    }
    rows > 0
}

fn proof(
    outcome: GateOutcome,
    exit_code: Option<i32>,
    started: Instant,
    stdout_hash: Sha256,
    stderr_hash: Sha256,
) -> ProcessProof {
    ProcessProof {
        owned_processes_closed: false,
        outcome,
        exit_code,
        duration_seconds: started.elapsed().as_secs_f64(),
        stdout_digest: format!("sha256:{:x}", stdout_hash.finalize()),
        stderr_digest: format!("sha256:{:x}", stderr_hash.finalize()),
    }
}

async fn hash_stream<R: AsyncRead + Unpin>(
    reader: &mut R,
    hasher: &mut Sha256,
) -> std::io::Result<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        let length = reader.read(&mut buffer).await?;
        if length == 0 {
            return Ok(());
        }
        hasher.update(&buffer[..length]);
    }
}

pub(crate) async fn terminate_and_reap(
    child: &mut Child,
    process_group: Option<u32>,
) -> crate::Result<()> {
    #[cfg(unix)]
    if let Some(pid) = process_group {
        let group = format!("-{pid}");
        let _group_status = Command::new("kill")
            .args(["-KILL", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;
    }
    #[cfg(not(unix))]
    let _ = process_group;
    if let Err(error) = child.kill().await
        && !matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
        )
    {
        return Err(error.into());
    }
    let _reaped = child.wait().await?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn ready_pids(path: &Path) -> crate::Result<Vec<String>> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(body) = std::fs::read_to_string(path) {
                    let pids = body.lines().map(str::to_owned).collect::<Vec<_>>();
                    if pids.len() == 2 {
                        return pids;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(Into::into)
    }

    async fn assert_reaped_and_not_running(pids: &[String]) -> crate::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let parent_alive = Command::new("/bin/kill")
                .args(["-0", &pids[0]])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await?
                .success();
            let state = Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", &pids[1]])
                .output()
                .await?;
            let state = String::from_utf8_lossy(&state.stdout);
            let descendant_done = state.trim().is_empty() || state.trim().starts_with('Z');
            if !parent_alive && descendant_done {
                return Ok(());
            }
            if Instant::now() >= deadline {
                assert!(!parent_alive, "direct child must be reaped");
                assert!(descendant_done, "descendant is still running: {state}");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    #[tokio::test]
    async fn hashes_large_output_without_retaining_it() -> crate::Result<()> {
        let root = tempfile::tempdir()?;
        let result = run(
            root.path(),
            Duration::from_secs(5),
            "sh",
            &["-c", "head -c 1048576 /dev/zero; printf stderr >&2"],
        )
        .await?;
        assert_eq!(result.outcome, GateOutcome::Completed);
        assert!(result.owned_processes_closed);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            result.stdout_digest,
            format!("sha256:{:x}", Sha256::digest(vec![0_u8; 1048576]))
        );
        assert_eq!(
            result.stderr_digest,
            format!("sha256:{:x}", Sha256::digest(b"stderr"))
        );
        Ok(())
    }

    #[test]
    fn group_inventory_requires_complete_valid_positive_observation() {
        assert!(inventory_proves_group_absent(b" 1\n2\n", 3));
        assert!(!inventory_proves_group_absent(b"1\n2\n", 2));
        assert!(!inventory_proves_group_absent(b"", 3));
        assert!(!inventory_proves_group_absent(b"1\nPGID\n", 3));
        assert!(!inventory_proves_group_absent(b"\xff", 3));
        assert!(!inventory_proves_group_absent(&b"1\n".repeat(131_073), 3));
    }

    #[tokio::test]
    async fn completed_gate_with_detached_stdio_descendant_is_not_completed_proof()
    -> crate::Result<()> {
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(body) = std::fs::read_to_string(&self.0)
                    && let Ok(pid) = body.trim().parse::<u32>()
                {
                    let _ = std::process::Command::new("/bin/kill")
                        .args(["-KILL", &pid.to_string()])
                        .status();
                }
            }
        }
        let root = tempfile::tempdir()?;
        let _owned_background = Cleanup(root.path().join("background-pid"));
        let proof = run(
            root.path(),
            Duration::from_secs(5),
            "sh",
            &[
                "-c",
                "sleep 30 </dev/null >/dev/null 2>&1 & printf '%s' \"$!\" > background-pid; exit 0",
            ],
        )
        .await?;
        assert_eq!(proof.exit_code, Some(0));
        assert!(!proof.owned_processes_closed);
        assert_eq!(proof.outcome, GateOutcome::OwnedProcessesRemain);
        Ok(())
    }

    #[tokio::test]
    async fn timeout_reaps_group_after_leader_exits_with_inherited_pipes() -> crate::Result<()> {
        let root = tempfile::tempdir()?;
        let workspace = root.path().to_path_buf();
        let task = tokio::spawn(async move {
            run(
                &workspace,
                Duration::from_secs(1),
                "sh",
                &[
                    "-c",
                    "sleep 30 & child=$!; printf '%s\\n%s\\n' \"$$\" \"$child\" > ready; exit 0",
                ],
            )
            .await
        });
        let pids = ready_pids(&root.path().join("ready")).await?;
        let proof = task.await??;
        assert_eq!(proof.outcome, GateOutcome::TimedOut);
        assert_eq!(proof.exit_code, None);
        assert_reaped_and_not_running(&pids).await
    }

    #[tokio::test]
    async fn supervisor_cancellation_reaps_owned_group() -> crate::Result<()> {
        let root = tempfile::tempdir()?;
        let workspace = root.path().to_path_buf();
        let supervisor = crate::supervisor::Supervisor::new(1);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        supervisor
            .spawn("verification-test".to_owned(), async move {
                let proof = run(
                    &workspace,
                    Duration::from_secs(30),
                    "sh",
                    &[
                        "-c",
                        "sleep 30 & child=$!; printf '%s\\n%s\\n' \"$$\" \"$child\" > ready; wait",
                    ],
                )
                .await;
                let _sent = sender.send(proof);
            })
            .await?;
        let pids = ready_pids(&root.path().join("ready")).await?;
        assert!(supervisor.cancel("verification-test").await);
        let proof = tokio::time::timeout(Duration::from_secs(5), receiver).await???;
        assert_eq!(proof.outcome, GateOutcome::Cancelled);
        assert_eq!(proof.exit_code, None);
        supervisor.shutdown(Duration::from_secs(1)).await?;
        assert_reaped_and_not_running(&pids).await
    }
}
