"""Auxiliary Codex runners for Git review and session resume/fork."""

from __future__ import annotations

import asyncio
import os
import tempfile
import time
import uuid
from datetime import UTC, datetime
from pathlib import Path

from codex_collab_mcp.evidence.workspace_digests import resolve_workspace_dir
from codex_collab_mcp.routing.auth_redaction import classify_stderr_and_auth_boundaries
from codex_collab_mcp.routing.policy_engine import (
    build_execution_receipt,
    build_rejected_pre_dispatch_response,
    validate_and_resolve_route,
)
from codex_collab_mcp.schemas import JsonObject, as_json_object
from codex_collab_mcp.transports.codex_cli_executor import (
    CodexCliProcessRunner,
    CodexCommandBuilder,
    CodexOutputParser,
    ResolveBinaryFn,
)


class CodexAuxiliaryRunner:
    """Runner for `codex review` and `codex exec resume/fork`."""

    @classmethod
    async def run_git_review(
        cls,
        workspace_dir: str | None = None,
        custom_instructions: str = "",
        uncommitted: bool = True,
        base_branch: str | None = None,
        commit_sha: str | None = None,
        model: str | None = None,
        reasoning_effort: str | None = None,
        timeout_seconds: float = 300.0,
        *,
        resolve_binary_fn: ResolveBinaryFn | None = None,
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
            resolve_binary_fn: Optional callable used to resolve the `codex` binary path.

        Returns:
            Review execution result dictionary with receipt and auth boundary diagnostics.
        """
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        started_mono = time.monotonic()
        resolved_cwd = str(resolve_workspace_dir(workspace_dir or os.getcwd()))
        req_id = f"cdx-{uuid.uuid4().hex[:10]}"
        call_path = "runner.run_codex_git_review -> subprocess(codex review)"

        route = validate_and_resolve_route("codex_cli", model, "openai_codex", reasoning_effort)
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            return build_rejected_pre_dispatch_response(
                req_id=req_id,
                idempotency_key=None,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
                extra_fields={"workspace_dir": resolved_cwd},
            )

        codex_bin = CodexCommandBuilder.resolve_binary(resolve_binary_fn)
        has_prompt = bool(custom_instructions.strip())
        cmd = CodexCommandBuilder.build_review_command(
            codex_bin=codex_bin,
            uncommitted=uncommitted,
            base_branch=base_branch,
            commit_sha=commit_sha,
            model=model,
            reasoning_effort=reasoning_effort,
            has_custom_prompt=has_prompt,
        )

        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdin=asyncio.subprocess.PIPE if has_prompt else None,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=resolved_cwd,
        )

        try:
            stdout_bytes, stderr_bytes = await CodexCliProcessRunner.communicate_with_cleanup(
                proc,
                custom_instructions.encode("utf-8") if has_prompt else None,
                timeout_seconds,
            )
        except TimeoutError:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            timeout_msg = f"codex review timed out after {duration}s"
            receipt = build_execution_receipt(
                request_id=req_id,
                idempotency_key=None,
                route=route,
                actual_model=route.resolved_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status="timed_out_unknown",
                outcome_state="outcome_unknown",
                failure_reason=timeout_msg,
            )
            return {
                "ok": False,
                "status": "timed_out_unknown",
                "outcome_state": "outcome_unknown",
                "request_id": req_id,
                "workspace_dir": resolved_cwd,
                "error": timeout_msg,
                "receipt": as_json_object(receipt),
            }

        stdout_text = stdout_bytes.decode("utf-8", errors="replace").strip()
        stderr_text = stderr_bytes.decode("utf-8", errors="replace").strip()
        auth_diag = classify_stderr_and_auth_boundaries(stderr_text)
        sanitized_stderr = auth_diag["sanitized_stderr"]
        ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        duration = round(time.monotonic() - started_mono, 2)
        exit_ok = proc.returncode == 0
        review_out = stdout_text or sanitized_stderr

        receipt = build_execution_receipt(
            request_id=req_id,
            idempotency_key=None,
            route=route,
            actual_model=route.resolved_model,
            call_path=call_path,
            started_at_iso=started_iso,
            ended_at_iso=ended_iso,
            duration_seconds=duration,
            status="succeeded" if exit_ok else "failed",
            outcome_state="confirmed_success" if exit_ok else "confirmed_failure",
            output_text=review_out if exit_ok else None,
            failure_reason=sanitized_stderr if not exit_ok else None,
        )

        return {
            "ok": exit_ok,
            "status": "succeeded" if exit_ok else "failed",
            "outcome_state": "confirmed_success" if exit_ok else "confirmed_failure",
            "request_id": req_id,
            "exit_code": proc.returncode,
            "workspace_dir": resolved_cwd,
            "review_output": review_out,
            "stderr": sanitized_stderr if not exit_ok else "",
            "receipt": as_json_object(receipt),
            "auth_boundary": as_json_object(auth_diag),
        }

    @classmethod
    async def run_session_followup(
        cls,
        session_id: str,
        prompt: str,
        workspace_dir: str | None = None,
        fork: bool = False,
        model: str | None = None,
        timeout_seconds: float = 300.0,
        *,
        resolve_binary_fn: ResolveBinaryFn | None = None,
    ) -> JsonObject:
        """Resume or fork an existing Codex session by session_id (`codex exec resume/fork`).

        Args:
            session_id: Existing thread/session identifier to resume or fork.
            prompt: Follow-up prompt text sent via stdin.
            workspace_dir: Target workspace directory path.
            fork: If True, fork into a new session instead of resuming in place.
            model: Optional explicit Codex model identifier.
            timeout_seconds: Maximum execution time in seconds.
            resolve_binary_fn: Optional callable used to resolve the `codex` binary path.

        Returns:
            Follow-up execution result dictionary with session IDs and receipt.
        """
        started_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        started_mono = time.monotonic()
        resolved_cwd = str(resolve_workspace_dir(workspace_dir or os.getcwd()))
        req_id = f"cdx-{uuid.uuid4().hex[:10]}"
        subcmd = "fork" if fork else "resume"
        call_path = f"runner.run_codex_session_followup -> subprocess(codex exec {subcmd})"

        sid = session_id.strip()
        if not sid or sid.startswith("-"):
            raise ValueError("Session ID must be a non-empty string not starting with '-'.")
        route = validate_and_resolve_route("codex_cli", model, "openai_codex")
        if not route.valid:
            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            return build_rejected_pre_dispatch_response(
                req_id=req_id,
                idempotency_key=None,
                route=route,
                call_path=call_path,
                started_iso=started_iso,
                ended_iso=ended_iso,
            )

        codex_bin = CodexCommandBuilder.resolve_binary(resolve_binary_fn)

        with tempfile.TemporaryDirectory(prefix="codex_resume_") as tmp_dir:
            output_file = Path(tmp_dir) / "last_message.txt"
            cmd = [
                codex_bin,
                "exec",
                subcmd,
                sid,
                "--ignore-user-config",
                "-c",
                "mcp_servers={}",
                "--cd",
                resolved_cwd,
                "--skip-git-repo-check",
                "--json",
                "-o",
                str(output_file),
            ]
            if model and (model_s := model.strip()) and model_s != "codex-default":
                if model_s.startswith("-"):
                    raise ValueError("Codex model identifier must not start with '-'.")
                cmd.extend(["-m", model_s])
            cmd.append("-")

            proc = await asyncio.create_subprocess_exec(
                *cmd,
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=resolved_cwd,
            )

            try:
                stdout_bytes, stderr_bytes = await CodexCliProcessRunner.communicate_with_cleanup(
                    proc,
                    prompt.encode("utf-8"),
                    timeout_seconds,
                )
            except TimeoutError:
                ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
                duration = round(time.monotonic() - started_mono, 2)
                timeout_msg = f"codex exec {subcmd} timed out after {duration}s"
                receipt = build_execution_receipt(
                    request_id=req_id,
                    idempotency_key=None,
                    route=route,
                    actual_model=route.resolved_model,
                    call_path=call_path,
                    started_at_iso=started_iso,
                    ended_at_iso=ended_iso,
                    duration_seconds=duration,
                    status="timed_out_unknown",
                    outcome_state="outcome_unknown",
                    failure_reason=timeout_msg,
                )
                return {
                    "ok": False,
                    "status": "timed_out_unknown",
                    "outcome_state": "outcome_unknown",
                    "request_id": req_id,
                    "mode": subcmd,
                    "parent_session_id": session_id,
                    "error": timeout_msg,
                    "receipt": as_json_object(receipt),
                }

            stdout_text = stdout_bytes.decode("utf-8", errors="replace")
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            new_session_id, event_messages, reported_model = (
                CodexOutputParser.parse_jsonl_events_with_model(stdout_text)
            )
            auth_diag = classify_stderr_and_auth_boundaries(stderr_text)
            sanitized_stderr = auth_diag["sanitized_stderr"]
            raw_response = CodexOutputParser.resolve_raw_response(
                output_file,
                event_messages,
                stdout_text,
            )

            ended_iso = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
            duration = round(time.monotonic() - started_mono, 2)
            exit_ok = proc.returncode == 0
            actual_model = reported_model or route.resolved_model
            receipt = build_execution_receipt(
                request_id=req_id,
                idempotency_key=None,
                route=route,
                actual_model=actual_model,
                call_path=call_path,
                started_at_iso=started_iso,
                ended_at_iso=ended_iso,
                duration_seconds=duration,
                status="succeeded" if exit_ok else "failed",
                outcome_state="confirmed_success" if exit_ok else "confirmed_failure",
                output_text=raw_response if exit_ok else None,
                failure_reason=sanitized_stderr if not exit_ok else None,
            )

            return {
                "ok": exit_ok,
                "status": "succeeded" if exit_ok else "failed",
                "outcome_state": "confirmed_success" if exit_ok else "confirmed_failure",
                "request_id": req_id,
                "exit_code": proc.returncode,
                "mode": subcmd,
                "parent_session_id": session_id,
                "session_id": new_session_id or session_id,
                "response": raw_response,
                "stderr": sanitized_stderr if not exit_ok else "",
                "receipt": as_json_object(receipt),
                "auth_boundary": as_json_object(auth_diag),
            }
