# Development Harnesses

`rust_dev_harness/` is the active Rust 2024 engineering harness materialized from Alexandria `Jobs/Harness/Engineering Harness/02 Rust/`. Read its profile, machine configuration, manifest and rules router first. Normal and type rules are baseline; async rules are active for Tokio process/RPC/TUI work. `shared/` owns cross-language authority, testing, execution, manifest and minimality contracts.

`cargo xtask rules` verifies policy, Cargo lint inheritance and actual materialized byte hashes. `make ci` delegates to `cargo xtask ci`. The retired project Python implementation, backups and environment are deleted; source hashes and historical SQLite compatibility fixtures remain. All active runtime and quality gates are Rust-native.
