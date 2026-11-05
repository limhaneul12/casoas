use super::*;
use crate::tui::{DagLaneView, LaneState, TuiCatalog, TuiUpdate};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::fs;
use std::path::PathBuf;

#[test]
fn research_and_planning_render_their_own_read_only_workspaces() -> crate::Result<()> {
    use crate::tui::{TuiAction, WorkspaceArtifactView, WorkspaceUpdate};
    use crate::workspaces::{WorkspaceKind, WorkspaceRunStatus};
    for kind in [WorkspaceKind::Research, WorkspaceKind::Planning] {
        for (width, height) in [(128, 36), (80, 24), (64, 18)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            let mut app =
                TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
            app.apply_update(TuiUpdate::ActiveModel("direct-building-selection".into()));
            app.apply_update(TuiUpdate::ValidatedAction(TuiAction::ToggleDag));
            app.apply_update(TuiUpdate::PermissionMode("auto-edit".into()));
            app.apply_update(TuiUpdate::ValidatedAction(TuiAction::SelectWorkspace(kind)));
            terminal.draw(|frame| draw(frame, &app))?;
            let idle = buffer_text(terminal.backend().buffer());
            let command = if kind == WorkspaceKind::Research {
                "/research --"
            } else {
                "/planning --"
            };
            assert!(idle.contains(command), "{kind:?} {width}x{height}: {idle}");
            for misleading in [
                "DAG topology",
                "No graph loaded",
                "AST & code stream",
                "TRI-POLAR",
                "auto-edit",
                "accept all",
                "direct-building-selection",
            ] {
                assert!(
                    !idle.contains(misleading),
                    "{kind:?} {width}x{height} still shows {misleading}: {idle}"
                );
            }
            assert!(idle.contains("read-only"));
            assert!(idle.contains("stage models · see receipts"));
            app.apply_update(TuiUpdate::WorkspaceStarted {
                run_id: "flow-evidence".into(),
                selected: vec![kind],
                objective: "Compare storage contracts".into(),
            });
            app.apply_update(TuiUpdate::WorkspaceScoped {
                run_id: "flow-evidence".into(),
                workspace: kind,
                update: WorkspaceUpdate::Status {
                    status: WorkspaceRunStatus::Running,
                    detail: "Observed stage work".into(),
                },
            });
            app.apply_update(TuiUpdate::WorkspaceScoped {
                run_id: "flow-evidence".into(),
                workspace: kind,
                update: WorkspaceUpdate::Artifact(WorkspaceArtifactView {
                    artifact_id: "artifact-original".into(),
                    digest: "sha256:observed".into(),
                    label: "Observed artifact".into(),
                    ready: false,
                }),
            });
            app.handle_paste("workspace draft");
            terminal.draw(|frame| draw(frame, &app))?;
            let running = buffer_text(terminal.backend().buffer());
            assert!(running.contains("Compare storage contracts"));
            assert!(running.contains("artifact-original"));
            assert!(running.contains("RUNNING"));
            assert!(running.contains("workspace draft"));
            let areas = workbench_areas(Rect::new(0, 0, width, height), 0);
            let composer = (0..width)
                .map(|x| terminal.backend().buffer()[(x, areas.composer.y + 1)].symbol())
                .collect::<String>();
            assert!(composer.contains("workspace draft"));
            let handoff = if kind == WorkspaceKind::Research {
                "/planning --from artifact-original"
            } else {
                "/building --plan artifact-original"
            };
            assert!(
                !running.contains(handoff),
                "running artifact offered handoff"
            );
            app.apply_update(TuiUpdate::WorkspaceScoped {
                run_id: "flow-evidence".into(),
                workspace: kind,
                update: WorkspaceUpdate::Artifact(WorkspaceArtifactView {
                    artifact_id: "artifact-original".into(),
                    digest: "sha256:observed".into(),
                    label: "Observed artifact".into(),
                    ready: kind == WorkspaceKind::Planning,
                }),
            });
            app.apply_update(TuiUpdate::WorkspaceScoped {
                run_id: "flow-evidence".into(),
                workspace: kind,
                update: WorkspaceUpdate::Status {
                    status: if kind == WorkspaceKind::Research {
                        WorkspaceRunStatus::IncompleteEvidence
                    } else {
                        WorkspaceRunStatus::BuildReady
                    },
                    detail: "Observed eligible artifact".into(),
                },
            });
            terminal.draw(|frame| draw(frame, &app))?;
            if width == 128 {
                let ready = buffer_text(terminal.backend().buffer());
                assert!(
                    ready.contains(handoff),
                    "{kind:?} eligible handoff missing: {ready}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn a_multiline_provider_reply_taller_than_the_log_still_shows_its_tail() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    app.apply_update(TuiUpdate::Transcript(crate::tui::TranscriptEntry {
        depth: 1,
        message: format!(
            "{}\nVISIBLE_PROVIDER_REPLY_TAIL",
            "source response line\n".repeat(60)
        ),
    }));
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(buffer_text(terminal.backend().buffer()).contains("VISIBLE_PROVIDER_REPLY_TAIL"));
    Ok(())
}

#[test]
fn a_soft_wrapped_single_line_can_be_paged_to_its_actual_tail() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    app.set_viewport_width(80);
    app.apply_update(TuiUpdate::View {
        title: "Long context".to_owned(),
        body: vec![format!("{}TAIL_CONTEXT_PROOF", "x".repeat(5000))],
    });
    for _ in 0..100 {
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))?;
    }
    assert!(app.view_scroll() > 10);
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(buffer_text(terminal.backend().buffer()).contains("TAIL_CONTEXT_PROOF"));
    app.set_viewport_width(160);
    assert!(app.view_scroll() < 80);
    Ok(())
}

