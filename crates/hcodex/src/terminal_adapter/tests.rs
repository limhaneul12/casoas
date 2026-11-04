use super::*;
use crate::dag::{
    DagNode, DagWave, GateProof, NodeKind, NodeRole, NodeState, SettlementReceipt, SettlementStage,
};
use crate::routing::SandboxMode;
use crate::tui::TuiTheme;
use std::path::Path;

#[test]
fn human_provider_output_uses_actual_code_or_summary_before_raw_envelopes() {
    assert_eq!(
        response_text(
            &json!({"result":{"structured_output":{"implementation_code":"VISIBLE_CODE\nsecond line","summary":"short"},"output":"raw JSON envelope"}})
        ),
        "VISIBLE_CODE\nsecond line"
    );
    assert_eq!(
        response_text(
            &json!({"structured_output":{"crafted_code":"","summary":"Actual read-only summary"},"output":"raw"})
        ),
        "Actual read-only summary"
    );
    assert_eq!(
        response_text(&json!({"output":"plain native review"})),
        "plain native review"
    );
}

#[tokio::test]
async fn a_late_context_receipt_keeps_its_model_origin_after_model_selection_changes() -> Result<()>
{
    let workspace = tempfile::tempdir()?;
    let root = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(root.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    let (sender, mut receiver) = mpsc::channel(4);
    handler.state.lock().await.active_model = "gemini-4-argon-high".to_owned();
    handler
        .apply_usage(
            &json!({"receipt":{
                "request_id":"late-codex", "actual_model":"gpt-6.1-sol", "tokens_in":80,"tokens_out":20,
                "context_window":1000,"current_context_tokens":100
            }}),
            &sender,
        )
        .await?;
    let Some(TuiUpdate::Tokens(usage)) = receiver.recv().await else {
        return Err("missing token event".into());
    };
    assert_eq!(usage.context_origin_model.as_deref(), Some("gpt-6.1-sol"));
    assert!(usage.context_status().contains("[gpt-6.1-sol]"));
    assert_eq!(
        handler.state.lock().await.active_model,
        "gemini-4-argon-high"
    );
    Ok(())
}

#[tokio::test]
async fn request_context_budget_is_evidence_backed_and_mixed_results_are_not_combined() -> Result<()>
{
    let workspace = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    let (sender, mut receiver) = mpsc::channel(4);
    handler
        .apply_usage(
            &json!({
                "budget":{"actual_input_bytes":1536,"max_input_bytes":256000},
                "result":{"budget":{"actual_input_bytes":1536,"max_input_bytes":256000},
                    "structured_output":{"budget":{"actual_input_bytes":1,"max_input_bytes":2}}}
            }),
            &sender,
        )
        .await?;
    assert!(matches!(
        receiver.recv().await,
        Some(TuiUpdate::Tokens(TokenUsage {
            request_input_bytes: Some(1536),
            request_limit_bytes: Some(256000),
            context_limit: None,
            ..
        }))
    ));
    handler
        .apply_usage(
            &json!({"roles":[
                {"budget":{"actual_input_bytes":1536,"max_input_bytes":256000}},
                {"budget":{"actual_input_bytes":4096,"max_input_bytes":128000}}
            ]}),
            &sender,
        )
        .await?;
    assert!(matches!(
        receiver.recv().await,
        Some(TuiUpdate::Tokens(TokenUsage {
            request_input_bytes: None,
            request_limit_bytes: None,
            ..
        }))
    ));
    Ok(())
}

#[test]
fn legacy_effect_aliases_validate_atomically() -> Result<()> {
    let mut config = EffectConfig::default();
    let previous = config.summary();
    assert!(apply_effect_argument(&mut config, "opus high").is_err());
    assert_eq!(config.summary(), previous);
    assert!(apply_effect_argument(&mut config, "codex unsupported-model high").is_err());
    assert_eq!(config.summary(), previous);
    apply_effect_argument(&mut config, "codex 6-1 sol xhigh")?;
    assert_eq!(config.codex.model, "gpt-6.1-sol");
    apply_effect_argument(&mut config, "argon low")?;
    assert_eq!(config.argon.effort, "low");
    apply_effect_argument(&mut config, "sonnet medium")?;
    assert_eq!(config.claude.model, "claude-sonnet-5-5-medium");
    apply_effect_argument(&mut config, "all medium")?;
    assert_eq!(config.argon.effort, "medium");
    assert_eq!(config.claude.effort, "medium");
    assert_eq!(config.codex.effort, "medium");
    let before_rejected_all = config.summary();
    assert!(apply_effect_argument(&mut config, "all high").is_err());
    assert_eq!(config.summary(), before_rejected_all);
    Ok(())
}

#[tokio::test]
async fn registered_prefixes_reserve_commands_preserve_quoted_paths_and_keep_plan_only()
-> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("lib.rs"), "pub fn actual() {}")?;
    std::fs::create_dir(workspace.path().join("src"))?;
    std::fs::write(
        workspace.path().join("src/my  file.rs"),
        "pub fn exact_path() {}",
    )?;
    for name in ["help", "review"] {
        let directory = home.path().join(".codex/skills").join(name);
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nREAL_{name}"),
        )?;
    }
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    handler.state.lock().await.skills = core_registry(workspace.path(), home.path())?;
    let (sender, mut receiver) = mpsc::channel(64);
    for (text, expected) in [
        ("$codegraph-operator /help", TuiAction::Help),
        ("$codegraph-operator ?", TuiAction::Help),
        ("$codegraph-operator q", TuiAction::Quit),
        (
            "$codegraph-operator /review exact focus",
            TuiAction::Review(Some("exact focus".to_owned())),
        ),
        (
            r#"$codegraph-operator /ask Read @"src/my  file.rs""#,
            TuiAction::SubmitPrompt(r#"Read @"src/my  file.rs""#.to_owned()),
        ),
    ] {
        assert_eq!(
            handler
                .registered_skill_action(TuiAction::SubmitPrompt(text.to_owned()), &sender)
                .await?,
            Some(expected)
        );
    }
    assert!(
        !handler
            .state
            .lock()
            .await
            .skills
            .active()
            .iter()
            .any(|name| matches!(name.as_str(), "help" | "review"))
    );
    let literal = "`$review /help` and \"$review\"\n```rust\n$review\n```";
    assert_eq!(
        handler
            .registered_skill_action(TuiAction::SubmitPrompt(literal.to_owned()), &sender)
            .await?,
        Some(TuiAction::SubmitPrompt(literal.to_owned()))
    );
    let goal = "$codegraph-operator /ask audit-only goal";
    assert_eq!(
        handler
            .registered_skill_action(TuiAction::PlanDag(goal.to_owned()), &sender)
            .await?,
        Some(TuiAction::PlanDag("/ask audit-only goal".to_owned()))
    );
    assert_eq!(
        handler
            .registered_skill_action(TuiAction::EnterDag(Some(goal.to_owned())), &sender)
            .await?,
        Some(TuiAction::EnterDag(Some("/ask audit-only goal".to_owned())))
    );
    handler
        .dispatch(
            TuiAction::PlanDag(r#"$codegraph-operator /ask audit @"src/my  file.rs""#.to_owned()),
            sender.clone(),
        )
        .await?;
    let jobs = handler
        .gateway
        .database(|store| store.list_jobs(20, None))
        .await?;
    assert!(!jobs.is_empty());
    assert!(
        jobs.iter()
            .filter_map(|job| job.result.as_ref())
            .filter_map(|result| result["compiled_plan"]["nodes"].as_array())
            .flatten()
            .any(|node| node["read_files"]
                .as_array()
                .is_some_and(|paths| paths.iter().any(|path| path == "src/my  file.rs"))),
        "exact quoted path must reach the plan scope"
    );
    assert!(
        jobs.iter()
            .all(|job| matches!(job.task_type.as_str(), "dag_ulw" | "dag_checkpoint")),
        "plan-only must not dispatch providers"
    );
    while receiver.try_recv().is_ok() {}
    assert_eq!(
        handler
            .registered_skill_action(TuiAction::SubmitPrompt("$review".to_owned()), &sender)
            .await?,
        None
    );
    assert!(
        handler
            .state
            .lock()
            .await
            .skills
            .active()
            .contains(&"review".to_owned())
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn native_default_selection_locks_a_concrete_codex_model_without_provider_dispatch()
-> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir()?;
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    let binary = root.path().join("codex");
    std::fs::write(
        root.path().join("response"),
        r#"{"id":2,"result":{"config":{"model":"gpt-6.1-sol","model_provider":null}}}"#,
    )?;
    std::fs::write(
        &binary,
        concat!(
            "#!/bin/sh\n",
            "test \"$1:$2\" = 'app-server:--stdio' || exit 90\n",
            "dir=$(dirname \"$0\")\n",
            "while IFS= read -r request; do\n",
            "printf '%s\\n' \"$request\" >> \"$dir/requests\"\n",
            "case \"$request\" in\n",
            "*'\"method\":\"initialize\"'*) printf '%s\\n' '{\"id\":1,\"result\":{}}';;\n",
            "*'\"method\":\"config/read\"'*) cat \"$dir/response\"; printf '\\n';;\n",
            "*'\"method\":\"initialized\"'*) :;;\n",
            "*) exit 91;;\n",
            "esac\ndone\n"
        ),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))?;
    let gateway = Gateway::new(root.path().join("state")).with_codex_binary(binary);
    let handler = AppHandler::new(gateway, workspace.clone())?;
    let (sender, _receiver) = mpsc::channel(16);
    handler
        .select_model("CODEX-DEFAULT HIGH".to_owned(), sender)
        .await?;
    let state = handler.state.lock().await;
    assert_eq!(state.active_model, "gpt-6.1-sol");
    assert_eq!(state.effort, "high");
    let (tool, args) = direct_request(
        &state.active_model,
        &state.effort,
        "inspect",
        "",
        &workspace,
    );
    assert_eq!(tool, "codex_targeted_synthesize");
    assert_eq!(args["model"], "gpt-6.1-sol");
    drop(state);
    let requests = std::fs::read_to_string(root.path().join("requests"))?;
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.contains("config/read"))
            .count(),
        1
    );
    assert!(
        handler
            .gateway
            .database(|store| store.list_jobs(10, None))
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn prefixed_commands_update_real_ui_state_without_redispatching() -> Result<()> {
    use crate::tui::{TuiApp, TuiCatalog};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    async fn input(
        handler: &AppHandler,
        app: &mut TuiApp,
        text: &str,
        sender: &mpsc::Sender<TuiUpdate>,
        receiver: &mut mpsc::Receiver<TuiUpdate>,
    ) -> Result<()> {
        app.handle_paste(text);
        let action = app
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?
            .ok_or("command was not submitted")?;
        handler.dispatch(action, sender.clone()).await?;
        while let Ok(update) = receiver.try_recv() {
            app.apply_update(update);
        }
        Ok(())
    }
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    handler.state.lock().await.skills = core_registry(workspace.path(), home.path())?;
    let mut app = TuiApp::new_with_motion(
        workspace.path().to_path_buf(),
        TuiCatalog::command_catalog(),
        false,
    );
    let (sender, mut receiver) = mpsc::channel(64);
    input(
        &handler,
        &mut app,
        "$codegraph-operator /dag-mode",
        &sender,
        &mut receiver,
    )
    .await?;
    assert!(app.dag_mode());
    assert!(app.lanes().is_empty());
    input(
        &handler,
        &mut app,
        "$codegraph-operator /dag-exit",
        &sender,
        &mut receiver,
    )
    .await?;
    assert!(!app.dag_mode());
    app.apply_update(TuiUpdate::View {
        title: "previous".to_owned(),
        body: vec!["before".to_owned()],
    });
    app.apply_update(TuiUpdate::View {
        title: "current".to_owned(),
        body: vec!["now".to_owned()],
    });
    input(
        &handler,
        &mut app,
        "$codegraph-operator /back",
        &sender,
        &mut receiver,
    )
    .await?;
    assert_eq!(app.current_view().map(|(title, _)| title), Some("previous"));
    input(
        &handler,
        &mut app,
        "$codegraph-operator ?",
        &sender,
        &mut receiver,
    )
    .await?;
    assert_eq!(app.current_view().map(|(title, _)| title), Some("Help"));
    input(
        &handler,
        &mut app,
        "$codegraph-operator /clear",
        &sender,
        &mut receiver,
    )
    .await?;
    assert!(app.current_view().is_none());
    assert!(app.transcript().is_empty());
    input(
        &handler,
        &mut app,
        "$codegraph-operator q",
        &sender,
        &mut receiver,
    )
    .await?;
    assert!(app.should_quit());
    assert!(
        handler
            .gateway
            .database(|store| store.list_jobs(10, None))
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn model_alias_with_effort_updates_actual_selection_and_rejects_atomically() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    let (sender, mut receiver) = mpsc::channel(32);
    handler
        .select_model("6-1 sol max".to_owned(), sender.clone())
        .await?;
    assert_eq!(handler.state.lock().await.active_model, "gpt-6.1-sol");
    assert_eq!(handler.state.lock().await.effort, "max");
    while receiver.try_recv().is_ok() {}
    assert!(
        handler
            .select_model("opus high".to_owned(), sender.clone())
            .await
            .is_err()
    );
    assert_eq!(handler.state.lock().await.active_model, "gpt-6.1-sol");
    assert!(
        matches!(receiver.recv().await,Some(TuiUpdate::ActiveModel(model)) if model=="gpt-6.1-sol")
    );
    handler.select_model("argon low".to_owned(), sender).await?;
    assert_eq!(
        handler.state.lock().await.active_model,
        "gemini-4-argon-low"
    );
    assert_eq!(handler.state.lock().await.effort, "low");
    Ok(())
}

#[tokio::test]
async fn registered_skill_prefixes_preserve_commands_and_do_not_dispatch_skill_only_input()
-> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    handler.state.lock().await.skills = core_registry(workspace.path(), home.path())?;
    let (sender, mut receiver) = mpsc::channel(16);
    assert_eq!(
        handler
            .registered_skill_action(
                TuiAction::SubmitPrompt("$codegraph-operator /ask inspect this".to_owned()),
                &sender
            )
            .await?,
        Some(TuiAction::SubmitPrompt("inspect this".to_owned()))
    );
    assert_eq!(
        handler
            .registered_skill_action(
                TuiAction::RunDag("$codegraph-operator audit this".to_owned()),
                &sender
            )
            .await?,
        Some(TuiAction::RunDag("audit this".to_owned()))
    );
    assert_eq!(
        handler
            .registered_skill_action(
                TuiAction::RunDag("$codegraph-operator /ask literal DAG objective".to_owned()),
                &sender
            )
            .await?,
        Some(TuiAction::RunDag("/ask literal DAG objective".to_owned()))
    );
    assert_eq!(
        handler
            .registered_skill_action(
                TuiAction::AutoDag("$codegraph-operator /ask direct question".to_owned()),
                &sender
            )
            .await?,
        Some(TuiAction::SubmitPrompt("direct question".to_owned()))
    );
    assert_eq!(
        handler
            .registered_skill_action(
                TuiAction::SubmitPrompt("$codegraph-operator".to_owned()),
                &sender
            )
            .await?,
        None
    );
    assert!(
        handler
            .state
            .lock()
            .await
            .skills
            .active()
            .contains(&"codegraph-operator".to_owned())
    );
    assert!(
        handler
            .gateway
            .database(|store| store.list_jobs(10, None))
            .await?
            .is_empty()
    );
    assert!(receiver.try_recv().is_ok());
    Ok(())
}

#[tokio::test]
async fn planned_prompt_inspector_shows_real_node_instructions_and_exact_filtering() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("lib.rs"), "pub fn actual() {}")?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    let mut registry = core_registry(workspace.path(), home.path())?;
    registry.activate_core();
    handler.state.lock().await.skills = registry;
    let (sender, mut receiver) = mpsc::channel(64);
    handler
        .plan_dag(
            "audit actual source".to_owned(),
            false,
            None,
            sender.clone(),
        )
        .await?;
    while receiver.try_recv().is_ok() {}
    handler
        .show_prompts(Some("codex".to_owned()), sender.clone())
        .await?;
    let view = receiver
        .recv()
        .await
        .ok_or("inspector did not emit a view")?;
    let TuiUpdate::View { body, .. } = view else {
        return Err("inspector must return view".into());
    };
    let text = body.join("\n");
    assert!(text.contains("1. TASK & REGISTERED SKILLS"));
    assert!(text.contains("REAL_BODY_codegraph-operator"));
    assert!(text.contains("5. SETTLEMENT & GATE EVIDENCE"));
    assert!(text.contains("Planned / unverified"));
    assert!(!text.contains("[n1_topology_spec]"));
    handler
        .show_prompts(Some("absent-node".to_owned()), sender)
        .await?;
    let TuiUpdate::View { body, .. } = receiver.recv().await.ok_or("missing filtered view")? else {
        return Err("expected view".into());
    };
    assert!(body.join("\n").contains("No node matches 'absent-node'"));
    Ok(())
}

