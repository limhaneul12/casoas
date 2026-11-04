# Axum Backend Profile

Production Axum HTTP backend specialization for the canonical Rust Engineering Harness.

## Role

This profile **adds framework/runtime rules to the Rust baseline; it does not fork or replace it**.

Authority chain:

```text
Shared Harness contracts
→ Rust normal + type + async rules
→ repository PROJECT_PROFILE / Cargo / toolchain / AGENTS
→ Axum Backend Profile
→ optional Realtime Backend Profile for SSE/WebSocket
→ task-relevant Skill / references
→ executed verification evidence
```

Parent bundle → [[Jobs/Harness/Engineering Harness/02 Rust/README|Rust Engineering Harness Bundle]]

Procedure → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/SKILL|Axum Backend Engineering Skill]]

Realtime profile → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/README|Rust Realtime Backend Profile]]

## Research baseline

Verified public baseline at the original Axum profile research date (2026-09-09) and rechecked for realtime mechanics on 2026-09-11:

- Axum `0.8.9`.
- Tower `0.5.3`.
- Tokio `1.53.1`.
- tower-http `0.7.x`.
- SQLx `0.9.0` when adopted.
- utoipa `5.5.x` / aide `0.15.x` are options, not mandatory OpenAPI authority.
- official Rust MCP SDK `rmcp` implements current stable MCP protocol support; MCP remains optional.

Repository-declared versions and lock/toolchain remain concrete authority. Shared research never silently upgrades a project.

## Activation

Activate this profile when Axum owns an HTTP transport boundary. Ordinary Axum backend work composes:

```text
Rust normal rules
+ Rust type rules
+ Rust async rules
+ Axum backend rules
```

Optional capabilities are explicit project decisions:

- `sqlx`
- `openapi`
- `rmcp`
- `sse`
- `websocket`
- browser cookie/session auth
- public Internet ingress

If `sse` or `websocket` is active, also activate the Realtime Backend Profile. Do not duplicate realtime delivery/backpressure/lifecycle policy in Axum-specific rules.

## Architectural invariant

Preferred dependency direction:

```text
HTTP / SSE / WebSocket transport
        ↓
Axum transport adapter
        ↓
application command/query/event projection
        ↓
domain
        ↓
ports/effects
```

HTTP status codes, headers, extractors, Axum rejection types, `Router`, Tower middleware, SSE/WebSocket transport objects and OpenAPI annotations remain at transport/composition boundaries unless a repository explicitly owns a different architecture.

A WebSocket/SSE connection is never durable domain state authority.

## Rule set

1. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/01-http-boundary-rules|HTTP Boundary Rules]]
2. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/02-routing-handler-rules|Routing and Handler Rules]]
3. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/03-extractor-validation-rules|Extractor and Validation Rules]]
4. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/04-state-dependency-rules|State and Dependency Rules]]
5. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/05-error-response-rules|Error and Response Rules]]
6. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/06-tower-middleware-rules|Tower Middleware Rules]]
7. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/07-security-resource-limit-rules|Security and Resource Limit Rules]]
8. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/08-observability-operation-rules|Observability and Operation Rules]]
9. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/09-api-contract-openapi-rules|API Contract and OpenAPI Rules]]
10. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/10-testing-verification-rules|Testing and Verification Rules]]
11. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/11-sse-realtime-rules|SSE Realtime Rules]] — conditional
12. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/12-websocket-realtime-rules|WebSocket Realtime Rules]] — conditional

## Profile-wide invariants

- Keep handlers thin because ownership belongs to application/domain owners, not because of arbitrary LOC limits.
- Use Axum typed extractors and Tower composition rather than inventing a parallel framework/runtime.
- Prefer existing Tower/Tower-HTTP capabilities before custom middleware.
- Every untrusted request/connection path has explicit resource bounds appropriate to its payload and workload.
- Middleware ordering is semantic behavior and must be reviewable; consequential ordering requires executable evidence.
- `State` is application-global composition state; request/connection identity stays scoped appropriately.
- Blocking/CPU-heavy work follows Rust async rules and never quietly blocks Tokio worker threads.
- HTTP/stream error mapping is explicit and does not leak internal error/debug/secret material.
- OpenAPI, SQLx, RMCP, SSE and WebSocket are capabilities, not universal requirements.
- Realtime streams declare latest/loss/replay/order semantics before transport selection.
- No performance or production-readiness claim without executed evidence.

## Realtime selection

Default decision:

```text
server → UI status/progress/events    → SSE
ordinary mutations + live updates    → HTTP command + SSE
continuous true bidirectional session → WebSocket
```

For SSE/WebSocket implementation use the shared Realtime Skill:

[[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/SKILL|Realtime Backend Engineering Skill]]

## Verification

Baseline:

```text
focused endpoint/adapter regression
→ cargo xtask rules
→ cargo xtask fmt
→ cargo xtask check
→ reviewed Clippy policy
→ targeted Axum integration tests
→ cargo xtask test
→ cargo xtask ci
```

Conditional gates:

- SQLx: migration/query metadata verification and real DB integration where required.
- OpenAPI: generated contract snapshot/diff or consumer validation when API stability matters.
- RMCP: negotiated protocol/transport integration and tool schema tests.
- SSE: delivery/keepalive/resume/slow-client/proxy tests according to deployed semantics.
- WebSocket: handshake/Origin/auth/message bounds/read-write lifecycle/close/reconnect tests.
- concurrency/backpressure: saturation/cancellation evidence when those claims change.
- performance: representative connection/event/load benchmarks before claiming improvement.

## Research discipline

Version-specific syntax and framework defaults live primarily in Skill references, not broad invariants. When Axum/Tower/Tokio move, update the research baseline and affected mechanics while preserving valid architectural rules.

Axum research provenance → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/research-baseline|Axum Research Baseline]]

Realtime research provenance → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/research-baseline|Realtime Research Baseline]]
