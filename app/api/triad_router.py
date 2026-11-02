"""MCP tool router for Claude surgical code craft, Triad orchestration, and Alexandria packs."""

from __future__ import annotations

import app.server as _srv
from app.clients.agy_bridge import DEFAULT_CLAUDE_MODEL
from app.core.collab_enums import (
    CanonicalModelId,
    OrchestrationMode,
    ReasoningTier,
)
from app.schemas.boundary_schemas import JsonObject
from app.schemas.dag_input_schemas import (
    DagNodeInputModel,
    DagSpawnPlanInputModel,
    DagUlwRequestModel,
)
from app.schemas.inner_dto import (
    ClaudeCraftRequest,
    DecisionPackRequest,
    TriadOrchestrationRequest,
)


@_srv.mcp.tool()
async def claude_code_craft(
    task_spec: str,
    target_code_or_context: str = "",
    invariants_and_rules: str = "",
    language: str = "python",
    claude_model: str = DEFAULT_CLAUDE_MODEL,
    timeout_seconds: float = 180.0,
    workspace_dir: str = ".",
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    files_inspected: list[str] | None = None,
    symbols_inspected: list[str] | None = None,
    executed_test_commands: list[JsonObject] | None = None,
    max_input_bytes: int = 256_000,
    async_background: bool = False,
) -> str:
    """Invoke Claude Sonnet 5.5 or Opus 5.5 (Medium tier) via Antigravity's local RPC bridge.

    Specialized for surgical code implementation, multi-file refactoring, clean API/UI ergonomics,
    and design review before passing the result to Codex for adversarial audit.

    Args:
        task_spec: Specification of the code craft or refactoring task.
        target_code_or_context: Existing code or architectural context to transform.
        invariants_and_rules: Non-negotiable invariants and repository rules.
        language: Target programming language.
        claude_model: Claude Medium-tier model identifier or alias.
        timeout_seconds: RPC execution timeout in seconds.
        workspace_dir: Target workspace directory path.
        request_id: Optional explicit request identifier.
        idempotency_key: Optional idempotency key for deduplication.
        retry_same_request: Whether to retry an existing request ID.
        files_inspected: Optional relative file paths inspected for evidence scope.
        symbols_inspected: Optional symbol names inspected for evidence scope.
        executed_test_commands: Optional caller-reported test command records.
        max_input_bytes: Maximum UTF-8 prompt byte budget.
        async_background: Whether to dispatch the craft job asynchronously in the background.

    Returns:
        Pretty-printed JSON string containing the Claude craft result and receipt.
    """
    req = ClaudeCraftRequest(
        task_spec=task_spec,
        target_code_or_context=target_code_or_context,
        invariants_and_rules=invariants_and_rules,
        language=language,
        claude_model=claude_model,
        timeout_seconds=timeout_seconds,
        workspace_dir=workspace_dir,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        files_inspected=files_inspected,
        symbols_inspected=symbols_inspected,
        executed_test_commands=executed_test_commands,
        async_background=async_background,
    )
    return await _srv._gateway.run_claude_code_craft(
        req=req,
        job_store=_srv.job_store,
        run_agy_fn=_srv.run_agy_model_prompt,
    )


