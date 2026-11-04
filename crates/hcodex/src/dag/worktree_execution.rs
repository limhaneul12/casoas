//! Isolated node verification followed by scoped canonical publication.
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc};

use super::execution::{
    DagExecutionEvent, emit, execute_one_with_dispatch, extract_patch_artifact,
    run_gate_for_manifest,
};
use super::patch::normalize_relative;
use super::settlement::SettlementEvidence;
use super::worktrees::WorktreeCampaign;
use super::{
    DagNode, DagSettlementVerifier, NodeKind, NodeRole, NodeState, SettlementReceipt,
    SettlementStage, WorkspaceManifest,
};
use crate::Result;

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_node<F, Fut>(
    node: DagNode,
    original: &Path,
    timeout: Duration,
    max_input_bytes: u64,
    wave_index: usize,
    baseline: &WorkspaceManifest,
    handoffs: &[SettlementReceipt],
    observer: &Option<mpsc::Sender<DagExecutionEvent>>,
    campaign: &WorktreeCampaign,
    expected: &Mutex<WorkspaceManifest>,
    dispatch: F,
) -> Result<(SettlementReceipt, Value)>
where
    F: FnOnce(&'static str, Value) -> Fut + Send,
    Fut: Future<Output = Result<Value>> + Send,
{
    for path in &node.write_files {
        normalize_relative(path)?;
        if path.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some(
                    ".git"
                        | ".agents"
                        | ".codex"
                        | ".omx"
                        | ".hcodex"
                        | ".codegraph"
                        | "target"
                        | "node_modules"
                        | ".venv"
                )
            )
        }) {
            return Err("worktree write scope targets protected metadata/build state".into());
        }
    }
    campaign.mark_preparing(&node.id).await?;
    let path = campaign.prepare_node(&node.id, baseline).await?;
    let isolated_pre = DagSettlementVerifier::capture(&path)?;
    if isolated_pre.files != baseline.files {
        return Err("isolated baseline mismatch".into());
    }
    let provider_id = campaign.provider_request_id(&node.id);
    let dispatch_node = node.id.clone();
    let dispatch = move |name: &'static str, mut args: Value| async move {
        args["request_id"] = json!(provider_id);
        campaign.mark_admitted(&dispatch_node).await?;
        dispatch(name, args).await
    };
    let (sender, mut receiver) = mpsc::channel(32);
    // Do not forward isolated settlement to the durable canonical checkpoint lane.
    // Both futures remain owned, and sender is dropped once execution finishes.
    let operation = async {
        let sink = Some(sender);
        execute_one_with_dispatch(
            node.clone(),
            &path,
            timeout,
            max_input_bytes,
            wave_index,
            &isolated_pre,
            handoffs,
            &sink,
            true,
            dispatch,
        )
        .await
    };
    let forwarding = async {
        while let Some(event) = receiver.recv().await {
            if !matches!(event, DagExecutionEvent::NodeSettled { .. }) {
                emit(observer, event).await;
            }
        }
    };
    let (result, ()) = tokio::join!(operation, forwarding);
    let (mut receipt, mut output) = result?;
    output["worktree_path"] = json!(path);
    let isolated_final = DagSettlementVerifier::capture(&path)?;
    if receipt.state != NodeState::Settled || !terminal_and_closed(&output) {
        if receipt.state == NodeState::Settled {
            reject(
                &mut receipt,
                "provider terminal/owned-process closure is unproven",
            );
        }
        export(
            campaign,
            &node.id,
            "retained",
            &json!({"receipt":receipt,"output":output}),
        )?;
        emit(
            observer,
            DagExecutionEvent::NodeSettled {
                receipt: receipt.clone(),
            },
        )
        .await;
        return Ok((receipt, output));
    }
    campaign.verify_original(baseline).await?;
    let artifact = if node.role == NodeRole::InterfaceBuilder && !node.write_files.is_empty() {
        Some(extract_patch_artifact(&output)?)
    } else {
        None
    };
    let mut expected_after_apply = baseline.files.clone();
    if let Some(artifact) = &artifact {
        for edit in &artifact.edits {
            expected_after_apply.insert(
                normalize_relative(&edit.path)?,
                format!("sha256:{:x}", Sha256::digest(edit.new_content.as_bytes())),
            );
        }
    }
    let mut before = Vec::new();
    for relative in &node.write_files {
        let relative = normalize_relative(relative)?;
        let bytes = if baseline.files.contains_key(&relative) {
            let file = original.join(&relative);
            if fs::metadata(&file)?.len() > 16 * 1024 * 1024 {
                return Err("publication preimage too large".into());
            }
            Some(fs::read(file)?)
        } else {
            None
        };
        before.push(json!({"path":relative,"bytes":bytes}));
    }
    // Export exact original bytes and provider artifact before any canonical effect.
    export(
        campaign,
        &node.id,
        "before-publication",
        &json!({
            "original":original,"baseline":baseline,"before":before,
            "isolated_manifest":isolated_final,"isolated_receipt":receipt,"output":output
        }),
    )?;
    campaign.verify_original(baseline).await?;
    if let Some(artifact) = artifact {
        // The campaign owns canonical publication. No Git merge/reset/commit.
        // A later gate failure retains exact evidence; no blind rollback over
        // potentially newer original bytes is attempted by this mode.
        #[cfg(not(test))]
        let _applied = super::patch::apply_scoped_patch_with_rollback(
            original,
            &node.write_files,
            baseline,
            &artifact,
        )?;
        #[cfg(test)]
        let _applied = super::patch::apply_scoped_patch_observed(
            original,
            &node.write_files,
            baseline,
            &artifact,
            &|phase| {
                let boundary = if phase == "staged" {
                    super::worktrees::TestBoundary::Staged
                } else {
                    super::worktrees::TestBoundary::PartialPublication
                };
                campaign.test_boundary(&node.id, boundary);
            },
        )?;
    }
    #[cfg(test)]
    campaign.test_boundary(&node.id, super::worktrees::TestBoundary::CanonicalPublished);
    let pre_gate = DagSettlementVerifier::capture(original)?;
    if pre_gate.files != expected_after_apply {
        return Err(
            "canonical source drift before verification; inspect retained worktree exports".into(),
        );
    }
    let gate = run_gate_for_manifest(
        original,
        node.verify_command.as_deref(),
        timeout,
        &pre_gate.digest,
        node.node_kind,
        false,
    )
    .await?;
    #[cfg(test)]
    campaign.test_boundary(
        &node.id,
        super::worktrees::TestBoundary::CanonicalGateCompleted,
    );
    let stopped = DagSettlementVerifier::capture(original)?;
    let reproduction_observed = node.node_kind == NodeKind::Reproduction
        && if node.reproduction_expected_exit_nonzero {
            gate.executed
                && gate.stdout_digest.is_some()
                && gate.exit_code.is_some_and(|code| code != 0)
        } else {
            gate.passed()
        };
    let model_id = receipt.model_id.clone();
    receipt = DagSettlementVerifier::settle_with_evidence(
        &node,
        baseline,
        &stopped,
        gate,
        SettlementEvidence {
            wave_index,
            duration_seconds: receipt.duration_seconds,
            reproduction_observed,
            output_summary: receipt.output_summary.clone(),
            ..SettlementEvidence::default()
        },
    );
    receipt.model_id = model_id;
    if stopped.files != isolated_final.files {
        reject(
            &mut receipt,
            "canonical result differs from verified isolated result",
        );
    }
    export(
        campaign,
        &node.id,
        "publication-result",
        &json!({"receipt":receipt,"output":output}),
    )?;
    if receipt.state == NodeState::Settled {
        campaign.mark_verified(&node.id, &isolated_final).await?;
        *expected.lock().await = stopped;
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

fn reject(receipt: &mut SettlementReceipt, message: &str) {
    receipt.state = NodeState::FailedExecution;
    receipt.settlement_stage = SettlementStage::FailedExecution;
    receipt.verify_passed = false;
    receipt.error = Some(message.to_owned());
}

fn terminal_and_closed(output: &Value) -> bool {
    ["/result/receipt", "/receipt", "/result/result/receipt"]
        .iter()
        .any(|pointer| {
            let Some(r) = output.pointer(pointer) else {
                return false;
            };
            let identity = r["actual_model"].as_str().is_some()
                && r["actual_model"] == r["requested_model"]
                && r["model_mismatch"] == false;
            identity
                && r["status"] == "succeeded"
                && r["outcome_state"] == "confirmed_success"
                && match r["engine"].as_str() {
                    Some("codex_cli") => r["owned_processes_closed"] == true,
                    Some("antigravity_rpc" | "antigravity_local_rpc" | "http_api") => true,
                    _ => false,
                }
        })
}

fn export(campaign: &WorktreeCampaign, node: &str, phase: &str, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("worktree export exceeds byte limit".into());
    }
    let name = format!("node-{:x}-{phase}.json", Sha256::digest(node.as_bytes()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(campaign.state_path().join(name))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::open(campaign.state_path())?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
