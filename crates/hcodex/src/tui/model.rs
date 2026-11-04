//! TUI state, actions, and input transitions.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

use super::{CompletionItem, CompletionKind, TuiCatalog, is_quit_key};
use crate::Result;

const TRANSCRIPT_LIMIT: usize = 500;
const VIEW_HISTORY_LIMIT: usize = 50;

/// One typed operation requested by the user interface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum TuiAction {
    SubmitPrompt(String),
    RunDag(String),
    /// Ordinary text entered while DAG mode is on, before registered-prefix parsing.
    AutoDag(String),
    PlanDag(String),
    ToggleDag,
    EnterDag(Option<String>),
    ExitDag,
    ShowOverview,
    ShowPrompts(Option<String>),
    Back,
    ShowModels,
    ShowActiveModel,
    SelectModel(String),
    SelectAndPrompt {
        model: String,
        prompt: String,
    },
    ShowSkills,
    ToggleSkill(String),
    SetEffort(String),
    ConfigureEffect(String),
    Compact,
    SetPermissions(Option<String>),
    Resume(Option<String>),
    RunTriad(String),
    ShowStatus,
    ShowInbox,
    ShowDiff,
    Review(Option<String>),
    SetTheme(Option<String>),
    UnknownCommand(String),
    Clear,
    Help,
    Quit,
}

/// Runtime update applied to TUI state.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum TuiUpdate {
    /// A registered-skill prefix resolved a user action; update local view only.
    ValidatedAction(TuiAction),
    Transcript(TranscriptEntry),
    DagSnapshot(Vec<DagLaneView>),
    DagInterrupted(String),
    LaneState {
        node_id: String,
        state: LaneState,
        note: String,
        elapsed: Option<Duration>,
    },
    HeaderProgress {
        done: usize,
        total: usize,
    },
    Tokens(TokenUsage),
    ActiveModel(String),
    Effort(String),
    EffectSummary(String),
    PermissionMode(String),
    Theme(TuiTheme),
    CompactHistory,
    ActiveSkills(Vec<String>),
    View {
        title: String,
        body: Vec<String>,
    },
    RuntimeUnavailable {
        action: TuiAction,
        reason: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub context_limit: Option<u64>,
    pub current_context: Option<u64>,
    pub usage_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_input_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_limit_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_origin_model: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TuiTheme {
    #[default]
    CodexDark,
    MinimalMono,
    RoyalPrism,
    CyberMatrix,
}

impl TuiTheme {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CodexDark => "codex-dark",
            Self::MinimalMono => "minimal-mono",
            Self::RoyalPrism => "royal-prism",
            Self::CyberMatrix => "cyber-matrix",
        }
    }
}

impl TokenUsage {
    pub fn status(&self) -> String {
        format!("{} · {}", self.context_status(), self.counts_status())
    }

    pub fn counts_status(&self) -> String {
        match (self.input, self.output, self.usage_complete) {
            (Some(input), Some(output), true) => format!("{input} in · {output} out"),
            (Some(input), Some(output), false) => {
                format!("≥ {input} in · ≥ {output} out · partial")
            }
            _ => "tokens unknown".to_owned(),
        }
    }

