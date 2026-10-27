"""FastMCP Server for Gemini 4 Argon <-> Claude Sonnet 5.5 <-> OpenAI Codex Heterarchy."""

from __future__ import annotations

import asyncio
import json

from mcp.server.mcpserver import MCPServer

from codex_collab_mcp.agy_bridge import (
    DEFAULT_CLAUDE_MODEL,
    discover_agy_endpoint,
    list_agy_models,
    run_agy_model_prompt,
)
from codex_collab_mcp.domain.service_dicts import (
    ClaudeCraftRequest,
    CodexSchemaToolRequest,
    DecisionPackRequest,
    EngineDispatchRequest,
    ServerGatewayBindings,
)
from codex_collab_mcp.execution.decision_pack_service import AlexandriaDecisionPackService
from codex_collab_mcp.execution.mcp_gateway_service import McpExecutionGateway
from codex_collab_mcp.execution.mcp_tool_handlers import McpToolDispatcher
from codex_collab_mcp.execution.triad_pipeline_service import TriadOrchestrationService
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.prompts import (
    build_adversarial_audit_prompt,
    build_differential_candidates_prompt,
    build_rescue_prompt,
    build_targeted_synthesis_prompt,
)
from codex_collab_mcp.runner import (
    resolve_codex_binary,
    run_codex_exec,
    run_codex_git_review,
    run_codex_session_followup,
    run_http_fallback,
)
from codex_collab_mcp.schemas import JsonObject

_INSTRUCTIONS = (
    "Tri-Polar Heterarchy collaboration & orchestration bridge pairing "
    "Gemini 4 Argon (macro-architect), Claude Sonnet 5.5 / Opus 5.5 (surgical code craftsman), "
    "and OpenAI Codex (adversarial red-team auditor, high-speed synthesizer, rescue sandbox)."
)
_DEFAULT_AXES = ["latency_and_throughput", "memory_footprint", "maintainability_and_clarity"]
mcp = MCPServer("codex-collab-mcp", instructions=_INSTRUCTIONS)

job_store = JobStore()
handoff_store = HandoffStore()
_background_tasks: set[asyncio.Task[None]] = set()
_background_job_semaphore: asyncio.Semaphore = asyncio.Semaphore(8)
_gateway = McpExecutionGateway(_background_tasks, _background_job_semaphore)
_tool_dispatcher = McpToolDispatcher()
_triad_service = TriadOrchestrationService()
_decision_pack_service = AlexandriaDecisionPackService()


