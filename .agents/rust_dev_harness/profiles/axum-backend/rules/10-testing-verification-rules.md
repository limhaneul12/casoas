# Axum Testing and Verification Rules

Specialization of the Shared Testing and Verification Contract and Rust testing rules for Axum HTTP adapters.

## AXUM-TEST-001 — Test at the narrowest truthful boundary [MUST]

Use:

- domain/application tests for semantic decisions;
- router/service tests for HTTP extraction/routing/middleware/response contracts;
- real socket/server tests only for listener, connection metadata, actual network protocol or transport integration properties;
- external integration tests for real databases/services when those effects are part of the claim.

Do not start a TCP server for every handler test merely to look realistic.

## AXUM-TEST-002 — Router-as-Tower-service is the default HTTP integration seam [SHOULD]

Axum `Router` implements Tower service behavior after required state is supplied. Prefer `tower::ServiceExt::oneshot` for isolated request/response tests and `ready + call` when multiple calls or explicit readiness semantics matter.

Build the application through the same composition function used by production, with test dependencies/config as needed.

## AXUM-TEST-003 — Test externally observable failure contracts [MUST]

For changed endpoint behavior test relevant negative paths, including:

- 404 and/or 405;
- malformed path/query/header/body;
- unsupported content type;
- body/resource limit;
- unauthenticated/forbidden;
- not-found/conflict/precondition;
- timeout/overload;
- canonical 5xx sanitization.

Do not test only the happy path when the change touches validation/security/error mapping.

## AXUM-TEST-004 — Middleware ordering needs behavioral tests [MUST]

When ordering matters, assert the resulting behavior rather than snapshotting source order. Examples: request ID visible to tracing/context, auth short-circuits protected work, limit rejects before expensive work, timeout/error mapping yields the intended status.

## AXUM-TEST-005 — Inject dependencies, do not mock Axum [SHOULD]

Test the real router/extractor/middleware stack. Substitute application/effect dependencies only where the architecture owns a legitimate seam. Do not mock Axum's `Router`, extractor protocol or `IntoResponse` machinery when direct service execution is cheap and deterministic.

## AXUM-TEST-006 — Connection metadata has explicit test support [SHOULD]

For handlers using `ConnectInfo`, prefer Axum's `MockConnectInfo` for transport-adapter tests. Use a real listener when the actual connection address/proxy/TLS behavior itself is the subject under test.

## AXUM-TEST-007 — SQLx capability gets database-truth evidence [MUST when active]

When SQLx query/migration behavior changes, HTTP mocks are insufficient to prove database compatibility. Run the repository's SQLx compile-time metadata check and/or real database integration lane according to project policy.

## AXUM-TEST-008 — OpenAPI capability gets contract evidence [MUST when active]

Generate/validate/diff the OpenAPI document when externally consumed contracts change. Router tests and schema generation test different claims; run both when both matter.

## AXUM-TEST-009 — RMCP capability gets protocol integration evidence [MUST when active]

When RMCP is mounted through Axum/Tower, test tool schema/dispatch and the negotiated Streamable HTTP behavior relevant to the project. Do not infer MCP correctness from the REST endpoint being healthy.

## AXUM-TEST-010 — Async shutdown/cancellation claims are executed [MUST]

If a change claims graceful shutdown, cancellation, timeout or bounded overload behavior, run a test/stress/fault scenario that exercises it. A cancellation token existing in source is not proof that every owned task terminates.

## AXUM-TEST-011 — Performance claims require comparable workloads [MUST]

Benchmark the actual composition and workload under comparison. Include middleware/database/serialization when they dominate the production path. Record methodology, relevant percentiles/resources and uncertainty; do not use unrelated framework hello-world benchmarks as product evidence.

## Canonical verification ladder

```text
focused unit/application regression
→ focused Router/Tower service test
→ cargo xtask rules
→ cargo xtask fmt
→ cargo xtask check
→ reviewed Clippy policy
→ cargo xtask test
→ capability-specific integration/contract gates
→ cargo xtask ci
→ stress/load/fault/perf only when the claim requires it
```

## Completion report

Report:

- changed endpoint/router/middleware contract;
- active optional capabilities;
- exact focused tests run;
- exact canonical gates run;
- any DB/OpenAPI/RMCP/network/concurrency lanes not run;
- performance evidence if performance is claimed.

No unexecuted lane is reported as PASS.
