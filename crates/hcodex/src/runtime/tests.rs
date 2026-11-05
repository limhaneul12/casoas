use super::*;

fn codex_request(workspace: &Path) -> RouteRequest {
    let _ = workspace;
    RouteRequest {
        provider: crate::routing::Provider::OpenaiCodex,
        engine: Engine::CodexCli,
        model: "gpt-6.1-sol".to_owned(),
        effort: "xhigh".to_owned(),
        prompt: "review".to_owned(),
    }
}
#[test]
fn extracts_first_complete_json_object_from_prose_and_concatenated_output() {
    assert_eq!(
        extract_json(
            "Analysis: {not JSON} then {\"verdict\":\"PASS\",\"nested\":{\"brace\":\"}\"}} {\"later\":true}"
        ),
        Some(serde_json::json!({"verdict":"PASS","nested":{"brace":"}"}}))
    );
    assert_eq!(
        extract_json("설명\n```json\n{\"ok\":true}\n```\nDone"),
        Some(serde_json::json!({"ok":true}))
    );
    assert!(extract_json(&"{".repeat(4 * 1024 * 1024 + 1)).is_none());
}
#[test]
fn parses_codex_identity_messages_and_usage() {
    let stream = concat!(
        "{\"type\":\"thread.started\",\"thread_id\":\"thr_1\",\"model\":\"gpt-6.1-sol\"}\n",
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"verdict\\\":\\\"PASS\\\"}\"}}\n",
        "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":37,\"output_tokens\":11}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":48},\"total_token_usage\":{\"total_tokens\":9999},\"model_context_window\":1000}}}\n"
    );
    let parsed = parse_codex_jsonl(stream);
    assert_eq!(parsed.session_id.as_deref(), Some("thr_1"));
    assert_eq!(parsed.actual_model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!((parsed.tokens_in, parsed.tokens_out), (37, 11));
    assert_eq!(parsed.context_window, Some(1000));
    assert_eq!(parsed.current_context_tokens, Some(48));
    assert_eq!(parsed.messages, ["{\"verdict\":\"PASS\"}"]);
}

fn write_session_fixture(
    sessions: &Path,
    session_id: &str,
    workspace: &Path,
    model: &str,
    output: &str,
) -> crate::Result<()> {
    let directory = sessions.join("2026/11/03");
    std::fs::create_dir_all(&directory)?;
    let events = [
        json!({"type":"session_meta","payload":{"id":session_id,"cwd":workspace,"source":"exec"}}),
        json!({"type":"turn_context","payload":{"cwd":workspace,"model":model}}),
        json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"cwd":workspace,"model":model}}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":output}]}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":48},"total_token_usage":{"total_tokens":9999},"model_context_window":1000}}}),
    ];
    let body = events
        .into_iter()
        .map(|event| serde_json::to_string(&event))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    std::fs::write(
        directory.join(format!("rollout-fixture-{session_id}.jsonl")),
        body,
    )?;
    Ok(())
}

