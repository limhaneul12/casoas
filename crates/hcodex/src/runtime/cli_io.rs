//! Bounded, owned CLI pipes shared by the execution adapters.
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;

pub(super) const MAX_LINE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RESULT_BYTES: usize = 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;
const STDERR_TAIL_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(super) struct StderrCapture {
    pub bytes: Vec<u8>,
    pub permission_denied: bool,
}

pub(super) async fn read_lines<R>(
    mut reader: R,
    sender: mpsc::Sender<Result<Vec<u8>, &'static str>>,
) where
    R: AsyncRead + Unpin,
{
    let mut pending = Vec::new();
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    loop {
        let size = match reader.read(&mut chunk).await {
            Ok(size) => size,
            Err(_) => {
                let _sent = sender.send(Err("stdout_read_failed")).await;
                return;
            }
        };
        if size == 0 {
            if !pending.is_empty() {
                let _sent = sender.send(Ok(pending)).await;
            }
            return;
        }
        for segment in chunk[..size].split_inclusive(|byte| *byte == b'\n') {
            let terminated = segment.last() == Some(&b'\n');
            let content = &segment[..segment.len() - usize::from(terminated)];
            if pending.len().saturating_add(content.len()) > MAX_LINE_BYTES {
                let _sent = sender.send(Err("stdout_line_byte_budget_exceeded")).await;
                return;
            }
            pending.extend_from_slice(content);
            if terminated {
                if pending.last() == Some(&b'\r') {
                    pending.pop();
                }
                if sender.send(Ok(std::mem::take(&mut pending))).await.is_err() {
                    return;
                }
            }
        }
    }
}

pub(super) async fn read_stderr<R>(mut reader: R) -> std::io::Result<StderrCapture>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    let mut permission_denied = false;
    loop {
        let size = reader.read(&mut chunk).await?;
        if size == 0 {
            break;
        }
        let new_bytes = bytes.len().saturating_sub(128);
        bytes.extend_from_slice(&chunk[..size]);
        let lower = String::from_utf8_lossy(&bytes[new_bytes..]).to_ascii_lowercase();
        if bytes.len() > STDERR_TAIL_BYTES {
            bytes.drain(..bytes.len() - STDERR_TAIL_BYTES);
        }
        permission_denied |= lower.contains("permission denied")
            || lower.contains("soft-denied")
            || lower.contains("tool permission") && lower.contains("denied");
    }
    Ok(StderrCapture {
        bytes,
        permission_denied,
    })
}

pub(super) fn read_result_file(path: &std::path::Path) -> crate::Result<Option<String>> {
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err("CLI result is not a regular file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_RESULT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RESULT_BYTES {
        return Err("CLI result byte budget exceeded".into());
    }
    Ok(Some(String::from_utf8(bytes)?))
}

/// Bound a single owned task. Independent handles may already be completed when
/// another hits its deadline, so never repoll a partially completed join.
pub(super) async fn finish_task<T>(mut task: tokio::task::JoinHandle<T>) -> Option<T> {
    match tokio::time::timeout(std::time::Duration::from_secs(2), &mut task).await {
        Ok(result) => result.ok(),
        Err(_) => {
            task.abort();
            let _joined = task.await;
            None
        }
    }
}

pub(super) async fn probe(binary: &std::path::Path, args: &[&str]) -> crate::Result<String> {
    use std::process::Stdio;
    use tokio::process::Command;
    let mut command = Command::new(binary);
    command
        .env("PATH", super::enriched_command_path())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let group = child.id();
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _cleanup = super::terminate_process_group(&mut child, group).await;
        return Err("CLI probe pipes unavailable".into());
    };
    let (sender, mut lines) = mpsc::channel(2);
    let reader = tokio::spawn(read_lines(stdout, sender));
    let errors = tokio::spawn(read_stderr(stderr));
    let mut output = String::new();
    let mut status = None;
    let mut problem = None;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        tokio::select! {
            ()=tokio::time::sleep_until(deadline)=>{problem=Some("CLI probe timed out");break;}
            value=child.wait(),if status.is_none()=>{status=value.ok();let _cleanup=super::terminate_process_group(&mut child,group).await;}
            line=lines.recv()=>match line {
                None=>break,
                Some(Err(_))=>{problem=Some("CLI probe output invalid");break;}
                Some(Ok(line))=>{
                    if output.len().saturating_add(line.len()).saturating_add(1)>MAX_RESULT_BYTES {problem=Some("CLI probe output budget exceeded");break;}
                    output.push_str(&String::from_utf8_lossy(&line));output.push('\n');
                }
            }
        }
    }
    if status.is_none() && problem.is_none() {
        status = tokio::time::timeout_at(deadline, child.wait())
            .await
            .ok()
            .and_then(Result::ok);
    }
    let _cleanup = super::terminate_process_group(&mut child, group).await;
    drop(lines);
    let (stdout_done, stderr_done) = tokio::join!(finish_task(reader), finish_task(errors));
    if stdout_done.is_none() || !matches!(stderr_done, Some(Ok(_))) {
        return Err("CLI probe pipe closure unconfirmed".into());
    }
    if let Some(problem) = problem {
        return Err(problem.into());
    }
    if !status.is_some_and(|status| status.success()) {
        return Err("CLI probe did not complete successfully".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn complete_or_partial_long_lines_fail_before_unbounded_buffering() {
        for newline in [false, true] {
            let mut bytes = vec![b'x'; MAX_LINE_BYTES + 1];
            if newline {
                bytes.push(b'\n');
            }
            let (sender, mut receiver) = mpsc::channel(2);
            read_lines(std::io::Cursor::new(bytes), sender).await;
            assert_eq!(
                receiver.recv().await,
                Some(Err("stdout_line_byte_budget_exceeded"))
            );
        }
    }
    #[tokio::test]
    async fn stderr_retains_a_fixed_tail_and_denial_observation() -> crate::Result<()> {
        let mut bytes = b"permission denied\n".to_vec();
        bytes.extend(vec![b'x'; STDERR_TAIL_BYTES * 3]);
        let result = read_stderr(std::io::Cursor::new(bytes)).await?;
        assert_eq!(result.bytes.len(), STDERR_TAIL_BYTES);
        assert!(result.permission_denied);
        Ok(())
    }
}
