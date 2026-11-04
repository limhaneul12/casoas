# Axum Extractor Selection

Non-normative decision reference. Governing Extractor/Validation Rules remain authority.

## 1. Core ownership split

Axum extractor choice is fundamentally about **which part of the request is consumed and who owns the conversion**.

```text
Request Parts only                  Whole request/body
------------------                  ------------------
Path                                Json<T>
Query                               Form<T>
State                               String
Extension                           Bytes
HeaderMap / individual headers      Multipart
ConnectInfo                          Request / Body
custom FromRequestParts             custom FromRequest
```

`FromRequestParts` cannot consume the body. `FromRequest` may consume it. In a handler, request-parts extractors come before the one body-consuming extractor.

## 2. Fast decision tree

### Is the value application-global?

Use `State<T>` or a coherent `FromRef` substate.

### Is it derived from URI/query/headers/extensions/connection metadata?

Use a built-in request-parts extractor or implement `FromRequestParts`.

### Does it parse/stream the body?

Use the appropriate body extractor or implement `FromRequest`; keep it last.

### Is it repeated transport context?

A custom request-parts extractor may be appropriate for typed auth principal, tenant header, locale, API version or similar transport concerns.

### Does it require business DB/network work?

Usually stop. Extract transport identity/context, then call the application operation. Do not turn an extractor into an invisible workflow engine.

## 3. Common extractors and intent

| Extractor | Typical intent | Main review point |
|---|---|---|
| `Path<T>` | route captures | parse/error contract, Axum-version route syntax |
| `Query<T>` | query DTO | bounds/default semantics, pagination/cardinality |
| `State<T>` | app-global dependency | coherent state, cheap clone/lifecycle |
| `Extension<T>` | request-derived injected context | missing extension behavior, trust source |
| `HeaderMap` | raw header access | avoid stringly repeated parsing; secrets |
| `Json<T>` | typed JSON body | media type, default body limit, transport/domain validation |
| `Form<T>` | form data | media type/body limit/browser semantics |
| `Bytes`/`String` | bounded whole body | default body limit and allocation |
| `Multipart` | multipart streaming/fields | per-field/total/file count bounds |
| `Request`/`Body` | custom/raw protocols | explicit resource bound; easy to bypass default body limit |
| `ConnectInfo<T>` | socket/connection metadata | proxy vs direct-connection trust |

Optional helpers from `axum-extra` can be useful when the project adopts them; do not add the dependency merely to avoid a few lines.

## 4. Custom request-parts extractor pattern

Use when a reusable transport concern deserves a type:

```rust
struct RequestId(String);

impl<S> FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let value = parts
            .headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(HttpError::missing_request_id)?;

        Ok(Self(value.to_owned()))
    }
}
```

This example demonstrates shape only. A real externally supplied request ID may require validation, length bounds and trust rules.

## 5. Authentication extractor split

A useful pattern:

```text
Authorization/Cookie header
→ parse/verify credential
→ AuthenticatedPrincipal
→ handler/application authorization
```

A custom extractor may verify a signed token when that is a transport authentication concern. If authorization depends on resource/domain state, keep that semantic decision in the application/domain owner rather than pretending token validity is permission.

## 6. Optional versus malformed

Be careful with `Option<T>` and custom optional extraction:

```text
absent       → maybe valid None
malformed    → usually rejection
unauthorized → rejection
```

Do not convert malformed or unauthorized input to `None` simply to simplify handler branches.

## 7. JSON/body validation layers

A robust conceptual split:

```text
HTTP/media-type/body bound
→ serde parse to transport DTO
→ transport field constraints
→ checked domain/application conversion
→ semantic invariant validation
```

One semantic invariant should have one canonical owner. Avoid validating the same rule independently in serde custom code, a validation crate and domain constructors unless each layer protects a distinct boundary.

## 8. Body limits

Axum 0.8.x has a default 2 MiB limit on `Bytes`-based extractors including common `Json`, `Form`, `String` and `Bytes` paths.

That is **not a global request-body firewall**. Raw/custom `Body` consumption can bypass it. Streaming/multipart/custom protocols therefore need explicit total/per-item/work bounds.

A smaller per-route limit can be preferable when the endpoint contract is narrow.

## 9. Rejection normalization

Framework rejections are acceptable as public behavior only when explicitly adopted. Otherwise map them to the service error envelope at a narrow HTTP boundary.

Test distinctions clients depend on:

- missing path/query/header;
- malformed syntax;
- unsupported content type;
- invalid JSON;
- body too large;
- semantically invalid but syntactically valid request.

## 10. Smells

Reconsider when:

- handler manually reparses JSON from raw bytes without a protocol reason;
- custom extractor executes a multi-write DB workflow;
- body gets read in middleware and again in handler;
- `Option` hides malformed credentials;
- raw `Body` is accepted from untrusted callers with no explicit bound;
- state/dependencies are pulled through a generic service locator inside the extractor;
- every header gets a bespoke extractor with no reuse or invariant value.
