use super::grounding::{
    boundary_schema, build_route_request, fallback_source_files, grounded_prompt,
    normalize_boundary, structured_prompt,
};
use super::*;
use crate::context::SkillRegistry;
use std::process::Stdio;

#[test]
fn request_identity_ignores_transport_and_binds_semantics() -> crate::Result<()> {
    let first = json!({
        "task_spec":"review identity", "target_code_or_diff":"fn first() {}",
        "workspace_dir":"/tmp/workspace", "model":"gpt-6.1-sol",
        "request_id":"cdx-first", "idempotency_key":"key-first",
        "retry_same_request":false, "async_background":false,
    });
    let mut second = first.clone();
    second["request_id"] = Value::String("cdx-second".to_owned());
    second["idempotency_key"] = Value::String("key-second".to_owned());
    second["retry_same_request"] = Value::Bool(true);
    second["async_background"] = Value::Bool(true);
    second["_background_reserved"] = Value::Bool(true);
    assert_eq!(
        triad_request_identity(&first)?,
        triad_request_identity(&second)?
    );
    second["target_code_or_diff"] = Value::String("fn changed() {}".to_owned());
    assert_ne!(
        triad_request_identity(&first)?,
        triad_request_identity(&second)?
    );

    let route = build_route_request("codex_adversarial_audit", &first, "prompt".to_owned())?;
    let first_provider = provider_request_identity(
        "codex_adversarial_audit",
        &first,
        &route,
        "gpt-6.1-sol",
        crate::routing::SandboxMode::ReadOnly,
        Duration::from_secs(30),
        256_000,
    )?;
    let mut transport_changed = first.clone();
    transport_changed["request_id"] = Value::String("cdx-other".to_owned());
    transport_changed["async_background"] = Value::Bool(true);
    assert_eq!(
        first_provider,
        provider_request_identity(
            "codex_adversarial_audit",
            &transport_changed,
            &route,
            "gpt-6.1-sol",
            crate::routing::SandboxMode::ReadOnly,
            Duration::from_secs(30),
            256_000,
        )?
    );
    let mut explicit_defaults = first.clone();
    explicit_defaults["timeout_seconds"] = json!(30.0);
    explicit_defaults["max_input_bytes"] = json!(256_000);
    explicit_defaults["sandbox_mode"] = Value::String("read-only".to_owned());
    assert_eq!(
        first_provider,
        provider_request_identity(
            "codex_adversarial_audit",
            &explicit_defaults,
            &route,
            "gpt-6.1-sol",
            crate::routing::SandboxMode::ReadOnly,
            Duration::from_secs(30),
            256_000,
        )?
    );
    let mut semantic_changed = first;
    semantic_changed["target_code_or_diff"] = Value::String("fn changed() {}".to_owned());
    assert_ne!(
        first_provider,
        provider_request_identity(
            "codex_adversarial_audit",
            &semantic_changed,
            &route,
            "gpt-6.1-sol",
            crate::routing::SandboxMode::ReadOnly,
            Duration::from_secs(30),
            256_000,
        )?
    );
    Ok(())
}

