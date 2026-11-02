-- SQL queries for JobStore & SqliteJobEngine (`job_store.sql`)
-- All runtime values MUST be bound via `?` positional parameters.

-- name: init_schema
PRAGMA trusted_schema=OFF;
PRAGMA cell_size_check=ON;
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;
CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY,
    task_type TEXT NOT NULL,
    status TEXT NOT NULL,
    outcome_state TEXT NOT NULL,
    idempotency_key TEXT,
    workspace_dir TEXT NOT NULL,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    record_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_idempotency_key
    ON jobs(idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_jobs_created_at
    ON jobs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_workspace_created
    ON jobs(workspace_dir, created_at DESC);

-- name: pragma_connect
PRAGMA trusted_schema=OFF;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;

-- name: pragma_wal_checkpoint
PRAGMA wal_checkpoint(TRUNCATE)

-- name: upsert_job
INSERT OR REPLACE INTO jobs (
    job_id,
    task_type,
    status,
    outcome_state,
    idempotency_key,
    workspace_dir,
    created_at,
    updated_at,
    record_json
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)

-- name: prune_completed_jobs
DELETE FROM jobs
WHERE status != 'running'
  AND job_id NOT IN (
      SELECT job_id
      FROM jobs
      WHERE status != 'running'
      ORDER BY created_at DESC
      LIMIT ?
  )
RETURNING job_id

-- name: select_job_by_id
SELECT record_json
FROM jobs
WHERE job_id = ?

-- name: select_job_by_idempotency_key
SELECT job_id, record_json
FROM jobs
WHERE idempotency_key = ?
ORDER BY created_at DESC
LIMIT 1

-- name: select_jobs_by_workspace
SELECT job_id, workspace_dir, record_json
FROM jobs
WHERE workspace_dir = ?
ORDER BY created_at DESC
LIMIT ?

-- name: select_jobs_excluding_workspace
SELECT job_id, workspace_dir, record_json
FROM jobs
WHERE workspace_dir != ?
ORDER BY created_at DESC
LIMIT ?

-- name: select_recent_jobs
SELECT job_id, workspace_dir, record_json
FROM jobs
ORDER BY created_at DESC
LIMIT ?

-- name: select_jobs_by_status
SELECT job_id, record_json
FROM jobs
WHERE status = ?

-- name: summarize_job_telemetry
SELECT
    COUNT(*),
    COALESCE(SUM(CASE WHEN status IN ('completed', 'succeeded') THEN 1 ELSE 0 END), 0),
    COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0),
    COALESCE(SUM(CASE WHEN status = 'timed_out_unknown' THEN 1 ELSE 0 END), 0),
    COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
    COALESCE(SUM(CASE WHEN status = 'cancelled' THEN 1 ELSE 0 END), 0),
    COALESCE(AVG(CASE WHEN status != 'running' AND updated_at >= created_at THEN updated_at - created_at END), 0.0)
FROM jobs
