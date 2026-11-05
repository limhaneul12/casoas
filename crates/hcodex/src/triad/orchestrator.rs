use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::gateway::{
    Gateway, optional, text, triad_mode, triad_model_set, triad_request_identity, workspace,
};
use crate::storage::{HandoffPriority, HandoffStatus, JobStatus, NewHandoff, NewJob, TriadRole};

use super::adjudication::{REQUIRED_ROLES, adjudicate, canonical_digest, inspect_role};
use super::gates;
use super::types::{RoleSnapshot, RoleState, Verdict};

pub async fn orchestrate(gateway: &Gateway, args: &Value) -> crate::Result<Value> {
    let task_digest = source_task_digest(args)?;
    let workspace_path = gates::resolve_workspace(workspace(args))?;
    if !workspace_path.is_dir() {
        return Err("Workspace directory must resolve to an existing directory.".into());
    }
    let mut scope = evidence_scope(&workspace_path, args)?;
    let aggregate_start = aggregate_start(gateway, args, &workspace_path, &task_digest).await?;
    if let AggregateStart::Deduplicated(result) = aggregate_start {
        return Ok(result);
    }
    let AggregateStart::Reserved(aggregate_id) = &aggregate_start else {
        return Err("triad aggregate reservation was not retained".into());
    };
    let aggregate_id = aggregate_id.clone();
    let resume_id = optional(args, "resume_collaboration_id");
    let mut resumed = if let Some(identifier) = resume_id.as_deref() {
        match load_resumable_roles(gateway, identifier, &workspace_path, &task_digest, &scope).await
        {
            Ok(resumed) => resumed,
            Err(error) => {
                fail_reserved_aggregate(gateway, &aggregate_id, &error).await?;
                return Err(error);
            }
        }
    } else {
        BTreeMap::new()
    };
    let resumed_roles = resumed.keys().cloned().collect::<Vec<_>>();
    let mode = normalized_mode(args);
    let role_execution = if mode == "council" {
        execute_council(gateway, args, &mut resumed).await
    } else {
        execute_pipeline(gateway, args, &mut resumed).await
    };
    let (sonnet, codex, argon, repair_rounds) = match role_execution {
        Ok(execution) => execution,
        Err(error) => {
            fail_reserved_aggregate(gateway, &aggregate_id, &error).await?;
            return Err(error);
        }
    };
    let argon_review = argon.clone();
    let sonnet_craft = sonnet.clone();
    let codex_audit = codex.clone();
    let roles = BTreeMap::from([
        ("argon".to_owned(), inspect_role("argon", argon)),
        ("sonnet".to_owned(), inspect_role("sonnet", sonnet)),
        ("codex".to_owned(), inspect_role("codex", codex)),
    ]);
    let codex_verdict = roles.get("codex").and_then(|snapshot| snapshot.verdict);
    let adjudication = adjudicate(&roles);
    let gates = if args["run_verification_gates"].as_bool().unwrap_or(false) {
        match gates::run(&workspace_path, timeout_duration(args)).await {
            Ok(receipts) => Some(receipts),
            Err(error) => {
                fail_reserved_aggregate(gateway, &aggregate_id, &error).await?;
                return Err(error);
            }
        }
    } else {
        None
    };
    if let Some(receipts) = gates.as_ref() {
        scope["test_execution_evidence_origin"] = Value::String("bridge_executed".to_owned());
        scope["evidence_dimensions"]["test_execution"] = Value::String(
            if receipts.iter().all(|receipt| receipt.passed) {
                "verified"
            } else {
                "executed_fail"
            }
            .to_owned(),
        );
    }
    let verification_passed = gates
        .as_ref()
        .is_none_or(|receipts| receipts.iter().all(|receipt| receipt.passed));
    let role_evidence_dimensions = roles
        .iter()
        .map(|(role, snapshot)| {
            let static_review = if snapshot.state == RoleState::Succeeded
                && snapshot.review_artifact_has_narrative
                && snapshot.verdict.is_some()
            {
                "verified"
            } else {
                "not_run"
            };
            (
                role.clone(),
                json!({
                    "static_code_review": static_review,
                    "test_generation": if generated_tests(snapshot) {"verified"} else {"not_run"},
                    "test_execution": "not_run",
                    "integration_verification": "not_run",
                    "runtime_confirmation": "not_run",
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let verified_reviews = role_evidence_dimensions
        .values()
        .filter(|dimensions| dimensions["static_code_review"] == "verified")
        .count();
    scope["evidence_dimensions"]["static_code_review"] = Value::String(
        if verified_reviews == REQUIRED_ROLES.len() {
            "verified"
        } else if verified_reviews > 0 {
            "partial"
        } else {
            "not_run"
        }
        .to_owned(),
    );
    scope["evidence_dimensions"]["test_generation"] = Value::String(
        if role_evidence_dimensions
            .values()
            .any(|dimensions| dimensions["test_generation"] == "verified")
        {
            "verified"
        } else {
            "not_run"
        }
        .to_owned(),
    );
    let participants = json!({
        "macro_architect": text(args,"argon_model"),
        "code_craftsman": text(args,"claude_model"),
        "red_team_auditor": optional(args,"codex_model").unwrap_or_else(||"gpt-6-astra".to_owned()),
    });
    let compact_roles = roles
        .iter()
        .map(|(role, snapshot)| (role.clone(), snapshot.compact()))
        .collect::<BTreeMap<_, _>>();
    let mut result = json!({
        "schema_version": 2,
        "ok": adjudication.collaboration_status == "complete" && adjudication.consensus_reached && verification_passed,
        "collaboration_status": adjudication.collaboration_status,
        "consensus_status": adjudication.consensus_status,
        "consensus_reached": adjudication.consensus_reached,
        "triad_consensus_ready": adjudication.consensus_reached,
        "review_approved": adjudication.review_approved,
        "orchestration_mode": mode,
        "workspace_dir": workspace_path,
        "source_task_digest": task_digest,
        "participants": participants,
        "adjudication": adjudication,
        "argon_review": argon_review,
        "sonnet_craft": sonnet_craft,
        "codex_audit": codex_audit,
        "codex_verdict": codex_verdict,
        "roles": compact_roles,
        "role_evidence_dimensions": role_evidence_dimensions,
        "evidence_scope": scope,
        "bridge_executed_gates": gates,
        "repair_rounds_executed": repair_rounds,
        "resumed_roles": resumed_roles,
        "resumed_from_collaboration_id": resume_id,
    });
    persist_aggregate(gateway, args, &task_digest, &mut result, aggregate_start).await;
    Ok(result)
}

async fn fail_reserved_aggregate(
    gateway: &Gateway,
    aggregate_id: &str,
    error: &crate::Error,
) -> crate::Result<()> {
    let job_id = aggregate_id.to_owned();
    let message = error.to_string();
    gateway
        .database(move |store| {
            store
                .fail_job(&job_id, message, None)?
                .ok_or("reserved triad aggregate disappeared before failure persistence")?;
            Ok(())
        })
        .await
}

enum AggregateStart {
    Reserved(String),
    Deduplicated(Value),
}

async fn aggregate_start(
    gateway: &Gateway,
    args: &Value,
    workspace: &Path,
    task_digest: &str,
) -> crate::Result<AggregateStart> {
    let retry = args["retry_same_request"].as_bool().unwrap_or(false);
    let background_reserved = args["_background_reserved"].as_bool().unwrap_or(false);
    let mut metadata = BTreeMap::from([
        ("current_stage".to_owned(), "running".to_owned()),
        ("schema_version".to_owned(), "2".to_owned()),
        ("source_task_digest".to_owned(), task_digest.to_owned()),
    ]);
    metadata.insert("request_identity".to_owned(), triad_request_identity(args)?);
    let request = NewJob {
        task_type: "triad_orchestration".to_owned(),
        workspace_dir: workspace.to_path_buf(),
        model: Some(triad_model_set(args)),
        metadata,
        job_id: optional(args, "request_id"),
        idempotency_key: optional(args, "idempotency_key"),
        provider: Some("heterarchy_triad".to_owned()),
        engine: Some(triad_mode(args)),
        created_at: None,
    };
    let registration = gateway
        .database(move |store| store.resolve_or_register(request, retry))
        .await?;
    if registration.should_dispatch {
        return Ok(AggregateStart::Reserved(registration.record.job_id));
    }
    let record = registration.record;
    if background_reserved && record.status == JobStatus::Running {
        return Ok(AggregateStart::Reserved(record.job_id));
    }
    if record.status == JobStatus::Running {
        return Err("triad collaboration request is already running".into());
    }
    record.result.map_or_else(
        || Err("triad collaboration request is terminal without a durable result".into()),
        |result| Ok(AggregateStart::Deduplicated(result)),
    )
}

async fn persist_aggregate(
    gateway: &Gateway,
    args: &Value,
    task_digest: &str,
    result: &mut Value,
    start: AggregateStart,
) {
    let reserved_id = match start {
        AggregateStart::Reserved(identifier) => identifier,
        AggregateStart::Deduplicated(_) => return,
    };
    let lookup = reserved_id.clone();
    let record = match gateway.database(move |store| store.get_job(&lookup)).await {
        Ok(Some(record)) => record,
        _ => {
            mark_persistence_failure(result, Some(reserved_id));
            return;
        }
    };
    let collaboration_id = record.job_id;
    let compact_snapshot = compact_snapshot(result, &collaboration_id);
    let Ok(mut snapshot) = compact_snapshot else {
        mark_persistence_failure(result, Some(collaboration_id));
        return;
    };
    let Ok(snapshot_digest) = canonical_digest(&snapshot) else {
        mark_persistence_failure(result, Some(collaboration_id));
        return;
    };
    snapshot["snapshot_digest"] = Value::String(snapshot_digest.clone());
    let stored = snapshot;
    let stored_id = collaboration_id.clone();
    let completed_model = Some(format!(
        "{}+{}+{}",
        text(args, "argon_model"),
        text(args, "claude_model"),
        optional(args, "codex_model").unwrap_or_else(|| "gpt-6-astra".to_owned())
    ));
    let completed_metadata = BTreeMap::from([
        ("current_stage".to_owned(), "completed".to_owned()),
        ("schema_version".to_owned(), "2".to_owned()),
        ("source_task_digest".to_owned(), task_digest.to_owned()),
    ]);
    match gateway
        .database(move |store| {
            store.complete_job_with_details(
                &stored_id,
                stored,
                None,
                None,
                completed_model,
                completed_metadata,
            )
        })
        .await
    {
        Ok(Some(record)) if record.status == JobStatus::Completed => {
            result["collaboration_id"] = Value::String(collaboration_id);
            result["collaboration_persistence"] = Value::String("recorded".to_owned());
            result["snapshot_digest"] = Value::String(snapshot_digest);
        }
        _ => mark_persistence_failure(result, Some(collaboration_id)),
    }
}

fn compact_snapshot(result: &Value, collaboration_id: &str) -> crate::Result<Value> {
    Ok(json!({
        "schema_version":2,
        "task_type":"triad_orchestration",
        "collaboration_id":collaboration_id,
        "orchestration_mode":result["orchestration_mode"],
        "source_task_digest":result["source_task_digest"],
        "workspace_dir":result["workspace_dir"],
        "evidence_scope":compact_scope(&result["evidence_scope"]),
        "roles":result["roles"],
        "role_evidence_dimensions":result["role_evidence_dimensions"],
        "adjudication":result["adjudication"],
    }))
}

fn compact_scope(scope: &Value) -> Value {
    json!({
        "workspace_dir":scope["workspace_dir"],
        "codegraph":scope["codegraph"],
        "scope_digest":scope["scope_digest"],
        "inline_context_bytes":scope["inline_context_bytes"],
        "inline_context_digest":scope["inline_context_digest"],
        "files_inspected_digests":scope["files_inspected_digests"],
        "unverified_or_missing_files":scope["unverified_or_missing_files"],
        "symbols_inspected":scope["symbols_inspected"],
        "evidence_dimensions":scope["evidence_dimensions"],
        "test_execution_evidence_origin":scope["test_execution_evidence_origin"],
    })
}

fn mark_persistence_failure(result: &mut Value, unconfirmed_id: Option<String>) {
    if let Some(object) = result.as_object_mut() {
        object.remove("collaboration_id");
        object.remove("snapshot_digest");
    }
    result["collaboration_persistence"] = Value::String("failed".to_owned());
    result["collaboration_persistence_error"] =
        Value::String("Triad aggregate could not be durably recorded.".to_owned());
    if let Some(identifier) = unconfirmed_id {
        result["unconfirmed_collaboration_id"] = Value::String(identifier);
    }
}

async fn execute_pipeline(
    gateway: &Gateway,
    args: &Value,
    resumed: &mut BTreeMap<String, Value>,
) -> crate::Result<(Option<Value>, Option<Value>, Option<Value>, usize)> {
    let mut sonnet = if let Some(value) = resumed.remove("sonnet") {
        value
    } else {
        role_result(
            gateway
                .execute_tool("claude_code_craft", &sonnet_args(args, None))
                .await,
        )
    };
    if inspect_role("sonnet", Some(sonnet.clone())).state != RoleState::Succeeded {
        return Ok((Some(sonnet), None, None, 0));
    }
    let mut codex = if let Some(value) = resumed.remove("codex") {
        value
    } else {
        role_result(
            gateway
                .execute_tool("codex_adversarial_audit", &codex_args(args, &sonnet))
                .await,
        )
    };
    let mut rounds = 0;
    let max_rounds = args["max_rounds"].as_u64().unwrap_or(1).clamp(1, 3);
    while u64::try_from(rounds + 1)? < max_rounds
        && matches!(
            inspect_role("codex", Some(codex.clone())).verdict,
            Some(Verdict::Warn | Verdict::Block)
        )
    {
        rounds += 1;
        let prior_codex = codex;
        sonnet = role_result(
            gateway
                .execute_tool("claude_code_craft", &sonnet_args(args, Some(&prior_codex)))
                .await,
        );
        if inspect_role("sonnet", Some(sonnet.clone())).state != RoleState::Succeeded {
            codex = prior_codex;
            break;
        }
        codex = execute_repair_audit(gateway, args, &sonnet, &prior_codex).await;
    }
    let argon = match invoke_argon(gateway, args, resumed.remove("argon"), &sonnet, &codex).await {
        Ok(value) => value,
        Err(error) => Some(role_result(Err(error))),
    };
    Ok((Some(sonnet), Some(codex), argon, rounds))
}

async fn execute_council(
    gateway: &Gateway,
    args: &Value,
    resumed: &mut BTreeMap<String, Value>,
) -> crate::Result<(Option<Value>, Option<Value>, Option<Value>, usize)> {
    let existing_sonnet = resumed.remove("sonnet");
    let existing_codex = resumed.remove("codex");
    let existing_argon = resumed.remove("argon");
    let initial_sonnet_args = sonnet_args(args, None);
    let empty_sonnet = json!({"context":text(args,"target_code_or_context")});
    let initial_codex_args = codex_args(args, &empty_sonnet);
    let initial_argon_args = argon_args(args, &empty_sonnet, &empty_sonnet);
    let invoke_argon = args["invoke_argon_rpc"].as_bool().unwrap_or(true);
    let sonnet_future = async {
        match existing_sonnet {
            Some(value) => value,
            None => role_result(
                gateway
                    .execute_tool("claude_code_craft", &initial_sonnet_args)
                    .await,
            ),
        }
    };
    let codex_future = async {
        match existing_codex {
            Some(value) => value,
            None => role_result(
                gateway
                    .execute_tool("codex_adversarial_audit", &initial_codex_args)
                    .await,
            ),
        }
    };
    let argon_future = async {
        match existing_argon {
            Some(value) => Some(value),
            None if invoke_argon => Some(role_result(
                gateway
                    .execute_tool("argon_architecture_review", &initial_argon_args)
                    .await,
            )),
            None => None,
        }
    };
    let (sonnet, codex, argon) = tokio::join!(sonnet_future, codex_future, argon_future);
    let mut sonnet = sonnet;
    let mut codex = codex;
    let mut argon = argon;
    if argon.is_none() && !invoke_argon {
        argon = Some(role_result(
            queue_argon(gateway, args, &sonnet, &codex).await,
        ));
    }
    let mut rounds = 0;
    let max_rounds = args["max_rounds"].as_u64().unwrap_or(1).clamp(1, 3);
    while u64::try_from(rounds + 1)? < max_rounds && invoke_argon {
        let current = BTreeMap::from([
            ("argon".to_owned(), inspect_role("argon", argon.clone())),
            (
                "sonnet".to_owned(),
                inspect_role("sonnet", Some(sonnet.clone())),
            ),
            (
                "codex".to_owned(),
                inspect_role("codex", Some(codex.clone())),
            ),
        ]);
        if adjudicate(&current).consensus_reached {
            break;
        }
        rounds += 1;
        let feedback = json!({"argon":argon,"sonnet":sonnet,"codex":codex,"round":rounds});
        let next_sonnet_args = sonnet_args(args, Some(&feedback));
        let next_codex_args = codex_args(args, &feedback);
        let next_argon_args = argon_args(args, &feedback, &feedback);
        let (next_sonnet, next_codex, next_argon) = tokio::join!(
            gateway.execute_tool("claude_code_craft", &next_sonnet_args),
            gateway.execute_tool("codex_adversarial_audit", &next_codex_args),
            gateway.execute_tool("argon_architecture_review", &next_argon_args),
        );
        sonnet = role_result(next_sonnet);
        codex = role_result(next_codex);
        argon = Some(role_result(next_argon));
    }
    Ok((Some(sonnet), Some(codex), argon, rounds))
}

async fn invoke_argon(
    gateway: &Gateway,
    args: &Value,
    resumed: Option<Value>,
    sonnet: &Value,
    codex: &Value,
) -> crate::Result<Option<Value>> {
    if resumed.is_some() {
        return Ok(resumed);
    }
    if !args["invoke_argon_rpc"].as_bool().unwrap_or(true) {
        return queue_argon(gateway, args, sonnet, codex).await.map(Some);
    }
    gateway
        .execute_tool(
            "argon_architecture_review",
            &argon_args(args, sonnet, codex),
        )
        .await
        .map(Some)
}

async fn queue_argon(
    gateway: &Gateway,
    args: &Value,
    sonnet: &Value,
    codex: &Value,
) -> crate::Result<Value> {
    let ticket = if let Some(identifier) = optional(args, "argon_handoff_ticket_id") {
        let lookup = identifier.clone();
        if let Some(ticket) = gateway
            .database(move |store| store.get_handoff(&lookup))
            .await?
        {
            let state = match ticket.status {
                HandoffStatus::Completed if ticket.verdict().is_some() => "succeeded",
                HandoffStatus::Rejected => "failed",
                HandoffStatus::InProgress => "running",
                _ => "queued",
            };
            if state == "succeeded" {
                return Ok(json!({
                    "ok":true, "status":"succeeded", "handoff_ticket_id":ticket.ticket_id,
                    "model_id":text(args,"argon_model"),
                    "structured_output":{
                        "verdict":ticket.verdict(),
                        "summary":ticket.resolution_note.unwrap_or_else(||"Argon handoff completed.".to_owned()),
                        "alexandria_ref":ticket.alexandria_ref,
                    }
                }));
            }
            return Ok(
                json!({"ok":false,"status":state,"handoff_ticket_id":ticket.ticket_id,"execution_completed":false}),
            );
        }
        identifier
    } else {
        let request = NewHandoff {
            sender: TriadRole::Codex,
            recipient: TriadRole::Argon,
            title: format!("Argon review: {}", text(args, "task_spec")),
            spec_or_payload: argon_args(args, sonnet, codex).to_string(),
            workspace_dir: workspace(args),
            priority: HandoffPriority::High,
            alexandria_ref: None,
        };
        gateway
            .database(move |store| store.create_handoff(request))
            .await?
            .ticket_id
    };
    Ok(json!({"ok":false,"status":"queued","handoff_ticket_id":ticket,"execution_completed":false}))
}

fn sonnet_args(args: &Value, feedback: Option<&Value>) -> Value {
    json!({
        "task_spec": text(args,"task_spec"),
        "target_code_or_context": format!("{}\n\nPrior Codex feedback:\n{}", text(args,"target_code_or_context"), feedback.map_or_else(String::new, Value::to_string)),
        "invariants_and_rules": text(args,"invariants_and_rules"),
        "language": text(args,"language"),
        "claude_model": text(args,"claude_model"),
        "workspace_dir": text(args,"workspace_dir"),
        "timeout_seconds": args["timeout_seconds"],
        "files_inspected": args["files_inspected"],
        "symbols_inspected": args["symbols_inspected"],
        "max_input_bytes": args["max_input_bytes"],
    })
}

fn codex_args(args: &Value, sonnet: &Value) -> Value {
    json!({
        "target_code_or_diff": format!("{}\n\nClaude role artifact:\n{}", text(args,"target_code_or_context"), sonnet),
        "design_rationale": format!("{}\n{}",text(args,"task_spec"),text(args,"invariants_and_rules")),
        "context_notes": "Independent adversarial red-team review. Return an explicit verdict and narrative.",
        "workspace_dir": text(args,"workspace_dir"),
        "model": optional(args,"codex_model").unwrap_or_else(||"gpt-6-astra".to_owned()),
        "reasoning_effort": text(args,"codex_reasoning_effort"),
        "engine": "cli",
        "timeout_seconds": args["timeout_seconds"],
        "files_inspected": args["files_inspected"],
        "symbols_inspected": args["symbols_inspected"],
        "max_input_bytes": args["max_input_bytes"],
    })
}

async fn execute_repair_audit(
    gateway: &Gateway,
    args: &Value,
    sonnet: &Value,
    previous_codex: &Value,
) -> Value {
    let prompt = repair_audit_prompt(args, sonnet, previous_codex);
    if let Some(session_id) = response_session_id(previous_codex) {
        return role_result(
            gateway
                .execute_tool(
                    "codex_session_followup",
                    &json!({
                        "session_id":session_id,
                        "prompt":prompt,
                        "workspace_dir":text(args,"workspace_dir"),
                        "fork":false,
                        "model":optional(args,"codex_model").unwrap_or_else(||"gpt-6-astra".to_owned()),
                        "timeout_seconds":args["timeout_seconds"],
                    }),
                )
                .await,
        );
    }
    let mut audit_args = codex_args(args, sonnet);
    audit_args["context_notes"] = Value::String(prompt);
    role_result(
        gateway
            .execute_tool("codex_adversarial_audit", &audit_args)
            .await,
    )
}

fn repair_audit_prompt(args: &Value, sonnet: &Value, previous_codex: &Value) -> String {
    let session_id = response_session_id(previous_codex).unwrap_or_else(|| "none".to_owned());
    let findings = role_artifact(previous_codex)
        .and_then(|artifact| artifact.get("findings"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let object = item.as_object()?;
                    Some(format!(
                        "[{}/{} @ {}] {}",
                        object
                            .get("severity")
                            .and_then(Value::as_str)
                            .unwrap_or("high"),
                        object
                            .get("category")
                            .and_then(Value::as_str)
                            .unwrap_or("issue"),
                        object
                            .get("location")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                        object
                            .get("flaw_description")
                            .and_then(Value::as_str)
                            .unwrap_or("unspecified finding"),
                    ))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| "No structured findings were supplied.".to_owned());
    format!(
        "Continue the prior red-team session and verify the repaired artifact.\n\
         session_id={session_id}\nPrior findings:\n{findings}\n\
         Task: {}\nRepaired Claude artifact:\n{sonnet}",
        text(args, "task_spec"),
    )
}

fn response_session_id(response: &Value) -> Option<String> {
    response
        .get("session_id")
        .or_else(|| response.pointer("/result/session_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn role_artifact(response: &Value) -> Option<&Value> {
    response
        .get("structured_output")
        .or_else(|| response.get("data"))
        .or_else(|| response.pointer("/result/structured_output"))
        .or_else(|| response.pointer("/result/data"))
}

fn role_result(result: crate::Result<Value>) -> Value {
    match result {
        Ok(value) => value,
        Err(error) => {
            let message = error.to_string();
            let status = if message.to_ascii_lowercase().contains("timed out") {
                "timed_out_unknown"
            } else {
                "failed"
            };
            json!({
                "ok":false,
                "status":status,
                "outcome_state":if status == "timed_out_unknown" {"outcome_unknown"} else {"confirmed_failure"},
                "error":message,
            })
        }
    }
}

fn argon_args(args: &Value, sonnet: &Value, codex: &Value) -> Value {
    json!({
        "task_spec": text(args,"task_spec"),
        "target_code_or_context": format!("{}\n\nClaude result:\n{}\n\nCodex result:\n{}",text(args,"target_code_or_context"),sonnet,codex),
        "invariants_and_rules": text(args,"invariants_and_rules"),
        "workspace_dir": text(args,"workspace_dir"),
        "model": text(args,"argon_model"),
        "reasoning_effort": "high",
        "engine": "antigravity_cli",
        "timeout_seconds": args["timeout_seconds"],
        "files_inspected": args["files_inspected"],
        "symbols_inspected": args["symbols_inspected"],
        "max_input_bytes": args["max_input_bytes"],
    })
}

fn normalized_mode(args: &Value) -> String {
    let mode = text(args, "mode").trim().to_ascii_lowercase();
    if mode.is_empty() {
        "pipeline".to_owned()
    } else {
        mode
    }
}

fn source_task_digest(args: &Value) -> crate::Result<String> {
    canonical_digest(&json!({
        "task_spec": text(args,"task_spec"),
        "target_code_or_context": text(args,"target_code_or_context"),
        "invariants_and_rules": text(args,"invariants_and_rules"),
        "language": text(args,"language"),
        "mode": normalized_mode(args),
    }))
}

fn evidence_scope(workspace: &Path, args: &Value) -> crate::Result<Value> {
    let root = workspace.canonicalize()?;
    let mut digests = BTreeMap::new();
    let mut missing = Vec::new();
    for relative in args["files_inspected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let candidate = root.join(relative);
        let resolved = candidate.canonicalize();
        match resolved {
            Ok(path) if path.starts_with(&root) && path.is_file() => {
                let bytes = std::fs::read(path)?;
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                digests.insert(
                    relative.to_owned(),
                    format!("sha256:{:x}", hasher.finalize()),
                );
            }
            _ => missing.push(relative.to_owned()),
        }
    }
    let inline = text(args, "target_code_or_context");
    let mut inline_hasher = Sha256::new();
    inline_hasher.update(inline.as_bytes());
    let inline_digest = format!("sha256:{:x}", inline_hasher.finalize());
    let mut symbols = args["symbols_inspected"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    symbols.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    let identity = json!({
        "inline_context_digest":inline_digest,
        "files_inspected_digests":digests,
        "symbols_inspected":symbols,
    });
    let scope = json!({
        "workspace_dir": root,
        "codegraph":{"workspace_dir":root,"indexed":root.join(".codegraph/codegraph.db").is_file()},
        "scope_digest":canonical_digest(&identity)?,
        "inline_context_bytes":inline.len(),
        "inline_context_digest":inline_digest,
        "files_inspected_digests": identity["files_inspected_digests"],
        "unverified_or_missing_files": missing,
        "symbols_inspected": identity["symbols_inspected"],
        "executed_test_commands":args["executed_test_commands"],
        "evidence_dimensions":{
            "static_code_review":"not_run","test_generation":"not_run","test_execution":"not_run",
            "integration_verification":"not_run","runtime_confirmation":"not_run"
        },
        "test_execution_evidence_origin": "caller_reported",
    });
    Ok(scope)
}

fn generated_tests(snapshot: &RoleSnapshot) -> bool {
    let Some(artifact) = snapshot.artifact.as_ref() else {
        return false;
    };
    artifact
        .get("failing_test_cases")
        .and_then(Value::as_array)
        .is_some_and(|tests| !tests.is_empty())
        || artifact
            .get("unit_tests")
            .and_then(Value::as_str)
            .is_some_and(|tests| !tests.trim().is_empty())
}

async fn load_resumable_roles(
    gateway: &Gateway,
    collaboration_id: &str,
    expected_workspace: &Path,
    task_digest: &str,
    scope: &Value,
) -> crate::Result<BTreeMap<String, Value>> {
    let identifier = collaboration_id.to_owned();
    let record = gateway
        .database(move |store| store.get_job(&identifier))
        .await?;
    let Some(record) = record else {
        return Ok(BTreeMap::new());
    };
    if record.task_type != "triad_orchestration"
        || record.status != JobStatus::Completed
        || PathBuf::from(&record.workspace_dir).canonicalize()? != expected_workspace
    {
        return Ok(BTreeMap::new());
    }
    let Some(result) = record.result else {
        return Ok(BTreeMap::new());
    };
    if result["source_task_digest"] != task_digest
        || result["evidence_scope"]["workspace_dir"] != scope["workspace_dir"]
        || result["evidence_scope"]["files_inspected_digests"] != scope["files_inspected_digests"]
        || result["evidence_scope"]["unverified_or_missing_files"]
            != scope["unverified_or_missing_files"]
    {
        return Ok(BTreeMap::new());
    }
    if !snapshot_valid(&result)? {
        return Ok(BTreeMap::new());
    }
    let snapshots = REQUIRED_ROLES
        .iter()
        .map(|role| {
            Ok((
                (*role).to_owned(),
                serde_json::from_value::<RoleSnapshot>(result["roles"][role].clone())?,
            ))
        })
        .collect::<crate::Result<Vec<_>>>()?;
    gateway
        .database(move |store| {
            let mut resumed = BTreeMap::new();
            for (role, snapshot) in snapshots {
                if snapshot.state != RoleState::Succeeded
                    || !snapshot.review_artifact_present
                    || snapshot.verdict.is_none()
                {
                    continue;
                }
                if let Some(job_id) = snapshot.job_id {
                    let Some(job) = store.get_job(&job_id)? else {
                        continue;
                    };
                    if job.status == JobStatus::Completed
                        && let Some(role_result) = job.result
                    {
                        resumed.insert(
                            role,
                            json!({"ok":true,"status":"succeeded","job_id":job_id,"result":role_result}),
                        );
                    }
                } else if let Some(ticket_id) = snapshot.handoff_ticket_id {
                    let Some(ticket) = store.get_handoff(&ticket_id)? else {
                        continue;
                    };
                    if ticket.status == HandoffStatus::Completed {
                        resumed.insert(
                            role,
                            json!({
                                "ok":true,"status":"succeeded","handoff_ticket_id":ticket_id,
                                "model_id":snapshot.model_id,
                                "structured_output":{
                                    "verdict":ticket.verdict(),
                                    "summary":ticket.resolution_note.unwrap_or_else(||"Argon handoff completed.".to_owned()),
                                    "alexandria_ref":ticket.alexandria_ref,
                                }
                            }),
                        );
                    }
                }
            }
            Ok(resumed)
        })
        .await
}

pub(crate) fn snapshot_valid(snapshot: &Value) -> crate::Result<bool> {
    let Some(stored) = snapshot["snapshot_digest"].as_str() else {
        return Ok(false);
    };
    let mut unsigned = snapshot.clone();
    unsigned
        .as_object_mut()
        .ok_or("snapshot must be object")?
        .remove("snapshot_digest");
    Ok(canonical_digest(&unsigned)? == stored)
}

fn timeout_duration(args: &Value) -> Duration {
    Duration::from_secs_f64(
        args["timeout_seconds"]
            .as_f64()
            .unwrap_or(0.0)
            .clamp(0.0, 86_400.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_identity_fences_normalized_orchestration_mode() -> crate::Result<()> {
        let base = json!({"task_spec":"review source","language":"rust"});
        let mut pipeline = base.clone();
        pipeline["mode"] = json!("pipeline");
        let mut council = base.clone();
        council["mode"] = json!("council");
        let mut padded_council = base.clone();
        padded_council["mode"] = json!(" Council ");
        assert_eq!(source_task_digest(&base)?, source_task_digest(&pipeline)?);
        assert_eq!(
            source_task_digest(&council)?,
            source_task_digest(&padded_council)?
        );
        assert_ne!(
            source_task_digest(&pipeline)?,
            source_task_digest(&council)?
        );
        assert_eq!(
            triad_request_identity(&base)?,
            triad_request_identity(&pipeline)?
        );
        assert_eq!(
            triad_request_identity(&council)?,
            triad_request_identity(&padded_council)?
        );
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_identity_is_reserved_before_roles_and_rejects_changed_request()
    -> crate::Result<()> {
        let temporary = tempfile::tempdir()?;
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir(&workspace)?;
        let gateway = Gateway::new(temporary.path().join("state"));
        let args = json!({
            "workspace_dir":workspace, "task_spec":"review source",
            "request_id":"cdx-triad-identity", "mode":"council",
        });
        let task_digest = source_task_digest(&args)?;
        let start = aggregate_start(&gateway, &args, &workspace, &task_digest).await?;
        assert!(matches!(start, AggregateStart::Reserved(ref id) if id == "cdx-triad-identity"));
        let stored = gateway
            .database(|store| store.get_job("cdx-triad-identity"))
            .await?
            .ok_or("triad reservation missing")?;
        assert_eq!(stored.status, JobStatus::Running);
        assert!(stored.metadata.contains_key("request_identity"));

        let mut changed = args;
        changed["task_spec"] = Value::String("different request".to_owned());
        let changed_digest = source_task_digest(&changed)?;
        assert!(
            aggregate_start(&gateway, &changed, &workspace, &changed_digest)
                .await
                .is_err()
        );
        gateway.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn reserved_aggregate_failure_is_persisted() -> crate::Result<()> {
        let temporary = tempfile::tempdir()?;
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir(&workspace)?;
        let gateway = Gateway::new(temporary.path().join("state"));
        let args = json!({
            "workspace_dir":workspace, "task_spec":"controlled failure",
            "request_id":"cdx-triad-failed-resume",
        });
        let task_digest = source_task_digest(&args)?;
        let start = aggregate_start(&gateway, &args, &workspace, &task_digest).await?;
        assert!(matches!(start, AggregateStart::Reserved(_)));
        let error: crate::Error = std::io::Error::other("controlled failure").into();
        fail_reserved_aggregate(&gateway, "cdx-triad-failed-resume", &error).await?;
        let stored = gateway
            .database(|store| store.get_job("cdx-triad-failed-resume"))
            .await?
            .ok_or("failed triad reservation missing")?;
        assert_eq!(stored.status, JobStatus::Failed);
        assert_eq!(stored.attempt_count, 1);
        gateway.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn file_workspace_is_rejected_before_aggregate_or_provider_registration()
    -> crate::Result<()> {
        let temporary = tempfile::tempdir()?;
        let workspace_file = temporary.path().join("not-a-directory");
        std::fs::write(&workspace_file, "file")?;
        let gateway = Gateway::new(temporary.path().join("state"));
        let outcome = orchestrate(&gateway, &json!({"workspace_dir":workspace_file,"task_spec":"review","request_id":"cdx-invalid-workspace"})).await;
        assert!(outcome.is_err());
        assert!(!temporary.path().join("state").exists());
        assert_eq!(gateway.active_background_jobs().await, 0);
        Ok(())
    }

    #[test]
    fn snapshot_digest_detects_tampering() -> crate::Result<()> {
        let mut snapshot = json!({"schema_version":2,"roles":{},"workspace_dir":"/tmp"});
        snapshot["snapshot_digest"] = Value::String(canonical_digest(&snapshot)?);
        assert!(snapshot_valid(&snapshot)?);
        snapshot["workspace_dir"] = Value::String("/other".to_owned());
        assert!(!snapshot_valid(&snapshot)?);
        Ok(())
    }

    #[test]
    fn evidence_scope_hashes_only_workspace_files() -> crate::Result<()> {
        let workspace = tempfile::tempdir()?;
        std::fs::write(workspace.path().join("inside.rs"), "fn inside() {}")?;
        let args =
            json!({"files_inspected":["inside.rs","../outside.rs"],"symbols_inspected":["inside"]});
        let scope = evidence_scope(workspace.path(), &args)?;
        assert!(
            scope["files_inspected_digests"]["inside.rs"]
                .as_str()
                .is_some()
        );
        assert_eq!(
            scope["unverified_or_missing_files"],
            json!(["../outside.rs"])
        );
        assert!(
            scope["scope_digest"]
                .as_str()
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        Ok(())
    }

    #[test]
    fn compact_snapshot_excludes_model_prose_and_failure_keeps_peer_outputs() -> crate::Result<()> {
        let role = inspect_role(
            "codex",
            Some(json!({
                "ok":true,"status":"succeeded","job_id":"cdx-role0001",
                "structured_output":{"verdict":"PASS","summary":"SECRET MODEL PROSE"}
            })),
        );
        let roles = BTreeMap::from([("codex".to_owned(), role.compact())]);
        let mut result = json!({
            "orchestration_mode":"council","source_task_digest":"sha256:task",
            "workspace_dir":"/tmp","evidence_scope":{},"roles":roles,
            "role_evidence_dimensions":{},"adjudication":{},
            "codex_audit":{"structured_output":{"summary":"SECRET MODEL PROSE"}}
        });
        let snapshot = compact_snapshot(&result, "cdx-aggregate")?;
        assert!(!serde_json::to_string(&snapshot)?.contains("SECRET MODEL PROSE"));
        mark_persistence_failure(&mut result, Some("cdx-aggregate".to_owned()));
        assert_eq!(result["collaboration_persistence"], "failed");
        assert!(result.get("collaboration_id").is_none());
        assert_eq!(
            result["codex_audit"]["structured_output"]["summary"],
            "SECRET MODEL PROSE"
        );
        Ok(())
    }

    #[test]
    fn repair_prompt_preserves_session_and_structured_findings() {
        let args = json!({"task_spec":"Repair CAS"});
        let previous = json!({
            "result":{
                "session_id":"thr_continuity_001",
                "structured_output":{"findings":[{
                    "severity":"critical","category":"concurrency","location":"job_store.py:92",
                    "flaw_description":"Missing CAS guard on cancelled status"
                }]}
            }
        });
        let prompt = repair_audit_prompt(&args, &json!({"crafted_code":"fixed"}), &previous);
        assert!(prompt.contains("session_id=thr_continuity_001"));
        assert!(prompt.contains(
            "[critical/concurrency @ job_store.py:92] Missing CAS guard on cancelled status"
        ));
    }

    #[test]
    fn role_errors_are_independent_outcomes() {
        let timed_out = role_result(Err("provider timed out after dispatch".into()));
        let failed = role_result(Err("provider unavailable".into()));
        assert_eq!(timed_out["status"], "timed_out_unknown");
        assert_eq!(failed["status"], "failed");
    }
}