#[tokio::test]
async fn manifest_diff_covers_non_git_untracked_content_changes() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("modified.rs"), "before")?;
    std::fs::write(workspace.path().join("deleted.rs"), "before")?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    let (sender, mut receiver) = mpsc::channel(8);
    handler.show_diff(sender.clone()).await?;
    let _baseline = receiver.recv().await;
    std::fs::write(workspace.path().join("modified.rs"), "after")?;
    std::fs::remove_file(workspace.path().join("deleted.rs"))?;
    std::fs::write(workspace.path().join("untracked.rs"), "new")?;
    handler.show_diff(sender).await?;
    let TuiUpdate::View { body, .. } = receiver.recv().await.ok_or("missing manifest diff")? else {
        return Err("expected view".into());
    };
    let text = body.join("\n");
    assert!(text.contains("Modified: modified.rs"));
    assert!(text.contains("Deleted: deleted.rs"));
    assert!(text.contains("Added: untracked.rs"));
    Ok(())
}

fn core_registry(workspace: &Path, home: &Path) -> Result<SkillRegistry> {
    for name in [
        "codex-heterarchy",
        "codegraph-operator",
        "codebase-refactor-guardian",
    ] {
        let root = if name == "codebase-refactor-guardian" {
            ".agents/skills"
        } else {
            ".codex/skills"
        };
        let directory = home.join(root).join(name);
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nREAL_BODY_{name}"),
        )?;
    }
    SkillRegistry::discover(workspace, Some(home))
}

