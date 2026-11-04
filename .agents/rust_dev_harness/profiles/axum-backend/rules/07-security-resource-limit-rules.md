# Axum Security and Resource Limit Rules

Axum-specific security specialization. General trust, secret, dependency and unsafe rules remain owned by the Shared/Rust Harness.

## AXUM-SEC-001 — Every untrusted request path has bounded resource exposure [MUST]

Define relevant limits for externally controlled work, including as applicable:

- request/body size;
- multipart/file size and count;
- decompressed/decoded size;
- query pagination/window size;
- concurrent in-flight work;
- queue depth;
- execution/deadline timeout;
- downstream connection pool limits;
- fan-out/batch cardinality.

A limit must be attached to the resource it actually protects.

## AXUM-SEC-002 — Understand Axum body-limit scope [MUST]

Axum `0.8.x` applies a default 2 MiB limit to `Bytes`-based extractors including `String`, `Json` and `Form`. This does **not** prove every request body is bounded: custom/raw `Body::poll_frame` consumption can bypass that default.

For untrusted raw/streaming/custom-body paths, add an explicit limit appropriate to the protocol, such as `tower_http::limit` or a deliberately implemented streaming bound.

Do not disable `DefaultBodyLimit` without installing an equivalent or stronger contract where external input is accepted.

## AXUM-SEC-003 — Authentication and authorization are separate [MUST]

Authentication establishes principal/credential context. Authorization decides whether that principal may perform the operation/resource transition.

Do not treat successful token parsing as authorization. Do not bury business authorization in a generic transport decoder when it requires domain/application state.

## AXUM-SEC-004 — Sensitive headers/body data are not observable by default [MUST]

Authorization tokens, cookies, session IDs, API keys and secret-bearing headers/fields must not appear in routine request/response logs or error bodies.

When tracing headers, mark sensitive headers before the trace layer or otherwise ensure redaction. Avoid logging full request/response bodies by default.

## AXUM-SEC-005 — CORS follows an explicit browser trust model [MUST]

CORS configuration is conditional on browser clients. Allow origins, methods, headers and credentials according to the actual frontend trust model.

Do not combine broad wildcard origins with credentialed/sensitive browser APIs without explicit security review.

CORS is not server-to-server authentication or authorization.

## AXUM-SEC-006 — CSRF is capability-dependent [MUST]

CSRF controls are required when browser ambient credentials such as cookies can authorize state-changing requests. Bearer-token/server-to-server APIs are not forced into cookie-oriented CSRF patterns merely because they use HTTP.

Project auth design owns the concrete mechanism.

## AXUM-SEC-007 — Timeout is not complete cancellation [MUST]

HTTP timeout/deadline limits how long the caller waits or the service processes an async future, but does not automatically cancel already-started blocking work or externally committed side effects.

Use cancellation-aware operations and idempotency/transaction semantics where correctness depends on stopping or retrying work.

## AXUM-SEC-008 — Blocking work is bounded [MUST]

`spawn_blocking` is for bounded blocking work that finishes. Tokio cannot reliably abort a blocking task once it has started. CPU-heavy blocking work requires an explicit concurrency bound or dedicated CPU executor strategy under Rust async rules.

Do not let one request spawn unbounded blocking jobs.

## AXUM-SEC-009 — Proxy/client IP trust is explicit [MUST]

Do not trust forwarded client-IP/scheme/host headers unless the deployment defines which proxy/ingress is trusted and normalizes them. Direct `ConnectInfo` and proxy-forwarded identity are different trust sources.

Rate limits/audit/security decisions must name which identity source they trust.

## AXUM-SEC-010 — Error responses are sanitized [MUST]

Public 5xx/authorization failures do not leak token material, database details, filesystem paths, stack traces or internal service topology. Correlate with request/trace IDs instead.

## AXUM-SEC-011 — Rate/overload control is threat- and workload-driven [SHOULD]

Use rate limiting, concurrency limiting and load shedding where abuse or overload risk justifies them. Select identity keys and limits from the service threat/workload model rather than copying one global requests-per-second number.

## AXUM-SEC-012 — Compression is deliberate on sensitive responses [SHOULD]

Do not enable/disable compression mechanically. Consider bandwidth, CPU and secret-reflection/compression side-channel risk for sensitive browser-facing responses. Keep the choice in deployment/application policy and measure its effect.

## Security review checklist

- Can raw/custom bodies exceed the default Axum body limit?
- What bounds total work after a small request triggers expensive fan-out?
- Are token/cookie headers redacted before tracing?
- Are browser credentials in use, and therefore are CORS/CSRF semantics correct?
- Are forwarded headers trusted only behind known proxies?
- Can timed-out work continue committing side effects?
- Is overload behavior bounded and observable?

## Evidence

Security/resource changes require negative tests for rejection/limits and evidence that secrets/internal errors are absent. Load-shedding/rate/concurrency claims need stress or targeted concurrency tests appropriate to the changed mechanism.
