# Axum API Contract and OpenAPI Rules

OpenAPI is an optional capability. These rules activate when the repository declares generated/published API contracts, client generation or contract review as part of its delivery surface.

## AXUM-API-001 — Runtime/API contract has one authority chain [MUST]

Do not maintain an unrelated hand-written OpenAPI document and Axum routes as two silent production authorities. Choose a documented direction: generated from code/route metadata, generated from a canonical schema, or explicitly reconciled contract-first workflow.

When OpenAPI is not a product/consumer requirement, do not add a documentation framework merely for completeness.

## AXUM-API-002 — OpenAPI library is project-selected [MUST]

The Harness does not globally mandate utoipa, aide or another library. Select based on the repository's desired source-of-truth model, schema ecosystem, macro tolerance, Axum integration and existing dependencies.

Research baseline examples:

- utoipa + `utoipa-axum`: derive/macro-oriented OpenAPI 3.1 integration and `OpenApiRouter` composition.
- aide: `schemars`-oriented API/router documentation integration.

Do not introduce both without a clear migration/boundary reason.

## AXUM-API-003 — Schema/serialization semantics must agree [MUST]

Generated schemas must reflect actual serde/request/response behavior, including field renaming, optionality/nullability, tagged enums, validation constraints where supported, security requirements and error responses.

A generated schema is not proof of runtime agreement; add contract tests/snapshots where drift matters.

## AXUM-API-004 — Operation identity is stable when consumed [MUST]

If generated clients, gateways or policy tools depend on `operationId`, tags, schema names or security scheme names, treat those identifiers as public API. Avoid accidental renames caused only by Rust module/function refactoring.

## AXUM-API-005 — Error contracts are documented [MUST]

Publicly documented operations include the meaningful error status/body families clients must handle. Do not publish only the success response while runtime exposes structured validation/auth/conflict/error contracts.

## AXUM-API-006 — Security declaration matches enforcement [MUST]

OpenAPI security schemes/requirements describe actual runtime authentication/authorization boundaries. Do not document bearer/OAuth/mTLS protection that middleware does not enforce, or omit protection from contract surfaces consumed for security review.

Documentation is evidence input, not enforcement.

## AXUM-API-007 — HTTP versioning is product/API policy [MUST]

Path/header/media-type API versioning is selected from consumer compatibility needs. The Harness does not force `/v1` for every service. Once a versioning scheme is adopted, route and OpenAPI evolution follow that policy.

## AXUM-API-008 — Breaking-change review is explicit [MUST]

When API compatibility matters, review at least:

- method/path removal or move;
- required request field changes;
- response field/type/enum changes;
- status/error changes;
- auth/security requirement changes;
- default/nullable semantics;
- pagination/order semantics.

Use generated-spec diff/consumer tests where mechanically useful; do not rely only on Rust SemVer because HTTP consumers are outside the crate type system.

## AXUM-API-009 — Framework docs stay at HTTP boundary [MUST]

OpenAPI derives/annotations/router wrappers remain in DTO/HTTP/composition owners. Do not annotate domain owners solely to satisfy documentation generation when doing so couples domain evolution to transport tooling.

Shared types may implement schema traits when that is an intentional public representation and dependency cost is accepted.

## AXUM-API-010 — Docs UI exposure is deployment policy [MUST]

Swagger/Scalar/Redoc or equivalent UI exposure is optional. Public production exposure requires an intentional access/security policy; internal APIs may serve docs only in development/admin contexts.

Do not assume hiding docs is a substitute for authenticating the API itself.

## AXUM-API-011 — Contract generation is deterministic [SHOULD]

Generated specs committed or compared in CI should be deterministic enough for meaningful review. Normalize unstable ordering/timestamps if the tool emits them.

## Verification

When OpenAPI capability is active, use the smallest appropriate evidence set:

- generate spec successfully;
- parse/validate the generated document;
- snapshot/diff public contract when compatibility matters;
- verify representative route/request/response/error/security agreement;
- run generated-client consumer tests when the client is part of the product contract.

No generated file or CI command is PASS unless it actually ran.
