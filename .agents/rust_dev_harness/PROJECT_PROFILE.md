# hcodex Rust Project Profile

- Product: Heterarchy Codex MCP/CLI and DAG-ULW++ interactive TUI.
- Rust Edition 2024; application workspace members: `crates/hcodex`, `xtask`.
- Execution toolchain: repository-owned `rust-toolchain.toml` (1.98.0); rolling stable policy, no published MSRV.
- Runtime: Tokio; explicit task ownership, timeouts/cancellation, bounded queues and subprocess kill/reap.
- Domain owners: routing/model contracts; deterministic DAG topology/hazard compiler and settlement; skill/source context.
- Effects: runtime owns Codex subprocess/Antigravity RPC; storage owns local SQLite; adapters own CLI/MCP/TUI.
- SQLite queries live in `crates/hcodex/sql/`; prepared parameters only; external CodeGraph read-only/immutable/query_only.
- Safe-only: workspace unsafe_code forbid; no approved unsafe crates.
- Framework profiles: async rules active; Axum/realtime HTTP profiles inactive (stdio MCP).
- Feature strategy: composable; canonical checks use all targets/all features.
- Cargo.lock is application dependency authority.
- Canonical checks: `cargo xtask rules`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-targets --all-features`; `make ci` delegates to `cargo xtask ci`.
- Production cutover: all active runtime, package entry points and tests are Rust-native. Project Python sources, implementation/harness backups, environment and helper scripts are deleted; immutable SQLite fixtures and source hash metadata retain compatibility/provenance proof.
