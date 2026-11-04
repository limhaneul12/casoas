# Realtime Resume Replay and Ordering Rules

## RT-REPLAY-001 — Resume source is explicit [MUST]

A resumable stream names the source that can reconstruct missed data: durable event log, append-only journal, revisioned state history, or another bounded canonical source. An in-memory channel alone is not durable replay.

## RT-REPLAY-002 — Live fan-out and replay are separate responsibilities [MUST]

Tokio `broadcast` or equivalent may serve live subscribers, but missed items after lag/restart require a separate replay source when loss is unacceptable.

## RT-REPLAY-003 — Resume cursor is monotonic in its declared scope [MUST]

Event IDs/revisions/cursors used for resume have a documented ordering scope and stable encoding. Do not reuse wall-clock timestamps as unique total order without proving collision/order semantics.

## RT-REPLAY-004 — Duplicate delivery is expected and safe [MUST when replay/reconnect exists]

Reconnect/replay boundaries can duplicate the last acknowledged or concurrently delivered event. Consumers or application handlers must tolerate duplicates according to the stream contract.

## RT-REPLAY-005 — Snapshot-to-live transition is atomic by revision, not by timing [MUST]

Use a revision/cursor fence:

```text
read snapshot at revision R
→ subscribe/replay events after R
→ transition to live tail
```

Do not depend on "subscribe quickly after reading" to avoid gaps.

## RT-REPLAY-006 — Replay is bounded [MUST]

Replay endpoints/flows define maximum history window, batch size, byte size, execution time and pagination/cursor behavior. A reconnecting client must not force an unbounded historical scan into memory.

## RT-REPLAY-007 — Retention policy matches resume promise [MUST]

If clients may resume from an old cursor, retention must cover the promised window or the protocol must return a clear resync-required outcome.

## RT-REPLAY-008 — Invalid/expired cursor fails explicitly [MUST]

Malformed, future, unknown, expired or unauthorized cursors are not silently treated as "start from now" when that could hide loss. Return a typed resync/reset/error contract.

## RT-REPLAY-009 — Per-tenant/workspace isolation applies to replay [MUST]

Replay queries and event cursors remain scoped to the authenticated/authorized stream owner. Event IDs are not capabilities and must not allow cross-scope history access.

## RT-REPLAY-010 — Reconnect/backoff is transport-specific but semantically consistent [MUST]

SSE retry hints, WebSocket reconnect algorithms and custom clients may differ, but all respect admission limits, authentication refresh and replay/idempotency policy.

## Evidence

Test disconnect between events, disconnect after send-before-observe, replay duplication, expired cursor, retention gap, snapshot/live boundary, restart/failover where applicable, and authorization isolation.
