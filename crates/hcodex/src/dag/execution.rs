use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::Result;
use crate::gateway::Gateway;

use super::patch::{
    AppliedScopedPatch, apply_scoped_patch_with_rollback, normalize_relative, preimage_contract,
    validate_destination,
};
use super::settlement::SettlementEvidence;
use super::{
    DagNode, DagPlan, DagSettlementVerifier, GateProof, NodeKind, NodeRole, NodeState,
    PatchArtifact, SettlementReceipt, SettlementStage, WorkspaceManifest,
};

const DEFAULT_MAX_PARALLEL_NODES: usize = 4;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DagExecutionEvent {
    WaveStarted {
        wave_index: usize,
        node_ids: Vec<String>,
    },
    NodeStarted {
        node: DagNode,
    },
    ProviderFinished {
        node_id: String,
        output: Value,
    },
    NodeStopped {
        node_id: String,
    },
    NodeSettled {
        receipt: SettlementReceipt,
    },
    WaveFinished {
        wave_index: usize,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct DagExecutionOptions {
    pub max_parallel: usize,
    pub max_input_bytes: u64,
    pub force_serial: bool,
    pub resumed: BTreeMap<String, SettlementReceipt>,
}

#[derive(Debug, Serialize)]
struct ExecutionReport {
    ok: bool,
    plan: DagPlan,
    receipts: Vec<SettlementReceipt>,
    outputs: BTreeMap<String, Value>,
    aimd_final_concurrency: usize,
}

struct AimdCapacity {
    ceiling: usize,
    current: usize,
}

impl AimdCapacity {
    fn new(ceiling: usize) -> Self {
        let ceiling = ceiling.clamp(1, 6);
        Self {
            ceiling,
            current: ceiling,
        }
    }

    fn congested(&mut self) {
        self.current = (self.current / 2).max(1);
    }

    fn wave_settled(&mut self) {
        self.current = (self.current + 1).min(self.ceiling);
    }
}

/// Read only transport-owned control fields, never the model's narrative.
fn congestion_feedback(output: &Value) -> bool {
    ["/result/receipt", "/receipt", "/result/result/receipt"]
        .iter()
        .any(|pointer| {
            let Some(receipt) = output.pointer(pointer) else {
                return false;
            };
            receipt["status"] == "timed_out_unknown"
                || matches!(
                    receipt["error_code"].as_str(),
                    Some(
                        "ANTIGRAVITY_QUOTA_EXHAUSTED"
                            | "ANTIGRAVITY_CIRCUIT_OPEN"
                            | "RATE_LIMITED"
                            | "RATE_LIMIT_EXCEEDED"
                            | "RESOURCE_EXHAUSTED"
                            | "MODEL_AT_CAPACITY"
                    )
                )
        })
}

pub async fn execute(
    gateway: &Gateway,
    plan: DagPlan,
    workspace: PathBuf,
    timeout: Duration,
) -> Result<Value> {
    execute_with_observer(gateway, plan, workspace, timeout, None).await
}

pub async fn execute_with_observer(
    gateway: &Gateway,
    plan: DagPlan,
    workspace: PathBuf,
    timeout: Duration,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
) -> Result<Value> {
    execute_with_options(
        gateway,
        plan,
        workspace,
        timeout,
        DagExecutionOptions {
            max_parallel: DEFAULT_MAX_PARALLEL_NODES,
            max_input_bytes: 256_000,
            force_serial: false,
            resumed: BTreeMap::new(),
        },
        observer,
    )
    .await
}

pub(crate) async fn execute_with_options(
    gateway: &Gateway,
    plan: DagPlan,
    workspace: PathBuf,
    timeout: Duration,
    options: DagExecutionOptions,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
) -> Result<Value> {
    if !plan.is_executable() {
        return Err("DAG contains blocking preflight hazards".into());
    }
    let workspace = workspace.canonicalize()?;
    let execution_gateway = gateway.clone();
    let execution_workspace = workspace.clone();
    let execution_observer = observer.clone();
    let max_input_bytes = options.max_input_bytes;
    let (mut receipts, outputs, aimd_final_concurrency) = execute_ready_nodes(
        &plan,
        options,
        observer,
        move |node, wave_index, handoffs| {
            let gateway = execution_gateway.clone();
            let workspace = execution_workspace.clone();
            let observer = execution_observer.clone();
            async move {
                let baseline = capture(&workspace).await?;
                if handoffs
                    .iter()
                    .any(|receipt| receipt.state != NodeState::Settled)
                {
                    let receipt = blocked_receipt(&node, &baseline.digest, wave_index);
                    emit(
                        &observer,
                        DagExecutionEvent::NodeSettled {
                            receipt: receipt.clone(),
                        },
                    )
                    .await;
                    return Ok((receipt, json!({"ok": false, "blocked_upstream": true})));
                }
                execute_one(
                    &gateway,
                    node,
                    &workspace,
                    timeout,
                    max_input_bytes,
                    wave_index,
                    &baseline,
                    &handoffs,
                    &observer,
                )
                .await
            }
        },
    )
    .await?;
    let receipt_values: Vec<SettlementReceipt> = plan
        .nodes
        .iter()
        .filter_map(|node| receipts.remove(&node.id))
        .collect();
    let ok = receipt_values.len() == plan.nodes.len()
        && receipt_values
            .iter()
            .all(|receipt| receipt.state == NodeState::Settled);
    Ok(serde_json::to_value(ExecutionReport {
        ok,
        plan,
        receipts: receipt_values,
        outputs,
        aimd_final_concurrency,
    })?)
}

/// Waves describe the graph for presentation; only dependency settlement and
/// capacity govern admission. A writer owns the shared workspace exclusively so
/// a full-manifest settlement cannot attribute another node's effects to it.
async fn execute_ready_nodes<F, Fut>(
    plan: &DagPlan,
    options: DagExecutionOptions,
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
    mut execute_node: F,
) -> Result<(
    BTreeMap<String, SettlementReceipt>,
    BTreeMap<String, Value>,
    usize,
)>
where
    F: FnMut(DagNode, usize, Vec<SettlementReceipt>) -> Fut,
    Fut: Future<Output = Result<(SettlementReceipt, Value)>> + Send + 'static,
{
    let mut pending: BTreeMap<String, DagNode> = plan
        .nodes
        .iter()
        .filter(|node| !options.resumed.contains_key(&node.id))
        .cloned()
        .map(|node| (node.id.clone(), node))
        .collect();
    let mut receipts = options.resumed;
    let mut outputs = BTreeMap::new();
    let wave_by_node: BTreeMap<&str, usize> = plan
        .waves
        .iter()
        .flat_map(|wave| {
            wave.node_ids
                .iter()
                .map(move |id| (id.as_str(), wave.index))
        })
        .collect();
    let max_parallel = if options.force_serial {
        1
    } else {
        options.max_parallel.clamp(1, 6)
    };
    let mut capacity = AimdCapacity::new(max_parallel);
    let mut tasks = JoinSet::new();
    let mut active_writer = None;
    let mut started_waves = BTreeSet::new();
    let mut finished_waves = plan
        .waves
        .iter()
        .filter(|wave| wave.node_ids.iter().all(|id| receipts.contains_key(id)))
        .map(|wave| wave.index)
        .collect::<BTreeSet<_>>();
    let mut congested_waves = BTreeSet::new();
    let mut first_error = None;

    for receipt in receipts.values() {
        emit(
            &observer,
            DagExecutionEvent::NodeSettled {
                receipt: receipt.clone(),
            },
        )
        .await;
    }

    while !pending.is_empty() || !tasks.is_empty() {
        if crate::supervisor::is_cancelled() {
            first_error.get_or_insert_with(|| "DAG campaign cancelled".into());
        }
        let ready: Vec<String> = pending
            .iter()
            .filter(|(_, node)| {
                node.dependencies
                    .iter()
                    .all(|dependency| receipts.contains_key(dependency))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ready {
            if first_error.is_some() || tasks.len() >= capacity.current || active_writer.is_some() {
                break;
            }
            let node = pending.get(&id).ok_or("ready DAG node disappeared")?;
            let writes = !node.write_files.is_empty();
            if writes && !tasks.is_empty() {
                continue;
            }
            let node = pending.remove(&id).ok_or("ready DAG node disappeared")?;
            let wave_index = *wave_by_node
                .get(id.as_str())
                .ok_or("DAG node has no wave")?;
            if started_waves.insert(wave_index) {
                let wave = plan
                    .waves
                    .iter()
                    .find(|wave| wave.index == wave_index)
                    .ok_or("DAG wave disappeared")?;
                emit(
                    &observer,
                    DagExecutionEvent::WaveStarted {
                        wave_index,
                        node_ids: wave.node_ids.clone(),
                    },
                )
                .await;
            }
            let handoffs = node
                .dependencies
                .iter()
                .filter_map(|dependency| receipts.get(dependency).cloned())
                .collect();
            let execution = execute_node(node, wave_index, handoffs);
            if writes {
                active_writer = Some(id.clone());
            }
            tasks.spawn(crate::supervisor::inherit_context(async move {
                (id, execution.await)
            }));
        }

        let Some(joined) = tasks.join_next().await else {
            if let Some(error) = first_error {
                return Err(error);
            }
            return Err("DAG has pending nodes with no settled dependency frontier".into());
        };
        match joined {
            Ok((id, result)) => {
                if active_writer.as_ref() == Some(&id) {
                    active_writer = None;
                }
                match result {
                    Ok((receipt, output)) => {
                        if congestion_feedback(&output) {
                            capacity.congested();
                            congested_waves.insert(receipt.wave_index);
                        }
                        outputs.insert(receipt.node_id.clone(), output);
                        receipts.insert(receipt.node_id.clone(), receipt);
                    }
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            Err(error) => {
                first_error.get_or_insert_with(|| error.into());
            }
        }
        for wave in &plan.waves {
            if !finished_waves.contains(&wave.index)
                && wave.node_ids.iter().all(|id| receipts.contains_key(id))
            {
                finished_waves.insert(wave.index);
                if !congested_waves.contains(&wave.index)
                    && wave.node_ids.iter().all(|id| {
                        receipts
                            .get(id)
                            .is_some_and(|receipt| receipt.state == NodeState::Settled)
                    })
                {
                    capacity.wave_settled();
                }
                emit(
                    &observer,
                    DagExecutionEvent::WaveFinished {
                        wave_index: wave.index,
                    },
                )
                .await;
            }
        }
        // Preserve ownership of durable provider waiters on an error. Stop new
        // admissions and drain already-admitted work to its bounded outcome;
        // dropping those waiters would lose provider receipts and identities.
        if first_error.is_some() && tasks.is_empty() {
            break;
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok((receipts, outputs, capacity.current))
}

#[allow(clippy::too_many_arguments)]
async fn execute_one(
    gateway: &Gateway,
    node: DagNode,
    workspace: &Path,
    timeout: Duration,
    max_input_bytes: u64,
    wave_index: usize,
    wave_baseline: &WorkspaceManifest,
    handoffs: &[SettlementReceipt],
    observer: &Option<mpsc::Sender<DagExecutionEvent>>,
) -> Result<(SettlementReceipt, Value)> {
    let started = Instant::now();
    emit(
        observer,
        DagExecutionEvent::NodeStarted { node: node.clone() },
    )
    .await;
    let pre = capture(workspace).await?;
    let dirty_read_files = dirty_read_files(&node, wave_baseline, &pre);
    if !dirty_read_files.is_empty() {
        let receipt = DagSettlementVerifier::settle_with_evidence(
            &node,
            &pre,
            &pre,
            GateProof {
                command: "not_run: dirty read detected before dispatch".to_owned(),
                executed: false,
                exit_code: None,
                stdout_digest: None,
            },
            SettlementEvidence {
                wave_index,
                duration_seconds: started.elapsed().as_secs_f64(),
                dirty_read_files,
                reproduction_observed: false,
                output_summary: "Rejected before provider dispatch due to dirty read scope."
                    .to_owned(),
                error: Some("declared read files changed after wave baseline".to_owned()),
            },
        );
        emit(
            observer,
            DagExecutionEvent::NodeStopped {
                node_id: node.id.clone(),
            },
        )
        .await;
        emit(
            observer,
            DagExecutionEvent::NodeSettled {
                receipt: receipt.clone(),
            },
        )
        .await;
        return Ok((receipt, json!({"ok": false, "dirty_read": true})));
    }
    let args = match node_arguments(&node, workspace, timeout, max_input_bytes, &pre, handoffs) {
        Ok(args) => args,
        Err(error) => {
            let reason = error.to_string();
            let receipt = DagSettlementVerifier::settle_with_evidence(
                &node,
                &pre,
                &pre,
                GateProof {
                    command: format!("not_run: {reason}"),
                    executed: false,
                    exit_code: None,
                    stdout_digest: None,
                },
                SettlementEvidence {
                    wave_index,
                    duration_seconds: started.elapsed().as_secs_f64(),
                    dirty_read_files: Vec::new(),
                    reproduction_observed: false,
                    output_summary: format!("Rejected before provider dispatch: {reason}"),
                    error: Some(reason.clone()),
                },
            );
            emit(
                observer,
                DagExecutionEvent::NodeStopped {
                    node_id: node.id.clone(),
                },
            )
            .await;
            emit(
                observer,
                DagExecutionEvent::NodeSettled {
                    receipt: receipt.clone(),
                },
            )
            .await;
            return Ok((
                receipt,
                json!({"ok": false, "rejected_pre_dispatch": true, "error": reason}),
            ));
        }
    };
    // The provider runtime owns its timeout and persists that exact outcome. An
    // outer timeout would drop the waiter before that durable write completes.
    let mut output = match gateway.execute_tool(tool_name(&node), &args).await {
        Ok(value) => value,
        Err(error) => json!({"ok": false, "error": error.to_string()}),
    };
    emit(
        observer,
        DagExecutionEvent::ProviderFinished {
            node_id: node.id.clone(),
            output: output.clone(),
        },
    )
    .await;
    let mut applied_patch: Option<AppliedScopedPatch> = None;
    if output.get("ok").and_then(Value::as_bool) == Some(true)
        && node.role == NodeRole::InterfaceBuilder
        && !node.write_files.is_empty()
    {
        let provider_stopped = capture(workspace).await?;
        let patch_result: Result<AppliedScopedPatch> = if provider_stopped.files != pre.files {
            Err("provider mutated workspace before scoped patch apply".into())
        } else {
            extract_patch_artifact(&output).and_then(|artifact| {
                apply_scoped_patch_with_rollback(workspace, &node.write_files, &pre, &artifact)
            })
        };
        match patch_result {
            Ok(applied) => applied_patch = Some(applied),
            Err(error) => {
                output = json!({
                    "ok": false,
                    "patch_error": error.to_string(),
                    "provider_output": output,
                });
            }
        }
    }
    emit(
        observer,
        DagExecutionEvent::NodeStopped {
            node_id: node.id.clone(),
        },
    )
    .await;
    let pre_gate = match capture(workspace).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if let Some(patch) = applied_patch.take() {
                let _ = patch.rollback();
            }
            return Err(error);
        }
    };
    let (gate, stopped) = if output.get("ok").and_then(Value::as_bool) == Some(true) {
        let gate = match run_gate_for_manifest(
            workspace,
            node.verify_command.as_deref(),
            timeout,
            &pre_gate.digest,
            node.node_kind,
        )
        .await
        {
            Ok(gate) => gate,
            Err(error) => {
                if let Some(patch) = applied_patch.take() {
                    let _ = patch.rollback();
                }
                return Err(error);
            }
        };
        let stopped = match capture(workspace).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(patch) = applied_patch.take() {
                    let _ = patch.rollback();
                }
                return Err(error);
            }
        };
        (gate, stopped)
    } else if let Some(patch_error) = output.get("patch_error").and_then(Value::as_str) {
        (
            GateProof {
                command: format!("not_run: {patch_error}"),
                executed: false,
                exit_code: None,
                stdout_digest: None,
            },
            pre_gate,
        )
    } else {
        (
            GateProof {
                command: "not_run: node execution failed".to_owned(),
                executed: false,
                exit_code: None,
                stdout_digest: None,
            },
            pre_gate,
        )
    };
    let reproduction_observed = node.node_kind == NodeKind::Reproduction
        && if node.reproduction_expected_exit_nonzero {
            gate.executed && gate.exit_code.is_some_and(|code| code != 0)
        } else {
            gate.passed()
        };
    let output_summary = extract_output_summary(&output);
    let execution_error =
        if let Some(patch_error) = output.get("patch_error").and_then(Value::as_str) {
            Some(patch_error.to_owned())
        } else if output.get("ok").and_then(Value::as_bool) != Some(true) {
            Some("provider execution did not confirm success".to_owned())
        } else {
            None
        };
    let mut receipt = DagSettlementVerifier::settle_with_evidence(
        &node,
        &pre,
        &stopped,
        gate,
        SettlementEvidence {
            wave_index,
            duration_seconds: started.elapsed().as_secs_f64(),
            dirty_read_files: Vec::new(),
            reproduction_observed,
            output_summary,
            error: execution_error,
        },
    );
    if !receipt.verify_passed
        && let Some(applied) = applied_patch.take()
    {
        applied.rollback()?;
    }
    if let Some(actual_model) = extract_actual_model(&output) {
        receipt.model_id = actual_model;
    }
    emit(
        observer,
        DagExecutionEvent::NodeSettled {
            receipt: receipt.clone(),
        },
    )
    .await;
    Ok((receipt, output))
}

fn dirty_read_files(
    node: &DagNode,
    wave_baseline: &WorkspaceManifest,
    current: &WorkspaceManifest,
) -> Vec<PathBuf> {
    node.read_files
        .iter()
        .filter(|path| wave_baseline.files.get(*path) != current.files.get(*path))
        .cloned()
        .collect()
}

fn extract_actual_model(output: &Value) -> Option<String> {
    [
        "/result/receipt/actual_model",
        "/receipt/actual_model",
        "/result/result/receipt/actual_model",
        "/provider_output/result/receipt/actual_model",
        "/provider_output/receipt/actual_model",
    ]
    .iter()
    .find_map(|pointer| output.pointer(pointer).and_then(Value::as_str))
    .map(str::to_owned)
}

fn extract_output_summary(output: &Value) -> String {
    let base = [
        "/result/structured_output/summary",
        "/result/structured_output/executive_critique",
        "/structured_output/summary",
        "/result/output",
        "/provider_output/result/structured_output/summary",
        "/provider_output/result/structured_output/executive_critique",
        "/provider_output/structured_output/summary",
        "/provider_output/result/output",
    ]
    .iter()
    .find_map(|pointer| output.pointer(pointer).and_then(Value::as_str))
    .unwrap_or("Provider completed without a structured summary.");
    let summary = if let Some(patch_error) = output.get("patch_error").and_then(Value::as_str) {
        format!("patch_error: {patch_error} — {base}")
    } else {
        base.to_owned()
    };
    summary.chars().take(1_000).collect()
}

fn tool_name(node: &DagNode) -> &'static str {
    if node.role == NodeRole::InterfaceBuilder {
        "claude_code_craft"
    } else {
        "codex_adversarial_audit"
    }
}

const NODE_CONTEXT_CEILING_BYTES: usize = 120 * 1024;
const NODE_CONTEXT_RESERVE_BYTES: usize = 64 * 1024;

fn node_arguments(
    node: &DagNode,
    workspace: &Path,
    timeout: Duration,
    max_input_bytes: u64,
    pre_manifest: &WorkspaceManifest,
    handoffs: &[SettlementReceipt],
) -> Result<Value> {
    let files: Vec<String> = node
        .read_files
        .iter()
        .chain(&node.write_files)
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let task = format!("{}{}", node.task, dependency_context(handoffs)?);
    let invariants = "Return structured_output.crafted_code as strict JSON: {\"edits\":[{\"path\":\"declared/relative/path\",\"preimage_sha256\":\"sha256:<exact>\",\"new_content\":\"complete replacement text\"}]}. Unknown fields, omitted preimages, prose, markdown fences, partial snippets, and undeclared paths are rejected. Do not mutate files directly. Each edit's new_content replaces the entire file; use the COMPLETE source blocks in target_code_or_context and preserve all lines and tests outside your intended changes.";
    let contract_value = if node.role == NodeRole::InterfaceBuilder {
        Some(serde_json::to_value(preimage_contract(
            pre_manifest,
            &node.write_files,
        )?)?)
    } else {
        None
    };
    let contract_bytes = contract_value
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .map_or(0, |text| text.len());
    let total_cap = usize::try_from(max_input_bytes).unwrap_or(256_000);
    let overhead = NODE_CONTEXT_RESERVE_BYTES
        .saturating_add(task.len())
        .saturating_add(invariants.len())
        .saturating_add(contract_bytes);
    let context_budget = NODE_CONTEXT_CEILING_BYTES.min(total_cap.saturating_sub(overhead));
    let target_context = node_target_context(workspace, node, pre_manifest, context_budget)?;
    let mut args = json!({
        "workspace_dir": workspace,
        "files_inspected": files,
        "symbols_inspected": node.symbols,
        "reasoning_effort": node.reasoning_effort,
        "timeout_seconds": timeout.as_secs_f64(),
        "max_input_bytes": max_input_bytes,
    });
    if node.role == NodeRole::InterfaceBuilder {
        args["task_spec"] = Value::String(task);
        args["target_code_or_context"] = Value::String(target_context);
        args["claude_model"] = Value::String(node.model.clone());
        args["invariants_and_rules"] = Value::String(invariants.to_owned());
        if let Some(contract) = contract_value {
            args["patch_contract"] = contract;
        }
    } else {
        args["audit_focus"] = Value::String(task);
        args["target_code_or_diff"] = Value::String(target_context);
        args["design_rationale"] = Value::String(format!("DAG lane {} ({:?})", node.id, node.role));
        args["model"] = Value::String(node.model.clone());
        if node.model.starts_with("gemini-") {
            args["engine"] = Value::String("antigravity_local_rpc".to_owned());
        }
    }
    Ok(args)
}

fn node_target_context(
    workspace: &Path,
    node: &DagNode,
    pre_manifest: &WorkspaceManifest,
    budget: usize,
) -> Result<String> {
    if node.write_files.is_empty() && node.read_files.is_empty() {
        return Ok(node.title.clone());
    }
    let root = workspace.canonicalize()?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "DAG Lane `{}` ({:?}) · {}",
        node.id, node.role, node.title
    );
    if !node.write_files.is_empty() {
        let _ = writeln!(
            out,
            "Write-lane contract: each edit's new_content replaces the ENTIRE file. Blocks marked COMPLETE contain every line; preserve all lines and tests you do not intend to change."
        );
    }
    let mut seen = BTreeSet::new();
    for raw in &node.write_files {
        let rel = normalize_relative(raw)?;
        if !seen.insert(rel.clone()) {
            continue;
        }
        let block = file_context_block(&root, &rel, "write", Some(pre_manifest))?;
        if out.len().saturating_add(block.len()) > budget {
            return Err(format!(
                "write_file `{}` requires {} context bytes which exceeds node budget {budget}; refusing truncated full-file context",
                rel.display(),
                block.len()
            )
            .into());
        }
        out.push_str(&block);
    }
    for raw in &node.read_files {
        let rel = normalize_relative(raw)?;
        if !seen.insert(rel.clone()) {
            continue;
        }
        let block = file_context_block(&root, &rel, "read", None)?;
        let remaining = budget.saturating_sub(out.len());
        if block.len() <= remaining {
            out.push_str(&block);
        } else {
            let _ = writeln!(
                out,
                "[read_file `{}` omitted from inline target context: {} bytes exceeds remaining budget {remaining}]",
                rel.display(),
                block.len()
            );
        }
    }
    Ok(out)
}

fn file_context_block(
    root: &Path,
    rel: &Path,
    role: &str,
    pre_manifest: Option<&WorkspaceManifest>,
) -> Result<String> {
    let absolute = validate_destination(root, rel)?;
    let metadata = std::fs::symlink_metadata(&absolute).ok();
    if metadata
        .as_ref()
        .is_some_and(|meta| meta.file_type().is_symlink())
    {
        return Err(format!("context path is a symlink: {}", rel.display()).into());
    }
    let bytes = match std::fs::read(&absolute) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(pre) = pre_manifest
                && pre.files.contains_key(rel)
            {
                return Err(format!(
                    "preimage drift for `{}`: file disappeared after manifest capture",
                    rel.display()
                )
                .into());
            }
            return Ok(format!(
                "=== {role} `{}` · ABSENT (new file; preimage_sha256 sha256:absent) ===\n",
                rel.display()
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let sha = format!("sha256:{:x}", Sha256::digest(&bytes));
    if let Some(pre) = pre_manifest
        && pre.files.get(rel) != Some(&sha)
    {
        return Err(format!(
            "preimage drift for `{}` between manifest capture and context read",
            rel.display()
        )
        .into());
    }
    let byte_len = bytes.len();
    let text = String::from_utf8(bytes).map_err(|_| {
        format!(
            "`{}` is not valid UTF-8; refusing text context",
            rel.display()
        )
    })?;
    let lines = text.lines().count();
    let trailing_newline = text.ends_with('\n');
    let fence = "`".repeat(longest_backtick_run(&text).max(2) + 1);
    let mut block = String::with_capacity(byte_len.saturating_add(256));
    let _ = writeln!(
        block,
        "=== {role} `{}` · {byte_len} bytes · {lines} lines · trailing_newline={trailing_newline} · preimage_sha256 {sha} · COMPLETE ===",
        rel.display()
    );
    let _ = writeln!(block, "{fence}");
    block.push_str(&text);
    if !trailing_newline {
        block.push('\n');
    }
    let _ = writeln!(block, "{fence}");
    Ok(block)
}

fn longest_backtick_run(text: &str) -> usize {
    let (mut best, mut run) = (0usize, 0usize);
    for ch in text.chars() {
        if ch == '`' {
            run = run.saturating_add(1);
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn dependency_context(handoffs: &[SettlementReceipt]) -> Result<String> {
    if handoffs.is_empty() {
        return Ok(String::new());
    }
    let summaries = handoffs
        .iter()
        .filter(|receipt| receipt.state == NodeState::Settled)
        .map(|receipt| {
            json!({
                "node_id": receipt.node_id,
                "actual_model": receipt.model_id,
                "summary": receipt.output_summary.chars().take(1_000).collect::<String>(),
                "post_manifest_digest": receipt.post_manifest_digest,
                "gate": receipt.gate,
            })
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "\n\nSettled direct dependency summaries (not complete provider artifacts):\n{}",
        serde_json::to_string(&summaries)?,
    ))
}

fn extract_patch_artifact(output: &Value) -> Result<PatchArtifact> {
    let crafted = [
        "/result/structured_output/crafted_code",
        "/structured_output/crafted_code",
        "/result/result/structured_output/crafted_code",
    ]
    .iter()
    .find_map(|pointer| output.pointer(pointer).and_then(Value::as_str))
    .ok_or("Claude response did not include structured crafted_code")?;
    let trimmed = crafted.trim();
    if trimmed.starts_with("```") {
        return Err("crafted_code must be raw JSON without markdown fences".into());
    }
    Ok(serde_json::from_str(trimmed)?)
}

async fn emit(observer: &Option<mpsc::Sender<DagExecutionEvent>>, event: DagExecutionEvent) {
    if let Some(sender) = observer {
        let _ignored_closed_receiver = sender.send(event).await;
    }
}

static CARGO_GATE_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

type GateCacheKey = (PathBuf, String, String);

static GATE_PROOF_CACHE: std::sync::LazyLock<std::sync::Mutex<BTreeMap<GateCacheKey, GateProof>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

async fn run_gate_for_manifest(
    workspace: &Path,
    requested: Option<&str>,
    timeout: Duration,
    manifest_digest: &str,
    node_kind: NodeKind,
) -> Result<GateProof> {
    let command = default_gate_command(workspace, requested);
    let cacheable = node_kind != NodeKind::Reproduction && !manifest_digest.is_empty();
    let cache_key = (
        workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf()),
        manifest_digest.to_owned(),
        command.clone(),
    );
    if cacheable
        && let Ok(guard) = GATE_PROOF_CACHE.lock()
        && let Some(cached) = guard.get(&cache_key).cloned()
    {
        return Ok(cached);
    }
    let Some((program, arguments)) = allowlisted_command(&command) else {
        return Ok(GateProof {
            command,
            executed: false,
            exit_code: None,
            stdout_digest: None,
        });
    };
    let _cargo_guard = if program == "cargo" && workspace.join("Cargo.lock").is_file() {
        Some(CARGO_GATE_LOCK.lock().await)
    } else {
        None
    };
    if cacheable
        && let Ok(guard) = GATE_PROOF_CACHE.lock()
        && let Some(cached) = guard.get(&cache_key).cloned()
    {
        return Ok(cached);
    }
    let mut proof =
        execute_gate_command(workspace, timeout, command.clone(), program, arguments).await?;
    if cacheable
        && !proof.passed()
        && program == "cargo"
        && matches!(
            arguments.first().copied(),
            Some("test" | "check" | "clippy")
        )
        && capture(workspace)
            .await
            .is_ok_and(|after| after.digest == manifest_digest)
    {
        proof = execute_gate_command(workspace, timeout, command, program, arguments).await?;
    }
    if cacheable
        && proof.passed()
        && let Ok(mut guard) = GATE_PROOF_CACHE.lock()
    {
        if guard.len() >= 64 {
            guard.clear();
        }
        guard.insert(cache_key, proof.clone());
    }
    Ok(proof)
}

#[cfg(test)]
async fn run_gate(
    workspace: &Path,
    requested: Option<&str>,
    timeout: Duration,
) -> Result<GateProof> {
    let command = default_gate_command(workspace, requested);
    let Some((program, arguments)) = allowlisted_command(&command) else {
        return Ok(GateProof {
            command,
            executed: false,
            exit_code: None,
            stdout_digest: None,
        });
    };
    let _cargo_guard = if program == "cargo" && workspace.join("Cargo.lock").is_file() {
        Some(CARGO_GATE_LOCK.lock().await)
    } else {
        None
    };
    execute_gate_command(workspace, timeout, command, program, arguments).await
}

fn default_gate_command(workspace: &Path, requested: Option<&str>) -> String {
    requested.map(str::to_owned).unwrap_or_else(|| {
        if workspace.join("Cargo.toml").is_file() {
            "cargo test --workspace --all-targets".to_owned()
        } else if workspace.join("pyproject.toml").is_file() {
            "uv run pytest -q".to_owned()
        } else {
            String::new()
        }
    })
}

async fn execute_gate_command(
    workspace: &Path,
    timeout: Duration,
    command: String,
    program: &str,
    arguments: &[&str],
) -> Result<GateProof> {
    let proof = crate::verification::run(workspace, timeout, program, arguments).await?;
    let completed = proof.outcome == crate::verification::GateOutcome::Completed;
    // Evidence covers both streams without retaining unbounded child output.
    // Hashing the ordered stream digests intentionally replaces raw concatenation.
    let digest = serde_json::to_vec(&[proof.stdout_digest, proof.stderr_digest])?;
    Ok(GateProof {
        command,
        executed: true,
        exit_code: proof.exit_code,
        stdout_digest: completed.then(|| format!("sha256:{:x}", Sha256::digest(digest))),
    })
}

fn allowlisted_command(command: &str) -> Option<(&'static str, &'static [&'static str])> {
    match command.trim() {
        "cargo test" => Some(("cargo", &["test"])),
        "cargo test --workspace --all-targets" => {
            Some(("cargo", &["test", "--workspace", "--all-targets"]))
        }
        "cargo check --workspace --all-targets" => {
            Some(("cargo", &["check", "--workspace", "--all-targets"]))
        }
        "cargo clippy --workspace --all-targets" => {
            Some(("cargo", &["clippy", "--workspace", "--all-targets"]))
        }
        "cargo clippy --all-targets --all-features -- -D warnings" => Some((
            "cargo",
            &[
                "clippy",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        )),
        "uv run pytest -q" => Some(("uv", &["run", "pytest", "-q"])),
        "uv run ruff check app tests" => Some(("uv", &["run", "ruff", "check", "app", "tests"])),
        _ => None,
    }
}

async fn capture(workspace: &Path) -> Result<WorkspaceManifest> {
    let workspace = workspace.to_path_buf();
    tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&workspace)).await?
}

fn blocked_receipt(node: &DagNode, digest: &str, wave_index: usize) -> SettlementReceipt {
    SettlementReceipt {
        node_id: node.id.clone(),
        state: NodeState::BlockedUpstream,
        settlement_stage: SettlementStage::BlockedUpstream,
        role: node.role.clone(),
        node_kind: node.node_kind,
        wave_index,
        model_id: "not_dispatched".to_owned(),
        duration_seconds: 0.0,
        pre_manifest_digest: digest.to_owned(),
        stopped_manifest_digest: digest.to_owned(),
        post_manifest_digest: digest.to_owned(),
        changed_files: Vec::new(),
        mutated_files: Vec::new(),
        out_of_scope_writes: Vec::new(),
        dirty_read_files: Vec::new(),
        verify_passed: false,
        reproduction_observed: false,
        output_summary: "Blocked by unsettled upstream dependency.".to_owned(),
        error: Some("upstream dependency did not settle".to_owned()),
        gate: GateProof {
            command: "not_run: blocked upstream".to_owned(),
            executed: false,
            exit_code: None,
            stdout_digest: None,
        },
    }
}

#[cfg(test)]
mod tests;