#[tokio::test]
async fn core_selection_confirms_real_paths_and_preserves_preset_semantics() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    handler.state.lock().await.skills = core_registry(workspace.path(), home.path())?;
    let (sender, mut receiver) = mpsc::channel(32);
    for _ in 0..2 {
        handler
            .toggle_skill("core".to_owned(), sender.clone())
            .await?;
    }
    let active = handler.state.lock().await.skills.active().to_vec();
    assert_eq!(
        active,
        [
            "codex-heterarchy",
            "codegraph-operator",
            "codebase-refactor-guardian"
        ]
    );
    assert!(
        handler
            .toggle_skill("unknown".to_owned(), sender.clone())
            .await
            .is_err()
    );
    assert_eq!(handler.state.lock().await.skills.active(), active);
    handler
        .toggle_skill("default".to_owned(), sender.clone())
        .await?;
    assert_eq!(
        handler.state.lock().await.skills.active(),
        ["codex-heterarchy"]
    );
    handler
        .toggle_skill("off".to_owned(), sender.clone())
        .await?;
    assert!(handler.state.lock().await.skills.active().is_empty());
    assert_eq!(handler.gateway.active_background_jobs().await, 0);
    let mut text = String::new();
    while let Ok(update) = receiver.try_recv() {
        if let TuiUpdate::Transcript(entry) = update {
            text.push_str(&entry.message);
        }
    }
    for name in &active {
        assert!(text.contains(&format!("Loaded ${name} from")));
    }
    handler.state.lock().await.skills = core_registry(workspace.path(), home.path())?;
    let prior = handler.state.lock().await.skills.active().to_vec();
    std::fs::remove_file(
        home.path()
            .join(".agents/skills/codebase-refactor-guardian/SKILL.md"),
    )?;
    assert!(
        handler
            .toggle_skill("core".to_owned(), sender.clone())
            .await
            .is_err()
    );
    assert_eq!(handler.state.lock().await.skills.active(), prior);
    assert!(
        receiver.try_recv().is_err(),
        "failed preflight published a selection"
    );
    let empty_home = tempfile::tempdir()?;
    handler.state.lock().await.skills =
        SkillRegistry::discover(workspace.path(), Some(empty_home.path()))?;
    handler.toggle_skill("core".to_owned(), sender).await?;
    let mut missing_report = String::new();
    while let Ok(update) = receiver.try_recv() {
        if let TuiUpdate::Transcript(entry) = update {
            missing_report.push_str(&entry.message);
        }
    }
    assert!(missing_report.contains("unavailable core skills:"));
    for name in active {
        assert!(missing_report.contains(&name));
    }
    Ok(())
}

