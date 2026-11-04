-- name: init_schema
PRAGMA trusted_schema=OFF;
PRAGMA cell_size_check=ON;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;
CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY,
    task_type TEXT NOT NULL,
    status TEXT NOT NULL,
    outcome_state TEXT NOT NULL,
    idempotency_key TEXT,
    idempotency_owner INTEGER NOT NULL DEFAULT 0,
    workspace_dir TEXT NOT NULL,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    record_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_idempotency_key ON jobs(idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_jobs_created_at ON jobs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_workspace_created ON jobs(workspace_dir, created_at DESC);

-- name: pragma_connect
PRAGMA trusted_schema=OFF;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;

-- name: pragma_enable_wal
PRAGMA journal_mode=WAL

-- name: pragma_wal_checkpoint
PRAGMA wal_checkpoint(TRUNCATE)

-- name: insert_job
INSERT INTO jobs (job_id, task_type, status, outcome_state, idempotency_key, idempotency_owner, workspace_dir, created_at, updated_at, record_json)
VALUES (?, ?, ?, ?, ?, CASE WHEN ? IS NULL THEN 0 ELSE 1 END, ?, ?, ?, ?)

-- name: update_job
UPDATE jobs
SET task_type = ?,
    status = ?,
    outcome_state = ?,
    idempotency_key = ?,
    workspace_dir = ?,
    created_at = ?,
    updated_at = ?,
    record_json = ?
WHERE job_id = ?

-- name: pragma_job_columns
PRAGMA table_info(jobs)

-- name: alter_add_idempotency_owner
ALTER TABLE jobs ADD COLUMN idempotency_owner INTEGER NOT NULL DEFAULT 0

-- name: pragma_job_indexes
PRAGMA index_list(jobs)

-- name: assign_idempotency_owners
UPDATE jobs
SET idempotency_owner = CASE
    WHEN idempotency_key IS NOT NULL
     AND job_id = (
        SELECT candidate.job_id
        FROM jobs AS candidate
        WHERE candidate.idempotency_key = jobs.idempotency_key
        ORDER BY candidate.created_at ASC, candidate.job_id ASC
        LIMIT 1
     )
    THEN 1
    ELSE 0
END

-- name: create_unique_idempotency_owner_index
CREATE UNIQUE INDEX IF NOT EXISTS uq_jobs_idempotency_owner
ON jobs(idempotency_key)
WHERE idempotency_key IS NOT NULL AND idempotency_owner = 1

-- name: prune_completed_jobs
DELETE FROM jobs WHERE status != 'running' AND job_id NOT IN (SELECT job_id FROM jobs WHERE status != 'running' ORDER BY created_at DESC LIMIT ?)

-- name: select_job_by_id
SELECT record_json FROM jobs WHERE job_id = ?

-- name: select_job_by_idempotency_key
SELECT record_json FROM jobs WHERE idempotency_key = ? ORDER BY idempotency_owner DESC, created_at ASC, job_id ASC LIMIT 1

-- name: select_jobs_by_workspace
SELECT workspace_dir, record_json FROM jobs WHERE workspace_dir = ? ORDER BY created_at DESC LIMIT ?

-- name: select_jobs_excluding_workspace
SELECT workspace_dir, record_json FROM jobs WHERE workspace_dir != ? ORDER BY created_at DESC LIMIT ?

-- name: select_recent_jobs
SELECT workspace_dir, record_json FROM jobs ORDER BY created_at DESC LIMIT ?

-- name: select_jobs_by_status
SELECT record_json FROM jobs WHERE status = ?

-- name: summarize_job_telemetry
SELECT COUNT(*), COALESCE(SUM(CASE WHEN status IN ('completed', 'succeeded') THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'timed_out_unknown' THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'cancelled' THEN 1 ELSE 0 END), 0), COALESCE(AVG(CASE WHEN status != 'running' AND updated_at >= created_at THEN updated_at - created_at END), 0.0) FROM jobs