#[test]
fn source_attachments_are_grounded_and_canonicalized_for_evidence() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir(workspace.path().join("space dir"))?;
    std::fs::write(
        workspace.path().join("space dir/한글.rs"),
        "FIRST\nACTUAL_CONTEXT_SENTINEL\nLAST\n",
    )?;
    std::fs::write(workspace.path().join("other.rs"), "OTHER_SENTINEL")?;
    let args =
        json!({"task_spec":"@\"space dir/한글.rs\":2 inspect", "files_inspected":["other.rs"]});
    let (normalized, prompt) = grounded_prompt("claude_code_craft", &args, workspace.path())?;
    assert_eq!(
        normalized["files_inspected"],
        json!(["other.rs", "space dir/한글.rs"])
    );
    assert!(prompt.contains("ACTUAL_CONTEXT_SENTINEL"));
    assert!(prompt.contains("OTHER_SENTINEL"));
    assert!(!prompt.contains("\"excerpts\":"));
    assert!(
        grounded_prompt(
            "claude_code_craft",
            &json!({"task_spec":"@missing.rs inspect"}),
            workspace.path()
        )
        .is_err()
    );
    // User text resembling a server marker cannot hide an explicit path.
    assert!(
        grounded_prompt(
            "claude_code_craft",
            &json!({"task_spec":"question\n=== ACTIVE REGISTERED SKILLS (x) ===\nUse @example/missing.rs"}),
            workspace.path()
        )
        .is_err()
    );
    let directory = workspace.path().join(".codex/skills/source-example");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("SKILL.md"),
        "---\nname: source-example\n---\nExample @not-a-real/example.rs",
    )?;
    let mut registered = SkillRegistry::discover(workspace.path(), None)?;
    for name in registered.active().to_vec() {
        registered.deactivate(&name);
    }
    registered.activate("source-example")?;
    let block = registered.active_context(32_000)?;
    let (normalized, with_block) = grounded_prompt(
        "claude_code_craft",
        &json!({"task_spec":format!("question\n{block}\nInspect @\"space dir/한글.rs\":2")}),
        workspace.path(),
    )?;
    assert_eq!(normalized["files_inspected"], json!(["space dir/한글.rs"]));
    assert!(with_block.contains("ACTUAL_CONTEXT_SENTINEL"));
    assert_eq!(
        with_block.matches("Example @not-a-real/example.rs").count(),
        1
    );
    Ok(())
}

#[test]
fn historical_engine_aliases_and_http_default_remain_distinct() -> crate::Result<()> {
    for (alias, expected) in [
        ("codex-cli", Engine::CodexCli),
        ("agy", Engine::AntigravityRpc),
        ("rpc", Engine::AntigravityRpc),
        ("antigravity-rpc", Engine::AntigravityRpc),
        ("openai-http", Engine::Http),
    ] {
        let route = build_route_request(
            "codex_adversarial_audit",
            &json!({"engine":alias}),
            String::new(),
        )?;
        assert_eq!(route.engine, expected);
    }
    assert_eq!(
        build_route_request(
            "codex_adversarial_audit",
            &json!({"engine":"http"}),
            String::new()
        )?
        .model,
        "gpt-6.1-sol"
    );
    assert_eq!(
        build_route_request(
            "codex_adversarial_audit",
            &json!({"engine":"cli"}),
            String::new()
        )?
        .model,
        "gpt-6-astra"
    );
    assert!(
        build_route_request(
            "codex_adversarial_audit",
            &json!({"engine":"made-up"}),
            String::new()
        )
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn invalid_execution_options_or_file_workspace_never_register_running_jobs()
-> crate::Result<()> {
    let temporary = tempfile::tempdir()?;
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let gateway = Gateway::new(temporary.path().join("state"))
        .with_codex_binary(temporary.path().join("must-not-execute"));
    let invalid = json!({"target_code_or_diff":"x", "sandbox_mode":"not-a-sandbox", "workspace_dir":workspace, "request_id":"cdx-invalid-options"});
    assert!(
        gateway
            .execute_tool("codex_rescue_sandbox", &invalid)
            .await
            .is_err()
    );
    assert!(
        gateway
            .database(|store| store.get_job("cdx-invalid-options"))
            .await?
            .is_none()
    );
    let file = temporary.path().join("file-workspace");
    std::fs::write(&file, "not a directory")?;
    assert!(
        gateway
            .execute_tool(
                "claude_code_craft",
                &json!({"task_spec":"x", "workspace_dir":file, "request_id":"cdx-file-workspace"})
            )
            .await
            .is_err()
    );
    assert!(
        gateway
            .database(|store| store.get_job("cdx-file-workspace"))
            .await?
            .is_none()
    );
    assert_eq!(gateway.active_background_jobs().await, 0);
    gateway.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn configured_default_preserves_raw_request_and_probes_once_per_dispatch() -> crate::Result<()>
{
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir()?;
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let binary = temporary.path().join("native-codex-fixture");
    let audit = json!({"verdict":"PASS","risk_score":0,"executive_critique":"Fixture passed.","findings":[],"failing_test_cases":[],"alexandria_decision_note":"fixture"});
    let event =
        json!({"type":"item.completed","item":{"type":"agent_message","text":audit.to_string()}});
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "app-server" ]; then
  printf x >> probe-count
  while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([^,}}]*\).*/\1/p')
    case "$line" in
      *'"method":"initialize"'*) printf '{{"id":%s,"result":{{"userAgent":"fixture"}}}}\n' "$id" ;;
      *'"method":"config/read"'*) printf '{{"id":%s,"result":{{"config":{{"model":"gpt-6.1-sol","model_provider":"openai"}},"origins":{{"model":{{"kind":"user","path":"fixture"}}}}}}}}\n' "$id" ;;
    esac
  done
  exit 0
