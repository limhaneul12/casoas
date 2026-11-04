//! Bounded Git subprocesses with explicit group ownership and cancellation.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::Result;

async fn bounded<R: AsyncRead + Unpin>(r: R) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    r.take(2 * 1024 * 1024 + 1).read_to_end(&mut v).await?;
    if v.len() > 2 * 1024 * 1024 {
        return Err(std::io::Error::other("Git output budget exceeded"));
    }
    Ok(v)
}
pub(crate) async fn run(root: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut c = Command::new("git");
    c.env("PATH", crate::runtime::enriched_command_path())
        .env("LC_ALL", "C")
        .args([
            "--no-optional-locks",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            c.env_remove(key);
        }
    }
    #[cfg(unix)]
    c.process_group(0);
    let mut child = c.spawn()?;
    let pg = child.id();
    let stdout = child.stdout.take().ok_or("Git stdout missing")?;
    let stderr = child.stderr.take().ok_or("Git stderr missing")?;
    let operation = async {
        let (status, out, err) = tokio::try_join!(child.wait(), bounded(stdout), bounded(stderr))?;
        Ok::<_, std::io::Error>((status, out, err))
    };
    let result = tokio::select! {
        value = tokio::time::timeout(timeout, operation) => value.map_err(|e|e.to_string()).and_then(|v|v.map_err(|e|e.to_string())),
        () = crate::supervisor::cancelled() => Err("Git operation cancelled".to_owned()),
    };
    match result {
        Ok((status, out, err)) => {
            if !crate::verification::owned_process_group_closed(pg).await {
                return Err(
                    format!("Git owned process-group closure unproven for {args:?}").into(),
                );
            }
            if !status.success() {
                return Err(
                    format!("Git {:?} failed: {}", args, String::from_utf8_lossy(&err)).into(),
                );
            }
            Ok(String::from_utf8(out)?)
        }
        other => {
            #[cfg(unix)]
            if let Some(pid) = pg {
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{pid}")])
                    .status()
                    .await;
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(format!("Git operation failed/timed out: {other:?}").into())
        }
    }
}
