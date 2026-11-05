use super::*;
use crate::routing::Provider;
use serde_json::json;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn request() -> RouteRequest {
    RouteRequest {
        provider: Provider::OpenaiCodex,
        engine: Engine::CodexCli,
        model: "gpt-6.1-sol".to_owned(),
        effort: "high".to_owned(),
        prompt: "find primary sources".to_owned(),
    }
}

fn schema() -> Value {
    json!({
        "type":"object",
        "additionalProperties":false,
        "properties":{"candidate_urls":{"type":"array","items":{"type":"string"}}},
        "required":["candidate_urls"]
    })
}

#[cfg(unix)]
fn fake_binary(directory: &Path, body: &str) -> crate::Result<PathBuf> {
    let path = directory.join("fake-codex-search");
    std::fs::write(&path, format!("#!/bin/sh\n{body}"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

#[tokio::test]
async fn newline_terminated_jsonl_line_is_bounded_before_parsing() -> crate::Result<()> {
    let (mut writer, reader) = tokio::io::duplex(MAX_JSONL_LINE_BYTES + 2);
    let producer = tokio::spawn(async move {
        writer
            .write_all(&vec![b'x'; MAX_JSONL_LINE_BYTES + 1])
            .await?;
        writer.write_all(b"\n").await?;
        writer.shutdown().await
    });
    let (violations, mut receiver) = mpsc::channel(1);
    let capture = read_stdout(
        reader,
        Arc::new(AtomicUsize::new(0)),
        violations,
        SearchPolicy::default(),
    )
    .await?;
    producer.await??;
    assert_eq!(
        capture.violation.as_deref(),
        Some("jsonl_line_limit_exceeded")
    );
    assert_eq!(
        receiver.recv().await.as_deref(),
        Some("jsonl_line_limit_exceeded")
    );
    Ok(())
}

#[tokio::test]
async fn spawn_failure_returns_a_confirmed_receipt_without_an_owned_child() -> crate::Result<()> {
    let fixture = tempfile::tempdir()?;
    let result = execute_codex_web_discovery_with_binary(
        request(),
        Duration::ZERO,
        &ExecutionOptions::default(),
        &schema(),
        SearchPolicy::default(),
        &fixture.path().join("missing-codex"),
        None,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Failed);
    assert_eq!(result.receipt.outcome_state, "confirmed_failure");
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    assert_eq!(
        result
            .search_evidence
            .and_then(|evidence| evidence.violation),
        Some("process_spawn_failed".to_owned())
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn accepts_only_candidate_urls_proven_by_completed_search_results() -> crate::Result<()> {
    let fixture = tempfile::tempdir()?;
    let argv = fixture.path().join("argv");
    let binary = fake_binary(
        fixture.path(),
        &format!(
            r#"printf '%s\n' "$@" > '{}'
out=''
prior=''
for arg in "$@"; do
  if [ "$prior" = '-o' ]; then out="$arg"; fi
  prior="$arg"
done
cat >/dev/null
printf '%s\n' '{{"type":"thread.started","thread_id":"thr_search","model":"gpt-6.1-sol"}}'
printf '%s\n' '{{"type":"item.started","item":{{"id":"search-1","type":"web_search","action":{{"type":"other"}}}}}}'
printf '%s\n' '{{"type":"item.completed","item":{{"id":"search-1","type":"web_search","action":{{"type":"search","query":"discarded"}},"results":[{{"url":"https://example.com/primary#section"}},{{"url":"https://example.com/private?x-amz-signature=secret"}}]}}}}'
printf '%s\n' '{{"type":"item.completed","item":{{"type":"agent_message","text":"done"}}}}'
printf '%s' '{{"candidate_urls":["https://example.com/primary#other","https://forged.example/not-observed","http://example.com/plain","https://example.com/private?x-amz-signature=secret"]}}' > "$out"
"#,
            argv.display()
        ),
    )?;
    let result = execute_codex_web_discovery_with_binary(
        request(),
        Duration::ZERO,
        &ExecutionOptions::default(),
        &schema(),
        SearchPolicy::default(),
        &binary,
        None,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Completed);
    assert_eq!(
        result.structured_output,
        Some(json!({"candidate_urls":["https://example.com/primary"]}))
    );
    let evidence = result.search_evidence.ok_or("missing search evidence")?;
    assert_eq!(evidence.observed_search_ids, ["search-1"]);
    assert_eq!(evidence.completed_search_ids, ["search-1"]);
    assert_eq!(evidence.observed_searches, 1);
    assert_eq!(evidence.completed_searches, 1);
    assert_eq!(evidence.rejected_candidate_count, 3);
    assert_eq!(evidence.stop_reason, SearchStopReason::Completed);
    assert!(evidence.violation.is_none());
    assert!(evidence.raw_output_sha256.is_some());
    assert_eq!(evidence.matching_event_sha256.len(), 2);
    let args = std::fs::read_to_string(argv)?;
    for expected in [
        "--search",
        "--ignore-user-config",
        "mcp_servers={}",
        "features.shell_tool=false",
        "features.unified_exec=false",
        "features.multi_agent=false",
        "read-only",
    ] {
        assert!(
            args.lines().any(|arg| arg == expected),
            "missing {expected}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_success_without_completed_web_search_evidence() -> crate::Result<()> {
    let fixture = tempfile::tempdir()?;
    let binary = fake_binary(
        fixture.path(),
        r#"out=''
prior=''
for arg in "$@"; do
  if [ "$prior" = '-o' ]; then out="$arg"; fi
  prior="$arg"
done
cat >/dev/null
printf '%s\n' '{"type":"thread.started","thread_id":"thr_empty","model":"gpt-6.1-sol"}'
printf '%s' '{"candidate_urls":["https://forged.example/"]}' > "$out"
"#,
    )?;
    let result = execute_codex_web_discovery_with_binary(
        request(),
        Duration::ZERO,
        &ExecutionOptions::default(),
        &schema(),
        SearchPolicy::default(),
        &binary,
        None,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Failed);
    let evidence = result.search_evidence.ok_or("missing search evidence")?;
    assert_eq!(evidence.stop_reason, SearchStopReason::PolicyViolation);
    assert_eq!(
        evidence.violation.as_deref(),
        Some("completed_search_evidence_missing")
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn event_budget_and_forbidden_items_fail_closed() -> crate::Result<()> {
    for (name, events, violation) in [
        (
            "budget",
            concat!(
                "printf '%s\\n' '{\"type\":\"item.started\",\"item\":{\"id\":\"one\",\"type\":\"web_search\",\"action\":{\"type\":\"other\"}}}'\n",
                "printf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"id\":\"one\",\"type\":\"web_search\",\"action\":{\"type\":\"search\"},\"results\":[]}}'\n",
                "printf '%s\\n' '{\"type\":\"item.started\",\"item\":{\"id\":\"two\",\"type\":\"web_search\",\"action\":{\"type\":\"other\"}}}'\n"
            ),
            "search_event_budget_exhausted",
        ),
        (
            "forbidden",
            "printf '%s\\n' '{\"type\":\"item.started\",\"item\":{\"id\":\"cmd\",\"type\":\"command_execution\"}}'\n",
            "forbidden_item_type",
        ),
    ] {
        let fixture = tempfile::tempdir()?;
        let binary = fake_binary(
            fixture.path(),
            &format!(
                "cat >/dev/null\nprintf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"thr_{name}\",\"model\":\"gpt-6.1-sol\"}}'\n{events}"
            ),
        )?;
        let result = execute_codex_web_discovery_with_binary(
            request(),
            Duration::ZERO,
            &ExecutionOptions::default(),
            &schema(),
            SearchPolicy {
                max_search_events: 1,
                max_candidates: 2,
            },
            &binary,
            None,
        )
        .await?;
        assert_eq!(result.receipt.status, ReceiptStatus::Failed);
        assert_eq!(result.receipt.owned_processes_closed, Some(true));
        assert_eq!(
            result
                .search_evidence
                .and_then(|evidence| evidence.violation),
            Some(violation.to_owned())
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_kills_owned_search_process_group_and_keeps_evidence() -> crate::Result<()> {
    let fixture = tempfile::tempdir()?;
    let pid_file = fixture.path().join("pid");
    let marker = fixture.path().join("ready.marker");
    let binary = fake_binary(
        fixture.path(),
        &format!(
            "printf '%s' $$ > '{}'\ncat >/dev/null\nprintf '%s\\n' '{{\"type\":\"item.started\",\"item\":{{\"id\":\"slow\",\"type\":\"web_search\",\"action\":{{\"type\":\"other\"}}}}}}'\nprintf R > '{}'\nsleep 30\n",
            pid_file.display(),
            marker.display()
        ),
    )?;
    let result = execute_codex_web_discovery_with_binary(
        request(),
        Duration::from_millis(100),
        &ExecutionOptions::default(),
        &schema(),
        SearchPolicy::default(),
        &binary,
        Some(&marker),
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::TimedOutUnknown);
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    assert_eq!(
        result
            .search_evidence
            .as_ref()
            .map(|evidence| evidence.stop_reason),
        Some(SearchStopReason::TimedOut)
    );
    let pid = std::fs::read_to_string(pid_file)?;
    let alive = Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?
        .success();
    assert!(!alive, "owned search process remained alive after timeout");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn completed_parent_closes_descendant_holding_output_pipe() -> crate::Result<()> {
    let fixture = tempfile::tempdir()?;
    let binary = fake_binary(
        fixture.path(),
        r#"out=''
prior=''
for arg in "$@"; do
  if [ "$prior" = '-o' ]; then out="$arg"; fi
  prior="$arg"
done
cat >/dev/null
sleep 30 &
printf '%s\n' '{"type":"thread.started","thread_id":"thr_descendant","model":"gpt-6.1-sol"}'
printf '%s\n' '{"type":"item.started","item":{"id":"descendant-search","type":"web_search","action":{"type":"other"}}}'
printf '%s\n' '{"type":"item.completed","item":{"id":"descendant-search","type":"web_search","action":{"type":"search"},"results":[{"url":"https://example.com/source"}]}}'
printf '%s' '{"candidate_urls":["https://example.com/source"]}' > "$out"
"#,
    )?;
    let result = execute_codex_web_discovery_with_binary(
        request(),
        Duration::ZERO,
        &ExecutionOptions::default(),
        &schema(),
        SearchPolicy::default(),
        &binary,
        None,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Completed);
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_oversized_final_output_without_reading_it_unbounded() -> crate::Result<()> {
    let fixture = tempfile::tempdir()?;
    let binary = fake_binary(
        fixture.path(),
        r#"out=''
prior=''
for arg in "$@"; do
  if [ "$prior" = '-o' ]; then out="$arg"; fi
  prior="$arg"
done
cat >/dev/null
printf '%s\n' '{"type":"thread.started","thread_id":"thr_large","model":"gpt-6.1-sol"}'
printf '%s\n' '{"type":"item.started","item":{"id":"large-search","type":"web_search","action":{"type":"other"}}}'
printf '%s\n' '{"type":"item.completed","item":{"id":"large-search","type":"web_search","action":{"type":"search"},"results":[{"url":"https://example.com/source"}]}}'
dd if=/dev/zero bs=1048576 count=3 2>/dev/null | tr '\000' x > "$out"
"#,
    )?;
    let result = execute_codex_web_discovery_with_binary(
        request(),
        Duration::ZERO,
        &ExecutionOptions::default(),
        &schema(),
        SearchPolicy::default(),
        &binary,
        None,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Failed);
    assert_eq!(
        result
            .search_evidence
            .and_then(|evidence| evidence.violation),
        Some("output_file_limit_exceeded".to_owned())
    );
    Ok(())
}
