---
name: axum-backend
description: Production Axum backend engineering methodology for HTTP, SSE and WebSocket adapters built on the Rust Engineering Harness. Use for routing, extractors, state, Tower middleware, API contracts, optional database or protocol integrations, realtime streaming, testing, security, operations and performance work around Axum.
version: 1.1.1
researched_at: 2026-09-09
realtime_updated_at: 2026-09-11
---

# Axum Backend Engineering

Procedure for executing Axum backend work under the governing Rust and Axum Rules. Rules are normative authority; this Skill does not create or weaken policy.

Profile → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/README|Axum Backend Profile]]

Rust methodology → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/rust-engineering/SKILL|Rust Engineering Skill]]

Realtime methodology → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/SKILL|Realtime Backend Engineering Skill]]

## 1. Load concrete authority

Read in this order:

1. closest repository-local `AGENTS.md` and project instructions;
2. `PROJECT_PROFILE.md` and `HARNESS.toml` when present;
3. repository Cargo workspace, lockfile, toolchain and configuration;
4. Rust normal, type and async rule routers;
5. Axum Backend Profile and task-relevant Axum Rules;
6. Realtime Backend Profile when SSE, WebSocket or another long-lived stream is touched;
7. current implementation, tests and runtime evidence.

Shared research is methodology. Repository-declared versions and accepted architecture remain concrete authority.

Do not silently upgrade Axum, Tower or Tokio or add optional frameworks because a shared reference uses a newer version.

## 2. Classify the change

Identify every affected capability before editing:

- route, method or fallback;
- handler or response;
- extractor or validation;
- state, dependency or composition root;
- middleware and Tower stack;
- authentication and authorization;
- request and resource limits;
- observability and shutdown;
- database integration;
- OpenAPI/public contract;
- RMCP/MCP;
- SSE;
- WebSocket;
- proxy or connection metadata;
- performance, backpressure or concurrency.

Then identify the externally observable contract: path, method, status, headers, body or stream message, auth, limits, idempotency, ordering, resume and lifecycle.

## 3. Find the current semantic owner

Trace the request/use-case end to end before creating a new abstraction:

```text
Router or transport
→ extractor or transport DTO
→ handler or connection adapter
→ application operation or event projection
→ domain
→ repository/effect
→ typed result/event
→ transport mapping
```

Ask:

- Does this application operation already exist?
- Is another protocol already calling it?
- Who owns this invariant, state transition or event revision?
- Is a new service, trait or module actually a new authority surface?

Prefer extending the existing correct owner over adding `Manager`, `Service`, trait, crate, `Arc`, lock or compatibility layer.

## 4. Model the transport contract

Before implementation, write the typed flow:

```text
external transport representation
→ syntactically checked transport value
→ checked application/domain value
→ command, query or subscription
→ typed result, error or event
→ HTTP response or live message
```

Separate malformed transport input from semantically invalid domain input.

For SSE/WebSocket first define latest, loss, ordering, replay and idempotency semantics under the Realtime Skill; do not start from framework API syntax.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/architecture|Architecture Patterns]]

## 5. Select extractors deliberately

Decision sequence:

1. application-global value → `State` or coherent `FromRef` substate;
2. request-derived without body consumption → built-in/request-parts extractor or `FromRequestParts`;
3. consumes or decodes the body → body extractor or `FromRequest`, last in the handler;
4. repeated transport-specific concern → consider a custom extractor;
5. proposed extractor hides business I/O or workflow → stop and move that work to the application owner.

Account for rejection semantics and body/resource limits.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/extractor-selection|Extractor Selection]]

## 6. Compose state and dependencies

Classify each dependency by:

- application-global versus request or connection scoped;
- construction and shutdown owner;
- cheap clone versus actual shared ownership;
- concurrency, pool and connection bounds;
- coherent substate ownership.

Avoid blanket `Arc`, outer mutexes around already concurrent pools/clients and universal service-locator state.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/state-patterns|State Patterns]]

## 7. Compose middleware from Tower outward

Selection ladder:

```text
existing Tower or Tower-HTTP capability
→ Tower combinator
→ axum::middleware::from_fn
→ from_fn_with_state
→ custom Layer/Service
→ custom future/runtime only with evidence
```

