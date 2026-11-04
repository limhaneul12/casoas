//! Real application adapter between the interactive terminal and [`Gateway`].

mod helpers;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};

use crate::Result;
use crate::context::SkillRegistry;
use crate::dag::{
    DagExecutionEvent, DagPlan, DagPlanner, DagSettlementVerifier, WorkspaceManifest,
    handle_request_with_observer, prepare_interactive_request,
};
use crate::gateway::Gateway;
use crate::tui::{
    ActionFuture, ActionHandler, LaneState, TokenUsage, TranscriptEntry, TuiAction, TuiUpdate,
};

use self::helpers::*;

const HISTORY_LIMIT: usize = 50;
const VIEW_BYTE_LIMIT: usize = 24_000;
const CODEX_DIRECT_MODEL: &str = "gpt-6-astra";

#[derive(Clone, Debug)]
struct PromptRecord {
    request: String,
    response: Value,
}

struct HandlerState {
    active_model: String,
    effort: String,
    skills: SkillRegistry,
    tokens_in: u64,
    tokens_out: u64,
    context_window: Option<u64>,
    current_context_tokens: Option<u64>,
    seen_usage_requests: BTreeSet<String>,
    usage_known_seen: bool,
    usage_unknown_seen: bool,
    history: VecDeque<PromptRecord>,
    permission_mode: String,
    effects: EffectConfig,
    last_dag_goal: Option<String>,
    last_dag_id: Option<String>,
    last_dag_request: Option<Value>,
    last_dag_result: Option<Value>,
    baseline_manifest: Option<WorkspaceManifest>,
}

/// Stateful, concurrency-safe adapter for the terminal action boundary.
#[derive(Clone)]
pub struct AppHandler {
    gateway: Gateway,
    workspace: PathBuf,
    state: Arc<Mutex<HandlerState>>,
    worktree_default: bool,
}

