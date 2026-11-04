# Shared Testing and Verification Contract

Canonical cross-language testing doctrine for Engineering Harness packages.

## Core philosophy

1. Test observable behavior, not implementation structure.
2. Mock only real external/system boundaries; keep owned internal collaborators real when practical.
3. Prefer Classist/Chicago-style tests over interaction-heavy Mockist tests.
4. Fewer meaningful tests beat large brittle suites.
5. Verification depth scales with change risk and claimed behavior.

## Test-first rule

For a behavior change that warrants automated coverage:
1. state the behavior in one sentence;
2. write a failing behavior test first when practical;
3. implement the smallest change;
4. run the smallest deterministic check that exercises it;
5. broaden only when project policy or risk requires it.

Do not create a test merely to satisfy TDD ceremony for getters, constants, framework glue, plain CRUD already covered by a critical journey, soon-deleted code, or similarly low-value cases.

## Mocking and assertions

- Mock/fake only external services, persistence engines, network/process boundaries, clock/randomness, and similarly non-owned effects.
- Prefer real temporary filesystems/databases/queues when deterministic and proportionate.
- Assert return values and observable state.
- Interaction assertions such as call counts/arguments are secondary evidence, not the primary proof.
- Never snapshot non-deterministic LLM, network, timestamp, or ordering-free output.

## Property-based testing

Use property/generative tests when a clear invariant spans a large input space: parser/codec/state machine/order/hash/round-trip/validator logic. If a fourth example test is being added for the same invariant, consider a property test.

Language tools:
- Python: Hypothesis
- Rust: proptest and stronger concurrency/soundness tools where appropriate
- TypeScript: fast-check

## Flaky tests

Never hide flakiness with retries, sleeps, or arbitrary timeout inflation. Fix shared global state, real-clock dependence, ordering, unseeded randomness, network coupling, or lifecycle defects at the root.

## Verification ladder

```text
focused behavior evidence
→ language/compiler/type/lint checks
→ module/integration tests
→ canonical CI gate
→ deeper safety/compatibility/performance/runtime evidence when claimed
```

A broad suite that already passed does not need to be rerun repeatedly unless relevant code changed, a failure appeared, or unresolved risk justifies it.

## Evidence states

- DISCOVERED: requirement/issue identified
- PLANNED: implementation/verification path identified
- IMPLEMENTED: change exists but required evidence is incomplete
- VERIFIED: required gates actually ran and passed
- BLOCKED: concrete authority/tooling/external dependency prevents completion

No unexecuted gate is PASS. Worker self-report is not independent verification.

## Language specialization

Language packages add only language-specific semantics:
- Python: pytest/type checker/runtime-boundary specifics
- Rust: Cargo test placement, proptest, Miri/Loom/Shuttle/feature/SemVer gates
- TypeScript: tsc/build/browser/framework/package evidence
- Python-Rust: ABI/wheel/free-threaded/FFI/async bridge provenance
