# Axum Anti-Patterns

Adversarial review reference. This catalog is not a second Rule source; governing Shared/Rust/Axum Rules remain authority.

## 1. Stale Axum route syntax

```rust
.route("/users/:id", get(get_user))
```

in an Axum 0.8.x project.

Use repository-version semantics; Axum 0.8.x uses `/{id}`. This exact stale pattern was found in a public Skill that claimed Axum 0.8.7 support, demonstrating why community templates must be verified against official release docs.

## 2. Handler as application layer

```rust
async fn create_user(...) {
    // parse request
    // open DB transaction
    // check business invariants
    // call three external services
    // retry
    // write audit log
    // map HTTP
}
```

The problem is not lines. The handler now owns unrelated semantic/effect responsibilities and cannot be reused by another transport.

## 3. SQL directly in handlers

```rust
async fn get_user(State(db): State<PgPool>, ...) {
    sqlx::query!(...).fetch_one(&db).await
}
```

For a trivial data-only service this may be an accepted repository architecture, but once persistence/business semantics have independent owners, direct handler SQL creates coupling and duplicated transaction/error policy.

## 4. Domain importing Axum

```rust
pub enum DomainError {
    NotFound(StatusCode),
}
```

when the status exists only for HTTP convenience.

Keep domain/application errors semantic and map them at the adapter.

## 5. Mega-state service locator

```rust
State<AppState>
```

where every handler can reach every pool/client/service/config and dynamically choose unrelated dependencies.

A large state object can be valid at composition root; the smell is unbounded capability access and hidden ownership.

## 6. Blanket `Arc`

```rust
Arc<PgPool>
Arc<reqwest::Client>
Arc<UserService>
Arc<Config>
```

added because “Axum state must be cloneable.”

Many handles already clone cheaply. Use `Arc` only for actual shared allocation/lifetime needs.

## 7. Mutex around a concurrency-safe pool/client

```rust
Arc<Mutex<PgPool>>
```

This serializes access around a handle whose purpose is concurrent pooling and can create avoidable contention.

## 8. Business logic in custom extractor

```text
extract auth header
→ query orders
→ mutate account
→ return CurrentUser
```

Custom extractors should own reusable request transport/context extraction, not invisible business workflows.

## 9. Body consumed twice

Middleware buffers/consumes the request body, then handler expects `Json<T>` without intentionally reconstructing the body.

Body ownership is singular. If middleware must inspect the body, define replacement/bounds/stream semantics explicitly.

## 10. Raw Body with assumed default limit

Using raw/custom `Body` consumption while assuming Axum's default 2 MiB extractor limit protects it.

Axum's default body limit applies to common Bytes-based extractors, not arbitrary raw streaming consumption. Add explicit bounds.

## 11. Custom middleware runtime beside Tower

Creating a framework-specific chain of boxed middleware traits/futures while Tower already provides the composition contract, without measured or semantic need.

This adds a second service model, interoperability burden and performance/error semantics to maintain.

## 12. Middleware order by guess

A stack is rearranged until tests pass or copied from a blog without specifying which layer must see/reject what.

Request ID/tracing, limits, auth, timeout/error mapping and compression can all change behavior with order. Define invariant, then test it.

## 13. Backpressure assumed through routed service

Nesting a service whose correctness relies on `poll_ready` into an Axum Router and assuming the router propagates ordinary Tower readiness/backpressure.

Axum's routing model has special readiness semantics; design explicit load shedding/buffering/concurrency or an outer service boundary.

## 14. Unbounded buffer as overload fix

```text
traffic spike
→ unbounded queue
→ temporarily fewer 503s
→ memory/latency collapse later
```

Every queue/in-flight/fan-out resource requires a bound and an intentional overload outcome.

## 15. Timeout equals cancellation

Assuming a timeout response means DB/external/blocking work was rolled back/stopped.

Started blocking work may be non-abortable; external side effects may already commit. Use cancellation-aware operations, transaction/idempotency semantics and dedicated resource limits.

## 16. `spawn_blocking` per request without bound

CPU-heavy or blocking requests spawn unlimited blocking tasks. Tokio's blocking pool can be large and started tasks cannot simply be aborted. Add an explicit concurrency strategy or CPU executor.

## 17. Authentication equals authorization

A successfully authenticated credential does not automatically grant every domain operation.

Credential validity identifies a principal; resource/action authorization is a separate policy and may require application/domain state.

## 18. Permissive CORS copied as boilerplate

`Any` origins/headers/methods are enabled for a credentialed browser API because it worked in development.

CORS must reflect the actual browser trust model and is not server authentication.

## 19. Sensitive headers/body in tracing

Logging full request headers or JSON bodies in default `TraceLayer` callbacks without marking authorization/cookies/API keys sensitive and without payload redaction/size policy.

## 20. Every endpoint gets a trait/service/module/crate

Mechanical clean-architecture expansion:

```text
CreateUserHandler
CreateUserService trait
CreateUserServiceImpl
CreateUserRepository trait
CreateUserRepositoryImpl
```

with one implementation and no independent boundary.

Use the fewest new authority surfaces that preserve real responsibilities.

## 21. OpenAPI as runtime proof

Generated spec says auth/field/response exists, therefore runtime is assumed correct.

Schema generation and Router behavior prove different claims. Test representative runtime agreement.

## 22. Hand-written OpenAPI + routes as dual authority

Spec and server are both edited independently with no generation/diff/consumer gate. Drift becomes inevitable.

## 23. Every REST route becomes an MCP tool

Generic automatic projection exposes internal endpoints/semantics to agents without tool design, authorization, idempotency or output-schema review.

Expose application operations intentionally.

## 24. REST loopback as MCP business reuse

```text
MCP tool
→ HTTP call to localhost REST endpoint
→ application
```

when both adapters live in the same process and can call the same application operation. This adds serialization/network/HTTP coupling and makes REST semantics the accidental domain contract.

## 25. Custom MCP protocol server despite official RMCP fit

Implementing initialize/version negotiation/Streamable HTTP/tool dispatch manually with a hardcoded protocol version when the official SDK satisfies the requirement.

This creates protocol drift risk for little architectural gain.

## 26. SQLx mock proves SQL correctness

A fake repository passes, so a changed `query!`/migration/constraint is declared verified. Run database-truth evidence for database claims.

## 27. Bigger DB pool as performance fix

Pool max connections is increased because requests wait. If the DB is already saturated, this increases contention and worsens tail latency. Measure pool wait and DB capacity.

## 28. Framework microbenchmark proves service performance

A hello-world Axum/RustAPI/other benchmark is cited as proof a database-heavy production endpoint got faster.

Measure the actual route/middleware/serialization/DB/downstream workload and relevant percentiles/resources.

## 29. Health check overload

Liveness performs multiple synchronous dependency calls on every probe, causing an outage to amplify probe load and restart churn.

Separate process liveness from traffic readiness according to deployment semantics.

## 30. Fire-and-forget request work

`tokio::spawn` sends an email/writes audit state/commits workflow without task ownership, retry/durability/error observation or shutdown policy.

Use a durable job/event mechanism or an explicitly owned task when delivery semantics matter.

## 31. Source scanner tries to prove semantic architecture

A verifier rejects any handler over N lines or any `State<AppState>` use as proof of bad architecture.

Mechanical verifiers should prove only reliable syntactic/config/evidence facts. Semantic ownership remains human/agent review backed by tests and architecture authority.

## 32. Completion by source appearance

“Looks bounded”, “has timeout middleware”, “has graceful shutdown code” is reported as PASS without executing the relevant tests/gates.

No unexecuted verification lane is PASS.
