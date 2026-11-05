//! Ratatui rendering for the fixed workbench layout.

use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::text::{inspector_width, wrap_display_line};
use super::{DagLaneView, LaneState, TuiApp, TuiTheme, WorkspacePane};
use crate::workspaces::{WorkspaceKind, WorkspaceRunStatus};

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkbenchAreas {
    pub header: Rect,
    pub workspace_rail: Rect,
    pub graph: Rect,
    pub transcript: Rect,
    pub composer: Rect,
    pub status: Rect,
}

pub(crate) fn workbench_areas(area: Rect, lane_count: usize) -> WorkbenchAreas {
    let composer_height = if area.height >= 8 { 4 } else { 2 };
    let status_height = if area.height >= 30 {
        u16::try_from(lane_count.saturating_add(5).min(13)).unwrap_or(13)
    } else if area.height >= 22 {
        u16::try_from(lane_count.saturating_add(4).min(10)).unwrap_or(10)
    } else if area.height >= 18 {
        u16::try_from(lane_count.saturating_add(4).min(8)).unwrap_or(8)
    } else if area.height >= 14 {
        4
    } else if area.height >= 5 {
        2
    } else {
        1
    };
    let graph_constraint = if lane_count == 0 && area.height >= 20 {
        Constraint::Length(7)
    } else if area.height >= 36 && lane_count >= 5 {
        Constraint::Percentage(42)
    } else {
        Constraint::Percentage(35)
    };
    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            graph_constraint,
            Constraint::Min(0),
            Constraint::Length(composer_height),
            Constraint::Length(status_height),
        ])
        .split(area);
    WorkbenchAreas {
        header: regions[0],
        workspace_rail: regions[1],
        graph: regions[2],
        transcript: regions[3],
        composer: regions[4],
        status: regions[5],
    }
}

pub fn draw(frame: &mut Frame<'_>, app: &TuiApp) {
    let areas = workbench_areas(frame.area(), app.lanes().len());
    draw_header(frame, areas.header, app);
    draw_workspace_rail(frame, areas.workspace_rail, app);
    draw_graph(frame, areas.graph, app);
    draw_transcript(frame, areas.transcript, app);
    draw_composer(frame, areas.composer, app);
    draw_status(frame, areas.status, app);
    draw_view(frame, app);
    draw_completions(frame, app, areas.composer);
}

