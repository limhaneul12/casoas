//! Compatibility DTOs for the established Python MCP wire surface.

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use crate::routing::{Engine, Receipt, ReceiptStatus, RouteErrorCode};
use crate::runtime::ExecutionResult;
use crate::storage::{JobRecord, JobStatus};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BudgetEnvelope {
    pub max_input_bytes: usize,
    pub actual_input_bytes: usize,
    pub max_input_tokens: usize,
    pub estimated_input_tokens: usize,
    pub timeout_seconds: f64,
    pub elapsed_seconds: f64,
    pub input_budget_exceeded: bool,
    pub time_budget_exceeded: bool,
    pub termination_reason: String,
}

impl BudgetEnvelope {
    pub(crate) fn evaluate(prompt: &str, max_input_bytes: usize, timeout_seconds: f64) -> Self {
        let actual_input_bytes = prompt.len();
        let input_budget_exceeded = actual_input_bytes > max_input_bytes;
        Self {
            max_input_bytes,
            actual_input_bytes,
            max_input_tokens: max_input_bytes / 4,
            estimated_input_tokens: actual_input_bytes.div_ceil(4),
            timeout_seconds,
            elapsed_seconds: 0.0,
            input_budget_exceeded,
            time_budget_exceeded: false,
            termination_reason: if input_budget_exceeded {
                "input_budget_exceeded".to_owned()
            } else {
                "completed".to_owned()
            },
        }
    }
}

pub(crate) fn job_value(record: &JobRecord) -> crate::Result<Value> {
    let mut value = serde_json::to_value(record)?;
    value["request_id"] = Value::String(record.job_id.clone());
    let ended = record
        .completed_at
        .unwrap_or_else(|| Utc::now().timestamp_millis() as f64 / 1000.0);
    value["elapsed_seconds"] = json!((ended - record.created_at).max(0.0));
    Ok(value)
}

pub(crate) fn job_status_response(record: Option<JobRecord>, lookup: &str) -> crate::Result<Value> {
    let Some(record) = record else {
        return Ok(json!({"ok":false,"error":format!("Job '{lookup}' not found.")}));
    };
    Ok(json!({
        "ok": true,
        "job": job_value(&record)?,
        "outcome_state": record.outcome_state,
        "cancel_requested": record.cancel_requested,
    }))
}

pub(crate) fn job_list_response(
    records: Vec<JobRecord>,
    workspace: Option<String>,
) -> crate::Result<Value> {
    let jobs = records
        .iter()
        .map(job_value)
        .collect::<crate::Result<Vec<_>>>()?;
    Ok(json!({"ok":true,"workspace_filter":workspace,"jobs":jobs}))
}

pub(crate) fn deduplicated_response(record: &JobRecord) -> crate::Result<Value> {
    Ok(json!({
        "ok": true,
        "async_dispatched": false,
        "deduplicated_existing_request": true,
        "request_id": record.job_id,
        "job_id": record.job_id,
        "status": record.status,
        "outcome_state": record.outcome_state,
        "attempt_count": record.attempt_count,
        "task_type": record.task_type,
        "receipt": record.receipt,
        "job": job_value(record)?,
    }))
}

pub(crate) fn background_response(record: &JobRecord, deduplicated: bool) -> Value {
    let pre_dispatch_terminal = matches!(
        record.status,
        JobStatus::RejectedPreDispatch | JobStatus::InputBudgetExceeded
    );
    json!({
        "ok": true,
        "async_dispatched": !deduplicated && !pre_dispatch_terminal,
        "deduplicated_existing_request": deduplicated,
        "request_id": record.job_id,
        "job_id": record.job_id,
        "status": record.status,
        "outcome_state": record.outcome_state,
        "attempt_count": record.attempt_count,
        "task_type": record.task_type,
        "receipt": record.receipt,
        "result_ref": format!("job://{}", record.job_id),
        "message": if deduplicated {
            "Existing durable request returned without redispatch."
        } else {
            "Background execution accepted and durably registered."
        },
    })
}

pub(crate) fn rejection_response(
    receipt: &Receipt,
    error_code: RouteErrorCode,
    error: &str,
) -> crate::Result<Value> {
    Ok(json!({
        "ok": false,
        "status": "rejected_pre_dispatch",
        "outcome_state": "confirmed_failure",
        "request_id": receipt.request_id,
        "job_id": receipt.request_id,
        "provider": receipt.provider,
        "engine": receipt.engine,
        "requested_model": receipt.requested_model,
        "error_code": error_code,
        "error": error,
        "receipt": receipt,
    }))
}