#[test]
fn inspector_scrolls_without_covering_composer_context_or_completion() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("workspace"),
        TuiCatalog::command_catalog(),
        false,
    );
    app.apply_update(TuiUpdate::View {
        title: "Prompt inspector".to_owned(),
        body: (0..60)
            .map(|index| format!("source-line-{index:03}"))
            .collect(),
    });
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(buffer_text(terminal.backend().buffer()).contains("source-line-000"));
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))?;
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(app.view_scroll() > 0);
    assert!(!buffer_text(terminal.backend().buffer()).contains("source-line-000"));
    app.handle_paste("/eff");
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("completion"));
    assert!(text.contains("Context unknown"));
    let area = workbench_areas(Rect::new(0, 0, 80, 24), 0).composer;
    let input = (0..80)
        .map(|x| terminal.backend().buffer()[(x, area.y + 1)].symbol())
        .collect::<String>();
    assert!(input.contains("/eff"));
    Ok(())
}

#[test]
fn context_and_active_skills_remain_visible_at_eighty_columns() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    app.apply_update(TuiUpdate::Tokens(crate::tui::TokenUsage {
        input: Some(100),
        output: Some(50),
        context_limit: Some(1_000),
        current_context: Some(250),
        usage_complete: true,
        ..crate::tui::TokenUsage::default()
    }));
    app.apply_update(TuiUpdate::ActiveSkills(vec![
        "codex-heterarchy".to_owned(),
        "codegraph-operator".to_owned(),
        "codebase-refactor-guardian".to_owned(),
    ]));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("Context 75.0% left (ctx 25.0%)"));
    assert!(text.contains("100 in · 50 out"));
    assert!(text.contains("$codebase-refactor-guardian"));
    app.apply_update(TuiUpdate::Tokens(crate::tui::TokenUsage {
        request_input_bytes: Some(1536),
        request_limit_bytes: Some(256_000),
        ..crate::tui::TokenUsage::default()
    }));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("Context input 1536/256000 B"));
    assert!(text.contains("model window unreported"));
    assert!(!text.contains("100% left"));
    Ok(())
}

#[test]
fn workflow_rail_remains_visible_and_compact_without_covering_composer() -> crate::Result<()> {
    for (width, height) in [(128, 36), (80, 24), (48, 16)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        let mut app =
            TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
        app.apply_update(TuiUpdate::WorkspaceStarted {
            run_id: "flow-render".to_owned(),
            selected: vec![
                crate::workspaces::WorkspaceKind::Research,
                crate::workspaces::WorkspaceKind::Planning,
                crate::workspaces::WorkspaceKind::Building,
            ],
            objective: "render rail".to_owned(),
        });
        app.handle_paste("composer draft");
        terminal.draw(|frame| draw(frame, &app))?;
        let text = buffer_text(terminal.backend().buffer());
        if width >= 72 {
            assert!(text.contains("Research & Search"));
            assert!(text.contains("Planning"));
            assert!(text.contains("Building"));
        } else {
            assert!(text.contains("R◐"));
            assert!(text.contains("P◐"));
            assert!(text.contains("B◐"));
        }
        assert!(text.contains("composer draft"));
        let areas = workbench_areas(Rect::new(0, 0, width, height), 0);
        assert_eq!(areas.workspace_rail.y, areas.header.y + 1);
        assert!(areas.workspace_rail.y < areas.composer.y);
    }
    Ok(())
}

