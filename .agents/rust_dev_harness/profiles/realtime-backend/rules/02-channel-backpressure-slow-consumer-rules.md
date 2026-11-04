# Realtime Channel Backpressure and Slow Consumer Rules

## RT-BP-001 — Every queue is bounded [MUST]

Every production real-time handoff, per-connection outbound queue, fan-in queue and retry buffer has an explicit finite capacity unless an external durable store is the intentional queue authority.

Do not use `tokio::sync::mpsc::unbounded_channel` as a normal network backpressure mechanism. System memory is not an acceptable implicit capacity.

## RT-BP-002 — Queue capacity has a reason [MUST]

Record what the capacity protects and what happens when full: wait/backpressure, drop/coalesce, reject/disconnect, spill to durable storage, or another deliberate outcome. Capacity is not a magic tuning number.

## RT-BP-003 — Slow-consumer policy matches delivery semantics [MUST]

For each stream choose deliberately:

- block/backpressure producer;
- coalesce to latest state;
- drop old/new items with observable loss;
- disconnect and require resync/replay;
- persist for replay.

Do not allow one slow client to cause unbounded memory growth or stall unrelated consumers unless that coupling is intentional.

## RT-BP-004 — `broadcast` lag is handled [MUST when broadcast is used]

Tokio `broadcast` is bounded live fan-out. A lagging receiver can receive `RecvError::Lagged(n)` after old values are overwritten. Code must choose whether to tolerate loss, emit a resync marker, replay from durable history, or terminate the subscriber. Never ignore lag silently when event loss matters.

## RT-BP-005 — `watch` means latest-value semantics [MUST when watch is used]

Tokio `watch` retains only the last value; intermediate values are intentionally lost. Use it for state/projection/config/progress where that is the contract, not for semantic event logs.

## RT-BP-006 — `mpsc` owns work, not broadcast fan-out [SHOULD]

Use bounded `mpsc` for owned work queues where one receiver processes each item. Do not duplicate per-subscriber mpsc queues without accounting for N × capacity memory and slow-client cleanup.

## RT-BP-007 — Connection fan-out is capacity-budgeted [MUST]

Memory/resource budgets account for connection count multiplied by per-connection buffers, serialized payloads, channel capacity and protocol overhead. A safe per-connection queue can become unsafe at thousands of connections.

## RT-BP-008 — Producers do not clone large payloads accidentally [SHOULD]

For broadcast/fan-out of substantial immutable payloads, review serialization and cloning strategy. Use shared immutable representations only when measurement and ownership justify them; do not add `Arc` mechanically.

## RT-BP-009 — Coalescing preserves semantic transitions [MUST]

Coalescing may replace redundant state updates but must not erase distinct transitions required for correctness, audit, approval, billing, execution completion or other semantic events.

## RT-BP-010 — Backpressure crosses effect boundaries deliberately [MUST]

If upstream producers are durable DB/event-log writers, request handlers, execution workers or external consumers, define where pressure is absorbed and where admission stops. Do not let backpressure accidentally hold database transactions, locks or other scarce resources across network waits.

## Evidence

Saturation tests assert bounded queue/in-flight counts, chosen slow-consumer behavior, lag observability, recovery after pressure, and absence of hidden unbounded buffering.