fi
cat >/dev/null
printf '%s\n' '{{"type":"thread.started","thread_id":"thr_configured","model":"gpt-6.1-sol"}}' '{event}'
"#
    );
    std::fs::write(&binary, script)?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let gateway = Gateway::new(temporary.path().join("state")).with_codex_binary(binary);
    for (id, background, model) in [
        ("cdx-default-sync", false, "codex-default"),
        ("cdx-default-async", true, "codex-default"),
        ("cdx-explicit-model", false, "gpt-6.1-sol"),
    ] {
        let response = gateway
            .call(
                "codex_adversarial_audit",
                json!({"target_code_or_diff":"fn actual() {}","workspace_dir":workspace,"model":model,"request_id":id,"async_background":background}),
            )
            .await?;
        if background {
            assert!(response["job_id"].as_str().is_some());
        }
        let stored = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let lookup = id.to_owned();
                if let Some(job) = gateway
                    .database(move |store| store.get_job(&lookup))
                    .await?
                    && job.status != JobStatus::Running
                {
                    return Ok::<_, crate::Error>(job);
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        assert_eq!(stored.status, JobStatus::Completed);
        let receipt = stored.receipt.ok_or("missing configured receipt")?;
        assert_eq!(receipt["requested_model"], model);
        assert_eq!(receipt["resolved_model"], "gpt-6.1-sol");
        assert_eq!(receipt["actual_model"], "gpt-6.1-sol");
        assert_eq!(receipt["model_mismatch"], false);
    }
    assert_eq!(std::fs::read(workspace.join("probe-count"))?, b"xx");
    gateway.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn inherited_campaign_cancel_persists_child_cancelled_with_receipt() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir()?;
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let binary = temporary.path().join("slow-codex");
    let pid_file = temporary.path().join("pid");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\necho $$ > '{}'\ncat >/dev/null\nsleep 30\n",
            pid_file.display()
        ),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let gateway = Gateway::new(temporary.path().join("state")).with_codex_binary(binary);
    let owner = gateway.clone();
    let child = gateway.clone();
    let execution = tokio::spawn(async move {
        owner
            .supervise("campaign-parent".to_owned(), async move {
                child
                    .execute_tool(
                        "codex_adversarial_audit",
                        &json!({
                            "target_code_or_diff": "fn main() {}", "workspace_dir": workspace,
                            "request_id": "cdx-inherited-child", "timeout_seconds": 30.0,
                        }),
                    )
                    .await
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !pid_file.is_file() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(gateway.supervisor.cancel("campaign-parent").await);
    let response = tokio::time::timeout(Duration::from_secs(5), execution).await???;
    assert_eq!(response["result"]["receipt"]["status"], "cancelled");
    let stored = gateway
        .database(|store| store.get_job("cdx-inherited-child"))
        .await?
        .ok_or("missing child job")?;
    assert_eq!(stored.status, JobStatus::Cancelled);
    assert_eq!(
        stored.outcome_state,
        crate::storage::OutcomeState::Cancelled
    );
    assert!(stored.cancel_requested);
    assert_eq!(
        stored
            .receipt
            .as_ref()
            .and_then(|receipt| receipt["status"].as_str()),
        Some("cancelled")
    );
    assert_eq!(stored.result, Some(response));
    gateway.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn background_job_is_queryable_and_cancel_reaps_process_group() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir()?;
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let binary = temporary.path().join("slow-codex");
    let pid_file = temporary.path().join("pid");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\necho $$ > '{}'\ncat >/dev/null\nsleep 30\n",
            pid_file.display()
        ),
    )?;
    let mut permissions = std::fs::metadata(&binary)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions)?;
    let gateway = Gateway::new(temporary.path().join("state")).with_codex_binary(binary);
    let queued = gateway
        .call(
            "codex_adversarial_audit",
            json!({
                "target_code_or_diff": "fn main() {}",
                "workspace_dir": workspace,
                "async_background": true,
                "request_id": "cdx-background-test",
                "timeout_seconds": 30.0,
            }),
        )
        .await?;
    assert_eq!(queued["status"], "running");
    assert_eq!(queued["async_dispatched"], true);
    assert_eq!(queued["job_id"], "cdx-background-test");
    let cancelled = gateway
        .call(
            "codex_job_status",
            json!({"job_id": "cdx-background-test", "cancel": true}),
        )
        .await?;
    assert_eq!(cancelled["job"]["status"], "cancelled");
    tokio::time::timeout(Duration::from_secs(3), async {
        while gateway.active_background_jobs().await != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    if let Ok(pid) = std::fs::read_to_string(pid_file) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let alive = tokio::process::Command::new("/bin/kill")
                .args(["-0", pid.trim()])
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
                "background process remained alive after cancellation"
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn terminal_idempotent_background_request_returns_durable_identity() -> crate::Result<()> {
    let temporary = tempfile::tempdir()?;
    let gateway = Gateway::new(temporary.path().join("state"));
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let request = NewJob {
        task_type: "codex_adversarial_audit".to_owned(),
        workspace_dir: workspace.clone(),
        model: Some("gpt-6-astra".to_owned()),
        metadata: BTreeMap::new(),
        job_id: Some("cdx-existing".to_owned()),
        idempotency_key: Some("existing-key".to_owned()),
        provider: Some("openai_codex".to_owned()),
        engine: Some("codex_cli".to_owned()),
        created_at: None,
    };
    gateway
        .database(move |store| {
            store.create_job(request)?;
            store.complete_job("cdx-existing", json!({"ok":true}), None, None)?;
            Ok(())
        })
        .await?;
    let response = gateway
        .call(
            "codex_adversarial_audit",
            json!({
                "target_code_or_diff": "fn main() {}",
                "workspace_dir": workspace,
                "async_background": true,
                "idempotency_key": "existing-key",
            }),
        )
        .await?;
    assert_eq!(response["job_id"], "cdx-existing");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["outcome_state"], "confirmed_success");
    assert_eq!(response["deduplicated_existing_request"], true);
    assert_eq!(response["async_dispatched"], false);
    gateway.shutdown().await?;
    let record = gateway
        .database(|store| store.get_job("cdx-existing"))
        .await?
        .ok_or("missing existing job")?;
    assert!(!record.cancel_requested);
    assert_eq!(record.status, JobStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn rejected_route_is_queryable_without_dispatch() -> crate::Result<()> {
    let temp = tempfile::tempdir()?;
    let gateway =
        Gateway::new(temp.path().join("state")).with_codex_binary(temp.path().join("nonexistent"));
    let response = gateway
        .call(
            "codex_adversarial_audit",
            json!({"target_code_or_diff":"x","workspace_dir":temp.path(),"model":"gpt-5.4","request_id":"cdx-rejected"}),
        )
        .await?;
    assert_eq!(response["status"], "rejected_pre_dispatch");
    assert_eq!(response["error_code"], "UNRECOGNIZED_CODEX_MODEL");
    assert_eq!(response["receipt"]["routing_decision"]["valid"], false);
    assert!(response["receipt"]["actual_model"].is_null());
    let stored = gateway
        .database(|store| store.get_job("cdx-rejected"))
        .await?
        .ok_or("missing rejected record")?;
    assert_eq!(stored.status, JobStatus::RejectedPreDispatch);
    assert_eq!(gateway.active_background_jobs().await, 0);
    let background = gateway
        .call(
            "codex_adversarial_audit",
            json!({
                "target_code_or_diff":"x",
                "workspace_dir":temp.path(),
                "model":"gpt-5.4",
                "request_id":"cdx-rejected-background",
                "async_background":true,
            }),
        )
        .await?;
    assert_eq!(background["status"], "rejected_pre_dispatch");
    assert_eq!(background["async_dispatched"], false);
    assert_eq!(gateway.active_background_jobs().await, 0);
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn input_budget_and_job_wire_are_durable_and_compatible() -> crate::Result<()> {
    let temp = tempfile::tempdir()?;
    std::fs::write(temp.path().join("small.rs"), "fn small() {}")?;
    let gateway =
        Gateway::new(temp.path().join("state")).with_codex_binary(temp.path().join("must-not-run"));
    let response = gateway
        .call(
            "codex_adversarial_audit",
            json!({
                "target_code_or_diff":"x".repeat(256),
                "workspace_dir":temp.path(),
                "request_id":"cdx-budget",
                "max_input_bytes":16,
            }),
        )
        .await?;
    assert_eq!(response["error_code"], "INPUT_BUDGET_EXCEEDED");
    assert_eq!(response["budget"]["input_budget_exceeded"], true);
    assert_eq!(
        response["budget"]["termination_reason"],
        "input_budget_exceeded"
    );
    let status = gateway
        .call("codex_job_status", json!({"job_id":"cdx-budget"}))
        .await?;
    assert_eq!(status["ok"], true);
    assert_eq!(status["job"]["request_id"], "cdx-budget");
    assert_eq!(status["job"]["status"], "input_budget_exceeded");
    assert_eq!(status["outcome_state"], "confirmed_failure");
    let jobs = gateway
        .call("codex_job_list", json!({"workspace_dir":temp.path()}))
        .await?;
    assert_eq!(jobs["jobs"][0]["request_id"], "cdx-budget");
    assert!(jobs["jobs"][0]["elapsed_seconds"].is_number());
    let missing = gateway
        .call("codex_job_status", json!({"job_id":"cdx-missing"}))
        .await?;
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["error"], "Job 'cdx-missing' not found.");
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn background_triad_persistence_failure_keeps_peer_payload_and_fails_job() -> crate::Result<()>
{
    let temp = tempfile::tempdir()?;
    let gateway = Gateway::new(temp.path().join("state"));
    let state_root = temp.path().join("workspace");
    std::fs::create_dir(&state_root)?;
    let create = NewJob {
        task_type: "triad_orchestration".to_owned(),
        workspace_dir: state_root.clone(),
        model: Some("triad".to_owned()),
        metadata: BTreeMap::new(),
        job_id: Some("cdx-partial-triad".to_owned()),
        idempotency_key: None,
        provider: Some("heterarchy_triad".to_owned()),
        engine: Some("pipeline".to_owned()),
        created_at: None,
    };
    gateway
        .database(move |store| {
            store.create_job(create)?;
            Ok(())
        })
        .await?;
    let partial = json!({
        "collaboration_persistence":"failed",
        "persistence_error":"aggregate write unavailable",
        "sonnet_craft":{"status":"succeeded"},
        "codex_audit":{"status":"succeeded"},
    });
    gateway
        .persist_background_outcome(
            "cdx-partial-triad",
            "heterarchy_triad_orchestrate",
            &json!({"workspace_dir":state_root}),
            Ok(partial),
        )
        .await?;
    let stored = gateway
        .database(|store| store.get_job("cdx-partial-triad"))
        .await?
        .ok_or("missing partial triad")?;
    assert_eq!(stored.status, JobStatus::Failed);
    assert_eq!(
        stored
            .result
            .as_ref()
            .map(|value| &value["sonnet_craft"]["status"]),
        Some(&json!("succeeded"))
    );
    assert_eq!(
        stored
            .result
            .as_ref()
            .and_then(|value| value.get("collaboration_id")),
        None
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_provider_output_persists_failed_job() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    let binary = temp.path().join("codex-fixture");
    std::fs::write(
        &binary,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"thr_test\",\"model\":\"gpt-6.1-sol\"}' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{}\"}}'\n",
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let gateway = Gateway::new(temp.path().join("state")).with_codex_binary(binary);
    let response = gateway
        .call(
            "codex_adversarial_audit",
            json!({"target_code_or_diff":"x","workspace_dir":temp.path(),"model":"gpt-6.1-sol","request_id":"cdx-malformed"}),
        )
        .await?;
    assert_eq!(response["ok"], false);
    let stored = gateway
        .database(|store| store.get_job("cdx-malformed"))
        .await?
        .ok_or("missing failure")?;
    assert_eq!(stored.status, JobStatus::Failed);
    assert_eq!(
        stored.receipt.as_ref().map(|r| r["status"].clone()),
        Some(json!("failed"))
    );
    gateway.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn valid_structured_output_sets_and_persists_schema_valid() -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    let binary = temp.path().join("codex-valid-fixture");
    let audit = json!({
        "verdict":"PASS",
        "risk_score":0,
        "executive_critique":"No defect found.",
        "findings":[],
        "failing_test_cases":[],
        "alexandria_decision_note":"Static fixture passed."
    });
    let event = json!({
        "type":"item.completed",
        "item":{"type":"agent_message","text":audit.to_string()}
    });
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}' '{}'\n",
            json!({"type":"thread.started","thread_id":"thr_valid","model":"gpt-6.1-sol"}),
            event,
        ),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let gateway = Gateway::new(temp.path().join("state")).with_codex_binary(binary);
    let response = gateway
        .call(
            "codex_adversarial_audit",
            json!({
                "target_code_or_diff":"fn valid() {}",
                "workspace_dir":temp.path(),
                "model":"gpt-6.1-sol",
                "request_id":"cdx-valid-schema",
            }),
        )
        .await?;
    assert_eq!(response["schema_valid"], true);
    assert_eq!(response["status"], "succeeded");
    assert_eq!(response["outcome_state"], "confirmed_success");
    assert_eq!(response["provider"], "openai_codex");
    assert_eq!(response["engine"], "codex_cli");
    assert_eq!(response["model"], "gpt-6.1-sol");
    assert_eq!(response["requested_model"], "gpt-6.1-sol");
    assert_eq!(response["session_id"], "thr_valid");
    assert_eq!(response["auth_boundary"]["cli_binary_ready"], true);
    assert!(response["budget"]["actual_input_bytes"].is_number());
    assert!(response["structured_output"].is_object());
    assert!(response["raw_output"].is_string());
    assert!(response["receipt"].is_object());
    assert!(response["result"]["evidence_scope"].is_object());
    let stored = gateway
        .database(|store| store.get_job("cdx-valid-schema"))
        .await?
        .ok_or("missing valid job")?;
    assert_eq!(stored.status, JobStatus::Completed);
    assert_eq!(stored.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(stored.session_id.as_deref(), Some("thr_valid"));
    assert_eq!(
        stored.metadata.get("schema_valid").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        stored.result.as_ref().map(|value| &value["schema_valid"]),
        Some(&Value::Bool(true))
    );
    gateway.shutdown().await?;
    Ok(())
}

#[test]
fn triad_argon_internal_tool_uses_frozen_review_schema() -> crate::Result<()> {
    let internal =
        boundary_schema("argon_architecture_review").ok_or("missing internal Argon schema")?;
    assert_eq!(Some(internal), boundary_schema("argon_review"));
    let mut response = json!({"verdict":"PASS","summary":"Architecture checked"});
    normalize_boundary("argon_architecture_review", &mut response)?;
    assert_eq!(response["architectural_risks"], json!([]));
    assert_eq!(response["final_recommendation"], "");
    Ok(())
}
#[test]
fn antigravity_prompt_contains_exact_closed_output_contract() -> crate::Result<()> {
    let schema = boundary_schema("claude_code_craft").ok_or("schema missing")?;
    let prompt = structured_prompt("Inspect actual source", &schema)?;
    assert!(prompt.contains("SERVER-OWNED RESPONSE CONTRACT"));
    assert!(prompt.contains(&serde_json::to_string(&schema)?));
    assert!(prompt.contains("Runtime owns receipt/evidence generation"));
    let mut minimal = json!({"verdict":"PASS","summary":"Reviewed"});
    normalize_boundary("claude_code_craft", &mut minimal)?;
    assert_eq!(minimal["crafted_code"], "");
    assert_eq!(minimal["design_highlights"], json!([]));
    Ok(())
}
#[test]
fn schema_and_fallback_source_are_closed_and_grounded() -> crate::Result<()> {
    let schema = boundary_schema("codex_rescue_sandbox").ok_or("missing rescue schema")?;
    let valid = json!({
        "root_cause":"race", "fix_strategy":"serialize", "patch_or_replacement_code":"diff",
        "verification_commands":["cargo test"]
    });
    crate::catalog::validate_schema(&valid, &schema)?;
    let mut invalid = valid;
    invalid["invented"] = Value::Bool(true);
    assert!(crate::catalog::validate_schema(&invalid, &schema).is_err());
    let temporary = tempfile::tempdir()?;
    std::fs::write(temporary.path().join("actual.rs"), "fn actual_source() {}")?;
    assert_eq!(
        fallback_source_files(temporary.path())?,
        [PathBuf::from("actual.rs")]
    );
    Ok(())
}

#[tokio::test]
async fn handoff_resolution_persists_structured_verdict() -> crate::Result<()> {
    let temporary = tempfile::tempdir()?;
    let gateway = Gateway::new(temporary.path().join("state"));
    let created = gateway
        .call(
            "heterarchy_handoff_create",
            json!({
                "title":"Review", "spec_or_payload":"Inspect contract", "workspace_dir":temporary.path()
            }),
        )
        .await?;
    let ticket_id = created["ticket"]["ticket_id"]
        .as_str()
        .ok_or("missing ticket ID")?;
    let resolved = gateway
        .call(
            "heterarchy_handoff_resolve",
            json!({
                "ticket_id":ticket_id, "status":"completed", "resolution_note":"Safe", "verdict":"PASS"
            }),
        )
        .await?;
    let note = resolved["ticket"]["resolution_note"]
        .as_str()
        .ok_or("missing resolution")?;
    let structured: Value = serde_json::from_str(note)?;
    assert_eq!(structured["verdict"], "PASS");
    assert_eq!(structured["summary"], "Safe");
    assert_eq!(resolved["ticket"]["collaboration_role_state"], "succeeded");
    assert_eq!(resolved["ticket"]["execution_completed"], true);
    assert_eq!(resolved["ticket"]["consensus_eligible"], true);
    Ok(())
}
