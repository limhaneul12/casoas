//! Explicit OpenAI-compatible HTTP completion execution.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::routing::{
    Engine, ReceiptStatus, RouteRequest, digest_output, new_receipt_with_context, validate_route,
};
use crate::runtime::{ExecutionOptions, ExecutionResult};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8787/v1";
const DEFAULT_API_KEY: &str = "headroom-local";
const SYSTEM_PROMPT: &str = "You are OpenAI Codex collaborating with Gemini 4 Argon (Antigravity) in a Heterarchy architecture. Respond with high technical precision.";

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct JsonSchemaSpec<'a> {
    name: &'a str,
    strict: bool,
    schema: &'a Value,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ResponseFormat<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    json_schema: JsonSchemaSpec<'a>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat<'a>>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    model: String,
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default, alias = "input_tokens")]
    prompt_tokens: u64,
    #[serde(default, alias = "output_tokens")]
    completion_tokens: u64,
}

/// Executes one explicitly selected OpenAI-compatible HTTP completion request.
pub async fn execute(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
    schema: Option<&Value>,
) -> crate::Result<ExecutionResult> {
    let base_url = std::env::var("CODEX_BASE_URL")
        .or_else(|_| std::env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| DEFAULT_API_KEY.to_owned());
    let client = Client::builder().timeout(limit).build()?;
    execute_with_client(
        request, limit, options, schema, &client, &base_url, &api_key,
    )
    .await
}

async fn execute_with_client(
    request: RouteRequest,
    limit: Duration,
    options: &ExecutionOptions,
    schema: Option<&Value>,
    client: &Client,
    base_url: &str,
    api_key: &str,
) -> crate::Result<ExecutionResult> {
    if limit.is_zero() {
        return Err("HTTP timeout must be greater than zero".into());
    }
    if request.prompt.len() > options.max_input_bytes {
        return Err("prompt exceeds max_input_bytes".into());
    }
    let route = validate_route(&request)?;
    if route.engine != Engine::Http {
        return Err("route is not HTTP".into());
    }
    let mut receipt = new_receipt_with_context(
        &route,
        options.request_id.as_deref(),
        options.idempotency_key.clone(),
    );
    let started = Instant::now();
    let schema_name = "hcodex_response";
    let payload = ChatRequest {
        model: &route.resolved_model,
        messages: [
            ChatMessage {
                role: "system",
                content: SYSTEM_PROMPT,
            },
            ChatMessage {
                role: "user",
                content: &request.prompt,
            },
        ],
        response_format: schema.map(|value| ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchemaSpec {
                name: schema_name,
                strict: true,
                schema: value,
            },
        }),
    };
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await;
    receipt.duration_ms = millis(started.elapsed());
    let response = match response {
        Ok(response) => response,
        Err(error) if error.is_timeout() || started.elapsed() >= limit => {
            receipt.status = ReceiptStatus::TimedOutUnknown;
            receipt.outcome_state = "outcome_unknown".to_owned();
            receipt.error =
                Some("HTTP completion timed out; provider outcome is unknown".to_owned());
            return Ok(empty_result(receipt));
        }
        Err(error) => {
            receipt.status = ReceiptStatus::Failed;
            receipt.outcome_state = "confirmed_failure".to_owned();
            receipt.error = Some(redacted_error(&error.to_string(), api_key));
            return Ok(empty_result(receipt));
        }
    };
    if !response.status().is_success() {
        receipt.status = ReceiptStatus::Failed;
        receipt.outcome_state = "confirmed_failure".to_owned();
        receipt.error = Some(format!(
            "HTTP completion failed with status {}",
            response.status()
        ));
        return Ok(empty_result(receipt));
    }
    let body = match response.json::<ChatResponse>().await {
        Ok(body) => body,
        Err(error) if error.is_timeout() || started.elapsed() >= limit => {
            receipt.duration_ms = millis(started.elapsed());
            receipt.status = ReceiptStatus::TimedOutUnknown;
            receipt.outcome_state = "outcome_unknown".to_owned();
            receipt.error =
                Some("HTTP completion body timed out; provider outcome is unknown".to_owned());
            return Ok(empty_result(receipt));
        }
        Err(error) => {
            receipt.status = ReceiptStatus::Failed;
            receipt.outcome_state = "confirmed_failure".to_owned();
            receipt.error = Some(format!("invalid HTTP completion response: {error}"));
            return Ok(empty_result(receipt));
        }
    };
    receipt.duration_ms = millis(started.elapsed());
    receipt.actual_model = Some(body.model.clone());
    receipt.model_mismatch = body.model != route.resolved_model;
    receipt.tokens_in = body.usage.prompt_tokens;
    receipt.tokens_out = body.usage.completion_tokens;
    let Some(choice) = body.choices.into_iter().next() else {
        receipt.status = ReceiptStatus::Failed;
        receipt.outcome_state = "confirmed_failure".to_owned();
        receipt.error = Some("HTTP completion response contains no choices".to_owned());
        return Ok(empty_result(receipt));
    };
    let output = choice.message.content;
    receipt.output_digest = Some(digest_output(&output));
    if receipt.model_mismatch {
        receipt.status = ReceiptStatus::Failed;
        receipt.outcome_state = "confirmed_failure".to_owned();
        receipt.error = Some(format!(
            "actual model '{}' does not match requested model '{}'",
            body.model, route.resolved_model
        ));
        crate::routing::close_receipt(&mut receipt);
        return Ok(ExecutionResult {
            receipt,
            placeholder: None,
            output,
            structured_output: None,
            session_id: None,
            exit_code: None,
        });
    }
    let structured_output = if let Some(output_schema) = schema {
        let parsed: Value = match serde_json::from_str(&output) {
            Ok(value) => value,
            Err(error) => {
                receipt.status = ReceiptStatus::Failed;
                receipt.outcome_state = "confirmed_failure".to_owned();
                receipt.error = Some(format!("structured HTTP output is not JSON: {error}"));
                crate::routing::close_receipt(&mut receipt);
                return Ok(ExecutionResult {
                    receipt,
                    placeholder: None,
                    output,
                    structured_output: None,
                    session_id: None,
                    exit_code: None,
                });
            }
        };
        if let Err(error) = crate::catalog::validate_schema(&parsed, output_schema) {
            receipt.status = ReceiptStatus::Failed;
            receipt.outcome_state = "confirmed_failure".to_owned();
            receipt.error = Some(format!(
                "structured HTTP output failed schema validation: {error}"
            ));
            crate::routing::close_receipt(&mut receipt);
            return Ok(ExecutionResult {
                receipt,
                placeholder: None,
                output,
                structured_output: Some(parsed),
                session_id: None,
                exit_code: None,
            });
        }
        Some(parsed)
    } else {
        None
    };
    receipt.status = ReceiptStatus::Completed;
    receipt.outcome_state = "confirmed_success".to_owned();
    crate::routing::close_receipt(&mut receipt);
    Ok(ExecutionResult {
        receipt,
        placeholder: None,
        output,
        structured_output,
        session_id: None,
        exit_code: Some(0),
    })
}

