# 10 — Unsafe Type Invariant Rules

## Default

Shared Rust policy is safe-only by default. Safe workspace members use `unsafe_code = "forbid"`.

A repository that genuinely requires unsafe code must declare an **approved dedicated Cargo crate boundary** in `PROJECT_PROFILE.md` and `.agents/rust_dev_harness/HARNESS.toml`, with written proof ownership and stronger verification. The reusable Harness does not support silent ad-hoc unsafe exceptions inside otherwise safe crates.

`unsafe` is a proof obligation, not a performance toggle or a workaround for borrow/type errors.

## Approved crate boundary

Preferred structure:

```text
crates/
├─ core/              # workspace safe lints, unsafe forbidden
├─ runtime/           # workspace safe lints, unsafe forbidden
└─ native_boundary/   # explicitly approved unsafe crate
```

An approved unsafe crate:
- is listed by exact Cargo package name in `HARNESS.toml [unsafe].approved_crates`;
- does not inherit the workspace `unsafe_code = "forbid"` lint;
- explicitly sets local `unsafe_code = "allow"` and `unsafe_op_in_unsafe_fn = "deny"`;
- keeps compiler warnings strict;
- documents unsafe invariants and proof ownership;
- receives stronger Miri/fuzz/stress/ABI evidence as applicable.

This crate-level exception exists because Rust `forbid` cannot be relaxed downstream. Safe crates therefore retain `forbid`, while the approved proof boundary has an explicit local lint contract.

## Localize the proof inside the approved crate

Within an approved unsafe crate, keep unsafe operations in the smallest practical module/function and expose a safe abstraction where possible.

For every unsafe boundary document:
- invariant relied upon;
- who establishes/preserves it;
- pointer validity/alignment/lifetime requirements;
- aliasing/mutation rules;
- initialization/layout/thread-safety requirements;
- what safe callers may do.

Safe public methods must not allow callers to violate internal safety invariants.

## `unsafe fn` and constructors

Use `unsafe fn` only when the caller must uphold a safety precondition that the callee cannot enforce. If the callee can validate/construct the invariant safely, expose a safe checked API.

Do not mark an operation unsafe merely because its implementation contains an unsafe block.

## Unsafe trait implementations

`unsafe impl` such as manual `Send`/`Sync` asserts the full safety contract. State the proof explicitly and review reachable interior state, destruction, concurrency, FFI and aliasing behavior.

Never add manual `Send`/`Sync` merely to satisfy Tokio/PyO3/thread-pool bounds.

## High-risk primitives

Raw pointers, `NonNull`, `MaybeUninit`, unions, manual allocation, FFI pointers, `transmute`, unchecked indexing/conversion and manual ownership transfer require precise validity reasoning.

Prefer standard-library safe/checked APIs. Do not create references before pointer validity/alignment/aliasing/lifetime requirements hold, read uninitialized memory, or construct duplicate owners accidentally.

## Layout / FFI

Ordinary Rust type layout is not stable ABI. Use `repr(C)` / `repr(transparent)` only when the layout contract is required and review padding/alignment/niche implications.

Mixed Python↔Rust projects additionally apply the cross-language Harness.

## Verification

Unsafe changes require stronger evidence according to risk:
- compiler + reviewed Clippy baseline;
- focused boundary tests;
- Miri where supported;
- fuzz/property/corpus tests for hostile conversion/input paths;
- Loom/stress/fault tests for synchronization protocols;
- real ABI/platform/runtime tests for FFI/layout contracts.

Passing tools do not prove soundness by themselves; the written invariant/proof remains required.

## Forbidden

- unsafe outside an approved crate when the Harness mode is safe-only/approved-crates;
- undocumented unsafe blocks/impls;
- broad workspace-wide unsafe allowance;
- unsafe introduced solely for unmeasured optimization;
- safe APIs that can manufacture invalid internal state;
- permanent suppression of soundness warnings without reviewed proof.
