-- name: replace_record_json
UPDATE jobs SET record_json = ? WHERE job_id = ?
