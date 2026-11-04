-- name: release_export_block
DROP TRIGGER block_worktree_export

-- name: delete_collision_fixture_job
DELETE FROM jobs WHERE job_id = ?