impl AppHandler {
    pub fn new(gateway: Gateway, workspace: PathBuf) -> Result<Self> {
        let home = std::env::var_os("HCODEX_HOME")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from);
        let skills = SkillRegistry::discover(&workspace, home.as_deref())?;
        Ok(Self {
            gateway,
            workspace,
            worktree_default: false,
            state: Arc::new(Mutex::new(HandlerState {
                active_model: "gemini-4-argon-high".to_owned(),
                effort: "high".to_owned(),
                skills,
                tokens_in: 0,
                tokens_out: 0,
                context_window: None,
                current_context_tokens: None,
                seen_usage_requests: BTreeSet::new(),
                usage_known_seen: false,
                usage_unknown_seen: false,
                history: VecDeque::new(),
                permission_mode: "auto-edit".to_owned(),
                effects: EffectConfig::default(),
                last_dag_goal: None,
                last_dag_id: None,
                last_dag_request: None,
                last_dag_result: None,
                baseline_manifest: None,
            })),
        })
    }

    fn with_worktree_default(mut self, enabled: bool) -> Self {
        self.worktree_default = enabled;
        self
    }

    async fn prepare_execution_request(
        &self,
        request: &Value,
        explicit_worktree: bool,
    ) -> Result<Value> {
        let mut prepared = prepare_interactive_request(request).await?;
        if explicit_worktree || self.worktree_default {
            prepared["spawn_plan"]["isolation_mode"] = Value::String("worktree".to_owned());
        }
        Ok(prepared)
    }

    async fn ensure_baseline_manifest(&self) {
        if self.state.lock().await.baseline_manifest.is_some() {
            return;
        }
        let root = self.workspace.clone();
        if let Ok(Ok(manifest)) =
            tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&root)).await
        {
            self.state
                .lock()
                .await
                .baseline_manifest
                .get_or_insert(manifest);
        }
    }

    fn dispatch(&self, action: TuiAction, updates: mpsc::Sender<TuiUpdate>) -> ActionFuture {
        let handler = self.clone();
        Box::pin(async move {
            let Some(action) = handler.registered_skill_action(action, &updates).await? else {
                return Ok(());
            };
            match action {
                TuiAction::SubmitPrompt(prompt) => handler.submit_prompt(prompt, updates).await,
                TuiAction::RunDag(goal) | TuiAction::AutoDag(goal) => {
                    handler.plan_dag(goal, true, None, updates).await
                }
                TuiAction::PlanDag(goal) => handler.plan_dag(goal, false, None, updates).await,
                TuiAction::ToggleDag => {
                    send_transcript(
                        &updates,
                        "DAG mode toggled; execution remains receipt-gated",
                    )
                    .await
                }
                TuiAction::EnterDag(goal) => {
                    let active = {
                        let mut state = handler.state.lock().await;
                        if state.skills.skills().contains_key("codex-heterarchy")
                            && !state
                                .skills
                                .active()
                                .iter()
                                .any(|name| name == "codex-heterarchy")
                        {
                            let mut selected = state.skills.clone();
                            selected.activate("codex-heterarchy")?;
                            let preflight = selected.clone();
                            let _body = tokio::task::spawn_blocking(move || {
                                preflight.active_context(32_000)
                            })
                            .await??;
                            state.skills = selected;
                        }
                        state.skills.active().to_vec()
                    };
                    updates.send(TuiUpdate::ActiveSkills(active)).await?;
                    send_transcript(&updates, "DAG mode enabled").await?;
                    if let Some(goal) = goal {
                        handler.plan_dag(goal, true, None, updates).await
                    } else {
                        Ok(())
                    }
                }
                TuiAction::ExitDag => send_transcript(&updates, "DAG mode exited").await,
                TuiAction::ShowOverview => handler.show_overview(updates).await,
                TuiAction::ShowWorkBoard(limit) => handler.show_workboard(limit, updates).await,
                TuiAction::RunCouncil(proposal) => handler.run_council(proposal, updates).await,
                TuiAction::ShowVotes(collaboration_id) => {
                    handler.show_votes(collaboration_id, updates).await
                }
                TuiAction::ShowRecovery(job_id) => handler.show_recovery(job_id, updates).await,
                TuiAction::ShowPrompts(selector) => handler.show_prompts(selector, updates).await,
                TuiAction::Back => Ok(()),
                TuiAction::ShowModels => handler.show_models(updates).await,
                TuiAction::ShowActiveModel => {
                    let active = {
                        let state = handler.state.lock().await;
                        format!("{}:{}", state.active_model, state.effort)
                    };
                    show_static_view(&updates, "Active model", vec![active]).await
                }
                TuiAction::SelectModel(model) => handler.select_model(model, updates).await,
                TuiAction::SelectAndPrompt { model, prompt } => {
                    handler.select_model(model, updates.clone()).await?;
                    handler.submit_prompt(prompt, updates).await
                }
                TuiAction::ShowSkills => handler.show_skills(updates).await,
                TuiAction::ToggleSkill(skill) => handler.toggle_skill(skill, updates).await,
                TuiAction::SetEffort(effort) => handler.set_effort(effort, updates).await,
                TuiAction::ConfigureEffect(effect) => {
                    handler.configure_effect(effect, updates).await
                }
                TuiAction::Compact => handler.compact(updates).await,
                TuiAction::SetPermissions(mode) => handler.set_permissions(mode, updates).await,
                TuiAction::Resume(dag_id) => handler.resume(dag_id, updates).await,
                TuiAction::RunTriad(goal) => handler.run_triad(goal, updates).await,
                TuiAction::ShowStatus => handler.show_status(updates).await,
                TuiAction::ShowInbox => handler.show_inbox(updates).await,
                TuiAction::ShowDiff => handler.show_diff(updates).await,
                TuiAction::Review(focus) => handler.review(focus, updates).await,
                TuiAction::SetTheme(theme) => handler.set_theme(theme, updates).await,
                TuiAction::UnknownCommand(command) => {
                    Err(format!("unknown slash command: {command}").into())
                }
                TuiAction::Clear => {
                    let mut state = handler.state.lock().await;
                    state.history.clear();
                    state.last_dag_request = None;
                    state.last_dag_result = None;
                    state.tokens_in = 0;
                    state.tokens_out = 0;
                    state.context_window = None;
                    state.current_context_tokens = None;
                    state.seen_usage_requests.clear();
                    state.usage_known_seen = false;
                    state.usage_unknown_seen = false;
                    Ok(())
                }
                TuiAction::Help => show_static_view(&updates, "Help", help_lines()).await,
                TuiAction::Quit => Ok(()),
            }
        })
    }

    async fn registered_skill_action(
        &self,
        action: TuiAction,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<Option<TuiAction>> {
        let (text, wrap, rewrite_command): (String, fn(String) -> TuiAction, bool) = match action {
            TuiAction::SubmitPrompt(text) => (text, TuiAction::SubmitPrompt, true),
            TuiAction::AutoDag(text) => (text, TuiAction::RunDag, true),
            TuiAction::RunDag(text) => (text, TuiAction::RunDag, false),
            TuiAction::PlanDag(text) => (text, TuiAction::PlanDag, false),
            TuiAction::EnterDag(Some(text)) => {
                (text, |text| TuiAction::EnterDag(Some(text)), false)
            }
            other => return Ok(Some(other)),
        };
        let (matched, cleaned, active) = {
            let mut state = self.state.lock().await;
            let mut selected = state.skills.clone();
            let (matched, cleaned) = selected.extract_mentions_where(&text, inline_skill_token);
            if !matched.is_empty() {
                let preflight = selected.clone();
                let _context =
                    tokio::task::spawn_blocking(move || preflight.active_context(32_000)).await??;
                state.skills = selected;
            }
            (matched, cleaned, state.skills.active().to_vec())
        };
        if matched.is_empty() {
            return Ok(Some(wrap(text)));
        }
        updates.send(TuiUpdate::ActiveSkills(active)).await?;
        if cleaned.is_empty() {
            send_transcript(
                updates,
                format!("Activated registered skills: {}", matched.join(", ")),
            )
            .await?;
            return Ok(None);
        }
        if rewrite_command {
            let parsed = crate::tui::action_from_input(&cleaned);
            if cleaned.starts_with('/') || !matches!(parsed, TuiAction::SubmitPrompt(_)) {
                if matches!(
                    &parsed,
                    TuiAction::EnterDag(_)
                        | TuiAction::ExitDag
                        | TuiAction::ToggleDag
                        | TuiAction::Clear
                        | TuiAction::Back
                        | TuiAction::Quit
                ) {
                    updates
                        .send(TuiUpdate::ValidatedAction(parsed.clone()))
                        .await?;
                }
                return Ok(Some(parsed));
            }
        }
        Ok(Some(wrap(cleaned)))
    }

    async fn execution_gateway(&self) -> Gateway {
        let mode = self.state.lock().await.permission_mode.clone();
        self.gateway.clone().with_sandbox(permission_sandbox(&mode))
    }

    async fn submit_prompt(&self, prompt: String, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        if prompt.trim().is_empty() {
            return Err("prompt cannot be empty".into());
        }
        self.ensure_baseline_manifest().await;
        let (model, effort, skills, clean_prompt, active_skills, skills_changed) = {
            let mut state = self.state.lock().await;
            let (activated, cleaned) = state
                .skills
                .extract_mentions_where(&prompt, inline_skill_token);
            let clean_prompt = if activated.is_empty() || cleaned.is_empty() {
                prompt.clone()
            } else {
                cleaned
            };
            (
                state.active_model.clone(),
                state.effort.clone(),
                state.skills.clone(),
                clean_prompt,
                state.skills.active().to_vec(),
                !activated.is_empty(),
            )
        };
        let skill_context =
            tokio::task::spawn_blocking(move || skills.active_context(32_000)).await??;
        if skills_changed {
            updates
                .send(TuiUpdate::ActiveSkills(active_skills.clone()))
                .await?;
        }
        send_transcript(
            &updates,
            format!(
                "Dispatching {model}:{effort} with {} active skill(s)",
                active_skills.len()
            ),
        )
        .await?;
        updates
            .send(TuiUpdate::HeaderProgress { done: 0, total: 1 })
            .await?;
        let (tool, input) = direct_request(
            &model,
            &effort,
            &clean_prompt,
            &skill_context,
            &self.workspace,
        );
        let response = self.execution_gateway().await.call(tool, input).await?;
        self.record_response(clean_prompt, response, updates).await
    }

    async fn plan_dag(
        &self,
        goal: String,
        execute: bool,
        resume_id: Option<String>,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let (mut goal, worktree) = worktree_goal(&goal);
        if let Some(id) = inline_resume_id(&goal)? {
            if !execute {
                return Err("resume executes a stored plan; use /resume <campaign_id>".into());
            }
            if worktree {
                return Err("resume uses stored isolation; omit the --worktree flag".into());
            }
            return self.resume(Some(id.to_owned()), updates).await;
        }
        if execute {
            self.ensure_baseline_manifest().await;
        }
        if goal == "--live" {
            goal.clear();
        } else if let Some(rest) = goal.strip_prefix("--live ") {
            goal = rest.trim().to_owned();
        }
        if goal.is_empty() {
            goal = "Verify and refactor module contracts".to_owned();
        }
        goal = match self
            .registered_skill_action(TuiAction::PlanDag(goal), &updates)
            .await?
        {
            Some(TuiAction::PlanDag(cleaned)) => cleaned,
            None => return Ok(()),
            Some(_) => {
                return Err(
                    "use the remaining slash command directly instead of as a DAG goal".into(),
                );
            }
        };
        send_transcript(&updates, "Compiling a workspace-grounded DAG plan").await?;
        let plan_goal = goal.clone();
        let plan_workspace = self.workspace.clone();
        let mut plan =
            tokio::task::spawn_blocking(move || DagPlanner::generate(&plan_goal, &plan_workspace))
                .await??;
        let (effects, skills, permission_mode) = {
            let mut state = self.state.lock().await;
            state.last_dag_goal = Some(goal.clone());
            (
                state.effects.clone(),
                state.skills.clone(),
                state.permission_mode.clone(),
            )
        };
        let skill_context =
            tokio::task::spawn_blocking(move || skills.active_context(32_000)).await??;
        apply_effects_to_plan(&mut plan, &effects);
        ensure_plan_permission(&permission_mode, &plan, execute)?;
        let lanes = dag_lanes(&plan);
        updates.send(TuiUpdate::DagSnapshot(lanes)).await?;
        let status = if plan.hazards.is_empty() && execute {
            format!(
                "Graph defined: {} lanes in {} layers; execution starting",
                plan.nodes.len(),
                plan.waves.len()
            )
        } else if plan.hazards.is_empty() {
            format!(
                "Plan ready: {} lanes in {} layers; nothing has run",
                plan.nodes.len(),
                plan.waves.len()
            )
        } else {
            format!(
                "Graph blocked before execution by {} static hazard(s)",
                plan.hazards.len()
            )
        };
        send_transcript(&updates, status).await?;
        for hazard in &plan.hazards {
            updates
                .send(TuiUpdate::Transcript(TranscriptEntry {
                    depth: 1,
                    message: format!("{:?}: {}", hazard.kind, hazard.message),
                }))
                .await?;
        }
        let mut request = json!({
            "campaign_goal": goal,
            "nodes": dag_request_nodes(&plan, &skill_context),
            "workspace_dir": self.workspace,
            "plan_only": !execute,
            "timeout_seconds": 3600.0,
            "max_input_bytes": 256000,
            "resume_dag_id": resume_id
        });
        {
            let mut state = self.state.lock().await;
            state.last_dag_request = Some(request.clone());
            state.last_dag_result = Some(json!({"node_receipts":[],"outputs":{}}));
        }
        if !plan.hazards.is_empty() {
            request["plan_only"] = Value::Bool(true);
            let response = handle_request_with_observer(&self.gateway, &request, None).await?;
            self.record_history(goal, response).await;
            return Ok(());
        }
        if !execute {
            let response = handle_request_with_observer(&self.gateway, &request, None).await?;
            self.record_history(goal, response).await;
            return Ok(());
        }
        let prepared = self.prepare_execution_request(&request, worktree).await?;
        self.state.lock().await.last_dag_request = Some(prepared.clone());
        let execution = self
            .execute_dag_live(prepared, plan, updates.clone())
            .await?;
        if let Some(dag_id) = execution["dag_id"].as_str() {
            self.state.lock().await.last_dag_id = Some(dag_id.to_owned());
        }
        let receipts = execution["node_receipts"]
            .as_array()
            .or_else(|| execution["receipts"].as_array());
        let receipt_count = receipts.map_or(0, Vec::len);
        let verified = receipts.map_or(0, |receipts| {
            receipts
                .iter()
                .filter(|receipt| receipt["state"] == "settled")
                .count()
        });
        send_transcript(
            &updates,
            format!("DAG finished: {verified}/{receipt_count} lanes verified"),
        )
        .await?;
        self.record_history(goal, execution).await;
        Ok(())
    }

    async fn execute_dag_live(
        &self,
        request: Value,
        plan: DagPlan,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<Value> {
        let nodes = plan
            .nodes
            .iter()
            .cloned()
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let (events_tx, mut events_rx) = mpsc::channel(128);
        let gateway = self.execution_gateway().await;
        let execution = handle_request_with_observer(&gateway, &request, Some(events_tx));
        tokio::pin!(execution);
        let mut started = BTreeMap::<String, Instant>::new();
        let mut events_open = true;
        loop {
            tokio::select! {
                event = events_rx.recv(), if events_open => {
                    if let Some(event) = event {
                        self.apply_dag_event(event, &nodes, &mut started, &updates).await?;
                    } else {
                        events_open = false;
                    }
                }
                result = &mut execution => {
                    while let Ok(event) = events_rx.try_recv() {
                        self.apply_dag_event(event, &nodes, &mut started, &updates).await?;
                    }
                    if let Err(error) = &result {
                        updates.send(TuiUpdate::DagInterrupted(error.to_string())).await?;
                    }
                    return result;
                }
            }
        }
    }

    async fn apply_dag_event(
        &self,
        event: DagExecutionEvent,
        nodes: &BTreeMap<String, crate::dag::DagNode>,
        started: &mut BTreeMap<String, Instant>,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        match event {
            DagExecutionEvent::WaveStarted {
                wave_index,
                node_ids,
            } => {
                send_transcript(
                    updates,
                    format!(
                        "Wave {} started at actual execution boundary: {}",
                        wave_index + 1,
                        node_ids.join(", ")
                    ),
                )
                .await
            }
            DagExecutionEvent::NodeStarted { node } => {
                started.insert(node.id.clone(), Instant::now());
                updates
                    .send(TuiUpdate::LaneState {
                        node_id: node.id.clone(),
                        state: LaneState::Running,
                        note: "provider execution in progress".to_owned(),
                        elapsed: None,
                    })
                    .await?;
                let file = node
                    .write_files
                    .first()
                    .or_else(|| node.read_files.first())
                    .map_or_else(|| "workspace".to_owned(), |path| path.display().to_string());
                updates
                    .send(TuiUpdate::Transcript(TranscriptEntry {
                        depth: 0,
                        message: format!(
                            "delegate_task(lane=\"{}\", role=\"{}\", agent=\"{}:{}\", file=\"{file}\")",
                            node.id,
                            node_role_label(&node.role, &node.model),
                            node.model,
                            node.reasoning_effort
                        ),
                    }))
                    .await?;
                if let Some(scan) = inspect_node_source(&self.workspace, &node) {
                    updates
                        .send(TuiUpdate::Transcript(TranscriptEntry {
                            depth: 1,
                            message: scan,
                        }))
                        .await?;
                }
                if let Some(verify_cmd) =
                    node.verify_command.as_deref().filter(|cmd| !cmd.is_empty())
                {
                    updates
                        .send(TuiUpdate::Transcript(TranscriptEntry {
                            depth: 1,
                            message: format!(
                                "gate=\"{verify_cmd}\" · task=\"{}\"",
                                truncate_utf8(&node.task, 96)
                            ),
                        }))
                        .await?;
                }
                Ok(())
            }
            DagExecutionEvent::ProviderFinished { node_id, output } => {
                if let Some(result) = &mut self.state.lock().await.last_dag_result {
                    result["outputs"][&node_id] = output.clone();
                }
                self.apply_usage(&output, updates).await?;
                let stage = provider_stage(&output);
                updates
                    .send(TuiUpdate::Transcript(TranscriptEntry {
                        depth: 1,
                        message: format!("provider returned for {node_id} · {stage}"),
                    }))
                    .await?;
                if let Some(preview) = provider_output_preview(nodes.get(&node_id), &output) {
                    updates
                        .send(TuiUpdate::Transcript(TranscriptEntry {
                            depth: 1,
                            message: preview,
                        }))
                        .await?;
                }
                Ok(())
            }
            DagExecutionEvent::NodeStopped { node_id } => {
                let elapsed = started.get(&node_id).map(Instant::elapsed);
                updates
                    .send(TuiUpdate::LaneState {
                        node_id: node_id.clone(),
                        state: LaneState::Stopped,
                        note: "provider reported done; verification pending".to_owned(),
                        elapsed,
                    })
                    .await?;
                updates
                    .send(TuiUpdate::Transcript(TranscriptEntry {
                        depth: 1,
                        message: format!(
                            "{node_id} STOPPED · reported done · awaiting verification"
                        ),
                    }))
                    .await?;
                Ok(())
            }
            DagExecutionEvent::NodeSettled { receipt } => {
                if let Some(result) = &mut self.state.lock().await.last_dag_result
                    && let Some(receipts) = result["node_receipts"].as_array_mut()
                {
                    receipts.retain(|previous| previous["node_id"] != receipt.node_id);
                    receipts.push(serde_json::to_value(&receipt)?);
                }
                let elapsed = started
                    .remove(&receipt.node_id)
                    .map(|start| start.elapsed());
                let (state, note) = settlement_view(&receipt);
                updates
                    .send(TuiUpdate::LaneState {
                        node_id: receipt.node_id.clone(),
                        state,
                        note: note.clone(),
                        elapsed,
                    })
                    .await?;
                updates
                    .send(TuiUpdate::Transcript(TranscriptEntry {
                        depth: 1,
                        message: format!("{} · {note}", receipt.node_id),
                    }))
                    .await?;
                Ok(())
            }
            DagExecutionEvent::WaveFinished { wave_index } => {
                let lane_count = nodes.len();
                send_transcript(
                    updates,
                    format!(
                        "Layer {} finished · {lane_count} total graph lanes · other ready layers run independently",
                        wave_index + 1
                    ),
                )
                .await
            }
        }
    }

    async fn apply_usage(&self, output: &Value, updates: &mpsc::Sender<TuiUpdate>) -> Result<()> {
        let usage = {
            let mut state = self.state.lock().await;
            update_usage_state(&mut state, output)
        };
        updates.send(TuiUpdate::Tokens(usage)).await?;
        Ok(())
    }

    async fn inspection_snapshot(&self) -> Result<(Value, Option<Value>)> {
        let (request, result, goal, effects, skills) = {
            let state = self.state.lock().await;
            let result = state.last_dag_result.clone();
            (
                state.last_dag_request.clone(),
                result,
                state
                    .last_dag_goal
                    .clone()
                    .unwrap_or_else(|| "Verify and refactor module contracts".to_owned()),
                state.effects.clone(),
                state.skills.clone(),
            )
        };
        if let Some(request) = request {
            return Ok((request, result));
        }
        let root = self.workspace.clone();
        let request = tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut plan = DagPlanner::generate(&goal, &root)?;
            apply_effects_to_plan(&mut plan, &effects);
            let body = skills.active_context(32_000)?;
            Ok(json!({"campaign_goal":goal,"workspace_dir":root,"nodes":dag_request_nodes(&plan,&body),"plan_only":true}))
        })
        .await??;
        Ok((request, None))
    }

    async fn show_overview(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let (request, result) = self.inspection_snapshot().await?;
        let mut lines = vec![
            format!(
                "Goal: {}",
                request["campaign_goal"].as_str().unwrap_or_default()
            ),
            "Actual workspace source and recorded gate evidence; a plan is not execution."
                .to_owned(),
        ];
        lines.extend(prompt_inspection_lines(&request, result.as_ref(), None));
        let files = request["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|node| {
                node["read_files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let root = self.workspace.clone();
        let context = tokio::task::spawn_blocking(move || {
            crate::context::SourceContextBuilder::new(&root)?.build(&files, &[], VIEW_BYTE_LIMIT)
        })
        .await??;
        lines.push("Workspace source excerpts (read from disk)".to_owned());
        lines.push(context.markdown);
        show_static_view(&updates, "DAG source and evidence board", lines).await
    }

    async fn show_workboard(
        &self,
        requested_limit: Option<String>,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let limit = requested_limit.as_deref().map_or(Ok(20), |value| {
            value
                .parse::<usize>()
                .map_err(|_| crate::Error::from("usage: /board [limit]"))
        })?;
        if !(1..=100).contains(&limit) {
            return Err("board limit must be between 1 and 100".into());
        }
        let value = crate::workbench::board(&self.gateway, &self.workspace, limit).await?;
        show_static_view(
            &updates,
            "Organization workboard",
            workboard_summary(&value),
        )
        .await
    }

    async fn run_council(&self, proposal: String, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let proposal = proposal.trim();
        if proposal.is_empty() {
            return Err("usage: /council <proposal>".into());
        }
        let response = self
            .gateway
            .call(
                "heterarchy_triad_orchestrate",
                crate::workbench::parse_council_request(proposal, &self.workspace)?,
            )
            .await?;
        self.record_history(format!("/council {proposal}"), response.clone())
            .await;
        show_static_view(
            &updates,
            "Review Council dispatch",
            vec![
                "Queued or running roles are not votes or approval. Use /votes <collaboration_id> after completion. Reviews without --file scope report source freshness as unknown."
                    .to_owned(),
                pretty_json_limited(&response, VIEW_BYTE_LIMIT),
            ],
        )
        .await
    }

    async fn show_votes(
        &self,
        collaboration_id: String,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let collaboration_id = collaboration_id.trim();
        if collaboration_id.is_empty() || collaboration_id.split_whitespace().count() != 1 {
            return Err("usage: /votes <collaboration_id>".into());
        }
        let value =
            crate::workbench::votes(&self.gateway, &self.workspace, collaboration_id).await?;
        show_static_view(&updates, "Review Council ballots", votes_summary(&value)).await
    }

    async fn show_recovery(&self, job_id: String, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let job_id = job_id.trim();
        if job_id.is_empty() || job_id.split_whitespace().count() != 1 {
            return Err("usage: /recovery <job_id>".into());
        }
        let value =
            crate::workbench::recovery(&self.gateway.state_root, &self.workspace, job_id).await?;
        show_static_view(&updates, "Recovery observations", recovery_summary(&value)).await
    }

    async fn show_prompts(
        &self,
        selector: Option<String>,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let direct = {
            let state = self.state.lock().await;
            if state.last_dag_request.is_none() {
                state.history.back().cloned()
            } else {
                None
            }
        };
        if let Some(record) = direct {
            let filter = selector.as_deref().unwrap_or_default();
            let body = if filter.is_empty() || filter == "all" || record.request.contains(filter) {
                vec![
                    format!("Actual user request\n{}", record.request),
                    "Transport-generated grounding is not reconstructed as an exact final prompt."
                        .to_owned(),
                    format!(
                        "Actual returned result and evidence\n{}",
                        pretty_json_limited(&record.response, VIEW_BYTE_LIMIT)
                    ),
                ]
            } else {
                vec![format!("No recorded direct request matches '{filter}'.")]
            };
            return show_static_view(
                &updates,
                "Direct request and returned context evidence",
                body,
            )
            .await;
        }
        let (request, result) = self.inspection_snapshot().await?;
        let lines = prompt_inspection_lines(&request, result.as_ref(), selector.as_deref());
        show_static_view(&updates, "DAG task, context and synthesis inspector", lines).await
    }

    async fn show_models(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let health = self
            .gateway
            .call(
                "codex_bridge_health",
                json!({"workspace_dir":self.workspace}),
            )
            .await?;
        let (active_model, active_effort, effect_summary) = {
            let state = self.state.lock().await;
            (
                state.active_model.clone(),
                state.effort.clone(),
                state.effects.summary(),
            )
        };
        let mut lines =
            helpers::models_summary(&health, &active_model, &active_effort, &effect_summary);
        lines.push(pretty_json_limited(&health, VIEW_BYTE_LIMIT));
        show_static_view(&updates, "Models and routing health", lines).await
    }

    async fn select_model(
        &self,
        requested: String,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let previous = self.state.lock().await.active_model.clone();
        let normalized_requested = requested.to_ascii_lowercase();
        let tokens = normalized_requested.split_whitespace().collect::<Vec<_>>();
        let values = if tokens.first() == Some(&"codex") {
            &tokens[1..]
        } else {
            tokens.as_slice()
        };
        let native_default = values
            .iter()
            .copied()
            .filter(|token| !is_effort(token))
            .collect::<Vec<_>>()
            .join(" ")
            .eq_ignore_ascii_case("codex-default");
        let resolved = if native_default {
            match self.gateway.configured_default_model(&self.workspace).await {
                Ok(configured) => Some(configured.model),
                Err(error) => {
                    updates.send(TuiUpdate::ActiveModel(previous)).await?;
                    return Err(error);
                }
            }
        } else {
            resolve_effect_model(values)
        };
        let Some(model) = resolved else {
            updates.send(TuiUpdate::ActiveModel(previous)).await?;
            return Err(format!("unsupported model: {requested}").into());
        };
        let efforts = values
            .iter()
            .copied()
            .filter(|token| is_effort(token))
            .collect::<Vec<_>>()
            .join(" ");
        let effect_argument = format!("{model} {efforts}");
        let selection = {
            let mut state = self.state.lock().await;
            let mut effects = state.effects.clone();
            match apply_effect_argument(&mut effects, &effect_argument) {
                Err(error) => Err(error),
                Ok(()) => {
                    let profile = if model.starts_with("claude-") {
                        &effects.claude
                    } else if model.starts_with("gemini-") {
                        &effects.argon
                    } else {
                        &effects.codex
                    };
                    let selected_model = profile.model.clone();
                    let effort = profile.effort.clone();
                    state.active_model.clone_from(&selected_model);
                    state.effort.clone_from(&effort);
                    state.effects = effects;
                    state.context_window = None;
                    state.current_context_tokens = None;
                    let usage = TokenUsage {
                        input: state.usage_known_seen.then_some(state.tokens_in),
                        output: state.usage_known_seen.then_some(state.tokens_out),
                        usage_complete: state.usage_known_seen && !state.usage_unknown_seen,
                        ..TokenUsage::default()
                    };
                    Ok((selected_model, effort, state.effects.summary(), usage))
                }
            }
        };
        let (model, effort, effect_summary, usage) = match selection {
            Ok(selected) => selected,
            Err(error) => {
                updates.send(TuiUpdate::ActiveModel(previous)).await?;
                return Err(error);
            }
        };
        updates.send(TuiUpdate::Effort(effort)).await?;
        updates.send(TuiUpdate::ActiveModel(model.clone())).await?;
        updates.send(TuiUpdate::Tokens(usage)).await?;
        updates
            .send(TuiUpdate::EffectSummary(effect_summary))
            .await?;
        send_transcript(&updates, format!("Active model: {model}")).await
    }

    async fn show_skills(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let (skills, active_bytes) = {
            let state = self.state.lock().await;
            let cloned = state.skills.clone();
            drop(state);
            let bytes = tokio::task::spawn_blocking({
                let cloned = cloned.clone();
                move || cloned.active_context(32_000).map(|ctx| ctx.len())
            })
            .await?
            .unwrap_or(0);
            (cloned, bytes)
        };
        let total_skills = skills.skills().len();
        let active_count = skills.active().len();
        let mut lines = vec![
            format!(
                "Skills Configuration Page · Active: {active_count}/{total_skills} · Budget: {active_bytes}/32000 B"
            ),
            "Controls: ↑/↓ select · Space/Enter toggle (● active ↔ ○ deactivate) · c core · d default · x clear · 1-7 pages".to_owned(),
            "────────────────────────────────────────────────────────────────────────────────────────".to_owned(),
        ];
        for skill in skills.skills().values() {
            let status = if skills.active().contains(&skill.name) {
                "● active    "
            } else {
                "○ deactivate"
            };
            lines.push(format!(
                "  ${:<28} {status} · {:?} · {}",
                skill.name, skill.scope, skill.description
            ));
        }
        show_static_view(&updates, "Discovered skills", lines).await
    }

    async fn toggle_skill(&self, skill: String, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let raw_trimmed = skill.trim();
        let command = raw_trimmed.to_ascii_lowercase();
        if matches!(command.as_str(), "" | "list") {
            return self.show_skills(updates).await;
        }
        let (explicit_mode, stripped_target) = if let Some(rest) =
            ["load ", "on ", "enable ", "activate "]
                .into_iter()
                .find_map(|prefix| {
                    command
                        .starts_with(prefix)
                        .then(|| raw_trimmed[prefix.len()..].trim())
                }) {
            (Some(true), rest)
        } else if let Some(rest) =
            ["off ", "disable ", "deactivate "]
                .into_iter()
                .find_map(|prefix| {
                    command
                        .starts_with(prefix)
                        .then(|| raw_trimmed[prefix.len()..].trim())
                })
        {
            (Some(false), rest)
        } else {
            (None, raw_trimmed)
        };
        let (active, missing, paths) = {
            // Serialize skill changes through preflight and commit. The Tokio
            // guard does not block executor threads; actual file I/O stays in
            // the blocking bridge, and no unvalidated selection is published.
            let mut state = self.state.lock().await;
            let mut selected = state.skills.clone();
            let prior_active = selected.active().to_vec();
            let missing = if matches!(
                command.as_str(),
                "clear" | "none" | "off" | "reset" | "default"
            ) {
                for name in selected.active().to_vec() {
                    selected.deactivate(&name);
                }
                if command == "default" {
                    selected.activate("codex-heterarchy")?;
                }
                Vec::new()
            } else if command == "core" {
                selected.activate_core()
            } else {
                for target in stripped_target.split_whitespace() {
                    match explicit_mode {
                        Some(true) => {
                            selected.activate(target)?;
                        }
                        Some(false) => {
                            selected.activate(target)?;
                            selected.deactivate(target);
                        }
                        None => {
                            let previous = selected.active().to_vec();
                            selected.activate(target)?;
                            if selected.active() == previous {
                                selected.deactivate(target);
                            }
                        }
                    }
                }
                Vec::new()
            };
            let paths = if !selected.active().is_empty() {
                let is_preset = matches!(command.as_str(), "core" | "default");
                let paths = selected
                    .active()
                    .iter()
                    .filter(|name| is_preset || !prior_active.contains(name))
                    .filter_map(|name| {
                        selected
                            .skills()
                            .get(name)
                            .map(|entry| (name.clone(), entry.path.clone()))
                    })
                    .collect::<Vec<_>>();
                let preflight = selected.clone();
                let _context =
                    tokio::task::spawn_blocking(move || preflight.active_context(32_000)).await??;
                paths
            } else {
                Vec::new()
            };
            let active = selected.active().to_vec();
            state.skills = selected;
            (active, missing, paths)
        };
        let names = active.join(", ");
        updates.send(TuiUpdate::ActiveSkills(active)).await?;
        for (name, path) in paths {
            send_transcript(&updates, format!("Loaded ${name} from {}", path.display())).await?;
        }
        let missing = if missing.is_empty() {
            String::new()
        } else {
            format!("; unavailable core skills: {}", missing.join(", "))
        };
        send_transcript(&updates, format!("Active skills: {names}{missing}")).await
    }

    async fn set_effort(&self, effort: String, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let normalized = effort.trim().to_ascii_lowercase();
        let previous = self.state.lock().await.effort.clone();
        if !matches!(
            normalized.as_str(),
            "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        ) {
            updates.send(TuiUpdate::Effort(previous)).await?;
            return Err(format!("unsupported reasoning effort: {effort}").into());
        }
        let (active_model, current_effort) = {
            let state = self.state.lock().await;
            (state.active_model.clone(), state.effort.clone())
        };
        if active_model.starts_with("claude-") && normalized != "medium" {
            updates.send(TuiUpdate::Effort(current_effort)).await?;
            return Err("Claude 5.5 routes accept medium effort only".into());
        }
        if active_model.starts_with("gemini-")
            && !matches!(normalized.as_str(), "low" | "medium" | "high")
        {
            updates.send(TuiUpdate::Effort(current_effort)).await?;
            return Err("Gemini model tiers are low, medium or high".into());
        }
        let effect_summary = {
            let mut state = self.state.lock().await;
            if active_model.starts_with("claude-") {
                state.effects.claude.effort.clone_from(&normalized);
            } else if active_model.starts_with("gemini-") {
                let mut profile = state.effects.argon.clone();
                profile.model = active_model.clone();
                configure_gemini_effort(&mut profile, &normalized)?;
                state.active_model.clone_from(&profile.model);
                state.effects.argon = profile;
            } else {
                state.effects.codex.effort.clone_from(&normalized);
            }
            state.effort.clone_from(&normalized);
            state.effects.summary()
        };
        updates.send(TuiUpdate::Effort(normalized)).await?;
        if active_model.starts_with("gemini-") {
            updates
                .send(TuiUpdate::ActiveModel(
                    self.state.lock().await.active_model.clone(),
                ))
                .await?;
        }
        updates
            .send(TuiUpdate::EffectSummary(effect_summary))
            .await?;
        Ok(())
    }

    async fn configure_effect(
        &self,
        argument: String,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        if argument.trim().is_empty()
            || matches!(
                argument.trim().to_ascii_lowercase().as_str(),
                "list" | "show" | "status" | "matrix"
            )
        {
            let summary = self.state.lock().await.effects.summary();
            return show_static_view(&updates, "DAG effect configuration", vec![summary]).await;
        }
        let (summary, theme, active_effort) = {
            let mut state = self.state.lock().await;
            apply_effect_argument(&mut state.effects, &argument)?;
            state.effort = if state.active_model.starts_with("claude-") {
                state.effects.claude.effort.clone()
            } else if state.active_model.starts_with("gemini-") {
                state
                    .active_model
                    .rsplit('-')
                    .next()
                    .unwrap_or("high")
                    .to_owned()
            } else {
                state.effects.codex.effort.clone()
            };
            (
                state.effects.summary(),
                state.effects.style,
                state.effort.clone(),
            )
        };
        updates
            .send(TuiUpdate::EffectSummary(summary.clone()))
            .await?;
        updates.send(TuiUpdate::Theme(theme)).await?;
        updates.send(TuiUpdate::Effort(active_effort)).await?;
        send_transcript(&updates, format!("DAG effect updated · {summary}")).await
    }

    async fn compact(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let removed = {
            let mut state = self.state.lock().await;
            let removed = state.history.len().saturating_sub(1);
            while state.history.len() > 1 {
                state.history.pop_front();
            }
            removed
        };
        updates.send(TuiUpdate::CompactHistory).await?;
        send_transcript(
            &updates,
            format!(
                "Compacted {removed} local history entries; receipt token totals remain unchanged"
            ),
        )
        .await
    }

    async fn set_permissions(
        &self,
        requested: Option<String>,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let Some(requested) = requested.filter(|value| !value.trim().is_empty()) else {
            let mode = self.state.lock().await.permission_mode.clone();
            return show_static_view(
                &updates,
                "Permissions",
                vec![format!(
                    "Current mode: {mode} · read-only | auto-edit | yolo"
                )],
            )
            .await;
        };
        let mode = normalize_permission(&requested)?;
        self.state.lock().await.permission_mode.clone_from(&mode);
        updates
            .send(TuiUpdate::PermissionMode(mode.clone()))
            .await?;
        send_transcript(
            &updates,
            format!("Permission mode updated: {mode} (applied to new provider requests)"),
        )
        .await
    }

    async fn resume(
        &self,
        requested_id: Option<String>,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let workspace = self.workspace.clone();
        let workspace = tokio::task::spawn_blocking(move || workspace.canonicalize()).await??;
        let known_id = self.state.lock().await.last_dag_id.clone();
        let target_id = requested_id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or(known_id);
        let dag_id = match target_id {
            Some(dag_id) => dag_id,
            None => {
                let jobs = self
                    .gateway
                    .call(
                        "codex_job_list",
                        json!({"limit":100,"workspace_dir":workspace}),
                    )
                    .await?;
                latest_dag_checkpoint(&jobs, &workspace)
                    .ok_or("no previous DAG checkpoint is available")?
            }
        };
        if dag_id.split_whitespace().count() != 1 {
            return Err("usage: /resume [campaign_id]".into());
        }
        let prepared =
            match crate::dag::prepare_resume_request(&self.gateway, &workspace, &dag_id).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    if let Some(goal) = self.legacy_direct_resume_goal(&workspace, &dag_id).await? {
                        send_transcript(
                            &updates,
                            format!("Resuming historical direct DAG checkpoint {dag_id}"),
                        )
                        .await?;
                        return Box::pin(self.plan_dag(goal, true, Some(dag_id), updates)).await;
                    }
                    return Err(error);
                }
            };
        let permission_mode = self.state.lock().await.permission_mode.clone();
        ensure_plan_permission(&permission_mode, &prepared.plan, true)?;
        self.ensure_baseline_manifest().await;
        let total = prepared.plan.nodes.len();
        {
            let mut state = self.state.lock().await;
            state.last_dag_goal = Some(prepared.plan.goal.clone());
            state.last_dag_request = Some(prepared.request.clone());
            state.last_dag_result = Some(json!({"node_receipts":[],"outputs":{}}));
        }
        updates
            .send(TuiUpdate::DagSnapshot(dag_lanes(&prepared.plan)))
            .await?;
        updates
            .send(TuiUpdate::HeaderProgress { done: 0, total })
            .await?;
        send_transcript(
            &updates,
            format!(
                "Resuming saved DAG {dag_id}; topology, skills, models and isolation are unchanged"
            ),
        )
        .await?;
        let execution = self
            .execute_dag_live(prepared.request, prepared.plan, updates.clone())
            .await?;
        if let Some(id) = execution["dag_id"].as_str() {
            self.state.lock().await.last_dag_id = Some(id.to_owned());
        }
        let verified = execution["node_receipts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|receipt| receipt["state"] == "settled" && receipt["verify_passed"] == true)
            .count();
        let reused = execution["resumed_nodes"].as_array().map_or(0, Vec::len);
        updates
            .send(TuiUpdate::HeaderProgress {
                done: verified,
                total,
            })
            .await?;
        send_transcript(
            &updates,
            format!("Resumed DAG finished: {verified}/{total} lanes verified; {reused} settled checkpoints reused"),
        )
        .await?;
        self.record_history(format!("/resume {dag_id}"), execution)
            .await;
        Ok(())
    }

    async fn legacy_direct_resume_goal(
        &self,
        workspace: &Path,
        id: &str,
    ) -> Result<Option<String>> {
        let root_id = id.to_owned();
        let checkpoint_id = format!("ckp-{id}-plan");
        let workspace = workspace.to_path_buf();
        self.gateway
            .database(move |store| {
                let Some(root) = store.get_job(&root_id)? else {
                    return Ok(None);
                };
                let Some(checkpoint) = store.get_job(&checkpoint_id)? else {
                    return Ok(None);
                };
                if root.task_type != "dag_ulw"
                    || !matches!(
                        root.status,
                        crate::storage::JobStatus::Completed | crate::storage::JobStatus::Failed
                    )
                    || Path::new(&root.workspace_dir) != workspace
                    || root.metadata.keys().any(|key| key.starts_with("worktree_"))
                    || checkpoint.task_type != "dag_checkpoint"
                    || checkpoint.status != crate::storage::JobStatus::Completed
                    || Path::new(&checkpoint.workspace_dir) != workspace
                    || checkpoint.metadata.get("campaign_id") != Some(&root_id)
                    || checkpoint
                        .metadata
                        .get("checkpoint_kind")
                        .map(String::as_str)
                        != Some("plan")
                {
                    return Ok(None);
                }
                let (Some(result), Some(saved)) = (root.result, checkpoint.result) else {
                    return Ok(None);
                };
                if result["plan_only"] != false
                    || result.get("worktrees").is_some()
                    || saved.get("request_recipe").is_some()
                    || !result["node_receipts"]
                        .as_array()
                        .or_else(|| result["receipts"].as_array())
                        .is_some_and(|receipts| !receipts.is_empty())
                {
                    return Ok(None);
                }
                let plan: DagPlan = serde_json::from_value(saved["plan"].clone())?;
                Ok(plan.is_executable().then_some(plan.goal))
            })
            .await
    }

    async fn run_triad(&self, goal: String, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        if goal.trim().is_empty() {
            return Err("/triad-review requires a task specification".into());
        }
        self.ensure_baseline_manifest().await;
        let (effects, skills) = {
            let state = self.state.lock().await;
            (state.effects.clone(), state.skills.clone())
        };
        let skill_context =
            tokio::task::spawn_blocking(move || skills.active_context(32_000)).await??;
        send_transcript(&updates, "Starting actual tri-polar orchestration").await?;
        let response = self
            .execution_gateway()
            .await
            .call(
                "heterarchy_triad_orchestrate",
                json!({
                    "task_spec": goal,
                    "target_code_or_context": "TUI triad request",
                    "invariants_and_rules": skill_context,
                    "language": "rust",
                    "mode": "pipeline",
                    "workspace_dir": self.workspace,
                    "claude_model": effects.claude.model,
                    "codex_model": effects.codex.model,
                    "codex_reasoning_effort": effects.codex.effort,
                    "argon_model": effects.argon.model,
                    "invoke_argon_rpc": true,
                    "max_rounds": 1,
                    "run_verification_gates": false
                }),
            )
            .await?;
        self.record_response(goal, response, updates).await
    }

    async fn show_inbox(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let response = self
            .execution_gateway()
            .await
            .call(
                "heterarchy_handoff_inbox",
                json!({"status":"","workspace_dir":self.workspace,"limit":20}),
            )
            .await?;
        let mut lines = helpers::inbox_summary(&response);
        lines.push(pretty_json_limited(&response, VIEW_BYTE_LIMIT));
        show_static_view(&updates, "Handoff inbox", lines).await
    }

    async fn set_theme(
        &self,
        requested: Option<String>,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let Some(requested) = requested.filter(|value| !value.trim().is_empty()) else {
            let theme = self.state.lock().await.effects.style;
            return show_static_view(
                &updates,
                "TUI theme",
                vec![format!(
                    "Active: {} · codex-dark | minimal-mono | royal-prism | cyber-matrix",
                    theme.label()
                )],
            )
            .await;
        };
        let theme = parse_theme(&requested)?;
        {
            let mut state = self.state.lock().await;
            state.effects.style = theme;
        }
        updates.send(TuiUpdate::Theme(theme)).await?;
        send_transcript(&updates, format!("TUI theme updated: {}", theme.label())).await
    }

    async fn show_status(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let (health, jobs, inbox) = tokio::try_join!(
            self.gateway.call(
                "codex_bridge_health",
                json!({"workspace_dir":self.workspace})
            ),
            self.gateway.call(
                "codex_job_list",
                json!({"limit":15,"workspace_dir":self.workspace})
            ),
            self.gateway.call(
                "heterarchy_handoff_inbox",
                json!({"status":"","workspace_dir":self.workspace,"limit":10})
            ),
        )?;
        let combined = json!({"health":health,"jobs":jobs,"inbox":inbox});
        let mut lines = helpers::status_summary(&health, &jobs, &inbox);
        lines.push(pretty_json_limited(&combined, VIEW_BYTE_LIMIT));
        show_static_view(&updates, "Bridge and jobs status", lines).await
    }

    async fn show_diff(&self, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        let root = self.workspace.clone();
        let current =
            tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&root)).await??;
        let baseline = {
            let mut state = self.state.lock().await;
            state
                .baseline_manifest
                .get_or_insert_with(|| current.clone())
                .clone()
        };
        let mut lines = vec![
            "Workspace SHA-256 changes since session baseline (tracked and untracked files)."
                .to_owned(),
            format!("Current files: {}", current.files.len()),
        ];
        for (path, digest) in &current.files {
            match baseline.files.get(path) {
                None => lines.push(format!("Added: {}", path.display())),
                Some(previous) if previous != digest => {
                    lines.push(format!("Modified: {}", path.display()))
                }
                _ => {}
            }
        }
        for path in baseline
            .files
            .keys()
            .filter(|path| !current.files.contains_key(*path))
        {
            lines.push(format!("Deleted: {}", path.display()));
        }
        if lines.len() == 2 {
            lines.push("No content changes since the baseline.".to_owned());
        }
        show_static_view(&updates, "Workspace manifest diff", lines).await
    }

    async fn review(&self, focus: Option<String>, updates: mpsc::Sender<TuiUpdate>) -> Result<()> {
        self.ensure_baseline_manifest().await;
        send_transcript(&updates, "Starting a real Codex working-tree review").await?;
        updates
            .send(TuiUpdate::HeaderProgress { done: 0, total: 1 })
            .await?;
        let response = self
            .execution_gateway()
            .await
            .call(
                "codex_git_review",
                json!({
                    "workspace_dir": self.workspace,
                    "custom_instructions": focus.unwrap_or_default(),
                    "uncommitted": true,
                    "model": CODEX_DIRECT_MODEL,
                    "reasoning_effort": "xhigh"
                }),
            )
            .await?;
        self.record_response("/review".to_owned(), response, updates)
            .await
    }

    async fn record_response(
        &self,
        request: String,
        response: Value,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let output = response_text(&response);
        let usage = {
            let mut state = self.state.lock().await;
            let usage = update_usage_state(&mut state, &response);
            push_history(&mut state.history, request, response);
            usage
        };
        updates.send(TuiUpdate::Tokens(usage)).await?;
        updates
            .send(TuiUpdate::HeaderProgress { done: 1, total: 1 })
            .await?;
        updates
            .send(TuiUpdate::Transcript(TranscriptEntry {
                depth: 1,
                message: output,
            }))
            .await?;
        Ok(())
    }

    async fn record_history(&self, request: String, response: Value) {
        let mut state = self.state.lock().await;
        if response.get("compiled_plan").is_some() {
            state.last_dag_result = Some(response.clone());
        }
        push_history(&mut state.history, request, response);
    }
}