#[test]
fn completion_selection_scrolls_into_view_and_ghost_is_not_input() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("workspace"),
        TuiCatalog::command_catalog(),
        false,
    );
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
    let count = app.completions().len();
    assert!(count > 10);
    for _ in 1..count {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
    }
    let selected = app.completions()[app.completion_index()]
        .display_label
        .clone();
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(buffer_text(terminal.backend().buffer()).contains(&selected));
    assert!(app.input().is_empty());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    app.handle_paste("/eff");
    terminal.draw(|frame| draw(frame, &app))?;
    assert_eq!(app.input(), "/eff");
    assert!(app.completion_ghost().is_some());
    let areas = workbench_areas(Rect::new(0, 0, 80, 24), 0);
    let composer = terminal.backend().buffer();
    let row = (0..80)
        .map(|x| composer[(x, areas.composer.y + 1)].symbol())
        .collect::<String>();
    assert!(row.contains("/effect"));
    Ok(())
}

#[test]
fn boxed_graph_shows_actual_fanout_and_fanin_arrows() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(160, 48))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    let mut nodes = vec![
        lane("a", "Argon", LaneState::Accepted),
        lane("b", "Claude", LaneState::Running),
        lane("c", "Codex", LaneState::Stopped),
        lane("d", "Argon fan-in", LaneState::Waiting),
    ];
    nodes[1].layer = 1;
    nodes[1].dependencies = vec!["a".to_owned()];
    nodes[2].layer = 1;
    nodes[2].dependencies = vec!["a".to_owned()];
    nodes[3].layer = 2;
    nodes[3].dependencies = vec!["b".to_owned(), "c".to_owned()];
    app.apply_update(TuiUpdate::DagSnapshot(nodes));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.matches('▶').count() >= 3);
    assert!(text.contains("← #2,#3"));
    assert!(text.contains("STOPPED · reported done"));
    assert!(text.contains("SETTLED · verified"));
    assert_eq!(app.progress(), (1, 4));
    Ok(())
}

#[test]
fn multi_wave_bipartite_frontiers_render_boxed_cards_and_vertical_bus() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(160, 48))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    let mut nodes = vec![
        lane("n1_spec", "Topology spec", LaneState::Accepted),
        lane("n1_map", "Boundary map", LaneState::Accepted),
        lane("n2_craft", "Interface craft", LaneState::Running),
        lane("n2_audit", "Red-team audit", LaneState::Stopped),
        lane("n3_verify", "Integration fan-in", LaneState::Waiting),
    ];
    nodes[2].layer = 1;
    nodes[2].dependencies = vec!["n1_spec".to_owned(), "n1_map".to_owned()];
    nodes[3].layer = 1;
    nodes[3].dependencies = vec!["n1_spec".to_owned(), "n1_map".to_owned()];
    nodes[4].layer = 2;
    nodes[4].dependencies = vec!["n2_craft".to_owned(), "n2_audit".to_owned()];
    app.apply_update(TuiUpdate::DagSnapshot(nodes));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("layer 1 · #1"));
    assert!(text.contains("layer 2 · #3"));
    assert!(text.contains("layer 3 · #5"));
    assert!(text.matches('▶').count() >= 3);
    assert!(!text.contains("Links:"));
    Ok(())
}