#[test]
fn selected_core_bodies_reach_direct_and_every_dag_node_request() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("module.rs"), "fn module() {}")?;
    let mut skills = core_registry(workspace.path(), home.path())?;
    assert!(skills.activate_core().is_empty());
    let context = skills.active_context(32_000)?;
    let plan = DagPlanner::generate("audit module", workspace.path())?;
    let nodes = dag_request_nodes(&plan, &context);
    for model in ["gpt-6.1-sol", "gemini-4-argon-high"] {
        let (_, request) =
            direct_request(model, "high", "audit module", &context, workspace.path());
        let serialized = serde_json::to_string(&request)?;
        for name in skills.active() {
            assert!(serialized.contains(&format!("REAL_BODY_{name}")));
        }
    }
    for (node, view_node) in nodes.iter().zip(&plan.nodes) {
        let task = node["task"].as_str().ok_or("node task missing")?;
        for name in skills.active() {
            assert!(task.contains(&format!("REAL_BODY_{name}")));
        }
        assert_eq!(view_node.task, "audit module");
    }
    Ok(())
}

#[test]
fn direct_requests_preserve_selected_provider_family() {
    let (codex_tool, codex) = direct_request(
        "gpt-6.1-sol",
        "xhigh",
        "implement",
        "REAL SKILL",
        Path::new("/tmp/work"),
    );
    assert_eq!(codex_tool, "codex_targeted_synthesize");
    assert_eq!(codex["model"], "gpt-6.1-sol");
    let (argon_tool, argon) = direct_request(
        "gemini-4-argon-high",
        "high",
        "design",
        "REAL SKILL",
        Path::new("/tmp/work"),
    );
    assert_eq!(argon_tool, "claude_code_craft");
    assert_eq!(argon["claude_model"], "gemini-4-argon-high");
}

