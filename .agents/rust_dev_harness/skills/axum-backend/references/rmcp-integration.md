# Axum RMCP Integration

Optional MCP capability reference. Governing Rust/Axum rules and the official MCP protocol/SDK remain authority.

Research baseline at 2026-09-09: the official Rust MCP SDK (`rmcp`) implements stable MCP `2026-07-28` with compatibility for earlier protocol versions. Its Streamable HTTP service is Tower-compatible and can be mounted into Axum.

## 1. Application operation remains authority

Preferred architecture:

```text
REST adapter ─┐
RMCP tool    ──┼→ application command/query/service → domain/effects
gRPC adapter ──┘
```

Do not make the REST endpoint the business source of truth and then call it over loopback HTTP from MCP solely to reuse behavior.

HTTP-to-MCP projection can be appropriate for intentionally HTTP-shaped generic gateways, but domain-heavy systems should usually share the application operation below transport.

## 2. Prefer official protocol implementation

For normal production MCP work, prefer `rmcp` over implementing JSON-RPC/MCP negotiation, Streamable HTTP sessions, task semantics, headers and protocol evolution manually.

A custom MCP stack requires a concrete requirement the official SDK cannot meet plus protocol conformance tests. Convenience or a smaller dependency graph alone is not enough for a long-lived protocol fork.

## 3. Axum mounting model

The official Streamable HTTP server is a Tower service, so Axum can mount it at a service boundary conceptually like:

```rust
let app = Router::new()
    .nest_service("/mcp", mcp_service);
```

Exact constructors/features depend on the repository's `rmcp` version. Follow current official SDK examples rather than freezing this reference as compile-ready version-specific code.

This is one reason Tower composition is valuable: MCP can coexist with Axum without a bespoke middleware runtime.

## 4. Stateless versus stateful server semantics

MCP `2026-07-28` supports stateless Streamable HTTP patterns and the SDK exposes service-factory/state mechanisms.

If using a stateless server factory, do not assume mutable per-session service instance memory survives across calls. Durable/shared state should live in explicit cloneable handles or application persistence according to semantics.

Distinguish:

```text
transport session state
application shared state
durable domain state
```

Do not conflate them.

## 5. Tool contract

A tool has its own protocol contract:

```text
tool name
description
input schema
output/content schema where used
errors/protocol outcomes
side-effect/idempotency semantics
```

Do not blindly mirror every REST route as an MCP tool. Expose operations that are meaningful and safe for agent invocation.

For tool inputs, prefer typed application commands/DTO projections with JSON Schema that accurately represents required fields and constraints.

## 6. Schema authority

The MCP SDK/tool macro/schema machinery may use JSON Schema independently from OpenAPI. If both OpenAPI and MCP expose the same application operation, avoid maintaining two unrelated semantic schemas.

Possible model:

```text
application command/result types
      │             │
HTTP DTO/OpenAPI   MCP input/output schema
```

Shared schema traits may be justified when the representations are intentionally identical; otherwise use explicit adapter DTOs.

## 7. Authentication/authorization

MCP transport authentication does not replace operation authorization. Use the deployment's official/authenticated MCP transport capability and pass authenticated principal/context into the same application authorization owner used by other adapters.

Avoid special MCP-only bypasses for operations that are protected in REST unless the trust boundary explicitly differs and is documented.

## 8. Side effects and agent safety

For mutating tools identify:

- authorization;
- idempotency/replay behavior;
- confirmation/approval boundary when applicable;
- request/tool execution timeout;
- partial completion behavior;
- durable audit/evidence;
- duplicate-call consequences.

MCP clients/agents may retry or issue similar calls. Do not rely on conversational intent as a transaction/idempotency mechanism.

## 9. Tower middleware around MCP

Because RMCP Streamable HTTP is a Tower service, shared transport controls can be composed when their semantics fit:

```text
trusted proxy normalization
request correlation/tracing
safe resource/concurrency limits
auth transport layer
RMCP StreamableHttpService
```

Do not assume arbitrary Axum route middleware and MCP protocol semantics are interchangeable. Test the negotiated MCP behavior after middleware is applied.

## 10. Protocol-version evolution

MCP evolves faster than ordinary HTTP route syntax. Therefore:

- pin/test the repository SDK version;
- use protocol negotiation from the SDK;
- avoid hardcoding a protocol version in application logic unless the product intentionally rejects others;
- review SDK/protocol release notes before upgrades;
- keep conformance/integration tests at the protocol boundary.

## 11. Testing

Test different claims separately:

```text
application operation tests       → business semantics
RMCP tool unit/schema tests        → tool contract
Streamable HTTP integration       → protocol negotiation/dispatch
Axum-mounted service tests        → shared middleware/routing integration
real client E2E if required       → interoperability
```

Representative checks:

- initialize/protocol negotiation accepted as intended;
- `tools/list`/tool schema reflects current contract;
- tool invocation maps typed input correctly;
- errors do not leak internal detail;
- mutating calls preserve authorization/idempotency policy;
- stateless/stateful behavior matches service design.

Do not infer MCP correctness because the REST endpoint or `/health` returns 200.

## 12. OpenAPI-to-MCP generation

Generating MCP tools from OpenAPI can be useful for generic external API mediation. It is less attractive as the primary internal architecture when:

- HTTP semantics distort the application command;
- path/query/body parameters need artificial merging;
- MCP output schemas need richer typed semantics;
- operation authorization differs;
- direct in-process application calls avoid unnecessary serialization/network work.

Use OpenAPI→MCP as a deliberate adapter/gateway technique, not a universal rule.

## 13. Anti-patterns

- hand-written MCP JSON-RPC server despite official SDK fitting requirements;
- hardcoded stale protocol version;
- REST loopback request used as the application layer;
- every HTTP route automatically exposed as a tool;
- stateful assumptions inside a stateless service factory;
- MCP tools bypassing application authorization;
- no idempotency/audit model for mutating tools;
- input schema describing only body while required path/query semantics are hidden;
- MCP success inferred only from server startup or HTTP 200;
- custom middleware breaking required MCP headers/stream semantics without integration tests.
