# Axum Research Baseline — 2026-09-09

Research/provenance reference for the Axum Backend Profile. This document records why the Rules/Skill say what they say. It is **not** an independent Rule source.

## 1. Source authority order

Research was resolved in this order:

```text
Axum / Tokio / Tower / tower-http official docs + repositories
→ official protocol/library docs for SQLx / RMCP / OpenAPI crates
→ current crate/release metadata
→ production-oriented community guides/Skills
→ examples/blogs only as hypotheses or anti-pattern sources
```

When a community Skill conflicts with official released behavior, official release documentation wins.

Repository-local Cargo/toolchain/source/tests still outrank this shared research for a concrete project.

## 2. Released framework baseline at research time

| Component | Research baseline | Role |
|---|---:|---|
| Axum | 0.8.9 | HTTP routing/extraction/response |
| Tower | 0.5.3 | Service/Layer composition |
| Tokio | 1.53.1 | async runtime |
| tower-http | 0.7.x | HTTP middleware/utilities |
| SQLx | 0.9.0 | optional persistence capability |
| utoipa | 5.5.x | optional OpenAPI option |
| aide | 0.15.x | optional OpenAPI option |
| MCP | 2026-07-28 stable | optional RMCP protocol capability |

Axum upstream `main` was moving toward 0.9; released project work should follow the repository's pinned version/release branch rather than examples from unreleased `main`.

## 3. Primary Axum findings

### Tower is the middleware model

Official Axum middleware documentation explicitly states that Axum does not invent a separate bespoke middleware system; it integrates with Tower/Tower-HTTP.

Harness consequence:

```text
reuse Tower/Tower-HTTP
→ local from_fn when appropriate
→ custom Layer/Service only for a real capability/control boundary
```

This is the basis for rejecting a parallel boxed middleware runtime as the default Axum pattern.

Primary references:

- https://docs.rs/axum/latest/axum/middleware/index.html
- https://github.com/tokio-rs/axum
- https://docs.rs/tower/latest/tower/
- https://docs.rs/tower-http/latest/tower_http/

### Middleware ordering is behavior

Tower `ServiceBuilder` documents layer order and shows that concurrency/buffer ordering changes admitted/in-flight semantics.

Harness consequence:

- middleware order must be reviewable;
- correctness-sensitive order gets behavioral tests;
- queue/concurrency/load-shedding policy is explicit rather than decorative.

Primary reference:

- https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html

### Axum Router backpressure is special

Official Axum routing/middleware documentation warns that routing assumes destination services are effectively ready and does not preserve arbitrary inner-service `poll_ready` backpressure in the way a general Tower pipeline might expect.

Harness consequence:

- do not mount a backpressure-sensitive service and assume readiness propagates;
- use explicit load shedding/bounded buffering/concurrency or an outer service boundary;
- no unbounded queue workaround.

Primary reference:

- https://docs.rs/axum/latest/axum/middleware/index.html

### State versus request extensions

Official Axum state docs distinguish application-global `State` from request-derived values commonly carried through request extensions, and provide `FromRef` for substates.

Harness consequence:

- `State` is composition state, not request identity;
- use coherent `FromRef` substates where useful;
- do not create a mega-state service locator by default.

Primary references:

- https://docs.rs/axum/latest/axum/extract/struct.State.html
- https://docs.rs/axum/latest/axum/extract/index.html

### Extractor ownership and handler contract

Official handler/extractor docs establish:

- request-parts extractors can coexist before body extraction;
- the body-consuming extractor owns the whole request/body and appears last;
- handler futures must satisfy Axum's `Send` requirements;
- common handler failures arise from holding non-`Send` values across `.await`.

Harness consequence:

- extractor choice follows request ownership;
- do not solve handler `Send` errors by broad `Arc`/`Send`/`'static` escalation without understanding ownership.

Primary references:

- https://docs.rs/axum/latest/axum/extract/index.html
- https://docs.rs/axum/latest/axum/handler/trait.Handler.html

### Default body limit is not universal

Axum 0.8 documentation states that common Bytes-based extractors (`Bytes`, `String`, `Json`, `Form`, etc.) have a default 2 MiB limit, while raw/custom body polling can bypass that extractor-level default.

