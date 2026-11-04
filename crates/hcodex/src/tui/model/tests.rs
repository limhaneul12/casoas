use super::*;

#[test]
fn inspector_pages_clamp_and_back_restores_the_previous_position() -> Result<()> {
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    app.apply_update(TuiUpdate::View {
        title: "Long prompt".to_owned(),
        body: (0..25).map(|line| format!("line {line}")).collect(),
    });
    for _ in 0..10 {
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))?;
    }
    assert_eq!(app.view_scroll(), 24);
    app.apply_update(TuiUpdate::View {
        title: "Next".to_owned(),
        body: vec!["single\nsecond".to_owned()],
    });
    assert_eq!(app.view_scroll(), 0);
    assert_eq!(app.current_view().map(|(_, body)| body.len()), Some(2));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    assert_eq!(app.view_scroll(), 24);
    for _ in 0..10 {
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE))?;
    }
    assert_eq!(app.view_scroll(), 0);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    assert_eq!(app.view_scroll(), 0);
    assert!(!app.should_quit());
    Ok(())
}

#[test]
fn contextual_completion_replaces_the_full_recipe_and_enter_executes_selection() -> Result<()> {
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("workspace"),
        TuiCatalog::command_catalog(),
        false,
    );
    app.handle_paste("/use a tail");
    app.cursor = "/use a".len();
    app.refresh_completions();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
    assert_eq!(app.input(), "/use argon tail");
    assert_eq!(app.cursor(), "/use argon".len());
    app.clear_input();
    app.handle_paste("/use o");
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?,
        Some(TuiAction::SelectModel("opus".to_owned()))
    );
    app.handle_paste("/permissions ");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
    let selected = app.completions()[app.completion_index()].value.clone();
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?,
        Some(action_from_input(&selected))
    );
    Ok(())
}

#[test]
fn completion_dismissal_history_and_unicode_edit_controls_match_terminal_contract() -> Result<()> {
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("workspace"),
        TuiCatalog::command_catalog(),
        false,
    );
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
    assert!(!app.completions().is_empty());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    assert!(app.completions().is_empty());
    assert!(!app.should_quit());
    app.handle_paste("첫째 둘째");
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))?;
    assert_eq!(app.input(), "첫째 ");
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))?;
    assert!(app.input().is_empty());
    app.handle_paste("a unique request");
    let _ = app.submit()?;
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))?;
    assert_eq!(app.input(), "a unique request");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
    assert!(app.input().is_empty());
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))?;
    assert_eq!(app.input(), "a unique request");
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))?;
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    assert!(!app.should_quit());
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?,
        Some(TuiAction::Back)
    );
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))?,
        Some(TuiAction::Quit)
    );
    Ok(())
}

#[test]
fn dag_mode_transition_is_bounded_and_plain_prompts_use_the_graph() -> Result<()> {
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), true);
    app.handle_paste("/dag-mode");
    assert_eq!(app.submit()?, Some(TuiAction::EnterDag(None)));
    assert_eq!(app.dag_transition(), Some((true, 0)));
    assert!(app.lanes().is_empty());
    assert_eq!(app.progress(), (0, 0));
    for _ in 0..35 {
        app.tick();
    }
    assert_eq!(app.dag_transition(), None);
    app.handle_paste("audit Rust contracts");
    assert_eq!(
        app.submit()?,
        Some(TuiAction::AutoDag("audit Rust contracts".to_owned()))
    );
    app.handle_paste("/ask explain this");
    assert_eq!(
        app.submit()?,
        Some(TuiAction::SubmitPrompt("explain this".to_owned()))
    );
    app.apply_local_action(&TuiAction::ExitDag);
    assert_eq!(app.dag_transition(), Some((false, 0)));
    for _ in 0..15 {
        app.tick();
    }
    assert_eq!(app.dag_transition(), None);
    app.handle_paste("explain Rust");
    assert_eq!(
        app.submit()?,
        Some(TuiAction::SubmitPrompt("explain Rust".to_owned()))
    );
    let mut static_app =
        TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    static_app.apply_local_action(&TuiAction::EnterDag(None));
    assert!(static_app.dag_mode());
    assert_eq!(static_app.dag_transition(), None);
    Ok(())
}