    pub fn context_status(&self) -> String {
        match (self.context_limit, self.current_context) {
            (Some(limit), Some(used)) if limit > 0 => {
                let used_percent = (used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0);
                let origin = self
                    .context_origin_model
                    .as_deref()
                    .map_or_else(String::new, |model| format!(" [{model}]"));
                format!(
                    "Context{origin} {:.1}% left (ctx {:.1}%)",
                    100.0 - used_percent,
                    used_percent
                )
            }
            _ => match (self.request_input_bytes, self.request_limit_bytes) {
                (Some(input), Some(limit)) if limit > 0 => {
                    let used_percent = (input as f64 / limit as f64 * 100.0).clamp(0.0, 100.0);
                    let left_percent = (100.0 - used_percent).clamp(0.0, 100.0);
                    format!(
                        "Context input {input}/{limit} B · model window unreported · {left_percent:.1}% left ({used_percent:.1}%)"
                    )
                }
                _ => "Context unknown".to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptEntry {
    pub depth: u8,
    pub message: String,
}

impl TranscriptEntry {
    pub fn root(message: impl Into<String>) -> Self {
        Self {
            depth: 0,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneState {
    Waiting,
    HandedOut,
    Running,
    Stopped,
    Unverified,
    Accepted,
    Rejected,
}

impl LaneState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "○ Waiting",
            Self::HandedOut => "✓ Handed out",
            Self::Running => "● Running",
            Self::Stopped => "◑ STOPPED · reported done",
            Self::Unverified => "◇ Interrupted · unverified",
            Self::Accepted => "✓ SETTLED · verified",
            Self::Rejected => "✗ Verification rejected",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DagLaneView {
    pub id: String,
    pub layer: usize,
    pub dependencies: Vec<String>,
    pub title: String,
    pub summary: String,
    pub role: String,
    pub model: String,
    pub effort: String,
    pub state: LaneState,
    pub note: String,
    pub elapsed: Option<Duration>,
}

#[derive(Clone, Debug)]
struct ViewState {
    title: String,
    body: Vec<String>,
    scroll: usize,
}

/// Complete terminal state. It contains no provider or persistence authority.
#[derive(Debug)]
pub struct TuiApp {
    workspace: PathBuf,
    catalog: TuiCatalog,
    input: String,
    cursor: usize,
    completion_index: usize,
    completions: Vec<CompletionItem>,
    completion_dismissed: bool,
    completion_forced: bool,
    completion_navigated: bool,
    input_history: VecDeque<String>,
    history_index: Option<usize>,
    history_draft: String,
    transcript: VecDeque<TranscriptEntry>,
    lanes: Vec<DagLaneView>,
    lane_clocks: BTreeMap<String, (Instant, Duration)>,
    active_model: String,
    active_skills: Vec<String>,
    effort: String,
    effect_summary: String,
    permission_mode: String,
    theme: TuiTheme,
    dag_mode: bool,
    motion_enabled: bool,
    dag_transition: Option<(bool, usize)>,
    done: usize,
    total: usize,
    tokens: TokenUsage,
    current_view: Option<ViewState>,
    view_history: Vec<ViewState>,
    skill_cursor: usize,
    viewport_width: u16,
    quit: bool,
    started: Instant,
    shimmer_tick: usize,
}

impl TuiApp {
    pub fn new(workspace: PathBuf, catalog: TuiCatalog) -> Self {
        let motion_enabled = std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").ok().as_deref() != Some("dumb");
        Self::new_with_motion(workspace, catalog, motion_enabled)
    }

    pub(crate) fn new_with_motion(
        workspace: PathBuf,
        catalog: TuiCatalog,
        motion_enabled: bool,
    ) -> Self {
        Self {
            workspace,
            catalog,
            input: String::new(),
            cursor: 0,
            completion_index: 0,
            completions: Vec::new(),
            completion_dismissed: false,
            completion_forced: false,
            completion_navigated: false,
            input_history: VecDeque::new(),
            history_index: None,
            history_draft: String::new(),
            transcript: VecDeque::from([TranscriptEntry::root(
                "Heterarchy Codex Rust workbench ready",
            )]),
            lanes: Vec::new(),
            lane_clocks: BTreeMap::new(),
            active_model: "gemini-4-argon-high".to_owned(),
            active_skills: vec!["codex-heterarchy".to_owned()],
            effort: "high".to_owned(),
            effect_summary: "argon:high · claude:medium · codex:xhigh".to_owned(),
            permission_mode: "auto-edit".to_owned(),
            theme: TuiTheme::CodexDark,
            dag_mode: false,
            motion_enabled,
            dag_transition: None,
            done: 0,
            total: 0,
            tokens: TokenUsage::default(),
            current_view: None,
            view_history: Vec::new(),
            skill_cursor: 0,
            viewport_width: 80,
            quit: false,
            started: Instant::now(),
            shimmer_tick: 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<TuiAction>> {
        if is_quit_key(key)
            || key.code == KeyCode::Char('d')
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && self.input.is_empty()
        {
            self.quit = true;
            return Ok(Some(TuiAction::Quit));
        }
        if let Some(view) = &mut self.current_view {
            match key.code {
                KeyCode::PageDown => {
                    let rows = view_row_count(&view.body, self.viewport_width);
                    view.scroll = view.scroll.saturating_add(10).min(rows.saturating_sub(1));
                    return Ok(None);
                }
                KeyCode::PageUp => {
                    view.scroll = view.scroll.saturating_sub(10);
                    return Ok(None);
                }
                _ => {}
            }
        }
        if self.current_view.is_some()
            && self.input.is_empty()
            && self.completions.is_empty()
            && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
        {
            match key.code {
                KeyCode::Char('1') => return Ok(Some(TuiAction::ShowSkills)),
                KeyCode::Char('2') => return Ok(Some(TuiAction::ShowModels)),
                KeyCode::Char('3') => return Ok(Some(TuiAction::ShowOverview)),
                KeyCode::Char('4') => return Ok(Some(TuiAction::ShowPrompts(None))),
                KeyCode::Char('5') => return Ok(Some(TuiAction::ShowDiff)),
                KeyCode::Char('6') => return Ok(Some(TuiAction::ShowStatus)),
                KeyCode::Char('7') => return Ok(Some(TuiAction::Help)),
                _ => {}
            }
            if self
                .current_view
                .as_ref()
                .is_some_and(|view| view.title == "Discovered skills")
            {
                match key.code {
                    KeyCode::Up => {
                        self.move_skill_cursor(false);
                        return Ok(None);
                    }
                    KeyCode::Down => {
                        self.move_skill_cursor(true);
                        return Ok(None);
                    }
                    KeyCode::Char(' ') | KeyCode::Enter if key.modifiers.is_empty() => {
                        if let Some(action) = self.toggle_selected_skill_action() {
                            return Ok(Some(action));
                        }
                        return Ok(None);
                    }
                    KeyCode::Char('c') => {
                        return Ok(Some(TuiAction::ToggleSkill("core".to_owned())));
                    }
                    KeyCode::Char('d') => {
                        return Ok(Some(TuiAction::ToggleSkill("default".to_owned())));
                    }
                    KeyCode::Char('x') => {
                        return Ok(Some(TuiAction::ToggleSkill("clear".to_owned())));
                    }
                    _ => {}
                }
            }
        }
        if key.code == KeyCode::Esc {
            if !self.completions.is_empty() {
                self.completions.clear();
                self.completion_dismissed = true;
                self.completion_forced = false;
                self.completion_navigated = false;
            } else if self.current_view.is_some() {
                self.back();
            } else if !self.input.is_empty() {
                self.clear_input();
            } else {
                self.back();
                return Ok(Some(TuiAction::Back));
            }
            return Ok(None);
        }
        if self.current_view.is_some() && key.code == KeyCode::Backspace && self.input.is_empty() {
            self.back();
            return Ok(None);
        }
        match key.code {
            KeyCode::Up => self.previous_input_or_completion(),
            KeyCode::Down => self.next_input_or_completion(),
            KeyCode::BackTab => self.previous_input_or_completion(),
            KeyCode::Tab => {
                if self.completions.is_empty() {
                    self.completion_forced = true;
                    self.completion_dismissed = false;
                    self.refresh_completions();
                } else {
                    self.apply_completion();
                }
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.insert('\n');
            }
            KeyCode::Enter => return self.submit(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right if self.cursor == self.input.len() && !self.completions.is_empty() => {
                self.apply_completion()
            }
            KeyCode::Right => self.move_right(),
            KeyCode::Home => {
                self.cursor = 0;
                self.refresh_completions();
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                self.refresh_completions();
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.previous_input_or_completion()
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.next_input_or_completion()
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.clear_input()
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_previous_word()
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.insert(character);
            }
            _ => {}
        }
        Ok(None)
    }

    /// Inserts one bracketed-paste payload without interpreting embedded lines as submits.
    pub fn handle_paste(&mut self, text: &str) {
        self.reset_completion_intent();
        let sanitized = sanitize_paste(text);
        self.input.insert_str(self.cursor, &sanitized);
        self.cursor += sanitized.len();
        self.refresh_completions();
    }

    pub fn apply_update(&mut self, update: TuiUpdate) {
        match update {
            TuiUpdate::ValidatedAction(action) => self.apply_local_action(&action),
            TuiUpdate::Transcript(entry) => self.push_transcript(entry),
            TuiUpdate::DagSnapshot(lanes) => {
                self.lane_clocks.clear();
                for lane in &lanes {
                    if matches!(lane.state, LaneState::Running | LaneState::Stopped) {
                        self.lane_clocks.insert(
                            lane.id.clone(),
                            (Instant::now(), lane.elapsed.unwrap_or_default()),
                        );
                    }
                }
                self.done = lanes
                    .iter()
                    .filter(|lane| lane.state == LaneState::Accepted)
                    .count();
                self.total = lanes.len();
                self.lanes = lanes;
            }
            TuiUpdate::DagInterrupted(reason) => {
                for lane in &mut self.lanes {
                    if matches!(
                        lane.state,
                        LaneState::Running | LaneState::Stopped | LaneState::HandedOut
                    ) {
                        lane.state = LaneState::Unverified;
                        lane.note = format!(
                            "Execution interrupted; outcome requires reconciliation: {reason}"
                        );
                    }
                }
                self.lane_clocks.clear();
            }
            TuiUpdate::LaneState {
                node_id,
                state,
                note,
                elapsed,
            } => {
                if let Some(lane) = self.lanes.iter_mut().find(|lane| lane.id == node_id) {
                    if matches!(state, LaneState::Running | LaneState::Stopped) {
                        self.lane_clocks
                            .entry(node_id)
                            .or_insert_with(|| (Instant::now(), elapsed.unwrap_or_default()));
                    } else {
                        self.lane_clocks.remove(&node_id);
                    }
                    lane.state = state;
                    lane.note = note;
                    lane.elapsed = elapsed;
                }
                self.done = self
                    .lanes
                    .iter()
                    .filter(|lane| lane.state == LaneState::Accepted)
                    .count();
            }
            TuiUpdate::HeaderProgress { done, total } => {
                self.done = done;
                self.total = total;
            }
            TuiUpdate::Tokens(tokens) => self.tokens = tokens,
            TuiUpdate::ActiveModel(model) => self.active_model = model,
            TuiUpdate::Effort(effort) => self.effort = effort,
            TuiUpdate::EffectSummary(summary) => self.effect_summary = summary,
            TuiUpdate::PermissionMode(mode) => self.permission_mode = mode,
            TuiUpdate::Theme(theme) => self.theme = theme,
            TuiUpdate::CompactHistory => {
                if self.view_history.len() > 1 {
                    self.view_history.drain(..self.view_history.len() - 1);
                }
                while self.transcript.len() > 50 {
                    self.transcript.pop_front();
                }
            }
            TuiUpdate::ActiveSkills(skills) => {
                self.active_skills = skills;
                self.refresh_skills_view_in_place();
                self.refresh_completions();
            }
            TuiUpdate::View { title, body } => self.open_view(title, body),
            TuiUpdate::RuntimeUnavailable { action, reason } => self.push_transcript(
                TranscriptEntry::root(format!("Runtime unavailable for {action:?}: {reason}")),
            ),
        }
    }

    pub fn tick(&mut self) {
        self.shimmer_tick = self.shimmer_tick.wrapping_add(1);
        if self.dag_transition.is_some_and(|(entering, started)| {
            self.shimmer_tick.wrapping_sub(started).saturating_mul(16)
                >= if entering { 560 } else { 240 }
        }) {
            self.dag_transition = None;
        }
        for lane in &mut self.lanes {
            if let Some((started, base)) = self.lane_clocks.get(&lane.id) {
                lane.elapsed = Some(*base + started.elapsed());
            }
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn workspace(&self) -> &PathBuf {
        &self.workspace
    }
    pub fn git_branch(&self) -> Option<&str> {
        self.catalog.git_branch()
    }
    pub fn input(&self) -> &str {
        &self.input
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn completions(&self) -> &[CompletionItem] {
        &self.completions
    }
    pub fn completion_index(&self) -> usize {
        self.completion_index
    }
    pub fn completion_ghost(&self) -> Option<String> {
        if self.cursor != self.input.len() || self.input.is_empty() {
            return None;
        }
        let item = self.completions.get(self.completion_index)?;
        let typed = self.input.get(item.replace_start..self.cursor)?;
        if item.value.to_lowercase().starts_with(&typed.to_lowercase()) {
            Some(format!(
                "{}  [{}: {}]",
                item.value.get(typed.len()..).unwrap_or_default(),
                item.badge,
                item.detail
            ))
        } else {
            Some(format!(" → {} ({})", item.display_label, item.detail))
        }
    }
    pub fn transcript(&self) -> &VecDeque<TranscriptEntry> {
        &self.transcript
    }
    pub fn lanes(&self) -> &[DagLaneView] {
        &self.lanes
    }
    pub fn active_model(&self) -> &str {
        &self.active_model
    }
    pub fn active_skills(&self) -> &[String] {
        &self.active_skills
    }
    pub fn effort(&self) -> &str {
        &self.effort
    }
    pub fn permission_mode(&self) -> &str {
        &self.permission_mode
    }
    pub fn effect_summary(&self) -> &str {
        &self.effect_summary
    }
    pub fn theme(&self) -> TuiTheme {
        self.theme
    }
    pub fn dag_mode(&self) -> bool {
        self.dag_mode
    }
    pub(crate) fn motion_enabled(&self) -> bool {
        self.motion_enabled
    }
    pub(crate) fn dag_transition(&self) -> Option<(bool, usize)> {
        self.dag_transition.map(|(entering, started)| {
            (
                entering,
                self.shimmer_tick.wrapping_sub(started).saturating_mul(16) / 40,
            )
        })
    }
    pub fn progress(&self) -> (usize, usize) {
        (self.done, self.total)
    }
    pub fn token_status(&self) -> String {
        self.tokens.status()
    }
    pub fn context_status(&self) -> String {
        self.tokens.context_status()
    }
    pub fn token_counts_status(&self) -> String {
        self.tokens.counts_status()
    }
    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }
    pub fn shimmer_tick(&self) -> usize {
        self.shimmer_tick
    }
    pub fn current_view(&self) -> Option<(&str, &[String])> {
        self.current_view
            .as_ref()
            .map(|view| (view.title.as_str(), view.body.as_slice()))
    }
    pub fn view_scroll(&self) -> usize {
        self.current_view.as_ref().map_or(0, |view| view.scroll)
    }
    pub fn skill_cursor(&self) -> usize {
        self.skill_cursor
    }
    pub fn selected_skill_line_index(&self) -> Option<usize> {
        let view = self.current_view.as_ref()?;
        if view.title != "Discovered skills" {
            return None;
        }
        skill_line_indices(&view.body)
            .get(self.skill_cursor)
            .copied()
    }
    pub fn set_viewport_width(&mut self, width: u16) {
        if self.viewport_width == width {
            return;
        }
        self.viewport_width = width;
        if let Some(view) = &mut self.current_view {
            let rows = view_row_count(&view.body, width);
            view.scroll = view.scroll.min(rows.saturating_sub(1));
        }
    }

    fn insert(&mut self, character: char) {
        self.reset_completion_intent();
        self.input.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        self.refresh_completions();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.reset_completion_intent();
        let previous = self.input[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.input.drain(previous..self.cursor);
        self.cursor = previous;
        self.refresh_completions();
    }

    fn delete(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        self.reset_completion_intent();
        let next = self.input[self.cursor..]
            .char_indices()
            .nth(1)
            .map_or(self.input.len(), |(index, _)| self.cursor + index);
        self.input.drain(self.cursor..next);
        self.refresh_completions();
    }

    fn move_left(&mut self) {
        self.cursor = self.input[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.refresh_completions();
    }

    fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = self.input[self.cursor..]
                .char_indices()
                .nth(1)
                .map_or(self.input.len(), |(index, _)| self.cursor + index);
        }
        self.refresh_completions();
    }

    fn refresh_completions(&mut self) {
        if self.completion_dismissed {
            self.completions.clear();
            return;
        }
        self.completions = self.catalog.complete_with_state(
            &self.input,
            self.cursor,
            64,
            &self.active_skills,
            self.completion_forced,
        );
        self.completion_index = 0;
        self.completion_navigated = false;
    }

    fn reset_completion_intent(&mut self) {
        self.completion_dismissed = false;
        self.completion_forced = false;
        self.completion_navigated = false;
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.completions.clear();
        self.history_index = None;
        self.history_draft.clear();
        self.reset_completion_intent();
    }

    fn delete_previous_word(&mut self) {
        let trimmed = self.input[..self.cursor].trim_end_matches(char::is_whitespace);
        let cut = trimmed
            .char_indices()
            .rev()
            .find(|(_, value)| value.is_whitespace())
            .map_or(0, |(index, value)| index + value.len_utf8());
        self.input.drain(cut..self.cursor);
        self.cursor = cut;
        self.reset_completion_intent();
        self.refresh_completions();
    }

    fn previous_input_or_completion(&mut self) {
        if !self.completions.is_empty() {
            self.select_previous_completion();
            return;
        }
        if self.input_history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => {
                self.history_draft = self.input.clone();
                self.input_history.len() - 1
            }
            Some(index) => index.saturating_sub(1),
        };
        self.history_index = Some(index);
        self.input.clone_from(&self.input_history[index]);
        self.cursor = self.input.len();
        self.reset_completion_intent();
        self.completions.clear();
        self.completion_index = 0;
    }

    fn next_input_or_completion(&mut self) {
        if !self.completions.is_empty() {
            self.select_next_completion();
            return;
        }
        if let Some(index) = self.history_index {
            if index + 1 < self.input_history.len() {
                self.history_index = Some(index + 1);
                self.input.clone_from(&self.input_history[index + 1]);
            } else {
                self.history_index = None;
                self.input.clone_from(&self.history_draft);
            }
            self.cursor = self.input.len();
            self.reset_completion_intent();
            self.completions.clear();
            self.completion_index = 0;
        } else if !self.input.trim().is_empty() {
            self.completion_dismissed = false;
            self.refresh_completions();
        }
    }

    fn select_previous_completion(&mut self) {
        self.completion_index = if self.completion_index == 0 {
            self.completions.len() - 1
        } else {
            self.completion_index - 1
        };
        self.completion_navigated = true;
    }

    fn select_next_completion(&mut self) {
        self.completion_index = (self.completion_index + 1) % self.completions.len();
        self.completion_navigated = true;
    }

    fn apply_completion(&mut self) {
        let Some(item) = self.completions.get(self.completion_index).cloned() else {
            return;
        };
        let token_start = item.replace_start;
        if self.input.get(token_start..self.cursor).is_none() {
            return;
        }
        let mut inserted = item.value;
        let takes_argument = matches!(
            inserted.as_str(),
            "/use"
                | "/model"
                | "/skill"
                | "/skills"
                | "/dag-mode"
                | "/effect"
                | "/effort"
                | "/reasoning"
                | "/dag-effect"
                | "/permissions"
                | "/approvals"
                | "/prompts"
                | "/promt"
                | "/trace"
                | "/theme"
        );
        let directory = inserted.ends_with('/') || inserted.ends_with("/\"");
        if (item.kind == CompletionKind::Skill
            || item.kind == CompletionKind::File && !directory
            || item.kind == CompletionKind::Command && (!item.submit_on_enter || takes_argument))
            && !self.input[self.cursor..].starts_with(char::is_whitespace)
        {
            inserted.push(' ');
        }
        self.input
            .replace_range(token_start..self.cursor, &inserted);
        self.cursor = token_start + inserted.len();
        self.reset_completion_intent();
        self.refresh_completions();
    }

    fn submit(&mut self) -> Result<Option<TuiAction>> {
        let cleaned = self.input.trim();
        let exact_command = self.catalog.is_exact_command(cleaned);
        let single_trigger = cleaned.starts_with(['/', '$'])
            && (!cleaned.contains(char::is_whitespace)
                || ["/use ", "/model ", "/dag-mode ", "/skill ", "/skills "]
                    .iter()
                    .any(|prefix| cleaned.starts_with(prefix)))
            && !exact_command;
        if !self.completions.is_empty() && (self.completion_navigated || single_trigger) {
            if self.completions[self.completion_index].submit_on_enter {
                let selected = &self.completions[self.completion_index];
                self.input
                    .replace_range(selected.replace_start..self.cursor, &selected.value);
                self.cursor = selected.replace_start + selected.value.len();
            } else {
                self.apply_completion();
                return Ok(None);
            }
        }
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return Ok(None);
        }
        if self.input_history.back() != Some(&text) {
            if self.input_history.len() == 100 {
                self.input_history.pop_front();
            }
            self.input_history.push_back(text.clone());
        }
        self.clear_input();
        let action = match action_from_input(&text) {
            TuiAction::UnknownCommand(command) if self.catalog.is_skill_reference(&command) => {
                if self.dag_mode {
                    TuiAction::AutoDag(text.clone())
                } else {
                    TuiAction::SubmitPrompt(text.clone())
                }
            }
            TuiAction::SubmitPrompt(prompt) if self.dag_mode && !text.starts_with('/') => {
                TuiAction::AutoDag(prompt)
            }
            action => action,
        };
        self.apply_local_action(&action);
        Ok(Some(action))
    }

    fn apply_local_action(&mut self, action: &TuiAction) {
        match action {
            TuiAction::ToggleDag => self.set_dag_mode(!self.dag_mode),
            TuiAction::EnterDag(goal) => {
                self.set_dag_mode(true);
                if let Some(goal) = goal {
                    self.push_transcript(TranscriptEntry::root(format!("> you: {goal}")));
                }
            }
            TuiAction::ExitDag => self.set_dag_mode(false),
            TuiAction::SelectModel(model) => self.active_model.clone_from(model),
            TuiAction::SelectAndPrompt { model, prompt } => {
                self.active_model.clone_from(model);
                self.push_transcript(TranscriptEntry::root(format!("> you: {prompt}")));
            }
            TuiAction::SetEffort(effort) => self.effort.clone_from(effort),
            TuiAction::Clear => {
                self.transcript.clear();
                self.lanes.clear();
                self.done = 0;
                self.total = 0;
                self.tokens = TokenUsage::default();
                self.current_view = None;
                self.view_history.clear();
            }
            TuiAction::Back => self.back(),
            TuiAction::Quit => self.quit = true,
            TuiAction::SubmitPrompt(prompt)
            | TuiAction::RunDag(prompt)
            | TuiAction::AutoDag(prompt)
            | TuiAction::PlanDag(prompt) => {
                self.push_transcript(TranscriptEntry::root(format!("> you: {prompt}")));
            }
            _ => {}
        }
    }

    fn set_dag_mode(&mut self, enabled: bool) {
        self.dag_mode = enabled;
        self.dag_transition = self.motion_enabled.then_some((enabled, self.shimmer_tick));
    }

    fn open_view(&mut self, title: String, body: Vec<String>) {
        let body: Vec<String> = body
            .into_iter()
            .flat_map(|line| line.split('\n').map(str::to_owned).collect::<Vec<_>>())
            .collect();
        if title == "Discovered skills" {
            let count = skill_line_indices(&body).len();
            self.skill_cursor = self.skill_cursor.min(count.saturating_sub(1));
        }
        let mut preserved_scroll = 0usize;
        if let Some(current) = self.current_view.take() {
            if current.title == title {
                preserved_scroll = current
                    .scroll
                    .min(view_row_count(&body, self.viewport_width).saturating_sub(1));
            } else {
                if self.view_history.len() == VIEW_HISTORY_LIMIT {
                    self.view_history.remove(0);
                }
                self.view_history.push(current);
            }
        }
        self.current_view = Some(ViewState {
            title,
            body,
            scroll: preserved_scroll,
        });
    }

    fn move_skill_cursor(&mut self, down: bool) {
        let Some(view) = &mut self.current_view else {
            return;
        };
        if view.title != "Discovered skills" {
            return;
        }
        let indices = skill_line_indices(&view.body);
        if indices.is_empty() {
            self.skill_cursor = 0;
            return;
        }
        self.skill_cursor = if down {
            (self.skill_cursor + 1).min(indices.len() - 1)
        } else {
            self.skill_cursor.saturating_sub(1)
        };
        let target_line = indices[self.skill_cursor];
        if self.skill_cursor == 0 {
            view.scroll = 0;
        } else if target_line < view.scroll {
            view.scroll = target_line;
        } else if target_line >= view.scroll.saturating_add(12) {
            view.scroll = target_line.saturating_sub(11);
        }
    }

    fn toggle_selected_skill_action(&self) -> Option<TuiAction> {
        let view = self.current_view.as_ref()?;
        if view.title != "Discovered skills" {
            return None;
        }
        let indices = skill_line_indices(&view.body);
        let &line_index = indices.get(self.skill_cursor)?;
        let (name, is_active) = parse_skill_row(view.body.get(line_index)?)?;
        let verb = if is_active { "off" } else { "on" };
        Some(TuiAction::ToggleSkill(format!("{verb} {name}")))
    }

    fn refresh_skills_view_in_place(&mut self) {
        let active_set: std::collections::BTreeSet<&str> =
            self.active_skills.iter().map(String::as_str).collect();
        for view in self
            .view_history
            .iter_mut()
            .chain(self.current_view.iter_mut())
        {
            if view.title != "Discovered skills" {
                continue;
            }
            let mut total_skills = 0usize;
            let mut active_count = 0usize;
            for line in &mut view.body {
                let Some((name, _)) = parse_skill_row(line).map(|(n, a)| (n.to_owned(), a)) else {
                    continue;
                };
                total_skills += 1;
                if active_set.contains(name.as_str()) {
                    active_count += 1;
                    if line.contains("○ deactivate") {
                        *line = line.replacen("○ deactivate", "● active    ", 1);
                    }
                } else if line.contains("● active    ") {
                    *line = line.replacen("● active    ", "○ deactivate", 1);
                } else if line.contains("● active") {
                    *line = line.replacen("● active", "○ deactivate", 1);
                }
            }
            if let Some(first) = view.body.first_mut()
                && first.starts_with("Skills Configuration Page · Active: ")
            {
                let suffix = first
                    .split_once(" · ")
                    .and_then(|(_, rest)| rest.split_once(" · "))
                    .map_or("Ready for prompt & DAG injection", |(_, tail)| tail);
                *first = format!(
                    "Skills Configuration Page · Active: {active_count}/{total_skills} · {suffix}"
                );
            }
        }
    }

    fn back(&mut self) {
        self.current_view = self.view_history.pop();
        if let Some(view) = &mut self.current_view {
            view.scroll = view
                .scroll
                .min(view_row_count(&view.body, self.viewport_width).saturating_sub(1));
        }
    }

    fn push_transcript(&mut self, entry: TranscriptEntry) {
        if self.transcript.len() == TRANSCRIPT_LIMIT {
            self.transcript.pop_front();
        }
        self.transcript.push_back(entry);
    }
}

fn view_row_count(body: &[String], terminal_width: u16) -> usize {
    let inner = super::text::inspector_width(terminal_width)
        .saturating_sub(2)
        .max(1) as usize;
    body.iter()
        .map(|line| super::text::wrap_display_line(line, inner).len())
        .fold(0, usize::saturating_add)
}

fn skill_line_indices(body: &[String]) -> Vec<usize> {
    body.iter()
        .enumerate()
        .filter_map(|(index, line)| parse_skill_row(line).map(|_| index))
        .collect()
}

pub(crate) fn parse_skill_row(line: &str) -> Option<(&str, bool)> {
    let trimmed = line.trim_start_matches([' ', '▸']);
    let rest = trimmed.strip_prefix('$')?;
    let name = rest.split_whitespace().next()?;
    if name.is_empty() {
        return None;
    }
    if line.contains("● active") {
        Some((name, true))
    } else if line.contains("○ deactivate") {
        Some((name, false))
    } else {
        None
    }
}

fn sanitize_paste(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|character| !character.is_control() || matches!(*character, '\n' | '\t'))
        .collect()
}

/// Parses one composer or non-interactive command into a typed action.
pub fn action_from_input(text: &str) -> TuiAction {
    let mut parts = text.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().to_ascii_lowercase();
    let argument = parts.next().map(str::trim).filter(|item| !item.is_empty());
    match command.as_str() {
        "q" | "exit" | "quit" => TuiAction::Quit,
        "?" => TuiAction::Help,
        "/dag-ulw" | "/dag" | "/triad"
            if argument.is_some_and(|value| value.contains("--plan-only")) =>
        {
            TuiAction::PlanDag(
                argument
                    .unwrap_or_default()
                    .replace("--plan-only", "")
                    .trim()
                    .to_owned(),
            )
        }
        "/dag-ulw" | "/dag-run" | "/dag" | "/triad" => TuiAction::RunDag(
            argument
                .unwrap_or_default()
                .replace("--plan-only", "")
                .trim()
                .to_owned(),
        ),
        "/dag-plan" => TuiAction::PlanDag(argument.unwrap_or_default().to_owned()),
        "/dag-mode" if argument.is_some_and(is_dag_exit_token) => TuiAction::ExitDag,
        "/dag-mode" => TuiAction::EnterDag(
            argument
                .filter(|value| {
                    !matches!(
                        value.to_ascii_lowercase().as_str(),
                        "on" | "start" | "true" | "1"
                    )
                })
                .map(str::to_owned),
        ),
        "/dag-exit" => TuiAction::ExitDag,
        "/omh" | "/board" | "/todo" | "/plan" => TuiAction::ShowOverview,
        "/prompts" | "/promt" | "/trace" if argument.is_some_and(is_back_token) => TuiAction::Back,
        "/prompts" | "/promt" | "/trace" | "/prompt-trace" => {
            TuiAction::ShowPrompts(argument.map(str::to_owned))
        }
        "/back" | "/prev" | "/return" | "/b" => TuiAction::Back,
        "/models" | "/lanes" => TuiAction::ShowModels,
        "/model" | "/use" if argument.is_none() => TuiAction::ShowActiveModel,
        "/model" | "/use" => TuiAction::SelectModel(argument.unwrap_or_default().to_owned()),
        "/ask" | "/prompt" => TuiAction::SubmitPrompt(argument.unwrap_or_default().to_owned()),
        "/sonnet" => model_action("claude-sonnet-5-5-medium", argument),
        "/opus" => model_action("claude-opus-5-5-medium", argument),
        "/argon" => model_action("gemini-4-argon-high", argument),
        "/flash" => model_action("gemini-3.8-flash-high", argument),
        "/skills" if argument.is_none() => TuiAction::ShowSkills,
        "/skills" => TuiAction::ToggleSkill(argument.unwrap_or_default().to_owned()),
        "/skill" => TuiAction::ToggleSkill(
            argument
                .unwrap_or_default()
                .trim_start_matches('$')
                .to_owned(),
        ),
        "/effect" | "/effort" | "/reasoning" => {
            TuiAction::ConfigureEffect(argument.unwrap_or_default().to_owned())
        }
        "/dag-effect" => TuiAction::ConfigureEffect(argument.unwrap_or_default().to_owned()),
        "/compact" => TuiAction::Compact,
        "/permissions" | "/approvals" => TuiAction::SetPermissions(argument.map(str::to_owned)),
        "/resume" => TuiAction::Resume(argument.map(str::to_owned)),
        "/triad-review" => TuiAction::RunTriad(argument.unwrap_or_default().to_owned()),
        "/status" | "/jobs" => TuiAction::ShowStatus,
        "/inbox" => TuiAction::ShowInbox,
        "/diff" => TuiAction::ShowDiff,
        "/review" => TuiAction::Review(argument.map(str::to_owned)),
        "/clear" => TuiAction::Clear,
        "/help" => TuiAction::Help,
        "/theme" => TuiAction::SetTheme(argument.map(str::to_owned)),
        "/quit" | "/exit" => TuiAction::Quit,
        _ if command.starts_with('$') => TuiAction::SubmitPrompt(text.to_owned()),
        _ if command.starts_with('/') => TuiAction::UnknownCommand(command.to_owned()),
        _ => TuiAction::SubmitPrompt(text.to_owned()),
    }
}

fn model_action(model: &str, prompt: Option<&str>) -> TuiAction {
    prompt.map_or_else(
        || TuiAction::SelectModel(model.to_owned()),
        |prompt| TuiAction::SelectAndPrompt {
            model: model.to_owned(),
            prompt: prompt.to_owned(),
        },
    )
}

fn is_dag_exit_token(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "exit" | "off" | "stop" | "quit" | "false" | "0"
    )
}

fn is_back_token(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "back" | "exit" | "prev" | "return" | "q"
    )
}

#[cfg(test)]
mod tests;
