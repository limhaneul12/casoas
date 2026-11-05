# Repository Agent Authority (`hcodex`)

Before modifying Rust code, read the locally materialized `.agents/README.md` and `.agents/rust_dev_harness/{PROJECT_PROFILE.md,HARNESS.toml,HARNESS_MANIFEST.toml,rules/README.md}`. Activate normal + type rules for all Rust work, and async rules for task/process/network work. Repository code, typed boundaries and executed gates own concrete behavior.

`.agents/` and `.cargo/` are local-only ignored directories; do not force-add them.
Portable verification uses `xtask/HARNESS.toml` and the project research asset
manifest. `make local-harness` explicitly verifies the local Rust/shared
materialization; a missing local bundle is not claimed as verified by public CI.

## Required contracts

- Rust Edition 2024; toolchain comes from `rust-toolchain.toml`; Cargo.lock is application dependency authority.
- Safe-only: `unsafe_code = "forbid"`; no unwrap/expect/panic/todo/unimplemented in production.
- Closed external input uses typed serde structures with `deny_unknown_fields`; historical persisted data remains readable without weakening new input validation.
- One owner per effect: routing, runtime, storage, deterministic DAG/settlement, context, and CLI/TUI adapters.
- SQL lives in feature `.sql` files and uses prepared bindings. Enforce NUL, stacked statement and dangerous meta-command defenses. External CodeGraph DB handles are encoded read-only/immutable/query_only.
- Own every task and child process. Bound queues/concurrency; cancellation and timeout kill/reap owned children. Persist unknown outcomes without inventing completion or starting a new request ID.
- Actual model/usage/evidence must come from runtime receipts; queued handoffs are not collaboration completion, and role completion is not consensus.
- Settled DAG nodes require real gate proof and scoped workspace-manifest verification. Skills and code context contain actual disk bodies/excerpts.
- Preserve pre-existing dirty work, persisted state, CLI/TUI behavior and source contracts. The retired hcodex MCP transport must not be restored as a second runtime authority.

## Canonical verification

```sh
make ci
```

This executes the published Rust mechanical policy and project asset integrity, formatting, workspace checks, strict all-target/all-feature Clippy with `-D warnings`, and tests. No skipped gate is PASS. The former project Python implementation, backups and environment are deleted; source hashes and historical SQLite compatibility fixtures remain.