fn draw_workspace_rail(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let compact = area.width < 72;
    let wide = area.width >= 96;
    let mono = app.theme() == TuiTheme::MinimalMono;
    let separator = if compact { " │ " } else { "  │  " };
    let sep_style = Style::default().fg(if mono {
        Color::DarkGray
    } else {
        Color::Indexed(242)
    });
    let mut spans = Vec::new();
    for (index, (kind, pane)) in app.workspace_tabs().into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(separator, sep_style));
        }
        let name = if compact {
            match kind {
                WorkspaceKind::Research => "R",
                WorkspaceKind::Planning => "P",
                WorkspaceKind::Building => "B",
            }
        } else {
            kind.label()
        };
        let active = kind == app.active_workspace();
        let color = if mono {
            if active { Color::White } else { Color::Gray }
        } else {
            match kind {
                WorkspaceKind::Research => HCODEX_SKY,
                WorkspaceKind::Planning => Color::Indexed(214),
                WorkspaceKind::Building => Color::Indexed(48),
            }
        };
        let label = if compact {
            if area.width >= 48 {
                format!("{name}{} {}", pane.status_glyph(), pane.status_label())
            } else {
                format!("{name}{}", pane.status_glyph())
            }
        } else if wide {
            format!(
                "Alt+{} {name} {} {}",
                index + 1,
                pane.status_glyph(),
                pane.status_label()
            )
        } else {
            format!("{name} {} {}", pane.status_glyph(), pane.status_label())
        };
        let style = if active {
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let (done, total) = app.progress();
    let phase = if total > 0 && done == total {
        "Completed"
    } else if total > 0
        && (app.lanes().is_empty()
            || app.lanes().iter().any(|lane| {
                matches!(
                    lane.state,
                    LaneState::Running | LaneState::Stopped | LaneState::HandedOut
                )
            }))
    {
        "Running"
    } else if app
        .lanes()
        .iter()
        .any(|lane| matches!(lane.state, LaneState::Rejected | LaneState::Unverified))
    {
        "Unverified"
    } else if total > 0 {
        "Planned"
    } else {
        "Ready"
    };
    let workspace = app
        .workspace()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let max_layer = app
        .lanes()
        .iter()
        .map(|lane| lane.layer)
        .max()
        .map_or(0, |layer| layer + 1);
    let active = app.lanes().iter().find(|lane| {
        matches!(
            lane.state,
            LaneState::Running | LaneState::HandedOut | LaneState::Stopped
        )
    });
    let lane_meta = active.map_or_else(
        || format!("layers {max_layer}"),
        |lane| format!("layer {}/{} · {}", lane.layer + 1, max_layer, lane.title),
    );
    let text = if app.active_workspace() == WorkspaceKind::Building {
        format!(
            "{phase} · Done {done}/{total} · {lane_meta} · {workspace} · {}",
            app.permission_mode()
        )
    } else {
        format!(
            "{} · {} · {workspace} · read-only",
            app.active_workspace_pane().status_label(),
            app.active_workspace().label()
        )
    };
    let fitted = fit(&text, area.width as usize);
    let left_used = UnicodeWidthStr::width(fitted.as_str());
    frame.render_widget(
        Paragraph::new(fitted).style(Style::default().fg(theme_accent(app.theme()))),
        area,
    );
    let (badge_text, badge_suffix) = match app.active_workspace() {
        WorkspaceKind::Research => ("[HCODEX · RESEARCH]", " · RESEARCH]"),
        WorkspaceKind::Planning => ("[HCODEX · PLANNING]", " · PLANNING]"),
        WorkspaceKind::Building => ("[HCODEX · TRI-POLAR]", " · TRI-POLAR]"),
    };
    let badge_width = UnicodeWidthStr::width(badge_text);
    if area.height > 0
        && usize::from(area.width) >= left_used.saturating_add(badge_width).saturating_add(2)
        && let Ok(slot_width) = u16::try_from(badge_width)
    {
        let slot = Rect::new(
            area.x.saturating_add(area.width.saturating_sub(slot_width)),
            area.y,
            slot_width,
            1,
        );
        let badge_line = if app.theme() == TuiTheme::MinimalMono {
            Line::from(Span::styled(
                badge_text,
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    "[HCODEX",
                    Style::default().fg(HCODEX_SKY).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    badge_suffix,
                    Style::default()
                        .fg(theme_accent(app.theme()))
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        };
        frame.render_widget(Paragraph::new(badge_line), slot);
    }
}

fn draw_graph(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    if app.active_workspace() != WorkspaceKind::Building {
        draw_workspace_overview(frame, area, app);
        return;
    }
    let edge_count: usize = app.lanes().iter().map(|lane| lane.dependencies.len()).sum();
    let graph_title = if !app.lanes().is_empty() && area.width >= 52 {
        format!(
            " DAG topology · {} lanes · {edge_count} edges ",
            app.lanes().len()
        )
    } else {
        " DAG topology ".to_owned()
    };
    let title_style = if app.theme() == TuiTheme::MinimalMono {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(HCODEX_SKY).add_modifier(Modifier::BOLD)
    };
    let block = Block::default()
        .title(Span::styled(graph_title, title_style))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Indexed(242)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.lanes().is_empty() {
        if app.dag_mode() {
            draw_routing_preview(frame, inner, app);
            return;
        }
        frame.render_widget(
            Paragraph::new("No graph loaded · /dag-ulw <goal>")
                .style(Style::default().fg(Color::Indexed(244))),
            inner,
        );
        return;
    }
    let max_layer = app.lanes().iter().map(|lane| lane.layer).max().unwrap_or(0);
    let gutter_width = 5;
    let column_width =
        usize::from(inner.width).saturating_sub(max_layer * gutter_width) / (max_layer + 1);
    let connected = app.lanes().iter().any(|lane| !lane.dependencies.is_empty());
    let cards_fit = (0..=max_layer).all(|layer| {
        let count = app
            .lanes()
            .iter()
            .filter(|lane| lane.layer == layer)
            .count();
        if count == 0 {
            return true;
        }
        let height = usize::from(inner.height).saturating_sub(usize::from(count > 1));
        let rows = if !connected && column_width / count >= 28 {
            1
        } else {
            count
        };
        height / rows >= 4
    });
    let adjacent_edges = app.lanes().iter().all(|lane| {
        lane.dependencies.iter().all(|parent| {
            app.lanes()
                .iter()
                .find(|candidate| &candidate.id == parent)
                .is_some_and(|parent| parent.layer + 1 == lane.layer)
        })
    });
    let min_column_width = if max_layer >= 3 { 22 } else { 28 };
    if column_width < min_column_width || !cards_fit || !adjacent_edges {
        draw_compact_graph(frame, inner, app);
        return;
    }
    let mut constraints = Vec::new();
    for layer in 0..=max_layer {
        if layer > 0 {
            constraints.push(Constraint::Length(gutter_width as u16));
        }
        constraints.push(Constraint::Ratio(1, (max_layer + 1) as u32));
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(inner);
    let mut positions = BTreeMap::new();
    for layer in 0..=max_layer {
        let column = columns[layer * 2];
        let lanes = app
            .lanes()
            .iter()
            .filter(|lane| lane.layer == layer)
            .collect::<Vec<_>>();
        for (id, rect) in draw_layer(
            frame,
            column,
            layer,
            &lanes,
            lanes.iter().any(|lane| {
                matches!(
                    lane.state,
                    LaneState::Running | LaneState::Stopped | LaneState::HandedOut
                )
            }),
            app,
        ) {
            positions.insert(id, rect);
        }
    }
    let edges = dependency_edges(app.lanes(), &positions);
    if ambiguous_edges(&edges) {
        frame.render_widget(Clear, inner);
        draw_compact_graph(frame, inner, app);
    } else {
        draw_dependency_edges(frame, app, &edges);
    }
}

fn research_handoff_ready(pane: &WorkspacePane) -> bool {
    matches!(
        pane.status(),
        WorkspaceRunStatus::Completed | WorkspaceRunStatus::IncompleteEvidence
    )
}

fn build_handoff_ready(pane: &WorkspacePane) -> bool {
    pane.status() == WorkspaceRunStatus::BuildReady
        && pane.artifact().is_some_and(|artifact| artifact.ready)
}

fn workspace_run_reference(id: &str) -> String {
    if id.starts_with("preview-") {
        format!("Preview: {id}")
    } else {
        format!("Flow: {id}")
    }
}

fn workspace_artifact_line(command: &str, id: &str, width: u16) -> String {
    let line = format!("{command} {id}");
    if UnicodeWidthStr::width(line.as_str()) <= usize::from(width) {
        line
    } else {
        format!("Artifact: {id}")
    }
}

fn draw_workspace_overview(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let kind = app.active_workspace();
    if kind == WorkspaceKind::Building {
        return;
    }
    let pane = app.active_workspace_pane();
    let color = if app.theme() == TuiTheme::MinimalMono {
        Color::Gray
    } else if kind == WorkspaceKind::Research {
        HCODEX_SKY
    } else {
        Color::Indexed(214)
    };
    let title = if area.width >= 56 {
        format!(
            " {} · {} {} · read-only ",
            kind.label(),
            pane.status_glyph(),
            pane.status_label()
        )
    } else {
        format!(" {} ", kind.label())
    };
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Indexed(242)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let max_rows = usize::from(inner.height);
    let research = app.workspace_pane(WorkspaceKind::Research);
    let research_artifact = research
        .artifact()
        .filter(|_| research_handoff_ready(research));
    let is_build_ready = kind == WorkspaceKind::Planning && build_handoff_ready(pane);
    let mut rows = Vec::new();
    if pane.objective().is_empty() {
        let hints = match kind {
            WorkspaceKind::Research => [
                "/research -- <question>",
                "Search sources, then investigate and compare evidence.",
                "/workflow --through planning -- <goal>",
            ],
            WorkspaceKind::Planning => [
                "/planning -- <goal>",
                "/planning --from <research_id> -- <goal>",
                "Requirements, scope, decisions and acceptance criteria.",
            ],
            WorkspaceKind::Building => return,
        };
        rows.extend(hints.into_iter().map(str::to_owned));
        rows.push(format!("{} · {}", pane.status_label(), pane.detail()));
        if pane.artifact().is_none() && rows.len() < max_rows {
            let footer = match (kind, research_artifact) {
                (WorkspaceKind::Research, _) => {
                    "Read-only outside repo · standalone or /planning --from <research_id>"
                        .to_owned()
                }
                (WorkspaceKind::Planning, Some(res)) => format!(
                    "Research handoff ready: /planning --from {} -- <goal>",
                    res.artifact_id
                ),
                (WorkspaceKind::Planning, None) => {
                    "BuildReady is mechanical only · /building --plan <plan_id> gates clean repo"
                        .to_owned()
                }
                (WorkspaceKind::Building, _) => String::new(),
            };
            if !footer.is_empty() {
                rows.push(footer);
            }
        }
    } else {
        rows.push(format!("Goal: {}", pane.objective()));
        match (
            pane.current_run_id(),
            pane.artifact().is_some() && max_rows <= 4,
        ) {
            (Some(id), true) => {
                rows.push(format!(
                    "{} · {} · {}",
                    workspace_run_reference(id),
                    pane.status_label(),
                    pane.detail()
                ));
            }
            (Some(id), false) => {
                rows.push(format!("{} · {}", pane.status_label(), pane.detail()));
                rows.push(workspace_run_reference(id));
            }
            (None, _) => {
                rows.push(format!("{} · {}", pane.status_label(), pane.detail()));
            }
        }
    }
    if let Some(artifact) = pane.artifact() {
        let artifact_line = match kind {
            WorkspaceKind::Research if research_handoff_ready(pane) => {
                workspace_artifact_line("/planning --from", &artifact.artifact_id, inner.width)
            }
            WorkspaceKind::Planning if is_build_ready => {
                workspace_artifact_line("/building --plan", &artifact.artifact_id, inner.width)
            }
            WorkspaceKind::Planning => format!(
                "Artifact: {} · not BuildReady (building gated)",
                artifact.artifact_id
            ),
            WorkspaceKind::Research | WorkspaceKind::Building => {
                format!("Artifact: {}", artifact.artifact_id)
            }
        };
        rows.push(artifact_line);
        rows.push(format!("Digest: {}", artifact.digest));
        if rows.len() < max_rows {
            let handoff = match kind {
                WorkspaceKind::Research if research_handoff_ready(pane) => format!(
                    "Handoff (optional): /planning --from {} -- <goal>",
                    artifact.artifact_id
                ),
                WorkspaceKind::Planning if is_build_ready => {
                    "Mechanical readiness only · explicit build requires a clean repo".into()
                }
                WorkspaceKind::Planning => {
                    "Building blocked until plan reaches mechanical BuildReady.".to_owned()
                }
                WorkspaceKind::Research | WorkspaceKind::Building => String::new(),
            };
            if !handoff.is_empty() {
                rows.push(handoff);
            }
        }
    } else if !pane.objective().is_empty() && rows.len() < max_rows {
        let note = match kind {
            WorkspaceKind::Research => {
                "Read-only outside original · optional handoff: /planning --from <research_id>"
            }
            WorkspaceKind::Planning if is_build_ready => {
                "BuildReady is mechanical readiness only, not semantic approval."
            }
            WorkspaceKind::Planning => {
                "Read-only outside original · Building requires explicit /building --plan <plan_id>"
            }
            WorkspaceKind::Building => "",
        };
        if !note.is_empty() {
            rows.push(note.to_owned());
        }
    }
    let rows = rows
        .into_iter()
        .map(|line| {
            Line::from(Span::styled(
                fit(&line, usize::from(inner.width)),
                Style::default().fg(color),
            ))
        })
        .take(max_rows)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rows), inner);
}

fn draw_routing_preview(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let diagram: &[&str] = if area.width >= 68 && area.height >= 5 {
        &[
            "Routing preview · not run",
            "                  ╭── Claude · craft ──╮",
            "[Argon · plan] ────┤                   ├──▶ [Argon · verify]",
            "                  ╰── Codex · audit ───╯",
            "Enter a goal to compile the actual dependency graph",
        ]
    } else {
        &["Preview · not run", "Argon → {Claude ∥ Codex} → Argon"]
    };
    let lines = diagram
        .iter()
        .enumerate()
        .map(|(row, text)| {
            let spans = fit(text, usize::from(area.width))
                .chars()
                .enumerate()
                .map(|(column, character)| {
                    let connector = "─│╭╮╰╯├┤▶→".contains(character);
                    let highlight = app.motion_enabled()
                        && connector
                        && (column + row * 5) % 24 == (app.shimmer_tick() / 4) % 24;
                    Span::styled(
                        character.to_string(),
                        Style::default().fg(if highlight {
                            Color::White
                        } else if connector {
                            theme_accent(app.theme())
                        } else {
                            Color::Gray
                        }),
                    )
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

#[derive(Debug)]
struct DependencyEdge<'a> {
    parent: &'a str,
    target: &'a DagLaneView,
    path: Vec<(u16, u16)>,
}

fn dependency_edges<'a>(
    lanes: &'a [DagLaneView],
    positions: &BTreeMap<String, Rect>,
) -> Vec<DependencyEdge<'a>> {
    let mut edges = Vec::new();
    for target in lanes {
        let Some(to) = positions.get(&target.id) else {
            continue;
        };
        for parent in &target.dependencies {
            let Some(from) = positions.get(parent) else {
                continue;
            };
            let start = from.right();
            let end = to.x.saturating_sub(1);
            if start > end {
                continue;
            }
            let middle = start + (end - start) / 2;
            let from_y = from.y + from.height / 2;
            let to_y = to.y + to.height / 2;
            let mut path = (start..=middle).map(|x| (x, from_y)).collect::<Vec<_>>();
            if from_y < to_y {
                path.extend((from_y + 1..=to_y).map(|y| (middle, y)));
            }
            if from_y > to_y {
                path.extend((to_y..from_y).rev().map(|y| (middle, y)));
            }
            path.extend((middle + 1..=end).map(|x| (x, to_y)));
            edges.push(DependencyEdge {
                parent,
                target,
                path,
            });
        }
    }
    edges
}

fn ambiguous_edges(edges: &[DependencyEdge<'_>]) -> bool {
    edges.iter().enumerate().any(|(index, edge)| {
        edges[index + 1..].iter().any(|other| {
            edge.parent != other.parent
                && edge.target.id != other.target.id
                && edge.path.iter().any(|point| other.path.contains(point))
                && !(has_edge(edges, edge.parent, &other.target.id)
                    && has_edge(edges, other.parent, &edge.target.id))
        })
    })
}

fn has_edge(edges: &[DependencyEdge<'_>], parent: &str, target: &str) -> bool {
    edges
        .iter()
        .any(|edge| edge.parent == parent && edge.target.id == target)
}

fn draw_dependency_edges(frame: &mut Frame<'_>, app: &TuiApp, edges: &[DependencyEdge<'_>]) {
    let mut junctions = BTreeMap::<(u16, u16), u8>::new();
    for edge in edges {
        for pair in edge.path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (forward, backward) = if a.0 < b.0 {
                (1, 2)
            } else if a.1 < b.1 {
                (8, 4)
            } else {
                (4, 8)
            };
            *junctions.entry(a).or_default() |= forward;
            *junctions.entry(b).or_default() |= backward;
        }
    }
    let buffer = frame.buffer_mut();
    let bounds = buffer.area;
    for ((x, y), connections) in junctions {
        let glyph = match connections {
            3 => "─",
            12 => "│",
            9 => "┌",
            10 => "┐",
            5 => "└",
            6 => "┘",
            13 => "├",
            14 => "┤",
            11 => "┬",
            7 => "┴",
            15 => "┼",
            _ => "─",
        };
        if x < bounds.right() && y < bounds.bottom() {
            buffer[(x, y)]
                .set_symbol(glyph)
                .set_style(Style::default().fg(Color::DarkGray));
        }
    }
    for edge in edges {
        let color = pole_color(edge.target, app);
        if let Some(&(x, y)) = edge.path.last()
            && x < bounds.right()
            && y < bounds.bottom()
        {
            buffer[(x, y)]
                .set_symbol("▶")
                .set_style(Style::default().fg(color));
        }
        if app.motion_enabled()
            && matches!(edge.target.state, LaneState::Running | LaneState::Stopped)
            && !edge.path.is_empty()
        {
            let travel = edge.path.len().saturating_sub(1).max(1);
            let (x, y) = edge.path[(app.shimmer_tick() / 6) % travel];
            if x < bounds.right() && y < bounds.bottom() {
                buffer[(x, y)]
                    .set_symbol("●")
                    .set_style(Style::default().fg(color));
            }
        }
    }
}

fn pole_color(lane: &DagLaneView, app: &TuiApp) -> Color {
    if app.theme() == TuiTheme::MinimalMono {
        return Color::Gray;
    }
    if lane.model.starts_with("gemini-") {
        Color::Indexed(141)
    } else if lane.model.contains("opus") {
        Color::Indexed(213)
    } else if lane.model.starts_with("claude-") {
        Color::Indexed(216)
    } else {
        theme_accent(app.theme())
    }
}

fn node_pin(app: &TuiApp, id: &str) -> String {
    app.lanes()
        .iter()
        .position(|lane| lane.id == id)
        .map_or_else(|| format!("?{id}"), |index| format!("#{}", index + 1))
}

fn parent_pins(lane: &DagLaneView, app: &TuiApp) -> String {
    if lane.dependencies.is_empty() {
        return String::new();
    }
    format!(
        "← {}",
        lane.dependencies
            .iter()
            .map(|id| node_pin(app, id))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// A compact diamond is shown only when those are the graph's exact edges.
fn compact_topology(app: &TuiApp, width: usize, max_rows: usize) -> Vec<Line<'static>> {
    let roots = app
        .lanes()
        .iter()
        .filter(|lane| lane.dependencies.is_empty())
        .collect::<Vec<_>>();
    let terminals = app
        .lanes()
        .iter()
        .filter(|lane| {
            !app.lanes()
                .iter()
                .any(|child| child.dependencies.contains(&lane.id))
        })
        .collect::<Vec<_>>();
    if roots.len() == 1 && terminals.len() == 1 {
        let root = roots[0];
        let terminal = terminals[0];
        let middle = app
            .lanes()
            .iter()
            .filter(|lane| lane.id != root.id && lane.id != terminal.id)
            .collect::<Vec<_>>();
        let exact = middle.len() >= 2
            && middle.len() <= max_rows
            && middle
                .iter()
                .all(|lane| lane.dependencies == [root.id.clone()])
            && terminal.dependencies.len() == middle.len()
            && middle
                .iter()
                .all(|lane| terminal.dependencies.contains(&lane.id));
        if exact {
            let start = node_pin(app, &root.id);
            let end = node_pin(app, &terminal.id);
            let indent = " ".repeat(start.len() + 2);
            return middle
                .iter()
                .enumerate()
                .map(|(index, lane)| {
                    let pin = node_pin(app, &lane.id);
                    let text = if index == 0 {
                        format!("{start} ─┬→ {pin} ─┬→ {end}")
                    } else if index + 1 == middle.len() {
                        format!("{indent}└→ {pin} ─┘")
                    } else {
                        format!("{indent}├→ {pin} ─┤")
                    };
                    let color = if matches!(lane.state, LaneState::Running | LaneState::Stopped) {
                        pole_color(lane, app)
                    } else {
                        Color::DarkGray
                    };
                    Line::from(Span::styled(fit(&text, width), Style::default().fg(color)))
                })
                .collect();
        }
    }
    let pairs = app
        .lanes()
        .iter()
        .flat_map(|lane| {
            lane.dependencies
                .iter()
                .map(move |parent| (parent, &lane.id))
        })
        .collect::<Vec<_>>();
    if pairs.is_empty() || max_rows == 0 {
        return Vec::new();
    }
    let mut text = "Links:".to_owned();
    for (index, (parent, target)) in pairs.iter().enumerate() {
        let token = format!(" {}→{}", node_pin(app, parent), node_pin(app, target));
        let remaining = pairs.len() - index - 1;
        let suffix = if remaining > 0 {
            format!(" … +{remaining}")
        } else {
            String::new()
        };
        if UnicodeWidthStr::width(text.as_str())
            + UnicodeWidthStr::width(token.as_str())
            + UnicodeWidthStr::width(suffix.as_str())
            > width
        {
            text.push_str(&format!(" … +{}", pairs.len() - index));
            break;
        }
        text.push_str(&token);
    }
    vec![Line::from(Span::styled(
        fit(&text, width),
        Style::default().fg(Color::DarkGray),
    ))]
}

fn draw_compact_graph(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    // Preserve the running/failed rows when the terminal cannot fit full cards.
    let mut lanes = app.lanes().iter().collect::<Vec<_>>();
    lanes.sort_by_key(|lane| match lane.state {
        LaneState::Running | LaneState::Stopped => 0,
        LaneState::Rejected | LaneState::Unverified => 1,
        LaneState::HandedOut => 2,
        LaneState::Waiting => 3,
        LaneState::Accepted => 4,
    });
    let mut lines = compact_topology(
        app,
        usize::from(area.width),
        usize::from(area.height).saturating_sub(3),
    );
    let available = usize::from(area.height).saturating_sub(lines.len());
    let visible = if lanes.len() > available {
        available.saturating_sub(1)
    } else {
        lanes.len()
    };
    lines.extend(
        lanes
            .iter()
            .take(visible)
            .map(|lane| {
                let elapsed = lane.elapsed.map_or_else(String::new, |elapsed| {
                    format!(" {:.1}s", elapsed.as_secs_f64())
                });
                let parents = parent_pins(lane, app);
                let prefix = format!(
                    "{}{} {parents} · L{} ",
                    lane_activity(lane, app),
                    node_pin(app, &lane.id),
                    lane.layer + 1
                );
                let stage = format!("{}{elapsed}", lane.state.label());
                let id_budget = usize::from(area.width)
                    .saturating_sub(
                        UnicodeWidthStr::width(prefix.as_str())
                            + UnicodeWidthStr::width(stage.as_str())
                            + 1,
                    )
                    .min(24);
                let id = fit(&lane.id, id_budget);
                let text = fit(
                    &format!("{prefix}{id} {stage} · {}:{}", lane.model, lane.effort),
                    usize::from(area.width),
                );
                Line::from(Span::styled(text, lane_motion_style(lane, app)))
            })
            .collect::<Vec<_>>(),
    );
    if lanes.len() > visible && available > 0 {
        lines.push(Line::from(format!(
            "… {} more lanes",
            lanes.len() - visible
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_layer(
    frame: &mut Frame<'_>,
    area: Rect,
    layer: usize,
    lanes: &[&DagLaneView],
    active: bool,
    app: &TuiApp,
) -> Vec<(String, Rect)> {
    if lanes.is_empty() {
        return Vec::new();
    }
    let (frontier_label, card_area) = if lanes.len() > 1 && area.height > 1 {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        (Some(parts[0]), parts[1])
    } else {
        (None, area)
    };
    if let Some(label) = frontier_label {
        frame.render_widget(
            Paragraph::new("· Same frontier").style(Style::default().fg(Color::DarkGray)),
            label,
        );
    }
    let horizontal = app.lanes().iter().all(|lane| lane.dependencies.is_empty())
        && usize::from(card_area.width) / lanes.len() >= 28;
    let constraints = lanes
        .iter()
        .map(|_| Constraint::Ratio(1, lanes.len() as u32))
        .collect::<Vec<_>>();
    let rows = Layout::default()
        .direction(if horizontal {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(constraints)
        .split(card_area);
    let mut card_rects = Vec::with_capacity(lanes.len());
    for (index, lane) in lanes.iter().enumerate() {
        let slot = rows[index];
        let card_height = slot.height.min(6);
        let card_y = slot.y + slot.height.saturating_sub(card_height) / 2;
        let card_rect = Rect::new(slot.x, card_y, slot.width, card_height);
        card_rects.push(card_rect);
        let state = match lane.elapsed {
            Some(elapsed) => format!(
                "{}{} ({:.1}s)",
                lane_activity(lane, app),
                lane.state.label(),
                elapsed.as_secs_f64()
            ),
            None => format!("{}{}", lane_activity(lane, app), lane.state.label()),
        };
        let title = if index == 0 {
            format!(" layer {} · {} ", layer + 1, node_pin(app, &lane.id))
        } else {
            format!(" {} ", node_pin(app, &lane.id))
        };
        let border = if active {
            theme_accent(app.theme())
        } else {
            pole_color(lane, app)
        };
        let inner_width = card_rect.width.saturating_sub(2) as usize;
        let card_lines = if card_rect.height >= 6 {
            vec![
                Line::from(Span::styled(
                    fit(&format!("{}: {}", lane.title, lane.summary), inner_width),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                styled_role_model_line(
                    &lane.role,
                    &lane.model,
                    &lane.effort,
                    inner_width,
                    lane,
                    app,
                ),
                Line::from(Span::styled(
                    fit(&state, inner_width),
                    lane_motion_style(lane, app),
                )),
                styled_parent_note_line(lane, inner_width, app),
            ]
        } else if card_rect.height == 5 {
            vec![
                Line::from(Span::styled(
                    fit(
                        &format!("{} · {}:{}", lane.title, lane.model, lane.effort),
                        inner_width,
                    ),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    fit(&state, inner_width),
                    lane_motion_style(lane, app),
                )),
                styled_parent_note_line(lane, inner_width, app),
            ]
        } else {
            vec![
                Line::from(Span::styled(
                    fit(
                        &format!("{} · {}:{}", lane.title, lane.model, lane.effort),
                        inner_width,
                    ),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    fit(
                        &format!("{state} · {} {}", parent_pins(lane, app), lane.note),
                        inner_width,
                    ),
                    lane_motion_style(lane, app),
                )),
            ]
        };
        let text = Text::from(card_lines);
        let card_title_style = Style::default().fg(border).add_modifier(Modifier::BOLD);
        frame.render_widget(
            Paragraph::new(text)
                .block(
                    Block::default()
                        .title(Span::styled(title, card_title_style))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border)),
                )
                .wrap(Wrap { trim: true }),
            card_rect,
        );
    }
    lanes
        .iter()
        .zip(card_rects)
        .map(|(lane, rect)| (lane.id.clone(), rect))
        .collect()
}

fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let available = area.height.saturating_sub(2) as usize;
    let width = area.width.saturating_sub(2).max(1) as usize;
    let mut rows = std::collections::VecDeque::new();
    if available > 0 {
        'entries: for entry in app.transcript().iter().rev() {
            let logical = entry.message.lines().collect::<Vec<_>>();
            for (index, line) in logical.into_iter().enumerate().rev() {
                let style = transcript_line_style(line, entry.depth, app);
                let prefix = if index == 0 {
                    if entry.depth == 0 { "● " } else { "  › " }
                } else {
                    "    "
                };
                let text = format!("{}{prefix}{line}", "  ".repeat(entry.depth as usize));
                for row in wrap_display_line(&text, width).into_iter().rev() {
                    rows.push_front(ListItem::new(Line::from(Span::styled(row, style))));
                    if rows.len() == available {
                        break 'entries;
                    }
                }
            }
        }
    }
    let transcript_title = match app.active_workspace() {
        WorkspaceKind::Research => " research activity · sources & evidence ",
        WorkspaceKind::Planning => " planning activity · requirements & decisions ",
        WorkspaceKind::Building if area.width >= 56 => {
            " live transcript · delegate_task · AST & code stream "
        }
        WorkspaceKind::Building => " live transcript ",
    };
    let title_style = if app.theme() == TuiTheme::MinimalMono {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(HCODEX_SKY).add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        List::new(rows).block(
            Block::default()
                .title(Span::styled(transcript_title, title_style))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Indexed(242))),
        ),
        area,
    );
}

fn transcript_line_style(line: &str, depth: u8, app: &TuiApp) -> Style {
    if app.theme() == TuiTheme::MinimalMono {
        return Style::default().fg(Color::Gray);
    }
    if line.starts_with("delegate_task(") {
        Style::default()
            .fg(Color::Indexed(213))
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("ast_scan(") {
        Style::default().fg(Color::Indexed(117))
    } else if line.starts_with("opus_arch(") {
        Style::default().fg(Color::Indexed(213))
    } else if line.starts_with("code_craft(") {
        Style::default().fg(Color::Indexed(216))
    } else if line.starts_with("red_team_audit(") {
        Style::default().fg(Color::Indexed(159))
    } else if line.starts_with("synthesis(") {
        Style::default().fg(Color::Indexed(141))
    } else if line.contains("SETTLED · verified") || line.starts_with("DAG finished:") {
        Style::default().fg(Color::Green)
    } else if line.contains("verification rejected")
        || line.contains("blocked by unverified")
        || line.contains("execution failed")
    {
        Style::default().fg(Color::Red)
    } else if line.starts_with("Wave ") || line.starts_with("Layer ") {
        Style::default().fg(theme_accent(app.theme()))
    } else if depth > 0 {
        Style::default().fg(Color::Indexed(246))
    } else {
        Style::default().fg(Color::White)
    }
}

fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let viewport = composer_view(
        app.input(),
        app.cursor(),
        area.height.saturating_sub(2) as usize,
        area.width.saturating_sub(2) as usize,
    );
    let display = if app.input().is_empty() {
        match app.active_workspace() {
            WorkspaceKind::Research => "> Ask a research question",
            WorkspaceKind::Planning => "> Describe what to plan",
            WorkspaceKind::Building => "> Ask Heterarchy Codex to do anything",
        }
        .to_owned()
    } else {
        viewport.text.clone()
    };
    let style = if app.input().is_empty() {
        Style::default().fg(Color::Indexed(245))
    } else {
        Style::default().fg(Color::White)
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_accent(app.theme())));
    let badge = if app.active_workspace() == WorkspaceKind::Building {
        dag_mode_badge(app, usize::from(area.width.saturating_sub(4)))
    } else {
        None
    };
    if let Some(badge) = badge {
        if usize::from(area.width) >= badge.width() + 24 {
            block = block.title(" composer ");
        }
        block = block.title_top(badge.alignment(Alignment::Center));
    } else {
        block = block.title(match app.active_workspace() {
            WorkspaceKind::Research => " research question ",
            WorkspaceKind::Planning => " planning goal ",
            WorkspaceKind::Building => " composer ",
        });
    }
    let paragraph = Paragraph::new(display).style(style).block(block);
    frame.render_widget(paragraph, area);
    if let Some(ghost) = app.completion_ghost() {
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(viewport.cursor_column);
        let y = area.y.saturating_add(1).saturating_add(viewport.cursor_row);
        let width = area.right().saturating_sub(1).saturating_sub(x);
        if width > 0 && y < area.bottom().saturating_sub(1) {
            frame.render_widget(
                Paragraph::new(fit(&ghost.replace('\n', " "), usize::from(width)))
                    .style(Style::default().fg(Color::Indexed(244))),
                Rect::new(x, y, width, 1),
            );
        }
    }
    if !app.input().is_empty() && area.width > 3 && area.height > 2 {
        let cursor_x = area
            .x
            .saturating_add(1)
            .saturating_add(viewport.cursor_column.min(area.width.saturating_sub(2)));
        let cursor_y = area
            .y
            .saturating_add(1)
            .saturating_add(viewport.cursor_row.min(area.height.saturating_sub(3)));
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

/// The historical mode badge owns the entrance sweep; it never represents work.
fn dag_mode_badge(app: &TuiApp, available: usize) -> Option<Line<'static>> {
    let transition = app.dag_transition();
    if !app.dag_mode() && transition.is_none() {
        return None;
    }
    let label = if available >= 29 {
        " D A G - H E T E R A R C H Y "
    } else {
        " DAG "
    };
    let label = fit(label, available);
    let (base, shoulder, highlight) = badge_palette(app.theme());
    let spans = label
        .chars()
        .enumerate()
        .map(|(index, character)| {
            let color = if !app.motion_enabled() {
                base
            } else if let Some((entering, step)) = transition {
                if entering {
                    let distance = index.abs_diff(step * 2);
                    if distance <= 1 {
                        highlight
                    } else if distance <= 3 {
                        shoulder
                    } else {
                        base
                    }
                } else {
                    match step {
                        0 | 1 => base,
                        2 | 3 => Color::DarkGray,
                        _ => Color::Indexed(236),
                    }
                }
            } else if app.theme() != TuiTheme::MinimalMono && (app.shimmer_tick() / 12) % 32 < 3 {
                shoulder
            } else {
                base
            };
            Span::styled(
                character.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )
        })
        .collect::<Vec<_>>();
    Some(Line::from(spans))
}

const fn badge_palette(theme: TuiTheme) -> (Color, Color, Color) {
    match theme {
        TuiTheme::CodexDark => (Color::Indexed(141), Color::Indexed(183), Color::Indexed(87)),
        TuiTheme::RoyalPrism => (
            Color::Indexed(141),
            Color::Indexed(183),
            Color::Indexed(225),
        ),
        TuiTheme::CyberMatrix => (Color::Indexed(40), Color::Indexed(46), Color::Indexed(157)),
        TuiTheme::MinimalMono => (Color::Gray, Color::Gray, Color::White),
    }
}

fn lane_activity(lane: &DagLaneView, app: &TuiApp) -> &'static str {
    if app.motion_enabled() && matches!(lane.state, LaneState::Running | LaneState::Stopped) {
        ["⠋ ", "⠙ ", "⠹ ", "⠸ ", "⠼ ", "⠴ ", "⠦ ", "⠧ "][(app.shimmer_tick() / 8) % 8]
    } else {
        ""
    }
}

const HCODEX_SKY: Color = Color::Indexed(117);

fn lane_role_color(role: &str, lane: &DagLaneView, app: &TuiApp) -> Color {
    if app.theme() == TuiTheme::MinimalMono {
        return Color::Gray;
    }
    match role {
        "ultrabrain" => Color::Indexed(213),
        "deep" => Color::Indexed(216),
        "architect" => Color::Indexed(141),
        "explore" => HCODEX_SKY,
        "verify" | "review" => Color::Indexed(114),
        "integrate" => Color::Indexed(159),
        _ => pole_color(lane, app),
    }
}

fn styled_role_model_line(
    role: &str,
    model: &str,
    effort: &str,
    inner_width: usize,
    lane: &DagLaneView,
    app: &TuiApp,
) -> Line<'static> {
    let text = fit(&format!("{role} · {model}:{effort}"), inner_width);
    if let Some(rest) = text.strip_prefix(role) {
        Line::from(vec![
            Span::styled(
                role.to_owned(),
                Style::default()
                    .fg(lane_role_color(role, lane, app))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_owned(), Style::default().fg(Color::Indexed(250))),
        ])
    } else {
        Line::from(Span::styled(
            text,
            Style::default().fg(lane_role_color(role, lane, app)),
        ))
    }
}

fn styled_parent_note_line(lane: &DagLaneView, inner_width: usize, app: &TuiApp) -> Line<'static> {
    let pins = parent_pins(lane, app);
    let text = fit(&format!("{pins} · {}", lane.note), inner_width);
    if !pins.is_empty()
        && app.theme() != TuiTheme::MinimalMono
        && let Some(rest) = text.strip_prefix(&pins)
    {
        Line::from(vec![
            Span::styled(pins, Style::default().fg(HCODEX_SKY)),
            Span::styled(rest.to_owned(), Style::default().fg(Color::Indexed(246))),
        ])
    } else {
        Line::from(Span::styled(text, Style::default().fg(Color::Indexed(246))))
    }
}

fn lane_motion_style(lane: &DagLaneView, app: &TuiApp) -> Style {
    if app.theme() == TuiTheme::MinimalMono {
        return Style::default().fg(Color::Gray);
    }
    match lane.state {
        LaneState::Running | LaneState::Stopped => Style::default().fg(theme_accent(app.theme())),
        LaneState::Accepted => Style::default().fg(Color::Green),
        LaneState::Rejected | LaneState::Unverified => Style::default().fg(Color::Red),
        LaneState::HandedOut => Style::default().fg(HCODEX_SKY),
        LaneState::Waiting => Style::default().fg(Color::Indexed(245)),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ComposerViewport {
    text: String,
    cursor_row: u16,
    cursor_column: u16,
}

fn composer_view(
    input: &str,
    cursor: usize,
    visible_height: usize,
    inner_width: usize,
) -> ComposerViewport {
    let safe_cursor = cursor.min(input.len());
    let before_cursor = &input[..safe_cursor];
    let cursor_line = before_cursor
        .chars()
        .filter(|character| *character == '\n')
        .count();
    let logical_lines = input.split('\n').collect::<Vec<_>>();
    let height = visible_height.max(1);
    let content_width = inner_width.saturating_sub(2).max(1);
    let mut visual_lines = Vec::new();
    let mut cursor_visual_row = 0usize;
    let mut cursor_column = 2usize;
    for (logical_index, line) in logical_lines.iter().enumerate() {
        let expanded = expand_tabs(line);
        let mut chunks = wrap_display_line(&expanded, content_width);
        if logical_index == cursor_line {
            let current_prefix = expand_tabs(before_cursor.rsplit('\n').next().unwrap_or_default());
            let prefix_width = UnicodeWidthStr::width(current_prefix.as_str());
            let row_in_line = prefix_width / content_width;
            while chunks.len() <= row_in_line {
                chunks.push(String::new());
            }
            cursor_visual_row = visual_lines.len() + row_in_line;
            cursor_column = 2usize.saturating_add(prefix_width % content_width);
        }
        for chunk in chunks {
            visual_lines.push(format!("  {chunk}"));
        }
    }
    if let Some(first) = visual_lines.first_mut() {
        first.replace_range(..2, "> ");
    }
    let start = cursor_visual_row.saturating_sub(height - 1);
    let end = (start + height).min(visual_lines.len());
    let text = visual_lines[start..end].join("\n");
    ComposerViewport {
        text,
        cursor_row: u16::try_from(cursor_visual_row.saturating_sub(start)).unwrap_or(u16::MAX),
        cursor_column: u16::try_from(cursor_column).unwrap_or(u16::MAX),
    }
}

fn expand_tabs(value: &str) -> String {
    value.replace('\t', "    ")
}

fn draw_status(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    if app.active_workspace() != WorkspaceKind::Building {
        draw_workspace_status(frame, area, app);
        return;
    }
    let skills = if app.active_skills().is_empty() {
        "(no skills)".to_owned()
    } else {
        app.active_skills()
            .iter()
            .map(|skill| format!("${skill}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mode = if app.dag_mode() { "DAG" } else { "standard" };
    let (done, total) = app.progress();
    let branch = app.git_branch().unwrap_or("no git");
    let context = app.context_status();
    let tokens_and_draft = format!(
        "{} · draft {} B",
        app.token_counts_status(),
        app.input().len()
    );
    let runtime = format!(
        "▸▸ accept all ({}) · {mode} · done {done}/{total} · {branch} · {}:{}",
        app.permission_mode(),
        app.active_model(),
        app.effort(),
    );
    let width = area.width as usize;
    let combined_hud_1 = format!(
        "{} · {branch} · {context} · {tokens_and_draft} · {skills}",
        app.active_model()
    );
    let combined_hud_2 = format!("{runtime} · {}", app.effect_summary());
    let mut lines = if width >= 110 && UnicodeWidthStr::width(combined_hud_1.as_str()) <= width {
        let hud_1_spans = vec![
            Span::styled(
                format!("{} · ", app.active_model()),
                Style::default().fg(Color::Indexed(252)),
            ),
            Span::styled(format!("{branch} · "), Style::default().fg(Color::Cyan)),
            Span::styled(format!("{context} · "), Style::default().fg(HCODEX_SKY)),
            Span::styled(
                format!("{tokens_and_draft} · "),
                Style::default().fg(Color::Indexed(245)),
            ),
            Span::styled(skills, Style::default().fg(Color::Indexed(211))),
        ];
        vec![
            Line::from(hud_1_spans),
            Line::from(vec![
                Span::styled(
                    fit(&runtime, width),
                    Style::default().fg(Color::Indexed(48)),
                ),
                Span::styled(
                    if UnicodeWidthStr::width(combined_hud_2.as_str()) <= width {
                        format!(" · {}", app.effect_summary())
                    } else {
                        String::new()
                    },
                    Style::default().fg(Color::Indexed(141)),
                ),
            ]),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                fit(&context, width),
                Style::default().fg(HCODEX_SKY),
            )),
            Line::from(Span::styled(
                fit(&tokens_and_draft, width),
                Style::default().fg(Color::Indexed(245)),
            )),
            Line::from(Span::styled(
                fit(&runtime, width),
                Style::default().fg(Color::Indexed(48)),
            )),
            Line::from(Span::styled(
                fit(&skills, width),
                Style::default().fg(Color::Indexed(211)),
            )),
            Line::from(Span::styled(
                fit(app.effect_summary(), width),
                Style::default().fg(Color::Indexed(141)),
            )),
        ]
    };
    let max_layer = app.lanes().iter().map(|lane| lane.layer).max();
    if let Some(max_layer) = max_layer {
        for layer in 0..=max_layer {
            let members = app
                .lanes()
                .iter()
                .filter(|lane| lane.layer == layer)
                .collect::<Vec<_>>();
            let done = members
                .iter()
                .filter(|lane| lane.state == LaneState::Accepted)
                .count();
            let summary = members.first().map_or("", |lane| lane.title.as_str());
            lines.push(styled_layer_header_line(
                layer + 1,
                members.len(),
                done,
                summary,
                width,
                app,
            ));
            for lane in members {
                lines.push(styled_lane_activity_line(lane, width, app));
            }
        }
    }
    lines.truncate(area.height as usize);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(Color::Gray)),
        area,
    );
}

fn draw_workspace_status(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let kind = app.active_workspace();
    let pane = app.active_workspace_pane();
    let is_build_ready = kind == WorkspaceKind::Planning && build_handoff_ready(pane);
    let gate_note = if is_build_ready {
        " · mechanical ready (not approval)"
    } else {
        " · outside original repo"
    };
    let reference = if let Some(artifact) = pane.artifact() {
        match kind {
            WorkspaceKind::Research if research_handoff_ready(pane) => {
                workspace_artifact_line("/planning --from", &artifact.artifact_id, area.width)
            }
            WorkspaceKind::Planning if is_build_ready => {
                workspace_artifact_line("/building --plan", &artifact.artifact_id, area.width)
            }
            WorkspaceKind::Planning => format!(
                "Artifact: {} · building gated (not BuildReady)",
                artifact.artifact_id
            ),
            WorkspaceKind::Research | WorkspaceKind::Building => {
                format!("Artifact: {}", artifact.artifact_id)
            }
        }
    } else if let Some(id) = pane.current_run_id() {
        workspace_run_reference(id)
    } else {
        "/workflow-status <flow_id> · /space building".into()
    };
    let handoff_row = if let Some(artifact) = pane.artifact() {
        match kind {
            WorkspaceKind::Research if research_handoff_ready(pane) => format!(
                "Handoff: /planning --from {} -- <goal> · Digest: {}",
                artifact.artifact_id, artifact.digest
            ),
            WorkspaceKind::Planning if is_build_ready => format!(
                "Explicit gate: /building --plan {} · Digest: {}",
                artifact.artifact_id, artifact.digest
            ),
            WorkspaceKind::Planning => format!(
                "Hold: {} is not mechanically BuildReady · Digest: {}",
                artifact.artifact_id, artifact.digest
            ),
            WorkspaceKind::Research | WorkspaceKind::Building => {
                format!("Digest: {}", artifact.digest)
            }
        }
    } else {
        match kind {
            WorkspaceKind::Research => {
                "Standalone: /research -- <q> · Chained: /workflow --through planning -- <goal>"
                    .into()
            }
            WorkspaceKind::Planning => {
                "Standalone: /planning -- <goal> · Chained: /workflow --stages planning,building -- <goal>"
                    .into()
            }
            WorkspaceKind::Building => "/building --plan <plan_id>".into(),
        }
    };
    let rows = [
        format!(
            "read-only · {} · {}{gate_note}",
            kind.label(),
            pane.status_label()
        ),
        "stage models · see receipts · Alt+1/2/3 · /space".into(),
        format!(
            "session {} · draft {} B",
            app.token_counts_status(),
            app.input().len()
        ),
        reference,
        handoff_row,
    ];
    let rows = rows
        .into_iter()
        .map(|line| {
            Line::from(Span::styled(
                fit(&line, usize::from(area.width)),
                Style::default().fg(Color::Gray),
            ))
        })
        .take(usize::from(area.height))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rows), area);
}

fn styled_layer_header_line(
    layer_num: usize,
    node_count: usize,
    done_count: usize,
    summary: &str,
    width: usize,
    app: &TuiApp,
) -> Line<'static> {
    let prefix = format!("▾ layer {layer_num}");
    let counts = format!(" · {node_count} nodes · {done_count} done");
    let suffix = format!(" · {summary}");
    let full = fit(&format!("{prefix}{counts}{suffix}"), width);
    if app.theme() == TuiTheme::MinimalMono {
        return Line::from(Span::styled(full, Style::default().fg(Color::Gray)));
    }
    if let Some(after_prefix) = full.strip_prefix(&prefix) {
        if let Some(after_counts) = after_prefix.strip_prefix(&counts) {
            return Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default().fg(HCODEX_SKY).add_modifier(Modifier::BOLD),
                ),
                Span::styled(counts, Style::default().fg(Color::Indexed(250))),
                Span::styled(
                    after_counts.to_owned(),
                    Style::default().fg(Color::Indexed(245)),
                ),
            ]);
        }
        return Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(HCODEX_SKY).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                after_prefix.to_owned(),
                Style::default().fg(Color::Indexed(250)),
            ),
        ]);
    }
    Line::from(Span::styled(full, Style::default().fg(HCODEX_SKY)))
}

