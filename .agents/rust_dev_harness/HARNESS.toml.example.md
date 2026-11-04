# Rust HARNESS.toml.example

Machine-readable projection of the repository-local Rust `PROJECT_PROFILE.md`. The Markdown profile remains semantic authority; this TOML exists so `cargo xtask` can verify policy without brittle Markdown parsing.

```toml
[toolchain]
# "current-stable" or "msrv"
policy = "current-stable"
# Required only when policy = "msrv".
# msrv = "1.98"

[verification]
# Whether `cargo xtask extended` includes `cargo xtask deps` / cargo-deny.
supply_chain = false

[unsafe]
# "forbid" or "approved-crates"
mode = "forbid"
approved_crates = []
```

## Contract

### Toolchain / MSRV
- `policy = "current-stable"`: do not invent a `rust-version` solely for the shared Harness. `workspace.package.rust-version` may be absent.
- `policy = "msrv"`: `msrv` is required and must match the repository's declared Cargo `rust-version` policy.
- `rust-toolchain.toml` may still pin the actual execution channel independently from whether a public MSRV exists.

### Supply chain
- `supply_chain = true`: `deny.toml` and `cargo deny check` are required in the extended lane.
- `supply_chain = false`: `cargo xtask extended` must not fail merely because cargo-deny/deny.toml are absent. A project may still run `cargo xtask deps` explicitly.

### Unsafe
- `mode = "forbid"`: shared workspace/member policy keeps project Rust source safe-only and the canonical baseline may use `unsafe_code = "forbid"`.
- `mode = "approved-crates"`: unsafe is permitted only inside the explicitly named Cargo packages. Those crates become reviewed proof boundaries and must have project-local lint/soundness configuration and stronger verification. Other crates remain safe-only.
- The shared Harness does not support ad-hoc per-function escape markers. Prefer a dedicated crate boundary for approved unsafe code.

## Authority

`PROJECT_PROFILE.md` owns the semantic rationale. `HARNESS.toml` is its deterministic verifier projection. If the two disagree, treat it as Harness/project configuration drift rather than guessing which one the user intended.
