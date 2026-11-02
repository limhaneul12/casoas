"""MCP tool router for OpenAI Codex audit, synthesis, rescue, review, and follow-up tools."""

from __future__ import annotations

import json

import app.server as _srv
from app.core.collab_enums import (
    ReasoningTier,
    SchemaName,
)
from app.core.model_contract_enums import (
    EngineAliasKind,
    OptimizationAxisKind,
    SandboxModeKind,
    TaskTypeKind,
)
from app.core.prompts import (
    build_adversarial_audit_prompt,
    build_differential_candidates_prompt,
    build_rescue_prompt,
    build_targeted_synthesis_prompt,
)
from app.schemas.boundary_schemas import JsonObject
from app.schemas.inner_dto import CodexSchemaToolRequest


@_srv.mcp.tool()
async def codex_adversarial_audit(
    target_code_or_diff: str,
    design_rationale: str = "",
    attack_vectors: list[str] | None = None,
    context_notes: str = "",
    workspace_dir: str = ".",
    model: str | None = None,
    reasoning_effort: str = ReasoningTier.XHIGH,
    async_background: bool = False,
    engine: str = EngineAliasKind.CLI,
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
    prompt = build_adversarial_audit_prompt(
        target_code_or_diff=target_code_or_diff,
        design_rationale=design_rationale,
        attack_vectors=attack_vectors or [],
        context_notes=context_notes,
    )
    req = CodexSchemaToolRequest(
        task_type=SchemaName.ADVERSARIAL_AUDIT,
        prompt=prompt,
        provided_context_or_code=target_code_or_diff,
        workspace_dir=workspace_dir,
        sandbox_mode=SandboxModeKind.READ_ONLY,
        model=model,
        reasoning_effort=reasoning_effort,
        async_background=async_background,
        engine=engine,
        timeout_seconds=timeout_seconds,
        provider=provider,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        files_inspected=files_inspected,
        symbols_inspected=symbols_inspected,
        executed_test_commands=executed_test_commands,
    )
    return await _srv._gateway.dispatch_codex_schema_tool(
        req=req,
        job_store=_srv.job_store,
        execute_fn=_srv._execute_with_engine,
    )


@_srv.mcp.tool()
async def codex_targeted_synthesize(
    signature: str,
    contract_and_constraints: str,
    context_types: str = "",
    language: str = "python",
    include_unit_tests: bool = True,
    workspace_dir: str = ".",
    model: str | None = None,
    reasoning_effort: str = ReasoningTier.HIGH,
    engine: str = EngineAliasKind.CLI,
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
    prompt = build_targeted_synthesis_prompt(
        signature=signature,
        contract_and_constraints=contract_and_constraints,
        context_types=context_types,
        language=language,
        include_unit_tests=include_unit_tests,
    )
    inline_ctx = f"{signature}\n{contract_and_constraints}\n{context_types}"
    req = CodexSchemaToolRequest(
        task_type=SchemaName.TARGETED_SYNTHESIS,
        prompt=prompt,
        provided_context_or_code=inline_ctx,
        workspace_dir=workspace_dir,
        sandbox_mode=SandboxModeKind.READ_ONLY,
        model=model,
        reasoning_effort=reasoning_effort,
        async_background=False,
        engine=engine,
        timeout_seconds=timeout_seconds,
        provider=provider,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        files_inspected=files_inspected,
        symbols_inspected=symbols_inspected,
        executed_test_commands=executed_test_commands,
    )
    return await _srv._gateway.dispatch_codex_schema_tool(
        req=req,
        job_store=_srv.job_store,
        execute_fn=_srv._execute_with_engine,
    )


@_srv.mcp.tool()
async def codex_differential_implementations(
    problem_spec: str,
    optimization_axes: list[str] | None = None,
    context_types: str = "",
    language: str = "python",
    workspace_dir: str = ".",
    model: str | None = None,
    reasoning_effort: str = ReasoningTier.HIGH,
    engine: str = EngineAliasKind.CLI,
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
    default_axes = [str(axis) for axis in OptimizationAxisKind]
    prompt = build_differential_candidates_prompt(
        problem_spec=problem_spec,
        optimization_axes=optimization_axes or default_axes,
        context_types=context_types,
        language=language,
    )
    req = CodexSchemaToolRequest(
        task_type=SchemaName.DIFFERENTIAL_CANDIDATES,
        prompt=prompt,
        provided_context_or_code=f"{problem_spec}\n{context_types}",
        workspace_dir=workspace_dir,
        sandbox_mode=SandboxModeKind.READ_ONLY,
        model=model,
        reasoning_effort=reasoning_effort,
        async_background=False,
        engine=engine,
        timeout_seconds=timeout_seconds,
        provider=provider,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        files_inspected=files_inspected,
        symbols_inspected=symbols_inspected,
        executed_test_commands=None,
    )
    return await _srv._gateway.dispatch_codex_schema_tool(
        req=req,
        job_store=_srv.job_store,
        execute_fn=_srv._execute_with_engine,
    )


@_srv.mcp.tool()
async def codex_rescue_sandbox(
    error_trace: str,
    target_code: str,
    previous_attempts: str = "",
    reproduction_command: str = "",
    workspace_dir: str = ".",
    sandbox_mode: str = SandboxModeKind.READ_ONLY,
    model: str | None = None,
    reasoning_effort: str = ReasoningTier.XHIGH,
    async_background: bool = False,
    engine: str = EngineAliasKind.CLI,
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
    prompt = build_rescue_prompt(
        error_trace=error_trace,
        target_code=target_code,
        previous_attempts=previous_attempts,
        reproduction_command=reproduction_command,
    )
    req = CodexSchemaToolRequest(
        task_type=TaskTypeKind.RESCUE_SANDBOX,
        prompt=prompt,
        provided_context_or_code=f"{error_trace}\n{target_code}",
        workspace_dir=workspace_dir,
        sandbox_mode=sandbox_mode,
        model=model,
        reasoning_effort=reasoning_effort,
        async_background=async_background,
        engine=engine,
        timeout_seconds=timeout_seconds,
        provider=provider,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        files_inspected=files_inspected,
        symbols_inspected=symbols_inspected,
        executed_test_commands=executed_test_commands,
    )
    return await _srv._gateway.dispatch_codex_schema_tool(
        req=req,
        job_store=_srv.job_store,
        execute_fn=_srv._execute_with_engine,
    )


@_srv.mcp.tool()
async def codex_git_review(
    workspace_dir: str = ".",
    custom_instructions: str = "",
    uncommitted: bool = True,
    base_branch: str | None = None,
    commit_sha: str | None = None,
    model: str | None = None,
    reasoning_effort: str = ReasoningTier.HIGH,
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
    res = await _srv.run_codex_git_review(
        workspace_dir=workspace_dir,
        custom_instructions=custom_instructions,
        uncommitted=uncommitted,
        base_branch=base_branch,
        commit_sha=commit_sha,
        model=model,
        reasoning_effort=reasoning_effort,
        timeout_seconds=timeout_seconds,
    )
    return json.dumps(res, ensure_ascii=False, indent=2)


@_srv.mcp.tool()
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
    res = await _srv.run_codex_session_followup(
        session_id=session_id,
        prompt=prompt,
        workspace_dir=workspace_dir,
        fork=fork,
        model=model,
        timeout_seconds=timeout_seconds,
    )
    return json.dumps(res, ensure_ascii=False, indent=2)


__all__ = [
    "codex_adversarial_audit",
    "codex_differential_implementations",
    "codex_git_review",
    "codex_rescue_sandbox",
    "codex_session_followup",
    "codex_targeted_synthesize",
]
