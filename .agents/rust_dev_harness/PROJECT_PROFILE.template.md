# Rust PROJECT_PROFILE Template

Repository-local semantic authority that connects the shared Rust harness to one concrete project. Materialize as `.agents/rust_dev_harness/PROJECT_PROFILE.md` together with the machine projection `.agents/rust_dev_harness/HARNESS.toml`.

```markdown
# <TARGET_PROJECT> Rust Project Profile

## Project shape
- kind: <library | application | service | cli | runtime | embedded | mixed>
- primary binaries/libraries: <TARGETS>
- normal rules: required
- type rules: required
- async/concurrency rules: <enabled | not-applicable>

## Framework / runtime profiles — when applicable
- activated profiles: <none | axum-backend | realtime-backend | other adopted profiles>
- framework/version authority: <Cargo workspace/lock/package path>
- profile-specific transport/runtime owner: <TARGET or not-applicable>
- activated optional capabilities: <none | sqlx | openapi | rmcp | sse | websocket | other>
- profile-specific verification: <TARGETS or not-applicable>

An activated framework/runtime profile **adds to** the Rust normal/type/async baseline; it does not replace or fork language rules. Optional capabilities are activated only when the repository actually uses them.

For `axum-backend`, record the repository's actual Axum/Tower/Tokio versions and only capabilities in use. Do not adopt SQLx/OpenAPI/RMCP/SSE/WebSocket or a fixed middleware/architecture template merely because the shared profile documents them.

For `realtime-backend`, additionally record:

- delivery semantics: <latest-state | lossy-live | ordered-live | resumable | bidirectional-command-event | mixed>
- ordering scope: <none | connection | entity/aggregate | workspace | global | other>
- live fan-out owner: <TARGET>
- durable replay/history owner: <TARGET or none>
- cursor/revision authority: <TARGET or not-applicable>
- per-connection/queue capacities: <bounds>
- slow-consumer policy: <backpressure | coalesce | drop-observable | disconnect-resync | replay | mixed>
- connection admission limits: <bounds>
- heartbeat/idle policy: <policy or not-applicable>
- reconnect/backoff policy: <policy>
- authentication/authorization lifetime: <policy>

When Axum SSE or WebSocket is active, activate both `axum-backend` and `realtime-backend`; Axum owns transport mechanics while Realtime owns delivery/backpressure/lifecycle/replay semantics.

Framework/runtime profile activation is semantic authority here. Do not invent new `HARNESS.toml` machine fields until xtask/verifier semantics for those fields are implemented and documented together.

## Toolchain and compatibility
- edition: 2024
- workspace resolver: 3
- toolchain policy: <pinned stable | rolling stable | explicit channel>
- rust-toolchain file: <path or none with rationale>
- MSRV policy: <explicit version | current-stable-only>
- Cargo.lock policy: <committed | library policy>

If `MSRV policy = explicit version`, `HARNESS.toml [toolchain].policy = "msrv"` and its `msrv` value must agree with Cargo `rust-version`. If current-stable-only, use `policy = "current-stable"`; do not invent `rust-version` solely to satisfy the Harness.

## Crate and authority map
| Crate / binary | Responsibility | Effects owned | Must not own |
| --- | --- | --- | --- |
| `<TARGET_CRATE>` | <domain/application/runtime/adapter/etc> | <effects> | <forbidden authority> |

Dependency direction and deliberate deviations are documented explicitly.

## Type and API contract
- public API stability: <internal-only | repository-stable | published SemVer>
- domain identity/newtype invariants: <authority>
- enum/state evolution: <closed | non-exhaustive/extensible>
- public trait policy: <downstream-implementable | sealed>
- conversion/parsing authority: <TARGET>
- ownership model for key resources: <single owner | borrowed | shared | other>
- intended thread-safety: <Send+Sync | Send-only | local/thread-affine | mixed by type>
- API/SemVer verification: <TARGET or not-applicable>

Do not add `Send`, `Sync`, `'static`, `Clone`, shared pointers, or broad trait bounds merely to satisfy one runtime implementation.

## Effects and external state
- persistence authority: <TARGET or none>
- network/transport authority: <TARGET or none>
- filesystem/process authority: <TARGET or none>
- clocks/randomness/IDs: <observation owner>
- configuration/environment ownership: <TARGET>

