//! Strict provider, engine, model, and execution-receipt contracts.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::LazyLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[serde(alias = "openai", alias = "codex")]
    OpenaiCodex,
    #[serde(alias = "google_antigravity", alias = "agy")]
    Antigravity,
    Google,
    Anthropic,
    LocalHttp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    CodexCli,
    AntigravityRpc,
    Http,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Accepted,
    RejectedPreDispatch,
    #[serde(rename = "succeeded")]
    Completed,
    Failed,
    TimedOutUnknown,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RouteErrorCode {
    #[serde(rename = "INVALID_REASONING_TIER")]
    InvalidReasoningTier,
    #[serde(rename = "UNSUPPORTED_ENGINE")]
    UnsupportedEngine,
    #[serde(rename = "PROVIDER_ENGINE_MISMATCH")]
    ProviderEngineMismatch,
    #[serde(rename = "MODEL_NOT_SUPPORTED_BY_ENGINE")]
    ModelNotSupportedByEngine,
    #[serde(rename = "UNRECOGNIZED_CODEX_MODEL")]
    UnrecognizedCodexModel,
    #[serde(rename = "CLAUDE_TIER_POLICY_VIOLATION")]
    ClaudeTierPolicyViolation,
    #[serde(rename = "INPUT_BUDGET_EXCEEDED")]
    InputBudgetExceeded,
    #[serde(rename = "ACTUAL_MODEL_MISMATCH")]
    ActualModelMismatch,
    #[serde(rename = "ANTIGRAVITY_CIRCUIT_OPEN")]
    AntigravityCircuitOpen,
    #[serde(rename = "ANTIGRAVITY_QUOTA_EXHAUSTED")]
    AntigravityQuotaExhausted,
    #[serde(rename = "DEPENDENCY_UNAVAILABLE")]
    DependencyUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteValidationError {
    pub error_code: RouteErrorCode,
    pub message: String,
    pub provider: Provider,
    pub engine: Engine,
    pub requested_model: String,
    pub reasoning_tier: String,
}

impl fmt::Display for RouteValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RouteValidationError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingDecision {
    pub valid: bool,
    pub provider: Provider,
    pub engine: Engine,
    pub requested_model: String,
    pub resolved_model: String,
    pub reasoning_tier: String,
    pub auth_context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub error_code: Option<RouteErrorCode>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalMcpOAuthWarning {
    pub external_mcp_server: String,
    pub error_type: String,
    pub message: String,
    pub affects_antigravity_rpc: bool,
    pub affects_codex_model_routing: bool,
    pub auto_reauth_triggered: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthDiagnostics {
    pub sanitized_stderr: String,
    pub isolated_external_mcp_oauth_warnings: Vec<ExternalMcpOAuthWarning>,
    pub antigravity_rpc_affected: bool,
    pub auto_credential_refresh_performed: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    #[default]
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRequest {
    pub provider: Provider,
    pub engine: Engine,
    pub model: String,
    pub effort: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Route {
    pub provider: Provider,
    pub engine: Engine,
    pub requested_model: String,
    pub resolved_model: String,
    pub effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub request_id: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default = "default_attempt_count")]
    pub attempt_count: u32,
    #[serde(default)]
    pub call_path: String,
    #[serde(default)]
    pub routing_decision: Option<RoutingDecision>,
    pub idempotency_key: Option<String>,
    pub provider: Provider,
    pub engine: Engine,
    pub requested_model: String,
    pub resolved_model: String,
    pub actual_model: Option<String>,
    pub identity_source: Option<String>,
    pub model_mismatch: bool,
    pub status: ReceiptStatus,
    pub outcome_state: String,
    pub output_digest: Option<String>,
    pub duration_ms: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_context_tokens: Option<u64>,
    /// Local process-group absence observed after owned child completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_processes_closed: Option<bool>,
    pub result_ref: String,
    pub credentials_redacted: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub error_code: Option<RouteErrorCode>,
    #[serde(default)]
    pub auth_diagnostics: AuthDiagnostics,
}

const fn default_attempt_count() -> u32 {
    1
}
pub fn close_receipt(receipt: &mut Receipt) {
    receipt.ended_at =
        Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
}

pub fn normalize_model(model: &str) -> Option<String> {
    let value = model
        .trim()
        .to_ascii_lowercase()
        .replace("gpt6.1", "gpt-6.1")
        .replace("gpt6", "gpt-6");
    let canonical = match value.as_str() {
        "argon" | "argon4" | "argon-4" | "gemini-4-argon" | "4-argon" | "argon-high" => {
            "gemini-4-argon-high"
        }
        "argon-medium" => "gemini-4-argon-medium",
        "argon-low" => "gemini-4-argon-low",
        "flash" | "gemini-4-flash" | "gemini-3-8-flash" | "gemini-3-8-flash-high" | "3.8-flash" => {
            "gemini-3.8-flash-high"
        }
        "opus"
        | "opus-medium"
        | "opus-5.5"
        | "opus-5-5"
        | "opus5.5"
        | "claude-opus"
        | "claude opus"
        | "claude-opus-5.5"
        | "claude-opus-5.5-medium" => "claude-opus-5-5-medium",
        "sonnet"
        | "sonnet-medium"
        | "sonnet-5.5"
        | "sonnet-5-5"
        | "sonnet5.5"
        | "claude"
        | "claude-sonnet"
        | "claude sonnet"
        | "claude-sonnet-5.5"
        | "claude-sonnet-5.5-medium" => "claude-sonnet-5-5-medium",
        "sonnet-high" => "claude-sonnet-5-5-high",
        "sonnet-low" => "claude-sonnet-5-5-low",
        "opus-high" => "claude-opus-5-5-high",
        "opus-low" => "claude-opus-5-5-low",
        "terra" | "6-terra" | "6 terra" | "gpt 6 terra" | "terra(6)" | "테라" | "테라(6)"
        | "6 테라" => "gpt-6-terra",
        "luna" | "6-luna" | "6 luna" | "gpt 6 luna" | "luna(6)" | "루나" | "루나(6)" | "6 루나" => {
            "gpt-6-luna"
        }
        "sol" | "6-sol" | "6 sol" | "gpt 6 sol" | "sol(6)" | "솔" | "솔(6)" | "6 솔" => {
            "gpt-6-sol"
        }
        "astra" | "6" | "6-astra" | "6 astra" | "gpt 6 astra" | "astra(6)" | "아스트라"
        | "아스트라(6)" | "6 아스트라" => "gpt-6-astra",
        "6-1-terra" | "6.1-terra" | "6-1 terra" | "6.1 terra" | "gpt 6.1 terra"
        | "gpt-6-1-terra" | "terra(6-1)" | "terra(6.1)" | "테라(6-1)" | "테라(6.1)"
        | "6-1 테라" | "6.1 테라" => "gpt-6.1-terra",
        "6-1" | "6.1" | "6-1-sol" | "6.1-sol" | "6-1 sol" | "6.1 sol" | "gpt 6.1 sol"
        | "gpt-6-1-sol" | "sol(6-1)" | "sol(6.1)" | "솔(6-1)" | "솔(6.1)" | "6-1 솔" | "6.1 솔" => {
            "gpt-6.1-sol"
        }
        "gpt-6-sol"
        | "gpt-6-luna"
        | "gpt-6-terra"
        | "gpt-6-astra"
        | "gpt-6.1-sol"
        | "gpt-6.1-terra"
        | "claude-sonnet-5-5-medium"
        | "claude-opus-5-5-medium"
        | "claude-sonnet-5-5-low"
        | "claude-sonnet-5-5-high"
        | "claude-opus-5-5-low"
        | "claude-opus-5-5-high"
        | "gemini-4-argon-high"
        | "gemini-4-argon-medium"
        | "gemini-4-argon-low"
        | "gemini-3.8-flash-high"
        | "gemini-3.8-flash-medium"
        | "gemini-3.8-flash-low" => value.as_str(),
        _ => return None,
    };
    Some(canonical.to_owned())
}

pub fn validate_route(request: &RouteRequest) -> crate::Result<Route> {
    let effort = request.effort.trim().to_ascii_lowercase();
    if !matches!(
        effort.as_str(),
        "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    ) {
        return Err(route_error(
            request,
            RouteErrorCode::InvalidReasoningTier,
            format!("Unsupported tier '{}'.", request.effort),
        )
        .into());
    }
    let provider_matches = matches!(
        (request.provider, request.engine),
        (Provider::OpenaiCodex, Engine::CodexCli)
            | (Provider::Antigravity, Engine::AntigravityRpc)
            | (Provider::LocalHttp, Engine::Http)
    );
    if !provider_matches {
        return Err(route_error(
            request,
            RouteErrorCode::ProviderEngineMismatch,
            "Provider and engine do not match.".to_owned(),
        )
        .into());
    }
    let Some(model) = normalize_model(&request.model) else {
        let lower = request.model.to_ascii_lowercase();
        let error_code = if request.engine == Engine::CodexCli
            && !lower.starts_with("gemini-")
            && !lower.starts_with("claude-")
        {
            RouteErrorCode::UnrecognizedCodexModel
        } else {
            RouteErrorCode::ModelNotSupportedByEngine
        };
        return Err(route_error(
            request,
            error_code,
            format!(
                "Model '{}' is unsupported on the selected engine.",
                request.model
            ),
        )
        .into());
    };
    let engine_accepts_model = match request.engine {
        Engine::CodexCli | Engine::Http => model.starts_with("gpt-"),
        Engine::AntigravityRpc => !model.starts_with("gpt-"),
    };
    if !engine_accepts_model {
        return Err(route_error(
            request,
            RouteErrorCode::ModelNotSupportedByEngine,
            format!(
                "Model '{}' is unsupported on the selected engine.",
                request.model
            ),
        )
        .into());
    }
    if model.starts_with("claude-") && (!model.ends_with("-medium") || effort != "medium") {
        return Err(route_error(
            request,
            RouteErrorCode::ClaudeTierPolicyViolation,
            format!(
                "Claude model '{}' violates Medium-only policy.",
                request.model
            ),
        )
        .into());
    }
    // Gemini's selected catalogue model owns its tier; generic caller effort
    // defaults do not create an unavailable "xhigh" RPC model.
    let effort = if request.engine == Engine::AntigravityRpc && model.starts_with("gemini-") {
        if model.ends_with("-low") {
            "low"
        } else if model.ends_with("-medium") {
            "medium"
        } else {
            "high"
        }
        .to_owned()
    } else {
        effort
    };
    Ok(Route {
        provider: request.provider,
        engine: request.engine,
        requested_model: request.model.clone(),
        resolved_model: model,
        effort,
    })
}

fn route_error(
    request: &RouteRequest,
    error_code: RouteErrorCode,
    message: String,
) -> RouteValidationError {
    RouteValidationError {
        error_code,
        message,
        provider: request.provider,
        engine: request.engine,
        requested_model: request.model.clone(),
        reasoning_tier: request.effort.clone(),
    }
}

pub fn route_validation_error<'a>(
    error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a RouteValidationError> {
    error.downcast_ref::<RouteValidationError>()
}

pub fn digest_output(output: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(output.as_bytes());
    format!("sha256:{:x}", hash.finalize())
}

pub fn new_receipt(route: &Route) -> Receipt {
    new_receipt_with_context(route, None, None)
}

pub fn new_receipt_with_context(
    route: &Route,
    request_id: Option<&str>,
    idempotency_key: Option<String>,
) -> Receipt {
    let request_id = request_id
        .filter(|v| !v.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("cdx-{}", &Uuid::new_v4().simple().to_string()[..10]));
    Receipt {
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ended_at: None,
        attempt_count: 1,
        call_path: match route.engine {
            Engine::CodexCli => "runtime::codex_cli",
            Engine::AntigravityRpc => "runtime::antigravity_local_rpc",
            Engine::Http => "http_runtime::chat_completions",
        }
        .into(),
        routing_decision: Some(RoutingDecision {
            valid: true,
            provider: route.provider,
            engine: route.engine,
            requested_model: route.requested_model.clone(),
            resolved_model: route.resolved_model.clone(),
            reasoning_tier: route.effort.clone(),
            auth_context: auth_context(route.engine).to_owned(),
            placeholder: None,
            error_code: None,
            error_message: None,
        }),
        result_ref: format!("job://{request_id}"),
        request_id,
        idempotency_key,
        provider: route.provider,
        engine: route.engine,
        requested_model: route.requested_model.clone(),
        resolved_model: route.resolved_model.clone(),
        actual_model: None,
        identity_source: None,
        model_mismatch: false,
        status: ReceiptStatus::Accepted,
        outcome_state: "in_flight".to_owned(),
        output_digest: None,
        duration_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
        context_window: None,
        current_context_tokens: None,
        owned_processes_closed: None,
        credentials_redacted: true,
        error: None,
        failure_reason: None,
        error_code: None,
        auth_diagnostics: AuthDiagnostics::default(),
    }
}

pub const fn auth_context(engine: Engine) -> &'static str {
    match engine {
        Engine::CodexCli => "codex_chatgpt_oauth",
        Engine::AntigravityRpc => "antigravity_local_csrf",
        Engine::Http => "http_bearer",
    }
}

pub fn rejected_receipt(
    rejection: &RouteValidationError,
    request_id: Option<&str>,
    idempotency_key: Option<String>,
) -> Receipt {
    let request_id = request_id
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("cdx-{}", &Uuid::new_v4().simple().to_string()[..10]));
    let message = rejection.message.clone();
    let mut receipt = Receipt {
        request_id: request_id.clone(),
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ended_at: None,
        attempt_count: 1,
        call_path: "routing::validate_route".to_owned(),
        routing_decision: Some(RoutingDecision {
            valid: false,
            provider: rejection.provider,
            engine: rejection.engine,
            requested_model: rejection.requested_model.clone(),
            resolved_model: String::new(),
            reasoning_tier: rejection.reasoning_tier.clone(),
            auth_context: auth_context(rejection.engine).to_owned(),
            placeholder: None,
            error_code: Some(rejection.error_code),
            error_message: Some(message.clone()),
        }),
        idempotency_key,
        provider: rejection.provider,
        engine: rejection.engine,
        requested_model: rejection.requested_model.clone(),
        resolved_model: String::new(),
        actual_model: None,
        identity_source: None,
        model_mismatch: false,
        status: ReceiptStatus::RejectedPreDispatch,
        outcome_state: "confirmed_failure".to_owned(),
        output_digest: None,
        duration_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
        context_window: None,
        current_context_tokens: None,
        owned_processes_closed: None,
        result_ref: format!("job://{request_id}"),
        credentials_redacted: true,
        error: Some(message.clone()),
        failure_reason: Some(message),
        error_code: Some(rejection.error_code),
        auth_diagnostics: AuthDiagnostics::default(),
    };
    close_receipt(&mut receipt);
    receipt
}

pub fn redact_sensitive_text(text: &str) -> String {
    static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (
                r#"(?i)(--csrf_token(?:=|\s+))([^\s"']+)"#,
                "$1***REDACTED***",
            ),
            (
                r#"(?i)(--host_bridge_token(?:=|\s+))([^\s"']+)"#,
                "$1***REDACTED***",
            ),
            (r"(?i)(Bearer\s+)([A-Za-z0-9._\-]+)", "$1***REDACTED***"),
            (
                r#"(?i)((?:api_key|access_token|refresh_token|client_secret|csrf_token|password)["'=:\s]+)([^\s"',}]+)"#,
                "$1***REDACTED***",
            ),
            (r"\bsk-[A-Za-z0-9_\-]{12,}\b", "sk-***REDACTED***"),
        ]
        .into_iter()
        .filter_map(|(pattern, replacement)| {
            Regex::new(pattern).ok().map(|matcher| (matcher, replacement))
        })
        .collect()
    });
    PATTERNS
        .iter()
        .fold(text.to_owned(), |current, (matcher, replacement)| {
            matcher.replace_all(&current, *replacement).into_owned()
        })
}

