use std::collections::BTreeSet;
use std::future::Future;
use std::pin::{Pin, pin};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use serde_json::json;
use tempfile::tempdir;

use super::*;
use crate::dag::handoff::{AgentReport, NodeHandoff};
use crate::dag::{
    DagPlanner, GateProof, NodeKind, NodeRole, NodeState, SettlementReceipt, SettlementStage,
};

fn digest(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn node(id: &str, dependencies: &[&str]) -> DagNode {
    DagNode {
        id: id.to_owned(),
        title: id.to_owned(),
        task: format!("task-{id}"),
        role: NodeRole::ContractVerifier,
        node_kind: NodeKind::Standard,
        model: "gpt-6.1-sol".to_owned(),
        reasoning_effort: "high".to_owned(),
        dependencies: dependencies
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        read_files: Vec::new(),
        write_files: Vec::new(),
        symbols: Vec::new(),
        verify_command: Some("cargo test".to_owned()),
        reproduction_expected_exit_nonzero: true,
    }
}

fn plan() -> crate::Result<DagPlan> {
    DagPlanner::compile(
        "checkpoint handoff",
        vec![
            node("one", &[]),
            node("two", &["one"]),
            node("three", &["two"]),
            node("unrelated", &[]),
        ],
    )
}

fn receipt(node_id: &str, summary: &str, wave_index: usize) -> SettlementReceipt {
    SettlementReceipt {
        node_id: node_id.to_owned(),
        state: NodeState::Settled,
        settlement_stage: SettlementStage::Settled,
        role: NodeRole::ContractVerifier,
        node_kind: NodeKind::Standard,
        wave_index,
        model_id: "gpt-6.1-sol".to_owned(),
        duration_seconds: 1.0,
        pre_manifest_digest: digest(1),
        stopped_manifest_digest: digest(2),
        post_manifest_digest: digest(2),
        changed_files: Vec::new(),
        mutated_files: Vec::new(),
        out_of_scope_writes: Vec::new(),
        dirty_read_files: Vec::new(),
        verify_passed: true,
        reproduction_observed: false,
        output_summary: summary.to_owned(),
        error: None,
        gate: GateProof {
            command: "cargo test".to_owned(),
            executed: true,
            exit_code: Some(0),
            stdout_digest: Some(digest(3)),
        },
    }
}

fn report(summary: &str, decision: &str) -> AgentReport {
    AgentReport {
        summary: summary.to_owned(),
        decisions: vec![decision.to_owned()],
        caveats: Vec::new(),
        truncated: false,
    }
}

fn context() -> crate::Result<(tempfile::TempDir, tempfile::TempDir, Arc<CheckpointContext>)> {
    let state = tempdir()?;
    let workspace = tempdir()?;
    let gateway = Gateway::new(state.path().to_path_buf());
    let context = CheckpointContext::new(
        gateway,
        workspace.path().canonicalize()?,
        "campaign".to_owned(),
        digest(4),
        digest(5),
        &plan()?,
        None,
        BTreeSet::new(),
    )?;
    Ok((state, workspace, context))
}

#[test]
fn context_rejects_an_invalid_node_identity_before_execution() -> crate::Result<()> {
    let state = tempdir()?;
    let workspace = tempdir()?;
    let mut plan = plan()?;
    plan.nodes[0].id = "bad/node".to_owned();

    let result = CheckpointContext::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().canonicalize()?,
        "campaign".to_owned(),
        digest(4),
        digest(5),
        &plan,
        None,
        BTreeSet::new(),
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn context_rejects_more_direct_dependencies_than_a_handoff_can_record() -> crate::Result<()> {
    let state = tempdir()?;
    let workspace = tempdir()?;
    let mut plan = plan()?;
    plan.nodes[2].dependencies = (0..9).map(|index| format!("parent-{index}")).collect();

    let result = CheckpointContext::new(
        Gateway::new(state.path().to_path_buf()),
        workspace.path().canonicalize()?,
        "campaign".to_owned(),
        digest(4),
        digest(5),
        &plan,
        None,
        BTreeSet::new(),
    );

    assert!(result.is_err());
    Ok(())
}

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    future.poll(&mut Context::from_waker(waker))
}

#[tokio::test]
async fn wait_remains_pending_until_checkpoint_is_committed() -> crate::Result<()> {
    let (_state, _workspace, context) = context()?;
    let mut waiting = pin!(context.wait("one", Duration::from_secs(10)));
    assert!(poll(waiting.as_mut()).is_pending());

    context.committed("one")?;

    assert!(matches!(poll(waiting.as_mut()), Poll::Ready(Ok(()))));
    Ok(())
}

#[tokio::test]
async fn failure_releases_every_pending_checkpoint_waiter() -> crate::Result<()> {
    let (_state, _workspace, context) = context()?;
    let mut first = pin!(context.wait("one", Duration::from_secs(10)));
    let mut second = pin!(context.wait("two", Duration::from_secs(10)));
    assert!(poll(first.as_mut()).is_pending());
    assert!(poll(second.as_mut()).is_pending());

    context.fail_pending("checkpoint write failed");

    for outcome in [poll(first.as_mut()), poll(second.as_mut())] {
        let Poll::Ready(Err(error)) = outcome else {
            return Err("pending checkpoint waiter was not released".into());
        };
        assert!(error.to_string().contains("checkpoint write failed"));
    }
    Ok(())
}

#[tokio::test]
async fn inherit_resolves_direct_parent_and_transitive_ancestor() -> crate::Result<()> {
    let (_state, workspace, context) = context()?;
    std::fs::write(workspace.path().join("sentinel"), "unchanged")?;
    context
        .persist(
            receipt("one", "first summary", 0),
            report("first summary", "preserve-api"),
        )
        .await?;
    context
        .persist(
            receipt("two", "second summary", 1),
            report("second summary", "reuse-first"),
        )
        .await?;

    let inherited = context
        .inherit(&node("three", &["two"]), Duration::from_secs(10))
        .await?;

    assert!(inherited.contains("first summary"));
    assert!(inherited.contains("preserve-api"));
    assert!(inherited.contains("second summary"));
    assert!(inherited.contains("reuse-first"));
    assert!(!inherited.contains("unrelated"));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("sentinel"))?,
        "unchanged"
    );
    Ok(())
}

