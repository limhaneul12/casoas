//! Native, bounded configuration reads. Never start a thread or provider turn.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};

use crate::Result;

const MAX_OUTPUT: u64 = 512 * 1024;
const MAX_STDERR: u64 = 64 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ConfiguredCodexDefault {
    pub model: String,
    pub origin: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    id: Option<u64>,
    result: Option<Value>,
    error: Option<NativeError>,
    method: Option<String>,
    params: Option<Value>,
    jsonrpc: Option<String>,
    #[serde(rename = "emittedAtMs")]
    _emitted_at_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeError {
    code: i64,
    message: String,
    data: Option<Value>,
}

pub(crate) async fn resolve_codex_default(
    binary: &Path,
    workspace: &Path,
    limit: Duration,
) -> Result<ConfiguredCodexDefault> {
    if limit.is_zero() {
        return Err("Codex configuration read requires a positive timeout".into());
    }
    let workspace = tokio::fs::canonicalize(workspace).await?;
    if !tokio::fs::metadata(&workspace).await?.is_dir() {
        return Err("Codex configuration workspace must be a directory".into());
    }
    let mut command = Command::new(binary);
    command
        .env("PATH", super::enriched_command_path())
        .args(["app-server", "--stdio"])
        .current_dir(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let group = child.id();
    let stdin = child.stdin.take().ok_or("Codex config stdin missing")?;
    let stdout = child.stdout.take().ok_or("Codex config stdout missing")?;
    let stderr = child.stderr.take().ok_or("Codex config stderr missing")?;
    let operation = async {
        let (configured, (), status) = tokio::try_join!(
            read_config(stdin, stdout, &workspace),
            discard_bounded(stderr, MAX_STDERR),
            async { Ok::<_, crate::Error>(child.wait().await?) },
        )?;
        if !status.success() {
            return Err("Codex configuration client exited unsuccessfully".into());
        }
        Ok::<_, crate::Error>(configured)
    };
    let outcome: Result<ConfiguredCodexDefault> = tokio::select! {
        result = tokio::time::timeout(limit.min(Duration::from_secs(5)), operation) => {
            match result {
                Ok(outcome) => outcome,
                Err(_) => Err("Codex configuration read timed out before dispatch".into()),
            }
        }
        () = crate::supervisor::cancelled() => Err("Codex configuration read cancelled before dispatch".into()),
    };
    // Keep the original process-group identity even after the leader exits.
    super::terminate_process_group(&mut child, group).await?;
    outcome
}

async fn read_config(
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    workspace: &Path,
) -> Result<ConfiguredCodexDefault> {
    let mut output = BufReader::new(stdout.take(MAX_OUTPUT + 1));
    let mut consumed = 0_u64;
    send(&mut stdin, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"hcodex-config-preflight","version":env!("CARGO_PKG_VERSION")},"capabilities":{"explicitGatewayOauth":true}}})).await?;
    let _initialized = response(&mut output, &mut consumed, 1).await?;
    send(&mut stdin, json!({"method":"initialized","params":{}})).await?;
    send(
        &mut stdin,
        json!({"id":2,"method":"config/read","params":{"cwd":workspace,"includeLayers":true}}),
    )
    .await?;
    let result = response(&mut output, &mut consumed, 2).await?;
    drop(stdin);
    project(result)
}

async fn send(stdin: &mut ChildStdin, request: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&request)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

async fn response<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    consumed: &mut u64,
    expected: u64,
) -> Result<Value> {
    loop {
        let mut bytes = Vec::new();
        let read = reader.read_until(b'\n', &mut bytes).await?;
        if read == 0 {
            return Err("Codex configuration client closed before its response".into());
        }
        *consumed += u64::try_from(read)?;
        if *consumed > MAX_OUTPUT {
            return Err("Codex configuration response exceeded its byte budget".into());
        }
        let frame: Frame = serde_json::from_slice(&bytes)?;
        let _transport_metadata = (frame.params, frame.jsonrpc);
        if frame.id.is_none() && frame.method.is_some() {
            continue;
        }
        if frame.id != Some(expected) || frame.method.is_some() {
            return Err("Unexpected request during read-only Codex configuration probe".into());
        }
        if let Some(error) = frame.error {
            let _private_error_data = error.data;
            let safe = crate::routing::redact_sensitive_text(&error.message)
                .chars()
                .take(1024)
                .collect::<String>();
            return Err(format!("Codex configuration read failed ({}): {safe}", error.code).into());
        }
        return frame
            .result
            .ok_or_else(|| "Codex configuration response has no result".into());
    }
}

