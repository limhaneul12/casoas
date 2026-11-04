# Axum Extractor and Validation Rules

## AXUM-EXTRACT-001 — Choose extractor by consumption boundary [MUST]

Use `FromRequestParts` for values derived without consuming the body. Use `FromRequest` only when the extractor requires ownership/consumption of the request body.

This distinction is an ownership contract, not merely an Axum API preference.

## AXUM-EXTRACT-002 — Body consumer is singular and last [MUST]

A handler has at most one body-consuming extraction path. The body-consuming extractor appears last after request-parts extractors.

Do not read the body in middleware/custom extraction and then expect a downstream `Json`, `Form`, `String`, `Bytes` or other body extractor to work unless the middleware intentionally reconstructs/replaces the body and owns that contract.

## AXUM-EXTRACT-003 — Transport validation and domain validation are distinct [MUST]

Transport validation owns parseability and protocol shape:

- required/malformed fields;
- JSON/form/query/path syntax;
- header/media-type constraints;
- bounded input size;
- transport DTO constraints.

Domain validation owns semantic invariants. Convert into a domain/application type through checked construction rather than letting transport DTOs become unchecked domain values.

## AXUM-EXTRACT-004 — Rejection mapping is deliberate [MUST]

Axum extractor rejections are external behavior. When a stable API contract matters, map rejections into the service's canonical error representation instead of leaking framework-default or internal parse/debug detail accidentally.

Preserve useful client diagnostics without exposing secrets, SQL, stack traces or internal topology.

## AXUM-EXTRACT-005 — Custom extractor threshold [SHOULD]

Create a custom extractor when a reusable **transport concern** has one clear owner, such as authenticated principal extraction, validated tenant context or a repeated header contract.

Do not create custom extractors to hide business workflows, database writes or arbitrary service-locator access.

## AXUM-EXTRACT-006 — `Option`/optional extraction semantics are explicit [MUST]

Using optional extraction means absence is valid under a defined protocol contract. Do not convert malformed/unauthorized data into `None` merely to simplify branching.

Where failure and absence differ semantically, preserve that distinction in the extractor/rejection contract.

## AXUM-EXTRACT-007 — Content type and representation are checked [MUST]

Body decoders such as JSON/form are used with the intended media type. Tests for externally exposed APIs include unsupported/missing media-type behavior when clients can trigger it.

Do not manually parse a typed format from raw bytes/string solely to bypass extractor behavior unless the custom protocol requires it.

## AXUM-EXTRACT-008 — State/request metadata ordering [MUST]

`State`, headers, path/query, extensions and other request-parts extraction occur before the body consumer. Request-derived context is kept request-scoped; application-global state uses `State`/`FromRef` under the state rules.

## AXUM-EXTRACT-009 — Validation dependency is capability-driven [SHOULD]

Do not mandate a validation crate globally. Use typed constructors, serde/schema constraints, `validator`, `garde` or another project-adopted mechanism only where it owns a real validation concern.

One invariant should not be independently reimplemented in multiple validation layers.

## AXUM-EXTRACT-010 — Bound expensive parsing [MUST]

Input that can cause large allocation, decompression, parsing, multipart handling or fan-out is bounded before or at the earliest reliable boundary. The body/resource-limit rules govern HTTP-wide limits; endpoint-specific lower limits are encouraged when the contract is narrower.

## Verification

For affected endpoints test, as applicable:

- valid extraction;
- missing versus malformed distinction;
- unsupported media type;
- invalid path/query/header values;
- body-limit rejection;
- canonical rejection/error body;
- semantic domain-invalid values after syntactic success.

Compile success does not prove rejection semantics.
