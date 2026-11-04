-- name: init_schema
PRAGMA trusted_schema=OFF;
PRAGMA cell_size_check=ON;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;
CREATE TABLE IF NOT EXISTS handoffs (
    ticket_id TEXT PRIMARY KEY,
    sender TEXT NOT NULL,
    recipient TEXT NOT NULL,
    status TEXT NOT NULL,
    priority TEXT NOT NULL,
    workspace_dir TEXT NOT NULL,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    ticket_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_handoffs_recipient_status ON handoffs(recipient, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_handoffs_workspace_updated ON handoffs(workspace_dir, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_handoffs_updated_at ON handoffs(updated_at DESC);

-- name: pragma_connect
PRAGMA trusted_schema=OFF;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;

-- name: pragma_enable_wal
PRAGMA journal_mode=WAL

-- name: pragma_wal_checkpoint
PRAGMA wal_checkpoint(TRUNCATE)

-- name: upsert_handoff
INSERT OR REPLACE INTO handoffs (ticket_id, sender, recipient, status, priority, workspace_dir, created_at, updated_at, ticket_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)

-- name: prune_resolved_handoffs
DELETE FROM handoffs WHERE status IN ('completed', 'rejected') AND ticket_id NOT IN (SELECT ticket_id FROM handoffs WHERE status IN ('completed', 'rejected') ORDER BY updated_at DESC LIMIT ?)

-- name: select_handoff_by_id
SELECT ticket_json FROM handoffs WHERE ticket_id = ?

-- name: select_handoffs_filtered_limited
SELECT workspace_dir, ticket_json FROM handoffs WHERE (? IS NULL OR recipient = ?) AND (? IS NULL OR status = ?) ORDER BY updated_at DESC LIMIT ?

-- name: count_handoffs_by_status
SELECT recipient, COUNT(*) FROM handoffs WHERE status = ? GROUP BY recipient