fn styled_lane_activity_line(lane: &DagLaneView, width: usize, app: &TuiApp) -> Line<'static> {
    let title_part = format!("  • {}", lane.title);
    let summary_part = format!(": {}  ", lane.summary);
    let role_part = lane.role.clone();
    let model_part = format!(" · {}:{}  ", lane.model, lane.effort);
    let state_part = lane.state.label().to_owned();
    let full = fit(
        &format!("{title_part}{summary_part}{role_part}{model_part}{state_part}"),
        width,
    );
    if app.theme() == TuiTheme::MinimalMono {
        return Line::from(Span::styled(full, Style::default().fg(Color::Gray)));
    }
    let prefix_1 = format!("{title_part}{summary_part}");
    let prefix_2 = format!("{prefix_1}{role_part}");
    let prefix_3 = format!("{prefix_2}{model_part}");
    if let Some(after_model) = full.strip_prefix(&prefix_3) {
        Line::from(vec![
            Span::styled(
                title_part,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(summary_part, Style::default().fg(Color::Indexed(248))),
            Span::styled(
                role_part,
                Style::default()
                    .fg(lane_role_color(&lane.role, lane, app))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(model_part, Style::default().fg(Color::Indexed(246))),
            Span::styled(after_model.to_owned(), lane_motion_style(lane, app)),
        ])
    } else if let Some(after_title) = full.strip_prefix(&title_part) {
        Line::from(vec![
            Span::styled(
                title_part,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                after_title.to_owned(),
                Style::default().fg(Color::Indexed(248)),
            ),
        ])
    } else {
        Line::from(Span::styled(full, Style::default().fg(Color::Indexed(248))))
    }
}

fn draw_completions(frame: &mut Frame<'_>, app: &TuiApp, composer: Rect) {
    if app.completions().is_empty() || composer.y < 3 {
        return;
    }
    let height = (app.completions().len() as u16 + 2).min(12).min(composer.y);
    let width = composer.width;
    let area = Rect::new(composer.x, composer.y - height, width, height);
    frame.render_widget(Clear, area);
    let rows = usize::from(height.saturating_sub(2));
    let start = app
        .completion_index()
        .saturating_sub(rows.saturating_sub(1));
    let items = app
        .completions()
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .map(|(index, item)| {
            let style = if index == app.completion_index() {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            let selected = if index == app.completion_index() {
                "›"
            } else {
                " "
            };
            ListItem::new(fit(
                &format!(
                    "{selected} {}  {}  {}",
                    item.display_label, item.badge, item.detail
                ),
                width.saturating_sub(2) as usize,
            ))
            .style(style)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    " completion {}/{} · ↑↓ select · Tab accept · Esc close ",
                    app.completion_index() + 1,
                    app.completions().len()
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn draw_view(frame: &mut Frame<'_>, app: &TuiApp) {
    let Some((title, body)) = app.current_view() else {
        return;
    };
    let outer = frame.area();
    let composer = workbench_areas(outer, app.lanes().len()).composer;
    let content_height = composer.y.saturating_sub(outer.y);
    let width = inspector_width(outer.width);
    let height = content_height.saturating_mul(4) / 5;
    let area = Rect::new(
        outer.x + outer.width.saturating_sub(width) / 2,
        outer.y + content_height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let selected_skill_line = (title == "Discovered skills")
        .then(|| app.selected_skill_line_index())
        .flatten();
    let rows = body
        .iter()
        .enumerate()
        .flat_map(|(line_index, line)| {
            let is_selected = selected_skill_line == Some(line_index);
            let display_line = if is_selected && line.starts_with("  $") {
                format!("▸ ${}", &line[3..])
            } else {
                line.clone()
            };
            wrap_display_line(&display_line, inner_width)
                .into_iter()
                .map(move |chunk| style_view_line(&chunk, is_selected, app))
        })
        .collect::<Vec<_>>();
    let mut block = Block::default()
        .title(format!(" {title} · PgUp/PgDn scroll · Esc to return "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_accent(app.theme())));
    if width >= 66 {
        block = block.title_bottom(
            workbench_page_tabs(title, width.saturating_sub(4) as usize, app)
                .alignment(Alignment::Center),
        );
    }
    frame.render_widget(
        Paragraph::new(rows)
            .scroll((u16::try_from(app.view_scroll()).unwrap_or(u16::MAX), 0))
            .block(block),
        area,
    );
}

fn workbench_page_tabs(title: &str, max_width: usize, app: &TuiApp) -> Line<'static> {
    let pages: [(&str, bool); 7] = [
        ("1:Skills", title == "Discovered skills"),
        (
            "2:Models",
            title == "Models and routing health" || title == "Active model",
        ),
        ("3:Board", title == "DAG source and evidence board"),
        (
            "4:Prompts",
            title.contains("inspector") || title.contains("Direct request"),
        ),
        ("5:Diff", title == "Workspace manifest diff"),
        (
            "6:Status",
            title == "Bridge and jobs status" || title == "Handoff inbox",
        ),
        ("7:Help", title == "Help"),
    ];
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (label, active) in pages {
        let token = if active {
            format!(" [{label}] ")
        } else {
            format!(" {label} ")
        };
        let token_width = UnicodeWidthStr::width(token.as_str());
        if used + token_width > max_width {
            break;
        }
        used += token_width;
        let style = if app.theme() == TuiTheme::MinimalMono {
            if active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            }
        } else if active {
            Style::default()
                .fg(Color::Black)
                .bg(theme_accent(app.theme()))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Indexed(246))
        };
        spans.push(Span::styled(token, style));
    }
    Line::from(spans)
}

fn style_view_line(chunk: &str, is_selected: bool, app: &TuiApp) -> Line<'static> {
    if app.theme() == TuiTheme::MinimalMono {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default().fg(Color::Gray),
        ));
    }
    if let Some((before, after)) = chunk.split_once("● active") {
        let name_style = if is_selected {
            Style::default()
                .fg(theme_accent(app.theme()))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        };
        let detail_style = if is_selected {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Indexed(250))
        };
        return Line::from(vec![
            Span::styled(before.to_owned(), name_style),
            Span::styled(
                "● active".to_owned(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(after.to_owned(), detail_style),
        ]);
    }
    if let Some((before, after)) = chunk.split_once("○ deactivate") {
        let name_style = if is_selected {
            Style::default()
                .fg(theme_accent(app.theme()))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Indexed(246))
        };
        let badge_style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Indexed(243))
        };
        let detail_style = if is_selected {
            Style::default().fg(Color::Indexed(250))
        } else {
            Style::default().fg(Color::Indexed(244))
        };
        return Line::from(vec![
            Span::styled(before.to_owned(), name_style),
            Span::styled("○ deactivate".to_owned(), badge_style),
            Span::styled(after.to_owned(), detail_style),
        ]);
    }
    if chunk.starts_with("Skills Configuration Page")
        || chunk.starts_with("Workbench Pages")
        || chunk.starts_with("Goal:")
        || chunk.starts_with("1. ")
        || chunk.starts_with("2. ")
        || chunk.starts_with("3. ")
        || chunk.starts_with("4. ")
        || chunk.starts_with("5. ")
        || chunk.starts_with('[')
    {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default()
                .fg(theme_accent(app.theme()))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if chunk.starts_with("Controls:") {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default().fg(Color::Indexed(141)),
        ));
    }
    if chunk.starts_with("────") {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default().fg(Color::Indexed(240)),
        ));
    }
    if chunk.starts_with("Added:") {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default().fg(Color::Green),
        ));
    }
    if chunk.starts_with("Modified:") {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default().fg(Color::Yellow),
        ));
    }
    if chunk.starts_with("Deleted:") {
        return Line::from(Span::styled(
            chunk.to_owned(),
            Style::default().fg(Color::Red),
        ));
    }
    if (chunk.starts_with('/') || chunk.starts_with('@'))
        && let Some((command_part, detail_part)) = chunk.split_once(" · ")
    {
        return Line::from(vec![
            Span::styled(
                command_part.to_owned(),
                Style::default()
                    .fg(theme_accent(app.theme()))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {detail_part}"),
                Style::default().fg(Color::Indexed(250)),
            ),
        ]);
    }
    Line::from(chunk.to_owned())
}

fn fit(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut result = String::with_capacity(text.len().min(width * 4));
    let mut used = 0usize;
    for character in text.chars() {
        let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + char_width >= width {
            break;
        }
        used += char_width;
        result.push(character);
    }
    result.push('…');
    result
}

const fn theme_accent(theme: TuiTheme) -> Color {
    match theme {
        TuiTheme::CodexDark => Color::Cyan,
        TuiTheme::MinimalMono => Color::Gray,
        TuiTheme::RoyalPrism => Color::Magenta,
        TuiTheme::CyberMatrix => Color::Green,
    }
}

#[cfg(test)]
mod tests;