fn project(result: Value) -> Result<ConfiguredCodexDefault> {
    let config = result
        .get("config")
        .and_then(Value::as_object)
        .ok_or("Native configuration response has no config object")?;
    let model = config
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .ok_or("codex-default requires an explicitly configured concrete model")?;
    let canonical = crate::routing::normalize_model(model)
        .filter(|canonical| canonical.starts_with("gpt-6") && canonical == model)
        .ok_or("Configured Codex default is not a supported concrete GPT6 model")?;
    let provider = config
        .get("model_provider")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .ok_or("Configured model provider must be a string")
        })
        .transpose()?;
    if provider.is_some_and(|provider| provider != "openai") {
        return Err("codex-default cannot discard a configured custom model provider".into());
    }
    if result
        .get("layers")
        .and_then(Value::as_array)
        .is_some_and(|layers| {
            layers.iter().any(|layer| {
                layer.get("disabledReason").is_none_or(Value::is_null)
                    && layer
                        .pointer("/config/model_providers/openai")
                        .is_some_and(|value| !value.is_null())
            })
        })
    {
        return Err("codex-default cannot discard custom OpenAI provider configuration".into());
    }
    Ok(ConfiguredCodexDefault {
        model: canonical,
        origin: result
            .pointer("/origins/model")
            .cloned()
            .filter(|value| !value.is_null()),
    })
}