impl ActionHandler for AppHandler {
    fn handle(&self, action: TuiAction, updates: mpsc::Sender<TuiUpdate>) -> ActionFuture {
        self.dispatch(action, updates)
    }

    fn shutdown(&self) -> ActionFuture {
        let gateway = self.gateway.clone();
        Box::pin(async move { gateway.shutdown().await })
    }
}

/// Runs the interactive terminal against the real Rust application gateway.
pub async fn run(workspace: PathBuf, state_root: PathBuf) -> Result<()> {
    run_configured(workspace, state_root, false).await
}

/// Interactive named session: every executing DAG opts into per-node worktrees.
pub async fn run_with_worktrees(workspace: PathBuf, state_root: PathBuf) -> Result<()> {
    run_configured(workspace, state_root, true).await
}

async fn run_configured(
    workspace: PathBuf,
    state_root: PathBuf,
    worktree_default: bool,
) -> Result<()> {
    let gateway = Gateway::new(state_root);
    let handler_workspace = workspace.clone();
    let handler = tokio::task::spawn_blocking(move || {
        AppHandler::new(gateway, handler_workspace)
            .map(|h| h.with_worktree_default(worktree_default))
    })
    .await??;
    handler.ensure_baseline_manifest().await;
    let run_result = crate::tui::run_with_handler(workspace, handler.clone()).await;
    let shutdown_result = handler.gateway.shutdown().await;
    run_result.and(shutdown_result)
}

