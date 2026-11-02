"""Read-only SQLite `.codegraph/codegraph.db` symbol context slicer for prompt injection."""

from __future__ import annotations

import contextlib
import json
import re
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from app.evidence.workspace_digests import WorkspaceDigestCalculator
from app.repositories.sql_loader import SQL_LOADER, build_readonly_sqlite_uri
from app.repositories.sql_query_enums import CodegraphSqlQueryId, SqlFeatureFile

_INLINE_SYMBOL_RE: Final[re.Pattern[str]] = re.compile(
    pattern=r"\b(?:def|class)\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
_DIFF_FILE_RE: Final[re.Pattern[str]] = re.compile(pattern=r"^\+\+\+\s+(?:b/)?([^\t\r\n]+)")
_DIFF_HUNK_RE: Final[re.Pattern[str]] = re.compile(
    pattern=r"^@@\s+-\d+(?:,\d+)?\s+\+(\d+)(?:,(\d+))?\s+@@"
)


def _extract_diff_hunks(inline_text: str) -> list[tuple[str, int, int]]:
    """Extract `(rel_file_path, start_line, end_line)` tuples from unified diff text.

    Args:
        inline_text: Raw inline code or unified diff text.

    Returns:
        List of `(rel_file_path, hunk_start_line, hunk_end_line)` tuples.
    """
    if "@@" not in inline_text or "+++" not in inline_text:
        return []
    hunks: list[tuple[str, int, int]] = []
    seen_hunks: set[tuple[str, int, int]] = set()
    current_file = ""
    for line in inline_text.splitlines():
        if (f_match := _DIFF_FILE_RE.match(line)) is not None:
            raw_path = f_match.group(1).strip().removeprefix("./")
            current_file = "" if raw_path == "/dev/null" else Path(raw_path).as_posix()
        elif current_file and (h_match := _DIFF_HUNK_RE.match(line)) is not None:
            start_ln = max(1, int(h_match.group(1)))
            count_ln = int(h_match.group(2)) if h_match.group(2) is not None else 1
            end_ln = max(start_ln, start_ln + max(1, count_ln) - 1)
            item = (current_file, start_ln, end_ln)
            if item not in seen_hunks:
                seen_hunks.add(item)
                hunks.append(item)
    return hunks


@dataclass(frozen=True, slots=True)
class CodegraphSymbolSlice:
    """Bounded source excerpt and 1-hop caller/callee neighborhood for an indexed symbol."""

    name: str
    qualified_name: str
    kind: str
    file_path: str
    start_line: int
    end_line: int
    callers: list[str]
    callees: list[str]
    source_excerpt: str


class CodegraphContextSlicer:
    """Read-only `.codegraph/codegraph.db` slicer extracting symbol definitions and 1-hop edges."""

    def _query_neighbors(
        self,
        conn: sqlite3.Connection,
        node_id: str,
    ) -> tuple[list[str], list[str]]:
        """Query 1-hop caller and callee symbol names for a given node ID.

        Args:
            conn: Open read-only SQLite connection to `.codegraph/codegraph.db`.
            node_id: Primary key identifier of the node in the `nodes` table.

        Returns:
            Tuple of `(callers, callees)` name lists, each bounded to 8 entries.
        """
        rows = SQL_LOADER.execute_safe(
            conn=conn,
            feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
            query_id=CodegraphSqlQueryId.SELECT_NEIGHBORS_BY_NODE_ID,
            params=(node_id, node_id),
        ).fetchall()
        callers = [str(r[1]) for r in rows if r and r[0] == "caller" and r[1] is not None]
        callees = [str(r[1]) for r in rows if r and r[0] == "callee" and r[1] is not None]
        return callers, callees

    def _read_file_lines(self, ws_path: Path, file_path: str) -> list[str]:
        """Read and split lines from a workspace-contained file once.

        Args:
            ws_path: Resolved workspace root directory path.
            file_path: Node file path relative to `ws_path` (or inside `ws_path`).

        Returns:
            List of lines from the file, or empty list if unreadable or outside workspace.
        """
        if not file_path.strip():
            return []
        try:
            candidate = (ws_path / file_path).resolve()
            if not candidate.is_relative_to(ws_path) or not candidate.is_file():
                return []
            return candidate.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError, RuntimeError, ValueError:
            return []

    def _slice_lines(
        self,
        lines: list[str],
        start_line: int,
        end_line: int,
        max_bytes: int,
    ) -> tuple[str, int]:
        """Extract `start_line..end_line` from pre-loaded file lines bounded by `max_bytes`.

        Args:
            lines: Pre-loaded lines of the source file.
            start_line: 1-based inclusive start line number.
            end_line: 1-based inclusive end line number.
            max_bytes: Maximum UTF-8 byte budget for the excerpt.

        Returns:
            Tuple of `(utf8_excerpt_str, used_utf8_bytes)`.
        """
        if max_bytes <= 0 or not lines:
            return "", 0
        lo = max(1, start_line) - 1
        hi = max(lo + 1, end_line)
        excerpt = "\n".join(lines[lo:hi])
        encoded = excerpt.encode("utf-8")
        if len(encoded) <= max_bytes:
            return excerpt, len(encoded)
        clipped = encoded[:max_bytes].decode("utf-8", errors="ignore")
        return clipped, len(clipped.encode("utf-8"))

    def slice_symbols(
        self,
        workspace_dir: str,
        symbols: list[str] | None,
        max_bytes: int = 48_000,
    ) -> list[CodegraphSymbolSlice]:
        """Extract symbol definitions, 1-hop callers/callees, and source excerpts from CodeGraph.

        Args:
            workspace_dir: Workspace directory containing `.codegraph/codegraph.db`.
            symbols: Optional list of symbol names or qualified names to slice.
            max_bytes: Maximum aggregate UTF-8 byte budget across all source excerpts.

        Returns:
            List of matched CodegraphSymbolSlice records.
        """
        if not symbols:
            return []
        resolved_ws = WorkspaceDigestCalculator().resolve_dir(workspace_dir=workspace_dir)
        if resolved_ws is None:
            return []
        ws_path = Path(resolved_ws)
        db_path = ws_path / ".codegraph" / "codegraph.db"
        if not db_path.is_file():
            return []

        seen_ids: set[str] = set()
        remaining_bytes = max(0, max_bytes)
        per_symbol_cap = max(8_000, max_bytes // max(1, len(symbols)))
        raw_nodes: list[tuple[str, str, str, str, int, int, list[str], list[str]]] = []
        try:
            ro_uri = build_readonly_sqlite_uri(db_path=db_path)
            with contextlib.closing(sqlite3.connect(ro_uri, uri=True)) as conn:
                SQL_LOADER.execute_script(
                    conn=conn,
                    feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
                    query_id=CodegraphSqlQueryId.PRAGMA_READONLY_DEFENSE,
                )
                for raw_sym in symbols:
                    sym = raw_sym.strip()
                    if not sym or "\x00" in sym:
                        continue
                    rows = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
                        query_id=CodegraphSqlQueryId.SELECT_NODES_EXACT,
                        params=(sym, sym),
                    ).fetchall()
                    if not rows:
                        like_pat = SQL_LOADER.escape_like_suffix(symbol_name=sym)
                        rows = SQL_LOADER.execute_safe(
                            conn=conn,
                            feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
                            query_id=CodegraphSqlQueryId.SELECT_NODES_SUFFIX_LIKE,
                            params=(like_pat,),
                        ).fetchall()
                    for row in rows:
                        node_id = str(row[0])
                        if node_id in seen_ids:
                            continue
                        seen_ids.add(node_id)
                        callers, callees = self._query_neighbors(conn=conn, node_id=node_id)
                        raw_nodes.append(
                            (
                                str(row[1]),
                                str(row[2]),
                                str(row[3]),
                                str(row[4]),
                                int(row[5]),
                                int(row[6]),
                                callers,
                                callees,
                            )
                        )
        except sqlite3.Error:
            return []

        if not raw_nodes:
            return []

        unique_files = list(dict.fromkeys(node[3] for node in raw_nodes))

        def _load_one_file(rel_path: str) -> tuple[str, list[str]]:
            """Read a single file's lines once for all symbols in that file.

            Args:
                rel_path: Relative file path within workspace.

            Returns:
                Tuple of `(rel_path, split_lines)`.
            """
            return rel_path, self._read_file_lines(ws_path=ws_path, file_path=rel_path)

        if len(unique_files) == 1:
            file_lines_map = dict([_load_one_file(rel_path=unique_files[0])])
        else:
            with ThreadPoolExecutor(max_workers=min(8, len(unique_files))) as pool:
                file_lines_map = dict(pool.map(_load_one_file, unique_files))

        slices: list[CodegraphSymbolSlice] = []
        for (
            kind,
            name,
            qualified_name,
            file_path,
            start_line,
            end_line,
            callers,
            callees,
        ) in raw_nodes:
            excerpt, used_bytes = self._slice_lines(
                lines=file_lines_map.get(file_path, []),
                start_line=start_line,
                end_line=end_line,
                max_bytes=min(per_symbol_cap, remaining_bytes),
            )
            remaining_bytes = max(0, remaining_bytes - used_bytes)
            slices.append(
                CodegraphSymbolSlice(
                    name=name,
                    qualified_name=qualified_name,
                    kind=kind,
                    file_path=file_path,
                    start_line=start_line,
                    end_line=end_line,
                    callers=callers,
                    callees=callees,
                    source_excerpt=excerpt,
                )
            )
        return slices

    def discover_symbols(
        self,
        workspace_dir: str,
        symbols: list[str] | None = None,
        files_inspected: list[str] | None = None,
        inline_text: str = "",
    ) -> list[str]:
        """Discover candidate symbols from explicit names, inline code, and `files_inspected`.

        Args:
            workspace_dir: Workspace directory containing `.codegraph/codegraph.db`.
            symbols: Optional explicit symbol names requested by the caller.
            files_inspected: Optional relative file paths to query in CodeGraph `nodes`.
            inline_text: Optional inline Python code or diff text to scan for `def`/`class`.

        Returns:
            Deduplicated list of up to 12 candidate symbol names.
        """
        discovered: list[str] = []
        seen: set[str] = set()
        for raw_sym in symbols or ():
            s = raw_sym.strip()
            if s and s not in seen:
                seen.add(s)
                discovered.append(s)
                if len(discovered) >= 12:
                    return discovered
        if inline_text:
            for m in _INLINE_SYMBOL_RE.finditer(inline_text):
                name = m.group(1)
                if name not in seen:
                    seen.add(name)
                    discovered.append(name)
                    if len(discovered) >= 12:
                        return discovered
        diff_hunks = _extract_diff_hunks(inline_text=inline_text) if inline_text else []
        if not files_inspected and not diff_hunks:
            return discovered[:12]
        resolved_ws = WorkspaceDigestCalculator().resolve_dir(workspace_dir=workspace_dir)
        if resolved_ws is None:
            return discovered[:12]
        db_path = Path(resolved_ws) / ".codegraph" / "codegraph.db"
        if not db_path.is_file():
            return discovered[:12]
        try:
            ro_uri = build_readonly_sqlite_uri(db_path=db_path)
            with contextlib.closing(sqlite3.connect(ro_uri, uri=True)) as conn:
                SQL_LOADER.execute_script(
                    conn=conn,
                    feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
                    query_id=CodegraphSqlQueryId.PRAGMA_READONLY_DEFENSE,
                )
                for rel_file, h_start, h_end in diff_hunks:
                    h_rows = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
                        query_id=CodegraphSqlQueryId.SELECT_NODES_BY_DIFF_HUNK,
                        params=(rel_file, h_end, h_start),
                    ).fetchall()
                    for row in h_rows:
                        sym_name = str(row[0] or "").strip()
                        if sym_name and sym_name not in seen:
                            seen.add(sym_name)
                            discovered.append(sym_name)
                            if len(discovered) >= 12:
                                return discovered
                norm_files = list(
                    dict.fromkeys(
                        Path(cleaned).as_posix()
                        for raw_file in files_inspected or ()
                        if (cleaned := raw_file.strip().removeprefix("./")) and cleaned != "."
                    )
                )
                if norm_files:
                    rows = SQL_LOADER.execute_safe(
                        conn=conn,
                        feature_file=SqlFeatureFile.CODEGRAPH_SLICER,
                        query_id=CodegraphSqlQueryId.SELECT_NODES_BY_FILES_JSON,
                        params=(json.dumps(obj=norm_files),),
                    ).fetchall()
                    for row in rows:
                        sym_name = str(row[0] or "").strip()
                        if sym_name and sym_name not in seen:
                            seen.add(sym_name)
                            discovered.append(sym_name)
                            if len(discovered) >= 12:
                                return discovered
        except sqlite3.Error:
            return discovered[:12]
        return discovered[:12]

    def format_context_block(
        self,
        workspace_dir: str,
        symbols: list[str] | None = None,
        max_bytes: int = 48_000,
        files_inspected: list[str] | None = None,
        inline_text: str = "",
    ) -> str:
        """Format matched CodeGraph symbol slices as a Markdown context block for prompts.

        Args:
            workspace_dir: Workspace directory containing `.codegraph/codegraph.db`.
            symbols: Optional list of symbol names or qualified names to slice.
            max_bytes: Maximum UTF-8 byte budget for symbol source excerpts.
            files_inspected: Optional relative file paths to auto-discover symbols from.
            inline_text: Optional inline code or diff text to auto-discover symbols from.

        Returns:
            Formatted Markdown string with symbol metadata and excerpts, or empty string.
        """
        effective_symbols = self.discover_symbols(
            workspace_dir=workspace_dir,
            symbols=symbols,
            files_inspected=files_inspected,
            inline_text=inline_text,
        )
        slices = self.slice_symbols(
            workspace_dir=workspace_dir,
            symbols=effective_symbols,
            max_bytes=max_bytes,
        )
        if not slices:
            return ""
        blocks: list[str] = ["### CodeGraph Symbol Slices"]
        for item in slices:
            header = (
                f"- `{item.qualified_name}` ({item.kind}) in "
                f"`{item.file_path}:{item.start_line}-{item.end_line}` | "
                f"callers={item.callers} | callees={item.callees}"
            )
            if item.source_excerpt:
                blocks.append(f"{header}\n```\n{item.source_excerpt}\n```")
            else:
                blocks.append(header)
        return "\n\n".join(blocks)