@_srv.mcp.tool()
async def heterarchy_triad_orchestrate(
    task_spec: str,
    target_code_or_context: str = "",
    invariants_and_rules: str = "",
    language: str = "python",
    mode: str = OrchestrationMode.PIPELINE,
    workspace_dir: str = ".",
    claude_model: str = DEFAULT_CLAUDE_MODEL,
    codex_model: str | None = None,
    codex_reasoning_effort: str = ReasoningTier.XHIGH,
    argon_model: str = CanonicalModelId.GEMINI_4_ARGON_HIGH,
    invoke_argon_rpc: bool = True,
    argon_handoff_ticket_id: str | None = None,
    timeout_seconds: float = 300.0,
    files_inspected: list[str] | None = None,
    symbols_inspected: list[str] | None = None,
    executed_test_commands: list[JsonObject] | None = None,
    max_input_bytes: int = 256_000,
    resume_collaboration_id: str | None = None,
    max_rounds: int = 1,
    run_verification_gates: bool = False,
    async_background: bool = False,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
) -> str:
    """Orchestrate Gemini 4 Argon + Claude Sonnet 5.5 + OpenAI Codex with explicit role states.

    Separates per-role collaboration state (`not_started | queued | running | succeeded |
    failed | timed_out_unknown`) from consensus state (`consensus_verified |
    incomplete_no_consensus | conflict_requires_adjudication | blocked_by_red_team`).
    Never reports 3-pole completion or consensus if Argon (or any required role) is pending,
    queued via handoff ticket, or timed out (`timed_out_unknown`).

    Args:
        task_spec: Specification of the task under review.
        target_code_or_context: Code or architectural context under review.
        invariants_and_rules: Non-negotiable invariants and repository rules.
        language: Target programming language.
        mode: Orchestration mode (`pipeline` or `council`).
        workspace_dir: Target workspace directory path.
        claude_model: Claude Medium-tier model identifier.
        codex_model: Optional Codex model identifier.
        codex_reasoning_effort: Reasoning effort tier for Codex CLI.
        argon_model: Gemini 4 Argon model identifier.
        invoke_argon_rpc: Whether to invoke Argon via local RPC.
        argon_handoff_ticket_id: Optional handoff ticket ID for Argon.
        timeout_seconds: Execution timeout budget in seconds.
        files_inspected: Optional relative file paths inspected for evidence scope.
        symbols_inspected: Optional symbol names inspected for evidence scope.
        executed_test_commands: Optional caller-reported test command records.
        max_input_bytes: Maximum UTF-8 prompt byte budget.
        resume_collaboration_id: Optional prior collaboration ID (`cdx-...`) to resume from.
        max_rounds: Maximum pipeline auto-repair rounds (`1..3`) when Codex blocks or warns.
        run_verification_gates: Whether to execute allowlisted verification gates in workspace.
        async_background: Whether to execute the triad orchestration in the background.
        request_id: Optional caller-supplied request identifier (`cdx-...`).
        idempotency_key: Optional idempotency key for safe deduplication/retry.
        retry_same_request: Whether to retry an existing request ID.

    Returns:
        Pretty-printed JSON string containing the orchestration outcome and evidence scope.
    """
    req = TriadOrchestrationRequest(
        task_spec=task_spec,
        target_code_or_context=target_code_or_context,
        invariants_and_rules=invariants_and_rules,
        language=language,
        mode=mode,
        workspace_dir=workspace_dir,
        claude_model=claude_model,
        codex_model=codex_model,
        codex_reasoning_effort=codex_reasoning_effort,
        argon_model=argon_model,
        invoke_argon_rpc=invoke_argon_rpc,
        argon_handoff_ticket_id=argon_handoff_ticket_id,
        timeout_seconds=timeout_seconds,
        files_inspected=files_inspected,
        symbols_inspected=symbols_inspected,
        executed_test_commands=executed_test_commands,
        max_input_bytes=max_input_bytes,
        resume_collaboration_id=resume_collaboration_id,
        max_rounds=max_rounds,
        run_verification_gates=run_verification_gates,
        async_background=async_background,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
    )
    return await _srv._triad_service.orchestrate_triad(
        req=req,
        job_store=_srv.job_store,
        handoff_store=_srv.handoff_store,
        run_agy_fn=_srv.run_agy_model_prompt,
        execute_fn=_srv._execute_with_engine,
        spawn_bg_fn=_srv._gateway.spawn_background_job,
    )


