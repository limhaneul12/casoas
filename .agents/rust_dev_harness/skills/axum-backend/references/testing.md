# Axum Testing Patterns

Non-normative testing reference. Governing Shared/Rust/Axum Testing Rules remain authority.

## 1. Test the real Router without the network by default

Axum `Router` is a Tower service after required state is supplied. For most HTTP adapter tests, exercise the real routing/extractor/middleware/response stack directly.

Typical pattern:

```rust
use axum::{body::Body, http::Request};
use tower::ServiceExt;

#[tokio::test]
async fn create_user_returns_created() {
    let app = build_test_router();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"a@example.test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}
```

Test code may use `unwrap`/`expect` for fixture construction under repository test policy; production request-path rules are distinct.

## 2. `oneshot` versus `ready + call`

Use `oneshot` for an isolated request:

```text
build service
→ send one request
→ consume service instance
```

Use `ready + call`/cloned router where multiple requests or readiness sequencing are part of the test.

Do not infer production backpressure behavior from `oneshot` alone when custom services depend on `poll_ready`.

## 3. Reuse production composition

Prefer one composition function:

```rust
pub fn build_router(state: AppState) -> Router { ... }
```

Tests provide controlled state/dependencies to the same router assembly used by production. Avoid a second test-only route tree that can drift from real middleware/order/routes.

## 4. Layer tests by claim

```text
domain/application test
  proves semantic decision

Router/Tower test
  proves route/extractor/middleware/status/body contract

real DB test
  proves SQL/schema/transaction semantics

real socket test
  proves actual connection/listener/proxy/network behavior

OpenAPI diff/test
  proves documented contract

RMCP integration
  proves MCP protocol/tool contract
```

Run only the layers needed for the claim, but do not substitute one for another.

## 5. Negative-path matrix

For changed public behavior, consider:

| Case | What it catches |
|---|---|
| unknown path | fallback/404 |
| known path wrong method | 405/method routing |
| missing path/query/header | extractor/rejection contract |
| malformed JSON/query/path | parse/rejection contract |
| wrong/missing content type | decoder semantics |
| oversized body | resource bound |
| unauthenticated | authentication short circuit |
| forbidden | authorization semantics |
| not found/conflict | semantic error mapping |
| timeout/overload | middleware/error mapping |
| internal failure | sanitized 5xx |

Do not mechanically add all cases to every endpoint; select those affected by the contract.

## 6. Verify middleware ordering through effects

Example: authentication must run before a protected side effect.

```text
unauthenticated request
→ assert 401/403 according to contract
→ assert fake/spy application operation was not invoked
```

Example: body limit before expensive work:

```text
oversized body
→ assert 413
→ assert handler/application operation not invoked
```

Example: request ID:

```text
request without ID
→ response has generated ID
→ captured handler/trace context sees same ID
```

## 7. Dependency substitutes

Substitute at legitimate application/effect seams:

```rust
#[derive(Clone)]
struct FakeUserRepo { ... }
```

or an existing trait/port when the architecture already owns one.

Do not create a new dynamic trait solely because one endpoint test needs a fake; a concrete in-memory test dependency or real ephemeral service may be simpler and more truthful.

## 8. Connection metadata

When using Axum `ConnectInfo`, official test utilities such as `MockConnectInfo` can provide deterministic connection metadata without binding a real socket.

Use an actual `TcpListener` when testing:

- socket address behavior itself;
- real HTTP protocol/interoperability;
- listener/server startup/shutdown;
- proxy/TLS integration that cannot be represented by direct service calls.

## 9. Real server test pattern

For real network behavior:

```text
bind 127.0.0.1:0
→ obtain chosen ephemeral port
→ spawn owned server task
→ call with real client
→ trigger graceful shutdown
→ await server task
```

Do not use fixed ports in parallel tests unless a harness owns isolation.

Track the spawned server task; do not leak detached servers between tests.

## 10. Body reading in tests

When asserting response bodies, apply a test-side limit appropriate to the expected response so a bug cannot allocate unbounded memory in the test harness.

Deserialize typed error/success envelopes where contract shape matters instead of checking fragile string fragments.

## 11. SQLx tests

When SQLx is active:

- use an isolated test database/schema/container according to repository harness;
- apply canonical migrations;
- test real constraints/transactions;
- avoid order dependence;
- clean/isolate per test or use rollback where semantics permit;
- run `cargo sqlx prepare --check` when offline metadata is part of CI.

HTTP mocks cannot prove SQL/schema correctness.

## 12. OpenAPI tests

If public contract is active:

```text
generate spec
→ parse/validate
→ compare canonical snapshot/diff
→ inspect intended change
→ Router tests for representative runtime agreement
```

Do not snapshot huge generated output if ordering noise makes review meaningless; canonicalize or use semantic diff tooling.

## 13. RMCP tests

Separate:

- application semantics;
- tool schema/name/description;
- protocol initialize/negotiation;
- tool call dispatch;
- Axum/Tower Streamable HTTP integration;
- auth/idempotency for mutating tools.

Do not claim MCP support from `/mcp` returning any HTTP response.

## 14. Shutdown/cancellation tests

Use deterministic coordination:

```text
start owned task/server
→ begin known in-flight operation
→ trigger cancellation/shutdown
→ assert new traffic stops as intended
→ assert owned task completes/cancels within policy
→ assert durable side-effect outcome
```

Virtual time may help with Tokio timer behavior. Blocking tasks require separate design because started `spawn_blocking` work cannot simply be aborted.

## 15. Load/backpressure tests

For concurrency/load-shedding changes, assert invariants rather than only throughput:

```text
max in-flight ≤ configured bound
queue ≤ configured bound
excess requests reject or wait according to policy
no memory growth from unbounded buffering
status/error class is intentional
recovery after saturation works
```

## 16. Completion evidence

A useful report is explicit:

```text
PASS  focused application test: ...
PASS  Router negative tests: ...
PASS  cargo xtask check/test: ...
NOT RUN real DB lane: not affected
NOT RUN real socket lane: connection behavior unchanged
NOT RUN load test: no performance claim
```

Never translate “not relevant/not run” into PASS.
