# Rust command launcher repair

The retired Python launcher in `~/.local/bin/hcodex` precedes the installed Rust binary on PATH. Installing Cargo's binary and updating MCP configuration alone therefore leaves the interactive command broken.

Keep launcher ownership in lifecycle setup/sync. Match only the historical three-line launcher naming this checkout's `PYTHONPATH`, virtualenv interpreter and `app.core.ctl` module. Replace that regular file atomically with a symlink to the resolved Rust executable. Preserve unrelated launchers and symlinks. Do not restore Python or change shell configuration. Cover migration, repeat setup and preservation with focused regression tests before changing production code. Then run lifecycle tests, formatting and canonical CI, and verify the actual PATH-selected command plus a native PTY TUI launch.

## Execution evidence

- The new regression failed against the old implementation, then lifecycle's 12 tests passed after the repair. Arbitrary command files and user symlinks remain unchanged; repeat sync reports no migration.
- `cargo clippy -p hcodex --all-targets --all-features -- -D warnings`, owned-file `rustfmt --check`, and `sh -n scripts/install.sh` passed. Root owns canonical CI after concurrent DAG changes finish.
- `scripts/install.sh --activate` installed Rust and returned `launcher_migrated=true`, with no MCP configuration change. The actual shadowing path is now a symlink to `~/.cargo/bin/hcodex`.
- PATH-selected `hcodex tui -c /models` and `/status` exited zero. Native PTY `hcodex tui` rendered topology/transcript and Ctrl-C exited zero, restoring bracketed paste, alternate screen and cursor visibility. These checks establish launcher and TUI startup, not provider/DAG execution.
- Receipts: `.omx/evidence/launcher-repair-20261103/`. This interim installed artifact must be replaced by the final root build after concurrent DAG edits.
