"""SQL file loader (`{feature}.sql`) and SQL injection defense guard for SQLite repositories."""

from __future__ import annotations

import re
import sqlite3
import threading
import urllib.parse
from pathlib import Path
from typing import Final

from app.repositories.sql_query_enums import SqlFeatureFile, SqlMultiStatementScriptId

type SqlParamScalar = str | int | float | None
type SqlParamTuple = tuple[SqlParamScalar, ...]

_SQL_DIR: Final[Path] = Path(__file__).resolve().parent / "sql"
_NAME_HEADER_RE: Final[re.Pattern[str]] = re.compile(
    pattern=r"^\s*--\s*name\s*:\s*([a-z][a-z0-9_]*)\s*$"
)
_STRING_LITERAL_RE: Final[re.Pattern[str]] = re.compile(pattern=r"'(?:''|[^'])*'")
_FORBIDDEN_SQL_RE: Final[re.Pattern[str]] = re.compile(
    pattern=r"\b(?:ATTACH|DETACH|LOAD_EXTENSION|WRITABLE_SCHEMA)\b",
    flags=re.IGNORECASE,
)
_MULTI_STATEMENT_SCRIPT_IDS: Final[frozenset[str]] = frozenset(SqlMultiStatementScriptId)


def _strip_literals_and_count_placeholders(sql_text: str) -> tuple[str, int]:
    """Strip single-quoted SQL string literals and count `?` parameter placeholders.

    Args:
        sql_text: Comment-stripped SQL query string.

    Returns:
        Tuple of `(literal_stripped_sql, placeholder_count)`.
    """
    stripped = _STRING_LITERAL_RE.sub(repl="''", string=sql_text)
    return stripped, stripped.count("?")


