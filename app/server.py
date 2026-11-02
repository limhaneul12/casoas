"""FastMCP Server for Gemini 4 Argon <-> Claude Sonnet 5.5 <-> OpenAI Codex Heterarchy."""

from __future__ import annotations

import asyncio
import sys

from mcp.server.mcpserver import MCPServer

from app.clients.agy_bridge import (
    DEFAULT_CLAUDE_MODEL,
    discover_agy_endpoint,
    list_agy_models,
    run_agy_model_prompt,
)
from app.clients.codex_runner import (
    resolve_codex_binary,
    run_codex_exec,
    run_codex_git_review,
    run_codex_session_followup,
    run_http_fallback,
)
from app.core.model_contract_enums import EngineAliasKind, SandboxModeKind
from app.repositories.handoff_store import HandoffStore
from app.repositories.job_store import JobStore
from app.schemas.boundary_schemas import JsonObject
from app.schemas.inner_dto import (
    EngineDispatchRequest,
    ServerGatewayBindings,
)
from app.services.dag_ulw_service import DagUlwExecutionService
from app.services.decision_pack_service import AlexandriaDecisionPackService
from app.services.mcp_gateway_service import McpExecutionGateway
from app.services.mcp_tool_handlers import McpToolDispatcher
from app.services.triad_pipeline_service import TriadOrchestrationService

_INSTRUCTIONS = (
    "Tri-Polar Heterarchy collaboration & orchestration bridge pairing "
    "Gemini 4 Argon (macro-architect), Claude Sonnet 5.5 / Opus 5.5 (surgical code craftsman), "
    "and OpenAI Codex (adversarial red-team auditor, high-speed synthesizer, rescue sandbox)."
)
mcp = MCPServer("codex-collab-mcp", instructions=_INSTRUCTIONS)

job_store = JobStore()
handoff_store = HandoffStore()
_background_tasks: set[asyncio.Task[None]] = set()
_background_job_semaphore: asyncio.Semaphore = asyncio.Semaphore(8)
_gateway = McpExecutionGateway(
    background_tasks=_background_tasks,
    background_job_semaphore=_background_job_semaphore,
)
_tool_dispatcher = McpToolDispatcher()
_triad_service = TriadOrchestrationService()
_decision_pack_service = AlexandriaDecisionPackService()
_dag_ulw_service = DagUlwExecutionService()


async def _execute_with_engine(
    prompt: str,
    schema_name: str,
    output_schema: JsonObject | None,
    workspace_dir: str | None = None,
    sandbox_mode: str = SandboxModeKind.READ_ONLY,
    model: str | None = None,
    reasoning_effort: str | None = None,
    engine: str = EngineAliasKind.CLI,
    timeout_seconds: float = 300.0,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    max_input_bytes: int = 256_000,
) -> JsonObject:
    """Validate `(provider, engine, model, reasoning_effort)` pre-dispatch and execute.

    Args:
        prompt: Prompt string to send to the selected model engine.
        schema_name: Closed-boundary schema name for output validation.
        output_schema: JSON Schema object for structured output, or None.
        workspace_dir: Target workspace directory path.
        sandbox_mode: Execution sandbox policy (`read-only` or `workspace-write`).
        model: Requested model identifier or alias.
        reasoning_effort: Reasoning effort tier for Codex CLI.
        engine: Execution engine identifier (`cli`, `http`, or `antigravity_local_rpc`).
        timeout_seconds: Execution timeout budget in seconds.
        provider: Explicit provider override (`antigravity`, `openai_codex`, `local_http`).
        request_id: Optional caller-supplied request identifier.
        idempotency_key: Optional idempotency key for safe deduplication/retry.
        retry_same_request: Whether to explicitly re-dispatch an existing request ID.
        max_input_bytes: Maximum UTF-8 prompt byte budget before pre-dispatch rejection.

    Returns:
        Execution result dictionary with receipt, status, and validated output.
    """
    req = EngineDispatchRequest(
        prompt=prompt,
        schema_name=schema_name,
        output_schema=output_schema,
        workspace_dir=workspace_dir,
        sandbox_mode=sandbox_mode,
        model=model,
        reasoning_effort=reasoning_effort,
        engine=engine,
        timeout_seconds=timeout_seconds,
        provider=provider,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
    )
    bindings = ServerGatewayBindings(
        job_store=job_store,
        run_agy_fn=run_agy_model_prompt,
        run_http_fn=run_http_fallback,
        run_codex_fn=run_codex_exec,
    )
    return await _gateway.execute_with_engine(req=req, bindings=bindings)


# Ensure `python -m app.server` aliases `__main__` as `app.server` before routers import it.
sys.modules.setdefault("app.server", sys.modules[__name__])

from app.api.codex_router import (  # noqa: E402
    codex_adversarial_audit,
    codex_differential_implementations,
    codex_git_review,
    codex_rescue_sandbox,
    codex_session_followup,
    codex_targeted_synthesize,
)
from app.api.job_handoff_router import (  # noqa: E402
    codex_bridge_health,
    codex_job_list,
    codex_job_status,
    heterarchy_handoff_create,
    heterarchy_handoff_inbox,
    heterarchy_handoff_resolve,
)
from app.api.triad_router import (  # noqa: E402
    claude_code_craft,
    codex_alexandria_decision_pack,
    heterarchy_dag_ulw,
    heterarchy_triad_orchestrate,
)


def main() -> None:
    """Entry point for running the FastMCP stdio server and initializing SQLite stores."""
    job_store.ensure_db_initialized()
    handoff_store.ensure_db_initialized()
    mcp.run(transport="stdio")


__all__ = [
    "DEFAULT_CLAUDE_MODEL",
    "_background_job_semaphore",
    "_background_tasks",
    "_dag_ulw_service",
    "_decision_pack_service",
    "_execute_with_engine",
    "_gateway",
    "_tool_dispatcher",
    "_triad_service",
    "claude_code_craft",
    "codex_adversarial_audit",
    "codex_alexandria_decision_pack",
    "codex_bridge_health",
    "codex_differential_implementations",
    "codex_git_review",
    "codex_job_list",
    "codex_job_status",
    "codex_rescue_sandbox",
    "codex_session_followup",
    "codex_targeted_synthesize",
    "discover_agy_endpoint",
    "handoff_store",
    "heterarchy_dag_ulw",
    "heterarchy_handoff_create",
    "heterarchy_handoff_inbox",
    "heterarchy_handoff_resolve",
    "heterarchy_triad_orchestrate",
    "job_store",
    "list_agy_models",
    "main",
    "mcp",
    "resolve_codex_binary",
    "run_agy_model_prompt",
    "run_codex_exec",
    "run_codex_git_review",
    "run_codex_session_followup",
    "run_http_fallback",
]


if __name__ == "__main__":
    main()
