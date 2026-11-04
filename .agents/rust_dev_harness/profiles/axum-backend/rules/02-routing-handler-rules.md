# Axum Routing and Handler Rules

## AXUM-ROUTE-001 — Released-version routing syntax [MUST]

Use the syntax owned by the repository's Axum version. For Axum `0.8.x`, path captures use `/{id}` and wildcard captures use `/{*rest}`. Do not import stale `/:id` or `/*rest` examples from Axum 0.7-era material.

The shared profile does not silently migrate route syntax when the repository intentionally pins an older version.

## AXUM-ROUTE-002 — Thin handler responsibility [MUST]

A handler coordinates the HTTP boundary:

```text
extract
→ transport validation/conversion
→ application operation
→ map result to HTTP response
```

Do not place SQL, domain state transitions, multi-step business workflows, retry loops or persistent orchestration directly in handlers.

Thin means **narrow responsibility**, not an arbitrary line-count limit.

## AXUM-ROUTE-003 — Router composition follows ownership [SHOULD]

Compose routers around coherent feature/protocol ownership. Prefer local feature routers that are merged/nested at a composition root over one growing global route-registration function.

Do not create a router module per endpoint solely for symmetry. Split when ownership/change reasons justify it.

## AXUM-ROUTE-004 — Handler async contract [MUST]

Handler futures must remain `Send` as required by Axum. Do not hold non-`Send` guards/values across `.await`; fix ownership/lifetime semantics rather than adding broad `Arc`/`Mutex`/`Send` bounds to silence the compiler.

Blocking or CPU-heavy work follows Rust async rules and must not run directly on Tokio worker threads.

## AXUM-ROUTE-005 — Extractor order is intentional [MUST]

Request-parts extractors precede the body-consuming extractor. Only the final body-consuming position may own the request body. `State` and request-parts context appear before body extraction.

See the extractor rule set for `FromRequestParts` versus `FromRequest` ownership.

## AXUM-ROUTE-006 — Avoid handler closure opacity [SHOULD]

Small static handlers may be closures. Prefer named functions when behavior needs independent tests, meaningful tracing, substantial extraction, reusable application calls, or clear ownership.

Do not wrap complex logic in a closure merely to keep it physically near the router.

## AXUM-ROUTE-007 — Method and fallback behavior is contract [MUST]

Changes to method routing, fallback behavior, 404/405 handling, nested prefixes, trailing-slash normalization or service mounting are observable API changes and require focused verification.

Do not conflate unknown path (`404`) with known path/wrong method (`405`) if clients depend on the distinction.

## AXUM-ROUTE-008 — Route-scoped middleware uses route ownership [SHOULD]

Apply authentication, authorization, body limits or special timeouts at the narrowest router/route scope that represents the actual policy. Do not globally apply a restrictive rule and then accumulate bypasses unless the policy is genuinely global.

## AXUM-ROUTE-009 — Diagnostic macros are diagnostic [SHOULD]

`#[axum::debug_handler]` / `axum-macros` may be used to diagnose difficult `Handler` type errors. Do not rely on a diagnostic macro as a substitute for understanding ownership, extractor ordering or `Send` failures. Preserve it permanently only when repository policy considers the compile-time diagnostics worth the macro/dependency cost.

## AXUM-ROUTE-010 — `Router<()>` is serving/test boundary [MUST]

Before serving or directly using the router as a Tower service, all required state must be supplied. App/router factories intended for direct `oneshot` tests should produce a fully composed service with required state installed.

## Review checklist

- Is the handler only adapting HTTP to an application operation?
- Is Axum 0.8 route syntax correct for the active repository version?
- Is any non-`Send` guard held across `.await`?
- Is middleware attached at the correct route scope?
- Are 404/405/fallback changes covered?
- Does the app factory expose a testable fully composed router/service?

## Evidence

Use router-level `oneshot`/`ready + call` tests for routing behavior and real-socket tests only where actual listener/connection behavior is under test.
