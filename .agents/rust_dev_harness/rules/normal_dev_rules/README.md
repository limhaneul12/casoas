# Normal Rust Development Rules

Reusable **baseline non-type/non-async Rust engineering rules**. These rules always compose with `../type_dev_rules/`; async/concurrent changes additionally compose with `../async_dev_rules/`.

Read [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/00-overview|00-overview.md]] first, then the files matching the change:

- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/01-boundary|01-boundary.md]]: crate/domain/effect authority and dependency direction
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/02-workspace-crate-rules|02-workspace-crate-rules.md]]: Cargo workspace, toolchain, MSRV, features, dependency policy
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/03-deterministic-state-machine-rules|03-deterministic-state-machine-rules.md]]: deterministic transition/state-machine calculations
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/04-ffi-interop-boundary-rules|04-ffi-interop-boundary-rules.md]]: optional FFI/interop boundary
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/05-error-panic-rules|05-error-panic-rules.md]]: typed errors, panic discipline, invariant failures
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/06-testing-verification-rules|06-testing-verification-rules.md]]: unit, integration, property, corpus, feature-matrix, and runtime verification
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/07-performance-memory-rules|07-performance-memory-rules.md]]: runtime/build performance, allocation, RSS, scaling
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/08-observability-operations-rules|08-observability-operations-rules.md]]: structured diagnostics and operational evidence
- [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/normal_dev_rules/09-module-layout-rules|09-module-layout-rules.md]]: 2026 module file layout, responsibility-driven decomposition, narrow visibility, unit/integration test placement

Type modeling, ownership/borrowing, traits/generics, `Option`/`Result`, conversions, public type evolution, `Send`/`Sync` type semantics and unsafe type invariants live in `../type_dev_rules/` and are baseline for every Rust project.

## Composition

```text
ordinary Rust
→ normal_dev_rules + type_dev_rules

async/concurrent Rust
→ normal_dev_rules + type_dev_rules + async_dev_rules
```

No rule family replaces another. Async rules add execution/lifecycle/concurrency policy; type rules own compile-time representation and soundness policy.

## 2026 baseline

- Rust 1.98.0 research baseline; repository-owned stable/MSRV policy remains authority.
- Rust Edition 2024 / workspace resolver 3.
- Stable-first development.
- Shared workspace lint policy; `unsafe_code = "forbid"` by default.
- Current module-file convention for new/refactored trees prefers `foo.rs` + `foo/bar.rs`; `foo/mod.rs` remains supported legacy style and is preserved where repository convention requires it.
- Module decomposition follows responsibility/authority/change reason, not arbitrary source-file line counts.
- Unit tests remain inside the production module tree; root `tests/` is reserved for crate/application integration contracts.
- Visibility is minimized in the order private → `pub(super)` → `pub(crate)` → `pub`.
- Explicit crate/effect authority.
- Bounded resource/concurrency design.
- Evidence-gated correctness, compatibility, performance, and operations.
- No permanent dual authority or silent fallback after cutover.

## Project specialization

Project-specific runtime, feature matrix, unsafe/FFI approvals, dependency policy, public API compatibility, async execution model and verification gates belong in repository-local `PROJECT_PROFILE.md`, adapted from `../../PROJECT_PROFILE.template.md`.

Repository-local profile/config/source and executed verification are final authority for a concrete project.

## Graph Navigation
2026-09-24: existing routing converted to exact Vault links; policy preserved.
- Parent: [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/rules/README]]