Deterministic domain decisions receive observations explicitly rather than reading hidden process-global state.

## Async / concurrency — when enabled
- runtime/executor: <Tokio/other>
- runtime model: <multi-thread/current-thread/local/etc>
- task ownership/supervision: <strategy>
- cancellation propagation: <strategy>
- queue/channel capacities: <bounds>
- connection/in-flight limits: <bounds>
- blocking bridge: <strategy/bound>
- CPU executor: <Rayon/dedicated/none and bound>
- graceful shutdown budget: <TARGET>
- concurrency verification: <virtual time/Loom/Shuttle/stress/etc>

## Feature policy
- composable features → supported combined verification; or
- constrained features → explicit supported matrix.

## Unsafe / FFI / native boundary
- default unsafe policy: `forbid`
- unsafe mode: <forbid | approved dedicated crates>
- approved unsafe crates: <none or package names>
- invariant/proof owner for each boundary: <TARGET>
- approved unsafe trait impls: <none or type + proof reference>
- FFI/native ABI: <none | C | cxx | UniFFI | PyO3 | other>
- required soundness verification: <Miri/Loom/real ABI/fuzz/stress/etc>

When unsafe is approved, prefer an isolated dedicated crate. Set `HARNESS.toml [unsafe].mode = "approved-crates"` and list exact Cargo package names. Safe members continue to inherit workspace `unsafe_code = "forbid"`; approved crates use explicit local lint/soundness policy.

## Dependency / supply-chain policy
- direct dependency review required: <yes/no>
- cargo-deny extended gate: <required | not-required>
- allowed registries/git sources: <policy>
- build-script/proc-macro/native-sys review: <policy>

Reflect cargo-deny requirement in `HARNESS.toml [verification].supply_chain`.

## Verification gates
### Mandatory CI
- `cargo xtask rules`
- `cargo xtask fmt`
- `cargo xtask check`
- `cargo xtask test`
- <project-specific mandatory gates>

### Framework/runtime conditional evidence
- Axum Router/Tower integration: <TARGET or not-applicable>
- SSE delivery/resume/proxy: <TARGET or not-applicable>
- WebSocket handshake/lifecycle/limits: <TARGET or not-applicable>
- Realtime lag/reconnect/shutdown: <TARGET or not-applicable>
- DB/OpenAPI/RMCP: <TARGETS or not-applicable>

### Extended / scheduled / high-risk
- `cargo xtask extended`
- explicit feature matrix / Miri / fuzz / Loom / Shuttle / API / integration / fault / load lanes as applicable

`cargo xtask deps` is always an explicit fail-closed command; `extended` includes it only when the machine profile enables the supply-chain gate.

### Performance
- `cargo xtask perf` is measurement evidence, not automatic PASS without project budgets and representative semantics.

## Operational lifecycle — when applicable
- readiness/liveness authority: <TARGET>
- shutdown sequence: <signal → stop admission → drain/cancel → close → join → report>
- task/resource leak evidence: <TARGET>
- long-lived connection drain/reconnect behavior: <TARGET or not-applicable>

## Explicit exceptions
Every permitted exception records the shared rule, scope, reason, compensating verification, owner, and review/expiry condition.
```

## Framework/runtime profile references

- Axum Backend → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/README|Axum Backend Profile]]
- Realtime Backend → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/README|Rust Realtime Backend Profile]]

## Machine projection

See [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/HARNESS.toml.example|Rust HARNESS.toml Example]]. `PROJECT_PROFILE.md` is semantic authority; `HARNESS.toml` exists so xtask can enforce deterministic fields without parsing prose.

Do not add machine fields that xtask does not understand merely to mirror semantic profile prose. Introduce profile-specific machine projection and verifier support as one bounded change when a reliable mechanical contract exists.

## Shared contracts
- [[Jobs/Harness/Engineering Harness/00 Shared/00 Harness Authority and Layering Contract|Harness Authority and Layering Contract]]
- [[Jobs/Harness/Engineering Harness/00 Shared/01 Testing and Verification Contract|Shared Testing and Verification Contract]]
- [[Jobs/Harness/Engineering Harness/00 Shared/02 Agent Execution Contract|Shared Agent Execution Contract]]
