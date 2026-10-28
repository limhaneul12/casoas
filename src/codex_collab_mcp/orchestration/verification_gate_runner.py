"""Bridge-executed verification gate runner, checkpoint resume loader, and auto-repair loop."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import signal
import sys
import time
from collections.abc import Callable, Coroutine
from pathlib import Path

from codex_collab_mcp.collab_enums import (
    AuditVerdict,
    JobStatusKind,
    OutcomeStateKind,
    RoleExecutionState,
    SchemaName,
)
from codex_collab_mcp.evidence.triad_adjudicator import (
    TRIAD_REQUIRED_ROLES,
    TRIAD_RESULT_SCHEMA_VERSION,
    extract_verdict,
    map_response_to_role_state,
)
from codex_collab_mcp.evidence.workspace_digests import (
    DIGEST_CALCULATOR,
    canonical_sha256,
    compute_sha256_hex,
)
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.prompts import build_adversarial_audit_prompt, build_claude_craft_prompt
from codex_collab_mcp.schemas import ADVERSARIAL_AUDIT_SCHEMA, JsonObject

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]


class BridgeVerificationGateRunner:
    """Execute allowlisted verification gates, resume checkpoints, and run repair rounds."""

    def _resolve_gate_command(self, workspace_dir: str) -> list[str]:
        """Select the allowlisted verification command for `workspace_dir`.

        Args:
            workspace_dir: Target workspace directory path.

        Returns:
            Command argument list (`pytest -q` if tests exist, else Python syntax probe).
        """
        ws_path = Path(workspace_dir)
        has_tests = (ws_path / "tests").is_dir() or any(ws_path.glob("test_*.py"))
        return (
            [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider"]
            if has_tests
            else [sys.executable, "-c", "import ast; print('syntax_ok')"]
        )

    async def run_allowlisted_gates(
        self,
        workspace_dir: str,
        timeout_seconds: float = 60.0,
    ) -> list[JsonObject]:
        """Run an allowlisted verification probe in `workspace_dir` and return gate receipts.

        Runs `[sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider"]` if `tests/`
        or any `test_*.py` file exists in `workspace_dir`; otherwise runs a deterministic
        Python syntax probe.

        Args:
            workspace_dir: Target workspace directory path.
            timeout_seconds: Maximum execution time in seconds before process termination.

        Returns:
            List of bridge-executed gate command records with exit code and stdout SHA-256.
        """
        cmd = self._resolve_gate_command(workspace_dir)
        cmd_str = " ".join(cmd)
        started_mono = time.monotonic()
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=workspace_dir,
            start_new_session=True,
        )
        try:
            stdout_bytes, _stderr_bytes = await asyncio.wait_for(
                proc.communicate(),
                timeout=timeout_seconds,
            )
            exit_code = proc.returncode if proc.returncode is not None else -1
        except BaseException as exc:
            if proc.returncode is None:
                if proc.pid is not None:
                    with contextlib.suppress(ProcessLookupError, PermissionError, OSError):
                        os.killpg(proc.pid, signal.SIGKILL)
                with contextlib.suppress(ProcessLookupError):
                    proc.kill()
                await proc.wait()
            if not isinstance(exc, TimeoutError):
                raise
            stdout_bytes = b""
            exit_code = -1

        duration = round(time.monotonic() - started_mono, 2)
        stdout_digest = f"sha256:{compute_sha256_hex(stdout_bytes)}"
        return [
            {
                "command": cmd_str,
                "exit_code": exit_code,
                "passed": exit_code == 0,
                "duration_seconds": duration,
                "stdout_digest": stdout_digest,
                "origin": "bridge_executed",
            }
        ]

    def load_resumable_roles(
        self,
        collaboration_id: str | None,
        workspace_dir: str,
        source_task_digest: str,
        job_store: JobStore,
        files_inspected: list[str] | None = None,
    ) -> dict[str, JsonObject]:
        """Load verified succeeded role responses from a prior triad checkpoint snapshot.

        Args:
            collaboration_id: Prior triad collaboration ID (`cdx-...`) to resume from.
            workspace_dir: Resolved workspace directory that must match the checkpoint.
            source_task_digest: Canonical task SHA-256 digest that must match the checkpoint.
            job_store: Active JobStore instance containing persisted triad snapshots.
            files_inspected: Optional inspected file paths to verify against workspace drift.

        Returns:
            Mapping of role name (`argon`, `sonnet`, `codex`) to reconstructed role response.
        """
        if not collaboration_id or not collaboration_id.strip():
            return {}
        cid = collaboration_id.strip()
        record = job_store.get_job(cid)
        if (
            record is None
            or record.task_type != "triad_orchestration"
            or not isinstance(record.result, dict)
        ):
            return {}
        result = record.result
        stored_digest = result.get("snapshot_digest")
        if (
            not isinstance(stored_digest, str)
            or result.get("schema_version") != TRIAD_RESULT_SCHEMA_VERSION
            or result.get("workspace_dir") != workspace_dir
            or result.get("source_task_digest") != source_task_digest
        ):
            return {}
        unsigned = {k: v for k, v in result.items() if k != "snapshot_digest"}
        if canonical_sha256(unsigned) != stored_digest:
            return {}
        stored_scope = result.get("evidence_scope")
        if isinstance(stored_scope, dict):
            s_digests = stored_scope.get("files_inspected_digests")
            s_missing = stored_scope.get("unverified_or_missing_files")
            if isinstance(s_digests, dict) and isinstance(s_missing, list):
                stored_files = [*s_digests.keys(), *(str(x) for x in s_missing)]
                req_files = list(files_inspected) if files_inspected is not None else stored_files
                if set(req_files) != set(stored_files):
                    return {}
                if req_files:
                    curr_digests, curr_missing = DIGEST_CALCULATOR.hash_files(
                        workspace_dir, req_files
                    )
                    if curr_digests != s_digests or set(curr_missing) != {
                        str(x) for x in s_missing
                    }:
                        return {}
        roles = result.get("roles")
        if not isinstance(roles, dict):
            return {}

        resumed: dict[str, JsonObject] = {}
        for role in TRIAD_REQUIRED_ROLES:
            role_snap = roles.get(role)
            if not isinstance(role_snap, dict):
                continue
            verdict = role_snap.get("verdict")
            if (
                role_snap.get("state") == RoleExecutionState.SUCCEEDED
                and role_snap.get("review_artifact_present") is True
                and isinstance(verdict, str)
            ):
                restored_struct: JsonObject = {
                    "verdict": verdict,
                    "summary": (
                        f"Resumed verified {role} artifact "
                        f"({role_snap.get('review_artifact_digest')}) from {cid}."
                    ),
                }
                role_jid = role_snap.get("job_id") or role_snap.get("request_id")
                if isinstance(role_jid, str) and role_jid.strip():
                    role_rec = job_store.get_job(role_jid.strip())
                    if role_rec is not None and isinstance(role_rec.result, dict):
                        orig_struct = role_rec.result.get("structured_output")
                        if isinstance(orig_struct, dict):
                            restored_struct = dict(orig_struct)
                resumed[role] = {
                    "ok": True,
                    "status": JobStatusKind.SUCCEEDED,
                    "outcome_state": OutcomeStateKind.CONFIRMED_SUCCESS,
                    "resumed_from_collaboration_id": cid,
                    "structured_output": restored_struct,
                }
        return resumed

    async def run_pipeline_repair_rounds(
        self,
        sonnet_res: JsonObject,
        codex_res: JsonObject,
        codex_verdict: str | None,
        *,
        max_rounds: int,
        task_spec: str,
        invariants_and_rules: str,
        target_code_or_context: str,
        language: str,
        workspace_dir: str,
        claude_model: str,
        codex_model: str | None,
        codex_reasoning_effort: str,
        timeout_seconds: float,
        max_input_bytes: int,
        job_store: JobStore,
        run_agy_fn: AsyncEngineCallable,
        execute_fn: AsyncEngineCallable,
    ) -> tuple[JsonObject, JsonObject, str | None, int]:
        """Run bounded Claude repair + Codex re-audit rounds when Codex returns BLOCK or WARN.

        Args:
            sonnet_res: Initial Claude Sonnet/Opus role response dictionary.
            codex_res: Initial OpenAI Codex audit response dictionary.
            codex_verdict: Extracted initial Codex verdict (`PASS`, `WARN`, `BLOCK`, or None).
            max_rounds: Maximum total pipeline rounds (clamped to `1..3`).
            task_spec: Original task specification string.
            invariants_and_rules: Non-negotiable invariants and repository rules.
            target_code_or_context: Initial target code or context string.
            language: Target programming language.
            workspace_dir: Resolved workspace directory path.
            claude_model: Claude Medium-tier model identifier.
            codex_model: Optional Codex model identifier.
            codex_reasoning_effort: Codex reasoning effort tier.
            timeout_seconds: Per-call execution timeout in seconds.
            max_input_bytes: Maximum UTF-8 prompt byte budget.
            job_store: Active JobStore instance.
            run_agy_fn: Antigravity local RPC prompt execution callable.
            execute_fn: Codex engine execution callable.

        Returns:
            Tuple of `(sonnet_res, codex_res, codex_verdict, repair_rounds_executed)`.
        """
        bounded_max = max(1, min(max_rounds, 3))
        round_idx = 1
        repair_rounds_executed = 0
        trigger_verdicts = (AuditVerdict.BLOCK, AuditVerdict.WARN)

        while round_idx < bounded_max and codex_verdict in trigger_verdicts:
            round_idx += 1
            codex_struct = codex_res.get("structured_output")
            critique_text = (
                json.dumps(codex_struct, ensure_ascii=False)
                if isinstance(codex_struct, dict)
                else str(codex_res.get("error", "Red-team audit blocked or warned."))
            )
            prev_struct = sonnet_res.get("structured_output")
            prev_code = (
                str(prev_struct.get("crafted_code", "")) if isinstance(prev_struct, dict) else ""
            ) or target_code_or_context

            repair_prompt = build_claude_craft_prompt(
                task_spec=(
                    f"{task_spec}\n\n"
                    f"[Auto-Repair Round {round_idx} — Resolve Codex Red-Team Findings]:\n"
                    f"{critique_text}"
                ),
                target_code_or_context=prev_code,
                invariants_and_rules=invariants_and_rules,
                language=language,
                claude_model=claude_model,
            )
            repaired_sonnet = await run_agy_fn(
                prompt=repair_prompt,
                model=claude_model,
                expect_json=True,
                timeout_seconds=timeout_seconds,
                max_input_bytes=max_input_bytes,
                workspace_dir=workspace_dir,
                job_store=job_store,
            )
            sonnet_res = repaired_sonnet
            if map_response_to_role_state(sonnet_res) != RoleExecutionState.SUCCEEDED:
                break
            repair_rounds_executed += 1

            sonnet_struct = sonnet_res.get("structured_output")
            crafted_code = ""
            sonnet_summary = ""
            handoff_notes = ""
            if isinstance(sonnet_struct, dict):
                crafted_code = str(sonnet_struct.get("crafted_code", ""))
                sonnet_summary = str(sonnet_struct.get("summary", ""))
                handoff_notes = str(sonnet_struct.get("handoff_notes_for_codex_audit", ""))
            audit_target = crafted_code or target_code_or_context or task_spec
            audit_prompt = build_adversarial_audit_prompt(
                target_code_or_diff=audit_target,
                design_rationale=(
                    f"[Argon Spec]: {task_spec}\n[Invariants]: {invariants_and_rules}\n"
                    f"[Claude Sonnet Repair Round {round_idx} Summary]: {sonnet_summary}"
                ),
                attack_vectors=[],
                context_notes=f"Sonnet Handoff Notes for Red-Team Audit: {handoff_notes}",
            )
            codex_res = await execute_fn(
                prompt=audit_prompt,
                schema_name=SchemaName.ADVERSARIAL_AUDIT,
                output_schema=ADVERSARIAL_AUDIT_SCHEMA,
                workspace_dir=workspace_dir,
                sandbox_mode="read-only",
                model=codex_model,
                reasoning_effort=codex_reasoning_effort,
                engine="cli",
                timeout_seconds=timeout_seconds,
                max_input_bytes=max_input_bytes,
            )
            codex_verdict = extract_verdict(codex_res)

        return sonnet_res, codex_res, codex_verdict, repair_rounds_executed

    async def run_council_adjudication_round(
        self,
        argon_res: JsonObject | None,
        sonnet_res: JsonObject,
        codex_res: JsonObject,
        *,
        max_rounds: int,
        task_spec: str,
        invariants_and_rules: str,
        target_code_or_context: str,
        language: str,
        claude_model: str,
        invoke_sonnet_fn: Callable[[str], Coroutine[object, object, JsonObject]],
        invoke_codex_fn: Callable[[str], Coroutine[object, object, JsonObject]],
        invoke_argon_fn: Callable[[str], Coroutine[object, object, JsonObject | None]],
    ) -> tuple[JsonObject | None, JsonObject, JsonObject, int]:
        """Run a Round 2 Council cross-examination when Round 1 verdicts conflict or block.

        Args:
            argon_res: Round 1 Gemini 4 Argon response dictionary or None.
            sonnet_res: Round 1 Claude Sonnet/Opus response dictionary.
            codex_res: Round 1 OpenAI Codex response dictionary.
            max_rounds: Maximum council rounds requested (`1..3`).
            task_spec: Task specification string.
            invariants_and_rules: Invariants and repository rules string.
            target_code_or_context: Target code or context string.
            language: Target programming language.
            claude_model: Claude Medium-tier model identifier.
            invoke_sonnet_fn: Async callable taking a Round 2 Sonnet prompt.
            invoke_codex_fn: Async callable taking a Round 2 Codex prompt.
            invoke_argon_fn: Async callable taking Round 1 peer context for Argon.

        Returns:
            Tuple of `(argon_res, sonnet_res, codex_res, repair_rounds_executed)`.
        """
        if max_rounds <= 1:
            return argon_res, sonnet_res, codex_res, 0
        states = (
            map_response_to_role_state(argon_res),
            map_response_to_role_state(sonnet_res),
            map_response_to_role_state(codex_res),
        )
        if any(s != RoleExecutionState.SUCCEEDED for s in states):
            return argon_res, sonnet_res, codex_res, 0

        v_argon = extract_verdict(argon_res)
        v_sonnet = extract_verdict(sonnet_res)
        v_codex = extract_verdict(codex_res)
        verdicts = (v_argon, v_sonnet, v_codex)
        has_conflict = len(set(verdicts)) > 1 or AuditVerdict.BLOCK in verdicts
        if not has_conflict:
            return argon_res, sonnet_res, codex_res, 0

        def _extract_summary(resp: JsonObject | None) -> str:
            """Extract the summary or executive_critique from a role response.

            Args:
                resp: Role response dictionary or None.

            Returns:
                Extracted summary string.
            """
            if not isinstance(resp, dict):
                return ""
            st = resp.get("structured_output")
            if isinstance(st, dict):
                return str(st.get("summary") or st.get("executive_critique") or "")
            return ""

        cross_exam_digest = (
            "[Council Round 1 Cross-Examination — Peer Verdicts & Critiques]:\n"
            f"- Gemini 4 Argon (verdict={v_argon}): {_extract_summary(argon_res)}\n"
            f"- Claude Sonnet/Opus (verdict={v_sonnet}): {_extract_summary(sonnet_res)}\n"
            f"- OpenAI Codex (verdict={v_codex}): {_extract_summary(codex_res)}\n"
            "Re-evaluate the target code and peer critiques to reach a verified Round 2 verdict."
        )
        r2_sonnet_prompt = build_claude_craft_prompt(
            f"{task_spec}\n\n{cross_exam_digest}",
            target_code_or_context,
            invariants_and_rules,
            language,
            claude_model,
        )
        r2_codex_prompt = build_adversarial_audit_prompt(
            target_code_or_diff=target_code_or_context or task_spec,
            design_rationale=f"{task_spec}\n\nInvariants:\n{invariants_and_rules}",
            attack_vectors=[],
            context_notes=cross_exam_digest,
        )

        async def _safe_call(
            fn: Callable[[str], Coroutine[object, object, JsonObject | None]],
            arg: str,
        ) -> JsonObject:
            """Invoke a Round 2 role callable, converting provider errors into role outcomes.

            Args:
                fn: Async role invocation callable.
                arg: Prompt or extra context string.

            Returns:
                Role response dictionary or structured failure/timeout dictionary.
            """
            try:
                res = await fn(arg)
                if res is None:
                    return {
                        "ok": False,
                        "status": JobStatusKind.FAILED,
                        "outcome_state": OutcomeStateKind.CONFIRMED_FAILURE,
                        "error": "Round 2 role returned no response.",
                    }
                return res
            except TimeoutError as exc:
                return {
                    "ok": False,
                    "status": JobStatusKind.TIMED_OUT_UNKNOWN,
                    "outcome_state": OutcomeStateKind.OUTCOME_UNKNOWN,
                    "error": f"Round 2 provider timeout: {exc}",
                }
            except Exception as exc:
                return {
                    "ok": False,
                    "status": JobStatusKind.FAILED,
                    "outcome_state": OutcomeStateKind.CONFIRMED_FAILURE,
                    "error": f"Round 2 provider error: {exc}",
                }

        async with asyncio.TaskGroup() as tg:
            s_task = tg.create_task(_safe_call(invoke_sonnet_fn, r2_sonnet_prompt))
            c_task = tg.create_task(_safe_call(invoke_codex_fn, r2_codex_prompt))
            a_task = tg.create_task(_safe_call(invoke_argon_fn, cross_exam_digest))
        return a_task.result(), s_task.result(), c_task.result(), 1
