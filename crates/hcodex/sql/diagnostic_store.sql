-- name: pragma_query_only
PRAGMA query_only=ON

-- name: pragma_trusted_schema
PRAGMA trusted_schema=OFF

-- name: pragma_schema_version
PRAGMA schema_version

-- name: select_bounded_job
SELECT CASE WHEN length(CAST(record_json AS BLOB)) <= ? THEN record_json ELSE NULL END,
       length(CAST(record_json AS BLOB))
FROM jobs WHERE job_id = ?
