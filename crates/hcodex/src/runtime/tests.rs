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
#[test]
fn antigravity_catalog_rejects_wrong_tier_and_no_quota() -> crate::Result<()> {
    let catalog = parse_antigravity_catalog(&json!({"clientModelConfigs": [
        {"label":"Opus","modelId":"claude-opus-5-5-high","modelOrAlias":{"model":"M1"},"quotaInfo":{"remainingFraction":1.0}},
        {"label":"Argon","modelId":"gemini-4-argon-high","modelOrAlias":{"model":"M2"},"quotaInfo":{"remainingFraction":0.0}},
        {"label":"Sonnet","modelId":"claude-sonnet-5-5-medium","modelOrAlias":{"model":"M3"},"quotaInfo":{"remainingFraction":0.5}}]}))?;
    assert!(!catalog[0].routable);
    assert!(!catalog[1].routable);
    assert!(catalog[2].routable);
    Ok(())
}
#[test]
fn parses_endpoint_and_port() -> crate::Result<()> {
    assert_eq!(
        endpoint_candidates(" 123 /app/language_server --csrf_token secret", None)?,
        [(123, "secret".to_owned())]
    );
    assert_eq!(listen_ports("TCP 127.0.0.1:61069 (LISTEN)")?, [61069]);
    Ok(())
}

#[test]
fn antigravity_circuit_opens_after_three_failures_and_success_resets() -> crate::Result<()> {
    antigravity_success(None)?;
    antigravity_failure()?;
    antigravity_failure()?;
    antigravity_failure()?;
    {
        let state = antigravity_state()
            .lock()
            .map_err(|_| "Antigravity state lock poisoned")?;
        assert_eq!(state.consecutive_failures, 3);
        assert!(state.open_until.is_some_and(|until| until > Instant::now()));
    }
    antigravity_success(None)?;
    let state = antigravity_state()
        .lock()
        .map_err(|_| "Antigravity state lock poisoned")?;
    assert_eq!(state.consecutive_failures, 0);
    assert!(state.open_until.is_none());
    Ok(())
}

#[test]
fn antigravity_discovery_errors_have_typed_codes() {
    assert_eq!(
        antigravity_discovery_error_code("Antigravity circuit breaker is open"),
        RouteErrorCode::AntigravityCircuitOpen
    );
    assert_eq!(
        antigravity_discovery_error_code("language_server unavailable"),
        RouteErrorCode::DependencyUnavailable
    );
}

#[tokio::test]
async fn antigravity_zero_quota_returns_confirmed_failure_receipt() -> crate::Result<()> {
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = vec![0_u8; 16_384];
        let _size = stream.read(&mut request).await?;
        let body = r#"{"clientModelConfigs":[{"label":"Sonnet","modelId":"claude-sonnet-5-5-medium","modelOrAlias":{"model":"M3"},"quotaInfo":{"remainingFraction":0.0}}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await?;
        Ok::<_, crate::Error>(())
    });
    let endpoint = AntigravityEndpoint {
        base_url: format!("http://{address}"),
        csrf_token: "secret".to_owned(),
        pid: 1,
    };
    let request = RouteRequest {
        provider: crate::routing::Provider::Antigravity,
        engine: Engine::AntigravityRpc,
        model: "claude-sonnet-5-5-medium".to_owned(),
        effort: "medium".to_owned(),
        prompt: "review".to_owned(),
    };
    let result = AntigravityClient::new()?
        .execute_at_with_options(
            request,
            Duration::from_secs(2),
            &endpoint,
            &ExecutionOptions::default(),
        )
        .await?;
    server.await??;
    assert_eq!(result.receipt.status, ReceiptStatus::Failed);
    assert_eq!(result.receipt.outcome_state, "confirmed_failure");
    assert_eq!(
        result.receipt.error_code,
        Some(RouteErrorCode::AntigravityQuotaExhausted)
    );
    assert_eq!(result.receipt.failure_reason, result.receipt.error);
    Ok(())
}

