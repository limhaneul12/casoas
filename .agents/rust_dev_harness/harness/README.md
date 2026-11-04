# Rust xtask Harness

Cargo-native mechanical verification harness for pure Rust projects.

`xtask` does not replace methodology. `normal_dev_rules + type_dev_rules` define baseline policy, `async_dev_rules` adds concurrency policy, `PROJECT_PROFILE.md` defines semantic project authority, and `HARNESS.toml` is the deterministic machine projection used by xtask.

## Canonical shape

```text
rust_dev_harness/
├── PROJECT_PROFILE.md
├── HARNESS.toml
├── rules/
├── skills/rust-engineering/SKILL.md
└── harness/
    ├── README.md
    ├── Cargo.workspace.example.toml
    ├── rust-toolchain.toml.example
    ├── cargo-config.toml.example
    ├── deny.toml.example
    └── xtask/
        ├── Cargo.toml
        └── src/main.rs
```

No separate Makefile layer is canonical. Polyglot root runners may delegate to `cargo xtask ...`.

## Canonical commands

```text
cargo xtask rules
cargo xtask fmt
cargo xtask check
cargo xtask test
cargo xtask ci
cargo xtask deps
cargo xtask doctor
cargo xtask extended
cargo xtask perf
```

## Harness v2 policy alignment

### `rules`
Verifies:
- project profile + machine profile;
- all canonical rule assets and Rust skill;
- Edition 2024 / resolver 3;
- toolchain availability when declared;
- MSRV only when `HARNESS.toml` declares an MSRV contract;
- Cargo workspace lint authority;
- safe member lint inheritance;
- explicitly approved unsafe crate boundaries.

It does not require fake `rust-version` metadata for current-stable-only repositories.

### `check`
Runs workspace `cargo check` and `cargo clippy` using the repository toolchain. Cargo lint configuration is the lint-level authority.

The shared harness intentionally does **not** append blanket `-D warnings` to Clippy. Compiler warnings remain denied through `[workspace.lints.rust] warnings = "deny"`, while reviewed Clippy policy such as `pedantic = "warn"` retains its configured level.

### Unsafe policy
Safe-only remains default. Approved unsafe is crate-scoped and machine-profile-declared. Safe crates continue to inherit workspace `unsafe_code = "forbid"`; approved unsafe crates use explicit local lints and stronger proof/evidence.

### `deps`
Always explicit and fail-closed: requires project-owned `deny.toml` and `cargo deny check`.

### `extended`
Runs baseline CI and documentation gates. It includes `deps` only when `HARNESS.toml [verification].supply_chain = true`.

### `doctor`
Diagnoses ambient vs declared toolchain identity. A PATH mismatch is reported as a warning when xtask can still route all child Cargo/rustc commands through the repository-declared toolchain. The declared toolchain remains execution authority.

## Toolchain authority

When `rust-toolchain.toml` exists, xtask routes Cargo/rustc child processes through `rustup run <channel> ...`. Ambient PATH is diagnostic, not a second build authority.

When no toolchain file exists, repository profile/machine config must explicitly represent the intended current-stable/rolling policy.

## Module/test organization boundary

Generic xtask verifies that the module/test-organization rule exists and then relies on compiler/Clippy/tests/review for semantics. It does not fail solely on file length, `mod.rs`, child-module count, directory depth, or guessed responsibility splits.

Unit tests stay in the production module tree; package-root `tests/` remains integration/external-consumer territory. Production visibility is never widened solely for tests.

## Deeper gates

Miri, fuzzing, Loom/Shuttle, feature matrices, SemVer/downstream checks, ABI/integration/fault/performance lanes remain project-activated according to actual claims/risk.

Missing required deeper evidence is UNVERIFIED/BLOCKED, never a synthetic PASS.

## Shared contracts

- [[Jobs/Harness/Engineering Harness/00 Shared/00 Harness Authority and Layering Contract|Harness Authority and Layering Contract]]
- [[Jobs/Harness/Engineering Harness/00 Shared/01 Testing and Verification Contract|Shared Testing and Verification Contract]]
- [[Jobs/Harness/Engineering Harness/00 Shared/02 Agent Execution Contract|Shared Agent Execution Contract]]
