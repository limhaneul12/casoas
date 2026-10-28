"""Read-only SQLite `.codegraph/codegraph.db` symbol context slicer for prompt injection."""

from __future__ import annotations

import contextlib
import re
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from codex_collab_mcp.evidence.workspace_digests import resolve_workspace_dir

_INLINE_SYMBOL_RE: Final[re.Pattern[str]] = re.compile(
    r"\b(?:def|class)\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)


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
        caller_rows = conn.execute(
            "SELECT DISTINCT n.name FROM edges e JOIN nodes n ON e.source = n.id "
            "WHERE e.target = ? LIMIT 8",
            (node_id,),
        ).fetchall()
        callee_rows = conn.execute(
            "SELECT DISTINCT n.name FROM edges e JOIN nodes n ON e.target = n.id "
            "WHERE e.source = ? LIMIT 8",
            (node_id,),
        ).fetchall()
        callers = [str(r[0]) for r in caller_rows if r and r[0] is not None]
        callees = [str(r[0]) for r in callee_rows if r and r[0] is not None]
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
        resolved_ws = resolve_workspace_dir(workspace_dir)
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
            with contextlib.closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
                for raw_sym in symbols:
                    sym = raw_sym.strip()
                    if not sym:
                        continue
                    rows = conn.execute(
                        "SELECT id, kind, name, qualified_name, file_path, start_line, end_line "
                        "FROM nodes "
                        "WHERE name = ? OR qualified_name = ? "
                        "ORDER BY start_line ASC LIMIT 3",
                        (sym, sym),
                    ).fetchall()
                    if not rows:
                        rows = conn.execute(
                            "SELECT id, kind, name, qualified_name, file_path, start_line, end_line "
                            "FROM nodes "
                            "WHERE qualified_name LIKE ? "
                            "ORDER BY start_line ASC LIMIT 3",
                            (f"%.{sym}",),
                        ).fetchall()
                    for row in rows:
                        node_id = str(row[0])
                        if node_id in seen_ids:
                            continue
                        seen_ids.add(node_id)
                        callers, callees = self._query_neighbors(conn, node_id)
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
            return rel_path, self._read_file_lines(ws_path, rel_path)

        if len(unique_files) == 1:
            file_lines_map = dict([_load_one_file(unique_files[0])])
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
                file_lines_map.get(file_path, []),
                start_line,
                end_line,
                min(per_symbol_cap, remaining_bytes),
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
        if not files_inspected:
            return discovered[:12]
        resolved_ws = resolve_workspace_dir(workspace_dir)
        if resolved_ws is None:
            return discovered[:12]
        db_path = Path(resolved_ws) / ".codegraph" / "codegraph.db"
        if not db_path.is_file():
            return discovered[:12]
        try:
            with contextlib.closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
                for raw_file in files_inspected:
                    cleaned = raw_file.strip().removeprefix("./")
                    rel = Path(cleaned).as_posix() if cleaned else ""
                    if not rel or rel == ".":
                        continue
                    rows = conn.execute(
                        "SELECT DISTINCT name FROM nodes "
                        "WHERE file_path = ? AND kind IN ('class', 'function', 'method') "
                        "ORDER BY start_line ASC LIMIT 4",
                        (rel,),
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
        *,
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
            workspace_dir, symbols, files_inspected=files_inspected, inline_text=inline_text
        )
        slices = self.slice_symbols(workspace_dir, effective_symbols, max_bytes=max_bytes)
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


_CODEGRAPH_SLICER = CodegraphContextSlicer()
slice_codegraph_symbols = _CODEGRAPH_SLICER.format_context_block