#[tokio::test]
async fn antigravity_success_exposes_only_selected_catalog_placeholder() -> crate::Result<()> {
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().await?;
            let mut request = vec![0_u8; 16_384];
            let _size = stream.read(&mut request).await?;
            let body = if index == 0 {
                r#"{"clientModelConfigs":[{"label":"Sonnet","modelId":"claude-sonnet-5-5-medium","modelOrAlias":{"model":"MODEL_PLACEHOLDER_M404"},"quotaInfo":{"remainingFraction":0.8}}]}"#
            } else {
                r#"{"modelId":"claude-sonnet-5-5-medium","response":"{\"verdict\":\"PASS\"}"}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        Ok::<_, crate::Error>(())
    });
    let endpoint = AntigravityEndpoint {
        base_url: format!("http://{address}"),
        csrf_token: "secret".to_owned(),
        pid: 1,
    };
    let request = RouteRequest {
        provider: crate::routing::Provider::Antigravity,
        engine: Engine::AntigravityRpc,
        model: "claude-sonnet-5-5-medium".to_owned(),
        effort: "medium".to_owned(),
        prompt: "review".to_owned(),
    };
    let result = AntigravityClient::new()?
        .execute_at_with_options(
            request,
            Duration::from_secs(2),
            &endpoint,
            &ExecutionOptions::default(),
        )
        .await?;
    server.await??;
    assert_eq!(result.receipt.status, ReceiptStatus::Completed);
    assert_eq!(
        result.placeholder.as_deref(),
        Some("MODEL_PLACEHOLDER_M404")
    );
    assert_eq!(
        result
            .receipt
            .routing_decision
            .as_ref()
            .and_then(|decision| decision.placeholder.as_deref()),
        Some("MODEL_PLACEHOLDER_M404")
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
        let result = codex_git_review_with_binary(
            request,
            target,
            Duration::from_secs(2),
            &options,
            &binary,
        )
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
        Duration::from_secs(2),
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
        Duration::from_secs(2),
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
        Duration::from_secs(2),
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
        Duration::from_secs(2),
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
async fn timeout_kills_and_reaps_owned_process_group() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempfile::tempdir()?;
    let binary = workspace.path().join("slow-codex");
    let pid_file = workspace.path().join("pid");
    let readiness_fifo = workspace.path().join("ready.fifo");
    let fifo_status = std::process::Command::new("mkfifo")
        .arg(&readiness_fifo)
        .status()?;
    if !fifo_status.success() {
        return Err("mkfifo failed for readiness handshake".into());
    }
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nprintf R > '{}'\nsleep 30\n",
            pid_file.display(),
            readiness_fifo.display()
        ),
    )?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;
    let options = ExecutionOptions {
        workspace_dir: Some(workspace.path().to_path_buf()),
        ..ExecutionOptions::default()
    };
    let result = execute_codex_with_binary_and_schema(
        codex_request(workspace.path()),
        Duration::from_millis(100),
        &binary,
        &options,
        None,
        Some(&readiness_fifo),
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

#[tokio::test]
async fn refresh_readback_is_injectable_and_redacts_csrf() -> crate::Result<()> {
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        for _request_index in 0..2 {
            let (mut stream, _) = listener.accept().await?;
            let mut request = vec![0_u8; 16_384];
            let size = stream.read(&mut request).await?;
            let request = String::from_utf8(request[..size].to_vec())?;
            let body = if request.contains("RefreshMcpServers") {
                r#"{"refreshed":true,"echo":"secret-csrf"}"#
            } else {
                r#"{"clientModelConfigs":[]}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        Ok::<_, crate::Error>(())
    });
    let endpoint = AntigravityEndpoint {
        base_url: format!("http://{address}"),
        csrf_token: "secret-csrf".to_owned(),
        pid: 42,
    };
    let result = AntigravityClient::new()?
        .refresh_at(&endpoint, Duration::from_secs(2))
        .await?;
    server.await??;
    let serialized = result.to_string();
    assert_eq!(result["ok"], true);
    assert_eq!(result["pid"], 42);
    assert!(!serialized.contains("secret-csrf"));
    assert!(serialized.contains("[REDACTED]"));
    Ok(())
}
