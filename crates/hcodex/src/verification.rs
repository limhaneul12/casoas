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
    TimedOut,
    Cancelled,
}

#[derive(Debug)]
pub(crate) struct ProcessProof {
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
    let mut process = Command::new(program);
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
    Ok(proof(outcome, exit_code, started, stdout_hash, stderr_hash))
}

fn proof(
    outcome: GateOutcome,
    exit_code: Option<i32>,
    started: Instant,
    stdout_hash: Sha256,
    stderr_hash: Sha256,
) -> ProcessProof {
    ProcessProof {
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

async fn terminate_and_reap(child: &mut Child, process_group: Option<u32>) -> crate::Result<()> {
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
