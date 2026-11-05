//! Source-grounded planning proposal and mechanical build-readiness checks.

use crate::execution_policy::ExecutionDeadline;
use std::collections::BTreeSet;
use std::path::Path;
#[cfg(test)]
use std::time::Instant;

use serde_json::{Value, json};

use crate::Result;
use crate::context::{SourceContextBuilder, discover_relevance};
use crate::dag::{DagNode, DagPlanner, DagSettlementVerifier, NodeKind, NodeRole};
use crate::gateway::Gateway;
use crate::research::ModelSpec;
use crate::routing::{Engine, Provider, RouteRequest, digest_output, validate_route};
use crate::storage::JobStatus;

use super::artifacts;
use super::{PlanRecord, PlanningArtifact, WorkspaceRunStatus};

const CONTRACT: &str = "workspace_planning/v1";
const MAX_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_LIST_ITEMS: usize = 64;
const MAX_TASKS: usize = 16;

pub fn planner_job_id(flow_id: &str) -> Result<String> {
    stable_id("wsp", &(CONTRACT, flow_id, "provider"))
}

pub fn plan_id(flow_id: &str) -> Result<String> {
    stable_id("wpa", &(CONTRACT, flow_id, "artifact"))
}

fn stable_id(prefix: &str, value: &impl serde::Serialize) -> Result<String> {
    let digest = digest_output(&serde_json::to_string(value)?);
    Ok(format!(
        "{prefix}-{}",
        &digest.trim_start_matches("sha256:")[..32]
    ))
}