pub(crate) fn budget_response(
    receipt: &Receipt,
    budget: &BudgetEnvelope,
    error: &str,
) -> crate::Result<Value> {
    Ok(json!({
        "ok": false,
        "status": "failed",
        "outcome_state": "confirmed_failure",
        "request_id": receipt.request_id,
        "job_id": receipt.request_id,
        "provider": receipt.provider,
        "engine": receipt.engine,
        "error_code": "INPUT_BUDGET_EXCEEDED",
        "error": error,
        "receipt": receipt,
        "budget": budget,
    }))
}

pub(crate) fn verified_at_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn model_execution_response(
    execution: &ExecutionResult,
    workspace_dir: &str,
    budget: &BudgetEnvelope,
    schema_valid: Option<bool>,
    evidence_scope: Value,
) -> crate::Result<Value> {
    let receipt = &execution.receipt;
    let ok = receipt.status == ReceiptStatus::Completed;
    let actual_model = receipt
        .actual_model
        .clone()
        .unwrap_or_else(|| receipt.resolved_model.clone());
    let data = execution
        .structured_output
        .clone()
        .unwrap_or_else(|| Value::String(execution.output.clone()));
    let auth_boundary = match receipt.engine {
        Engine::CodexCli => json!({
            "auth_context":"codex_chatgpt_oauth",
            "cli_binary_ready":true,
            "independent_of_antigravity_rpc":true,
            "auto_reauth_enabled":false,
            "isolated_external_mcp_oauth_warnings":receipt.auth_diagnostics.isolated_external_mcp_oauth_warnings,
        }),
        Engine::AntigravityRpc => json!({
            "auth_context":"antigravity_local_csrf",
            "antigravity_rpc_connected":true,
            "csrf_token_redacted":true,
            "independent_of_codex_cli_oauth":true,
        }),
        Engine::Http => json!({
            "auth_context":"http_bearer",
            "credentials_redacted":true,
        }),
    };
    let mut response = json!({
        "ok":ok,
        "status":receipt.status,
        "outcome_state":receipt.outcome_state,
        "request_id":receipt.request_id,
        "job_id":receipt.request_id,
        "provider":receipt.provider,
        "engine":receipt.engine,
        "model":actual_model,
        "model_id":actual_model,
        "requested_model":receipt.requested_model,
        "placeholder":execution.placeholder,
        "duration_seconds":receipt.duration_ms as f64 / 1000.0,
        "exit_code":execution.exit_code,
        "session_id":execution.session_id,
        "workspace_dir":workspace_dir,
        "data":data,
        "structured_output":execution.structured_output,
        "raw_output":execution.output,
        "stderr":if ok { String::new() } else { receipt.auth_diagnostics.sanitized_stderr.clone() },
        "receipt":receipt,
        "budget":budget,
        "auth_boundary":auth_boundary,
        "schema_valid":schema_valid,
        "evidence_scope":evidence_scope,
        // Preserve the native Rust representation for newer clients.
        "result":execution,
    });
    if let Some(error) = receipt.failure_reason.as_ref().or(receipt.error.as_ref()) {
        response["error"] = Value::String(error.clone());
    }
    if let Some(error_code) = receipt.error_code {
        response["error_code"] = serde_json::to_value(error_code)?;
    }
    Ok(response)
}