Harness consequence:

- do not write the inaccurate rule “Axum requests are always limited to 2 MiB”;
- raw/streaming/custom body paths require explicit bounds;
- disabling `DefaultBodyLimit` on untrusted inputs requires an equivalent resource contract.

Primary reference:

- https://docs.rs/axum/latest/axum/extract/struct.DefaultBodyLimit.html

## 4. Tokio findings

### Graceful shutdown requires owned tasks/cancellation

Tokio guidance uses explicit shutdown detection/signaling and waiting for owned tasks, including patterns based on `CancellationToken`/task tracking.

Harness consequence:

- server shutdown is not only `Ctrl-C` handling;
- background tasks need owner, cancellation, error observation and completion policy.

Primary references:

- https://tokio.rs/tokio/topics/shutdown
- https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html

### `spawn_blocking` is not an unlimited CPU executor

Tokio documentation notes that started blocking tasks cannot simply be aborted, and recommends bounding CPU-heavy blocking work or using a dedicated CPU executor/threading strategy.

Harness consequence:

- request timeout != blocking-task cancellation;
- per-request unbounded `spawn_blocking` is prohibited;
- CPU-heavy work gets an explicit concurrency/executor owner.

Primary reference:

- https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html

### Bounded concurrency/backpressure remains baseline

Tokio bounded channels/semaphores provide explicit capacity controls. This reinforces the existing Rust async Harness rather than creating Axum-specific duplicate concurrency rules.

Primary references:

- https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html
- https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html

## 5. Tower/tower-http findings

### Request ID and tracing order

Tower HTTP request-ID documentation/examples show request IDs must be established before tracing if spans should observe them, and response propagation order matters.

Harness consequence: observability middleware ordering has executable semantics.

References:

- https://docs.rs/tower-http/latest/tower_http/request_id/
- https://docs.rs/tower-http/latest/tower_http/trace/

### Request body limits

`tower_http::limit::RequestBodyLimitLayer` can reject excessive request bodies with `413 Payload Too Large` and can use known `Content-Length` to reject before reading the body.

Harness consequence: use a service-level limit for raw/custom body paths when Axum extractor limits are not sufficient.

Reference:

- https://docs.rs/tower-http/latest/tower_http/limit/

## 6. SQLx findings

SQLx `query!`/`query_as!` support compile-time checked SQL metadata against the intended database/schema. Offline build/CI workflows can use prepared `.sqlx` metadata and `cargo sqlx prepare --check`.

Harness consequence:

- SQLx is optional capability, not Axum baseline;
- DB claims require DB/schema truth evidence;
- adopted offline metadata has a deterministic freshness gate;
- SQL/transactions belong to persistence/application operation owners, not handlers.

Primary references:

- https://docs.rs/sqlx/latest/sqlx/macro.query.html
- https://docs.rs/sqlx/latest/sqlx/macro.migrate.html
- https://github.com/launchbadge/sqlx/tree/main/sqlx-cli

## 7. OpenAPI findings

### utoipa

`utoipa` supports modern OpenAPI generation with derive/macros, and `utoipa-axum` integrates route collection with Axum.

References:

- https://docs.rs/utoipa/latest/utoipa/
- https://docs.rs/utoipa-axum/latest/utoipa_axum/

### aide

`aide` offers Axum API/router integration around `schemars`-style schema generation.

Reference:

- https://docs.rs/aide/latest/aide/axum/

Harness decision:

- do not mandate one library globally;
- select according to existing schema authority/macro tolerance/dependencies;
- generated OpenAPI is not runtime/auth proof;
- spec/runtime/consumer compatibility gets separate evidence when relevant.

## 8. RMCP / MCP findings

The official Rust MCP SDK implements current MCP protocol support and provides a Streamable HTTP service compatible with Tower/Axum mounting.

Current stable protocol baseline at research time: MCP `2026-07-28`.

Harness consequence:

- prefer official RMCP over a home-grown MCP JSON-RPC server when it meets requirements;
- application operation, not REST transport, is the semantic reuse boundary;
- stateless service factories must not rely on hidden mutable per-session service state;
- MCP schema/negotiation/tool dispatch requires its own integration evidence.

