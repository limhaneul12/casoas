# 02 — Workspace, Toolchain, and Crate Rules

## Required

- Use an in-repository Cargo workspace when production Rust spans multiple crates or clearly separated authorities.
- Use Rust Edition 2024 for new/current code unless a documented compatibility requirement prevents it.
- Virtual workspaces explicitly use `resolver = "3"`.
- Declare a repository-owned toolchain policy. An accidental developer-local compiler must not define the build.
- If the project supports an MSRV, declare `rust-version` consistently and verify it. If it intentionally tracks current stable only, record that policy instead of inventing an MSRV.
- Put shared package metadata/dependencies/lints in workspace tables only where reuse is real.
- Safe workspace members inherit `[workspace.lints]` with `[lints] workspace = true`.
- Default safe-only policy uses `unsafe_code = "forbid"`.
- Use strict Clippy intentionally. Do not enable the entire restriction group mechanically.
- Add dependencies only for concrete capability and enable the smallest required feature set.
- Commit `Cargo.lock` for applications/binaries/services; libraries follow repository release policy.

## Machine projection

Repository-local `.agents/rust_dev_harness/HARNESS.toml` is the deterministic projection of `PROJECT_PROFILE.md` for xtask.

### Toolchain/MSRV

- `toolchain.policy = "current-stable"` → `workspace.package.rust-version` is not required by the shared Harness.
- `toolchain.policy = "msrv"` → configured MSRV must match Cargo `rust-version`.
- `rust-toolchain.toml` owns the actual execution channel when present, independent of whether a public MSRV exists.

## Lint authority

Cargo lint configuration is the lint-level authority.

Recommended safe baseline:

```toml
[workspace.lints.rust]
warnings = "deny"
unsafe_code = "forbid"
unused_must_use = "deny"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }
```

The canonical xtask does not append blanket `-D warnings` to Clippy, because doing so would silently upgrade reviewed `pedantic = "warn"` policy into deny. Rust compiler warnings remain strict through the Cargo rust lint group.

## Approved unsafe boundary

Safe-only is the default. If unsafe is genuinely required, prefer an isolated dedicated crate and declare its Cargo package name in `HARNESS.toml [unsafe].approved_crates`.

Approved unsafe crates:
- do not inherit workspace `unsafe_code = "forbid"`;
- define explicit local rust lint policy;
- explicitly allow unsafe code only within that crate boundary;
- deny `unsafe_op_in_unsafe_fn`;
- carry written proof/invariant ownership and stronger verification.

All non-approved members continue to inherit the safe workspace lints. There is no ad-hoc per-function shared-Harness escape marker.

## Feature policy

Choose one explicit strategy:
1. composable features → verify supported combined surface;
2. constrained/mutually exclusive features → define an explicit matrix in project profile.

`--all-features` is not a substitute for a real matrix when combinations are invalid.

## Crate boundaries

Crates represent real ownership/compile/dependency boundaries. Do not split crates merely to imitate architecture diagrams; do not keep unrelated high-churn domains together when a real authority/dependency boundary would improve the system.

## Dependency and supply-chain review

Review new direct dependencies, feature activation, build scripts, proc macros, native/sys crates and git sources proportionally to risk.

`cargo xtask deps` is an explicit fail-closed dependency gate. `cargo xtask extended` includes it only when `HARNESS.toml [verification].supply_chain = true`; repositories that do not require cargo-deny do not fail the unrelated extended lane solely because `deny.toml` is absent.

## Mechanical verification

Canonical entry points remain:

```text
cargo xtask rules
cargo xtask fmt
cargo xtask check
cargo xtask test
cargo xtask ci
cargo xtask perf
cargo xtask extended
cargo xtask doctor
cargo xtask deps
```

The executable harness and documentation MUST agree. Verification uses Cargo/rustc metadata semantically where possible rather than brittle source scanning.

## Shared contracts

Testing and generic agent execution are inherited from the Engineering Harness Shared contracts; this rule owns Rust-specific workspace/toolchain/crate/lint semantics only.