#[test]
fn dag_plan_is_rendered_as_waiting_not_accepted() {
    let plan = DagPlan {
        goal: "migrate".to_owned(),
        nodes: vec![DagNode {
            id: "n1".to_owned(),
            title: "Map".to_owned(),
            task: "Map boundary".to_owned(),
            role: NodeRole::BoundaryMapper,
            node_kind: NodeKind::Standard,
            model: "gemini-4-argon-high".to_owned(),
            reasoning_effort: "high".to_owned(),
            dependencies: Vec::new(),
            read_files: Vec::new(),
            write_files: Vec::new(),
            symbols: Vec::new(),
            verify_command: None,
            reproduction_expected_exit_nonzero: true,
        }],
        waves: vec![DagWave {
            index: 0,
            node_ids: vec!["n1".to_owned()],
        }],
        hazards: Vec::new(),
        total_nodes: 1,
        total_waves: 1,
        blocking_hazards_count: 0,
        ascii_graph: "n1".to_owned(),
    };
    let lanes = dag_lanes(&plan);
    assert_eq!(lanes[0].state, LaneState::Waiting);
    assert_eq!(lanes[0].note, "Start node");
}

#[test]
fn usage_is_summed_only_from_real_receipt_fields() {
    let value = json!({
        "receipt":{"request_id":"r1","tokens_in":10,"tokens_out":3},
        "result":{"receipt":{"request_id":"r1","tokens_in":10,"tokens_out":3}},
        "roles": [
            {"receipt":{"request_id":"r1","tokens_in":10,"tokens_out":3}},
            {"receipt":{"request_id":"r2","tokens_in":7,"tokens_out":2}}
        ],
        "claimed_tokens": 9999,
        "bare":{"tokens_in":500,"tokens_out":500},
        "structured_output":{"receipt":{"request_id":"fake","tokens_in":900,"tokens_out":900}}
    });
    assert_eq!(extract_usage(&value), (17, 5));
    let retried = json!({
        "receipt":{"request_id":"r1","attempt_count":1,"tokens_in":10,"tokens_out":3},
        "result":{"receipt":{"request_id":"r1","attempt_count":2,"tokens_in":4,"tokens_out":1}}
    });
    assert_eq!(extract_usage(&retried), (14, 4));
    let context = json!({
        "result": {"receipt": {
            "request_id":"r1",
            "context_window": 128000,
            "current_context_tokens": 32000,
            "total_token_usage": {"total_tokens": 999999}
        }}
    });
    assert_eq!(
        extract_context_usage(&context),
        Some((128_000, 32_000, None))
    );
    let duplicated = json!({
        "receipt":{"request_id":"r1","context_window":128000,"current_context_tokens":32000},
        "result":{"receipt":{"request_id":"r1","context_window":128000,"current_context_tokens":32000}}
    });
    assert_eq!(
        extract_context_usage(&duplicated),
        Some((128_000, 32_000, None))
    );
    let ambiguous = json!({"roles":[
        {"receipt":{"request_id":"r1","context_window":128000,"current_context_tokens":32000}},
        {"receipt":{"request_id":"r2","context_window":128000,"current_context_tokens":32000}}
    ]});
    assert_eq!(extract_context_usage(&ambiguous), None);
    let retry_context = json!({"roles":[
        {"receipt":{"request_id":"r1","attempt_count":1,"context_window":128000,"current_context_tokens":32000}},
        {"receipt":{"request_id":"r1","attempt_count":2,"context_window":128000,"current_context_tokens":32000}}
    ]});
    assert_eq!(extract_context_usage(&retry_context), None);
}

