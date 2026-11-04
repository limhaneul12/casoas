# Axum Tower Middleware Rules

## AXUM-TOWER-001 — Tower is the middleware composition model [MUST]

Axum intentionally integrates with Tower instead of defining a bespoke middleware runtime. Reuse Tower/Tower-HTTP `Layer`/`Service` capabilities before inventing a parallel middleware framework.

## AXUM-TOWER-002 — Prefer `ServiceBuilder` for multi-layer stacks [SHOULD]

When composing multiple middleware, prefer `tower::ServiceBuilder` so request ordering is written and read top-to-bottom. Repeated `.layer(...)` calls may be valid, but their wrapping order is easier to misread.

Middleware order is behavioral semantics.

## AXUM-TOWER-003 — Ordering is tested when it affects correctness [MUST]

Add focused evidence when ordering changes security/reliability/observability, including cases such as:

- request ID creation before tracing;
- request ID propagation on responses;
- auth before protected handler work;
- body/resource limits before expensive processing;
- timeout relative to tracing/error mapping;
- compression relative to response/header mutation.

Do not rely on visual inspection alone for consequential ordering.

## AXUM-TOWER-004 — Use the narrowest middleware abstraction [SHOULD]

Selection ladder:

1. existing `tower-http`/Tower layer;
2. Tower combinator for a small transformation;
3. `axum::middleware::from_fn` for application-local async middleware;
4. `from_fn_with_state` when local middleware explicitly needs state;
5. custom `Layer`/`Service` when reusable, cross-framework, backpressure-sensitive or performance/control requirements justify it;
6. manual custom future only when evidence shows the additional complexity is required.

## AXUM-TOWER-005 — Custom middleware does not hide application workflows [MUST]

Middleware owns cross-cutting transport/service behavior. Do not place multi-step business workflows, persistent state transitions or arbitrary database writes into generic middleware.

## AXUM-TOWER-006 — Axum Router backpressure semantics are respected [MUST]

Axum routing assumes destination services are effectively always-ready and drives readiness inside the response future. Do not mount a service/middleware inside routing when its correctness depends on ordinary Tower `poll_ready` backpressure semantics without analyzing this mismatch.

For backpressure-sensitive services, use an appropriate outer service boundary and/or explicit load shedding/buffering/concurrency control. Keep all queues bounded.

## AXUM-TOWER-007 — Overload behavior is explicit [MUST]

If concurrency limits, buffers, rate limits or load shedding are introduced, specify:

- what is bounded;
- capacity;
- queueing versus fail-fast behavior;
- overload response/error mapping;
- fairness/tenant implications if relevant;
- metrics/observability.

Do not add an unbounded queue to make a load test look smoother.

## AXUM-TOWER-008 — Timeout layer choice includes error semantics [MUST]

Tower and tower-http provide different timeout/error behaviors. Choose the layer consistent with the application's response/error model and ensure timeout failures become a deliberate HTTP outcome rather than an unhandled service error.

Timeout does not guarantee cancellation of underlying blocking/non-cancel-safe side effects; Rust async/cancellation rules remain authority.

## AXUM-TOWER-009 — CORS is a policy, not boilerplate [MUST]

Configure CORS from the actual client/origin/credential requirement. Do not copy permissive `Any` settings into credentialed or sensitive APIs without an explicit trust model.

## AXUM-TOWER-010 — Middleware performance claims are measured [MUST]

Do not replace Tower composition with custom boxed/dynamic/manual machinery for speculative performance. Measure the real stack and workload first. If a custom layer is justified, benchmark and regression-test the relevant path.

## AXUM-TOWER-011 — Error channel matches Axum service expectations [MUST]

Custom middleware that can fail with non-infallible service errors must map those errors to responses using a compatible error-handling layer before reaching the Axum application boundary.

## Review checklist

- Does Tower/Tower-HTTP already own this capability?
- Is the stack order obvious in source?
- Does any layer depend on `poll_ready` backpressure?
- Are buffer/concurrency/rate limits bounded and observable?
- Does timeout actually stop the side effect, or only stop waiting?
- Is a custom `Layer` solving a reusable capability or just adding framework machinery?

## Evidence

Changed middleware behavior requires focused stack tests. Backpressure/overload/performance changes additionally require workload-appropriate stress/benchmark evidence before production claims are made.