#[test]
fn compact_diamond_keeps_branching_join_and_active_rows_at_terminal_sizes() -> crate::Result<()> {
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    let mut nodes = vec![
        lane("a", "Argon", LaneState::Accepted),
        lane("b", "Claude", LaneState::Running),
        lane("c", "Codex", LaneState::Rejected),
        lane("d", "Audit", LaneState::Waiting),
        lane("e", "Fan-in", LaneState::Waiting),
    ];
    for node in &mut nodes[1..4] {
        node.layer = 1;
        node.dependencies = vec!["a".to_owned()];
    }
    nodes[4].layer = 2;
    nodes[4].dependencies = vec!["b".to_owned(), "c".to_owned(), "d".to_owned()];
    app.apply_update(TuiUpdate::DagSnapshot(nodes));
    for (width, height) in [(100, 32), (80, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| draw(frame, &app))?;
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("#1 ─┬→ #2 ─┬→ #5"));
        assert!(text.contains("├→ #3 ─┤"));
        assert!(text.contains("└→ #4 ─┘"));
        assert!(text.contains("● Running"));
        assert!(text.contains("Verification rejected"));
        assert!(text.contains("composer"));
        let areas = workbench_areas(Rect::new(0, 0, width, height), app.lanes().len());
        assert_eq!(areas.composer.bottom(), areas.status.top());
        assert_eq!(areas.status.bottom(), height);
    }
    Ok(())
}

#[test]
fn crossing_and_skipping_dependencies_use_unambiguous_parent_pins() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(160, 48))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    let mut nodes = vec![
        lane("a", "A", LaneState::Waiting),
        lane("b", "B", LaneState::Waiting),
        lane("c", "C", LaneState::Waiting),
        lane("d", "D", LaneState::Waiting),
    ];
    nodes[2].layer = 1;
    nodes[2].dependencies = vec!["b".to_owned()];
    nodes[3].layer = 1;
    nodes[3].dependencies = vec!["a".to_owned()];
    let positions = BTreeMap::from([
        ("a".to_owned(), Rect::new(0, 0, 20, 6)),
        ("b".to_owned(), Rect::new(0, 8, 20, 6)),
        ("c".to_owned(), Rect::new(25, 0, 20, 6)),
        ("d".to_owned(), Rect::new(25, 8, 20, 6)),
    ]);
    let edges = dependency_edges(&nodes, &positions);
    assert_eq!(
        edges
            .iter()
            .map(|edge| (edge.parent, edge.target.id.as_str()))
            .collect::<Vec<_>>(),
        [("b", "c"), ("a", "d")]
    );
    assert!(ambiguous_edges(&edges));
    app.apply_update(TuiUpdate::DagSnapshot(nodes.clone()));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("#3 ← #2"));
    assert!(text.contains("#4 ← #1"));
    assert!(!text.contains("#3 ← #1"));
    assert!(text.contains("Links: #2→#3 #1→#4"));
    nodes[2].layer = 2;
    app.apply_update(TuiUpdate::DagSnapshot(nodes));
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(buffer_text(terminal.backend().buffer()).contains("#3 ← #2"));
    Ok(())
}

#[test]
fn compact_long_ids_keep_all_parent_pins_and_active_stage_visible() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    let mut nodes = vec![
        lane("n1_topology_spec", "A", LaneState::Accepted),
        lane("n2_interface_craft", "B", LaneState::Accepted),
        lane("n3_red_team_security", "C", LaneState::Accepted),
        lane("n4_integration_fan_in", "D", LaneState::Running),
    ];
    nodes[3].layer = 2;
    nodes[3].dependencies = nodes[..3].iter().map(|lane| lane.id.clone()).collect();
    app.apply_update(TuiUpdate::DagSnapshot(nodes));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("#4 ← #1,#2,#3"));
    assert!(text.contains("n4_integration_fan_in ● Running"));
    assert_eq!(app.progress(), (3, 4));
    let mut long_nodes = app.lanes().to_vec();
    long_nodes[3].id = "long".repeat(16);
    long_nodes[3].state = LaneState::Stopped;
    long_nodes[3].elapsed = Some(std::time::Duration::from_millis(123_400));
    app.apply_update(TuiUpdate::DagSnapshot(long_nodes));
    terminal.draw(|frame| draw(frame, &app))?;
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("#4 ← #1,#2,#3"));
    assert!(text.contains("STOPPED · reported done 123.4s"));
    Ok(())
}

#[test]
fn dag_mode_badge_sweeps_without_creating_graph_or_progress() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(100, 32))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), true);
    app.handle_paste("/dag-mode on");
    let _action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    terminal.draw(|frame| draw(frame, &app))?;
    let first = terminal.backend().buffer().clone();
    for _ in 0..10 {
        app.tick();
    }
    terminal.draw(|frame| draw(frame, &app))?;
    let second = terminal.backend().buffer();
    assert!(buffer_text(second).contains("D A G - H E T E R A R C H Y"));
    assert_ne!(first, *second);
    assert_eq!(buffer_text(&first), buffer_text(second));
    assert!(app.lanes().is_empty());
    assert_eq!(app.progress(), (0, 0));
    let preview = buffer_text(second);
    assert!(preview.contains("Routing preview · not run"));
    assert!(preview.contains("Claude · craft"));
    assert!(preview.contains("Codex · audit"));
    assert!(preview.contains("Argon · verify"));
    Ok(())
}

