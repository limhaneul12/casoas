# Axum State and Dependency Rules

## AXUM-STATE-001 — `State` is application-global composition state [MUST]

Use Axum `State` for dependencies/configuration that are intentionally shared across requests. Request-derived information such as authenticated principal, request ID or connection-derived context is not application-global state and should travel through request extensions/extractors.

## AXUM-STATE-002 — AppState is a composition projection, not a service locator [MUST]

`AppState` may aggregate the dependencies required to compose the HTTP application, but handlers should not receive an unbounded bag of unrelated capabilities and dynamically choose among them.

Prefer explicit handler/application dependencies and coherent substates over a growing universal context object.

## AXUM-STATE-003 — Use `FromRef` for coherent substates [SHOULD]

When a feature/router needs a smaller view of the application's state, use Axum's `FromRef` pattern or an equally explicit composition boundary so the feature does not depend on unrelated state.

Do not create one substate per field mechanically. Substates represent coherent capability ownership.

## AXUM-STATE-004 — Do not add `Arc` by reflex [MUST]

Follow Rust type/ownership rules. Many production handles (`PgPool`, clients, immutable config handles) are already cheap `Clone` handles. Do not wrap the entire state or every dependency in `Arc` merely because Axum requires clonable state.

Use `Arc` when there is an actual shared-ownership need, not as an async-framework default.

## AXUM-STATE-005 — Do not mutex pools/clients [MUST]

Do not wrap connection pools, thread-safe clients or other concurrency-safe handles in an outer `Mutex`/`RwLock` unless the wrapped API is genuinely not safe for concurrent use and the lock owns a documented invariant.

The Rust shared-state rules remain authority for lock choice and lock scope.

## AXUM-STATE-006 — Dependency initialization is outside request hot path [MUST]

Initialize configuration, pools, clients, migrations/bootstrap prerequisites and long-lived resources before accepting traffic unless lazy initialization is an explicit resilience design.

Startup failure should be typed/observable rather than converted into repeated request-time initialization failures.

## AXUM-STATE-007 — Capability lifetime is explicit [MUST]

For each shared dependency identify:

- construction owner;
- clone/share semantics;
- shutdown owner;
- pool/concurrency bounds;
- credential/config refresh semantics if applicable.

Do not let Axum state storage obscure resource lifecycle.

## AXUM-STATE-008 — Traits follow substitutability, not mocking fashion [SHOULD]

Do not introduce `Arc<dyn Repository>` / `Arc<dyn Service>` solely because tests might need a mock. Add a trait/port when multiple implementations, isolation boundaries, architecture or testing seams justify a stable abstraction.

Shared Solution Minimality remains authority.

## AXUM-STATE-009 — Mutable shared state needs a semantic owner [MUST]

If request handlers mutate shared in-memory state, define the consistency/invariant model first. Choose lock/channel/actor/state-machine semantics using Rust async/type rules. Do not default to `Arc<Mutex<HashMap<...>>>` without considering contention, lifecycle and persistence semantics.

## AXUM-STATE-010 — State type changes are composition changes [MUST]

Changing state shape can affect router composition, tests and middleware. Verify nested/merged routers still agree on state types and that app factories supply all required state before serving/testing.

## Review checklist

- Is this value truly global across requests?
- Is it already a cheap thread-safe clone handle?
- Can a sub-router depend on a smaller coherent substate?
- Is a new trait/Arc/lock semantically necessary?
- Who starts and shuts down the dependency?
- Are pool/concurrency bounds explicit?

## Evidence

State/composition changes require at least compile/type verification plus focused router/handler tests. Lifecycle or contention claims require corresponding startup/shutdown/concurrency evidence.
