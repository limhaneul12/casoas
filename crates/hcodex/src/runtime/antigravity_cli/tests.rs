use super::*;
use crate::routing::{Provider, ReceiptStatus};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

fn request() -> RouteRequest {
    RouteRequest {
        provider: Provider::Antigravity,
        engine: Engine::AntigravityCli,
        model: "gemini-4-argon-high".into(),
        effort: "high".into(),
        prompt: "payload".into(),
    }
}

fn fixture(dir: &Path, tools: &str, tail: &str) -> crate::Result<PathBuf> {
    let binary = dir.join("agy-fixture");
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
printf '%s\n' '{{"event":"init","conversation_id":"test-session","init":{{"cwd":"'"$PWD"'","permission_mode":"strict","model":"gemini-4-argon-high","tools":{tools}}}}}'
IFS= read -r input || exit 1
printf '%s\n' "$input" > "${{0%/*}}/input"
{tail}
"#
        ),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    Ok(binary)
}

#[tokio::test]
async fn durable_init_precedes_input_and_final_usage_is_counted_once() -> crate::Result<()> {
    let dir = tempfile::tempdir()?;
    let binary = fixture(
        dir.path(),
        "[]",
        r#"
test -f "${0%/*}/ack" || exit 2
printf '%s\n' '{"event":"step_update","step_update":{"conversation_id":"test-session","step_index":0,"step_type":"agent_response","state":"DONE","usage":{"input_tokens":2,"output_tokens":3}}}'
printf '%s\n' '{"event":"result","result":{"conversation_id":"test-session","status":"SUCCESS","response":"artifact","num_turns":1,"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12}}}'
IFS= read -r unexpected && exit 3
exit 0
"#,
    )?;
    let ack = dir.path().join("ack");
    let phases = Arc::new(Mutex::new(Vec::new()));
    let collected = Arc::clone(&phases);
    let options = ExecutionOptions {
        progress: Some(Arc::new(move |progress| {
            let ack = ack.clone();
            let collected = Arc::clone(&collected);
            Box::pin(async move {
                if progress.phase == ExecutionPhase::AwaitingInput {
                    tokio::fs::write(ack, b"committed").await?;
                }
                collected
                    .lock()
                    .map_err(|_| "lock poisoned")?
                    .push(progress);
                Ok(())
            })
        })),
        ..ExecutionOptions::default()
    };
    let result = execute_antigravity_with_binary_and_options(
        request(),
        Duration::ZERO,
        &binary,
        &options,
        None,
    )
    .await?;
    assert_eq!(
        result.receipt.status,
        ReceiptStatus::Completed,
        "{:?}",
        result.receipt
    );
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    assert_eq!(
        (result.receipt.tokens_in, result.receipt.tokens_out),
        (5, 7)
    );
    assert_eq!(result.session_id.as_deref(), Some("test-session"));
    let phases = phases.lock().map_err(|_| "lock poisoned")?;
    assert_eq!(
        phases.iter().map(|p| p.phase).collect::<Vec<_>>(),
        [
            ExecutionPhase::AwaitingInput,
            ExecutionPhase::InputSent,
            ExecutionPhase::Running,
            ExecutionPhase::ResultObserved
        ]
    );
    assert!(
        phases
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    Ok(())
}

#[tokio::test]
async fn failed_checkpoint_or_untrusted_tools_send_no_input() -> crate::Result<()> {
    for (tools, reject_commit) in [("[]", true), ("[\"write_file\"]", false)] {
        let dir = tempfile::tempdir()?;
        let binary = fixture(dir.path(), tools, "exit 0")?;
        let options = ExecutionOptions {
            progress: Some(Arc::new(move |_| {
                Box::pin(async move {
                    if reject_commit {
                        return Err("checkpoint rejected".into());
                    }
                    Ok(())
                })
            })),
            ..ExecutionOptions::default()
        };
        let result = execute_antigravity_with_binary_and_options(
            request(),
            Duration::ZERO,
            &binary,
            &options,
            None,
        )
        .await?;
        assert_eq!(result.receipt.status, ReceiptStatus::Failed);
        assert_eq!(result.receipt.owned_processes_closed, Some(true));
        assert_eq!(result.session_id.as_deref(), Some("test-session"));
        assert!(!dir.path().join("input").exists());
    }
    Ok(())
}

#[tokio::test]
async fn dispatched_process_with_missing_or_foreign_result_stays_unknown() -> crate::Result<()> {
    for tail in [
        "exit 0",
        r#"printf '%s\n' '{"event":"result","result":{"conversation_id":"foreign","status":"SUCCESS"}}'"#,
    ] {
        let dir = tempfile::tempdir()?;
        let binary = fixture(dir.path(), "[]", tail)?;
        let result = execute_antigravity_with_binary_and_options(
            request(),
            Duration::ZERO,
            &binary,
            &ExecutionOptions::default(),
            None,
        )
        .await?;
        assert_eq!(result.receipt.status, ReceiptStatus::TimedOutUnknown);
        assert_eq!(result.receipt.owned_processes_closed, Some(true));
        assert_eq!(result.session_id.as_deref(), Some("test-session"));
        assert!(dir.path().join("input").exists());
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_during_init_checkpoint_keeps_session_without_sending_input()
-> crate::Result<()> {
    let dir = tempfile::tempdir()?;
    let binary = fixture(dir.path(), "[]", "exit 0")?;
    let entered = Arc::new(tokio::sync::Notify::new());
    let started = Arc::clone(&entered);
    let options = ExecutionOptions {
        progress: Some(Arc::new(move |_| {
            let started = Arc::clone(&started);
            Box::pin(async move {
                started.notify_one();
                std::future::pending().await
            })
        })),
        ..ExecutionOptions::default()
    };
    let supervisor = crate::supervisor::Supervisor::new(1);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    supervisor
        .spawn("cancel-init".to_owned(), async move {
            let result = execute_antigravity_with_binary_and_options(
                request(),
                Duration::ZERO,
                &binary,
                &options,
                None,
            )
            .await;
            let _sent = sender.send(result);
        })
        .await?;
    tokio::time::timeout(Duration::from_secs(30), entered.notified()).await?;
    assert!(supervisor.cancel("cancel-init").await);
    let result = tokio::time::timeout(Duration::from_secs(10), receiver).await???;
    supervisor.shutdown(Duration::from_secs(5)).await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Cancelled);
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    assert_eq!(result.session_id.as_deref(), Some("test-session"));
    assert!(!dir.path().join("input").exists());
    Ok(())
}

#[test]
fn verified_inventory_admits_init_but_canonical_tool_errors_reject_the_artifact()
-> crate::Result<()> {
    let workspace = Path::new("/fixture");
    let init = serde_json::to_vec(&json!({
        "event":"init", "conversation_id":"test-session",
        "init":{"cwd":workspace,"permission_mode":"strict","model":"gemini-4-argon-high","tools":["write_to_file","run_command"]}
    }))?;
    let mut state = AgyState::default();
    assert_eq!(
        state.observe(&init, workspace, true),
        Ok(ExecutionPhase::AwaitingInput)
    );
    let denial = serde_json::to_vec(&json!({
        "event":"step_update","step_update":{
            "conversation_id":"test-session","step_index":2,"state":"ERROR","step_type":"tool_call",
            "tool_name":"write_file","tool_info":{"error":{"type":"TOOL_ERROR","message":"permission denied"}}
        }
    }))?;
    assert_eq!(
        state.observe(&denial, workspace, false),
        Err("cli_forbidden_tool_or_subagent")
    );
    assert!(state.result.is_none());
    Ok(())
}

#[tokio::test]
async fn structured_output_uses_a_bounded_prompt_and_no_native_finalizer() -> crate::Result<()> {
    let dir = tempfile::tempdir()?;
    let binary = fixture(
        dir.path(),
        "[]",
        r#"
for argument in "$@"; do
  [ "$argument" != "--json-schema" ] || exit 2
done
printf '%s\n' '{"event":"result","result":{"conversation_id":"test-session","status":"SUCCESS","response":"{\"ok\":true}","num_turns":1}}'
IFS= read -r unexpected && exit 3
exit 0
"#,
    )?;
    let schema = json!({
        "type":"object","properties":{"ok":{"type":"boolean"}},
        "required":["ok"],"additionalProperties":false
    });
    let restricted = ExecutionOptions {
        max_input_bytes: request().prompt.len(),
        ..ExecutionOptions::default()
    };
    let over_budget = execute_antigravity_with_binary_and_options(
        request(),
        Duration::ZERO,
        &binary,
        &restricted,
        Some(&schema),
    )
    .await;
    assert!(over_budget.is_err());
    assert!(!dir.path().join("input").exists());
    let result = execute_antigravity_with_binary_and_options(
        request(),
        Duration::ZERO,
        &binary,
        &ExecutionOptions::default(),
        Some(&schema),
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Completed);
    assert_eq!(result.structured_output, Some(json!({"ok":true})));
    let line = std::fs::read(dir.path().join("input"))?;
    let event: Value = serde_json::from_slice(&line)?;
    let prompt = event
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or("missing prompt")?;
    assert!(prompt.starts_with("payload\n\n"));
    assert!(prompt.ends_with(&serde_json::to_string(&schema)?));
    Ok(())
}