#[test]
fn active_lane_clock_advances_through_verification_and_freezes_on_settlement() {
    let mut app = TuiApp::new(PathBuf::from("workspace"), TuiCatalog::default());
    app.apply_update(TuiUpdate::DagSnapshot(vec![DagLaneView {
        id: "live".to_owned(),
        layer: 0,
        dependencies: Vec::new(),
        title: "Live".to_owned(),
        summary: String::new(),
        role: "audit".to_owned(),
        model: "gpt-6.1-sol".to_owned(),
        effort: "medium".to_owned(),
        state: LaneState::Waiting,
        note: String::new(),
        elapsed: None,
    }]));
    for state in [LaneState::Running, LaneState::Stopped] {
        app.apply_update(TuiUpdate::LaneState {
            node_id: "live".to_owned(),
            state,
            note: String::new(),
            elapsed: None,
        });
        app.tick();
        assert!(app.lanes()[0].elapsed.is_some());
    }
    app.apply_update(TuiUpdate::LaneState {
        node_id: "live".to_owned(),
        state: LaneState::Accepted,
        note: String::new(),
        elapsed: Some(Duration::from_secs(7)),
    });
    app.tick();
    assert_eq!(app.lanes()[0].elapsed, Some(Duration::from_secs(7)));
    assert_eq!(app.progress(), (1, 1));
    app.apply_update(TuiUpdate::DagInterrupted("late error".to_owned()));
    assert_eq!(app.lanes()[0].state, LaneState::Accepted);
    app.apply_update(TuiUpdate::LaneState {
        node_id: "live".to_owned(),
        state: LaneState::Running,
        note: String::new(),
        elapsed: None,
    });
    app.apply_update(TuiUpdate::DagInterrupted("storage unavailable".to_owned()));
    assert_eq!(app.lanes()[0].state, LaneState::Unverified);
    assert!(app.lanes()[0].note.contains("requires reconciliation"));
    assert_eq!(app.progress(), (0, 1));
}

const PYTHON_SLASH_FIXTURE: &[(&str, &str)] = &[
    ("/help", "help"),
    ("/ask", "submit_prompt"),
    ("/prompt", "submit_prompt"),
    ("/sonnet", "select_and_prompt"),
    ("/opus", "select_and_prompt"),
    ("/argon", "select_and_prompt"),
    ("/flash", "select_and_prompt"),
    ("/use", "select_model"),
    ("/model", "select_model"),
    ("/skills", "show_skills"),
    ("/skill", "toggle_skill"),
    ("/dag-mode", "enter_dag"),
    ("/dag-exit", "exit_dag"),
    ("/dag-ulw", "run_dag"),
    ("/dag", "run_dag"),
    ("/dag-run", "run_dag"),
    ("/effect", "configure_effect"),
    ("/effort", "configure_effect"),
    ("/reasoning", "configure_effect"),
    ("/dag-effect", "configure_effect"),
    ("/prompts", "show_prompts"),
    ("/promt", "show_prompts"),
    ("/trace", "show_prompts"),
    ("/back", "back"),
    ("/prev", "back"),
    ("/compact", "compact"),
    ("/clear", "clear"),
    ("/diff", "show_diff"),
    ("/review", "review"),
    ("/permissions", "set_permissions"),
    ("/approvals", "set_permissions"),
    ("/resume", "resume"),
    ("/triad", "run_dag"),
    ("/models", "show_models"),
    ("/lanes", "show_models"),
    ("/omh", "show_overview"),
    ("/board", "show_overview"),
    ("/todo", "show_overview"),
    ("/plan", "show_overview"),
    ("/status", "show_status"),
    ("/inbox", "show_inbox"),
    ("/jobs", "show_status"),
    ("/theme", "set_theme"),
    ("/quit", "quit"),
    ("/exit", "quit"),
];

