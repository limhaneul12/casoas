# Axum Architecture Patterns

Non-normative implementation reference. Governing Rules remain authority.

## 1. Preferred dependency direction

For a service with meaningful domain/application logic:

```text
                 composition root
                       │
               Axum Router / Tower
                       │
                 HTTP adapter
        ┌──────────────┼──────────────┐
    extractors       handlers      responses
        │              │
        └──────► application ◄───────┘
                     │    │
                  domain  ports
                           │
                 persistence/clients
```

The important property is **semantic direction**, not directory count.

HTTP-specific types terminate at the adapter/application boundary. SQLx, network clients and other effects terminate at effect/repository owners. The domain does not need to know which protocol invoked the operation.

## 2. Small service — do not over-architect

A small, low-domain-complexity service can remain one crate:

```text
src/
├─ main.rs
├─ app.rs             # composition/startup
├─ http/
│  ├─ mod.rs
│  ├─ users.rs
│  └─ error.rs
├─ domain.rs           # only if real domain semantics exist
└─ persistence.rs      # only if persistence exists
```

Do not create `<REDACTED_LONG_VALUE>` folders merely because an architecture diagram contains them. Split when responsibility, invariant, lifecycle, dependency or change ownership warrants it.

## 3. Medium/complex service

A common shape when application/domain/effects are independently substantial:

```text
src/
├─ app/                # commands/queries/use cases
├─ domain/             # invariants/value/state logic
├─ http/               # Axum transport adapter
│  ├─ router.rs
│  ├─ dto.rs
│  ├─ error.rs
│  └─ feature/...      # when feature ownership is real
├─ persistence/        # SQLx repository/effect owner
├─ clients/            # external network effects
└─ bootstrap.rs        # composition root
```

This remains one crate unless crate boundaries provide real compilation/API/unsafe/ownership isolation value.

## 4. Application operation as protocol-neutral authority

Prefer:

```rust
pub struct CreateUser {
    pub email: EmailAddress,
}

pub async fn create_user(
    deps: &CreateUserDeps,
    command: CreateUser,
) -> Result<User, CreateUserError> {
    // semantic operation
}
```

Then adapters project it:

```text
REST POST /users ─┐
MCP create_user  ──┼→ create_user(command)
gRPC CreateUser  ──┘
```

Do not make REST JSON/HTTP semantics the domain authority merely because REST was implemented first.

## 5. App factory and composition root

A useful testable pattern:

```rust
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(users::router())
        .merge(admin::router())
        .with_state(state)
}
```

Initialization that may fail stays outside the pure router assembly where practical:

```text
load config
→ establish pools/clients
→ migrations/startup validation if policy requires
→ construct AppState
→ build Router
→ bind listener
→ serve with graceful shutdown
```

This makes startup failures explicit and lets router tests reuse production composition.

## 6. Feature router pattern

When a feature owns several related endpoints:

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", get(get_user).delete(delete_user))
}
```

Keep feature-local DTO/error/authorization helpers nearby if they have coherent ownership. Do not move every handler into a separate module solely to keep files tiny.

## 7. Where authentication context belongs

Typical flow:

```text
request credentials
→ auth middleware/custom request-parts extractor
→ AuthenticatedPrincipal request extension/extractor
→ handler
→ application authorization/domain policy where semantic state is needed
```

Transport authentication may reject malformed/invalid credentials early. Resource/business authorization often needs application/domain state and should not be reduced to a generic middleware boolean if that loses semantics.

## 8. State boundary

`AppState` represents shared capabilities needed to compose the HTTP application. It is not itself the domain model.

Example:

```rust
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub user_service: UserService,
    pub config: Arc<AppConfig>,
}
```

This is only an example. Do not add `Arc` around cheap clone handles, and do not create service wrappers with no semantic reason.

## 9. HTTP error boundary

```text
Domain/Application error
          │
          ▼
HTTP mapping owner
          │
          ▼
status + headers + safe body
```

Keep mapping stable and testable. Internal cause chains go to trusted telemetry, not the client body.

## 10. Protocol coexistence

For a service exposing Axum HTTP plus RMCP/Tonic:

```text
             application/domain
                ▲      ▲
                │      │
      ┌─────────┘      └─────────┐
      │                          │
 Axum/Tower                  RMCP/Tonic
 HTTP adapter               protocol adapter
```

Tower can be a useful shared service composition substrate, but do not force unlike protocol semantics into one HTTP-shaped contract.

## 11. Architecture smells

Question the design when:

- handler imports SQL query construction and domain transition code together;
- domain imports `axum` or `StatusCode` for convenience;
- every operation gets a trait with one implementation;
- every dependency is `Arc<dyn Trait>`;
- `AppState` becomes a global service locator;
- a custom middleware runtime duplicates Tower;
- REST and MCP independently implement the same business operation;
- each endpoint creates a new crate/module without independent ownership.

See [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/axum-backend/references/anti-patterns|Axum Anti-Patterns]].
