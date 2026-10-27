"""Execution engine facade for OpenAI Codex CLI (`codex exec`, `codex review`) and HTTP."""

from __future__ import annotations

from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.schemas import JsonObject
from codex_collab_mcp.transports.codex_aux_runners import CodexAuxiliaryRunner
from codex_collab_mcp.transports.codex_cli_executor import (
    CodexCliProcessRunner,
    CodexCommandBuilder,
    CodexOutputParser,
)
from codex_collab_mcp.transports.codex_http_fallback import CodexHttpFallbackRunner

resolve_codex_binary = CodexCommandBuilder.resolve_binary
build_codex_exec_command = CodexCommandBuilder.build_exec_command
build_codex_review_command = CodexCommandBuilder.build_review_command
parse_jsonl_events_with_model = CodexOutputParser.parse_jsonl_events_with_model
run_http_fallback = CodexHttpFallbackRunner.run_http_fallback

__all__ = [
    "build_codex_exec_command",
    "build_codex_review_command",
    "parse_jsonl_events_with_model",
    "resolve_codex_binary",
    "run_codex_exec",
    "run_codex_git_review",
    "run_codex_session_followup",
    "run_http_fallback",
]


async def run_codex_exec(
    prompt: str,
    workspace_dir: str | None = None,
    output_schema: JsonObject | None = None,
    sandbox_mode: str = "read-only",
    model: str | None = None,
    reasoning_effort: str | None = None,
    timeout_seconds: float = 300.0,
    *,
    provider: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    retry_same_request: bool = False,
    max_input_bytes: int = 256_000,
    job_store: JobStore | None = None,
) -> JsonObject:
    """Execute a non-interactive Codex task via `codex exec` with verified routing and receipts.

    Args:
        prompt: Task prompt piped to `codex exec` over stdin.
        workspace_dir: Target workspace directory path.
        output_schema: Optional strict JSON schema for structured output.
        sandbox_mode: Sandbox mode passed to `--sandbox`.
        model: Optional Codex model identifier.
        reasoning_effort: Optional model reasoning effort level.
        timeout_seconds: Maximum execution time in seconds before timeout handling.
        provider: Optional explicit provider name for route verification.
        request_id: Optional caller-specified request ID (`cdx-...`).
        idempotency_key: Optional idempotency key for deduplicating dispatches.
        retry_same_request: Whether to re-dispatch an existing idempotency key.
        max_input_bytes: Maximum UTF-8 byte budget for `prompt`.
        job_store: Optional `JobStore` instance for durable state persistence.

    Returns:
        Execution result dictionary with receipt, budget envelope, and structured output.
    """
    return await CodexCliProcessRunner.run_exec(
        prompt=prompt,
        workspace_dir=workspace_dir,
        output_schema=output_schema,
        sandbox_mode=sandbox_mode,
        model=model,
        reasoning_effort=reasoning_effort,
        timeout_seconds=timeout_seconds,
        provider=provider,
        request_id=request_id,
        idempotency_key=idempotency_key,
        retry_same_request=retry_same_request,
        max_input_bytes=max_input_bytes,
        job_store=job_store,
        resolve_binary_fn=resolve_codex_binary,
    )


async def run_codex_git_review(
    workspace_dir: str | None = None,
    custom_instructions: str = "",
    uncommitted: bool = True,
    base_branch: str | None = None,
    commit_sha: str | None = None,
    model: str | None = None,
    reasoning_effort: str | None = None,
    timeout_seconds: float = 300.0,
) -> JsonObject:
    """Run native `codex review` on a Git repository.

    Args:
        workspace_dir: Target Git repository workspace path.
        custom_instructions: Optional custom review instructions piped to stdin.
        uncommitted: Whether to review uncommitted working tree changes.
        base_branch: Optional base branch name for diff review.
        commit_sha: Optional specific commit SHA to review.
        model: Optional explicit Codex model identifier.
        reasoning_effort: Optional reasoning effort configuration.
        timeout_seconds: Maximum execution time in seconds.

    Returns:
        Review execution result dictionary with receipt and auth boundary diagnostics.
    """
    return await CodexAuxiliaryRunner.run_git_review(
        workspace_dir=workspace_dir,
        custom_instructions=custom_instructions,
        uncommitted=uncommitted,
        base_branch=base_branch,
        commit_sha=commit_sha,
        model=model,
        reasoning_effort=reasoning_effort,
        timeout_seconds=timeout_seconds,
        resolve_binary_fn=resolve_codex_binary,
    )


async def run_codex_session_followup(
    session_id: str,
    prompt: str,
    workspace_dir: str | None = None,
    fork: bool = False,
    model: str | None = None,
    timeout_seconds: float = 300.0,
) -> JsonObject:
    """Resume or fork an existing Codex session by session_id (`codex exec resume/fork`).

    Args:
        session_id: Existing thread/session identifier to resume or fork.
        prompt: Follow-up prompt text sent via stdin.
        workspace_dir: Target workspace directory path.
        fork: If True, fork into a new session instead of resuming in place.
        model: Optional explicit Codex model identifier.
        timeout_seconds: Maximum execution time in seconds.

    Returns:
        Follow-up execution result dictionary with session IDs and receipt.
    """
    return await CodexAuxiliaryRunner.run_session_followup(
        session_id=session_id,
        prompt=prompt,
        workspace_dir=workspace_dir,
        fork=fork,
        model=model,
        timeout_seconds=timeout_seconds,
        resolve_binary_fn=resolve_codex_binary,
    )