// Keep execution ownership, immutable source context and model policy explicit
// at this internal stage boundary, matching the research stage API.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    gateway: &Gateway,
    flow_id: &str,
    original: &Path,
    execution_workspace: &Path,
    goal: &str,
    research: Option<&Value>,
    model: &ModelSpec,
    deadline: impl Into<ExecutionDeadline> + Send,
) -> Result<PlanRecord> {
    let deadline = deadline.into();
    bounded_text(flow_id, 256, "flow ID")?;
    bounded_text(goal, 16_384, "planning goal")?;
    let original = original.canonicalize()?;
    let execution_workspace = execution_workspace.canonicalize()?;
    if execution_workspace.starts_with(&original) || !execution_workspace.is_dir() {
        return Err("planning execution workspace must be an external directory".into());
    }
    let before = capture(&original).await?;
    let research_value = match research {
        Some(value) => {
            let id = value["request_id"]
                .as_str()
                .ok_or("planning research reference has no original request ID")?;
            artifacts::read_research(gateway, &original, id)
                .await?
                .ok_or("planning research reference was not found")?
        }
        None => Value::Null,
    };
    if serde_json::to_vec(&research_value)?.len() > 1_048_576 {
        return Err("planning research input exceeds byte budget".into());
    }
    let context_root = original.clone();
    let context_goal = goal.to_owned();
    let context = tokio::task::spawn_blocking(move || {
        let relevance = discover_relevance(&context_root, &context_goal, 8)?;
        SourceContextBuilder::new(&context_root)?.build(
            &relevance.files,
            &relevance.symbols,
            32_000,
        )
    })
    .await??;
    let research_id = research_value["request_id"].as_str().map(str::to_owned);
    let research_digest = research_id
        .as_ref()
        .map(|_| digest_output(&research_value.to_string()));
    let planner_id = planner_job_id(flow_id)?;
    let remaining = deadline
        .remaining_seconds()
        .ok_or("workflow deadline exhausted before planning admission")?;
    let route = planning_route(model)?;
    let default_gate = crate::dag::default_gate_command(&original, None);
    let host_gate = crate::dag::gate_command_supported(&default_gate).then_some(default_gate);
    let task = json!({
        "goal": goal,
        "original_workspace": original,
        "source_manifest_digest": before.digest,
        "research": research_value,
        "research_digest": research_digest,
        "code_context": context,
        "planning_contract": {
            "proposal_not_semantic_approval": true,
            "maximum_tasks": MAX_TASKS,
            "implementation_task_example":{"role":"interface_builder","model":"claude-sonnet-5-5-medium","reasoning_effort":"medium"},
            "read_only_task_example":{"role":"contract_verifier","model":"gpt-6.1-sol","reasoning_effort":"high"},
            "host_default_verification_command":crate::dag::default_gate_command(&original,None),
            "one_task_gains_host_downstream_verifier":true,
            "required_waves": "2-4",
            "verification_commands_are_host_allowlisted": true,
            "host_default_verification_command": host_gate,
            "single_implementation_task_gets_host_verifier": true
        }
    });
    let key = digest_output(&serde_json::to_string(&(
        CONTRACT,
        flow_id,
        &before.digest,
        &research_digest,
        model,
        goal,
    ))?);
    let args = json!({
        "task_spec": serde_json::to_string(&task)?,
        "workspace_dir": execution_workspace,
        "model": model.model,
        "reasoning_effort": model.effort,
        "engine": if route.engine == Engine::CodexCli { "cli" } else { "antigravity_cli" },
        "timeout_seconds": remaining,
        "request_id": planner_id,
        "idempotency_key": key,
        "max_input_bytes": 1_500_000,
        "sandbox_mode": "read-only"
    });
    if crate::supervisor::is_cancelled() || deadline.is_elapsed() {
        return Err("planning was not admitted after cancellation/deadline".into());
    }
    let response = gateway.call("planning_stage", args).await;
    let lookup = planner_id.clone();
    let stored = gateway
        .database(move |store| store.get_job(&lookup))
        .await?
        .ok_or("planning stage has no durable observation; inspect its original ID")?;
    let receipt = stored.receipt.clone().unwrap_or(Value::Null);
    if stored.status == JobStatus::TimedOutUnknown || receipt["outcome_state"] == "outcome_unknown"
    {
        return Err("planning outcome is unknown; inspect the original planning job ID".into());
    }
    if stored.status != JobStatus::Completed
        || receipt["actual_model"] != receipt["resolved_model"]
        || !receipt["actual_model"].is_string()
        || receipt["requested_model"] != model.model
        || receipt["model_mismatch"] == true
        || (matches!(route.engine, Engine::CodexCli | Engine::AntigravityCli)
            && receipt["owned_processes_closed"] != true)
    {
        return Err(format!(
            "planning stage did not produce matching confirmed evidence: {}",
            stored.status.as_str()
        )
        .into());
    }
    let artifact = response
        .as_ref()
        .ok()
        .and_then(|value| value.pointer("/result/structured_output"))
        .or_else(|| {
            stored
                .result
                .as_ref()
                .and_then(|value| value.pointer("/result/structured_output"))
        })
        .ok_or("planning stage has no structured artifact")?;
    if serde_json::to_vec(artifact)?.len() > MAX_ARTIFACT_BYTES {
        return Err("planning artifact exceeds byte budget".into());
    }
    let mut artifact: PlanningArtifact = serde_json::from_value(artifact.clone())?;
    let mut readiness_reasons = validate_artifact(&original, &mut artifact)?;
    let after = capture(&original).await?;
    if before != after {
        readiness_reasons.push("original workspace changed during planning".into());
    }
    readiness_reasons.sort();
    readiness_reasons.dedup();
    let artifact_digest = digest_output(&serde_json::to_string(&artifact)?);
    let record = PlanRecord {
        schema_version: CONTRACT.into(),
        plan_id: plan_id(flow_id)?,
        flow_id: flow_id.into(),
        workspace_dir: original,
        source_manifest_digest: before.digest,
        research_id,
        research_digest,
        planner_job_id: planner_id,
        planner_receipt: receipt,
        execution_workspace,
        artifact,
        artifact_digest,
        status: if readiness_reasons.is_empty() {
            WorkspaceRunStatus::BuildReady
        } else {
            WorkspaceRunStatus::Completed
        },
        readiness_reasons,
    };
    artifacts::persist_plan(gateway, &record).await?;
    Ok(record)
}

