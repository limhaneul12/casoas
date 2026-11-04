# Axum Endpoint Workflow

Non-normative procedure for adding or modifying an Axum endpoint. Governing Axum Rules remain authority.

## 1. Find the operation owner first

Before writing a route, answer:

- What user/system operation is being exposed?
- Does the application/domain already own this operation?
- Is another adapter (MCP/gRPC/job) already invoking it?
- Which side effects and invariants are involved?

If the operation already exists, the endpoint should be an adapter, not a second implementation.

## 2. Write the HTTP contract

Record the intended external behavior:

```text
METHOD + PATH
request headers/query/path/body
authentication/authorization
success status + body
meaningful error statuses + body
resource limits / timeout
idempotency or concurrency preconditions
```

For Axum 0.8.x route captures use `/{id}` and wildcards `/{*rest}`.

## 3. Define transport representations

Example:

```rust
#[derive(Deserialize)]
struct CreateUserBody {
    email: String,
}

#[derive(Serialize)]
struct UserResponse {
    id: Uuid,
    email: String,
}
```

Convert external strings/optionality into checked application/domain values before invoking the operation.

Do not reuse a domain type as a request DTO when transport compatibility/validation semantics differ. Do not duplicate it when the representation is genuinely identical and stable.

## 4. Select extractors

Example endpoint shape:

```rust
async fn create_user(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(body): Json<CreateUserBody>,
) -> Result<(StatusCode, Json<UserResponse>), HttpError> {
    // adapter only
}
```

The body-consuming extractor is last. Use `Path`, `Query`, headers/extensions and state before it.

For repeated transport context, consider a typed custom `FromRequestParts` extractor rather than parsing the same headers in every handler.

## 5. Convert and call the application operation

Conceptual flow:

```rust
let email = EmailAddress::parse(body.email)
    .map_err(HttpError::invalid_email)?;

let user = state
    .users
    .create_user(CreateUser { email }, principal)
    .await
    .map_err(HttpError::from)?;
```

The example does not mandate a service object or method shape. Use the repository's existing operation owner.

No SQL, retry loop or domain state-machine logic should appear here if another owner exists.

## 6. Map result to HTTP explicitly

Return status/body/header semantics intentionally:

```rust
Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
```

Ensure application errors map through the canonical HTTP error contract, not ad hoc per-handler tuples.

## 7. Attach route at the correct ownership scope

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", post(create_user))
}
```

Attach authentication, body limits, timeout or other route-specific middleware at the narrowest correct router/route scope.

Do not add a global bypass-oriented policy when only one feature needs the behavior.

## 8. Update optional contract surfaces

If active:

- OpenAPI: request/response/error/security documentation and compatibility review;
- SQLx: repository query/migration and DB evidence;
- RMCP: only if the underlying application operation/tool contract changed; do not proxy REST simply to keep code reuse;
- generated client: regenerate/diff/test according to repository policy.

## 9. Focused endpoint tests

Minimum relevant cases depend on the change. Typical set:

```text
success
malformed request
semantic validation failure
unauthenticated / forbidden
not found / conflict
wrong method / unknown path
body/resource limit
canonical internal error sanitization
```

Use the production router composition through `oneshot` where possible.

## 10. Review before declaring done

- Could this handler be called only by HTTP, while the operation remains reusable?
- Did the body consumer remain last?
- Did I add an unnecessary Arc/trait/service wrapper?
- Does a small request trigger unbounded fan-out or blocking work?
- Is the error status/body stable and safe?
- Does middleware ordering change the result?
- If OpenAPI is active, does generated contract match runtime?
- Were the actual focused + canonical gates executed?

## Endpoint change mini-checklist

```text
[ ] authority found
[ ] HTTP contract defined
[ ] typed input conversion
[ ] one application operation
[ ] explicit error mapping
[ ] resource limits reviewed
[ ] route/middleware scope correct
[ ] negative tests added/updated
[ ] optional contract surfaces updated
[ ] canonical verification executed
```
