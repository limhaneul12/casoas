# Realtime Channel Selection

Non-normative Tokio primitive reference.

## Bounded mpsc

Use for an owned queue with one receiver and explicit backpressure.

```text
producer(s) → bounded mpsc(N) → one owner/worker
```

When full, async `send` waits. Capacity should protect memory/downstream concurrency and have a saturation policy. Do not hold scarce locks/transactions while waiting to send.

## Unbounded mpsc

`unbounded_channel` has no backpressure; arbitrary messages can accumulate until memory is exhausted. Treat it as exceptional tooling/control use only when an external bound proves safety. It is not the default for network-facing or producer-faster-than-consumer paths.

## Broadcast

Use for bounded live fan-out to multiple active subscribers.

```text
producer(s) → broadcast(capacity) → subscriber A/B/C
```

Important semantics:

- new subscribers receive items sent after subscription;
- old items are overwritten once retained capacity is exceeded;
- slow receivers get `RecvError::Lagged(n)`;
- lag does not automatically disconnect the receiver;
- receiver code must choose resync/drop/continue/terminate policy.

Do not call broadcast a durable event log.

## Watch

Use when only the newest state matters.

```text
status/progress/config owner → watch(latest) → subscribers
```

All intermediate values may disappear. Each receiver tracks whether it has observed the current value. Good fits include latest Goal/DAG projection, queue count, runtime status and progress. Bad fit: audit/approval/execution terminal event history.

## Oneshot

Use for one completion/result/ack between owners. It is not a stream or fan-out primitive.

## CancellationToken

Use as cooperative lifecycle signal for service/connection/task trees. A token is a signal, not proof each child task or blocking operation can actually cancel. Owners still join tasks and handle non-abortable effects.

## Durable replay

Use persistence/event log/revision history when clients need missed events after restart/lag beyond in-memory retention.

```text
live path: durable append → broadcast → subscribers
replay: cursor → bounded durable query → catch up → live tail
```

## Selection matrix

| Need | Primitive |
| --- | --- |
| one worker queue | bounded mpsc |
| one event to many live consumers | broadcast |
| current state only | watch |
| one response/completion | oneshot |
| cancellation signal | CancellationToken |
| restart-safe history | durable store/log |

## Composition examples

### Latest UI projection

```text
DB/domain state revision
→ projection reducer
→ watch
→ SSE connections
```

### Resumable semantic events

```text
transaction
→ durable event row/revision
→ broadcast
→ SSE subscribers

Lagged / reconnect cursor
→ durable replay
→ live tail
```

### WebSocket command processing

```text
WS reader
→ validation/auth
→ bounded mpsc commands
→ application owner
→ result/event
→ bounded/coalesced outbound path
→ WS writer
```

## Review traps

- per-client unbounded mpsc;
- using watch for events that must all be seen;
- ignoring `Lagged`;
- capacity chosen without multiplying by connection count;
- broadcast clone cost ignored for large payloads;
- send backpressure while holding DB transaction/Mutex guard;
- CancellationToken created but child task never selects/observes it;
- in-memory channel called “resume history”.