#[test]
fn skill_labels_wait_for_registry_confirmation() -> Result<()> {
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    let before = app.active_skills().to_vec();
    app.handle_paste("/skills not-registered");
    assert_eq!(
        app.submit()?,
        Some(TuiAction::ToggleSkill("not-registered".to_owned()))
    );
    assert_eq!(app.active_skills(), before);
    app.apply_update(TuiUpdate::ActiveSkills(vec![
        "codex-heterarchy".to_owned(),
        "codegraph-operator".to_owned(),
        "codebase-refactor-guardian".to_owned(),
    ]));
    assert_eq!(app.active_skills().len(), 3);
    app.handle_paste("/skills off");
    assert_eq!(
        app.submit()?,
        Some(TuiAction::ToggleSkill("off".to_owned()))
    );
    assert_eq!(app.active_skills().len(), 3);
    app.apply_update(TuiUpdate::ActiveSkills(Vec::new()));
    assert!(app.active_skills().is_empty());
    Ok(())
}

#[test]
fn aliases_and_commands_map_to_typed_actions() {
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("workspace"),
        TuiCatalog::command_catalog(),
        false,
    );
    app.handle_paste("/STATUS");
    assert!(matches!(app.submit(), Ok(Some(TuiAction::ShowStatus))));
    assert_eq!(
        action_from_input("/SKILLS core"),
        TuiAction::ToggleSkill("core".to_owned())
    );
    assert_eq!(action_from_input("/DAG-MODE ON"), TuiAction::EnterDag(None));
    assert_eq!(
        action_from_input("/ASK Read @src/Case.rs"),
        TuiAction::SubmitPrompt("Read @src/Case.rs".to_owned())
    );
    assert_eq!(
        action_from_input("/triad goal"),
        TuiAction::RunDag("goal".to_owned())
    );
    assert_eq!(
        action_from_input("/triad-review goal"),
        TuiAction::RunTriad("goal".to_owned())
    );
    assert_eq!(
        action_from_input("/skills core"),
        TuiAction::ToggleSkill("core".to_owned())
    );
    assert_eq!(
        action_from_input("/skills default"),
        TuiAction::ToggleSkill("default".to_owned())
    );
    assert_eq!(action_from_input("/skills"), TuiAction::ShowSkills);
    assert_eq!(
        action_from_input("/promt n1"),
        TuiAction::ShowPrompts(Some("n1".to_owned()))
    );
    assert_eq!(
        action_from_input("/dag-ulw migrate all"),
        TuiAction::RunDag("migrate all".to_owned())
    );
    assert_eq!(
        action_from_input("/dag-plan inspect only"),
        TuiAction::PlanDag("inspect only".to_owned())
    );
    assert_eq!(
        action_from_input("/dag --plan-only inspect only"),
        TuiAction::PlanDag("inspect only".to_owned())
    );
    assert_eq!(
        action_from_input("$codegraph-operator"),
        TuiAction::SubmitPrompt("$codegraph-operator".to_owned())
    );
    assert_eq!(
        action_from_input("/argon inspect routing"),
        TuiAction::SelectAndPrompt {
            model: "gemini-4-argon-high".to_owned(),
            prompt: "inspect routing".to_owned(),
        }
    );
}

#[test]
fn frozen_python_slash_fixture_maps_without_falling_through() -> crate::Result<()> {
    for (command, expected_kind) in PYTHON_SLASH_FIXTURE {
        // Bare /skills still lists; its now-supported arguments are
        // covered by the core/default selection regression above.
        let input = if *command == "/skills" {
            (*command).to_owned()
        } else {
            format!("{command} goal")
        };
        let encoded = serde_json::to_value(action_from_input(&input))?;
        assert_eq!(encoded["kind"], *expected_kind, "{command}");
    }
    Ok(())
}