class SqlQueryLoader:
    """Thread-safe `{feature}.sql` loader and parameterized SQL injection defense executor."""

    def __init__(self, sql_dir: Path | None = None) -> None:
        """Initialize the SQL query loader and cache.

        Args:
            sql_dir: Optional custom directory containing `{feature}.sql` files.
        """
        self._sql_dir: Path = (sql_dir or _SQL_DIR).resolve()
        self._lock: threading.RLock = threading.RLock()
        self._cache: dict[str, dict[str, tuple[str, int]]] = {}

    def load_feature_queries(self, feature_file: SqlFeatureFile) -> dict[str, tuple[str, int]]:
        """Parse, validate, and cache all `-- name: <id>` queries from a `{feature}.sql` file.

        Args:
            feature_file: Target `SqlFeatureFile` enum member (`job_store.sql`, etc.).

        Returns:
            Mapping of `query_id` to `(validated_sql_text, placeholder_count)`.
        """
        fname = str(feature_file)
        with self._lock:
            if (cached := self._cache.get(fname)) is not None:
                return cached
            target_path = (self._sql_dir / fname).resolve()
            if not target_path.is_relative_to(self._sql_dir) or not target_path.is_file():
                raise FileNotFoundError(f"Missing or untrusted SQL file: {target_path}")
            raw_text = target_path.read_text(encoding="utf-8")
            parsed: dict[str, tuple[str, int]] = {}
            current_name: str | None = None
            current_lines: list[str] = []

            for raw_ln in raw_text.splitlines():
                if (m := _NAME_HEADER_RE.match(raw_ln)) is not None:
                    if current_name is not None:
                        parsed[current_name] = self._finalize_block(
                            query_id=current_name, lines=current_lines
                        )
                    current_name = m.group(1)
                    current_lines = []
                    continue
                if raw_ln.strip().startswith("--"):
                    continue
                if current_name is not None:
                    current_lines.append(raw_ln)

            if current_name is not None:
                parsed[current_name] = self._finalize_block(
                    query_id=current_name, lines=current_lines
                )
            self._cache[fname] = parsed
            return parsed

    def _finalize_block(self, query_id: str, lines: list[str]) -> tuple[str, int]:
        """Validate a single SQL query block against injection and stacked-query hazards.

        Args:
            query_id: Identifier of the query from `-- name: <query_id>`.
            lines: Non-comment SQL lines belonging to the block.

        Returns:
            Tuple of `(cleaned_sql, placeholder_count)`.
        """
        sql_text = "\n".join(ln for ln in lines if ln.strip()).strip()
        if not sql_text:
            raise ValueError(f"Empty SQL query block for '{query_id}'.")
        if _FORBIDDEN_SQL_RE.search(sql_text) is not None:
            raise ValueError(f"Forbidden SQL meta-command detected in '{query_id}'.")
        stripped, placeholder_cnt = _strip_literals_and_count_placeholders(sql_text=sql_text)
        if query_id not in _MULTI_STATEMENT_SCRIPT_IDS and ";" in stripped.rstrip(";"):
            raise ValueError(
                f"Stacked SQL statements (';') are forbidden in single-statement query '{query_id}'."
            )
        return sql_text, placeholder_cnt

    def get_query(self, feature_file: SqlFeatureFile, query_id: str) -> str:
        """Return the validated SQL string for `(feature_file, query_id)`.

        Args:
            feature_file: Target `SqlFeatureFile` enum member.
            query_id: Named query identifier inside `feature_file`.

        Returns:
            Validated SQL text string.
        """
        queries = self.load_feature_queries(feature_file=feature_file)
        if (entry := queries.get(query_id)) is None:
            raise KeyError(f"Unknown SQL query '{query_id}' in '{feature_file}'.")
        return entry[0]

    def execute_safe(
        self,
        conn: sqlite3.Connection,
        feature_file: SqlFeatureFile,
        query_id: str,
        params: SqlParamTuple = (),
    ) -> sqlite3.Cursor:
        """Validate parameter types/arity against the `{feature}.sql` template and execute safely.

        Args:
            conn: Active `sqlite3.Connection`.
            feature_file: Target `SqlFeatureFile` enum member.
            query_id: Named query identifier inside `feature_file`.
            params: Tuple of scalar positional parameters bound to `?` placeholders.

        Returns:
            Resulting `sqlite3.Cursor` from parameterized execution.
        """
        if query_id in _MULTI_STATEMENT_SCRIPT_IDS:
            raise ValueError(
                f"Multi-statement script '{query_id}' must be run via execute_script(), not execute_safe()."
            )
        queries = self.load_feature_queries(feature_file=feature_file)
        if (entry := queries.get(query_id)) is None:
            raise KeyError(f"Unknown SQL query '{query_id}' in '{feature_file}'.")
        sql_text, expected_arity = entry
        if len(params) != expected_arity:
            raise ValueError(
                f"SQL parameter arity mismatch for '{query_id}': expected {expected_arity}, got {len(params)}."
            )
        for idx, val in enumerate(params):
            if val is not None and not isinstance(val, (str, int, float)):
                raise TypeError(
                    f"Unsupported SQL parameter type at index {idx} for '{query_id}': {type(val).__name__}"
                )
            if isinstance(val, str) and "\x00" in val:
                raise ValueError(f"NUL byte injection blocked in SQL parameter {idx} for '{query_id}'.")
        return conn.execute(sql_text, params)

    def execute_script(
        self,
        conn: sqlite3.Connection,
        feature_file: SqlFeatureFile,
        query_id: str,
    ) -> None:
        """Execute an allowlisted zero-parameter schema/pragma script from `{feature}.sql`.

        Args:
            conn: Active `sqlite3.Connection`.
            feature_file: Target `SqlFeatureFile` enum member.
            query_id: Allowlisted script identifier (`init_schema`, `pragma_connect`, etc.).
        """
        if query_id not in _MULTI_STATEMENT_SCRIPT_IDS:
            raise ValueError(f"Query '{query_id}' is not an allowlisted multi-statement SQL script.")
        sql_text, expected_arity = self.load_feature_queries(feature_file=feature_file)[query_id]
        if expected_arity != 0:
            raise ValueError(f"SQL script '{query_id}' must not contain parameter placeholders.")
        for statement in (stmt.strip() for stmt in sql_text.split(";") if stmt.strip()):
            conn.execute(statement)

    @staticmethod
    def escape_like_suffix(symbol_name: str) -> str:
        """Escape SQL `LIKE` wildcard characters (`\\`, `%`, `_`) and format a `%.<symbol>` suffix pattern.

        Args:
            symbol_name: Raw symbol name supplied by a caller.

        Returns:
            Escaped `LIKE` pattern string safe for use with `ESCAPE '\\'`.
        """
        if "\x00" in symbol_name:
            raise ValueError("NUL byte injection blocked in LIKE pattern.")
        escaped = (
            symbol_name.replace("\\", "\\\\")
            .replace("%", "\\%")
            .replace("_", "\\_")
        )
        return f"%.{escaped}"


def build_readonly_sqlite_uri(db_path: Path) -> str:
    """Construct a URI-escaped read-only SQLite connection string preventing URI parameter injection.

    Args:
        db_path: Filesystem path to the target SQLite database file.

    Returns:
        Safe `file:<encoded_path>?mode=ro` URI string for `sqlite3.connect(..., uri=True)`.
    """
    resolved_posix = db_path.resolve().as_posix()
    if "\x00" in resolved_posix:
        raise ValueError("NUL byte blocked in SQLite database path.")
    encoded_path = urllib.parse.quote(string=resolved_posix, safe="/:")
    return f"file:{encoded_path}?mode=ro"


SQL_LOADER: Final[SqlQueryLoader] = SqlQueryLoader()

__all__ = [
    "SQL_LOADER",
    "SqlParamScalar",
    "SqlParamTuple",
    "SqlQueryLoader",
    "build_readonly_sqlite_uri",
]
