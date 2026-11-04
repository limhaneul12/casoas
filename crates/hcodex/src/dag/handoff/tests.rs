use super::*;
use serde_json::json;

fn digest(byte: u8) -> String {
    let pair = format!("{byte:02x}");
    format!("sha256:{}", pair.repeat(32))
}

fn settled_receipt() -> SettlementReceipt {
    SettlementReceipt {
        node_id: "writer".into(),
        state: NodeState::Settled,
        settlement_stage: SettlementStage::Settled,
        role: super::super::NodeRole::InterfaceBuilder,
        node_kind: super::super::NodeKind::Standard,
        wave_index: 0,
        model_id: "claude-sonnet-5-5-medium".into(),
        duration_seconds: 1.5,
        pre_manifest_digest: digest(1),
        stopped_manifest_digest: digest(2),
        post_manifest_digest: digest(2),
        changed_files: vec!["src/lib.rs".into()],
        mutated_files: vec!["src/lib.rs".into()],
        out_of_scope_writes: Vec::new(),
        dirty_read_files: Vec::new(),
        verify_passed: true,
        reproduction_observed: true,
        output_summary: "Implemented the verified writer boundary.".into(),
        error: None,
        gate: GateProof {
            command: "cargo test".into(),
            executed: true,
            exit_code: Some(0),
            stdout_digest: Some(digest(3)),
        },
    }
}

fn parent() -> HandoffRef {
    HandoffRef {
        campaign_id: "campaign".into(),
        node_id: "planner".into(),
        checkpoint_id: "ckp-campaign-planner".into(),
        content_digest: digest(4),
    }
}

#[test]
fn stable_record_separates_machine_facts_from_agent_report() -> crate::Result<()> {
    let output = json!({"result":{"data":{
        "design_highlights":["Keep the public boundary stable."],
        "edge_cases_guarded":["Legacy input remains readable."],
        "handoff_notes_for_codex_audit":"Verify the negative fixture."
    }, "structured_output":{"summary":"Provider-selected summary."},
    "actual_model":"fabricated-model", "post_manifest_digest":digest(9)}});
    let captured = AgentReport::from_output("", Some(&output));
    assert_eq!(captured.summary, "Provider-selected summary.");
    let first = NodeHandoff::create(
        "campaign",
        &digest(5),
        &digest(6),
        &settled_receipt(),
        vec![parent()],
        Some(&output),
    )?;
    let second = NodeHandoff::create(
        "campaign",
        &digest(5),
        &digest(6),
        &settled_receipt(),
        vec![parent()],
        Some(&output),
    )?;

    assert_eq!(first, second);
    assert_eq!(first.system_facts.actual_model, "claude-sonnet-5-5-medium");
    assert_eq!(first.system_facts.post_manifest_digest, digest(2));
    assert_eq!(
        first.agent_report.decisions,
        ["Keep the public boundary stable."]
    );
    assert_eq!(first.reference().content_digest, first.content_digest);
    first.verify()
}

#[test]
fn content_digest_detects_body_changes_and_unknown_fields() -> crate::Result<()> {
    let handoff = NodeHandoff::create(
        "campaign",
        &digest(5),
        &digest(6),
        &settled_receipt(),
        Vec::new(),
        None,
    )?;
    let mut changed = handoff.clone();
    changed.agent_report.summary.push_str(" changed");
    assert!(changed.verify().is_err());

    let mut encoded = serde_json::to_value(&handoff)?;
    encoded["unknown"] = json!(true);
    assert!(serde_json::from_value::<NodeHandoff>(encoded).is_err());
    Ok(())
}

#[test]
fn agent_report_is_utf8_safe_and_byte_bounded() -> crate::Result<()> {
    let mut receipt = settled_receipt();
    receipt.output_summary = "한".repeat(2_000);
    let output = json!({"data":{
        "design_highlights":["결정".repeat(1_000)],
        "edge_cases_guarded":["주의".repeat(1_000)]
    }});
    let handoff = NodeHandoff::create(
        "campaign",
        &digest(5),
        &digest(6),
        &receipt,
        Vec::new(),
        Some(&output),
    )?;

    assert!(handoff.agent_report.truncated);
    assert!(handoff.agent_report.content_bytes() <= MAX_AGENT_REPORT_BYTES);
    assert!(
        handoff
            .agent_report
            .summary
            .is_char_boundary(handoff.agent_report.summary.len())
    );
    handoff.verify()?;

    let items = (0..40)
        .map(|index| format!("decision-{index}"))
        .collect::<Vec<_>>();
    let report = AgentReport::from_output("", Some(&json!({"data":{"design_highlights":items}})));
    assert_eq!(report.decisions.len(), MAX_AGENT_REPORT_ITEMS);
    assert!(report.truncated);
    let report = report.with_summary(&"검증 요약".repeat(1_000));
    assert!(report.content_bytes() <= MAX_AGENT_REPORT_BYTES);
    assert!(report.truncated);
    Ok(())
}

#[test]
fn accepts_cross_campaign_resume_refs_and_rejects_malformed_parents() -> crate::Result<()> {
    let receipt = settled_receipt();
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &receipt,
            vec![parent(), parent()],
            None,
        )
        .is_err()
    );

    let mut foreign = parent();
    foreign.campaign_id = "other".into();
    foreign.checkpoint_id = "ckp-other-planner".into();
    let resumed = NodeHandoff::create(
        "campaign",
        &digest(5),
        &digest(6),
        &receipt,
        vec![foreign],
        None,
    )?;
    assert_eq!(resumed.parents[0].campaign_id, "other");

    let mut self_ref = parent();
    self_ref.campaign_id = "prior".into();
    self_ref.node_id = receipt.node_id.clone();
    self_ref.checkpoint_id = "ckp-prior-writer".into();
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &receipt,
            vec![self_ref],
            None,
        )
        .is_err()
    );

    let mut malformed = parent();
    malformed.checkpoint_id = "wrong".into();
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &receipt,
            vec![malformed],
            None,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn requires_a_settled_receipt_with_terminal_evidence() {
    let mut receipt = settled_receipt();
    receipt.state = NodeState::FailedExecution;
    receipt.settlement_stage = SettlementStage::FailedExecution;
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &receipt,
            Vec::new(),
            None,
        )
        .is_err()
    );

    let mut receipt = settled_receipt();
    receipt.gate.stdout_digest = None;
    receipt.verify_passed = false;
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &receipt,
            Vec::new(),
            None,
        )
        .is_err()
    );

    let mut forged = settled_receipt();
    forged.gate.exit_code = Some(1);
    forged.verify_passed = false;
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &forged,
            Vec::new(),
            None,
        )
        .is_err()
    );

    forged.node_kind = super::super::NodeKind::Reproduction;
    assert!(
        NodeHandoff::create(
            "campaign",
            &digest(5),
            &digest(6),
            &forged,
            Vec::new(),
            None,
        )
        .is_ok()
    );
}

#[test]
fn plan_preflight_rejects_late_handoff_contract_failures() -> crate::Result<()> {
    validate_plan_node("campaign-or-node", MAX_PARENT_REFS)?;
    assert!(validate_plan_node("unsafe/node", 0).is_err());
    assert!(validate_plan_node("node", MAX_PARENT_REFS + 1).is_err());
    Ok(())
}