#[test]
fn settlement_view_requires_real_gate_evidence_for_verified_label() {
    let receipt = SettlementReceipt {
        node_id: "n1".to_owned(),
        state: NodeState::Settled,
        settlement_stage: SettlementStage::Settled,
        role: NodeRole::ContractVerifier,
        node_kind: NodeKind::Verification,
        wave_index: 0,
        model_id: "gpt-6.1-sol".to_owned(),
        duration_seconds: 1.0,
        pre_manifest_digest: "sha256:pre".to_owned(),
        stopped_manifest_digest: "sha256:stopped".to_owned(),
        post_manifest_digest: "sha256:post".to_owned(),
        changed_files: Vec::new(),
        mutated_files: Vec::new(),
        out_of_scope_writes: Vec::new(),
        dirty_read_files: Vec::new(),
        verify_passed: true,
        reproduction_observed: true,
        output_summary: "verified".to_owned(),
        error: None,
        gate: GateProof {
            command: "cargo test".to_owned(),
            executed: true,
            exit_code: Some(0),
            stdout_digest: Some("sha256:evidence".to_owned()),
        },
    };
    let (state, note) = settlement_view(&receipt);
    assert_eq!(state, LaneState::Accepted);
    assert!(note.contains("SETTLED · verified"));
    assert!(note.contains("sha256:evidence"));
}

