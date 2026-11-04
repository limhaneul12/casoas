CREATE TRIGGER block_worktree_export BEFORE INSERT ON jobs
WHEN NEW.task_type = 'dag_worktree_export'
BEGIN
    SELECT RAISE(ABORT, 'fixture export failure');
END;
