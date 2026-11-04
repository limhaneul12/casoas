# Axum OpenAPI Integration

Optional capability reference. Governing API Contract/OpenAPI Rules remain authority.

Research baseline at 2026-09-09:

- `utoipa` 5.5.x: OpenAPI 3.1-oriented derive/macro ecosystem; `utoipa-axum` can compose route metadata through an `OpenApiRouter`.
- `aide` 0.15.x: Axum documentation integration built around `schemars`/generated operation metadata.

Neither is a universal Axum requirement.

## 1. Decide source of truth before choosing a crate

Possible models:

```text
A. code/routes/DTO annotations → generated OpenAPI
B. canonical schema/contract → generated server/client adapters
C. contract-first file + explicit runtime reconciliation
```

The important rule is **one authority chain**. Do not maintain a manually edited spec and independent runtime routes with no reconciliation gate.

## 2. Choosing utoipa versus aide

Use this as a decision aid, not a mandate:

| Concern | utoipa / utoipa-axum | aide |
|---|---|---|
| style | derive/attribute-heavy | schemars + API operation integration |
| Axum router integration | `OpenApiRouter` ecosystem | `ApiRouter` ecosystem |
| schema source | `ToSchema`/derive-oriented | `JsonSchema`/schemars-oriented |
| macro tolerance | higher | still code-integrated, often less endpoint attribute-centric |
| existing ecosystem | good when utoipa already adopted | good when schemars is already central |

Select the library that fits the existing schema authority and dependency surface. Do not install both for the same contract unless migration/interoperability genuinely requires it.

## 3. Keep documentation at the transport boundary

A typical code-first structure:

```text
HTTP DTO + endpoint metadata
→ generated OpenAPI operation/schema
→ merged API document
```

Do not force domain types to derive OpenAPI/schema traits when the domain representation is not itself the external API contract.

A shared value type may derive schema metadata when it intentionally is part of the public representation and that dependency is accepted.

## 4. Runtime/schema agreement

Review mismatches caused by:

- `serde(rename)` / `rename_all`;
- `skip_serializing_if`;
- default values;
- `Option<T>` and nullable semantics;
- enum tagging/content representation;
- custom serializers/deserializers;
- validation rules not expressible in schema;
- extractor-specific content-type behavior;
- response headers/status/error envelopes.

Generated OpenAPI is **not runtime proof**.

## 5. Stable operation identity

If consumers use generated clients/gateways/policy, treat these as API:

```text
operationId
schema names
security scheme names
tags where automation depends on them
path/method/status/body shapes
```

A Rust function/module rename should not accidentally break remote consumers through regenerated `operationId` if the external contract is intended to remain stable.

## 6. Errors belong in the spec

Document meaningful client-visible failure families:

```text
validation / malformed request
authentication / authorization
not found
conflict / precondition
rate/overload/timeout when contractual
internal error envelope
```

Do not publish only `200`/`201` while clients actually depend on structured non-success responses.

## 7. Security declarations

OpenAPI security metadata must correspond to runtime enforcement:

```text
spec says Bearer <REDACTED>
↕
runtime auth middleware/extractor actually enforces it
```

A security scheme in documentation is not an access-control mechanism.

## 8. Public docs UI

Swagger UI / Scalar / Redoc exposure is separate from spec generation.

Possible policies:

```text
development only
internal/admin only
public authenticated
public unauthenticated by deliberate product decision
```

Do not assume disabling the docs UI secures an unauthenticated API.

## 9. Contract verification

When API compatibility matters:

```text
build/generate OpenAPI
→ parse/validate document
→ canonicalize unstable ordering if needed
→ diff against accepted baseline
→ review breaking changes
→ run representative Router tests
→ optional generated-client consumer test
```

A useful CI split:

```text
Rust compile/tests       prove source/runtime claims
OpenAPI generation/diff  proves documented contract claims
consumer tests           prove selected client compatibility
```

## 10. Breaking-change review examples

Potentially breaking:

- path or method removed/moved;
- request field becomes required;
- response type changes;
- enum case removed/renamed;
- nullable becomes non-nullable or reverse if generated clients care;
- error status/body changes;
- auth requirement added/removed;
- pagination/order guarantees change;
- generated operation/schema identifier changes.

Not every additive schema change is safe for every generated client, so use the actual consumer/toolchain evidence.

## 11. Minimality

Do not add OpenAPI because “production APIs should have Swagger.” Activate it when it serves a concrete need:

- public/partner API contract;
- generated client;
- gateway/policy integration;
- API review/testing;
- agent/tool schema pipeline;
- internal documentation with enough value to justify maintenance.

For a private narrow internal service, typed Rust tests and protocol docs may be sufficient.

## 12. Anti-patterns

- annotations scattered into domain types solely for docs generation;
- manual OpenAPI file and independent routes with no diff/reconciliation;
- generated spec treated as proof auth middleware is installed;
- only success response documented;
- accidental operationId changes from code refactors;
- two OpenAPI libraries producing competing schemas;
- Swagger UI publicly exposed by default without considering API sensitivity;
- API version prefix added mechanically with no compatibility policy.