pub(crate) fn prompt_prefix(name: &str, args: &Value) -> String {
    match name {
        "codex_adversarial_audit" => format!(
            "[ROLE: CODEX ADVERSARIAL RED-TEAM AUDITOR]\nAct as an uncompromising Devil's Advocate. Test concrete failure modes, not style. Perform a pure static audit using only supplied source and CodeGraph context; do not invoke shell commands or external tools.\nArchitect rationale: {}\nAttack vectors: {}\nTarget code or diff:\n{}",
            args["design_rationale"]
                .as_str()
                .unwrap_or("Infer invariants from supplied code."),
            args["attack_vectors"],
            args["target_code_or_diff"].as_str().unwrap_or_default(),
        ),
        "codex_targeted_synthesize" => format!(
            "[ROLE: CODEX HIGH-PRECISION SYNTHESIS ENGINE]\nProduce complete production code with zero placeholders.\nLanguage: {}\nSignature: {}\nContract: {}\nContext types: {}\nUnit tests requested: {}",
            args["language"].as_str().unwrap_or("rust"),
            args["signature"].as_str().unwrap_or_default(),
            args["contract_and_constraints"]
                .as_str()
                .unwrap_or_default(),
            args["context_types"].as_str().unwrap_or_default(),
            args["include_unit_tests"].as_bool().unwrap_or(true),
        ),
        "codex_differential_implementations" => {
            let defaults = json!([
                "latency_and_throughput",
                "memory_footprint",
                "maintainability_and_clarity"
            ]);
            let axes = if args["optimization_axes"]
                .as_array()
                .is_some_and(|axes| !axes.is_empty())
            {
                &args["optimization_axes"]
            } else {
                &defaults
            };
            format!(
                "[ROLE: CODEX DIFFERENTIAL ALGORITHM EXPLORER]\nCreate one genuinely distinct candidate per requested optimization axis, including pros, cons, complexity, and an Argon selection guide.\nProblem: {}\nAxes: {}\nContext: {}",
                args["problem_spec"].as_str().unwrap_or_default(),
                axes,
                args["context_types"].as_str().unwrap_or_default(),
            )
        }
        "codex_rescue_sandbox" => format!(
            "[ROLE: CODEX DEEP RESCUE & ROOT-CAUSE SPECIALIST]\nExplain why previous attempts failed, identify the true root cause, produce a minimal patch, and list exact verification commands.\nTrace: {}\nPrevious attempts: {}\nReproduction: {}\nTarget: {}",
            args["error_trace"].as_str().unwrap_or_default(),
            args["previous_attempts"].as_str().unwrap_or_default(),
            args["reproduction_command"].as_str().unwrap_or_default(),
            args["target_code"].as_str().unwrap_or_default(),
        ),
        "claude_code_craft" => {
            let model = args["claude_model"].as_str().unwrap_or_default();
            let role = if model.contains("opus") {
                "CLAUDE OPUS 5.5 DEEP ARCHITECTURAL & SURGICAL CODE CRAFTSMAN"
            } else {
                "CLAUDE SONNET SURGICAL CODE CRAFTSMAN & REFACTORING SPECIALIST"
            };
            format!(
                "[ROLE: {role}]\nImplement the requested work completely while preserving repository rules and preparing a handoff for Codex audit.\nTask: {}\nLanguage: {}\nInvariants: {}\nTarget context: {}",
                args["task_spec"].as_str().unwrap_or_default(),
                args["language"].as_str().unwrap_or("rust"),
                args["invariants_and_rules"].as_str().unwrap_or_default(),
                args["target_code_or_context"].as_str().unwrap_or_default(),
            )
        }
        _ => format!("[ROLE: HETERARCHY EXECUTION ADAPTER]\nTool: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Provider, Route};
    use crate::storage::{JobStatus, OutcomeState};
    use std::collections::BTreeMap;

    fn record() -> JobRecord {
        JobRecord {
            job_id: "cdx-contract".to_owned(),
            task_type: "audit".to_owned(),
            status: JobStatus::Completed,
            created_at: 10.0,
            workspace_dir: "/tmp/work".to_owned(),
            model: Some("gpt-6.1-sol".to_owned()),
            session_id: None,
            completed_at: Some(12.5),
            result: Some(json!({"ok":true})),
            error: None,
            metadata: BTreeMap::new(),
            outcome_state: OutcomeState::ConfirmedSuccess,
            idempotency_key: Some("same".to_owned()),
            provider: Some("openai_codex".to_owned()),
            engine: Some("codex_cli".to_owned()),
            attempt_count: 2,
            cancel_requested: false,
            cancelled_at: None,
            receipt: Some(json!({"status":"succeeded"})),
        }
    }

    #[test]
    fn compatibility_job_and_background_shapes_are_derived() -> crate::Result<()> {
        let record = record();
        let status = job_status_response(Some(record.clone()), "unused")?;
        assert_eq!(status["job"]["request_id"], "cdx-contract");
        assert_eq!(status["job"]["elapsed_seconds"], 2.5);
        assert_eq!(status["outcome_state"], "confirmed_success");
        assert_eq!(status["cancel_requested"], false);
        let dedup = deduplicated_response(&record)?;
        assert_eq!(dedup["deduplicated_existing_request"], true);
        assert_eq!(dedup["attempt_count"], 2);
        let background = background_response(&record, false);
        assert_eq!(background["async_dispatched"], true);
        assert_eq!(background["request_id"], "cdx-contract");
        assert_eq!(
            job_status_response(None, "missing")?["error"],
            "Job 'missing' not found."
        );
        Ok(())
    }

    #[test]
    fn differential_defaults_match_python_for_absent_null_and_empty_axes() {
        for args in [
            json!({"problem_spec":"queue"}),
            json!({"problem_spec":"queue", "optimization_axes":null}),
            json!({"problem_spec":"queue", "optimization_axes":[]}),
        ] {
            let prompt = prompt_prefix("codex_differential_implementations", &args);
            for axis in [
                "latency_and_throughput",
                "memory_footprint",
                "maintainability_and_clarity",
            ] {
                assert!(prompt.contains(axis));
            }
        }
        let custom = prompt_prefix(
            "codex_differential_implementations",
            &json!({"problem_spec":"queue", "optimization_axes":["energy"]}),
        );
        assert!(custom.contains("energy"));
        assert!(!custom.contains("memory_footprint"));
    }

    #[test]
    fn budget_and_specialized_prompts_preserve_contracts() {
        let budget = BudgetEnvelope::evaluate("12345", 4, 30.0);
        assert!(budget.input_budget_exceeded);
        assert_eq!(budget.actual_input_bytes, 5);
        assert_eq!(budget.termination_reason, "input_budget_exceeded");
        assert!(prompt_prefix("codex_adversarial_audit", &json!({
            "target_code_or_diff":"fn f(){}", "design_rationale":"atomic", "attack_vectors":[]
        })).contains("ADVERSARIAL RED-TEAM"));
        assert!(prompt_prefix("codex_targeted_synthesize", &json!({
            "language":"rust", "signature":"fn f()", "contract_and_constraints":"total", "context_types":"", "include_unit_tests":true
        })).contains("HIGH-PRECISION SYNTHESIS"));
        assert!(
            prompt_prefix(
                "codex_differential_implementations",
                &json!({
                    "problem_spec":"queue", "optimization_axes":["latency"], "context_types":""
                })
            )
            .contains("DIFFERENTIAL ALGORITHM")
        );
        assert!(prompt_prefix("codex_rescue_sandbox", &json!({
            "error_trace":"boom", "previous_attempts":"none", "reproduction_command":"cargo test", "target_code":"fn f(){}"
        })).contains("DEEP RESCUE"));
        assert!(prompt_prefix("claude_code_craft", &json!({
            "claude_model":"claude-opus-5-5-medium", "task_spec":"refactor", "language":"rust", "invariants_and_rules":"safe", "target_code_or_context":"code"
        })).contains("CLAUDE OPUS 5.5"));
    }

    #[test]
    fn model_success_wire_flattens_legacy_fields_and_preserves_native_result() -> crate::Result<()>
    {
        let route = Route {
            provider: Provider::Antigravity,
            engine: Engine::AntigravityRpc,
            requested_model: "sonnet-5.5".to_owned(),
            resolved_model: "claude-sonnet-5-5-medium".to_owned(),
            effort: "medium".to_owned(),
        };
        let mut receipt = crate::routing::new_receipt(&route);
        receipt.status = ReceiptStatus::Completed;
        receipt.outcome_state = "confirmed_success".to_owned();
        receipt.actual_model = Some("claude-sonnet-5-5-medium".to_owned());
        receipt.duration_ms = 1250;
        let execution = ExecutionResult {
            receipt,
            output: "{\"verdict\":\"PASS\"}".to_owned(),
            structured_output: Some(json!({"verdict":"PASS"})),
            session_id: None,
            exit_code: Some(0),
            placeholder: Some("MODEL_PLACEHOLDER_M404".to_owned()),
        };
        let response = model_execution_response(
            &execution,
            "/tmp/workspace",
            &BudgetEnvelope::evaluate("review", 256_000, 180.0),
            Some(true),
            json!({"scope_digest":"sha256:scope"}),
        )?;
        assert_eq!(response["status"], "succeeded");
        assert_eq!(response["model_id"], "claude-sonnet-5-5-medium");
        assert_eq!(response["requested_model"], "sonnet-5.5");
        assert_eq!(response["placeholder"], "MODEL_PLACEHOLDER_M404");
        assert_eq!(response["duration_seconds"], 1.25);
        assert_eq!(response["auth_boundary"]["csrf_token_redacted"], true);
        assert_eq!(response["schema_valid"], true);
        assert_eq!(response["result"]["placeholder"], "MODEL_PLACEHOLDER_M404");
        Ok(())
    }
}
