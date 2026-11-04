//! Newline-delimited JSON-RPC MCP adapter; stdout is reserved for protocol data.
use crate::gateway::Gateway;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::task::JoinSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCall {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    #[serde(rename = "_meta", default)]
    _meta: Option<Value>,
}
fn empty_object() -> Value {
    json!({})
}

pub async fn run(gateway: Gateway) -> crate::Result<()> {
    run_io(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        gateway,
    )
    .await
}
pub async fn run_io<R, W>(mut reader: R, mut writer: W, gateway: Gateway) -> crate::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut tasks = JoinSet::new();
    let mut buffer = Vec::new();
    let request_result:crate::Result<()>=async {
    loop {
        tokio::select! {
            read=next_message(&mut reader,&mut buffer) => {
                let Some(line)=read? else {break;};
                let request=serde_json::from_slice::<Request>(&line);
                match request {
                    Ok(request) if request.jsonrpc=="2.0" => {
                        if request.id.is_none() {continue;}
                        if tasks.len()>=8 {write(&mut writer,error(request.id.unwrap_or(Value::Null),-32000,"request capacity reached")).await?;continue;}
                        let service=gateway.clone(); tasks.spawn(async move {respond(request,&service).await});
                    }
                    Err(cause) if cause.is_syntax() || cause.is_eof() => write(&mut writer,error(Value::Null,-32700,"JSON parse error")).await?,
                    _ => write(&mut writer,error(Value::Null,-32600,"invalid JSON-RPC request")).await?,
                }
            }
            completed=tasks.join_next(), if !tasks.is_empty() => {
                if let Some(result)=completed {write(&mut writer,result?).await?;}
            }
        }
    }
    // A finite piped request stream still receives every response before exit.
    while let Some(result) = tasks.join_next().await {
        write(&mut writer, result?).await?;
    }
    Ok(())
    }.await;
    // Cleanup also runs for malformed framing, read/write errors and failed joins.
    let cleanup_result = gateway.shutdown().await;
    tasks.shutdown().await;
    request_result.and(cleanup_result)
}
async fn respond(request: Request, gateway: &Gateway) -> Value {
    let id = request.id.unwrap_or(Value::Null);
    let modern = request
        .params
        .pointer("/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)
        == Some("2026-07-28");
    if let Some(version) = request
        .params
        .pointer("/_meta/io.modelcontextprotocol~1protocolVersion")
    {
        if !version
            .as_str()
            .is_some_and(|version| SUPPORTED_VERSIONS.contains(&version))
        {
            return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":SUPPORTED_VERSIONS,"requested":version}}});
        }
        if version == "2026-07-28"
            && !request
                .params
                .pointer("/_meta/io.modelcontextprotocol~1clientCapabilities")
                .is_some_and(Value::is_object)
        {
            return error(
                id,
                -32602,
                "modern request requires clientCapabilities metadata",
            );
        }
    }
    let result: crate::Result<Value> = match request.method.as_str() {
        "initialize" => initialize_result(&request.params),
        "server/discover" => Ok(
            json!({"resultType":"complete","supportedVersions":SUPPORTED_VERSIONS,"capabilities":{"tools":{"listChanged":false}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"codex-collab-mcp","version":env!("CARGO_PKG_VERSION")}},"instructions":"Tri-polar Heterarchy Codex Rust MCP","ttlMs":3600000,"cacheScope":"public"}),
        ),
        "ping" => Ok(json!({})),
        "tools/list" => crate::catalog::tools().map(|tools| json!({"tools":tools})),
        "tools/call" => match serde_json::from_value::<ToolCall>(request.params) {
            Ok(call) => match gateway.call(&call.name, call.arguments).await {
                Ok(result) => Ok(
                    json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":{"result":result.to_string()},"isError":false}),
                ),
                Err(cause) => {
                    Ok(json!({"content":[{"type":"text","text":cause.to_string()}],"isError":true}))
                }
            },
            Err(cause) => Err(cause.into()),
        },
        _ => return error(id, -32601, "method not found"),
    };
    match result {
        Ok(mut result) => {
            if modern {
                result["resultType"] = json!("complete");
                if request.method == "tools/list" {
                    result["ttlMs"] = json!(0);
                    result["cacheScope"] = json!("private");
                }
            }
            json!({"jsonrpc":"2.0","id":id,"result":result})
        }
        Err(cause) => error(id, -32602, &cause.to_string()),
    }
}
const SUPPORTED_VERSIONS: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
fn initialize_result(params: &Value) -> crate::Result<Value> {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or("initialize requires a protocolVersion string")?;
    let negotiated = SUPPORTED_VERSIONS[1..]
        .iter()
        .find(|version| **version == requested)
        .copied()
        .unwrap_or("2025-11-25");
    Ok(
        json!({"protocolVersion":negotiated,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"codex-collab-mcp","version":env!("CARGO_PKG_VERSION")},"instructions":"Tri-polar Heterarchy Codex Rust MCP"}),
    )
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
async fn write<W: AsyncWrite + Unpin>(writer: &mut W, value: Value) -> crate::Result<()> {
    writer.write_all(value.to_string().as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn next_message<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> crate::Result<Option<Vec<u8>>> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if buffer.is_empty() {
                Ok(None)
            } else {
                Ok(Some(std::mem::take(buffer)))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index + 1);
        if buffer.len() + take > 2_000_000 {
            return Err("MCP message exceeds byte limit".into());
        }
        buffer.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some(std::mem::take(buffer)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn distinguishes_parse_errors_from_invalid_request_data() -> crate::Result<()> {
        let temp = tempfile::tempdir()?;
        let service = Gateway::new(temp.path().into());
        let mut output = Vec::new();
        let input = b"{invalid\n{\"jsonrpc\":\"2.0\",\"method\":42}\n";
        run_io(&input[..], &mut output, service).await?;
        let responses = String::from_utf8(output)?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[1]["error"]["code"], -32600);
        Ok(())
    }
    #[tokio::test]
    async fn modern_discovery_and_version_negotiation() -> crate::Result<()> {
        let root = tempfile::tempdir()?;
        let gateway = Gateway::new(root.path().into());
        assert_eq!(
            initialize_result(&json!({"protocolVersion":"2025-06-18"}))?["protocolVersion"],
            "2025-06-18"
        );
        assert_eq!(
            initialize_result(&json!({"protocolVersion":"future"}))?["protocolVersion"],
            "2025-11-25"
        );
        assert!(initialize_result(&json!({"protocolVersion":12})).is_err());
        let response=respond(Request{jsonrpc:"2.0".into(),id:Some(json!(1)),method:"server/discover".into(),params:json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}})},&gateway).await;
        assert_eq!(response["result"]["resultType"], "complete");
        let list=respond(Request{jsonrpc:"2.0".into(),id:Some(json!(3)),method:"tools/list".into(),params:json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}})},&gateway).await;
        assert_eq!(list["result"]["resultType"], "complete");
        assert_eq!(list["result"]["ttlMs"], 0);
        assert_eq!(list["result"]["cacheScope"], "private");
        let ping=respond(Request{jsonrpc:"2.0".into(),id:Some(json!(4)),method:"ping".into(),params:json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}})},&gateway).await;
        assert_eq!(ping["result"]["resultType"], "complete");
        let response = respond(
            Request {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "tools/list".into(),
                params: json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"bad"}}),
            },
            &gateway,
        )
        .await;
        assert_eq!(response["error"]["code"], -32022);
        Ok(())
    }
    #[tokio::test]
    async fn finite_stdio_stream_receives_every_response() -> crate::Result<()> {
        let temp = tempfile::tempdir()?;
        let service = Gateway::new(temp.path().into());
        let input=b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-03-26\"}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
        let mut output = Vec::new();
        run_io(&input[..], &mut output, service).await?;
        let text = String::from_utf8(output)?;
        let responses = text
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(responses.len(), 2);
        assert!(
            responses
                .iter()
                .any(|value| value["result"]["serverInfo"]["name"] == "codex-collab-mcp")
        );
        assert!(responses.iter().any(|value| {
            value["result"]["tools"]
                .as_array()
                .is_some_and(|tools| tools.len() == 16)
        }));
        Ok(())
    }

    #[tokio::test]
    async fn initialization_catalog_and_strict_wire() -> crate::Result<()> {
        let temp = tempfile::tempdir()?;
        let service = Gateway::new(temp.path().into());
        let request = Request {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/list".into(),
            params: Value::Null,
        };
        let response = respond(request, &service).await;
        assert_eq!(
            response["result"]["tools"].as_array().map(Vec::len),
            Some(16)
        );
        let bad: std::result::Result<Request, _> =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":"ping","extra":true}));
        assert!(bad.is_err());
        let request = Request {
            jsonrpc: "2.0".into(),
            id: Some(json!(2)),
            method: "tools/call".into(),
            params: json!({"name":"codex_job_list","arguments":{"limit":"bad"}}),
        };
        assert_eq!(respond(request, &service).await["result"]["isError"], true);
        assert!(!temp.path().join("jobs").exists());
        Ok(())
    }
}
