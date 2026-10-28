"""Workspace path resolution, CodeGraph index snapshot, and 3.14t parallel SHA-256 file hashing."""

from __future__ import annotations

import hashlib
import json
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from codex_collab_mcp.evidence.evidence_dicts import (
    CodegraphIndexSnapshotDict,
    TaskHashPayloadDict,
)
from codex_collab_mcp.routing.receipt_budget import compute_sha256_hex


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

    def canonical_hash(self, value: object) -> str:
        """Hash a JSON-serializable value using a stable compact representation.

        Args:
            value: Arbitrary JSON-serializable value or TypedDict to hash.

        Returns:
            Prefixed `sha256:...` digest string.
        """
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        return f"sha256:{compute_sha256_hex(encoded)}"

    def compute_task_hash(
        self,
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
        payload = TaskHashPayloadDict(
            task_spec=task_spec,
            target_code_or_context=target_code_or_context,
            invariants_and_rules=invariants_and_rules,
            language=language,
            mode=mode,
        )
        return self.canonical_hash(payload)

    def inspect_codegraph(self, workspace_dir: str) -> CodegraphIndexSnapshotDict:
        """Inspect `.codegraph` index state in `workspace_dir` without fabricating freshness.

        Args:
            workspace_dir: Workspace directory path to inspect.

        Returns:
            Typed CodegraphIndexSnapshotDict describing CodeGraph index presence and snapshot ID.
        """
        ws_path = Path(self.resolve_dir(workspace_dir or ".") or ".")
        db_path = ws_path / ".codegraph" / "codegraph.db"
        if not db_path.is_file():
            note = "No .codegraph/codegraph.db found in workspace; static graph unindexed."
            return CodegraphIndexSnapshot(
                status="unindexed",
                snapshot_id=None,
                workspace_dir=str(ws_path),
                db_mtime_utc=None,
                db_size_bytes=None,
                note=note,
            ).to_dict()
        stat = db_path.stat()
        mtime_iso = datetime.fromtimestamp(stat.st_mtime, tz=UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        seed = f"{ws_path}::{stat.st_size}::{int(stat.st_mtime)}"
        snap_id = f"cg-{compute_sha256_hex(seed)[:12]}"
        return CodegraphIndexSnapshot(
            status="indexed",
            snapshot_id=snap_id,
            workspace_dir=str(ws_path),
            db_mtime_utc=mtime_iso,
            db_size_bytes=stat.st_size,
            note=None,
        ).to_dict()

    def hash_files(
        self,
        workspace_dir: str,
        files_inspected: list[str] | None = None,
    ) -> tuple[dict[str, str], list[str]]:
        """Compute SHA-256 digests for inspected workspace files in parallel on 3.14t.

        Args:
            workspace_dir: Base workspace directory path.
            files_inspected: Optional list of relative or absolute file paths to hash.

        Returns:
            Tuple of `(file_hashes_dict, missing_files_list)`.
        """
        items = list(files_inspected or [])
        if not items:
            return {}, []
        ws_path = Path(self.resolve_dir(workspace_dir or ".") or ".")

        def _hash_one(rel_or_abs: str) -> tuple[str, str | None]:
            """Read and hash a single file path.

            Args:
                rel_or_abs: Relative or absolute path string to inspect.

            Returns:
                Tuple of `(rel_or_abs, sha256_digest_or_none)`.
            """
            candidate = Path(rel_or_abs)
            full_path = candidate if candidate.is_absolute() else (ws_path / candidate)
            if not full_path.is_file():
                return rel_or_abs, None
            try:
                digest = hashlib.sha256(full_path.read_bytes()).hexdigest()
                return rel_or_abs, f"sha256:{digest}"
            except OSError:
                return rel_or_abs, None

        file_hashes: dict[str, str] = {}
        missing_files: list[str] = []
        if len(items) == 1:
            results = [_hash_one(items[0])]
        else:
            with ThreadPoolExecutor(max_workers=min(8, len(items))) as pool:
                results = list(pool.map(_hash_one, items))

        for path_str, digest_val in results:
            if digest_val is not None:
                file_hashes[path_str] = digest_val
            else:
                missing_files.append(path_str)
        return file_hashes, missing_files


DIGEST_CALCULATOR = WorkspaceDigestCalculator()
canonical_sha256 = DIGEST_CALCULATOR.canonical_hash
resolve_workspace_dir = DIGEST_CALCULATOR.resolve_dir
compute_task_digest = DIGEST_CALCULATOR.compute_task_hash


def matches_workspace(record_ws: str, ws_filter: str) -> bool:
    """Check whether a stored workspace directory matches the resolved filter path.

    Args:
        record_ws: Workspace directory string stored on a job or handoff record.
        ws_filter: Resolved absolute workspace directory filter string.

    Returns:
        True if the workspace paths match, otherwise False.
    """
    if record_ws == ws_filter:
        return True
    if not record_ws.strip() or Path(record_ws).is_absolute():
        return False
    return resolve_workspace_dir(record_ws) == ws_filter