#[test]
fn compact_reduces_local_views_without_fabricating_token_reclamation() {
    let mut app = TuiApp::new(PathBuf::from("."), TuiCatalog::default());
    app.apply_update(TuiUpdate::Tokens(TokenUsage {
        input: Some(100),
        output: Some(50),
        context_limit: None,
        current_context: None,
        usage_complete: true,
        ..TokenUsage::default()
    }));
    app.apply_update(TuiUpdate::View {
        title: "one".to_owned(),
        body: vec!["one".to_owned()],
    });
    app.apply_update(TuiUpdate::View {
        title: "two".to_owned(),
        body: vec!["two".to_owned()],
    });
    for index in 0..60 {
        app.apply_update(TuiUpdate::Transcript(TranscriptEntry::root(format!(
            "line {index}"
        ))));
    }
    app.apply_update(TuiUpdate::CompactHistory);
    assert_eq!(app.view_history.len(), 1);
    assert_eq!(app.transcript.len(), 50);
    assert_eq!(app.tokens.input, Some(100));
    assert_eq!(app.tokens.output, Some(50));
}

#[test]
fn unknown_slash_never_falls_through_to_provider_prompt() {
    assert_eq!(
        action_from_input("/not-a-command secret"),
        TuiAction::UnknownCommand("/not-a-command".to_owned())
    );
}

#[test]
fn hard_newline_and_paste_preserve_one_submit_boundary() -> crate::Result<()> {
    let mut app = TuiApp::new(PathBuf::from("."), TuiCatalog::default());
    assert!(
        app.handle_key(KeyEvent::new(KeyCode::Char('한'), KeyModifiers::NONE))?
            .is_none()
    );
    assert!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))?
            .is_none()
    );
    app.handle_paste("둘\r\n셋\t🙂\0\u{1b}");
    assert_eq!(app.input(), "한\n둘\n셋\t🙂");
    assert_eq!(app.cursor(), app.input().len());
    let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    assert_eq!(
        action,
        Some(TuiAction::SubmitPrompt("한\n둘\n셋\t🙂".to_owned()))
    );
    Ok(())
}

#[test]
fn alt_enter_inserts_newline_without_submit() -> crate::Result<()> {
    let mut app = TuiApp::new(PathBuf::from("."), TuiCatalog::default());
    assert!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))?
            .is_none()
    );
    assert_eq!(app.input(), "\n");
    Ok(())
}

#[test]
fn absent_usage_is_reported_as_unknown() {
    assert_eq!(
        TokenUsage::default().status(),
        "Context unknown · tokens unknown"
    );
    assert!(!TokenUsage::default().status().contains("100%"));
    let verified = TokenUsage {
        input: Some(100),
        output: Some(50),
        context_limit: Some(1_000),
        current_context: Some(250),
        usage_complete: true,
        ..TokenUsage::default()
    };
    assert!(verified.status().contains("Context 75.0% left (ctx 25.0%)"));
    let byte_budget = TokenUsage {
        request_input_bytes: Some(25_600),
        request_limit_bytes: Some(256_000),
        ..TokenUsage::default()
    };
    assert!(
        byte_budget.context_status().contains("90.0% left (10.0%)"),
        "byte-budget context status must report percentage: {}",
        byte_budget.context_status()
    );
}

#[test]
fn history_recall_does_not_open_completion_popup_while_typing_autocompletes() -> crate::Result<()> {
    let mut app = TuiApp::new(PathBuf::from("."), TuiCatalog::command_catalog());
    assert!(
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?
            .is_none()
    );
    assert!(app.completions().is_empty());
    for ch in "/qu".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
    }
    assert!(!app.completions().is_empty());
    let _ = app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
    assert_eq!(app.input(), "/quit");
    let _ = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    assert_eq!(app.input(), "");
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))?;
    assert_eq!(app.input(), "/quit");
    assert!(
        app.completions().is_empty(),
        "recalling input history with Up must not spam the completion popup"
    );
    Ok(())
}