async def _execute_with_engine(
    prompt: str,
    schema_name: str,
    output_schema: JsonObject | None,
    workspace_dir: str | None = None,
    sandbox_mode: str = "read-only",
    model: str | None = None,
    reasoning_effort: str | None = None,
    engine: str = "cli",
    timeout_seconds: float = 300.0,
    *,
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
    c = (prompt, schema_name, output_schema, workspace_dir, sandbox_mode, model, reasoning_effort)
    o = (engine, timeout_seconds, provider, request_id, idempotency_key, retry_same_request)
    req = EngineDispatchRequest(*c, *o, max_input_bytes)
    b = ServerGatewayBindings(job_store, run_agy_model_prompt, run_http_fallback, run_codex_exec)
    return await _gateway.execute_with_engine(req, b)


@mcp.tool()
async def codex_adversarial_audit(
    target_code_or_diff: str,
    design_rationale: str = "",
    attack_vectors: list[str] | None = None,
    context_notes: str = "",
    workspace_dir: str = ".",
    model: str | None = None,
    reasoning_effort: str = "xhigh",
    async_background: bool = False,
    engine: str = "cli",
    timeout_seconds: float = 300.0,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    files_inspected: list[str] | None = None,
    symbols_inspected: list[str] | None = None,
    executed_test_commands: list[JsonObject] | None = None,
    max_input_bytes: int = 256_000,
) -> str:
    """Run an uncompromising Red-Team / Adversarial Audit on code or diffs from Gemini 4 Argon.

    Finds subtle race conditions, boundary bugs, security flaws, contract violations,
    and returns concrete failing test cases plus an Alexandria decision summary and receipt.

    Args:
        target_code_or_diff: Code snippet or unified diff to audit.
        design_rationale: Architectural intent and invariants for the target code.
        attack_vectors: Optional list of specific failure or security vectors to probe.
        context_notes: Additional repository or caller context notes.
        workspace_dir: Target workspace directory path.
        model: Optional model identifier or alias.
        reasoning_effort: Codex reasoning effort tier (`low`, `medium`, `high`, `xhigh`).
        async_background: Whether to dispatch the audit asynchronously in the background.
        engine: Execution engine identifier (`cli`, `http`, or `antigravity_local_rpc`).
        timeout_seconds: Execution timeout budget in seconds.
        provider: Optional explicit provider name.
        request_id: Optional explicit request identifier.
        idempotency_key: Optional idempotency key for safe deduplication/retry.
        retry_same_request: Whether to explicitly retry an existing request ID.
        files_inspected: Optional relative file paths inspected for evidence scope.
        symbols_inspected: Optional symbol names inspected for evidence scope.
        executed_test_commands: Optional caller-reported test command records.
        max_input_bytes: Maximum UTF-8 prompt byte budget.

    Returns:
        Pretty-printed JSON string containing the adversarial audit findings and receipt.
    """
    p_in = (target_code_or_diff, design_rationale, attack_vectors or [], context_notes)
    c = ("adversarial_audit", build_adversarial_audit_prompt(*p_in), target_code_or_diff)
    e = (
        workspace_dir,
        "read-only",
        model,
        reasoning_effort,
        async_background,
        engine,
        timeout_seconds,
    )
    rq = (provider, request_id, idempotency_key, retry_same_request, max_input_bytes)
    ev = (files_inspected, symbols_inspected, executed_test_commands)
    return await _gateway.dispatch_codex_schema_tool(
        CodexSchemaToolRequest(*c, *e, *rq, *ev), job_store, _execute_with_engine
    )


@mcp.tool()
async def codex_targeted_synthesize(
    signature: str,
    contract_and_constraints: str,
    context_types: str = "",
    language: str = "python",
    include_unit_tests: bool = True,
    workspace_dir: str = ".",
    model: str | None = None,
    reasoning_effort: str = "high",
    engine: str = "cli",
    timeout_seconds: float = 240.0,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    files_inspected: list[str] | None = None,
    symbols_inspected: list[str] | None = None,
    executed_test_commands: list[JsonObject] | None = None,
    max_input_bytes: int = 256_000,
) -> str:
    """Offload high-speed, zero-fluff code and unit test synthesis to Codex from Argon's spec.

    Ideal when Gemini 4 Argon has established the architectural interfaces and type contracts
    and wants Codex to synthesize deterministic implementation blocks and edge-case tests.

    Args:
        signature: Target function, class, or module signature to synthesize.
        contract_and_constraints: Behavioral contract, invariants, and complexity bounds.
        context_types: Supporting type definitions and imports.
        language: Target programming language.
        include_unit_tests: Whether to synthesize unit tests alongside the implementation.
        workspace_dir: Target workspace directory path.
        model: Optional model identifier or alias.
        reasoning_effort: Codex reasoning effort tier.
        engine: Execution engine identifier.
        timeout_seconds: Execution timeout budget in seconds.
        provider: Optional explicit provider name.
        request_id: Optional explicit request identifier.
        idempotency_key: Optional idempotency key for safe deduplication/retry.
        retry_same_request: Whether to explicitly retry an existing request ID.
        files_inspected: Optional relative file paths inspected for evidence scope.
        symbols_inspected: Optional symbol names inspected for evidence scope.
        executed_test_commands: Optional caller-reported test command records.
        max_input_bytes: Maximum UTF-8 prompt byte budget.

    Returns:
        Pretty-printed JSON string containing the synthesized code, tests, and receipt.
    """
    p_in = (signature, contract_and_constraints, context_types, language, include_unit_tests)
    inline_ctx = f"{signature}\n{contract_and_constraints}\n{context_types}"
    c = ("targeted_synthesis", build_targeted_synthesis_prompt(*p_in), inline_ctx)
    e = (workspace_dir, "read-only", model, reasoning_effort, False, engine, timeout_seconds)
    rq = (provider, request_id, idempotency_key, retry_same_request, max_input_bytes)
    ev = (files_inspected, symbols_inspected, executed_test_commands)
    return await _gateway.dispatch_codex_schema_tool(
        CodexSchemaToolRequest(*c, *e, *rq, *ev), job_store, _execute_with_engine
    )


@mcp.tool()
async def codex_differential_implementations(
    problem_spec: str,
    optimization_axes: list[str] | None = None,
    context_types: str = "",
    language: str = "python",
    workspace_dir: str = ".",
    model: str | None = None,
    reasoning_effort: str = "high",
    engine: str = "cli",
    timeout_seconds: float = 300.0,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    files_inspected: list[str] | None = None,
    symbols_inspected: list[str] | None = None,
    max_input_bytes: int = 256_000,
) -> str:
    """Synthesize multiple candidate implementations optimized along distinct trade-off axes.

    Default axes: ['latency_and_throughput', 'memory_footprint', 'maintainability_and_clarity'].
    Returns each candidate's full code, pros/cons, and a selection guide for Gemini 4 Argon.

    Args:
        problem_spec: Specification of the algorithmic or architectural problem.
        optimization_axes: Optional list of distinct trade-off axes to optimize across.
        context_types: Supporting type definitions and interfaces.
        language: Target programming language.
        workspace_dir: Target workspace directory path.
        model: Optional model identifier or alias.
        reasoning_effort: Codex reasoning effort tier.
        engine: Execution engine identifier.
        timeout_seconds: Execution timeout budget in seconds.
        provider: Optional explicit provider name.
        request_id: Optional explicit request identifier.
        idempotency_key: Optional idempotency key for safe deduplication/retry.
        retry_same_request: Whether to explicitly retry an existing request ID.
        files_inspected: Optional relative file paths inspected for evidence scope.
        symbols_inspected: Optional symbol names inspected for evidence scope.
        max_input_bytes: Maximum UTF-8 prompt byte budget.

    Returns:
        Pretty-printed JSON string containing the differential candidates and receipt.
    """
    p_in = (problem_spec, optimization_axes or _DEFAULT_AXES, context_types, language)
    c = (
        "differential_candidates",
        build_differential_candidates_prompt(*p_in),
        f"{problem_spec}\n{context_types}",
    )
    e = (workspace_dir, "read-only", model, reasoning_effort, False, engine, timeout_seconds)
    rq = (provider, request_id, idempotency_key, retry_same_request, max_input_bytes)
    ev = (files_inspected, symbols_inspected, None)
    return await _gateway.dispatch_codex_schema_tool(
        CodexSchemaToolRequest(*c, *e, *rq, *ev), job_store, _execute_with_engine
    )


@mcp.tool()
async def codex_rescue_sandbox(
    error_trace: str,
    target_code: str,
    previous_attempts: str = "",
    reproduction_command: str = "",
    workspace_dir: str = ".",
    sandbox_mode: str = "read-only",
    model: str | None = None,
    reasoning_effort: str = "xhigh",
    async_background: bool = False,
    engine: str = "cli",
    timeout_seconds: float = 300.0,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    files_inspected: list[str] | None = None,
    symbols_inspected: list[str] | None = None,
    executed_test_commands: list[JsonObject] | None = None,
    max_input_bytes: int = 256_000,
) -> str:
    """Delegate a stubborn bug or failing test to an isolated Codex rescue session.

    Set `sandbox_mode='workspace-write'` if Codex should actively run commands or edit files,
    or keep `sandbox_mode='read-only'` for safe root-cause analysis and patch generation.
    Set `async_background=True` to run non-blocking in the background.

    Args:
        error_trace: Failing stack trace, compiler error, or test failure log.
        target_code: Suspect source code or module context.
        previous_attempts: Summary of prior debugging or fix attempts.
        reproduction_command: Command used to reproduce the failure.
        workspace_dir: Target workspace directory path.
        sandbox_mode: Execution sandbox mode (`read-only` or `workspace-write`).
        model: Optional model identifier or alias.
        reasoning_effort: Codex reasoning effort tier.
        async_background: Whether to run the rescue job asynchronously in the background.
        engine: Execution engine identifier.
        timeout_seconds: Execution timeout budget in seconds.
        provider: Optional explicit provider name.
        request_id: Optional explicit request identifier.
        idempotency_key: Optional idempotency key for safe deduplication/retry.
        retry_same_request: Whether to explicitly retry an existing request ID.
        files_inspected: Optional relative file paths inspected for evidence scope.
        symbols_inspected: Optional symbol names inspected for evidence scope.
        executed_test_commands: Optional caller-reported test command records.
        max_input_bytes: Maximum UTF-8 prompt byte budget.

    Returns:
        Pretty-printed JSON string containing the rescue diagnosis and receipt.
    """
    p_in = (error_trace, target_code, previous_attempts, reproduction_command)
    c = ("rescue_sandbox", build_rescue_prompt(*p_in), f"{error_trace}\n{target_code}")
    e = (
        workspace_dir,
        sandbox_mode,
        model,
        reasoning_effort,
        async_background,
        engine,
        timeout_seconds,
    )
    rq = (provider, request_id, idempotency_key, retry_same_request, max_input_bytes)
    ev = (files_inspected, symbols_inspected, executed_test_commands)
    return await _gateway.dispatch_codex_schema_tool(
        CodexSchemaToolRequest(*c, *e, *rq, *ev), job_store, _execute_with_engine
    )


@mcp.tool()
async def codex_git_review(
    workspace_dir: str = ".",
    custom_instructions: str = "",
    uncommitted: bool = True,
    base_branch: str | None = None,
    commit_sha: str | None = None,
    model: str | None = None,
    reasoning_effort: str = "high",
    timeout_seconds: float = 300.0,
) -> str:
    """Run native `codex review` on uncommitted changes, a base branch, or a commit SHA.

    Args:
        workspace_dir: Git repository workspace directory path.
        custom_instructions: Optional focus instructions for the review.
        uncommitted: Whether to review uncommitted working tree changes.
        base_branch: Optional base branch name to diff against.
        commit_sha: Optional specific commit SHA to review.
        model: Optional Codex model identifier.
        reasoning_effort: Codex reasoning effort tier.
        timeout_seconds: Execution timeout budget in seconds.

    Returns:
        Pretty-printed JSON string containing the native Codex review output.
    """
    a = (workspace_dir, custom_instructions, uncommitted, base_branch, commit_sha)
    res = await run_codex_git_review(*a, model, reasoning_effort, timeout_seconds)
    return json.dumps(res, ensure_ascii=False, indent=2)


@mcp.tool()
async def codex_session_followup(
    session_id: str,
    prompt: str,
    workspace_dir: str = ".",
    fork: bool = False,
    model: str | None = None,
    timeout_seconds: float = 300.0,
) -> str:
    """Continue (`resume`) or branch (`fork`) a previous Codex session by its `session_id`.

    Enables multi-turn debate or iterative refinement between Gemini 4 Argon and Codex.

    Args:
        session_id: Existing Codex session/thread identifier (`thr_...`).
        prompt: Follow-up prompt or rebuttal message.
        workspace_dir: Target workspace directory path.
        fork: Whether to fork into a new session branch instead of resuming in place.
        model: Optional Codex model identifier.
        timeout_seconds: Execution timeout budget in seconds.

    Returns:
        Pretty-printed JSON string containing the follow-up session response.
    """
    a = (session_id, prompt, workspace_dir, fork, model, timeout_seconds)
    return json.dumps(await run_codex_session_followup(*a), ensure_ascii=False, indent=2)


@mcp.tool()
async def codex_job_status(
    job_id: str = "", idempotency_key: str | None = None, cancel: bool = False
) -> str:
    """Check status, retrieve results, or safely cancel a job by `job_id` or `idempotency_key`.

    Args:
        job_id: Job/request identifier (`cdx-...`) to query.
        idempotency_key: Optional idempotency key fallback when `job_id` is omitted.
        cancel: Whether to request safe cancellation of the target job.

    Returns:
        Pretty-printed JSON string containing the job record and outcome state.
    """
    return _tool_dispatcher.job_status(job_store, job_id, idempotency_key, cancel)


@mcp.tool()
async def codex_job_list(limit: int = 15, workspace_dir: str | None = None) -> str:
    """List recent asynchronous Codex collaboration jobs and their current statuses.

    Args:
        limit: Maximum number of recent jobs to return.
        workspace_dir: Optional workspace directory filter.

    Returns:
        Pretty-printed JSON string listing matching job records.
    """
    return _tool_dispatcher.job_list(job_store, limit, workspace_dir)


@mcp.tool()
async def heterarchy_handoff_create(
    title: str,
    spec_or_payload: str,
    sender: str = "codex",
    recipient: str = "argon",
    workspace_dir: str = ".",
    priority: str = "normal",
) -> str:
    """Create a bidirectional task handoff ticket between GPT/Codex and Antigravity (Argon).

    Note: Ticket creation only places the task in `queued` (`pending`) state.
    It is never completion or consensus evidence until resolved with an output artifact.

    Args:
        title: Concise summary title for the handoff ticket.
        spec_or_payload: Detailed specification, diff, or payload for the recipient.
        sender: Originating pole identifier (`codex`, `argon`, `sonnet`, `opus`).
        recipient: Target pole identifier (`argon`, `codex`, `sonnet`, `opus`).
        workspace_dir: Target workspace directory path.
        priority: Ticket priority level (`low`, `normal`, `high`, `critical`).

    Returns:
        Pretty-printed JSON string describing the newly queued handoff ticket.
    """
    a = (title, spec_or_payload, sender, recipient, workspace_dir, priority)
    return _tool_dispatcher.handoff_create(handoff_store, *a)


@mcp.tool()
async def heterarchy_handoff_inbox(
    recipient: str | None = None,
    status: str | None = "pending",
    workspace_dir: str | None = None,
    limit: int = 20,
) -> str:
    """Read the shared bidirectional handoff inbox for `'argon'` (Antigravity) or `'codex'` (GPT).

    Pass `status=None` or `status=''` to inspect all tickets regardless of status.
    Pass `workspace_dir` to filter tickets for a specific project workspace.

    Args:
        recipient: Optional recipient pole filter (`argon`, `codex`, `sonnet`, `opus`).
        status: Optional ticket status filter (`pending`, `in_progress`, `completed`, `rejected`).
        workspace_dir: Optional workspace directory path filter.
        limit: Maximum number of tickets to return.

    Returns:
        Pretty-printed JSON string containing matching handoff tickets.
    """
    return _tool_dispatcher.handoff_inbox(handoff_store, recipient, status, workspace_dir, limit)


@mcp.tool()
async def heterarchy_handoff_resolve(
    ticket_id: str,
    status: str = "completed",
    resolution_note: str = "",
    alexandria_ref: str = "",
) -> str:
    """Update or resolve a handoff ticket (`in_progress`, `completed`, or `rejected`).

    Args:
        ticket_id: Identifier of the handoff ticket (`hnd-...`) to update.
        status: New ticket status (`in_progress`, `completed`, or `rejected`).
        resolution_note: Optional summary note explaining the resolution.
        alexandria_ref: Optional Alexandria note or decision reference.

    Returns:
        Pretty-printed JSON string containing the updated handoff ticket.
    """
    a = (ticket_id, status, resolution_note, alexandria_ref)
    return _tool_dispatcher.handoff_resolve(handoff_store, *a)


@mcp.tool()
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

    Returns:
        Pretty-printed JSON string containing the Claude craft result and receipt.
    """
    spec = (task_spec, target_code_or_context, invariants_and_rules, language, claude_model)
    opts = (
        timeout_seconds,
        workspace_dir,
        request_id,
        idempotency_key,
        retry_same_request,
        max_input_bytes,
    )
    ev = (files_inspected, symbols_inspected, executed_test_commands)
    return await _gateway.run_claude_code_craft(
        ClaudeCraftRequest(*spec, *opts, *ev), job_store, run_agy_model_prompt
    )


@mcp.tool()
async def heterarchy_triad_orchestrate(
    task_spec: str,
    target_code_or_context: str = "",
    invariants_and_rules: str = "",
    language: str = "python",
    mode: str = "pipeline",
    workspace_dir: str = ".",
    claude_model: str = DEFAULT_CLAUDE_MODEL,
    codex_model: str | None = None,
    codex_reasoning_effort: str = "xhigh",
    argon_model: str = "gemini-4-argon-high",
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

    Returns:
        Pretty-printed JSON string containing the orchestration outcome and evidence scope.
    """
    spec = (task_spec, target_code_or_context, invariants_and_rules, language, mode, workspace_dir)
    models = (claude_model, codex_model, codex_reasoning_effort, argon_model)
    opts = (invoke_argon_rpc, argon_handoff_ticket_id, timeout_seconds, max_input_bytes)
    ev = (files_inspected, symbols_inspected, executed_test_commands)
    stores = (job_store, handoff_store)
    fns = (run_agy_model_prompt, _execute_with_engine)
    cont_opts = (resume_collaboration_id, max_rounds, run_verification_gates)
    return await _triad_service.orchestrate_triad(spec, models, opts, ev, stores, fns, cont_opts)


@mcp.tool()
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
    notes = (
        project_scope,
        topic_title,
        argon_architecture_intent,
        codex_audit_or_synthesis_summary,
        final_resolution,
        sonnet_craft_or_review_summary,
        verification_evidence,
    )
    fences = (collaboration_id, workspace_dir, expected_scope_digest)
    return _decision_pack_service.build_decision_pack(
        DecisionPackRequest(*notes, *fences), job_store
    )


@mcp.tool()
async def codex_bridge_health() -> str:
    """Check health, verified routability, and independent auth boundaries of the bridge.

    Returns:
        Pretty-printed JSON string reporting bridge health and routing contracts.
    """
    probes = (resolve_codex_binary, discover_agy_endpoint, list_agy_models)
    return await _tool_dispatcher.check_bridge_health(probes, handoff_store, len(_background_tasks))


def main() -> None:
    """Entry point for running the FastMCP stdio server."""
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
