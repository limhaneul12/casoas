# Axum Error and Response Rules

## AXUM-ERR-001 — Domain/application errors stay transport-neutral [MUST]

Domain/application owners return typed semantic errors/outcomes. Do not encode HTTP status codes, headers or Axum response types into a domain error solely for handler convenience.

The HTTP adapter owns semantic-error → HTTP-response mapping.

## AXUM-ERR-002 — HTTP errors implement one deliberate response contract [MUST]

Use a coherent transport error type or narrow response mapping that implements `IntoResponse` and yields the service's canonical error representation.

A single giant `AppError` enum is not mandatory; one canonical mapping policy is. Split by feature when ownership warrants it, but prevent inconsistent duplicate mappings for the same semantic failure.

## AXUM-ERR-003 — Internal detail is not client detail [MUST]

Never expose raw `anyhow`, SQLx, parser internals, filesystem paths, stack traces, secret values or debug-formatted internal errors in public 5xx responses.

Log internal cause/evidence with safe correlation metadata; return a stable client-facing classification/message.

## AXUM-ERR-004 — Preserve semantic distinctions [MUST]

Map distinct outcomes intentionally, for example:

- malformed transport input;
- unauthenticated;
- authenticated but forbidden;
- domain not found;
- conflict/state precondition;
- resource/timeout overload;
- internal failure.

Do not collapse all failures to `500` or all client failures to `400` merely to simplify code.

Exact status choices remain API-contract decisions.

## AXUM-ERR-005 — Extractor rejection policy is explicit [MUST]

If the service defines a stable error envelope, Axum extractor rejections that escape to clients must be mapped/normalized consistently. Default framework rejections may be retained only when they are intentionally accepted as the public contract.

## AXUM-ERR-006 — Middleware errors become responses before Axum boundary [MUST]

Axum's routing model expects infallible application service errors. A custom Tower middleware/service that produces a fallible error must be converted through an appropriate `HandleErrorLayer` or equivalent response mapping before it reaches the Axum service boundary.

Prefer middleware that returns a response rather than inventing a new opaque error channel when the failure is naturally HTTP-visible.

## AXUM-ERR-007 — Retryability is not inferred from status alone [MUST]

If clients/workers need retry semantics, define them explicitly through contract/documentation/headers or typed internal outcomes. Do not assume every `5xx` is safe to retry or every `4xx` is permanent.

Idempotency and side-effect completion must be considered before automatic retries.

## AXUM-ERR-008 — Serialization failure has a defined policy [MUST]

Response serialization failure is an internal server failure, not a client validation error. Ensure observability captures the cause without leaking the failed object or secret-bearing content.

For critical public APIs, test representative response serialization and schema compatibility.

## AXUM-ERR-009 — Panic is not routine error handling [MUST]

Do not use panic/catch-panic as ordinary request error control flow. Rust baseline panic rules remain authority. A panic-catching HTTP layer may be used as a process-boundary resilience measure, but it does not make panic acceptable inside normal request logic and requires operational observability.

## AXUM-ERR-010 — Request path `unwrap`/`expect` follows Rust invariant rules [MUST]

Do not introduce `unwrap`/`expect` for external or runtime-fallible request data. A repository may use them only where the Rust baseline permits a locally proven invariant; the proof must not depend on untrusted request correctness.

## Error response review

- Is the underlying semantic error transport-neutral?
- Is client-visible detail safe and stable?
- Are auth/not-found/conflict/overload/internal failures distinguished intentionally?
- Can middleware errors escape Axum's expected response model?
- Could an automatic retry duplicate a side effect?
- Does the response carry a correlation/request ID where operationally useful?

## Evidence

Test status + canonical body shape for changed externally visible failures. Security-sensitive error changes include assertions that protected/internal material is absent.