pub fn validate_artifact(original: &Path, artifact: &mut PlanningArtifact) -> Result<Vec<String>> {
    for (value, label, limit) in [
        (&artifact.title, "title", 512),
        (&artifact.objective, "objective", 8_192),
    ] {
        bounded_text(value, limit, label)?;
    }
    for (values, label) in [
        (&artifact.requirements, "requirement"),
        (&artifact.acceptance_criteria, "acceptance criterion"),
        (&artifact.scope, "scope"),
        (&artifact.non_goals, "non-goal"),
        (&artifact.decisions, "decision"),
        (&artifact.risks, "risk"),
        (&artifact.blocking_questions, "blocking question"),
    ] {
        bounded_list(values, label)?;
    }
    if artifact.tasks.len() == 1 {
        let upstream = artifact.tasks[0].clone();
        artifact.tasks.push(DagNode {
            id: "host_verification".into(),
            title: "Host verification".into(),
            task: format!(
                "Verify the bounded implementation for planning artifact task '{}'.",
                upstream.id
            ),
            role: NodeRole::ContractVerifier,
            node_kind: NodeKind::Verification,
            model: "gpt-6.1-sol".into(),
            reasoning_effort: "high".into(),
            dependencies: vec![upstream.id],
            read_files: upstream
                .read_files
                .into_iter()
                .chain(upstream.write_files)
                .collect(),
            write_files: Vec::new(),
            symbols: Vec::new(),
            verify_command: upstream.verify_command.clone(),
            reproduction_expected_exit_nonzero: true,
        });
    }
    if artifact.tasks.len() > MAX_TASKS {
        return Err("planning task count exceeds budget".into());
    }
    let mut reasons = Vec::new();
    if artifact.requirements.is_empty() {
        reasons.push("plan has no requirements".into());
    }
    if artifact.acceptance_criteria.is_empty() {
        reasons.push("plan has no acceptance criteria".into());
    }
    if artifact.tasks.is_empty() {
        reasons.push("plan has no executable tasks".into());
    }
    if !artifact.blocking_questions.is_empty() {
        reasons.push("plan retains blocking questions".into());
    }
    // Freeze the same host-owned default the DAG would actually execute. A
    // missing/unsupported default is a readiness gap, never a runnable plan.
    for task in &mut artifact.tasks {
        let command = crate::dag::default_gate_command(original, task.verify_command.as_deref());
        if crate::dag::gate_command_supported(&command) {
            task.verify_command = Some(command);
        } else {
            reasons.push(format!(
                "task '{}' has no supported verification command (unsupported or missing)",
                task.id
            ));
        }
    }
    let mut ids = BTreeSet::new();
    for task in &artifact.tasks {
        bounded_text(&task.id, 96, "task ID")?;
        if !task
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            reasons.push(format!("task '{}' has an invalid task ID", task.id));
        }
        bounded_text(&task.title, 512, "task title")?;
        bounded_text(&task.task, 16_384, "task instruction")?;
        if !ids.insert(task.id.as_str()) {
            reasons.push("plan has duplicate task IDs".into());
        }
        if crate::dag::validate_node_route(task).is_err() {
            reasons.push(format!(
                "task '{}' has an unsupported model route for its DAG role",
                task.id
            ));
        }
        if !task.write_files.is_empty() && task.role != NodeRole::InterfaceBuilder {
            reasons.push(format!(
                "task '{}' declares writes outside the craft role",
                task.id
            ));
        }
        for path in &task.read_files {
            let relative = validated_task_path(path)?;
            validate_task_location(original, &relative)?;
            let selected = original.join(&relative);
            let metadata = std::fs::symlink_metadata(&selected);
            if !metadata
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                && !artifact.tasks.iter().any(|writer| {
                    writer.id != task.id
                        && depends_on_task(&artifact.tasks, task, &writer.id)
                        && writer.write_files.iter().any(|written| {
                            validated_task_path(written).ok().as_ref() == Some(&relative)
                        })
                })
            {
                reasons.push(format!(
                    "task '{}' reads an unavailable or unsafe file",
                    task.id
                ));
            }
        }
        for path in &task.write_files {
            let relative = validated_task_path(path)?;
            validate_task_location(original, &relative)?;
        }
    }
    let compiled =
        DagPlanner::compile_with_workspace(&artifact.objective, artifact.tasks.clone(), original)?;
    reasons.extend(
        compiled
            .hazards
            .into_iter()
            .map(|hazard| format!("{}: {}", hazard.code, hazard.message)),
    );
    Ok(reasons)
}

