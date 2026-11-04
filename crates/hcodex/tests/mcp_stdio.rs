//! Real binary MCP framing and durable local-tool smoke tests.
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout, Command};

async fn request(
    stdin: &mut ChildStdin,
    lines: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
    method: &str,
    params: Value,
) -> hcodex::Result<Value> {
    let wire = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    stdin.write_all(format!("{wire}\n").as_bytes()).await?;
    stdin.flush().await?;
    let line = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
        .await??
        .ok_or("server closed stdout")?;
    let response: Value = serde_json::from_str(&line)?;
    assert_eq!(response["id"], id);
    assert!(response.get("error").is_none(), "{response}");
    Ok(response)
}
fn payload(response: &Value) -> hcodex::Result<Value> {
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = response["result"]["structuredContent"]["result"]
        .as_str()
        .ok_or("missing structured result")?;
    assert_eq!(response["result"]["content"][0]["text"], text);
    Ok(serde_json::from_str(text)?)
}
#[tokio::test]
async fn real_stdio_catalog_handoff_and_job_contracts() -> hcodex::Result<()> {
    let root = tempfile::tempdir()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_hcodex"))
        .args([
            "--state-root",
            root.path().join("state").to_str().ok_or("state path")?,
            "mcp",
        ])
        .env("HCODEX_HOME", root.path().join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("stdout")?).lines();
    let init=request(&mut stdin,&mut lines,1,"initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"rust-parity-fixture","version":"1"}})).await?;
    assert_eq!(init["result"]["serverInfo"]["name"], "codex-collab-mcp");
    let tools = request(&mut stdin, &mut lines, 2, "tools/list", json!({})).await?;
    assert_eq!(tools["result"]["tools"].as_array().map(Vec::len), Some(16));
    let created=payload(&request(&mut stdin,&mut lines,3,"tools/call",json!({"name":"heterarchy_handoff_create","arguments":{"title":"Rust parity","spec_or_payload":"read-only fixture","workspace_dir":root.path()}})).await?)?;
    let ticket = created["ticket"]["ticket_id"].as_str().ok_or("ticket id")?;
    assert!(ticket.starts_with("hnd-"));
    assert_eq!(created["execution_completed"], false);
    let inbox=payload(&request(&mut stdin,&mut lines,4,"tools/call",json!({"name":"heterarchy_handoff_inbox","arguments":{"status":"","workspace_dir":root.path()}})).await?)?;
    assert_eq!(inbox["tickets"].as_array().map(Vec::len), Some(1));
    let jobs = payload(
        &request(
            &mut stdin,
            &mut lines,
            5,
            "tools/call",
            json!({"name":"codex_job_list","arguments":{}}),
        )
        .await?,
    )?;
    assert_eq!(jobs["jobs"], json!([]));
    let invalid = request(
        &mut stdin,
        &mut lines,
        6,
        "tools/call",
        json!({"name":"codex_job_list","arguments":{"unexpected":true}}),
    )
    .await?;
    assert_eq!(invalid["result"]["isError"], true);
    drop(stdin);
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_stream_still_reaps_background_provider() -> hcodex::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir()?;
    let fixture = root.path().join("slow-codex");
    let pid_file = root.path().join("provider-pid");
    std::fs::write(
        &fixture,
        format!(
            "#!/bin/sh\necho $$ > '{}'\ncat >/dev/null\nsleep 60\n",
            pid_file.display()
        ),
    )?;
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700))?;
    let mut server = Command::new(env!("CARGO_BIN_EXE_hcodex"))
        .args([
            "--state-root",
            root.path().join("state").to_str().ok_or("state path")?,
            "mcp",
        ])
        .env("HCODEX_HOME", root.path().join("home"))
        .env("CODEX_CLI_BIN", &fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = server.stdin.take().ok_or("stdin")?;
    let mut lines = BufReader::new(server.stdout.take().ok_or("stdout")?).lines();
    let accepted=payload(&request(&mut stdin,&mut lines,1,"tools/call",json!({"name":"codex_adversarial_audit","arguments":{"target_code_or_diff":"fn f(){}","model":"gpt-6.1-sol","workspace_dir":root.path(),"async_background":true,"request_id":"cdx-broken-stream"}})).await?)?;
    assert_eq!(accepted["async_dispatched"], true);
    let pid = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let Ok(pid) = tokio::fs::read_to_string(&pid_file).await
                && !pid.trim().is_empty()
            {
                break pid.trim().to_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    let oversized = vec![b'x'; 2_000_001];
    let _sent = stdin.write_all(&oversized).await;
    drop(stdin);
    let exit = tokio::time::timeout(std::time::Duration::from_secs(15), server.wait()).await??;
    assert!(!exit.success());
    let alive = Command::new("/bin/kill")
        .args(["-0", &pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?
        .success();
    assert!(!alive, "MCP framing failure leaked owned provider process");
    Ok(())
}
