use std::fs;

use tempfile::tempdir;

use super::*;
use crate::dag::{NodeKind, NodeRole};

fn test_node() -> DagNode {
    DagNode {
        id: "writer".to_owned(),
        title: "writer".to_owned(),
        task: "write allowed".to_owned(),
        role: NodeRole::InterfaceBuilder,
        node_kind: NodeKind::Standard,
        model: "claude-sonnet-5-5-medium".to_owned(),
        reasoning_effort: "medium".to_owned(),
        dependencies: Vec::new(),
        read_files: Vec::new(),
        write_files: vec![PathBuf::from("allowed.rs")],
        symbols: Vec::new(),
        verify_command: Some("cargo test".to_owned()),
        reproduction_expected_exit_nonzero: true,
    }
}

fn passed_gate() -> GateProof {
    GateProof {
        command: "cargo test".to_owned(),
        executed: true,
        exit_code: Some(0),
        stdout_digest: Some("sha256:test".to_owned()),
    }
}

#[test]
fn requires_scope_and_executed_gate_to_settle() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("allowed.rs"), "one")?;
    fs::write(workspace.path().join("forbidden.rs"), "one")?;
    let pre = DagSettlementVerifier::capture(workspace.path())?;
    fs::write(workspace.path().join("forbidden.rs"), "two")?;
    let stopped = DagSettlementVerifier::capture(workspace.path())?;
    let rejected = DagSettlementVerifier::settle(&test_node(), &pre, &stopped, passed_gate());
    assert_eq!(rejected.state, NodeState::RejectedOutOfScopeWrite);
    assert_eq!(
        rejected.out_of_scope_writes,
        [PathBuf::from("forbidden.rs")]
    );

    fs::write(workspace.path().join("forbidden.rs"), "one")?;
    fs::write(workspace.path().join("allowed.rs"), "two")?;
    let stopped = DagSettlementVerifier::capture(workspace.path())?;
    let settled = DagSettlementVerifier::settle(&test_node(), &pre, &stopped, passed_gate());
    assert_eq!(settled.state, NodeState::Settled);
    Ok(())
}

#[test]
fn stopped_without_gate_proof_is_not_settled() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("allowed.rs"), "one")?;
    let pre = DagSettlementVerifier::capture(workspace.path())?;
    fs::write(workspace.path().join("allowed.rs"), "two")?;
    let stopped = DagSettlementVerifier::capture(workspace.path())?;
    let receipt = DagSettlementVerifier::settle(
        &test_node(),
        &pre,
        &stopped,
        GateProof {
            command: "cargo test".to_owned(),
            executed: false,
            exit_code: None,
            stdout_digest: None,
        },
    );
    assert_eq!(receipt.state, NodeState::RejectedVerification);
    Ok(())
}

#[test]
fn write_lane_with_text_only_success_is_not_accepted() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("allowed.rs"), "unchanged")?;
    let pre = DagSettlementVerifier::capture(workspace.path())?;
    let stopped = DagSettlementVerifier::capture(workspace.path())?;
    let receipt = DagSettlementVerifier::settle(&test_node(), &pre, &stopped, passed_gate());
    assert_eq!(receipt.state, NodeState::RejectedVerification);
    assert!(receipt.changed_files.is_empty());
    Ok(())
}

#[test]
fn reproduction_requires_expected_nonzero_gate_evidence() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("observed.rs"), "bug")?;
    let manifest = DagSettlementVerifier::capture(workspace.path())?;
    let mut node = test_node();
    node.node_kind = NodeKind::Reproduction;
    node.write_files.clear();
    let failed_gate = GateProof {
        command: "cargo test".to_owned(),
        executed: true,
        exit_code: Some(101),
        stdout_digest: Some("sha256:failure-evidence".to_owned()),
    };
    let accepted = DagSettlementVerifier::settle_with_evidence(
        &node,
        &manifest,
        &manifest,
        failed_gate,
        SettlementEvidence {
            reproduction_observed: true,
            output_summary: "failure reproduced".to_owned(),
            ..SettlementEvidence::default()
        },
    );
    assert_eq!(accepted.state, NodeState::Settled);
    assert!(!accepted.verify_passed);
    assert!(accepted.reproduction_observed);

    let not_observed = DagSettlementVerifier::settle_with_evidence(
        &node,
        &manifest,
        &manifest,
        passed_gate(),
        SettlementEvidence::default(),
    );
    assert_eq!(
        not_observed.settlement_stage,
        SettlementStage::RejectedReproductionNotObserved
    );
    Ok(())
}

#[test]
fn dirty_read_evidence_rejects_settlement_before_verification() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("read.rs"), "baseline")?;
    let manifest = DagSettlementVerifier::capture(workspace.path())?;
    let mut node = test_node();
    node.write_files.clear();
    node.read_files = vec![PathBuf::from("read.rs")];
    let receipt = DagSettlementVerifier::settle_with_evidence(
        &node,
        &manifest,
        &manifest,
        passed_gate(),
        SettlementEvidence {
            dirty_read_files: vec![PathBuf::from("read.rs")],
            reproduction_observed: true,
            ..SettlementEvidence::default()
        },
    );
    assert_eq!(receipt.state, NodeState::RejectedDirtyRead);
    assert_eq!(receipt.dirty_read_files, [PathBuf::from("read.rs")]);
    Ok(())
}