#[test]
fn codex_session_identity_requires_session_cwd_settings_and_output_digest() -> crate::Result<()> {
    let temporary = tempfile::tempdir()?;
    let workspace = temporary.path().join("workspace");
    let sessions = temporary.path().join("sessions");
    std::fs::create_dir(&workspace)?;
    let session_id = "01a19cc4-bc08-7702-b0fb-9ee97cb8256e";
    let output = "{\"verdict\":\"PASS\"}";
    write_session_fixture(&sessions, session_id, &workspace, "gpt-6.1-sol", output)?;
    let proof =
        prove_codex_session_identity_at(&sessions, session_id, &workspace, &digest_output(output))
            .ok_or("expected verified identity")?;
    assert_eq!(proof.model, "gpt-6.1-sol");
    assert_eq!(proof.source, "codex_turn_context_verified");
    assert_eq!(proof.context_window, Some(1000));
    assert_eq!(proof.current_context_tokens, Some(48));
    assert!(
        prove_codex_session_identity_at(
            &sessions,
            "wrong-session",
            &workspace,
            &digest_output(output),
        )
        .is_none()
    );
    assert!(
        prove_codex_session_identity_at(
            &sessions,
            session_id,
            &workspace,
            &digest_output("stale-output"),
        )
        .is_none()
    );
    let wrong_workspace = temporary.path().join("wrong-workspace");
    std::fs::create_dir(&wrong_workspace)?;
    assert!(
        prove_codex_session_identity_at(
            &sessions,
            session_id,
            &wrong_workspace,
            &digest_output(output),
        )
        .is_none()
    );
    Ok(())
}
#[cfg(unix)]
#[tokio::test]
async fn native_review_targets_capture_real_argv_and_receipt_identity() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempfile::tempdir()?;
    let binary = workspace.path().join("fake-review");
    std::fs::write(
        &binary,
        concat!(
            "#!/bin/sh\n",
            "printf '%s\\n' \"$@\" > argv\n",
            "pwd -P > review-cwd\n",
            "cat > review-stdin\n",
            "printf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"thr_review\",\"model\":\"gpt-6.1-sol\"}'\n",
            "printf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"verdict\\\":\\\"PASS\\\",\\\"summary\\\":\\\"read-only native review\\\"}\"}}'\n",
            "printf '%s\\n' '{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":7,\"output_tokens\":2}}'\n",
        ),
    )?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;
    let options = ExecutionOptions {
        workspace_dir: Some(workspace.path().to_path_buf()),
        sandbox_mode: SandboxMode::ReadOnly,
        ..ExecutionOptions::default()
    };
    for (target, expected) in [
        (
            ReviewTarget {
                uncommitted: true,
                ..ReviewTarget::default()
            },
            vec!["--uncommitted"],
        ),
        (
            ReviewTarget {
                uncommitted: true,
                base_branch: Some(" main ".to_owned()),
                commit_sha: None,
            },
            vec!["--base", "main"],
        ),
        (
            ReviewTarget {
                uncommitted: true,
                base_branch: Some("ignored".to_owned()),
                commit_sha: Some(" abc123 ".to_owned()),
            },
            vec!["--commit", "abc123"],
        ),
        (
            ReviewTarget {
                uncommitted: true,
                base_branch: Some("main".to_owned()),
                commit_sha: Some(" ".to_owned()),
            },
            vec!["--base", "main"],
        ),
        (ReviewTarget::default(), vec![]),
    ] {
        let request = codex_request(workspace.path());
        let prompt = request.prompt.clone();
        let result =
            codex_git_review_with_binary(request, target, Duration::ZERO, &options, &binary)
                .await?;
        assert_eq!(result.receipt.status, ReceiptStatus::Completed);
        assert_eq!(result.receipt.actual_model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(result.session_id.as_deref(), Some("thr_review"));
        assert_eq!(
            (result.receipt.tokens_in, result.receipt.tokens_out),
            (7, 2)
        );
        assert_eq!(
            result.structured_output.as_ref().map(|v| &v["verdict"]),
            Some(&json!("PASS"))
        );
        let argv = std::fs::read_to_string(workspace.path().join("argv"))?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(&argv[..2], ["exec", "review"]);
        assert!(argv.iter().any(|arg| arg == "--json"));
        assert!(argv.iter().any(|arg| arg == "--ignore-user-config"));
        assert!(argv.windows(2).any(|pair| pair == ["-m", "gpt-6.1-sol"]));
        assert!(
            expected.is_empty()
                || argv.windows(expected.len()).any(|window| {
                    window
                        .iter()
                        .map(String::as_str)
                        .eq(expected.iter().copied())
                })
        );
        assert_eq!(
            argv.iter()
                .filter(|arg| matches!(arg.as_str(), "--uncommitted" | "--base" | "--commit"))
                .count(),
            usize::from(!expected.is_empty())
        );
        for expected in [
            "mcp_servers={}",
            "model_reasoning_effort=\"xhigh\"",
            "sandbox_mode=\"read-only\"",
        ] {
            assert!(
                argv.iter().any(|arg| arg == expected),
                "missing review argument: {expected}"
            );
        }
        if expected.is_empty() {
            assert_eq!(argv.last().map(String::as_str), Some("-"));
            assert_eq!(
                std::fs::read_to_string(workspace.path().join("review-stdin"))?,
                prompt
            );
        } else {
            assert!(!argv.iter().any(|arg| arg == "-"));
            let supplied = argv
                .iter()
                .find_map(|arg| arg.strip_prefix("developer_instructions="))
                .ok_or("targeted review instructions missing")?;
            assert_eq!(serde_json::from_str::<String>(supplied)?, prompt);
            assert!(std::fs::read_to_string(workspace.path().join("review-stdin"))?.is_empty());
        }
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("review-cwd"))?.trim(),
            workspace.path().canonicalize()?.to_string_lossy()
        );
    }
    let bad = codex_git_review_with_binary(
        codex_request(workspace.path()),
        ReviewTarget {
            commit_sha: Some("--help".to_owned()),
            ..ReviewTarget::default()
        },
        Duration::ZERO,
        &options,
        &binary,
    )
    .await;
    assert!(bad.is_err());
    let prior = std::fs::read(workspace.path().join("argv"))?;
    let mut oversized = codex_request(workspace.path());
    oversized.prompt = "\"".repeat(60 * 1024);
    let rejected = codex_git_review_with_binary(
        oversized,
        ReviewTarget {
            uncommitted: true,
            ..ReviewTarget::default()
        },
        Duration::ZERO,
        &options,
        &binary,
    )
    .await;
    let error = rejected
        .err()
        .ok_or("oversized targeted review must be rejected")?;
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::InvalidInput)
    );
    assert_eq!(
        std::fs::read(workspace.path().join("argv"))?,
        prior,
        "oversized review must not start a child"
    );
    std::fs::write(
        &binary,
        "#!/bin/sh\ncat >/dev/null\nprintf 'plain review without identity'\n",
    )?;
    let unproven = codex_git_review_with_binary(
        codex_request(workspace.path()),
        ReviewTarget {
            uncommitted: true,
            ..ReviewTarget::default()
        },
        Duration::ZERO,
        &options,
        &binary,
    )
    .await?;
    assert_eq!(unproven.receipt.status, ReceiptStatus::Failed);
    assert!(unproven.receipt.actual_model.is_none());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn real_subprocess_jsonl_proves_identity_and_tokens() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempfile::tempdir()?;
    let binary = workspace.path().join("fake-codex");
    std::fs::write(
        &binary,
        concat!(
            "#!/bin/sh\n",
            "cat >/dev/null\n",
            "printf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"thr_test\",\"model\":\"gpt-6.1-sol\"}'\n",
            "printf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"verdict\\\":\\\"PASS\\\"}\"}}'\n",
            "printf '%s\\n' '{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":9,\"output_tokens\":4}}'\n",
        ),
    )?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;
    let options = ExecutionOptions {
        workspace_dir: Some(workspace.path().to_path_buf()),
        request_id: Some("cdx-runtime-test".to_owned()),
        idempotency_key: Some("runtime-test".to_owned()),
        max_input_bytes: 1024,
        ..ExecutionOptions::default()
    };
    let result = execute_codex_with_binary_and_options(
        codex_request(workspace.path()),
        Duration::ZERO,
        &binary,
        &options,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Completed);
    assert_eq!(result.receipt.actual_model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(
        (result.receipt.tokens_in, result.receipt.tokens_out),
        (9, 4)
    );
    assert_eq!(result.session_id.as_deref(), Some("thr_test"));
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn leader_exit_with_active_background_is_unknown_after_owned_cleanup() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    struct Cleanup(PathBuf);
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
    let workspace = tempfile::tempdir()?;
    let binary = workspace.path().join("background-codex");
    let _owned_background = Cleanup(workspace.path().join("background-pid"));
    std::fs::write(
        &binary,
        concat!(
            "#!/bin/sh\ncat >/dev/null\n",
            "sleep 30 </dev/null >/dev/null 2>&1 & printf '%s' \"$!\" > background-pid\n",
            "printf '%s\\n' '{\"model\":\"gpt-6.1-sol\",\"message\":\"{\\\"verdict\\\":\\\"PASS\\\"}\"}'\nexit 0\n"
        ),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let options = ExecutionOptions {
        workspace_dir: Some(workspace.path().to_path_buf()),
        max_input_bytes: 1024,
        ..ExecutionOptions::default()
    };
    let result = execute_codex_with_binary_and_options(
        codex_request(workspace.path()),
        Duration::ZERO,
        &binary,
        &options,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::TimedOutUnknown);
    assert_eq!(result.receipt.owned_processes_closed, Some(true));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn subprocess_auth_noise_is_redacted_and_isolated() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempfile::tempdir()?;
    let binary = workspace.path().join("auth-noise-codex");
    std::fs::write(
        &binary,
        concat!(
            "#!/bin/sh\n",
            "cat >/dev/null\n",
            "printf '%s\\n' 'codex_rmcp_client::oauth invalid_grant for server docs: Bearer downstream.secret' >&2\n",
            "printf '%s\\n' 'primary api_key=topsecret sk-1234567890abcdef' >&2\n",
            "printf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"thr_auth\",\"model\":\"gpt-6.1-sol\"}'\n",
            "printf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"verdict\\\":\\\"PASS\\\"}\"}}'\n",
        ),
    )?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;
    let options = ExecutionOptions {
        workspace_dir: Some(workspace.path().to_path_buf()),
        ..ExecutionOptions::default()
    };
    let result = execute_codex_with_binary_and_options(
        codex_request(workspace.path()),
        Duration::from_secs(30),
        &binary,
        &options,
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::Completed);
    let diagnostics = result.receipt.auth_diagnostics;
    assert_eq!(diagnostics.isolated_external_mcp_oauth_warnings.len(), 1);
    assert_eq!(
        diagnostics.isolated_external_mcp_oauth_warnings[0].external_mcp_server,
        "docs"
    );
    assert!(!diagnostics.antigravity_rpc_affected);
    assert!(!diagnostics.auto_credential_refresh_performed);
    assert!(!diagnostics.sanitized_stderr.contains("topsecret"));
    assert!(!diagnostics.sanitized_stderr.contains("1234567890abcdef"));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_kills_and_reaps_owned_process_group_with_blocked_stdin() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempfile::tempdir()?;
    let binary = workspace.path().join("slow-codex");
    let pid_file = workspace.path().join("pid");
    let readiness_marker = workspace.path().join("ready.marker");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nprintf R > '{}'\nsleep 30\n",
            pid_file.display(),
            readiness_marker.display()
        ),
    )?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;
    let options = ExecutionOptions {
        workspace_dir: Some(workspace.path().to_path_buf()),
        ..ExecutionOptions::default()
    };
    let mut request = codex_request(workspace.path());
    request.prompt = "x".repeat(options.max_input_bytes);
    let result = execute_codex_with_binary_and_schema(
        request,
        Duration::from_millis(100),
        &binary,
        &options,
        None,
        Some(&readiness_marker),
    )
    .await?;
    assert_eq!(result.receipt.status, ReceiptStatus::TimedOutUnknown);
    assert_eq!(result.receipt.outcome_state, "outcome_unknown");
    let pid = std::fs::read_to_string(pid_file)?.trim().to_owned();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let alive = Command::new("/bin/kill")
            .args(["-0", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?
            .success();
        if !alive {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "owned process remained alive after timeout cleanup"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    Ok(())
}