For every stack change reason about request/response order, error conversion, resource limits, auth placement, request ID/tracing, timeout/cancellation, readiness assumptions and bounded concurrency.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/tower-composition|Tower Composition]]

## 8. Implement the narrowest correct HTTP adapter

A normal handler should look conceptually like:

```text
extract
→ convert and validate transport representation
→ call application operation
→ map typed result/error into HTTP
```

Do not put SQL, retry loops, cross-service orchestration, state-machine decisions or business authorization directly into the handler when those have another owner.

For endpoint work follow → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/endpoint-workflow|Endpoint Workflow]]

## 9. Activate optional capability procedures only when adopted

### SQLx

Use the project repository/effect boundary, pool lifecycle, compile-time query checks where applicable, migration policy and real database evidence.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/sqlx-integration|SQLx Integration]]

### OpenAPI

Determine source-of-truth direction and the repository-selected integration. Verify runtime/schema/security/error agreement and compatibility where the contract is consumed.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/openapi|OpenAPI Integration]]

### RMCP

Reuse the application operation as authority. Prefer the official Rust MCP SDK and Tower/Axum mounting model over an independent protocol stack when it meets requirements.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/rmcp-integration|RMCP Integration]]

### SSE and WebSocket

Activate the Realtime Backend Profile first. Select SSE for server-push/live UI by default and WebSocket only for true duplex session requirements.

SSE rules → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/11-sse-realtime-rules|Axum SSE Realtime Rules]]

WebSocket rules → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/axum-backend/rules/12-websocket-realtime-rules|Axum WebSocket Realtime Rules]]

Realtime procedure → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/SKILL|Realtime Backend Engineering Skill]]

## 10. Test at the narrowest truthful boundary

Default adapter loop:

```text
application/domain focused test
→ fully composed Router as Tower Service
→ realtime channel/lifecycle test when active
→ real socket only when network behavior matters
→ capability integration lanes as needed
```

Exercise affected failure, limit and security paths, not only happy paths.

HTTP tests → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/testing|Testing Patterns]]

Realtime tests → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/testing|Realtime Testing]]

## 11. Run the verification ladder

```text
focused regression
→ cargo xtask rules
→ cargo xtask fmt
→ cargo xtask check
→ reviewed Clippy policy
→ targeted Axum and realtime tests
→ cargo xtask test
→ active capability integration evidence
→ cargo xtask ci
→ load, stress, fault or performance evidence when required
```

No unexecuted gate is PASS.

## 12. Performance and production claims

Before optimizing, identify whether the bottleneck is routing/middleware, connection fan-out, serialization, database wait, replay scanning, downstream I/O, lock contention, executor work, queue pressure, network/TLS or proxy behavior.

Measure the real stack and delivery semantics. Do not cite unrelated hello-world framework benchmarks as proof of product performance.

## 13. Adversarial review

Before completion, explicitly challenge:

- Did I move domain semantics into HTTP or connection types?
- Did I create a second operation/event authority for another transport?
- Did I add a state/service abstraction only for convenience?
- Can an untrusted request or connection create unbounded work?
- Does timeout actually cancel the side effect?
- Is middleware ordering security or reliability sensitive?
- Can secrets/internal details reach telemetry, error bodies, events or close reasons?
- Does generated OpenAPI match runtime behavior?
- For realtime, what happens to the slowest client and after reconnect?
- Is a framework-specific dependency leaking inward?

Axum anti-patterns → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/anti-patterns|Axum Anti-Patterns]]

Realtime anti-patterns → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/anti-patterns|Realtime Anti-Patterns]]

## 14. Completion report

Report:

- changed HTTP, application and domain authority boundaries;
- active optional capabilities;
- route, extractor, state, middleware, API and stream contract changes;
- delivery and replay implications;
- resource, security and async implications;
- exact focused tests and canonical gates executed;
- any database, OpenAPI, RMCP, network, reconnect, concurrency or performance evidence not run;
- benchmark evidence for every performance claim.

Axum research baseline → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/research-baseline|Axum Research Baseline]]

Realtime research baseline → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/research-baseline|Realtime Research Baseline]]
