# Axum SQLx Integration

Optional capability reference. Axum itself does not require SQLx. Governing Rust/Axum rules and repository persistence policy remain authority.

Research baseline: SQLx `0.9.0` at 2026-09-09.

## 1. Boundary

Preferred flow:

```text
Axum handler
→ application operation
→ repository/persistence owner
→ SQLx query/transaction
→ typed result
```

Do not write SQL directly in handlers when persistence has an independent owner. Avoid hiding SQL writes in extractors/middleware.

## 2. Pool in state

`PgPool`/other SQLx pools are designed as cloneable concurrent handles:

```rust
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}
```

Do not wrap a pool in an outer `Mutex`/`RwLock`. Pool max connections already represent a bounded downstream resource and should be sized against DB capacity/workload, not request count alone.

## 3. Compile-time checked queries

When practical, prefer SQLx checked macros (`query!`, `query_as!`, etc.) so SQL/result metadata is verified against the intended schema.

Example shape:

```rust
let row = sqlx::query_as!(
    UserRow,
    r#"
    SELECT id, email
    FROM users
    WHERE id = $1
    "#,
    id
)
.fetch_optional(pool)
.await?;
```

Repository style may prefer dynamic query builders for legitimate dynamic SQL. Do not force macros when the query shape genuinely cannot be static; compensate with focused DB tests.

## 4. Offline metadata

For CI/build environments without live DB access, SQLx supports prepared offline metadata under `.sqlx`.

Typical development loop:

```text
schema/migration/query changed
→ run against canonical dev/test DB
→ cargo sqlx prepare
→ commit updated .sqlx metadata if repository adopts offline mode
→ cargo sqlx prepare --check in CI
```

Treat `.sqlx` as generated verification input owned by the query/schema contract, not hand-edited source.

## 5. Migrations

Migrations are schema authority/evolution artifacts, not handler setup code.

Decide repository policy explicitly:

- external deployment migration step;
- application startup migration;
- test fixture migration;
- embedded `migrate!` approach.

If using `sqlx::migrate!`, account for build-system change detection requirements so new migration files are rebuilt/embedded correctly. Follow the repository's existing xtask/build policy rather than adding ad hoc startup migration behavior.

## 6. Transaction ownership

Transactions should span the semantic atomic operation, not an arbitrary HTTP handler lifetime.

Prefer application/persistence ownership such as:

```text
application operation
→ begin transaction
→ repository actions participating in same invariant
→ commit/rollback
```

Do not begin a transaction in generic middleware for every request unless the whole application architecture intentionally uses request-scoped transactions and has defined streaming/error semantics.

## 7. Error translation

SQLx errors are infrastructure detail:

```text
sqlx::Error
→ persistence/application semantic error
→ HTTP mapping
```

Map known constraint/conflict/not-found semantics explicitly when the database provides authoritative evidence. Avoid exposing raw database messages, SQL or connection details to clients.

Be careful with unique-constraint mapping: identify the actual constraint/semantic owner rather than converting every DB constraint error to the same `409`.

## 8. Cancellation and transactions

An HTTP timeout/cancel does not automatically prove an external transaction or DB-side operation did not execute. Use SQLx/driver/database transaction semantics and application idempotency where request cancellation can race with commit.

For long queries, configure DB/query timeouts at the correct layer if the application requires a stronger bound than caller waiting time.

## 9. N+1 and fan-out

HTTP concurrency can multiply query cost:

```text
requests × application fan-out × queries per item
```

Review list endpoints for N+1 queries and unbounded pagination. Prefer set-based/batched queries when they preserve semantics and measurement justifies them.

Do not optimize by loading unbounded result sets into memory.

## 10. Pagination

Every externally controlled page/window limit should be bounded. Prefer stable ordering and cursor/keyset pagination where large offsets become operationally expensive and the product semantics fit.

Pagination strategy is API/database policy, not an Axum default.

## 11. Testing

Use multiple evidence layers:

```text
application tests                → semantic behavior
router oneshot tests             → HTTP mapping
real DB integration tests        → SQL/schema/constraint behavior
cargo sqlx prepare --check       → offline metadata freshness when active
```

Do not mock the database and then claim a migration/SQL compatibility issue is verified.

For concurrency/transaction races, use real DB tests with deterministic coordination where possible.

## 12. Startup/readiness

If DB is required before serving traffic, fail startup or readiness according to the deployment contract. Avoid a liveness endpoint that synchronously queries the database on every probe.

Pool creation may be lazy depending on SQLx options; distinguish "pool handle constructed" from "required DB connectivity verified" in startup/readiness claims.

## 13. Performance review

Measure:

- query latency/distribution;
- pool acquisition wait;
- rows scanned/returned;
- query count per request;
- transaction duration;
- lock contention/deadlocks;
- DB CPU/I/O;
- serialization separately.

Framework request throughput is rarely meaningful if DB wait dominates.

## 14. SQLx anti-patterns

- SQL strings and transaction orchestration directly in handlers;
- `Arc<Mutex<PgPool>>`;
- increasing pool size without DB capacity evidence;
- committed query macro changes without refreshing adopted offline metadata;
- mocking SQLx for a claim about real schema compatibility;
- startup migrations silently racing between many replicas without a deployment policy;
- one DB query per list item;
- unbounded `fetch_all` driven by user-controlled filters;
- converting every SQLx error to a client-visible raw string.