#[test]
fn effect_permissions_and_theme_are_typed_and_validated() -> crate::Result<()> {
    let mut effects = EffectConfig::default();
    apply_effect_argument(&mut effects, "codex 6.1 sol max")?;
    assert_eq!(effects.codex.model, "gpt-6.1-sol");
    assert_eq!(effects.codex.effort, "max");
    apply_effect_argument(&mut effects, "opus medium")?;
    assert_eq!(effects.claude.model, "claude-opus-5-5-medium");
    apply_effect_argument(&mut effects, "style cyber-matrix")?;
    assert_eq!(effects.style, TuiTheme::CyberMatrix);
    assert_eq!(permission_sandbox("read-only"), SandboxMode::ReadOnly);
    assert_eq!(permission_sandbox("auto-edit"), SandboxMode::WorkspaceWrite);
    assert_eq!(permission_sandbox("yolo"), SandboxMode::DangerFullAccess);
    assert!(apply_effect_argument(&mut effects, "opus high").is_err());
    let mut write_plan = DagPlan {
        goal: "write".to_owned(),
        nodes: vec![DagNode {
            id: "n1".to_owned(),
            title: "write".to_owned(),
            task: "write".to_owned(),
            role: NodeRole::InterfaceBuilder,
            node_kind: NodeKind::Standard,
            model: effects.claude.model.clone(),
            reasoning_effort: "medium".to_owned(),
            dependencies: Vec::new(),
            read_files: Vec::new(),
            write_files: vec![PathBuf::from("src/lib.rs")],
            symbols: Vec::new(),
            verify_command: None,
            reproduction_expected_exit_nonzero: true,
        }],
        waves: vec![DagWave {
            index: 0,
            node_ids: vec!["n1".to_owned()],
        }],
        hazards: Vec::new(),
        total_nodes: 1,
        total_waves: 1,
        blocking_hazards_count: 0,
        ascii_graph: "n1".to_owned(),
    };
    assert!(ensure_plan_permission("read-only", &write_plan, true).is_err());
    assert!(ensure_plan_permission("auto-edit", &write_plan, true).is_ok());
    apply_effects_to_plan(&mut write_plan, &effects);
    assert_eq!(write_plan.nodes[0].model, "claude-opus-5-5-medium");
    Ok(())
}

#[test]
fn persisted_dag_result_recovers_resume_identity() {
    let value = json!({
        "jobs": [{
            "job_id": "dag-1",
            "task_type": "dag_ulw",
            "result": {"compiled_plan": {"goal": "resume exact campaign"}}
        }]
    });
    assert_eq!(
        latest_dag_checkpoint(&value),
        Some(("dag-1".to_owned(), "resume exact campaign".to_owned()))
    );
}

#[tokio::test]
async fn local_commands_apply_real_state_and_storage_behavior() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let permissions = command(
        workspace.path().to_path_buf(),
        state.path().to_path_buf(),
        "/permissions read-only".to_owned(),
    )
    .await?;
    assert!(
        permissions
            .iter()
            .any(|update| matches!(update, TuiUpdate::PermissionMode(mode) if mode == "read-only"))
    );
    let theme = command(
        workspace.path().to_path_buf(),
        state.path().to_path_buf(),
        "/theme cyber-matrix".to_owned(),
    )
    .await?;
    assert!(
        theme
            .iter()
            .any(|update| matches!(update, TuiUpdate::Theme(TuiTheme::CyberMatrix)))
    );
    let compact = command(
        workspace.path().to_path_buf(),
        state.path().to_path_buf(),
        "/compact".to_owned(),
    )
    .await?;
    assert!(
        compact
            .iter()
            .any(|update| matches!(update, TuiUpdate::CompactHistory))
    );
    let inbox = command(
        workspace.path().to_path_buf(),
        state.path().to_path_buf(),
        "/inbox".to_owned(),
    )
    .await?;
    assert!(
        inbox.iter().any(
            |update| matches!(update, TuiUpdate::View { title, .. } if title == "Handoff inbox")
        )
    );
    Ok(())
}

