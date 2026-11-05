use super::*;

fn request(model: &str, provider: Provider, engine: Engine, effort: &str) -> RouteRequest {
    RouteRequest {
        provider,
        engine,
        model: model.to_owned(),
        effort: effort.to_owned(),
        prompt: "review".to_owned(),
    }
}

#[test]
fn historical_receipt_does_not_invent_local_process_closure() -> crate::Result<()> {
    let route = validate_route(&request(
        "gpt-6.1-sol",
        Provider::OpenaiCodex,
        Engine::CodexCli,
        "high",
    ))?;
    let receipt = new_receipt(&route);
    let legacy = serde_json::to_value(&receipt)?;
    assert!(legacy.get("owned_processes_closed").is_none());
    let decoded: Receipt = serde_json::from_value(legacy)?;
    assert_eq!(decoded.owned_processes_closed, None);
    Ok(())
}

#[test]
fn historical_interactive_aliases_preserve_model_family_and_medium_gate() -> crate::Result<()> {
    for (alias, model) in [
        ("sonnet", "claude-sonnet-5-5-medium"),
        ("claude-sonnet-5.5", "claude-sonnet-5-5-medium"),
        ("opus", "claude-opus-5-5-medium"),
        ("opus5.5", "claude-opus-5-5-medium"),
        ("6", "gpt-6-astra"),
        ("6.1", "gpt-6.1-sol"),
        ("6-1 솔", "gpt-6.1-sol"),
        ("테라(6.1)", "gpt-6.1-terra"),
        ("루나", "gpt-6-luna"),
        ("6 sol", "gpt-6-sol"),
        ("gpt6.1-terra", "gpt-6.1-terra"),
        ("argon-low", "gemini-4-argon-low"),
    ] {
        assert_eq!(normalize_model(alias).as_deref(), Some(model));
    }
    assert!(
        validate_route(&request(
            "sonnet-high",
            Provider::Antigravity,
            Engine::AntigravityCli,
            "high"
        ))
        .is_err()
    );
    assert!(normalize_model("gpt-5.6-sol").is_none());
    for alias in ["openai", "codex"] {
        assert_eq!(
            serde_json::from_value::<Provider>(serde_json::json!(alias))?,
            Provider::OpenaiCodex
        );
    }
    for alias in ["google_antigravity", "agy"] {
        assert_eq!(
            serde_json::from_value::<Provider>(serde_json::json!(alias))?,
            Provider::Antigravity
        );
    }
    Ok(())
}

#[test]
fn flash_aliases_resolve_to_catalog_models_without_changing_requested_identity() -> crate::Result<()>
{
    for alias in ["flash", "gemini-4-flash", "gemini-3.8-flash-high"] {
        let route = validate_route(&request(
            alias,
            Provider::Antigravity,
            Engine::AntigravityCli,
            "high",
        ))?;
        assert_eq!(route.resolved_model, "gemini-3.8-flash-high");
        assert_eq!(route.requested_model, alias);
    }
    for (model, effort) in [
        ("gemini-3.8-flash-medium", "medium"),
        ("gemini-3.8-flash-low", "low"),
        ("gemini-4-argon-low", "low"),
    ] {
        let route = validate_route(&request(
            model,
            Provider::Antigravity,
            Engine::AntigravityCli,
            "xhigh",
        ))?;
        assert_eq!(route.resolved_model, model);
        assert_eq!(route.effort, effort);
    }
    assert!(
        validate_route(&request(
            "gpt-6.1-sol",
            Provider::OpenaiCodex,
            Engine::CodexCli,
            "minimal"
        ))
        .is_err()
    );
    Ok(())
}

#[test]
fn validates_exact_engine_and_tier_contracts() {
    assert!(
        validate_route(&request(
            "argon4",
            Provider::Antigravity,
            Engine::AntigravityCli,
            "high"
        ))
        .is_ok()
    );
    assert!(
        validate_route(&request(
            "opus-5.5",
            Provider::Antigravity,
            Engine::AntigravityCli,
            "medium"
        ))
        .is_ok()
    );
    assert!(
        validate_route(&request(
            "claude-opus-5-5-medium",
            Provider::Antigravity,
            Engine::AntigravityCli,
            "high"
        ))
        .is_err()
    );
    assert!(
        validate_route(&request(
            "gemini-4-argon-high",
            Provider::OpenaiCodex,
            Engine::CodexCli,
            "high"
        ))
        .is_err()
    );
    assert!(
        validate_route(&request(
            "gpt-5.4",
            Provider::OpenaiCodex,
            Engine::CodexCli,
            "xhigh"
        ))
        .is_err()
    );
}

#[test]
fn route_rejections_keep_exact_machine_codes() {
    let mismatch = validate_route(&request(
        "gpt-6.1-sol",
        Provider::Antigravity,
        Engine::CodexCli,
        "xhigh",
    ));
    assert_eq!(
        mismatch.err().and_then(|error| {
            route_validation_error(error.as_ref()).map(|route| route.error_code)
        }),
        Some(RouteErrorCode::ProviderEngineMismatch)
    );
    let wrong_engine = validate_route(&request(
        "gemini-4-argon-high",
        Provider::OpenaiCodex,
        Engine::CodexCli,
        "high",
    ));
    assert_eq!(
        wrong_engine.err().and_then(|error| {
            route_validation_error(error.as_ref()).map(|route| route.error_code)
        }),
        Some(RouteErrorCode::ModelNotSupportedByEngine)
    );
    let claude_tier = validate_route(&request(
        "claude-opus-5-5-high",
        Provider::Antigravity,
        Engine::AntigravityCli,
        "high",
    ));
    assert_eq!(
        claude_tier.err().and_then(|error| {
            route_validation_error(error.as_ref()).map(|route| route.error_code)
        }),
        Some(RouteErrorCode::ClaudeTierPolicyViolation)
    );
}

#[test]
fn auth_classifier_redacts_and_isolates_external_oauth() {
    let diagnostics = classify_stderr_auth(
        "codex_rmcp_client::oauth invalid_grant for server docs: Bearer abc.secret\n\
         primary failure api_key='topsecret' sk-1234567890abcdef",
    );
    assert_eq!(diagnostics.isolated_external_mcp_oauth_warnings.len(), 1);
    let warning = &diagnostics.isolated_external_mcp_oauth_warnings[0];
    assert_eq!(warning.external_mcp_server, "docs");
    assert!(!warning.affects_antigravity_rpc);
    assert!(!warning.affects_codex_model_routing);
    assert!(!warning.auto_reauth_triggered);
    assert!(!diagnostics.antigravity_rpc_affected);
    assert!(!diagnostics.auto_credential_refresh_performed);
    assert!(!diagnostics.sanitized_stderr.contains("topsecret"));
    assert!(!diagnostics.sanitized_stderr.contains("1234567890abcdef"));
    assert!(diagnostics.sanitized_stderr.contains("***REDACTED***"));
}

#[test]
fn historical_rpc_engine_decodes_but_cannot_start_new_execution() -> crate::Result<()> {
    let engine: Engine = serde_json::from_str("\"antigravity_rpc\"")?;
    assert_eq!(engine, Engine::AntigravityRpc);
    assert_eq!(
        serde_json::from_str::<Engine>("\"antigravity_local_rpc\"")?,
        engine
    );
    let rejected = validate_route(&RouteRequest {
        provider: Provider::Antigravity,
        engine,
        model: "gemini-4-argon-high".into(),
        effort: "high".into(),
        prompt: "old input".into(),
    });
    assert!(rejected.is_err());
    Ok(())
}
