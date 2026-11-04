# Realtime Connection Lifecycle and Cancellation Rules

## RT-LIFE-001 — Every long-lived connection has an owner [MUST]

A connection/session task has one lifecycle owner responsible for admission, child tasks, cancellation, close/drain, joins and final telemetry. Do not detach meaningful reader/writer tasks without ownership.

## RT-LIFE-002 — Child reader/writer tasks are structured [MUST]

If read and write halves run concurrently, define sibling-failure behavior. When one half terminates, the owner decides whether to cancel the other, flush/close, drain, or keep the connection alive. Spawned halves are joined or otherwise deterministically supervised.

## RT-LIFE-003 — Cooperative cancellation is propagated [MUST]

Use a service/connection cancellation mechanism such as `CancellationToken` or an equivalent owned signal. Cancellation must reach blocking bridges, background producers and per-connection writers according to their actual abortability.

## RT-LIFE-004 — Timeout is not proof of side-effect cancellation [MUST]

An idle/request/connection timeout may stop waiting or close a transport while blocking/external work continues. Mutating effects require separate transaction/idempotency/cancellation semantics.

## RT-LIFE-005 — Heartbeat/keepalive has a purpose [MUST when used]

Heartbeat/keepalive exists to keep intermediaries from idling a connection, detect dead peers, or both. Configure interval/timeout from deployment/network behavior and workload; do not emit high-frequency heartbeat traffic by habit.

## RT-LIFE-006 — Idle policy is explicit [MUST]

Define how idle-but-valid clients differ from dead/stalled clients. Idle timeout, heartbeat timeout and application session timeout are separate concepts and may have different values.

## RT-LIFE-007 — Shutdown stops admission before drain [MUST]

Graceful shutdown follows a deliberate sequence:

```text
stop new connection admission
→ signal existing connections
→ flush/emit terminal state if contract allows
→ close/cancel connection tasks
→ join owned tasks
→ close shared fan-out resources
→ report remaining failures/timeouts
```

Do not keep accepting new long-lived sessions while waiting indefinitely for old ones to drain.

## RT-LIFE-008 — Close reason is deliberate [SHOULD]

Where the protocol supports close reasons/codes, distinguish normal shutdown, policy violation, overload/restart, oversized/invalid message and internal error without leaking sensitive implementation detail.

## RT-LIFE-009 — Reconnection storms are bounded [MUST]

Client/server reconnect design uses retry/backoff/jitter or server hints where appropriate so failover/restart does not create an immediate thundering herd. Admission limits remain in force during recovery.

## RT-LIFE-010 — Connection registries do not leak [MUST]

Subscriber/session registries remove terminated connections under every exit path: peer close, I/O error, cancellation, panic boundary, auth expiry/revocation and server shutdown.

## Evidence

Lifecycle changes require tests that exercise normal close, remote disconnect, owner cancellation, sibling task termination, graceful shutdown, idle/heartbeat policy and restart/reconnect behavior as applicable.