#[tokio::test]
async fn inherit_rejects_a_tampered_ancestor_reference() -> crate::Result<()> {
    let (_state, _workspace, context) = context()?;
    context
        .persist(
            receipt("one", "first summary", 0),
            report("first summary", "preserve-api"),
        )
        .await?;

    let one = context
        .scope()
        .load_node(&Store::open(&context.gateway.state_root)?, "one")?;
    let mut wrong = one.reference();
    wrong.content_digest = digest(9);
    let two_receipt = receipt("two", "second summary", 1);
    let malicious = NodeHandoff::create_with_report(
        "campaign",
        &digest(4),
        &digest(5),
        &two_receipt,
        vec![wrong],
        report("second summary", "reuse-first"),
    )?;
    let checkpoint_id = "ckp-campaign-two".to_owned();
    let checkpoint_workspace = context.workspace.clone();
    let mut metadata = super::super::request::checkpoint_metadata(
        "campaign",
        &digest(4),
        &digest(5),
        &two_receipt.stopped_manifest_digest,
        "node",
    );
    metadata.insert("node_id".to_owned(), "two".to_owned());
    context
        .gateway
        .database(move |store| {
            store.complete_checkpoint_once(
                NewJob {
                    task_type: "dag_checkpoint".to_owned(),
                    workspace_dir: checkpoint_workspace,
                    model: Some("dag-node".to_owned()),
                    metadata,
                    job_id: Some(checkpoint_id.clone()),
                    idempotency_key: Some(checkpoint_id),
                    provider: Some("heterarchy_triad".to_owned()),
                    engine: Some("dag_ulw".to_owned()),
                    created_at: None,
                },
                json!({"receipt":two_receipt,"handoff":malicious}),
            )?;
            Ok(())
        })
        .await?;
    context.committed("two")?;

    let error = context
        .inherit(&node("three", &["two"]), Duration::from_secs(10))
        .await
        .err()
        .ok_or("tampered ancestor was accepted")?;
    assert!(
        error
            .to_string()
            .contains("ancestor reference digest mismatch")
    );
    Ok(())
}
