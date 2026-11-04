-- name: init_schema
CREATE TABLE IF NOT EXISTS recovery_changes (
    change_id TEXT PRIMARY KEY NOT NULL,
    record_json TEXT NOT NULL,
    record_sha256 TEXT NOT NULL
);

-- name: insert_recovery
INSERT INTO recovery_changes(change_id, record_json, record_sha256) VALUES (?, ?, ?)

-- name: select_recovery
SELECT change_id,
       CASE WHEN length(CAST(record_json AS BLOB)) <= 16384 THEN record_json END,
       substr(record_sha256, 1, 72)
FROM recovery_changes WHERE change_id = ?

-- name: corrupt_recovery_test
UPDATE recovery_changes SET record_json = ? WHERE change_id = ?
