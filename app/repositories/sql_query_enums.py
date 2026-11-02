"""StrEnum definitions for `{feature}.sql` files and named SQL query identifiers."""

from __future__ import annotations

from enum import StrEnum


class SqlFeatureFile(StrEnum):
    """Canonical `{feature}.sql` filenames managed under `app/repositories/sql/`."""

    JOB_STORE = "job_store.sql"
    HANDOFF_STORE = "handoff_store.sql"
    CODEGRAPH_SLICER = "codegraph_slicer.sql"


class JobSqlQueryId(StrEnum):
    """Named SQL query identifiers inside `app/repositories/sql/job_store.sql`."""

    INIT_SCHEMA = "init_schema"
    PRAGMA_CONNECT = "pragma_connect"
    PRAGMA_WAL_CHECKPOINT = "pragma_wal_checkpoint"
    UPSERT_JOB = "upsert_job"
    PRUNE_COMPLETED_JOBS = "prune_completed_jobs"
    SELECT_JOB_BY_ID = "select_job_by_id"
    SELECT_JOB_BY_IDEMPOTENCY_KEY = "select_job_by_idempotency_key"
    SELECT_JOBS_BY_WORKSPACE = "select_jobs_by_workspace"
    SELECT_JOBS_EXCLUDING_WORKSPACE = "select_jobs_excluding_workspace"
    SELECT_RECENT_JOBS = "select_recent_jobs"
    SELECT_JOBS_BY_STATUS = "select_jobs_by_status"
    SUMMARIZE_JOB_TELEMETRY = "summarize_job_telemetry"


class HandoffSqlQueryId(StrEnum):
    """Named SQL query identifiers inside `app/repositories/sql/handoff_store.sql`."""

    INIT_SCHEMA = "init_schema"
    PRAGMA_CONNECT = "pragma_connect"
    PRAGMA_WAL_CHECKPOINT = "pragma_wal_checkpoint"
    UPSERT_HANDOFF = "upsert_handoff"
    PRUNE_RESOLVED_HANDOFFS = "prune_resolved_handoffs"
    SELECT_HANDOFF_BY_ID = "select_handoff_by_id"
    SELECT_HANDOFFS_FILTERED_LIMITED = "select_handoffs_filtered_limited"
    SELECT_HANDOFFS_FILTERED_ALL = "select_handoffs_filtered_all"
    COUNT_HANDOFFS_BY_STATUS = "count_handoffs_by_status"


class CodegraphSqlQueryId(StrEnum):
    """Named SQL query identifiers inside `app/repositories/sql/codegraph_slicer.sql`."""

    PRAGMA_READONLY_DEFENSE = "pragma_readonly_defense"
    SELECT_NEIGHBORS_BY_NODE_ID = "select_neighbors_by_node_id"
    SELECT_NODES_EXACT = "select_nodes_exact"
    SELECT_NODES_SUFFIX_LIKE = "select_nodes_suffix_like"
    SELECT_NODES_BY_DIFF_HUNK = "select_nodes_by_diff_hunk"
    SELECT_NODES_BY_FILES_JSON = "select_nodes_by_files_json"


class SqlMultiStatementScriptId(StrEnum):
    """Allowlisted multi-statement DDL/PRAGMA script identifiers across `{feature}.sql` files."""

    INIT_SCHEMA = "init_schema"
    PRAGMA_CONNECT = "pragma_connect"
    PRAGMA_READONLY_DEFENSE = "pragma_readonly_defense"


__all__ = [
    "CodegraphSqlQueryId",
    "HandoffSqlQueryId",
    "JobSqlQueryId",
    "SqlFeatureFile",
    "SqlMultiStatementScriptId",
]