#[tokio::test]
async fn context_clears_on_contextless_response_and_model_change() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let state_root = tempfile::tempdir()?;
    let handler = AppHandler::new(
        Gateway::new(state_root.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    let (updates, mut receiver) = mpsc::channel(16);
    handler
        .apply_usage(
            &json!({"receipt":{
                "request_id":"codex-1",
                "tokens_in":10,
                "tokens_out":2,
                "context_window":1000,
                "current_context_tokens":250
            }}),
            &updates,
        )
        .await?;
    let first = receiver.recv().await.ok_or("missing token update")?;
    assert!(matches!(
        first,
        TuiUpdate::Tokens(TokenUsage {
            context_limit: Some(1000),
            current_context: Some(250),
            ..
        })
    ));
    handler
        .apply_usage(
            &json!({"receipt":{
                "request_id":"claude-1",
                "tokens_in":5,
                "tokens_out":1
            }}),
            &updates,
        )
        .await?;
    let second = receiver.recv().await.ok_or("missing contextless update")?;
    assert!(matches!(
        second,
        TuiUpdate::Tokens(TokenUsage {
            input: Some(15),
            output: Some(3),
            context_limit: None,
            current_context: None,
            usage_complete: true,
            ..
        })
    ));
    handler
        .apply_usage(
            &json!({"receipt":{
                "request_id":"codex-1",
                "tokens_in":10,
                "tokens_out":2,
                "context_window":1000,
                "current_context_tokens":250
            }}),
            &updates,
        )
        .await?;
    let duplicate = receiver.recv().await.ok_or("missing duplicate update")?;
    assert!(matches!(
        duplicate,
        TuiUpdate::Tokens(TokenUsage {
            input: Some(15),
            output: Some(3),
            ..
        })
    ));
    handler
        .apply_usage(
            &json!({"receipt":{
                "request_id":"codex-1",
                "attempt_count":2,
                "tokens_in":4,
                "tokens_out":1,
                "context_window":1000,
                "current_context_tokens":300
            }}),
            &updates,
        )
        .await?;
    let retry = receiver.recv().await.ok_or("missing retry usage update")?;
    assert!(matches!(
        retry,
        TuiUpdate::Tokens(TokenUsage {
            input: Some(19),
            output: Some(4),
            context_limit: Some(1000),
            current_context: Some(300),
            ..
        })
    ));
    handler
        .apply_usage(
            &json!({"receipt":{
                "request_id":"argon-unknown",
                "tokens_in":0,
                "tokens_out":0
            }}),
            &updates,
        )
        .await?;
    let partial = receiver
        .recv()
        .await
        .ok_or("missing partial usage update")?;
    assert!(matches!(
        partial,
        TuiUpdate::Tokens(TokenUsage {
            input: Some(19),
            output: Some(4),
            usage_complete: false,
            ..
        })
    ));
    handler
        .select_model("claude-sonnet-5-5-medium".to_owned(), updates)
        .await?;
    let state = handler.state.lock().await;
    assert_eq!(state.tokens_in, 19);
    assert_eq!(state.tokens_out, 4);
    assert_eq!(state.context_window, None);
    assert_eq!(state.current_context_tokens, None);
    Ok(())
}

#[tokio::test]
async fn heterogeneous_codex_role_defaults_and_file_scoped_titles_round_trip() -> crate::Result<()>
{
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let state_root = tempfile::tempdir()?;
    for file in ["alpha.rs", "beta.rs", "gamma.rs"] {
        std::fs::write(workspace.path().join(file), "pub fn item() {}\n")?;
    }
    let mut plan = DagPlanner::generate("refactor alpha, beta, and gamma", workspace.path())?;
    apply_effects_to_plan(&mut plan, &EffectConfig::default());
    let boundary = plan
        .nodes
        .iter()
        .find(|node| node.role == NodeRole::BoundaryMapper)
        .ok_or("missing boundary mapper")?;
    assert_eq!(boundary.model, "gpt-6-luna");
    assert_eq!(boundary.reasoning_effort, "max");
    let mut bugfix_plan = DagPlanner::generate("audit alpha and beta", workspace.path())?;
    apply_effects_to_plan(&mut bugfix_plan, &EffectConfig::default());
    let verifier = bugfix_plan
        .nodes
        .iter()
        .find(|node| node.role == NodeRole::ContractVerifier)
        .ok_or("missing contract verifier")?;
    assert_eq!(verifier.model, "gpt-6.1-sol");
    assert_eq!(verifier.reasoning_effort, "xhigh");
    let lanes = dag_lanes(&plan);
    assert!(lanes.iter().any(|lane| lane.title.contains("alpha.rs")));
    assert!(lanes.iter().any(|lane| lane.title.contains("beta.rs")));
    let request_nodes = dag_request_nodes(&bugfix_plan, "");
    assert!(
        request_nodes
            .iter()
            .any(|node| node["role"] == "reproduction_prober")
    );
    assert!(
        dag_request_nodes(&plan, "")
            .iter()
            .any(|node| node["role"] == "interface_contractor")
    );

    let handler = AppHandler::new(
        Gateway::new(state_root.path().to_path_buf()),
        workspace.path().to_path_buf(),
    )?;
    handler.state.lock().await.skills = core_registry(workspace.path(), home.path())?;
    let (sender, mut receiver) = mpsc::channel(16);
    handler
        .toggle_skill("load codegraph-operator".to_owned(), sender)
        .await?;
    let mut messages = String::new();
    while let Ok(update) = receiver.try_recv() {
        if let TuiUpdate::Transcript(entry) = update {
            messages.push_str(&entry.message);
            messages.push('\n');
        }
    }
    assert!(messages.contains("Loaded $codegraph-operator from"));
    Ok(())
}
