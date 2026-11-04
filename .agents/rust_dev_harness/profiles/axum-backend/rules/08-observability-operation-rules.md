# Axum Observability and Operation Rules

## AXUM-OBS-001 — Structured tracing is the request-path baseline [MUST]

Use `tracing`/`tracing-subscriber` compatible structured telemetry for production request-path observability. Do not use ad-hoc `println!`/`dbg!` as the operational signal for normal server behavior.

## AXUM-OBS-002 — Request correlation exists where operations need it [SHOULD]

For multi-service, asynchronous, support-sensitive or externally exposed systems, establish a request/correlation ID and propagate it through the response and downstream context where practical.

When using tower-http request IDs with `TraceLayer`, install/set the request ID before tracing if spans must observe it, and propagate the response ID in the correct middleware order.

Do not trust arbitrary externally supplied correlation IDs for authorization or identity.

## AXUM-OBS-003 — Low-cardinality request fields [MUST]

Prefer stable fields such as:

- HTTP method;
- matched route template;
- status class/status;
- latency;
- request ID/trace ID;
- bounded error category;
- service/version/instance metadata where applicable.

Avoid using raw user IDs, arbitrary path values, query strings or request bodies as metric labels. High-cardinality identifiers may exist in traces/log fields when needed but require retention/privacy consideration.

## AXUM-OBS-004 — Secrets and bodies are opt-in observability [MUST]

Do not log authorization/cookie/API-key headers or full request/response bodies by default. Mark sensitive headers before header-aware trace instrumentation or redact them explicitly.

Payload logging requires an explicit operational need, bounded size and redaction policy.

## AXUM-OBS-005 — Error observability separates client and internal detail [MUST]

Client-facing errors are sanitized; internal cause chains are recorded only in trusted telemetry with request/trace correlation. Do not make client error bodies the only source of failure diagnosis.

## AXUM-OBS-006 — Readiness and liveness have different semantics [SHOULD]

When deployment health probes are required:

- liveness answers whether the process/runtime is alive enough to continue;
- readiness answers whether the instance should receive traffic under the project's dependency policy.

Do not make every liveness request synchronously fan out to all dependencies. Do not declare readiness based only on process existence when a required startup dependency is unavailable.

Concrete probe semantics are deployment/application policy.

## AXUM-OBS-007 — Graceful shutdown is owned and observable [MUST]

Production servers define how they stop accepting traffic and how owned background work/resources terminate. Use Axum/Tokio graceful-shutdown mechanisms and Rust async task ownership rules.

Shutdown-critical tasks should receive cancellation and be awaited/tracked where correctness requires completion. Emit bounded lifecycle telemetry for start, drain, timeout and completion/failure.

## AXUM-OBS-008 — Background tasks do not become detached side effects [MUST]

Do not `tokio::spawn` request-triggered or service-lifetime work without an owner, cancellation policy, error observation and shutdown semantics. Fire-and-forget is acceptable only when loss/error semantics are explicitly acceptable.

## AXUM-OBS-009 — Saturation is observable [MUST]

If the application introduces concurrency limits, queueing, pool limits, retries, rate limits or load shedding, expose enough telemetry to diagnose saturation: rejected/queued work, wait time, in-flight count, timeout/error classes or equivalent signals.

A silent queue is not an operational control.

## AXUM-OBS-010 — Performance telemetry reflects the actual bottleneck [MUST]

Do not infer server performance from framework microbenchmarks alone. Distinguish route/middleware overhead from database, serialization, downstream I/O, lock contention, pool wait, executor delay and network behavior.

## AXUM-OBS-011 — OpenTelemetry/export backends are optional [MUST]

This profile mandates structured observability semantics, not a particular telemetry exporter/vendor. Adopt OpenTelemetry, tracing exporters or proprietary backends only when the repository/deployment requires them.

## Verification

Where behavior changes, verify:

- request IDs appear/propagate as intended;
- sensitive headers are not emitted;
- status/error classification is correct;
- shutdown drains or times out according to policy;
- overload controls produce observable signals;
- health endpoints reflect their declared semantics.

Operational screenshots/log snippets alone do not replace executable checks when the behavior can be tested.
