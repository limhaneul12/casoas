//! Workspace-specific presentation and UI admission; runtime authority stays in workspaces.
use super::*;
use crate::tui::{WorkspaceArtifactView, WorkspaceUpdate};
use crate::workspace_cli::{self, WorkspaceCommand};
use crate::workspaces::{WorkflowEvent, WorkflowRequest, WorkspaceKind, WorkspaceRunStatus};

impl AppHandler {
    pub(super) async fn begin_legacy_building(
        &self,
        action: &TuiAction,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<Option<String>> {
        if !matches!(
            action,
            TuiAction::SubmitPrompt(_)
                | TuiAction::RunDag(_)
                | TuiAction::AutoDag(_)
                | TuiAction::PlanDag(_)
                | TuiAction::EnterDag(Some(_))
                | TuiAction::Resume(_)
                | TuiAction::RunTriad(_)
                | TuiAction::RunCouncil(_)
                | TuiAction::Review(_)
                | TuiAction::SelectAndPrompt { .. }
        ) {
            return Ok(None);
        }
        let id = format!("legacy-{}", uuid::Uuid::new_v4().simple());
        self.reserve_spaces(&id, &[WorkspaceKind::Building]).await?;
        let sent = updates
            .send(TuiUpdate::WorkspaceStarted {
                run_id: id.clone(),
                selected: vec![WorkspaceKind::Building],
                objective: "Explicit Building command".into(),
            })
            .await;
        if let Err(error) = sent {
            self.release_spaces(&id).await;
            return Err(error.into());
        }
        if let Err(error) = scoped(
            updates,
            &id,
            WorkspaceKind::Building,
            WorkspaceUpdate::Status {
                status: WorkspaceRunStatus::Running,
                detail: "Executing the explicit Building command".into(),
            },
        )
        .await
        {
            self.release_spaces(&id).await;
            return Err(error);
        }
        Ok(Some(id))
    }

    pub(super) async fn relay_legacy_action<F>(
        &self,
        id: &str,
        future: F,
        mut receiver: mpsc::Receiver<TuiUpdate>,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()>
    where
        F: std::future::Future<Output = Result<()>>,
    {
        tokio::pin!(future);
        let mut delivery_error: Option<crate::Error> = None;
        let result = loop {
            tokio::select! {
                result=&mut future=>break result,
                update=receiver.recv()=>match update {
                    Some(update)=>{
                        if delivery_error.is_none()
                            && let Err(error)=updates.send(legacy_projection(id,update)).await {
                            delivery_error=Some(error.into());
                            // This terminal sink is gone; its Gateway owns all
                            // terminal work and drains it before the caller exits.
                            let _shutdown=self.gateway.shutdown().await;
                        }
                    }
                    None=>break future.await,
                }
            }
        };
        while let Some(update) = receiver.recv().await {
            if delivery_error.is_none()
                && let Err(error) = updates.send(legacy_projection(id, update)).await
            {
                delivery_error = Some(error.into());
            }
        }
        match delivery_error {
            Some(error) => Err(error),
            None => result,
        }
    }

    pub(super) async fn finish_legacy_building(
        &self,
        id: &str,
        result: &Result<()>,
        updates: &mpsc::Sender<TuiUpdate>,
    ) {
        // Returning from an adapter command is STOPPED, never new SETTLED proof.
        let status = if result.is_ok() {
            WorkspaceRunStatus::Stopped
        } else {
            WorkspaceRunStatus::Failed
        };
        let _delivered = updates
            .send(TuiUpdate::WorkspaceFinished {
                run_id: id.into(),
                status,
            })
            .await;
        self.release_spaces(id).await;
    }

    pub(super) async fn run_workspace_command(
        &self,
        command: WorkspaceCommand,
        arguments: String,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let request = workspace_cli::request(command, &arguments, self.workspace.clone())?;
        self.run_workspace_request(request, updates).await
    }

    async fn reserve_spaces(&self, id: &str, spaces: &[WorkspaceKind]) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(space) = spaces
            .iter()
            .find(|space| state.workspace_runs.contains_key(space))
        {
            return Err(format!(
                "{} already has an active run; inspect or cancel its original flow ID",
                space.label()
            )
            .into());
        }
        for space in spaces {
            state.workspace_runs.insert(*space, id.to_owned());
        }
        Ok(())
    }

    async fn release_spaces(&self, id: &str) {
        self.state
            .lock()
            .await
            .workspace_runs
            .retain(|_, owner| owner != id);
    }

    pub(super) async fn run_workspace_request(
        &self,
        mut request: WorkflowRequest,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let id = request
            .request_id
            .get_or_insert_with(|| format!("wfl-{}", uuid::Uuid::new_v4().simple()))
            .clone();
        let selected = request.stages.clone();
        let objective = request.goal.clone();
        self.reserve_spaces(&id, &selected).await?;
        if selected.contains(&WorkspaceKind::Building) {
            let plan_id = match &request.plan_id {
                Some(id) => id.clone(),
                None => crate::workspaces::planning::plan_id(&id)?,
            };
            self.state
                .lock()
                .await
                .workspace_plan_ids
                .insert(id.clone(), plan_id);
        }
        let result = async {
            updates
                .send(TuiUpdate::WorkspaceStarted {
                    run_id: id.clone(),
                    selected: selected.clone(),
                    objective,
                })
                .await?;
            for space in &selected {
                scoped(&updates,&id,*space,WorkspaceUpdate::Transcript(TranscriptEntry::root(format!("flow_id: {id} · selected stages: {}",selected.iter().map(|stage|stage.as_str()).collect::<Vec<_>>().join(" → "))))).await?;
            }
            let (events, mut receiver) = mpsc::channel(32);
            let run = crate::workspaces::run(&self.gateway, request, Some(events));
            tokio::pin!(run);
            let mut started = BTreeSet::new();
            let mut finished = BTreeSet::new();
            let mut delivery_error: Option<crate::Error> = None;
            let result = loop {
                tokio::select! {
                    result=&mut run=>break result,
                    event=receiver.recv()=>match event {
                        Some(event)=>{
                            match &event {
                                WorkflowEvent::StageStarted{stage,..}=>{started.insert(*stage);}
                                WorkflowEvent::StageFinished{stage,..}=>{finished.insert(*stage);}
                                WorkflowEvent::Dag{..}=>{}
                            }
                            if delivery_error.is_none()
                                && let Err(error)=self.workspace_event(event,&updates).await {
                                delivery_error=Some(error);
                                let _cancelled=crate::workspaces::cancel(&self.gateway,&self.workspace,&id).await;
                                self.gateway.cancel_supervised(&id).await;
                            }
                        }
                        None=>break run.await,
                    }
                }
            };
            while let Some(event) = receiver.recv().await {
                match &event {
                    WorkflowEvent::StageStarted { stage, .. } => {
                        started.insert(*stage);
                    }
                    WorkflowEvent::StageFinished { stage, .. } => {
                        finished.insert(*stage);
                    }
                    WorkflowEvent::Dag { .. } => {}
                }
                if delivery_error.is_none()
                    && let Err(error)=self.workspace_event(event,&updates).await {
                    delivery_error=Some(error);
                    let _cancelled=crate::workspaces::cancel(&self.gateway,&self.workspace,&id).await;
                    self.gateway.cancel_supervised(&id).await;
                }
            }
            if let Some(error)=delivery_error { return Err(error); }
            match result {
                Ok(value) => {
                    self.workspace_snapshot(&id, &value, &updates).await?;
                    let status = run_status(&value["workflow_status"])
                        .unwrap_or(WorkspaceRunStatus::Stopped);
                    updates
                        .send(TuiUpdate::WorkspaceFinished {
                            run_id: id.clone(),
                            status,
                        })
                        .await?;
                    self.record_history(format!("/workflow {id}"), value).await;
                    Ok(())
                }
                Err(error) => {
                    for space in &selected {
                        if !finished.contains(space) {
                            scoped(
                                &updates,
                                &id,
                                *space,
                                WorkspaceUpdate::Status {
                                    status: if started.contains(space) {
                                        WorkspaceRunStatus::TimedOutUnknown
                                    } else {
                                        WorkspaceRunStatus::Blocked
                                    },
                                    detail: format!("Inspect original flow {id}: {error}"),
                                },
                            )
                            .await?;
                        }
                    }
                    updates
                        .send(TuiUpdate::WorkspaceFinished {
                            run_id: id.clone(),
                            status: WorkspaceRunStatus::TimedOutUnknown,
                        })
                        .await?;
                    Err(error)
                }
            }
        }
        .await;
        if result.is_err() {
            let _finished = updates
                .send(TuiUpdate::WorkspaceFinished {
                    run_id: id.clone(),
                    status: WorkspaceRunStatus::Failed,
                })
                .await;
        }
        self.release_spaces(&id).await;
        {
            let mut state = self.state.lock().await;
            state.workspace_lanes.remove(&id);
            state.workspace_plan_ids.remove(&id);
        }
        result
    }

    pub(super) async fn show_research_plan(
        &self,
        request: crate::research::ResearchRequest,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let id = format!("preview-{}", uuid::Uuid::new_v4().simple());
        self.reserve_spaces(&id, &[WorkspaceKind::Research]).await?;
        let result = async {
            let value = crate::research::plan(&request)?;
            updates
                .send(TuiUpdate::WorkspaceStarted {
                    run_id: id.clone(),
                    selected: vec![WorkspaceKind::Research],
                    objective: request.question,
                })
                .await?;
            scoped(
                &updates,
                &id,
                WorkspaceKind::Research,
                WorkspaceUpdate::View {
                    title: "Research plan · no execution".into(),
                    body: bounded_lines(crate::research_cli::report_lines(&value)),
                },
            )
            .await?;
            scoped(
                &updates,
                &id,
                WorkspaceKind::Research,
                WorkspaceUpdate::Status {
                    status: WorkspaceRunStatus::Idle,
                    detail: "Plan only; no source, provider or database effects".into(),
                },
            )
            .await?;
            updates
                .send(TuiUpdate::WorkspaceFinished {
                    run_id: id.clone(),
                    status: WorkspaceRunStatus::Stopped,
                })
                .await?;
            Ok(())
        }
        .await;
        self.release_spaces(&id).await;
        result
    }

    async fn workspace_event(
        &self,
        event: WorkflowEvent,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        match event {
            WorkflowEvent::StageStarted {
                flow_id,
                stage,
                child_id,
            } => {
                if stage == WorkspaceKind::Building {
                    self.project_build_plan(&flow_id, updates).await?;
                }
                scoped(
                    updates,
                    &flow_id,
                    stage,
                    WorkspaceUpdate::Status {
                        status: WorkspaceRunStatus::Running,
                        detail: format!("{} · {child_id}", stage.label()),
                    },
                )
                .await?;
                scoped(
                    updates,
                    &flow_id,
                    stage,
                    WorkspaceUpdate::Transcript(TranscriptEntry::root(format!(
                        "Started {child_id}"
                    ))),
                )
                .await
            }
            WorkflowEvent::StageFinished {
                flow_id,
                stage,
                status,
                result,
            } => {
                self.workspace_stage(&flow_id, stage, status, &result, updates)
                    .await
            }
            WorkflowEvent::Dag { flow_id, event } => {
                self.workspace_dag_event(&flow_id, *event, updates).await
            }
        }
    }

    async fn workspace_stage(
        &self,
        id: &str,
        stage: WorkspaceKind,
        status: WorkspaceRunStatus,
        value: &Value,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let observed = value
            .get("observed_result")
            .filter(|v| v.is_object())
            .unwrap_or(value);
        if stage == WorkspaceKind::Building {
            self.restore_build_lanes(id, observed, status, updates)
                .await?;
        }
        let mut body = vec![
            format!("flow_id: {id}"),
            format!("workspace status: {status:?}"),
        ];
        body.extend(stage_lines(stage, observed));
        scoped(
            updates,
            id,
            stage,
            WorkspaceUpdate::Status {
                status,
                detail: stage_detail(stage, status, value),
            },
        )
        .await?;
        scoped(
            updates,
            id,
            stage,
            WorkspaceUpdate::View {
                title: stage.label().to_owned(),
                body: bounded_lines(body),
            },
        )
        .await?;
        if let Some(artifact) = artifact_view(stage, observed) {
            scoped(updates, id, stage, WorkspaceUpdate::Artifact(artifact)).await?;
        }
        self.apply_workspace_usage(value, updates).await
    }

    async fn workspace_snapshot(
        &self,
        id: &str,
        value: &Value,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        if let Some(plan_id) = value["plan_id"].as_str() {
            self.state
                .lock()
                .await
                .workspace_plan_ids
                .insert(id.into(), plan_id.into());
        }
        let mut seen = BTreeSet::new();
        for entry in value["stages"].as_array().into_iter().flatten() {
            let Some(stage) = serde_json::from_value::<WorkspaceKind>(entry["stage"].clone()).ok()
            else {
                continue;
            };
            let status = run_status(&entry["status"]).unwrap_or(WorkspaceRunStatus::Stopped);
            self.workspace_stage(id, stage, status, &entry["result"], updates)
                .await?;
            seen.insert(stage);
        }
        for stage in value["selected_stages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value::<WorkspaceKind>(v.clone()).ok())
        {
            if !seen.contains(&stage) {
                scoped(
                    updates,
                    id,
                    stage,
                    WorkspaceUpdate::Status {
                        status: WorkspaceRunStatus::Blocked,
                        detail: "Not admitted; inspect the preceding stage and original flow ID"
                            .into(),
                    },
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn apply_workspace_usage(
        &self,
        value: &Value,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let mut pending = vec![value];
        let mut remaining = 128;
        while let Some(item) = pending.pop() {
            if remaining == 0 {
                break;
            }
            remaining -= 1;
            match item {
                Value::Object(map) => {
                    if let Some(receipt) = map.get("receipt").or_else(|| map.get("planner_receipt"))
                    {
                        self.apply_usage(&json!({"receipt":receipt}), updates)
                            .await?;
                    }
                    pending.extend(map.values().filter(|v| v.is_object() || v.is_array()));
                }
                Value::Array(items) => {
                    pending.extend(items.iter().filter(|v| v.is_object() || v.is_array()))
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn project_build_plan(&self, id: &str, updates: &mpsc::Sender<TuiUpdate>) -> Result<()> {
        let plan_id = self.state.lock().await.workspace_plan_ids.get(id).cloned();
        let Some(plan_id) = plan_id else {
            return Ok(());
        };
        let record =
            match crate::workspaces::artifacts::read_plan(&self.gateway, &self.workspace, &plan_id)
                .await
            {
                Ok(Some(record)) => record,
                Ok(None) => return Ok(()),
                Err(error) => {
                    return scoped(
                        updates,
                        id,
                        WorkspaceKind::Building,
                        WorkspaceUpdate::Transcript(TranscriptEntry::root(format!(
                            "Plan projection unavailable; inspect {plan_id}: {error}"
                        ))),
                    )
                    .await;
                }
            };
        let plan = DagPlanner::compile(&record.artifact.objective, record.artifact.tasks)?;
        self.state
            .lock()
            .await
            .workspace_lanes
            .insert(id.into(), dag_lanes(&plan));
        self.publish_workspace_lanes(id, updates).await
    }

    async fn publish_workspace_lanes(
        &self,
        id: &str,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let lanes = self
            .state
            .lock()
            .await
            .workspace_lanes
            .get(id)
            .cloned()
            .unwrap_or_default();
        if lanes.is_empty() {
            return Ok(());
        }
        let done = lanes
            .iter()
            .filter(|lane| lane.state == LaneState::Accepted)
            .count();
        let total = lanes.len();
        scoped(
            updates,
            id,
            WorkspaceKind::Building,
            WorkspaceUpdate::Lanes(lanes),
        )
        .await?;
        scoped(
            updates,
            id,
            WorkspaceKind::Building,
            WorkspaceUpdate::Progress { done, total },
        )
        .await
    }

    async fn workspace_dag_event(
        &self,
        id: &str,
        event: DagExecutionEvent,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        if let DagExecutionEvent::ProviderFinished { output, .. } = &event {
            self.apply_workspace_usage(output, updates).await?;
        }
        {
            let mut state = self.state.lock().await;
            let lanes = state.workspace_lanes.entry(id.into()).or_default();
            match &event {
                DagExecutionEvent::WaveStarted {
                    wave_index,
                    node_ids,
                } => {
                    for lane in lanes.iter_mut().filter(|lane| node_ids.contains(&lane.id)) {
                        lane.layer = *wave_index;
                    }
                }
                DagExecutionEvent::NodeStarted { node } => {
                    if !lanes.iter().any(|lane| lane.id == node.id) {
                        let layers = node
                            .dependencies
                            .iter()
                            .map(|dependency| {
                                lanes
                                    .iter()
                                    .find(|lane| lane.id == *dependency)
                                    .map(|lane| lane.layer)
                            })
                            .collect::<Option<Vec<_>>>();
                        if let Some(layers) = layers {
                            lanes.push(crate::tui::DagLaneView {
                                id: node.id.clone(),
                                layer: layers.into_iter().max().map_or(0, |layer| layer + 1),
                                dependencies: node.dependencies.clone(),
                                title: node.title.clone(),
                                summary: node.task.clone(),
                                role: format!("{:?}", node.role),
                                model: node.model.clone(),
                                effort: node.reasoning_effort.clone(),
                                state: LaneState::Waiting,
                                note: "Observed node admission".into(),
                                elapsed: None,
                            });
                        }
                    }
                    if let Some(lane) = lanes.iter_mut().find(|lane| lane.id == node.id) {
                        lane.state = LaneState::Running;
                        lane.note = "Provider running".into();
                    }
                }
                DagExecutionEvent::ProviderFinished { node_id, .. }
                | DagExecutionEvent::NodeStopped { node_id } => {
                    if let Some(lane) = lanes.iter_mut().find(|lane| lane.id == *node_id)
                        && matches!(
                            lane.state,
                            LaneState::Running | LaneState::HandedOut | LaneState::Waiting
                        )
                    {
                        lane.state = LaneState::Stopped;
                        lane.note = "Provider stopped; verification/settlement pending".into();
                    }
                }
                DagExecutionEvent::NodeSettled { receipt } => {
                    if let Some(lane) = lanes.iter_mut().find(|lane| lane.id == receipt.node_id) {
                        lane.state = settlement_lane_state(&receipt.state);
                        lane.note = receipt
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("Gate passed: {}", receipt.verify_passed));
                        lane.elapsed =
                            std::time::Duration::try_from_secs_f64(receipt.duration_seconds).ok();
                    }
                }
                DagExecutionEvent::ProjectionLagged { .. }
                | DagExecutionEvent::WaveFinished { .. } => {}
            }
        }
        self.publish_workspace_lanes(id, updates).await?;
        let message = match event {
            DagExecutionEvent::ProjectionLagged { dropped_events } => format!(
                "Projection lagged ({dropped_events}); read original workflow/campaign status"
            ),
            DagExecutionEvent::WaveStarted {
                wave_index,
                node_ids,
            } => format!("Layer {} started: {}", wave_index + 1, node_ids.join(", ")),
            DagExecutionEvent::NodeStarted { node } => {
                format!("{} running · {}", node.id, node.model)
            }
            DagExecutionEvent::ProviderFinished { node_id, .. } => {
                format!("{node_id} provider returned; settlement remains pending")
            }
            DagExecutionEvent::NodeStopped { node_id } => {
                format!("{node_id} stopped; verification/settlement pending")
            }
            DagExecutionEvent::NodeSettled { receipt } => format!(
                "{} · {:?} · gate passed: {}",
                receipt.node_id, receipt.state, receipt.verify_passed
            ),
            DagExecutionEvent::WaveFinished { wave_index } => {
                format!("Layer {} finished", wave_index + 1)
            }
        };
        scoped(
            updates,
            id,
            WorkspaceKind::Building,
            WorkspaceUpdate::Transcript(TranscriptEntry::root(message)),
        )
        .await
    }

    async fn restore_build_lanes(
        &self,
        id: &str,
        value: &Value,
        status: WorkspaceRunStatus,
        updates: &mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        if self
            .state
            .lock()
            .await
            .workspace_lanes
            .get(id)
            .is_none_or(|lanes| lanes.is_empty())
        {
            self.project_build_plan(id, updates).await?;
        }
        let receipts = value["node_receipts"]
            .as_array()
            .or_else(|| value["receipts"].as_array());
        if let Some(receipts) = receipts {
            for value in receipts {
                if let Ok(receipt) =
                    serde_json::from_value::<crate::dag::SettlementReceipt>(value.clone())
                {
                    self.workspace_dag_event(
                        id,
                        DagExecutionEvent::NodeSettled { receipt },
                        updates,
                    )
                    .await?;
                }
            }
        }
        if matches!(
            status,
            WorkspaceRunStatus::Cancelled
                | WorkspaceRunStatus::TimedOutUnknown
                | WorkspaceRunStatus::Failed
                | WorkspaceRunStatus::Stopped
        ) {
            let mut state = self.state.lock().await;
            if let Some(lanes) = state.workspace_lanes.get_mut(id) {
                for lane in lanes.iter_mut().filter(|lane| {
                    matches!(
                        lane.state,
                        LaneState::Running | LaneState::Stopped | LaneState::HandedOut
                    )
                }) {
                    lane.state = LaneState::Unverified;
                    lane.note = "Interrupted; inspect original campaign evidence".into();
                }
            }
            drop(state);
            self.publish_workspace_lanes(id, updates).await?;
        }
        Ok(())
    }

    pub(super) async fn show_workflow_status(
        &self,
        id: String,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let value = crate::workspaces::status(&self.gateway, &self.workspace, id.trim()).await?;
        if value["error"].is_string()
            || value["selected_stages"]
                .as_array()
                .is_none_or(|stages| stages.is_empty())
        {
            return show_static_view(
                &updates,
                "Workflow observation",
                bounded_lines(vec![serde_json::to_string_pretty(&value)?]),
            )
            .await;
        }
        let Some(flow_id) = value["flow_id"].as_str() else {
            return show_static_view(
                &updates,
                "Workflow observation",
                bounded_lines(vec![serde_json::to_string_pretty(&value)?]),
            )
            .await;
        };
        let spaces = value["selected_stages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value::<WorkspaceKind>(v.clone()).ok())
            .collect::<Vec<_>>();
        let (busy, owned) = {
            let state = self.state.lock().await;
            (
                spaces.iter().any(|space| {
                    state
                        .workspace_runs
                        .get(space)
                        .is_some_and(|owner| owner != flow_id)
                }),
                spaces.iter().any(|space| {
                    state
                        .workspace_runs
                        .get(space)
                        .is_some_and(|owner| owner == flow_id)
                }),
            )
        };
        if busy {
            return show_static_view(
                &updates,
                "Workflow observation · active panes preserved",
                bounded_lines(vec![
                    format!("flow_id: {flow_id}"),
                    format!("status: {}", value["workflow_status"]),
                    "A selected pane belongs to another active run; its contents were preserved."
                        .into(),
                ]),
            )
            .await;
        }
        if !owned {
            updates
                .send(TuiUpdate::WorkspaceStarted {
                    run_id: flow_id.into(),
                    selected: spaces,
                    objective: "Restored explicit workflow snapshot".into(),
                })
                .await?;
        }
        self.workspace_snapshot(flow_id, &value, &updates).await?;
        if !owned {
            updates
                .send(TuiUpdate::WorkspaceFinished {
                    run_id: flow_id.into(),
                    status: run_status(&value["workflow_status"])
                        .unwrap_or(WorkspaceRunStatus::Stopped),
                })
                .await?;
        }
        if !owned {
            let mut state = self.state.lock().await;
            state.workspace_lanes.remove(flow_id);
            state.workspace_plan_ids.remove(flow_id);
        }
        Ok(())
    }

    pub(super) async fn cancel_workflow(
        &self,
        id: String,
        updates: mpsc::Sender<TuiUpdate>,
    ) -> Result<()> {
        let accepted = crate::workspaces::cancel(&self.gateway, &self.workspace, id.trim()).await?;
        show_static_view(&updates,"Workflow cancellation",vec![format!("flow_id: {}",id.trim()),if accepted{"Cancellation requested from the owning Gateway; current work must drain before later stages can proceed.".into()}else{"No cancellation was sent: this process does not own that active workflow.".into()}]).await
    }
}

async fn scoped(
    updates: &mpsc::Sender<TuiUpdate>,
    id: &str,
    workspace: WorkspaceKind,
    update: WorkspaceUpdate,
) -> Result<()> {
    updates
        .send(TuiUpdate::WorkspaceScoped {
            run_id: id.into(),
            workspace,
            update,
        })
        .await?;
    Ok(())
}
fn run_status(value: &Value) -> Option<WorkspaceRunStatus> {
    serde_json::from_value(value.clone()).ok()
}
fn stage_detail(stage: WorkspaceKind, status: WorkspaceRunStatus, value: &Value) -> String {
    if let Some(error) = value["error"].as_str() {
        return error.into();
    }
    if stage == WorkspaceKind::Research {
        return format!(
            "Research: {}",
            value["research_status"].as_str().unwrap_or("not observed")
        );
    }
    format!("{} · {status:?}", stage.label())
}
fn artifact_view(stage: WorkspaceKind, value: &Value) -> Option<WorkspaceArtifactView> {
    let (artifact_id, digest) = match stage {
        WorkspaceKind::Research => (
            value["request_id"].as_str()?,
            value["source_snapshot_digest"].as_str()?,
        ),
        WorkspaceKind::Planning => (
            value["plan_id"].as_str()?,
            value["artifact_digest"].as_str()?,
        ),
        WorkspaceKind::Building => (value["dag_id"].as_str()?, value["plan_identity"].as_str()?),
    };
    Some(WorkspaceArtifactView {
        artifact_id: artifact_id.into(),
        digest: digest.into(),
        label: stage.label().into(),
        ready: stage == WorkspaceKind::Planning && value["status"] == "build_ready",
    })
}
fn stage_lines(stage: WorkspaceKind, value: &Value) -> Vec<String> {
    match stage {
        WorkspaceKind::Research => crate::research_cli::report_lines(value),
        WorkspaceKind::Planning => {
            let mut lines =
                vec!["Planning proposal · mechanical readiness is not semantic approval".into()];
            for field in [
                "plan_id",
                "artifact_digest",
                "source_manifest_digest",
                "research_id",
                "status",
            ] {
                if let Some(text) = value[field].as_str() {
                    lines.push(format!("{field}: {text}"));
                }
            }
            let artifact = &value["artifact"];
            for field in ["title", "objective"] {
                if let Some(text) = artifact[field].as_str() {
                    lines.push(text.into());
                }
            }
            for field in [
                "requirements",
                "acceptance_criteria",
                "scope",
                "non_goals",
                "decisions",
                "risks",
                "blocking_questions",
            ] {
                if let Some(items) = artifact[field].as_array().filter(|items| !items.is_empty()) {
                    lines.push(format!("\n{field}"));
                    for item in items {
                        lines.push(format!(
                            "- {}",
                            item.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| item.to_string())
                        ));
                    }
                }
            }
            if let Some(tasks) = artifact["tasks"].as_array() {
                lines.push(format!("\nDevelopment tasks: {}", tasks.len()));
                for task in tasks {
                    lines.push(format!(
                        "{} · {} · writes: {}",
                        task["id"].as_str().unwrap_or("?"),
                        task["title"].as_str().unwrap_or(""),
                        task["write_files"]
                    ));
                }
            }
            if let Some(reasons) = value["readiness_reasons"].as_array() {
                for reason in reasons {
                    if let Some(text) = reason.as_str() {
                        lines.push(format!("Readiness: {text}"));
                    }
                }
            }
            lines
        }
        WorkspaceKind::Building => {
            let mut lines = vec!["Building · worktree DAG settlement".into()];
            for field in ["dag_id", "status", "error", "plan_id", "artifact_digest"] {
                if let Some(text) = value[field].as_str() {
                    lines.push(format!("{field}: {text}"));
                }
            }
            if let Some(ok) = value["ok"].as_bool() {
                lines.push(format!("verified result: {ok}"));
            }
            for receipt in value["node_receipts"]
                .as_array()
                .or_else(|| value["receipts"].as_array())
                .into_iter()
                .flatten()
            {
                lines.push(format!(
                    "{} · {}",
                    receipt["node_id"].as_str().unwrap_or("?"),
                    receipt["state"].as_str().unwrap_or("not observed")
                ));
            }
            if lines.len() == 1 {
                lines.push("Inspect the persisted workflow and building campaign IDs for current evidence.".into());
            }
            lines
        }
    }
}
fn bounded_lines(lines: Vec<String>) -> Vec<String> {
    let mut remaining = VIEW_BYTE_LIMIT;
    lines
        .into_iter()
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control() || *c == '\t' || *c == '\n')
                .collect::<String>()
        })
        .map_while(|line| {
            if remaining == 0 {
                return None;
            }
            let text = truncate_utf8(&line, remaining).to_owned();
            remaining = remaining.saturating_sub(text.len() + 1);
            Some(text)
        })
        .collect()
}

fn settlement_lane_state(state: &crate::dag::NodeState) -> LaneState {
    use crate::dag::NodeState;
    match state {
        NodeState::Settled => LaneState::Accepted,
        NodeState::Planned => LaneState::Waiting,
        NodeState::Running => LaneState::Running,
        NodeState::Stopped => LaneState::Stopped,
        NodeState::RejectedOutOfScopeWrite
        | NodeState::RejectedDirtyRead
        | NodeState::RejectedVerification
        | NodeState::RejectedReproductionNotObserved
        | NodeState::BlockedUpstream
        | NodeState::FailedExecution => LaneState::Rejected,
    }
}

fn legacy_projection(id: &str, update: TuiUpdate) -> TuiUpdate {
    let update = match update {
        TuiUpdate::Transcript(entry) => WorkspaceUpdate::Transcript(entry),
        TuiUpdate::View { title, body } => WorkspaceUpdate::View { title, body },
        TuiUpdate::DagSnapshot(lanes) => WorkspaceUpdate::Lanes(lanes),
        TuiUpdate::HeaderProgress { done, total } => WorkspaceUpdate::Progress { done, total },
        other => return other,
    };
    TuiUpdate::WorkspaceScoped {
        run_id: id.into(),
        workspace: WorkspaceKind::Building,
        update,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::{
        DagNode, GateProof, NodeKind, NodeRole, NodeState, SettlementReceipt, SettlementStage,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn node(id: &str, dependencies: Vec<String>) -> DagNode {
        DagNode {
            id: id.into(),
            title: id.into(),
            task: "bounded UI fixture".into(),
            role: NodeRole::InterfaceBuilder,
            node_kind: NodeKind::Standard,
            model: "gpt-6.1-sol".into(),
            reasoning_effort: "high".into(),
            dependencies,
            read_files: vec![],
            write_files: vec![],
            symbols: vec![],
            verify_command: Some("cargo test".into()),
            reproduction_expected_exit_nonzero: true,
        }
    }
    fn settled(id: &str) -> SettlementReceipt {
        SettlementReceipt {
            node_id: id.into(),
            state: NodeState::Settled,
            settlement_stage: SettlementStage::Settled,
            role: NodeRole::InterfaceBuilder,
            node_kind: NodeKind::Standard,
            wave_index: 0,
            model_id: "gpt-6.1-sol".into(),
            duration_seconds: 0.25,
            pre_manifest_digest: "sha256:before".into(),
            stopped_manifest_digest: "sha256:after".into(),
            post_manifest_digest: "sha256:after".into(),
            changed_files: vec![],
            mutated_files: vec![],
            out_of_scope_writes: vec![],
            dirty_read_files: vec![],
            verify_passed: true,
            reproduction_observed: false,
            output_summary: "observed fixture".into(),
            error: None,
            gate: GateProof {
                command: "cargo test".into(),
                executed: true,
                exit_code: Some(0),
                stdout_digest: Some("sha256:gate".into()),
            },
        }
    }
    #[tokio::test]
    async fn reserved_chain_rejects_legacy_building_before_provider_or_workspace_effects()
    -> Result<()> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        std::fs::create_dir(&original)?;
        let state = root.path().join("state");
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let gateway = Gateway::new(state.clone()).with_test_provider(move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Err("provider must not be admitted".into())
        });
        let handler = AppHandler::new(gateway, original)?;
        let selected = [
            WorkspaceKind::Research,
            WorkspaceKind::Planning,
            WorkspaceKind::Building,
        ];
        handler.reserve_spaces("chain-original", &selected).await?;
        for action in [
            TuiAction::RunDag("change a module".into()),
            TuiAction::SubmitPrompt("change a module".into()),
            TuiAction::PlanDag("change a module".into()),
        ] {
            let (tx, _rx) = mpsc::channel(16);
            assert!(handler.dispatch(action, tx).await.is_err());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!state.exists());
        assert_eq!(handler.state.lock().await.workspace_runs.len(), 3);
        handler.release_spaces("chain-original").await;
        let (tx, mut rx) = mpsc::channel(8);
        let lease = handler
            .begin_legacy_building(&TuiAction::SubmitPrompt("explicit".into()), &tx)
            .await?
            .ok_or("legacy lease missing")?;
        assert_eq!(
            handler
                .state
                .lock()
                .await
                .workspace_runs
                .get(&WorkspaceKind::Building),
            Some(&lease)
        );
        handler.finish_legacy_building(&lease, &Ok(()), &tx).await;
        assert!(handler.state.lock().await.workspace_runs.is_empty());
        assert!(matches!(
            rx.recv().await,
            Some(TuiUpdate::WorkspaceStarted { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn observed_dag_events_update_scoped_lanes_and_only_receipts_advance_verified_progress()
    -> Result<()> {
        let root = tempfile::tempdir()?;
        let handler = AppHandler::new(Gateway::new(root.path().join("state")), root.path().into())?;
        let (tx, mut rx) = mpsc::channel(32);
        handler
            .workspace_dag_event(
                "flow",
                DagExecutionEvent::NodeStarted {
                    node: node("writer", vec![]),
                },
                &tx,
            )
            .await?;
        handler
            .workspace_dag_event(
                "flow",
                DagExecutionEvent::ProviderFinished {
                    node_id: "writer".into(),
                    output: json!({}),
                },
                &tx,
            )
            .await?;
        assert_eq!(
            handler.state.lock().await.workspace_lanes["flow"][0].state,
            LaneState::Stopped
        );
        handler
            .workspace_dag_event(
                "flow",
                DagExecutionEvent::NodeSettled {
                    receipt: settled("writer"),
                },
                &tx,
            )
            .await?;
        handler
            .workspace_dag_event(
                "flow",
                DagExecutionEvent::NodeStarted {
                    node: node("verifier", vec!["writer".into()]),
                },
                &tx,
            )
            .await?;
        let lanes = handler.state.lock().await.workspace_lanes["flow"].clone();
        assert_eq!(lanes[0].state, LaneState::Accepted);
        assert_eq!(lanes[1].layer, 1);
        assert_eq!(lanes[1].state, LaneState::Running);
        drop(tx);
        let mut progress = Vec::new();
        let mut other = false;
        while let Some(update) = rx.recv().await {
            match update {
                TuiUpdate::WorkspaceScoped {
                    run_id,
                    workspace,
                    update: WorkspaceUpdate::Progress { done, total },
                } => {
                    assert_eq!(run_id, "flow");
                    assert_eq!(workspace, WorkspaceKind::Building);
                    progress.push((done, total));
                }
                TuiUpdate::DagSnapshot(_)
                | TuiUpdate::LaneState { .. }
                | TuiUpdate::HeaderProgress { .. } => other = true,
                _ => {}
            }
        }
        assert_eq!(progress, [(0, 1), (0, 1), (1, 1), (1, 2)]);
        assert!(!other);
        Ok(())
    }

    #[test]
    fn workflow_observation_and_cancellation_never_request_new_named_sessions() -> Result<()> {
        let root = tempfile::tempdir()?;
        for command in [
            "/workflow-status flow-original",
            "/workflow-cancel flow-original",
        ] {
            assert!(initial_existing_session_input(root.path(), None, command)?);
        }
        Ok(())
    }
    #[test]
    fn interactive_legacy_view_is_scoped_and_workspace_switch_dismisses_operator_overlay() {
        use crate::tui::{TuiApp, TuiCatalog};
        let mut app =
            TuiApp::new_with_motion(PathBuf::from("workspace"), TuiCatalog::default(), false);
        app.apply_update(TuiUpdate::WorkspaceStarted {
            run_id: "legacy".into(),
            selected: vec![WorkspaceKind::Building],
            objective: "build".into(),
        });
        app.apply_update(TuiUpdate::ValidatedAction(TuiAction::SelectWorkspace(
            WorkspaceKind::Research,
        )));
        app.apply_update(legacy_projection(
            "legacy",
            TuiUpdate::View {
                title: "Late Building result".into(),
                body: vec!["actual result".into()],
            },
        ));
        assert_eq!(app.active_workspace(), WorkspaceKind::Research);
        assert!(app.current_view().is_none());
        app.apply_update(TuiUpdate::ValidatedAction(TuiAction::SelectWorkspace(
            WorkspaceKind::Building,
        )));
        assert_eq!(
            app.current_view().map(|v| v.0),
            Some("Late Building result")
        );
        app.apply_update(TuiUpdate::View {
            title: "Models".into(),
            body: vec!["operator overlay".into()],
        });
        assert_eq!(app.current_view().map(|v| v.0), Some("Models"));
        app.apply_update(TuiUpdate::ValidatedAction(TuiAction::SelectWorkspace(
            WorkspaceKind::Research,
        )));
        assert!(app.current_view().is_none());
    }

    #[tokio::test]
    async fn missing_workflow_status_is_visible_without_reserving_or_initializing_spaces()
    -> Result<()> {
        let root = tempfile::tempdir()?;
        let state = root.path().join("missing-state");
        let handler = AppHandler::new(Gateway::new(state.clone()), root.path().into())?;
        let (tx, mut rx) = mpsc::channel(8);
        handler
            .show_workflow_status("missing-flow".into(), tx)
            .await?;
        let Some(TuiUpdate::View { title, body }) = rx.recv().await else {
            return Err("read-only observation missing".into());
        };
        assert_eq!(title, "Workflow observation");
        assert!(body.join("\n").contains("missing_database"));
        assert!(handler.state.lock().await.workspace_runs.is_empty());
        assert!(!state.exists());
        Ok(())
    }
    #[tokio::test]
    async fn closed_ui_sink_drains_the_original_workflow_and_releases_space_ownership() -> Result<()>
    {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        std::fs::create_dir(&original)?;
        std::fs::write(
            original.join("README.md"),
            "Manual review planning fixture.\n",
        )?;
        let gateway=Gateway::new(root.path().join("state")).with_test_provider(|request,options|{
            let route=crate::routing::validate_route(request)?;
            let mut receipt=crate::routing::new_receipt_with_context(&route,options.request_id.as_deref(),options.idempotency_key.clone());
            receipt.actual_model=Some(route.resolved_model);receipt.identity_source=Some("external_test_transport".into());
            receipt.owned_processes_closed=Some(true);receipt.status=crate::routing::ReceiptStatus::Completed;receipt.outcome_state="confirmed_success".into();crate::routing::close_receipt(&mut receipt);
            let artifact=json!({"title":"Manual review","objective":"Plan a manual review","requirements":["Read the specification"],"acceptance_criteria":["A review checklist exists"],"scope":["Planning only"],"non_goals":["Code changes"],"decisions":[],"risks":[],"blocking_questions":[],"tasks":[]});
            let output=artifact.to_string();receipt.output_digest=Some(crate::routing::digest_output(&output));
            Ok(crate::runtime::ExecutionResult{receipt,output,structured_output:Some(artifact),placeholder:None,session_id:None,exit_code:Some(0),search_evidence:None})
        });
        let handler = AppHandler::new(gateway.clone(), original.clone())?;
        let mut request = WorkflowRequest::new(
            "Plan a manual review",
            original,
            vec![WorkspaceKind::Planning],
        );
        request.request_id = Some("flow-closed-ui".into());
        let (tx, mut rx) = mpsc::channel(1);
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            tokio::join!(handler.run_workspace_request(request, tx), async move {
                while let Some(update) = rx.recv().await {
                    if matches!(
                        update,
                        TuiUpdate::WorkspaceScoped {
                            update: WorkspaceUpdate::Status {
                                status: WorkspaceRunStatus::Running,
                                ..
                            },
                            ..
                        }
                    ) {
                        break;
                    }
                }
                drop(rx);
            })
        })
        .await?;
        assert!(result.is_err());
        let record = gateway
            .database(|store| store.get_job("flow-closed-ui"))
            .await?
            .ok_or("original workflow record missing")?;
        assert_ne!(record.status, crate::storage::JobStatus::Running);
        assert!(record.metadata.contains_key("expected_planning_job_id"));
        assert!(handler.state.lock().await.workspace_runs.is_empty());
        gateway.shutdown().await?;
        Ok(())
    }
}
