# Owned verification processes

The DAG gate previously timed out `Command::output`, which only killed the direct child on drop and did not account for descendants. Triad gates already owned a process group and reaped it, but buffered all output. Preserve each caller's command allowlist and gate DTO while moving the shared effect into a crate-private verification runner.

The runner owns a child process group, concurrently hashes stdout and stderr in fixed-size buffers, and responds to runtime cancellation or timeout by killing the owned process group and waiting for the direct child. It returns explicit completed/timed-out/cancelled outcome, exit code and separate output digests. No output collection, dependency, public command or persistence schema is added. Triad keeps its stdout-only receipt digest. DAG adopts the same explicit stdout evidence scope.

Tests exercise actual subprocess success/output digests, timeout cleanup after a ready marker, and supervisor cancellation with descendants. Parent DAG owns adapter integration. Root owns canonical CI and final runtime proof.

Focused evidence: three shared-runner tests pass, including 1 MiB stdout plus separate stderr hashing, supervisor cancellation after both PIDs are recorded, and timeout after the direct child has exited but a descendant still holds its pipes. The direct child is reaped and descendants are absent or non-running zombies awaiting the OS parent reaper. The two existing triad gate tests pass unchanged. DAG now consumes a SHA-256 digest of the ordered stdout/stderr digest pair; historic receipt values remain readable.
