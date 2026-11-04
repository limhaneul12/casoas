# 06 — Rust Testing and Verification Rules

Rust-specific supplement to [[Jobs/Harness/Engineering Harness/00 Shared/01 Testing and Verification Contract|Shared Testing and Verification Contract]]. Shared behavior-first, boundary-mocking, flaky-test, evidence-state, and verification-calibration rules are inherited rather than duplicated here.

## Rust-specific layers

Choose layers according to the actual contract/risk:

1. **Unit** — pure functions, invariants, edge/error paths inside the production module tree.
2. **Integration** — crate/application boundaries and real adapters where relevant.
3. **Golden / compatibility** — observable migration/compatibility oracles.
4. **Property / generative** — parsers, codecs, state transitions, ordering, hashing, numerical invariants and round-trips; prefer `proptest` when examples begin multiplying around one invariant.
5. **Corpus / fixture** — real-world protocols/documents/formats.
6. **Feature matrix** — supported combinations, not blindly `--all-features` when combinations are invalid.
7. **Async / concurrency** — cancellation, shutdown, ownership, backpressure, virtual time/interleavings; Loom/Shuttle/stress when the risk warrants it.
8. **Unsafe / FFI** — real boundary tests plus Miri/sanitizer/fuzz/stress as applicable.
9. **Public API / SemVer** — downstream/consumer compatibility where published identity matters.
10. **Runtime/release provenance** — prove the executed artifact is the intended build when artifact identity is a claim.

## Test organization

Rust test location follows compilation/privacy semantics:

- unit tests remain in the production module tree under `#[cfg(test)]`;
- small suites may be inline;
- large suites may move to an adjacent child `tests.rs` while preserving private-module semantics;
- package-root `tests/` is integration/external-consumer territory;
- never widen production visibility solely so an integration test can reach internals.

Detailed ownership/layout policy lives in `09-module-layout-rules.md`.

## CI tiers

### Fast inner loop
- compiler/rust-analyzer feedback;
- focused `cargo check`;
- focused behavior/regression test.

### Mandatory CI
- `cargo xtask rules`;
- formatting;
- workspace check;
- Cargo-configured reviewed Clippy policy;
- primary tests;
- project-required feature/dependency/source gates.

### Extended / high-risk
Only when applicable:
- broader feature matrix;
- Miri;
- fuzz + regression corpus;
- Loom/Shuttle;
- downstream/SemVer checks;
- expensive integration/fault/performance lanes.

Missing deeper tooling makes that lane UNVERIFIED/BLOCKED, not PASS.

## Quality

- deterministic fixtures/stable seeds where expected;
- avoid arbitrary wall-clock sleeps;
- cover failure/cancellation/shutdown paths;
- declare tolerances and source/version assumptions for approximate algorithms;
- execute commands before claiming success.

## Migration / cutover

Compatibility oracles may be temporary evidence. Once authority cuts over, retire the old production implementation unless a product requirement explicitly retains it. Tests may retain fixtures/reference expectations without keeping a second production owner.

## Completion

Report exact commands/gates that ran, what passed/failed, and what remains unverified.
