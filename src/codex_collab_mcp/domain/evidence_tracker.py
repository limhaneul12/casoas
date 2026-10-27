"""Evidence scope, CodeGraph snapshot provenance, file digests, and triad role evidence."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from codex_collab_mcp.collab_enums import EvidenceDimensionState, RoleExecutionState, TriadRole
from codex_collab_mcp.domain.evidence_dicts import (
    CodegraphIndexSnapshotDict,
    EvidenceDimensionsDict,
    EvidenceScopeDict,
)
from codex_collab_mcp.domain.routing_catalog import compute_sha256_hex
from codex_collab_mcp.domain.triad_adjudicator import (
    TRIAD_REQUIRED_ROLES,
    TRIAD_VERDICTS,
    adjudicate_collaboration_and_consensus,
    has_review_narrative,
    is_successful_role_response,
    is_valid_review_response,
    map_response_to_role_state,
    structured_review_artifact,
    verdict_from_artifact,
)
from codex_collab_mcp.schemas import JsonObject, JsonValue


@dataclass(frozen=True, slots=True)
class CodegraphIndexSnapshot:
    """Snapshot metadata for the workspace `.codegraph/codegraph.db` index."""

    status: str
    snapshot_id: str | None
    workspace_dir: str
    db_mtime_utc: str | None
    db_size_bytes: int | None = None
    note: str | None = None

    def to_dict(self) -> CodegraphIndexSnapshotDict:
        """Serialize the CodeGraph snapshot metadata to a TypedDict.

        Returns:
            Serialized CodegraphIndexSnapshotDict mapping.
        """
        return CodegraphIndexSnapshotDict(
            status=self.status,
            snapshot_id=self.snapshot_id,
            workspace_dir=self.workspace_dir,
            db_mtime_utc=self.db_mtime_utc,
            db_size_bytes=self.db_size_bytes,
            note=self.note,
        )


class WorkspaceDigestCalculator:
    """Cohesive calculator for workspace paths, CodeGraph index state, and file SHA-256 digests."""

    def resolve_dir(self, workspace_dir: str) -> str | None:
        """Normalize a workspace path for exact source-run attribution.

        Args:
            workspace_dir: Raw workspace directory path string.

        Returns:
            Resolved absolute directory path string, or None if invalid or empty.
        """
        if not workspace_dir.strip():
            return None
        try:
            return str(Path(workspace_dir).expanduser().resolve())
        except OSError, RuntimeError, ValueError:
            return None

    def canonical_hash(self, value: JsonValue) -> str:
        """Hash a JSON value using a stable compact representation.

        Args:
            value: Arbitrary JSON-serializable value to hash.

        Returns:
            Prefixed `sha256:...` digest string.
        """
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        return f"sha256:{hashlib.sha256(encoded.encode('utf-8')).hexdigest()}"

    def compute_task_hash(
        self,
        *,
        task_spec: str,
        target_code_or_context: str,
        invariants_and_rules: str,
        language: str,
        mode: str,
    ) -> str:
        """Identify the exact triad input without persisting task or source text.

        Args:
            task_spec: Task specification text.
            target_code_or_context: Supplied target code or context text.
            invariants_and_rules: Invariants and engineering rules text.
            language: Target language name.
            mode: Orchestration mode (`pipeline` or `council`).

        Returns:
            Canonical `sha256:...` digest of the task input tuple.
        """
        payload: JsonObject = {
            "task_spec": task_spec,
            "target_code_or_context": target_code_or_context,
            "invariants_and_rules": invariants_and_rules,
            "language": language,
            "mode": mode,
        }
        return self.canonical_hash(payload)

    def inspect_codegraph(self, workspace_dir: str) -> JsonObject:
        """Inspect `.codegraph` index state in `workspace_dir` without fabricating freshness.

        Args:
            workspace_dir: Workspace directory path to inspect.

        Returns:
            JSON object describing CodeGraph index presence, mtime, and snapshot ID.
        """
        ws_path = Path(self.resolve_dir(workspace_dir or ".") or ".")
        db_path = ws_path / ".codegraph" / "codegraph.db"
        if not db_path.is_file():
            note = "No .codegraph/codegraph.db found in workspace; static graph unindexed."
            m = CodegraphIndexSnapshot("unindexed", None, str(ws_path), None, None, note).to_dict()
            return {
                "status": m["status"],
                "snapshot_id": m["snapshot_id"],
                "workspace_dir": m["workspace_dir"],
                "db_mtime_utc": m["db_mtime_utc"],
                "note": m["note"],
            }
        stat = db_path.stat()
        mtime_iso = datetime.fromtimestamp(stat.st_mtime, tz=UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        seed = f"{ws_path}::{stat.st_size}::{int(stat.st_mtime)}"
        snap_id = f"cg-{compute_sha256_hex(seed)[:12]}"
        ready = CodegraphIndexSnapshot(
            "indexed", snap_id, str(ws_path), mtime_iso, stat.st_size, None
        ).to_dict()
        return {
            "status": ready["status"],
            "snapshot_id": ready["snapshot_id"],
            "workspace_dir": ready["workspace_dir"],
            "db_mtime_utc": ready["db_mtime_utc"],
            "db_size_bytes": ready["db_size_bytes"],
        }

    def hash_files(
        self,
        workspace_dir: str,
        files_inspected: list[str] | None = None,
    ) -> tuple[JsonObject, list[JsonValue]]:
        """Compute SHA-256 digests for inspected workspace files and report missing paths.

        Args:
            workspace_dir: Base workspace directory path.
            files_inspected: Optional list of relative or absolute file paths to hash.

        Returns:
            Tuple of `(file_hashes_dict, missing_files_list)`.
        """
        ws_path = Path(self.resolve_dir(workspace_dir or ".") or ".")
        file_hashes: JsonObject = {}
        missing_files: list[JsonValue] = []
        for rel_or_abs in files_inspected or []:
            candidate = Path(rel_or_abs)
            full_path = candidate if candidate.is_absolute() else (ws_path / candidate)
            if full_path.is_file():
                try:
                    digest = hashlib.sha256(full_path.read_bytes()).hexdigest()
                    file_hashes[rel_or_abs] = f"sha256:{digest}"
                except OSError:
                    missing_files.append(rel_or_abs)
            else:
                missing_files.append(rel_or_abs)
        return file_hashes, missing_files


_DIGEST_CALCULATOR = WorkspaceDigestCalculator()
_NOT_RUN = str(EvidenceDimensionState.NOT_RUN)


def _build_dimensions(
    static_code_review: str = _NOT_RUN,
    test_generation: str = _NOT_RUN,
    test_execution: str = _NOT_RUN,
    integration_verification: str = _NOT_RUN,
    runtime_confirmation: str = _NOT_RUN,
) -> JsonObject:
    """Build a serialized evidence dimensions JSON object via EvidenceDimensionsDict.

    Args:
        static_code_review: Verification status for static code review.
        test_generation: Verification status for test generation.
        test_execution: Verification status for test execution.
        integration_verification: Verification status for integration verification.
        runtime_confirmation: Verification status for runtime confirmation.

    Returns:
        JSON object mapping each evidence dimension to its verification state.
    """
    dims = EvidenceDimensionsDict(
        static_code_review=static_code_review,
        test_generation=test_generation,
        test_execution=test_execution,
        integration_verification=integration_verification,
        runtime_confirmation=runtime_confirmation,
    )
    return {
        "static_code_review": dims["static_code_review"],
        "test_generation": dims["test_generation"],
        "test_execution": dims["test_execution"],
        "integration_verification": dims["integration_verification"],
        "runtime_confirmation": dims["runtime_confirmation"],
    }


class EvidenceScopeBuilder:
    """Cohesive builder for verification evidence scopes and triad role evidence snapshots."""

    def has_tests(self, result: JsonObject) -> bool:
        """Return True if the response contains non-empty generated unit or failing tests.

        Args:
            result: Collaboration response dictionary.

        Returns:
            True if non-empty `failing_test_cases` or `unit_tests` are present.
        """
        for key in ("structured_output", "data"):
            if isinstance(obj := result.get(key), dict):
                if isinstance(f := obj.get("failing_test_cases"), list) and len(f) > 0:
                    return True
                if isinstance(u := obj.get("unit_tests"), str) and u.strip():
                    return True
        return False

    def build_scope(
        self,
        *,
        workspace_dir: str,
        provided_context_or_code: str,
        files_inspected: list[str] | None = None,
        symbols_inspected: list[str] | None = None,
        static_code_review: str = _NOT_RUN,
        test_generation: str = _NOT_RUN,
        test_execution: str = _NOT_RUN,
        integration_verification: str = _NOT_RUN,
        runtime_confirmation: str = _NOT_RUN,
        executed_test_commands: list[JsonObject] | None = None,
    ) -> JsonObject:
        """Anchor review and synthesis claims to explicit digests and verification dimensions.

        Args:
            workspace_dir: Workspace directory path.
            provided_context_or_code: Inline code or context string supplied to the role.
            files_inspected: Optional list of file paths inspected.
            symbols_inspected: Optional list of symbol names inspected.
            static_code_review: Verification status for static code review.
            test_generation: Verification status for test generation.
            test_execution: Verification status for test execution.
            integration_verification: Verification status for integration verification.
            runtime_confirmation: Verification status for runtime confirmation.
            executed_test_commands: Optional list of executed test command records.

        Returns:
            JSON object representing the verified evidence scope and digests.
        """
        ws_dir = _DIGEST_CALCULATOR.resolve_dir(workspace_dir or ".") or "."
        file_hashes, missing_files = _DIGEST_CALCULATOR.hash_files(ws_dir, files_inspected)
        context_bytes = provided_context_or_code.encode("utf-8")
        inline_sha = hashlib.sha256(context_bytes).hexdigest()
        syms = sorted(symbols_inspected or [])
        scope_obj = {
            "inline_context_sha256": inline_sha,
            "file_hashes": file_hashes,
            "symbols": syms,
        }
        scope_payload = json.dumps(scope_obj, sort_keys=True)
        executed_cmds: list[JsonValue] = [dict(c) for c in (executed_test_commands or [])]
        eff_exec = _NOT_RUN if not executed_cmds else test_execution
        integ, runtime = integration_verification, runtime_confirmation
        dims_json = _build_dimensions(static_code_review, test_generation, eff_exec, integ, runtime)
        symbols_json: list[JsonValue] = list(symbols_inspected or [])
        scope = EvidenceScopeDict(
            codegraph=_DIGEST_CALCULATOR.inspect_codegraph(ws_dir),
            scope_digest=f"sha256:{compute_sha256_hex(scope_payload)}",
            inline_context_bytes=len(context_bytes),
            inline_context_digest=f"sha256:{inline_sha}",
            files_inspected_digests=file_hashes,
            unverified_or_missing_files=missing_files,
            symbols_inspected=symbols_json,
            evidence_dimensions=dims_json,
            executed_test_commands=executed_cmds,
        )
        return {
            "codegraph": scope["codegraph"],
            "scope_digest": scope["scope_digest"],
            "inline_context_bytes": scope["inline_context_bytes"],
            "inline_context_digest": scope["inline_context_digest"],
            "files_inspected_digests": scope["files_inspected_digests"],
            "unverified_or_missing_files": scope["unverified_or_missing_files"],
            "symbols_inspected": scope["symbols_inspected"],
            "evidence_dimensions": scope["evidence_dimensions"],
            "executed_test_commands": scope["executed_test_commands"],
        }

    def attach_scope(
        self,
        result: JsonObject,
        *,
        workspace_dir: str,
        provided_context_or_code: str,
        files_inspected: list[str] | None = None,
        symbols_inspected: list[str] | None = None,
        executed_test_commands: list[JsonObject] | None = None,
        bridge_executed_gates: list[JsonObject] | None = None,
    ) -> JsonObject:
        """Attach verified evidence scope to a collaboration response without inflating claims.

        Args:
            result: Mutable collaboration response dictionary.
            workspace_dir: Workspace directory path.
            provided_context_or_code: Inline code or context string supplied to the role.
            files_inspected: Optional list of file paths inspected.
            symbols_inspected: Optional list of symbol names inspected.
            executed_test_commands: Optional list of caller-reported test command records.
            bridge_executed_gates: Optional list of bridge-executed quality gate records.

        Returns:
            Updated response dictionary with `evidence_scope` attached.
        """
        succeeded = is_successful_role_response(result)
        static_review = "verified" if is_valid_review_response(result) else _NOT_RUN
        test_gen = "verified" if (succeeded and self.has_tests(result)) else _NOT_RUN
        if bridge_executed_gates:
            all_passed = all(g.get("exit_code") == 0 for g in bridge_executed_gates)
            test_exec_dim = "verified" if all_passed else "executed_fail"
            eff_cmds = bridge_executed_gates
            origin: str | None = "bridge_executed"
        else:
            test_exec_dim = _NOT_RUN
            eff_cmds = executed_test_commands or []
            origin = "caller_reported" if eff_cmds else None
        evidence_scope = self.build_scope(
            workspace_dir=workspace_dir,
            provided_context_or_code=provided_context_or_code,
            files_inspected=files_inspected,
            symbols_inspected=symbols_inspected,
            static_code_review=static_review,
            test_generation=test_gen,
            test_execution=test_exec_dim,
            executed_test_commands=eff_cmds,
        )
        if origin is not None:
            evidence_scope["test_execution_evidence_origin"] = origin
        result["evidence_scope"] = evidence_scope
        return result

    def extract_references(self, res: JsonObject | None) -> JsonObject:
        """Keep only lookup references and receipt selectors from a role response.

        Args:
            res: Role execution response dictionary or None.

        Returns:
            Compact dictionary of request, job, receipt, and handoff references.
        """
        refs: JsonObject = {}
        if res is None:
            return refs
        for k in ("request_id", "job_id", "result_ref"):
            if isinstance(v := res.get(k), str) and v.strip():
                refs[k] = v.strip()
        if isinstance(rec := res.get("receipt"), dict):
            if (
                isinstance(am := rec.get("actual_model"), str)
                and (am_s := am.strip())
                and am_s != "codex-default"
            ):
                refs["model_id"] = am_s
            for k in ("request_id", "result_ref", "output_digest"):
                if isinstance(v := rec.get(k), str) and v.strip():
                    refs[f"receipt_{k}"] = v.strip()
        for k in ("model_id", "model"):
            if "model_id" not in refs and isinstance(m := res.get(k), str):
                if (m_s := m.strip()) and m_s != "codex-default":
                    refs["model_id"] = m_s
        if isinstance(ht := res.get("handoff_ticket"), dict):
            if isinstance(tid := ht.get("ticket_id"), str) and tid.strip():
                refs["handoff_ticket_id"] = tid.strip()
        return refs

    def prepare_triad_evidence(
        self,
        role_responses: dict[str, JsonObject | None],
        decision_authority: str | None = None,
    ) -> tuple[JsonObject, dict[str, JsonObject], dict[str, JsonObject]]:
        """Build same-call role states, review evidence, and compact artifact references.

        Args:
            role_responses: Mapping of role name to execution response dictionary or None.
            decision_authority: Optional override for the adjudication authority identifier.

        Returns:
            Tuple of `(adjudication, role_snapshots, role_evidence_dimensions)`.
        """
        role_states: dict[str, str] = {}
        role_verdicts: dict[str, str | None] = {}
        role_outputs: dict[str, JsonValue] = {}
        role_snapshots: dict[str, JsonObject] = {}
        role_evidence_dimensions: dict[str, JsonObject] = {}

        for role in TRIAD_REQUIRED_ROLES:
            response = role_responses.get(role)
            state = map_response_to_role_state(response)
            artifact = structured_review_artifact(response)
            verdict = verdict_from_artifact(artifact)
            artifact_has_narrative = has_review_narrative(artifact)
            artifact_digest = _DIGEST_CALCULATOR.canonical_hash(artifact) if artifact else None
            ok_succ = (
                is_successful_role_response(response) and state == RoleExecutionState.SUCCEEDED
            )
            review_valid = (
                ok_succ
                and artifact is not None
                and artifact_has_narrative
                and verdict in TRIAD_VERDICTS
                and isinstance(artifact_digest, str)
            )
            generated_tests = ok_succ and response is not None and self.has_tests(response)
            raw_status = response.get("status") if response is not None else None
            normalized_status = raw_status.strip().lower() if isinstance(raw_status, str) else None
            raw_ok = response.get("ok") if response is not None else None
            response_ok = raw_ok if isinstance(raw_ok, bool) else None
            rev_dim = "verified" if review_valid else _NOT_RUN
            test_dim = "verified" if generated_tests else _NOT_RUN
            evidence_dimensions = _build_dimensions(rev_dim, test_dim)
            role_snapshot: JsonObject = {
                "state": state,
                "response_present": response is not None,
                "response_status": normalized_status,
                "response_ok": response_ok,
                "verdict": verdict if review_valid else None,
                "review_artifact_present": artifact is not None,
                "review_artifact_has_narrative": artifact_has_narrative,
                "review_artifact_digest": artifact_digest,
                "generated_tests_present": generated_tests,
                "evidence_dimensions": evidence_dimensions,
            }
            role_snapshot.update(self.extract_references(response))
            role_states[role] = state
            role_verdicts[role] = verdict if review_valid else None
            role_outputs[role] = artifact
            role_snapshots[role] = role_snapshot
            role_evidence_dimensions[role] = evidence_dimensions

        default_auth = (
            "gemini-4-argon"
            if role_states.get(str(TriadRole.ARGON)) == RoleExecutionState.SUCCEEDED
            else "pending_argon_or_user_adjudication"
        )
        adjudication = adjudicate_collaboration_and_consensus(
            required_roles=list(TRIAD_REQUIRED_ROLES),
            role_states=role_states,
            role_verdicts=role_verdicts,
            role_outputs=role_outputs,
            decision_authority=decision_authority or default_auth,
        )
        return adjudication, role_snapshots, role_evidence_dimensions

    def compact_scope(self, evidence_scope: JsonObject) -> JsonObject:
        """Persist digest-oriented scope details without command text or model output.

        Args:
            evidence_scope: Full evidence scope dictionary to compact.

        Returns:
            Compacted evidence scope dictionary suitable for durable storage.
        """
        compact: JsonObject = {
            k: v
            for k in (
                "codegraph",
                "scope_digest",
                "inline_context_bytes",
                "inline_context_digest",
                "files_inspected_digests",
                "unverified_or_missing_files",
                "symbols_inspected",
                "evidence_dimensions",
                "test_execution_evidence_origin",
            )
            if (v := evidence_scope.get(k)) is not None
        }
        raw_commands = evidence_scope.get("executed_test_commands")
        if isinstance(raw_commands, list):
            command_evidence: list[JsonValue] = []
            for raw_cmd in raw_commands:
                digest = _DIGEST_CALCULATOR.canonical_hash(raw_cmd)
                if not isinstance(raw_cmd, dict):
                    command_evidence.append({"item_digest": digest})
                    continue
                passed = raw_cmd.get("passed")
                is_bool = isinstance(passed, bool)
                item: JsonObject = {"item_digest": digest, "passed_is_boolean": is_bool}
                if is_bool:
                    item["passed"] = passed
                exit_code = raw_cmd.get("exit_code")
                if isinstance(exit_code, int) and not isinstance(exit_code, bool):
                    item["exit_code"] = exit_code
                command_evidence.append(item)
            compact["executed_test_commands"] = command_evidence
        return compact


_SCOPE_BUILDER = EvidenceScopeBuilder()

canonical_sha256 = _DIGEST_CALCULATOR.canonical_hash
resolve_workspace_dir = _DIGEST_CALCULATOR.resolve_dir
compute_task_digest = _DIGEST_CALCULATOR.compute_task_hash
attach_evidence_scope = _SCOPE_BUILDER.attach_scope
prepare_triad_role_evidence = _SCOPE_BUILDER.prepare_triad_evidence
compact_evidence_scope = _SCOPE_BUILDER.compact_scope