fn depends_on_task(tasks: &[DagNode], task: &DagNode, candidate: &str) -> bool {
    let mut pending = task
        .dependencies
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if id == candidate {
            return true;
        }
        if let Some(parent) = tasks.iter().find(|node| node.id == id) {
            pending.extend(parent.dependencies.iter().map(String::as_str));
        }
    }
    false
}

fn validated_task_path(path: &Path) -> Result<std::path::PathBuf> {
    let relative = crate::dag::normalize_relative(path)?;
    if relative.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".git"
                | ".codex"
                | ".omx"
                | ".hcodex"
                | ".codegraph"
                | ".gemini"
                | ".ssh"
                | ".aws"
                | ".azure"
                | "auth.json"
                | "credentials.json"
                | "credentials.toml"
                | "secrets.toml"
        ) || name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
    }) {
        return Err("planning task path selects private state or credentials".into());
    }
    Ok(relative)
}

fn validate_task_location(original: &Path, relative: &Path) -> Result<()> {
    let mut selected = original.to_path_buf();
    for component in relative.components() {
        selected.push(component.as_os_str());
        match std::fs::symlink_metadata(&selected) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("planning task traverses a symlink".into());
            }
            Ok(metadata) if selected != original.join(relative) && !metadata.is_dir() => {
                return Err("planning task parent is not a directory".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn planning_route(model: &ModelSpec) -> Result<crate::routing::Route> {
    let cli = model.model.starts_with("gpt-") && !model.model.starts_with("gpt-oss");
    validate_route(&RouteRequest {
        provider: if cli {
            Provider::OpenaiCodex
        } else {
            Provider::Antigravity
        },
        engine: if cli {
            Engine::CodexCli
        } else {
            Engine::AntigravityCli
        },
        model: model.model.clone(),
        effort: model.effort.clone(),
        prompt: String::new(),
    })
}

async fn capture(path: &Path) -> Result<crate::dag::WorkspaceManifest> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || DagSettlementVerifier::capture(&path)).await?
}

fn bounded_list(values: &[String], label: &str) -> Result<()> {
    if values.len() > MAX_LIST_ITEMS {
        return Err(format!("planning {label} count exceeds budget").into());
    }
    for value in values {
        bounded_text(value, 4_096, label)?;
    }
    Ok(())
}
fn bounded_text(value: &str, limit: usize, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > limit
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(format!("planning {label} is empty, oversized or contains controls").into());
    }
    Ok(())
}

pub fn output_schema() -> Value {
    let strings = |max: usize| json!({"type":"array","maxItems":max,"items":{"type":"string","maxLength":4096}});
    json!({
        "type":"object","additionalProperties":false,
        "properties":{
            "title":{"type":"string","maxLength":512},"objective":{"type":"string","maxLength":8192},
            "requirements":strings(64),"acceptance_criteria":strings(64),"scope":strings(64),
            "non_goals":strings(64),"decisions":strings(64),"risks":strings(64),"blocking_questions":strings(64),
            "tasks":{"type":"array","maxItems":16,"items":task_schema()}
        },
        "required":["title","objective","requirements","acceptance_criteria","scope","non_goals","decisions","risks","blocking_questions","tasks"]
    })
}

