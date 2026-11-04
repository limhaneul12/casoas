# Rust Realtime Backend Profile

Production real-time/streaming specialization for the canonical Rust Engineering Harness.

## Role

This profile adds real-time transport/runtime semantics to the Rust baseline; it does not fork normal/type/async rules.

Authority chain:

```text
Shared Harness contracts
→ Rust normal + type + async rules
→ repository PROJECT_PROFILE / Cargo / toolchain / AGENTS
→ Realtime Backend Profile
→ transport-specific profile (for example Axum SSE/WebSocket)
→ task-relevant Skill / references
→ executed verification evidence
```

Parent bundle → [[Jobs/Harness/Engineering Harness/02 Rust/README|Rust Engineering Harness Bundle]]

Procedure → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/SKILL|Realtime Backend Engineering Skill]]

## Activation

Activate when the repository owns long-lived server/client streams, live fan-out, push delivery, SSE, WebSocket, streaming RPC, or equivalent connection-oriented behavior.

The profile governs semantics shared across transports:

- canonical state/event authority;
- bounded queues and backpressure;
- slow-consumer policy;
- connection/task ownership;
- cancellation, heartbeat and graceful shutdown;
- ordering, deduplication, resume and replay;
- authentication/authorization lifetime;
- per-connection/message/resource limits;
- observability and capacity evidence;
- real-time testing and performance verification.

## Architectural invariant

A transport connection is never the durable/domain state authority.

```text
Domain/Application state + events
           │
           ├── durable state/event history when required
           ├── latest-value projection when appropriate
           └── live fan-out
                    │
             SSE / WebSocket / other transport
```

Live transport is a projection/delivery mechanism. Reconnect, lag or connection loss must not silently corrupt semantic state.

## Delivery semantics

Every stream explicitly selects semantics rather than inheriting them accidentally:

- latest-state: intermediate values may coalesce;
- lossy live-event: lag may drop old events but loss is observable;
- resumable event stream: event IDs + replay source close the reconnect gap;
- command stream: mutating commands require authentication, authorization, idempotency/replay policy and typed acknowledgements/outcomes.

Do not claim exactly-once delivery from TCP/WebSocket/SSE alone.

## Rule set

1. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/01-stream-authority-delivery-rules|Stream Authority and Delivery Rules]]
2. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/02-channel-backpressure-slow-consumer-rules|Channel Backpressure and Slow Consumer Rules]]
3. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/03-connection-lifecycle-cancellation-rules|Connection Lifecycle and Cancellation Rules]]
4. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/04-resume-replay-ordering-rules|Resume Replay and Ordering Rules]]
5. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/05-security-resource-admission-rules|Realtime Security and Admission Rules]]
6. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/06-observability-capacity-performance-rules|Realtime Observability Capacity and Performance Rules]]
7. [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/rules/07-testing-verification-rules|Realtime Testing and Verification Rules]]

## Tokio primitive guidance

Primitive choice follows delivery semantics, not habit:

```text
bounded mpsc  → owned work queue / backpressure
broadcast     → bounded live fan-out; lag is explicit and may lose old items
watch         → latest-value state projection; intermediate values intentionally disappear
oneshot       → one completion/response
CancellationToken → cooperative cancellation signal
persistence/event log → durable replay/history
```

An unbounded channel is not a normal production real-time queue; memory must not be the accidental capacity policy.

## Transport selection

Prefer the least complex transport satisfying the requirement:

- server → client live state/events: SSE or equivalent one-way streaming;
- ordinary mutations: normal HTTP command + SSE state/event projection;
- continuous bidirectional interactive protocol: WebSocket;
- protocol-specific streaming RPC: use its native transport/profile.

Do not choose WebSocket merely because a feature is called “real-time”.

## Forge-oriented default

For Forge/Orchestration-style live Activity, Goal, DAG, Agent, Writer Lease, Queue and Verification projections, default to resumable SSE or latest-state SSE where the server is the primary event producer. Reserve WebSocket for true long-lived bidirectional interactions such as terminal/session traffic or collaborative control.

## Verification

```text
focused delivery/lifecycle test
→ Rust async/concurrency rules
→ realtime profile rules
→ transport-specific tests
→ canonical cargo xtask rules/fmt/check/test/ci
→ stress/fault/load evidence when capacity/performance/reconnect claims change
```

No source-level appearance of a bound, heartbeat or cancellation token is itself proof that the behavior works.

Research provenance → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/research-baseline|Realtime Research Baseline]]