#[test]
fn static_motion_and_mode_off_frames_stay_still_on_small_terminal() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(24, 12))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
    app.handle_paste("/dag-mode on");
    let _action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    terminal.draw(|frame| draw(frame, &app))?;
    let first = terminal.backend().buffer().clone();
    for _ in 0..40 {
        app.tick();
    }
    terminal.draw(|frame| draw(frame, &app))?;
    assert_eq!(first, *terminal.backend().buffer());
    assert!(buffer_text(&first).contains("DAG"));
    app.handle_paste("/dag-mode off");
    let _action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    terminal.draw(|frame| draw(frame, &app))?;
    let off = terminal.backend().buffer().clone();
    for _ in 0..40 {
        app.tick();
    }
    terminal.draw(|frame| draw(frame, &app))?;
    assert_eq!(off, *terminal.backend().buffer());
    assert!(!buffer_text(&off).contains("D A G - H E T E R A R C H Y"));
    assert!(!app.dag_mode());
    Ok(())
}

#[test]
fn activity_motion_keeps_running_stopped_and_settled_evidence_distinct() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), true);
    app.apply_update(TuiUpdate::DagSnapshot(vec![
        lane("running", "Audit", LaneState::Running),
        lane("stopped", "Verify", LaneState::Stopped),
        lane("settled", "Done", LaneState::Accepted),
    ]));
    terminal.draw(|frame| draw(frame, &app))?;
    let first = terminal.backend().buffer().clone();
    for _ in 0..8 {
        app.tick();
    }
    terminal.draw(|frame| draw(frame, &app))?;
    let second = terminal.backend().buffer();
    assert_ne!(first, *second);
    let text = buffer_text(second);
    assert!(text.contains("● Running"));
    assert!(text.contains("STOPPED · reported done"));
    assert!(text.contains("SETTLED · verified"));
    assert_eq!(app.progress(), (1, 3));
    assert_eq!(lane_activity(&app.lanes()[2], &app), "");
    Ok(())
}

#[test]
fn small_terminal_keeps_composer_and_status_at_bottom() {
    let area = Rect::new(0, 0, 40, 8);
    let regions = workbench_areas(area, 0);
    assert_eq!(regions.status.bottom(), area.bottom());
    assert_eq!(regions.composer.bottom(), regions.status.top());
    assert!(regions.composer.height >= 2);
}

#[test]
fn narrow_terminal_keeps_active_and_failed_lanes_visible() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut app = TuiApp::new(PathBuf::from("workspace"), TuiCatalog::default());
    let mut lanes = (0..8)
        .map(|index| {
            let mut item = lane(&format!("node{index}"), "Lane", LaneState::Waiting);
            item.layer = index / 2;
            item
        })
        .collect::<Vec<_>>();
    lanes[7].state = LaneState::Running;
    lanes[6].state = LaneState::Rejected;
    app.apply_update(TuiUpdate::DagSnapshot(lanes));
    app.apply_update(TuiUpdate::PermissionMode("read-only".to_owned()));
    terminal.draw(|frame| draw(frame, &app))?;
    let snapshot = buffer_text(terminal.backend().buffer());
    assert!(snapshot.contains("node7"));
    assert!(snapshot.contains("node6"));
    assert!(snapshot.contains("Running"));
    assert!(snapshot.contains("read-only"));
    Ok(())
}

#[test]
fn parallel_frontier_uses_compact_rows_when_card_height_hides_states() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(100, 32))?;
    let mut app = TuiApp::new(PathBuf::from("workspace"), TuiCatalog::default());
    let mut lanes = (0..5)
        .map(|index| {
            let mut item = lane(&format!("node{index}"), "Lane", LaneState::Waiting);
            item.layer = match index {
                0 => 0,
                4 => 2,
                _ => 1,
            };
            item
        })
        .collect::<Vec<_>>();
    lanes[2].state = LaneState::Running;
    app.apply_update(TuiUpdate::DagSnapshot(lanes));
    terminal.draw(|frame| draw(frame, &app))?;
    let snapshot = buffer_text(terminal.backend().buffer());
    assert!(snapshot.contains("L2 node2 ● Running"));
    app.apply_update(TuiUpdate::LaneState {
        node_id: "node2".to_owned(),
        state: LaneState::Rejected,
        note: "provider capacity failure".to_owned(),
        elapsed: None,
    });
    terminal.draw(|frame| draw(frame, &app))?;
    assert!(buffer_text(terminal.backend().buffer()).starts_with("Unverified"));
    Ok(())
}