/// Observes whether an initial command resolves to recovery before session creation.
/// Uses the TUI's registered-skill prefix rules without changing session state or dispatching work.
pub fn initial_recovery_input(workspace: &Path, home: Option<&Path>, input: &str) -> Result<bool> {
    Ok(matches!(
        resolved_initial_action(workspace, home, input)?,
        TuiAction::ShowRecovery(_)
    ))
}

/// Diagnostics and resume must target an existing workspace rather than create a named session.
pub fn initial_existing_session_input(
    workspace: &Path,
    home: Option<&Path>,
    input: &str,
) -> Result<bool> {
    match resolved_initial_action(workspace, home, input)? {
        TuiAction::ShowRecovery(_) | TuiAction::Resume(_) => Ok(true),
        TuiAction::RunDag(goal)
        | TuiAction::PlanDag(goal)
        | TuiAction::AutoDag(goal)
        | TuiAction::EnterDag(Some(goal)) => {
            Ok(inline_resume_id(&worktree_goal(&goal).0)?.is_some())
        }
        _ => Ok(false),
    }
}

fn resolved_initial_action(
    workspace: &Path,
    home: Option<&Path>,
    input: &str,
) -> Result<TuiAction> {
    let input = input.trim();
    let action = crate::tui::action_from_input(input);
    if !matches!(
        action,
        TuiAction::SubmitPrompt(_) | TuiAction::UnknownCommand(_)
    ) {
        return Ok(action);
    }
    let mut skills = SkillRegistry::discover(workspace, home)?;
    if let TuiAction::UnknownCommand(ref command) = action
        && skills
            .extract_mentions_where(command, inline_skill_token)
            .0
            .is_empty()
    {
        return Ok(action);
    }
    let (matched, cleaned) = skills.extract_mentions_where(input, inline_skill_token);
    Ok(if matched.is_empty() {
        action
    } else {
        crate::tui::action_from_input(&cleaned)
    })
}