pub fn classify_stderr_auth(stderr: &str) -> AuthDiagnostics {
    static SERVER_MATCHER: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"for server ([A-Za-z0-9_\-]+):").ok());
    let redacted = redact_sensitive_text(stderr);
    let mut warnings = Vec::new();
    let mut ordinary = Vec::new();
    for line in redacted
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.contains("codex_rmcp_client::oauth") || line.contains("invalid_grant") {
            let external_mcp_server = SERVER_MATCHER
                .as_ref()
                .and_then(|matcher| matcher.captures(line))
                .and_then(|captures| captures.get(1))
                .map_or_else(
                    || "unknown_external_mcp".to_owned(),
                    |value| value.as_str().to_owned(),
                );
            warnings.push(ExternalMcpOAuthWarning {
                external_mcp_server,
                error_type: "external_mcp_oauth_invalid_grant".to_owned(),
                message: line.to_owned(),
                affects_antigravity_rpc: false,
                affects_codex_model_routing: false,
                auto_reauth_triggered: false,
            });
        } else {
            ordinary.push(line.to_owned());
        }
    }
    AuthDiagnostics {
        sanitized_stderr: ordinary.join("\n"),
        isolated_external_mcp_oauth_warnings: warnings,
        antigravity_rpc_affected: false,
        auto_credential_refresh_performed: false,
    }
}

#[cfg(test)]
mod tests;