#[test]
fn unicode_content_renders_without_overflow() -> crate::Result<()> {
    let backend = TestBackend::new(42, 12);
    let mut terminal = Terminal::new(backend)?;
    let app = TuiApp::new(PathBuf::from("한글-워크스페이스"), TuiCatalog::default());
    terminal.draw(|frame| draw(frame, &app))?;
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer.area.width, 42);
    assert_eq!(buffer.area.height, 12);
    Ok(())
}

#[test]
fn fit_uses_display_width_for_hangul() {
    assert!(UnicodeWidthStr::width(fit("한글입니다", 6).as_str()) <= 6);
}

#[test]
fn full_workbench_snapshot_distinguishes_reported_and_verified() -> crate::Result<()> {
    let backend = TestBackend::new(128, 36);
    let mut terminal = Terminal::new(backend)?;
    let mut app = TuiApp::new(
        PathBuf::from("intelligent-oppenheimer"),
        TuiCatalog::default(),
    );
    let mut long_goal = lane("n1", "Boundary map", LaneState::Stopped);
    long_goal.summary = "long actual goal ".repeat(100);
    app.apply_update(TuiUpdate::DagSnapshot(vec![
        long_goal,
        lane("n2", "Contract verify", LaneState::Accepted),
    ]));
    terminal.draw(|frame| draw(frame, &app))?;
    let snapshot = buffer_text(terminal.backend().buffer());
    assert_eq!(snapshot.matches("Same frontier").count(), 1);
    assert!(snapshot.contains("STOPPED · reported done"));
    assert!(snapshot.contains("SETTLED · verified"));
    assert!(snapshot.contains("composer"));
    let artifact_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&artifact_dir)?;
    fs::write(artifact_dir.join("tui-full-evidence.txt"), snapshot)?;
    Ok(())
}

#[test]
fn theme_update_changes_rendered_header_accent() -> crate::Result<()> {
    let backend = TestBackend::new(80, 18);
    let mut terminal = Terminal::new(backend)?;
    let mut app = TuiApp::new(PathBuf::from("workspace"), TuiCatalog::default());
    app.apply_update(TuiUpdate::Theme(TuiTheme::RoyalPrism));
    terminal.draw(|frame| draw(frame, &app))?;
    assert_eq!(terminal.backend().buffer()[(0, 0)].fg, Color::Magenta);
    Ok(())
}

#[test]
fn multiline_composer_tracks_unicode_cursor_and_scrolls_to_active_line() {
    let input = "첫째\n둘째\n셋\t🙂";
    let viewport = composer_view(input, input.len(), 2, 40);
    assert_eq!(viewport.text, "  둘째\n  셋    🙂");
    assert_eq!(viewport.cursor_row, 1);
    assert_eq!(viewport.cursor_column, 10);
}

#[test]
fn long_wide_line_soft_wraps_and_test_backend_places_cursor() -> crate::Result<()> {
    let backend = TestBackend::new(16, 10);
    let mut terminal = Terminal::new(backend)?;
    let mut app = TuiApp::new(PathBuf::from("workspace"), TuiCatalog::default());
    app.handle_paste(&"한".repeat(20));
    terminal.draw(|frame| draw(frame, &app))?;
    let position = terminal.get_cursor_position()?;
    assert_eq!(position.x, 7);
    assert_eq!(position.y, 6);
    let viewport = composer_view(app.input(), app.cursor(), 2, 14);
    assert_eq!(viewport.cursor_row, 1);
    assert_eq!(viewport.cursor_column, 6);
    Ok(())
}

