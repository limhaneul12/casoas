"""Codex CLI JSONL event parser, structured output parser, and command builders."""

from __future__ import annotations

import json
import os
import shutil
from collections.abc import Callable
from pathlib import Path

from codex_collab_mcp.domain.transport_dicts import CodexCliAuthBoundary
from codex_collab_mcp.execution.agy_rpc_transport import AntigravityModelCatalog
from codex_collab_mcp.job_store import CodexJobRecord
from codex_collab_mcp.schemas import JsonObject, JsonValue

ResolveBinaryFn = Callable[[], str | None]


class CodexOutputParser:
    """Parser for Codex CLI JSONL event streams, structured JSON outputs, and stderr auth state."""

    @staticmethod
    def parse_jsonl_events_with_model(
        stdout_text: str,
    ) -> tuple[str | None, list[str], str | None]:
        """Extract thread/session ID, agent messages, and reported model from JSONL output.

        Args:
            stdout_text: Raw newline-delimited JSON event text emitted by `codex exec --json`.

        Returns:
            Tuple of `(session_id, agent_messages, reported_model)`.
        """
        session_id: str | None = None
        reported_model: str | None = None
        messages: list[str] = []
        for raw_line in stdout_text.splitlines():
            line = raw_line.strip()
            if not line or not line.startswith("{"):
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(event, dict):
                continue

            for key in ("thread_id", "session_id", "conversation_id"):
                val = event.get(key)
                if isinstance(val, str) and val:
                    session_id = val

            for mkey in ("model", "model_id", "actual_model"):
                mval = event.get(mkey)
                if isinstance(mval, str) and mval.strip():
                    reported_model = mval.strip()

            item = event.get("item")
            if isinstance(item, dict) and item.get("type") in ("agent_message", "message"):
                text = item.get("text") or item.get("content")
                if isinstance(text, str) and text.strip():
                    messages.append(text.strip())

            msg = event.get("message")
            if isinstance(msg, str) and msg.strip():
                messages.append(msg.strip())

        return session_id, messages, reported_model

    @staticmethod
    def parse_structured_response(raw_response: str) -> JsonValue:
        """Parse raw text response into JSON value, stripping markdown code fences if present.

        Args:
            raw_response: Raw text emitted by Codex CLI or HTTP completion.

        Returns:
            Parsed JSON value, or a fallback `{"raw_output": raw_response}` dictionary.
        """
        extracted = AntigravityModelCatalog.extract_json_from_model_text(raw_response)
        if extracted is not None:
            return extracted
        try:
            loaded: JsonValue = json.loads(raw_response)
            return loaded
        except json.JSONDecodeError:
            return {"raw_output": raw_response}

    @staticmethod
    def resolve_raw_response(
        output_file: Path,
        event_messages: list[str],
        stdout_text: str,
    ) -> str:
        """Select the final response text from output file, JSONL messages, or raw stdout.

        Args:
            output_file: Path to the `-o` last-message file written by Codex CLI.
            event_messages: Extracted agent message strings from JSONL stdout.
            stdout_text: Raw stdout text fallback.

        Returns:
            Trimmed response text string.
        """
        if output_file.exists():
            return output_file.read_text(encoding="utf-8").strip()
        if event_messages:
            return event_messages[-1]
        return stdout_text.strip()

    @staticmethod
    def prepare_temp_files(
        tmp_dir: str,
        output_schema: JsonObject | None,
    ) -> tuple[Path, str | None]:
        """Prepare the output file path and optional schema file inside a temporary directory.

        Args:
            tmp_dir: Temporary directory path string.
            output_schema: Optional JSON schema object to serialize into `schema.json`.

        Returns:
            Tuple of `(output_file_path, schema_file_path_or_none)`.
        """
        tmp_path = Path(tmp_dir)
        output_file = tmp_path / "last_message.txt"
        if output_schema is None:
            return output_file, None
        schema_file = tmp_path / "schema.json"
        schema_file.write_text(
            json.dumps(output_schema, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )
        return output_file, str(schema_file)


class CodexCommandBuilder:
    """Binary locator and CLI argument builder for `codex exec` and `codex review`."""

    @staticmethod
    def resolve_binary(resolve_binary_fn: ResolveBinaryFn | None = None) -> str:
        """Locate the `codex` CLI binary on the system or via an injected resolver callable.

        Args:
            resolve_binary_fn: Optional zero-argument callable returning the binary path.

        Returns:
            Filesystem path to the `codex` CLI executable.
        """
        err = "Could not locate `codex` CLI binary. Set CODEX_CLI_BIN or install @openai/codex."
        if resolve_binary_fn is not None:
            resolved = resolve_binary_fn()
            if resolved:
                return resolved
            raise FileNotFoundError(err)
        env_bin = os.environ.get("CODEX_CLI_BIN")
        if env_bin and Path(env_bin).exists():
            return env_bin
        found = shutil.which("codex")
        if found:
            return found
        app_bin = (
            "/Applications/ChatGPT.app/Contents/Resources/codex-cli/"
            "CodexCLI.app/Contents/MacOS/codex"
        )
        for candidate in ("/opt/homebrew/bin/codex", "/usr/local/bin/codex", app_bin):
            if Path(candidate).exists():
                return candidate
        raise FileNotFoundError(err)

    @staticmethod
    def build_exec_command(
        codex_bin: str,
        workspace_dir: str,
        output_file: str,
        schema_file: str | None = None,
        sandbox_mode: str = "read-only",
        model: str | None = None,
        reasoning_effort: str | None = None,
    ) -> list[str]:
        """Construct the command list for `codex exec` reading prompt from stdin (`-`).

        Args:
            codex_bin: Path to the `codex` executable.
            workspace_dir: Working directory passed to `--cd`.
            output_file: File path passed to `-o` for capturing the last agent message.
            schema_file: Optional JSON schema file path passed to `--output-schema`.
            sandbox_mode: Execution sandbox policy (`read-only`, `workspace-write`, etc.).
            model: Optional explicit Codex model identifier.
            reasoning_effort: Optional reasoning effort setting (`low`, `medium`, `high`, `xhigh`).

        Returns:
            Command argument list suitable for `asyncio.create_subprocess_exec`.
        """
        cmd: list[str] = [
            codex_bin,
            "exec",
            "--ignore-user-config",
            "-c",
            "mcp_servers={}",
            "--cd",
            workspace_dir,
            "--sandbox",
            sandbox_mode,
            "--skip-git-repo-check",
            "--color",
            "never",
            "--json",
            "-o",
            output_file,
        ]
        if schema_file:
            cmd.extend(["--output-schema", schema_file])
        if model and (model_s := model.strip()) and model_s != "codex-default":
            if model_s.startswith("-"):
                raise ValueError("Codex model identifier must not start with '-'.")
            cmd.extend(["-m", model_s])
        if reasoning_effort and (effort_s := reasoning_effort.strip()):
            if effort_s.startswith("-"):
                raise ValueError("Reasoning effort must not start with '-'.")
            effort_str = json.dumps(effort_s, ensure_ascii=False).replace("\x7f", "\\u007f")
            cmd.extend(["-c", f"model_reasoning_effort={effort_str}"])
        cmd.append("-")
        return cmd

    @staticmethod
    def build_review_command(
        codex_bin: str,
        uncommitted: bool = True,
        base_branch: str | None = None,
        commit_sha: str | None = None,
        model: str | None = None,
        reasoning_effort: str | None = None,
        has_custom_prompt: bool = False,
    ) -> list[str]:
        """Construct the command list for `codex review`.

        Args:
            codex_bin: Path to the `codex` executable.
            uncommitted: Whether to review uncommitted working tree changes.
            base_branch: Optional base branch name for diff comparison.
            commit_sha: Optional specific commit SHA to review.
            model: Optional explicit Codex model identifier.
            reasoning_effort: Optional reasoning effort configuration.
            has_custom_prompt: Whether custom instructions will be piped via stdin (`-`).

        Returns:
            Command argument list for `codex review` (note: top-level `codex review` does not
            accept `--ignore-user-config`, so MCP isolation is enforced via `-c mcp_servers={}`).
        """
        cmd: list[str] = [codex_bin, "review"]
        if commit_sha and (sha_s := commit_sha.strip()):
            if sha_s.startswith("-"):
                raise ValueError("Commit SHA must not start with '-'.")
            cmd.extend(["--commit", sha_s])
        elif base_branch and (branch_s := base_branch.strip()):
            if branch_s.startswith("-"):
                raise ValueError("Base branch must not start with '-'.")
            cmd.extend(["--base", branch_s])
        elif uncommitted:
            cmd.append("--uncommitted")
        cmd.extend(["-c", "mcp_servers={}"])
        if model and (model_s := model.strip()) and model_s != "codex-default":
            if model_s.startswith("-"):
                raise ValueError("Codex model identifier must not start with '-'.")
            model_str = json.dumps(model_s, ensure_ascii=False).replace("\x7f", "\\u007f")
            cmd.extend(["-c", f"model={model_str}"])
        if reasoning_effort and (effort_s := reasoning_effort.strip()):
            if effort_s.startswith("-"):
                raise ValueError("Reasoning effort must not start with '-'.")
            effort_str = json.dumps(effort_s, ensure_ascii=False).replace("\x7f", "\\u007f")
            cmd.extend(["-c", f"model_reasoning_effort={effort_str}"])
        if has_custom_prompt:
            cmd.append("-")
        return cmd

    @staticmethod
    def build_auth_boundary(warnings: JsonValue = None) -> JsonObject:
        """Build the Codex CLI authentication boundary metadata dictionary.

        Args:
            warnings: Optional list of isolated external MCP OAuth warnings.

        Returns:
            Authentication boundary dictionary for Codex CLI responses.
        """
        return CodexCliAuthBoundary(
            isolated_external_mcp_oauth_warnings=[] if warnings is None else warnings,
        ).to_json_object()

    @staticmethod
    def build_deduplicated_response(
        record: CodexJobRecord,
        provider: str,
        engine: str,
        resolved_cwd: str,
        resolved_model: str,
    ) -> JsonObject:
        """Build the response payload for an idempotent deduplicated `codex exec` request.

        Args:
            record: Existing persisted `CodexJobRecord` matching the request.
            provider: Resolved provider identifier.
            engine: Resolved engine identifier.
            resolved_cwd: Resolved workspace directory path string.
            resolved_model: Resolved model identifier.

        Returns:
            Deduplicated execution response dictionary.
        """
        return {
            "ok": record.status == "completed",
            "deduplicated_existing_request": True,
            "request_id": record.job_id,
            "job_id": record.job_id,
            "status": record.status,
            "outcome_state": record.outcome_state,
            "attempt_count": record.attempt_count,
            "provider": provider,
            "engine": engine,
            "workspace_dir": resolved_cwd,
            "model": resolved_model,
            "result": record.result,
            "error": record.error,
            "receipt": record.receipt,
        }
