"""Read-only SQLite `.codegraph/codegraph.db` symbol context slicer for prompt injection."""

from __future__ import annotations

import contextlib
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

from codex_collab_mcp.evidence.evidence_dicts import CodegraphSymbolSliceDict


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

    def to_dict(self) -> CodegraphSymbolSliceDict:
        """Serialize the symbol slice to an explicit TypedDict.

        Returns:
            Serialized CodegraphSymbolSliceDict mapping.
        """
        return CodegraphSymbolSliceDict(
            name=self.name,
            qualified_name=self.qualified_name,
            kind=self.kind,
            file_path=self.file_path,
            start_line=self.start_line,
            end_line=self.end_line,
            callers=self.callers,
            callees=self.callees,
            source_excerpt=self.source_excerpt,
        )


class CodegraphContextSlicer:
    """Read-only `.codegraph/codegraph.db` slicer extracting symbol definitions and 1-hop edges."""

    def _resolve_workspace(self, workspace_dir: str) -> Path | None:
        """Resolve and validate the workspace directory path.

        Args:
            workspace_dir: Raw workspace directory path string.

        Returns:
            Resolved workspace Path if valid, otherwise None.
        """
        if not workspace_dir.strip():
            return None
        try:
            return Path(workspace_dir).expanduser().resolve()
        except OSError, RuntimeError, ValueError:
            return None

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

    def _read_excerpt(
        self,
        ws_path: Path,
        file_path: str,
        start_line: int,
        end_line: int,
        max_bytes: int,
    ) -> str:
        """Read lines `start_line..end_line` from a workspace-contained file bounded by `max_bytes`.

        Args:
            ws_path: Resolved workspace root directory path.
            file_path: Node file path relative to `ws_path` (or inside `ws_path`).
            start_line: 1-based inclusive start line number.
            end_line: 1-based inclusive end line number.
            max_bytes: Remaining UTF-8 byte budget for the excerpt.

        Returns:
            UTF-8 source excerpt string (or empty string if unreadable or out of budget).
        """
        if max_bytes <= 0 or not file_path.strip():
            return ""
        try:
            candidate = (ws_path / file_path).resolve()
            if not candidate.is_relative_to(ws_path) or not candidate.is_file():
                return ""
            lines = candidate.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError, RuntimeError, ValueError:
            return ""
        lo = max(1, start_line) - 1
        hi = max(lo + 1, end_line)
        excerpt = "\n".join(lines[lo:hi])
        encoded = excerpt.encode("utf-8")
        if len(encoded) <= max_bytes:
            return excerpt
        return encoded[:max_bytes].decode("utf-8", errors="ignore")

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
        ws_path = self._resolve_workspace(workspace_dir)
        if ws_path is None:
            return []
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
                        "WHERE name = ? OR qualified_name = ? OR qualified_name LIKE ? "
                        "ORDER BY start_line ASC LIMIT 3",
                        (sym, sym, f"%.{sym}"),
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

        def _load_node_excerpt(
            item: tuple[str, str, str, str, int, int, list[str], list[str]],
        ) -> str:
            """Read a single node's source excerpt bounded by `per_symbol_cap`.

            Args:
                item: Tuple of node metadata and caller/callee lists.

            Returns:
                Extracted UTF-8 source excerpt string.
            """
            return self._read_excerpt(ws_path, item[3], item[4], item[5], per_symbol_cap)

        if len(raw_nodes) == 1:
            excerpts = [_load_node_excerpt(raw_nodes[0])]
        else:
            with ThreadPoolExecutor(max_workers=min(8, len(raw_nodes))) as pool:
                excerpts = list(pool.map(_load_node_excerpt, raw_nodes))

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
        ), raw_ex in zip(raw_nodes, excerpts, strict=True):
            encoded = raw_ex.encode("utf-8")
            if len(encoded) > remaining_bytes:
                excerpt = encoded[:remaining_bytes].decode("utf-8", errors="ignore")
            else:
                excerpt = raw_ex
            remaining_bytes = max(0, remaining_bytes - len(excerpt.encode("utf-8")))
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

    def format_context_block(
        self,
        workspace_dir: str,
        symbols: list[str] | None,
        max_bytes: int = 48_000,
    ) -> str:
        """Format matched CodeGraph symbol slices as a Markdown context block for prompts.

        Args:
            workspace_dir: Workspace directory containing `.codegraph/codegraph.db`.
            symbols: Optional list of symbol names or qualified names to slice.
            max_bytes: Maximum UTF-8 byte budget for symbol source excerpts.

        Returns:
            Formatted Markdown string with symbol metadata and excerpts, or empty string.
        """
        slices = self.slice_symbols(workspace_dir, symbols, max_bytes=max_bytes)
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
