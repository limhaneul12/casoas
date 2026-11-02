"""External transport clients for OpenAI Codex CLI/HTTP and Antigravity local RPC."""

from __future__ import annotations

from app.clients.agy_bridge import (
    DEFAULT_CLAUDE_MODEL,
    discover_agy_endpoint,
    extract_json_from_model_text,
    list_agy_models,
    run_agy_model_prompt,
)
from app.clients.agy_rpc_transport import (
    AgyEndpoint,
    AntigravityCircuitBreaker,
    AntigravityModelCatalog,
    AntigravityRpcClient,
    DiscoverEndpointFn,
    RpcTransportFn,
)
from app.clients.codex_aux_runners import CodexAuxiliaryRunner
from app.clients.codex_cli_executor import CodexCliProcessRunner
from app.clients.codex_http_fallback import CodexHttpFallbackRunner
from app.clients.codex_output_parser import (
    CodexCommandBuilder,
    CodexOutputParser,
    ResolveBinaryFn,
)
from app.clients.codex_runner import (
    build_codex_exec_command,
    build_codex_review_command,
    parse_jsonl_events_with_model,
    resolve_codex_binary,
    run_codex_exec,
    run_codex_git_review,
    run_codex_session_followup,
    run_http_fallback,
)

__all__ = [
    "DEFAULT_CLAUDE_MODEL",
    "AgyEndpoint",
    "AntigravityCircuitBreaker",
    "AntigravityModelCatalog",
    "AntigravityRpcClient",
    "CodexAuxiliaryRunner",
    "CodexCliProcessRunner",
    "CodexCommandBuilder",
    "CodexHttpFallbackRunner",
    "CodexOutputParser",
    "DiscoverEndpointFn",
    "ResolveBinaryFn",
    "RpcTransportFn",
    "build_codex_exec_command",
    "build_codex_review_command",
    "discover_agy_endpoint",
    "extract_json_from_model_text",
    "list_agy_models",
    "parse_jsonl_events_with_model",
    "resolve_codex_binary",
    "run_agy_model_prompt",
    "run_codex_exec",
    "run_codex_git_review",
    "run_codex_session_followup",
    "run_http_fallback",
]