fn empty_result(mut receipt: crate::routing::Receipt) -> ExecutionResult {
    crate::routing::close_receipt(&mut receipt);
    ExecutionResult {
        receipt,
        placeholder: None,
        output: String::new(),
        structured_output: None,
        session_id: None,
        exit_code: None,
    }
}

fn redacted_error(message: &str, api_key: &str) -> String {
    let without_key = if api_key.is_empty() {
        message.to_owned()
    } else {
        message.replace(api_key, "[REDACTED]")
    };
    redact_url_credentials(&without_key)
}

fn redact_url_credentials(message: &str) -> String {
    message
        .split_whitespace()
        .map(|part| {
            if part.contains("@") && part.contains("://") {
                "[REDACTED_URL]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::routing::{Engine, Provider, SandboxMode};

    fn request(prompt: &str) -> RouteRequest {
        RouteRequest {
            provider: Provider::LocalHttp,
            engine: Engine::Http,
            model: "gpt-6.1-sol".to_owned(),
            effort: "xhigh".to_owned(),
            prompt: prompt.to_owned(),
        }
    }

    fn options() -> ExecutionOptions {
        ExecutionOptions {
            requested_model_override: None,
            workspace_dir: None,
            sandbox_mode: SandboxMode::ReadOnly,
            request_id: Some("cdx-http-test".to_owned()),
            idempotency_key: Some("http-idem-test".to_owned()),
            max_input_bytes: 8_192,
            attempt_count: 1,
        }
    }

    async fn fixture_server(
        response: &'static str,
    ) -> crate::Result<(String, tokio::task::JoinHandle<crate::Result<String>>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let response = Arc::new(response.to_owned());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut buffer = vec![0_u8; 32_768];
            let size = stream.read(&mut buffer).await?;
            let request_text = String::from_utf8(buffer[..size].to_vec())?;
            let body = response.as_bytes();
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                response
            );
            stream.write_all(wire.as_bytes()).await?;
            Ok(request_text)
        });
        Ok((format!("http://{address}/v1"), server))
    }

    #[tokio::test]
    async fn sends_strict_schema_and_records_actual_identity_and_usage() -> crate::Result<()> {
        let body = r#"{"id":"chatcmpl-fixture","object":"chat.completion","created":1,"model":"gpt-6.1-sol","choices":[{"index":0,"message":{"role":"assistant","content":"{\"verdict\":\"PASS\"}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}"#;
        let (base_url, server) = fixture_server(body).await?;
        let schema = json!({
            "type":"object",
            "additionalProperties":false,
            "required":["verdict"],
            "properties":{"verdict":{"type":"string","enum":["PASS","WARN","BLOCK"]}}
        });
        let result = execute_with_client(
            request("review this"),
            Duration::from_secs(2),
            &options(),
            Some(&schema),
            &Client::new(),
            &base_url,
            "test-secret",
        )
        .await?;
        let captured = server.await??;
        assert!(
            captured
                .to_ascii_lowercase()
                .contains("authorization: bearer test-secret")
        );
        assert!(captured.contains("chat/completions"));
        assert!(captured.contains("json_schema"));
        assert_eq!(result.receipt.status, ReceiptStatus::Completed);
        assert_eq!(result.receipt.outcome_state, "confirmed_success");
        assert_eq!(result.receipt.actual_model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(
            (result.receipt.tokens_in, result.receipt.tokens_out),
            (12, 4)
        );
        assert_eq!(result.structured_output, Some(json!({"verdict":"PASS"})));
        assert!(result.receipt.credentials_redacted);
        Ok(())
    }

    #[tokio::test]
    async fn model_mismatch_and_malformed_structured_output_fail_closed() -> crate::Result<()> {
        let mismatch = r#"{"model":"gpt-6-sol","choices":[{"message":{"content":"ok"}}]}"#;
        let (base_url, server) = fixture_server(mismatch).await?;
        let result = execute_with_client(
            request("review"),
            Duration::from_secs(2),
            &options(),
            None,
            &Client::new(),
            &base_url,
            "secret",
        )
        .await?;
        server.await??;
        assert_eq!(result.receipt.status, ReceiptStatus::Failed);
        assert!(result.receipt.model_mismatch);

        let malformed = r#"{"model":"gpt-6.1-sol","choices":[{"message":{"content":"not-json"}}]}"#;
        let (base_url, server) = fixture_server(malformed).await?;
        let schema = json!({"type":"object","additionalProperties":false});
        let result = execute_with_client(
            request("review"),
            Duration::from_secs(2),
            &options(),
            Some(&schema),
            &Client::new(),
            &base_url,
            "secret",
        )
        .await?;
        server.await??;
        assert_eq!(result.receipt.status, ReceiptStatus::Failed);
        assert!(result.structured_output.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn timeout_is_unknown_and_never_reports_success() -> crate::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<_, crate::Error>(())
        });
        let result = execute_with_client(
            request("review"),
            Duration::from_millis(30),
            &options(),
            None,
            &Client::builder()
                .timeout(Duration::from_millis(30))
                .build()?,
            &format!("http://{address}/v1"),
            "secret",
        )
        .await?;
        server.abort();
        assert_eq!(result.receipt.status, ReceiptStatus::TimedOutUnknown);
        assert_eq!(result.receipt.outcome_state, "outcome_unknown");
        assert!(result.output.is_empty());
        Ok(())
    }

    #[test]
    fn transport_errors_redact_bearer_material_and_url_credentials() {
        let error = "failed OPENAI_API_KEY=secret-value https://user:secret-value@host/v1";
        let redacted = redacted_error(error, "secret-value");
        assert!(!redacted.contains("secret-value"));
        assert!(!redacted.contains("user:"));
        assert!(redacted.contains("[REDACTED]"));
        assert!(redacted.contains("[REDACTED_URL]"));
    }
}
