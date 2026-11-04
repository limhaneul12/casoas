# Axum State Patterns

Non-normative state/composition reference. Governing State and Dependency Rules remain authority.

## 1. State means application-global shared capability

Axum `State<T>` is appropriate for values intentionally shared across requests:

```text
configuration
DB pool
HTTP client
cache/client handle
application service/operation dependencies
metrics handle
```

Request-derived values such as current principal, request ID, locale or per-request transaction context are not global state merely because handlers need them.

## 2. Minimal `AppState`

A straightforward composition object is often enough:

```rust
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<AppConfig>,
}
```

`PgPool` is already a cloneable concurrent handle; wrapping it in another `Arc<Mutex<_>>` is normally unnecessary and harmful.

`Arc<AppConfig>` is only illustrative. An owned `AppConfig`, `Arc`, or another handle depends on size/lifetime/ownership.

## 3. `FromRef` coherent substate

A feature may need only one coherent capability:

```rust
#[derive(Clone)]
pub struct AppState {
    pub users: UserService,
    pub audit: AuditService,
}

impl FromRef<AppState> for UserService {
    fn from_ref(state: &AppState) -> Self {
        state.users.clone()
    }
}
```

Then a handler can extract:

```rust
async fn get_user(
    State(users): State<UserService>,
    Path(id): Path<UserId>,
) -> Result<Json<UserResponse>, HttpError> {
    // ...
}
```

Use this when the substate represents a real capability boundary. Do not create a `FromRef` impl for every field mechanically.

## 4. Request-scoped authenticated principal

Typical flow:

```text
middleware / custom request-parts extractor
→ verify credential
→ insert/return AuthenticatedPrincipal
→ handler extracts principal
```

For middleware-inserted context:

```rust
async fn handler(
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> ...
```

Do not write the current principal into `AppState`, which would create cross-request corruption and incorrect ownership.

## 5. Avoid the mega-state service locator

Smell:

```rust
struct AppState {
    db: PgPool,
    redis: RedisClient,
    config: Config,
    users: Users,
    orders: Orders,
    billing: Billing,
    audit: Audit,
    queue: Queue,
    search: Search,
    mail: Mail,
    // and every future capability...
}
```

A large state is not automatically wrong. The issue is handlers gaining arbitrary access to unrelated capabilities.

Countermeasures:

- coherent substates;
- feature/application operations that encapsulate their required dependencies;
- narrower visibility;
- composition-time construction rather than dynamic service lookup.

Do not over-correct by creating dozens of one-field wrapper types without semantic value.

## 6. Cheap clone handles versus shared ownership

Before `Arc<T>`, ask:

```text
Does T already implement cheap Clone?
Is shared ownership actually required?
Is immutable ownership enough?
Does T internally manage its own concurrency?
```

Common pool/client handles are often designed to clone cheaply.

Use `Arc` when multiple owners need the same allocation/lifetime, not because the web framework is async.

## 7. Mutable in-memory state

If state must mutate across requests, choose the concurrency model from the invariant:

```text
Mutex/RwLock        → small shared critical state
bounded channel     → serialized owner / backpressure
actor/task owner    → lifecycle + command processing
concurrent map      → independent keyed mutation if semantics fit
persistent store    → durability/source-of-truth required
```

Do not begin with `Arc<Mutex<HashMap<...>>>` and discover consistency requirements later.

## 8. Pool sizing is not handler state design

A DB pool in `AppState` is a resource handle. Its limits belong to resource/workload policy:

```text
HTTP concurrency
→ application fan-out
→ DB pool max connections
→ DB server capacity
```

Increasing pool size can worsen downstream saturation. Measure pool wait and DB capacity instead of treating `max_connections` as a speed knob.

## 9. Startup lifecycle

Prefer:

```text
load + validate config
→ establish required external resources
→ migration/bootstrap according to policy
→ construct dependencies
→ construct AppState
→ build Router
→ accept traffic
```

Lazy initialization can be valid for optional/resilient dependencies, but it should be an explicit design with failure/circuit semantics.

## 10. Shutdown lifecycle

`AppState` cloning does not define shutdown ownership. Long-lived background tasks, subscriptions and resources need explicit lifecycle owners/cancellation.

For normal pool/client handles, dropping clones may be sufficient; for owned tasks, ensure shutdown coordination under Rust async rules.

## 11. Trait/service pattern

Do not automatically turn every dependency into:

```rust
Arc<dyn FooService + Send + Sync>
```

Prefer concrete types when there is one stable implementation and no boundary reason. Introduce a trait when substitutability, plugin architecture, test seam, package boundary or multiple implementations justify it.

If a trait exists, keep it at the owner that needs abstraction; avoid making HTTP transport define domain ports.

## 12. State review questions

- Is the value global across all requests?
- Is it request-scoped instead?
- Is `Clone` already cheap?
- What invariant justifies a lock?
- What is the pool/concurrency bound?
- Does a handler receive capabilities it should not own?
- Who constructs and shuts down this dependency?
- Is the trait/service layer real architecture or mocking ceremony?