async fn discard_bounded<R: AsyncRead + Unpin>(reader: R, limit: u64) -> Result<()> {
    let mut bounded = reader.take(limit + 1);
    let count = tokio::io::copy(&mut bounded, &mut tokio::io::sink()).await?;
    if count > limit {
        return Err("Codex configuration diagnostics exceeded their byte budget".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(root: &Path, script: &str) -> Result<std::path::PathBuf> {
        let binary = root.join("fake-codex");
        std::fs::write(&binary, script)?;
        let mut permissions = std::fs::metadata(&binary)?.permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions)?;
        Ok(binary)
    }

    fn config_binary(root: &Path, result: Value) -> Result<std::path::PathBuf> {
        std::fs::write(
            root.join("response.json"),
            serde_json::to_vec(&json!({"id":2,"result":result,"emittedAtMs":1793696860000_u64}))?,
        )?;
        executable(
            root,
            concat!(
                "#!/bin/sh\n",
                "test \"$1:$2\" = 'app-server:--stdio' || exit 90\n",
                "dir=$(dirname \"$0\")\n",
                "while IFS= read -r request; do\n",
                "  printf '%s\\n' \"$request\" >> \"$dir/requests\"\n",
                "  case \"$request\" in\n",
                "    *'\"method\":\"initialize\"'*) printf '%s\\n' '{\"method\":\"config/updated\",\"params\":{},\"emittedAtMs\":1793696860000}' '{\"id\":1,\"result\":{}}';;\n",
                "    *'\"method\":\"config/read\"'*) cat \"$dir/response.json\"; printf '\\n';;\n",
                "    *'\"method\":\"initialized\"'*) :;;\n",
                "    *) exit 91;;\n",
                "  esac\n",
                "done\n",
            ),
        )
    }

    #[tokio::test]
    async fn native_timestamp_metadata_stays_typed_and_other_extensions_remain_closed() -> Result<()>
    {
        for timestamp in [json!("1793696860000"), json!(-1), json!(1.5)] {
            assert!(
                serde_json::from_value::<Frame>(
                    json!({"method":"config/updated","params":{},"emittedAtMs":timestamp})
                )
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<Frame>(
                json!({"method":"config/updated","params":{},"unexpectedTelemetry":1})
            )
            .is_err()
        );
        let root = tempfile::tempdir()?;
        let binary = config_binary(root.path(), json!({"config":{"model":"gpt-6.1-sol"}}))?;
        std::fs::write(
            root.path().join("response.json"),
            serde_json::to_vec(
                &json!({"method":"config/updated","params":{},"emittedAtMs":"invalid"}),
            )?,
        )?;
        assert!(
            resolve_codex_default(&binary, root.path(), Duration::from_secs(2))
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn native_config_protocol_resolves_model_and_preserves_alias_without_fabricating_actual()
    -> Result<()> {
        let root = tempfile::tempdir()?;
        let binary = config_binary(
            root.path(),
            json!({"config":{"model":"gpt-6.1-sol","model_provider":null},"origins":{"model":{"name":{"type":"project","dotCodexFolder":"/fixture/.codex"},"version":"sha256:fixture"}},"layers":[]}),
        )?;
        let configured =
            resolve_codex_default(&binary, root.path(), Duration::from_secs(2)).await?;
        assert_eq!(configured.model, "gpt-6.1-sol");
        assert_eq!(
            configured
                .origin
                .as_ref()
                .and_then(|v| v.pointer("/name/type")),
            Some(&json!("project"))
        );
        let requests = std::fs::read_to_string(root.path().join("requests"))?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            requests
                .iter()
                .filter_map(|v| v["method"].as_str())
                .collect::<Vec<_>>(),
            ["initialize", "initialized", "config/read"]
        );
        assert_eq!(
            requests[2]["params"]["cwd"],
            json!(root.path().canonicalize()?)
        );
        let route = crate::routing::validate_route(&crate::routing::RouteRequest {
            provider: crate::routing::Provider::OpenaiCodex,
            engine: crate::routing::Engine::CodexCli,
            model: configured.model,
            effort: "high".to_owned(),
            prompt: "review".to_owned(),
        })?;
        let options = super::super::ExecutionOptions {
            requested_model_override: Some("codex-default".to_owned()),
            ..super::super::ExecutionOptions::default()
        };
        let receipt = super::super::receipt(&route, &options);
        assert_eq!(receipt.requested_model, "codex-default");
        assert_eq!(receipt.resolved_model, "gpt-6.1-sol");
        assert!(receipt.actual_model.is_none());
        assert!(receipt.identity_source.is_none());
        assert_eq!(
            receipt
                .routing_decision
                .as_ref()
                .map(|v| v.requested_model.as_str()),
            Some("codex-default")
        );
        Ok(())
    }

    #[tokio::test]
    async fn default_resolution_rejects_missing_unsupported_and_custom_provider_configs()
    -> Result<()> {
        for result in [
            json!({"config":{"model":null}}),
            json!({"config":{"model":"codex-default"}}),
            json!({"config":{"model":"gpt-5.6-sol"}}),
            json!({"config":{"model":"gpt-6.1-sol","model_provider":"private"}}),
            json!({"config":{"model":"gpt-6.1-sol"},"layers":[{"config":{"model_providers":{"openai":{"base_url":"https://example.invalid"}}}}]}),
        ] {
            let root = tempfile::tempdir()?;
            let binary = config_binary(root.path(), result)?;
            assert!(
                resolve_codex_default(&binary, root.path(), Duration::from_secs(2))
                    .await
                    .is_err()
            );
        }
        let root = tempfile::tempdir()?;
        let closed = executable(root.path(), "#!/bin/sh\nexit 0\n")?;
        assert!(
            resolve_codex_default(&closed, root.path(), Duration::from_secs(2))
                .await
                .is_err()
        );
        let oversized = executable(root.path(), "#!/bin/sh\nhead -c 524289 /dev/zero\n")?;
        assert!(
            resolve_codex_default(&oversized, root.path(), Duration::from_secs(2))
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn config_timeout_kills_owned_descendant_after_leader_exits() -> Result<()> {
        let root = tempfile::tempdir()?;
        let binary = executable(
            root.path(),
            concat!(
                "#!/bin/sh\n",
                "sleep 30 & descendant=$!\n",
                "dir=$(dirname \"$0\")\n",
                "printf '%s\\n%s\\n' \"$$\" \"$descendant\" > \"$dir/pids\"\n",
                "IFS= read -r request\n",
                "printf '%s\\n' '{\"id\":1,\"result\":{}}'\n",
                "IFS= read -r initialized\nIFS= read -r request\n",
                "exit 0\n",
            ),
        )?;
        // Use the actual configured startup cap so parallel test process load
        // cannot consume an artificial one-second handshake allowance.
        let outcome = resolve_codex_default(&binary, root.path(), Duration::from_secs(5)).await;
        let error = outcome
            .err()
            .ok_or("hanging configuration read must time out")?;
        assert!(error.to_string().contains("timed out"));
        for pid in std::fs::read_to_string(root.path().join("pids"))?.lines() {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let output = Command::new("/bin/ps")
                    .args(["-o", "stat=", "-p", pid])
                    .output()
                    .await?;
                let state = String::from_utf8_lossy(&output.stdout);
                let state = state.trim();
                if state.is_empty() || state.starts_with('Z') {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "config descendant remained running: {pid}"
                );
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        }
        Ok(())
    }
}