#[test]
fn native_hcodex_chrome_renders_badges_and_role_colors_without_omh_labels() -> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(160, 40))?;
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("intelligent-oppenheimer"),
        TuiCatalog::default(),
        false,
    );
    let mut opus_lane = lane("n2_arch_synth", "Architecture synth", LaneState::Accepted);
    opus_lane.role = "ultrabrain".to_owned();
    opus_lane.model = "claude-opus-5-5-medium".to_owned();
    opus_lane.effort = "medium".to_owned();
    app.handle_paste("/dag-mode on");
    let _action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    app.apply_update(TuiUpdate::DagSnapshot(vec![opus_lane]));
    terminal.draw(|frame| draw(frame, &app))?;
    let snapshot = buffer_text(terminal.backend().buffer());
    assert!(snapshot.contains("[HCODEX · TRI-POLAR]"));
    assert!(snapshot.contains("DAG topology · 1 lanes · 0 edges"));
    assert!(snapshot.contains("live transcript · delegate_task · AST & code stream"));
    assert!(!snapshot.contains("OMH"));
    assert!(snapshot.contains("▸▸ accept all (auto-edit) · DAG · done 1/1"));
    assert!(snapshot.contains("ultrabrain · claude-opus-5-5-medium:medium"));
    Ok(())
}

#[test]
fn interactive_skills_page_toggles_active_and_deactivate_and_switches_workbench_tabs()
-> crate::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(120, 32))?;
    let mut app = TuiApp::new_with_motion(
        PathBuf::from("intelligent-oppenheimer"),
        TuiCatalog::command_catalog(),
        false,
    );
    app.apply_update(TuiUpdate::View {
        title: "Discovered skills".to_owned(),
        body: vec![
            "Skills Configuration Page · Active: 1/3 · Budget: 4200/32000 B".to_owned(),
            "Controls: ↑/↓ select · Space/Enter toggle (● active ↔ ○ deactivate) · c core · d default · x clear · 1-7 pages".to_owned(),
            "────────────────────────────────────────────────────────────────────────────────────────".to_owned(),
            "  $codex-heterarchy             ● active     · Builtin · Tri-Polar collaboration".to_owned(),
            "  $codegraph-operator           ○ deactivate · Codex · CodeGraph symbol operator".to_owned(),
            "  $codebase-refactor-guardian   ○ deactivate · Agents · Refactor guardian".to_owned(),
        ],
    });
    terminal.draw(|frame| draw(frame, &app))?;
    let snapshot = buffer_text(terminal.backend().buffer());
    assert!(snapshot.contains("▸ $codex-heterarchy"));
    assert!(snapshot.contains("● active"));
    assert!(snapshot.contains("○ deactivate"));
    assert!(snapshot.contains("[1:Skills]"));
    assert!(snapshot.contains("2:Models"));

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
    assert_eq!(app.skill_cursor(), 1);
    let toggle_action = app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))?;
    assert_eq!(
        toggle_action,
        Some(crate::tui::TuiAction::ToggleSkill(
            "on codegraph-operator".to_owned()
        ))
    );
    app.apply_update(TuiUpdate::ActiveSkills(vec![
        "codex-heterarchy".to_owned(),
        "codegraph-operator".to_owned(),
    ]));
    terminal.draw(|frame| draw(frame, &app))?;
    let updated = buffer_text(terminal.backend().buffer());
    assert!(updated.contains("Active: 2/3"));
    assert!(updated.contains("▸ $codegraph-operator           ● active"));

    let deactivate_action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    assert_eq!(
        deactivate_action,
        Some(crate::tui::TuiAction::ToggleSkill(
            "off codegraph-operator".to_owned()
        ))
    );
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))?,
        Some(crate::tui::TuiAction::ToggleSkill("core".to_owned()))
    );
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))?,
        Some(crate::tui::TuiAction::ShowModels)
    );

    app.handle_paste("/use ");
    assert!(
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))?
            .is_none(),
        "non-empty composer must receive digits instead of switching pages"
    );
    assert_eq!(app.input(), "/use 2");
    Ok(())
}

fn lane(id: &str, title: &str, state: LaneState) -> DagLaneView {
    DagLaneView {
        id: id.to_owned(),
        layer: 0,
        dependencies: Vec::new(),
        title: title.to_owned(),
        summary: "actual execution evidence".to_owned(),
        role: "verify".to_owned(),
        model: "gpt-6.1-sol".to_owned(),
        effort: "xhigh".to_owned(),
        state,
        note: "receipt-backed state".to_owned(),
        elapsed: None,
    }
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut lines = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}