/// Runs one composer command through the same real action adapter without a TTY.
pub async fn command(
    workspace: PathBuf,
    state_root: PathBuf,
    input: String,
) -> Result<Vec<TuiUpdate>> {
    command_configured(workspace, state_root, input, false).await
}

/// One named-session command using the same node-isolation default as the TUI.
pub async fn command_with_worktrees(
    workspace: PathBuf,
    state_root: PathBuf,
    input: String,
) -> Result<Vec<TuiUpdate>> {
    command_configured(workspace, state_root, input, true).await
}

async fn command_configured(
    workspace: PathBuf,
    state_root: PathBuf,
    input: String,
    worktree_default: bool,
) -> Result<Vec<TuiUpdate>> {
    if input.trim().is_empty() {
        return Err("command input cannot be empty".into());
    }
    let gateway = Gateway::new(state_root);
    let handler_workspace = workspace.clone();
    let handler = tokio::task::spawn_blocking(move || {
        AppHandler::new(gateway, handler_workspace)
            .map(|h| h.with_worktree_default(worktree_default))
    })
    .await??;
    let action = crate::tui::action_from_input(input.trim());
    let action = if let TuiAction::UnknownCommand(ref command) = action {
        let mut skills = handler.state.lock().await.skills.clone();
        if !skills
            .extract_mentions_where(command, inline_skill_token)
            .0
            .is_empty()
        {
            TuiAction::SubmitPrompt(input.trim().to_owned())
        } else {
            action
        }
    } else {
        action
    };
    let (updates_tx, mut updates_rx) = mpsc::channel(256);
    let mut dispatch = handler.dispatch(action, updates_tx);
    let mut updates = Vec::new();
    let dispatch_result = loop {
        tokio::select! {
            maybe_update = updates_rx.recv() => {
                if let Some(update) = maybe_update {
                    updates.push(update);
                }
            }
            result = &mut dispatch => {
                while let Ok(update) = updates_rx.try_recv() {
                    updates.push(update);
                }
                break result;
            }
        }
    };
    let shutdown_result = handler.gateway.shutdown().await;
    dispatch_result.and(shutdown_result).map(|()| updates)
}