Primary references:

- https://github.com/modelcontextprotocol/rust-sdk
- https://modelcontextprotocol.io/specification/2026-07-28

## 9. Official Axum testing findings

Axum's official examples/documentation show the Router used directly as a Tower service with `oneshot` for request/response tests, `ready + call` for repeated service use, and real sockets only when network behavior itself matters. Axum also provides connection-info testing support such as `MockConnectInfo`.

Harness consequence:

- default HTTP integration seam is the real composed Router without TCP;
- real listener tests are narrower, not universally “more correct.”

Primary references:

- https://github.com/tokio-rs/axum/tree/main/examples/testing
- https://docs.rs/axum/latest/axum/extract/connect_info/struct.MockConnectInfo.html

When the upstream `main` example targets unreleased Axum, translate syntax/mechanics to the repository's released version rather than copying blindly.

## 10. Secondary community sources audited

### `gruberb/bulletproof-rust-web`

Useful production-oriented signals:

- thin HTTP handlers;
- explicit architecture/dependency direction;
- SQLx repository patterns;
- tracing/security/CI guidance;
- `AGENTS.md` + Skill style compatible with agentic engineering.

Used as a **secondary pattern source**, not authority.

Repository:

- https://github.com/gruberb/bulletproof-rust-web

### `fusengine/agents` Rust web backend Skill

Useful signals:

- current Axum 0.8 route syntax;
- State/Tower/error/SQLx patterns;
- concise agent-oriented workflow.

Its captured dependency baseline was not treated as version authority because current official Tokio release metadata had moved beyond the version embedded in that Skill.

Repository:

- https://github.com/fusengine/agents

### Axum multi-Skill packages

Broad Axum Skill packages were useful as a coverage checklist for extractors, middleware, auth, SSE/WebSocket, OpenAPI, testing and deployment. Their content is treated as hypotheses until cross-checked against official docs.

## 11. Rejected/stale pattern example

A public `axum-service-architecture` Skill claimed Axum 0.8.7 while still showing route syntax such as:

```rust
.route("/users/:id", ...)
```

Axum 0.8 uses `/{id}` syntax. This mismatch is preserved as an anti-pattern lesson:

> A Skill's stated version is not proof its examples match that version.

Harness consequence:

- version-specific examples live in references;
- official released docs are rechecked when framework mechanics change;
- architectural invariants are kept separate from syntax trivia.

## 12. Decisions deliberately NOT encoded as universal rules

Research did **not** justify these as mandatory for every Axum project:

- SQLx;
- PostgreSQL;
- utoipa;
- aide;
- OpenAPI at all;
- RMCP/MCP;
- JWT;
- cookie sessions;
- `/v1` path versioning;
- repository pattern/trait per entity;
- `Arc<dyn Trait>` dependency injection;
- Swagger UI in production;
- one fixed middleware stack/order;
- one fixed concurrency/body/rate limit;
- one fixed directory architecture.

These remain capability/project decisions constrained by the governing Rules.

## 13. Research-to-rule mapping

```text
Official Axum transport/Tower design
→ HTTP/routing/Tower Rules

Extractor/State/DefaultBodyLimit docs
→ extractor/state/security Rules

Tokio shutdown/blocking docs
→ operations/security + inherited async Rules

SQLx official docs
→ optional sqlx-integration reference + conditional verification

utoipa/aide official docs
→ optional OpenAPI decision reference

official RMCP/MCP
→ optional RMCP reference and protocol integration evidence

community guides/Skills
→ coverage, examples, anti-pattern hypotheses only
```

## 14. Maintenance rule

When updating this baseline:

1. read released Axum/Tower/Tokio/RMCP/SQLx/OpenAPI sources;
2. identify which findings are mechanics versus enduring invariants;
3. update version-specific references first;
4. modify normative Rules only when their semantic premise changed;
5. update tests/verifier assets only when they can prove the changed contract reliably;
6. never distort production code to satisfy a stale shared verifier/example.

Research date: **2026-09-09 (KST)**.