@_srv.mcp.tool()
async def codex_alexandria_decision_pack(
    project_scope: str,
    topic_title: str,
    argon_architecture_intent: str,
    codex_audit_or_synthesis_summary: str,
    final_resolution: str,
    sonnet_craft_or_review_summary: str = "",
    verification_evidence: str = "",
    collaboration_id: str | None = None,
    workspace_dir: str | None = None,
    expected_scope_digest: str | None = None,
) -> str:
    """Package a Tri-Polar collaboration outcome into a deterministic Alexandria bundle.

    Generates a stable idempotency key and formatted Markdown record ready to pass to
    `alexandria_verified_upsert` or `alexandria_memory_steward_refresh_current_compact`.
    Consensus can only be attributed to a completed aggregate matching all supplied scope fences.

    Args:
        project_scope: Target Alexandria project scope identifier.
        topic_title: Short title of the architectural decision topic.
        argon_architecture_intent: Argon's architectural intent and rationale.
        codex_audit_or_synthesis_summary: Codex's audit findings or synthesis summary.
        final_resolution: Final agreed trade-off resolution.
        sonnet_craft_or_review_summary: Claude Sonnet/Opus craft or review summary.
        verification_evidence: Verification evidence notes.
        collaboration_id: Optional same-run triad collaboration ID (`cdx-...`).
        workspace_dir: Optional workspace directory fence for verification.
        expected_scope_digest: Optional expected SHA-256 evidence scope digest fence.

    Returns:
        Pretty-printed JSON string containing the Alexandria decision bundle.
    """
    req = DecisionPackRequest(
        project_scope=project_scope,
        topic_title=topic_title,
        argon_architecture_intent=argon_architecture_intent,
        codex_audit_or_synthesis_summary=codex_audit_or_synthesis_summary,
        final_resolution=final_resolution,
        sonnet_craft_or_review_summary=sonnet_craft_or_review_summary,
        verification_evidence=verification_evidence,
        collaboration_id=collaboration_id,
        workspace_dir=workspace_dir,
        expected_scope_digest=expected_scope_digest,
    )
    return _srv._decision_pack_service.build_decision_pack(
        req=req,
        job_store=_srv.job_store,
    )


@_srv.mcp.tool()
async def heterarchy_dag_ulw(
    campaign_goal: str,
    nodes: list[JsonObject] | None = None,
    workspace_dir: str = ".",
    plan_only: bool = True,
    spawn_plan: JsonObject | None = None,
    resume_dag_id: str | None = None,
    timeout_seconds: float = 300.0,
    max_input_bytes: int = 256_000,
    request_id: str | None = None,
    idempotency_key: str | None = None,
) -> str:
    """Compile and execute a Tri-Polar Graph Engineering (`DAG-ULW++`) campaign.

    Compiles a DAG of nodes (`topology_planner`, `reproduction_prober`, `interface_contractor`,
    `disjoint_builder`, `adversarial_auditor`, `verification_fan_in`) into Kahn topological waves,
    checks WAW/RAW and CodeGraph 1-hop symbol hazards, enforces `SpawnPlanGate`, and executes
    waves with AIMD concurrency and two-stage settlement (`STOPPED != SETTLED` with full-workspace
    SHA-256 `WorkspaceManifest` verification).

    Args:
        campaign_goal: High-level engineering objective for the DAG campaign.
        nodes: Optional list of node specification dictionaries (`DagNodeInputModel`).
        workspace_dir: Target workspace directory path.
        plan_only: If True, compile waves, hazards, and ASCII graph without executing nodes.
        spawn_plan: Optional spawn plan acknowledgment dictionary (`DagSpawnPlanInputModel`).
        resume_dag_id: Optional prior DAG campaign ID (`cdx-...`) to resume settled nodes from.
        timeout_seconds: Per-node execution timeout budget in seconds.
        max_input_bytes: Maximum UTF-8 prompt byte budget per node.
        request_id: Optional caller-supplied campaign identifier (`cdx-...`).
        idempotency_key: Optional idempotency key for deduplicating campaign persistence.

    Returns:
        Pretty-printed JSON string containing the `DagUlwResponseDict` payload.
    """
    parsed_nodes = tuple(DagNodeInputModel.model_validate(obj=n) for n in (nodes or []))
    parsed_spawn = DagSpawnPlanInputModel.model_validate(obj=spawn_plan) if spawn_plan else None
    req = DagUlwRequestModel(
        campaign_goal=campaign_goal,
        nodes=parsed_nodes,
        workspace_dir=workspace_dir,
        plan_only=plan_only,
        spawn_plan=parsed_spawn,
        resume_dag_id=resume_dag_id,
        timeout_seconds=timeout_seconds,
        max_input_bytes=max_input_bytes,
        request_id=request_id,
        idempotency_key=idempotency_key,
    )
    return await _srv._dag_ulw_service.run_dag_campaign(
        req=req,
        job_store=_srv.job_store,
        run_agy_fn=_srv.run_agy_model_prompt,
        execute_fn=_srv._execute_with_engine,
    )


__all__ = [
    "claude_code_craft",
    "codex_alexandria_decision_pack",
    "heterarchy_dag_ulw",
    "heterarchy_triad_orchestrate",
]
