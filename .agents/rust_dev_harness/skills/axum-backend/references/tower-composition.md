# Axum Tower Composition

Non-normative composition reference. Governing Tower Middleware Rules remain authority.

## 1. Why Tower first

Axum deliberately uses Tower's `Service`/`Layer` ecosystem instead of defining an independent middleware runtime. This gives one composition model across Axum and other Tower-compatible services.

Selection preference:

```text
tower-http / Tower capability
→ Tower combinator
→ axum::middleware::from_fn
→ from_fn_with_state
→ custom Layer/Service
→ manual future/runtime only when justified
```

## 2. Read middleware order as behavior

With `ServiceBuilder`, request flow follows the order layers are added; responses unwind in reverse.

Conceptual example:

```rust
let middleware = ServiceBuilder::new()
    .layer(SetRequestIdLayer::new(...))
    .layer(TraceLayer::new_for_http())
    .layer(PropagateRequestIdLayer::new(...));
```

Request direction:

```text
Set Request ID
→ Trace
→ Propagate wrapper
→ handler
```

Response direction unwinds back outward.

The exact best stack is workload/policy dependent. Do not copy one universal ordering list without reasoning about what each layer must observe or reject.

## 3. Request ID + tracing

If tracing spans must include the request ID, establish the ID before `TraceLayer`. If the response must carry it, include the response propagation layer in the composition with the intended unwind behavior.

Sensitive headers should be marked/redacted before header-aware tracing sees them.

Behavioral test idea:

```text
request without ID
→ middleware assigns ID
→ handler/tracing context observes same ID
→ response returns same ID
```

## 4. Resource limits before expensive work

Place body/request/concurrency/rate controls early enough to prevent expensive parsing/fan-out from happening first.

But distinguish resources:

```text
body size limit       protects bytes/allocation/parsing
concurrency limit     protects simultaneous service work
rate limit            protects request arrival rate
buffer                 trades immediate rejection for bounded queueing
load shed              fails fast when inner service is not ready
DB pool                bounds downstream DB connections
```

One layer does not replace the others.

## 5. Error handling

Axum application routing expects response-producing/infallible service behavior. A Tower layer/service returning a fallible `Error` requires conversion before the Axum boundary.

Typical shape:

```text
fallible timeout/custom service
→ HandleErrorLayer / response mapping
→ Axum Router
```

Do not use `HandleErrorLayer` reflexively when a `tower-http` layer already returns an HTTP response directly; choose according to actual error semantics.

## 6. Timeout choices

A timeout can mean different things depending on layer implementation:

- Tower timeout may produce a service error that needs `HandleErrorLayer`.
- tower-http timeout can produce an HTTP response directly.

The choice must match the application's canonical error contract.

Remember:

```text
HTTP future timed out
≠
all underlying work definitely stopped
```

Blocking tasks and already-committed external side effects require separate cancellation/idempotency design.

## 7. Axum Router and backpressure

This is a critical Axum-specific nuance.

Axum routing assumes routed destination services are effectively always ready and invokes readiness/call in a way that does not preserve ordinary Tower backpressure semantics for arbitrary inner services.

Therefore a service whose correctness relies on `poll_ready` must not simply be nested/routed with the assumption that Axum will propagate its backpressure.

Options depend on the design:

```text
explicit load shedding
bounded buffer + understood semantics
concurrency limit
outer middleware around the entire app/service
separate service boundary
```

Never introduce an unbounded buffer as a backpressure fix.

## 8. Concurrency limit + buffer ordering

Tower's own docs highlight that order changes semantics. Conceptually:

```text
Buffer → ConcurrencyLimit
```

can allow queued requests beyond the concurrency limit, while:

```text
ConcurrencyLimit → Buffer
```

can count queued work differently/limit total admitted work earlier.

Do not decide by aesthetic preference. Specify the desired invariant:

- maximum active handlers?
- maximum admitted requests including queue?
- fail-fast or wait?
- queue memory bound?
- fairness?

Then test saturation behavior.

## 9. `from_fn` versus custom Layer

Use `from_fn` when:

- middleware is application-local;
- async code is natural;
- no special `poll_ready` behavior is needed;
- reuse outside Axum/Tower is not important.

Use a custom `Layer`/`Service` when:

- it is a reusable library capability;
- readiness/backpressure matters;
- custom service error/future/control is needed;
- composition outside a single Axum application is desired.

Avoid custom manual futures solely to eliminate an allocation until profiling proves that path matters.

## 10. Example review matrix

| Concern | Likely mechanism | Key trap |
|---|---|---|
| tracing | `TraceLayer` | secrets/cardinality/order |
| request ID | request-id layers | must exist before trace if span needs it |
| body size | `RequestBodyLimitLayer` / Axum default | raw Body may bypass Axum default |
| timeout | Tower/tower-http timeout | error mapping + cancellation semantics |
| CORS | `CorsLayer` | permissive credential/browser policy |
| compression | `CompressionLayer` | CPU + sensitive-response policy |
| auth | route middleware/custom extractor | authentication != business authorization |
| concurrency | Tower concurrency limit | queue/order/backpressure semantics |
| load shedding | Tower load shed | map overload errors intentionally |

## 11. Verification targets

For a stack change test the actual observable behavior:

```text
correct request ID propagation
protected route rejects before handler effect
body too large rejects before expensive handler work
timeout maps to expected status/error envelope
overload rejects/queues according to declared policy
sensitive headers absent from trace output
```

For performance changes benchmark the real middleware stack, not an isolated synthetic closure.