fn task_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "id":{"type":"string","maxLength":96},"title":{"type":"string","maxLength":512},
        "task":{"type":"string","maxLength":16384},
        "role":{"type":"string","enum":["topology_planner","boundary_mapper","interface_builder","contract_verifier","security_auditor","integration_fan_in"]},
        "node_kind":{"type":"string","enum":["standard","reproduction","verification"]},
        "model":{"type":"string","maxLength":128},"reasoning_effort":{"type":"string","enum":["low","medium","high","xhigh","max","ultra"]},
        "dependencies":{"type":"array","maxItems":16,"items":{"type":"string","maxLength":96}},
        "read_files":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":2048}},
        "write_files":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":2048}},
        "symbols":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":512}},
        "verify_command":{"anyOf":[{"type":"string","maxLength":256},{"type":"null"}]},
        "reproduction_expected_exit_nonzero":{"type":"boolean"}
    },"required":["id","title","task","role","node_kind","model","reasoning_effort","dependencies","read_files","write_files","symbols","verify_command","reproduction_expected_exit_nonzero"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{ReceiptStatus, close_receipt, new_receipt_with_context};
    use crate::runtime::ExecutionResult;
    fn task() -> DagNode {
        DagNode {
            id: "implement".into(),
            title: "Implement".into(),
            task: "Implement bounded change".into(),
            role: NodeRole::InterfaceBuilder,
            node_kind: NodeKind::Standard,
            model: "claude-sonnet-5-5-medium".into(),
            reasoning_effort: "medium".into(),
            dependencies: vec![],
            read_files: vec!["src/lib.rs".into()],
            write_files: vec!["src/lib.rs".into()],
            symbols: vec![],
            verify_command: Some("cargo test".into()),
            reproduction_expected_exit_nonzero: true,
        }
    }
    #[test]
    fn single_task_gains_host_verifier_and_arbitrary_shell_is_not_ready() -> Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("src"))?;
        std::fs::write(root.path().join("src/lib.rs"), "pub fn x() {}\n")?;
        let mut artifact = PlanningArtifact {
            title: "Feature".into(),
            objective: "Implement feature".into(),
            requirements: vec!["Bounded".into()],
            acceptance_criteria: vec!["Tests pass".into()],
            scope: vec!["src/lib.rs".into()],
            non_goals: vec!["No API break".into()],
            decisions: vec!["Reuse code".into()],
            risks: vec!["Regression".into()],
            blocking_questions: vec![],
            tasks: vec![task()],
        };
        assert!(validate_artifact(root.path(), &mut artifact)?.is_empty());
        assert_eq!(artifact.tasks.len(), 2);
        artifact.tasks[0].verify_command = Some("sh -c whoami".into());
        assert!(
            validate_artifact(root.path(), &mut artifact)?
                .iter()
                .any(|r| r.contains("unsupported"))
        );
        Ok(())
    }

    #[test]
    fn actual_dag_role_route_is_required_before_build_ready() -> Result<()> {
        let mut node = task();
        node.model = "gpt-6.1-sol".into();
        node.reasoning_effort = "high".into();
        assert!(crate::dag::validate_node_route(&node).is_err());
        node.role = NodeRole::ContractVerifier;
        node.model = "claude-sonnet-5-5-medium".into();
        node.reasoning_effort = "medium".into();
        assert!(crate::dag::validate_node_route(&node).is_err());
        node.model = "gpt-6.1-sol".into();
        node.reasoning_effort = "high".into();
        assert!(crate::dag::validate_node_route(&node).is_ok());
        Ok(())
    }

    #[test]
    fn missing_default_gate_is_not_ready_and_real_default_is_frozen() -> Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("src"))?;
        std::fs::write(root.path().join("src/lib.rs"), "pub fn value() {}\n")?;
        let mut artifact = PlanningArtifact {
            title: "Feature".into(),
            objective: "Implement feature".into(),
            requirements: vec!["bounded".into()],
            acceptance_criteria: vec!["tests pass".into()],
            scope: vec!["src/lib.rs".into()],
            non_goals: vec![],
            decisions: vec![],
            risks: vec![],
            blocking_questions: vec![],
            tasks: vec![task()],
        };
        artifact.tasks[0].verify_command = None;
        assert!(
            validate_artifact(root.path(), &mut artifact)?
                .iter()
                .any(|reason| reason.contains("no supported verification"))
        );
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname='gate_fixture'\nversion='0.1.0'\nedition='2024'\n",
        )?;
        assert!(validate_artifact(root.path(), &mut artifact)?.is_empty());
        assert!(
            artifact
                .tasks
                .iter()
                .all(|task| task.verify_command.as_deref()
                    == Some("cargo test --workspace --all-targets"))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn planning_rejects_symlink_parents_and_private_state_before_build_readiness() -> Result<()> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        let outside = root.path().join("outside");
        std::fs::create_dir(&original)?;
        std::fs::create_dir(&outside)?;
        std::fs::write(outside.join("private.rs"), "outside evidence")?;
        std::os::unix::fs::symlink(&outside, original.join("bridge"))?;
        assert!(validate_task_location(&original, Path::new("bridge/private.rs")).is_err());
        assert!(validate_task_location(&original, Path::new("bridge/new.rs")).is_err());
        assert!(validated_task_path(Path::new(".hcodex/jobs.db")).is_err());
        assert!(validated_task_path(Path::new("auth.json")).is_err());
        Ok(())
    }

    #[test]
    fn absent_read_requires_an_upstream_producer_not_a_later_writer() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut writer = task();
        writer.id = "writer".into();
        writer.read_files.clear();
        writer.write_files = vec!["created.rs".into()];
        writer.dependencies = vec!["reader".into()];
        let mut reader = task();
        reader.id = "reader".into();
        reader.read_files = vec!["created.rs".into()];
        reader.write_files.clear();
        let mut artifact = PlanningArtifact {
            title: "Feature".into(),
            objective: "Implement feature".into(),
            requirements: vec!["bounded".into()],
            acceptance_criteria: vec!["tests pass".into()],
            scope: vec!["created.rs".into()],
            non_goals: vec![],
            decisions: vec![],
            risks: vec![],
            blocking_questions: vec![],
            tasks: vec![reader, writer],
        };
        assert!(
            validate_artifact(root.path(), &mut artifact)?
                .iter()
                .any(|reason| reason.contains("unavailable"))
        );
        artifact.tasks[0].dependencies = vec!["writer".into()];
        artifact.tasks[1].dependencies.clear();
        assert!(validate_artifact(root.path(), &mut artifact)?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn planning_executes_externally_and_persists_mechanical_artifact() -> Result<()> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("repo");
        let state = root.path().join("state");
        let external = state.join("workspaces/flow/planning");
        std::fs::create_dir_all(original.join("src"))?;
        std::fs::create_dir_all(&external)?;
        std::fs::write(original.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n")?;
        let artifact = PlanningArtifact {
            title: "Change value".into(),
            objective: "Change the bounded value implementation".into(),
            requirements: vec!["Preserve the public signature".into()],
            acceptance_criteria: vec!["cargo test passes".into()],
            scope: vec!["src/lib.rs".into()],
            non_goals: vec!["No dependency changes".into()],
            decisions: vec!["Use the existing module".into()],
            risks: vec!["Behavior regression".into()],
            blocking_questions: Vec::new(),
            tasks: vec![task()],
        };
        let structured = serde_json::to_value(artifact)?;
        let provider_output = structured.clone();
        let gateway = Gateway::new(state.clone()).with_test_provider(move |request, options| {
            assert!(request.prompt.contains("ROLE: PROJECT PLANNING"));
            let route = validate_route(request)?;
            let mut receipt = new_receipt_with_context(
                &route,
                options.request_id.as_deref(),
                options.idempotency_key.clone(),
            );
            receipt.status = ReceiptStatus::Completed;
            receipt.actual_model = Some(route.resolved_model);
            receipt.identity_source = Some("planning_test_boundary".into());
            receipt.owned_processes_closed = Some(true);
            receipt.outcome_state = "confirmed_success".into();
            let output = provider_output.to_string();
            receipt.output_digest = Some(digest_output(&output));
            close_receipt(&mut receipt);
            Ok(ExecutionResult {
                receipt,
                output,
                structured_output: Some(provider_output.clone()),
                placeholder: None,
                session_id: None,
                exit_code: Some(0),
                search_evidence: None,
            })
        });
        let research_report = json!({"request_id":"research-planning-fixture","workspace_dir":original,"research_status":"incomplete_evidence","sources":[{"excerpt":{"text":"actual captured source"}}],"claims":[],"unresolved":["bounded evidence gap"]});
        let raw_report = research_report.clone();
        let research_original = original.clone();
        gateway
            .database(move |store| {
                store.create_job(crate::storage::NewJob {
                    task_type: "research_campaign".into(),
                    workspace_dir: research_original,
                    model: None,
                    metadata: std::collections::BTreeMap::new(),
                    job_id: Some("research-planning-fixture".into()),
                    idempotency_key: None,
                    provider: None,
                    engine: None,
                    created_at: None,
                })?;
                store.complete_job("research-planning-fixture", raw_report, None, None)?;
                Ok(())
            })
            .await?;
        let mut decorated_research = research_report.clone();
        decorated_research["read_only"] = json!(true);
        decorated_research["deduplicated_existing_request"] = json!(true);
        let record = execute(
            &gateway,
            "flow-planning-test",
            &original,
            &external,
            "Change the value in src/lib.rs",
            Some(&decorated_research),
            &ModelSpec {
                model: "gpt-6.1-sol".into(),
                effort: "high".into(),
            },
            Instant::now() + std::time::Duration::from_secs(30),
        )
        .await?;
        assert!(
            record.mechanically_ready(),
            "{:?}",
            record.readiness_reasons
        );
        assert_eq!(record.artifact.tasks.len(), 2);
        assert_eq!(record.workspace_dir, original.canonicalize()?);
        assert_eq!(record.execution_workspace, external.canonicalize()?);
        let read = artifacts::read_plan(&gateway, &original, &record.plan_id)
            .await?
            .ok_or("planning artifact missing")?;
        assert_eq!(read.artifact_digest, record.artifact_digest);
        assert_eq!(
            record.research_digest,
            Some(digest_output(&research_report.to_string()))
        );
        assert_ne!(
            record.research_digest,
            Some(digest_output(&decorated_research.to_string()))
        );
        let foreign = root.path().join("foreign");
        std::fs::create_dir(&foreign)?;
        assert!(
            artifacts::read_plan(&gateway, &foreign, &record.plan_id)
                .await
                .is_err()
        );
        for fault in [
            "inner_workspace",
            "source_manifest",
            "flow_id",
            "artifact_digest",
            "snapshot_digest",
        ] {
            let flow_id = format!("corrupt-{fault}");
            let id = plan_id(&flow_id)?;
            let mut damaged = record.clone();
            damaged.flow_id = flow_id.clone();
            damaged.plan_id = id.clone();
            let mut value = serde_json::to_value(&damaged)?;
            match fault {
                "inner_workspace" => value["workspace_dir"] = json!(foreign),
                "source_manifest" => {
                    value["source_manifest_digest"] = json!("sha256:changed-source")
                }
                "flow_id" => value["flow_id"] = json!("foreign-flow"),
                "artifact_digest" => {
                    value["artifact"]["objective"] = json!("changed proposal body")
                }
                _ => {}
            }
            let snapshot_digest = if fault == "snapshot_digest" {
                "sha256:wrong".into()
            } else {
                digest_output(&value.to_string())
            };
            let metadata = std::collections::BTreeMap::from([
                ("snapshot_digest".into(), snapshot_digest),
                ("flow_id".into(), flow_id),
                (
                    "source_manifest_digest".into(),
                    record.source_manifest_digest.clone(),
                ),
                ("artifact_digest".into(), record.artifact_digest.clone()),
            ]);
            let owned_id = id.clone();
            let original = original.clone();
            gateway
                .database(move |store| {
                    store.create_job(crate::storage::NewJob {
                        task_type: "workspace_plan".into(),
                        workspace_dir: original,
                        model: None,
                        metadata,
                        job_id: Some(owned_id.clone()),
                        idempotency_key: None,
                        provider: None,
                        engine: None,
                        created_at: None,
                    })?;
                    store.complete_job(&owned_id, value, None, None)?;
                    Ok(())
                })
                .await?;
            assert!(
                artifacts::read_plan(&gateway, &record.workspace_dir, &id)
                    .await
                    .is_err(),
                "{fault}"
            );
        }
        gateway.shutdown().await?;
        Ok(())
    }
}
