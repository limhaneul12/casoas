# Realtime Stream Authority and Delivery Rules

## RT-STREAM-001 — Connection is not semantic authority [MUST]

A live SSE/WebSocket/stream connection is a delivery/projection boundary, not the canonical owner of domain state or durable history. Connection loss, reconnect, proxy reset or client refresh must not silently mutate semantic truth.

## RT-STREAM-002 — Delivery semantics are declared [MUST]

Every stream identifies which model it uses:

- latest-state/coalesced;
- lossy live-event;
- ordered live-event;
- resumable/replayable event stream;
- bidirectional command/event session.

Do not use words such as "reliable", "exactly once" or "resumable" without defining the actual acknowledgement, replay and deduplication mechanics.

## RT-STREAM-003 — Exactly-once is not inferred from transport [MUST]

TCP, SSE and WebSocket do not by themselves provide exactly-once application semantics. Mutating commands require explicit idempotency/deduplication/transaction policy when retry or reconnect can repeat delivery.

## RT-STREAM-004 — Event versus state is explicit [MUST]

A state projection may intentionally coalesce intermediate values. A semantic event that clients must observe must not be silently treated as latest-state data.

Examples:

```text
progress 71% → 72% → 73%      latest-state may coalesce
approval.created               semantic event; loss policy must be explicit
execution.completed            semantic event; resume/replay may be required
```

## RT-STREAM-005 — Transport adapters do not duplicate business operations [MUST]

REST, SSE, WebSocket, MCP and gRPC adapters may expose one application operation/state/event authority. Do not reimplement domain transitions inside connection handlers.

## RT-STREAM-006 — Event identity is semantic when resume/dedup needs it [MUST]

If reconnect/replay/deduplication matters, assign a stable event sequence/cursor/ID at the canonical event owner or durable projection boundary. Connection-local counters are insufficient when they reset on restart or failover.

## RT-STREAM-007 — Ordering scope is defined [MUST]

State whether ordering is guaranteed globally, per workspace, per aggregate/entity, per connection, or not at all. Do not imply global order merely because one Tokio task currently sends messages sequentially.

## RT-STREAM-008 — Snapshot + stream handoff closes the gap [MUST when used]

If clients build state from a snapshot plus later events, define a cursor/revision boundary that prevents missing or double-applying events between snapshot creation and live subscription.

## RT-STREAM-009 — Delivery payloads are typed and versioned by contract [SHOULD]

Use typed envelopes/events and stable discriminants when consumers depend on them. Versioning strategy follows consumer compatibility needs; do not add a version field mechanically to every private stream.

## RT-STREAM-010 — Connection-local convenience state stays disposable [MUST]

Heartbeat timestamps, socket subscriptions and transient send buffers may be connection-local. Durable business state, authorization decisions that outlive the connection, or workflow progress must remain with their real owner.

## Evidence

Changes to delivery guarantees require executable reconnect/lag/order/idempotency tests at the narrowest truthful boundary. Source code shape is not proof of delivery semantics.
