# Axum HTTP Boundary Rules

Normative specialization of the Rust Engineering Harness for Axum-owned HTTP transport boundaries.

## AXUM-HTTP-001 — Transport ownership [MUST]

Axum owns HTTP transport concerns only. Keep domain and application decisions independent of Axum-specific types unless the repository explicitly defines a different accepted architecture.

Transport concerns include:

- route/method selection;
- HTTP headers/status codes;
- request extraction and protocol-level validation;
- authentication material extraction;
- response serialization/content negotiation;
- HTTP middleware;
- connection/request metadata.

Business invariants, state transitions and use-case decisions belong to their canonical application/domain owner.

## AXUM-HTTP-002 — Dependency direction [MUST]

Do not introduce `axum::*`, `Router`, extractor/rejection types, Tower middleware types, HTTP status/header types or OpenAPI annotations into a pure domain owner merely to simplify a handler.

Prefer:

```text
HTTP representation
→ checked transport DTO
→ application/domain value
→ application operation
→ typed result
→ HTTP representation
```

A repository may intentionally combine layers in a small service, but semantic ownership still follows this direction even if files remain few.

## AXUM-HTTP-003 — One canonical operation owner [MUST]

Do not implement the same business operation independently in REST, MCP, gRPC or background-job handlers. Multiple protocol adapters may project one application contract.

Avoid:

```text
REST handler → business algorithm A
MCP tool     → business algorithm B
```

Prefer:

```text
REST ─┐
MCP  ─┼→ application operation → domain/effects
 gRPC ─┘
```

## AXUM-HTTP-004 — Transport DTO versus domain value [SHOULD]

Keep externally shaped/request-evolution DTOs separate from domain types when transport concerns introduce optionality, naming, compatibility or validation semantics that are not domain semantics.

Do not create duplicate DTOs when the representation is genuinely identical and stable; Shared Solution Minimality still applies.

## AXUM-HTTP-005 — Protocol semantics are explicit [MUST]

An endpoint change must account for the externally observable contract it changes: method, path, status, headers, body shape, authentication/authorization, idempotency expectations and compatibility where applicable.

Do not treat a successful compile as evidence that the HTTP contract is preserved.

## AXUM-HTTP-006 — Request-derived versus application-global data [MUST]

Request-derived identity, authorization context, trace/request metadata and connection metadata are request-scoped. Do not place them into application-global `State`.

## AXUM-HTTP-007 — No hidden effect ownership [MUST]

Do not hide database/network/filesystem/process side effects inside generic extractors or serialization helpers. Transport extraction may resolve transport identity/context, but application effects remain explicit at the operation/effect boundary.

## AXUM-HTTP-008 — Version-specific syntax is not architecture [MUST]

Axum-version mechanics such as route syntax belong to the current framework profile/Skill references. Do not turn release-specific spelling into broad architectural policy. Repository version remains authority.

## Review questions

- Which layer owns the semantic decision?
- Could another transport call the same application operation without constructing Axum types?
- Are status/header/body decisions centralized at an HTTP owner?
- Did this change create a second implementation of an existing operation?
- Is request-scoped information accidentally becoming global state?

## Evidence

HTTP-boundary claims require focused handler/router tests where behavior changed. Cross-protocol reuse claims require tests at the shared application boundary or adapter integration tests; file layout alone is not proof.
