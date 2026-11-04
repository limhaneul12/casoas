# Realtime Capacity Planning

Non-normative capacity/performance reference. Governing Realtime observability/capacity rules remain authority.

## 1. Capacity is multiplicative

Model at least:

```text
N connections
× per-connection task/state cost
× per-connection outbound capacity/payload size
+ shared live-fanout retention
+ replay/history working set
+ serialization/compression buffers
+ heartbeat traffic
+ downstream DB/network fan-out
```

A 32-message queue is not “small” if every one of 20,000 connections owns it and each payload is large.

## 2. Per-connection memory estimate

Approximate before load testing:

```text
connection runtime state
+ read/write buffers
+ channel slots × retained payload/reference size
+ subscription metadata
+ auth/session context
+ serialization scratch
```

Then verify RSS empirically under realistic connection count and payload distribution.

Do not treat object `size_of` as total heap/network runtime memory proof.

## 3. Event-rate budget

Separate:

```text
producer events/sec
× subscribers/event
= logical fan-out deliveries/sec
```

from actual serialized bytes/sec. A single global event broadcast to 5,000 subscribers is 5,000 deliveries even if the application produces one event.

## 4. State coalescing advantage

For replaceable status/progress, coalescing can reduce work substantially:

```text
producer 100 updates/sec
client can render 10/sec
→ latest-state coalescing avoids 90 obsolete sends/sec
```

Only use this when intermediate updates are semantically disposable.

## 5. Replay storm budget

After restart/network outage many clients may reconnect together:

```text
reconnecting clients
× missed events/client
× replay payload size
+ current live traffic
```

Bound replay concurrency, page size and per-client work. Consider jitter/backoff and snapshot reset when history gaps are too large.

## 6. File descriptors and sockets

Long-lived connections consume OS/runtime resources even when idle. Load tests and deployment sizing account for listener/socket file descriptors, TLS/proxy resources, kernel socket memory and any ingress connection limits.

Do not raise OS limits as the first fix for leaked/idle connections.

## 7. Heartbeat budget

Heartbeat rate contributes:

```text
connections × heartbeat frequency
```

messages plus wakeups/syscalls/TLS/proxy traffic. Choose the slowest interval that meets intermediary/liveness requirements and measure large-connection effects.

## 8. Serialization budget

Measure whether payload serialization is:

- once per event and reused safely;
- once per subscriber due personalization;
- compressed per connection;
- cloned as structured Rust values before serialization.

Do not introduce shared pre-serialized buffers until content/auth/personalization semantics make reuse safe and profiling shows value.

## 9. Downstream fan-out

One live subscription should not trigger unbounded polling/queries per connection. Prefer shared application projections/event producers where many clients observe the same state.

Smell:

```text
5000 SSE clients
→ each polls SQLite every 100 ms
```

Prefer:

```text
one/bounded projection owner
→ state/event update
→ fan-out to subscribers
```

## 10. Tail latency and lag

Measure:

```text
event committed/created timestamp
→ queued
→ serialized
→ socket write / client observe
```

where instrumentable. Track p50/p95/p99 and lag/resync rates under slow-client mixes. Average latency can hide queue collapse.

## 11. Admission and overload

Define target capacity and behavior above it:

```text
connection cap reached → reject new connection
subscriber queue saturated → coalesce/drop/disconnect/replay policy
replay concurrency saturated → bounded wait/reject/backoff
command concurrency saturated → reject/queue within bound
```

Never transition silently to unbounded memory.

## 12. Benchmark matrix

A useful minimum matrix varies:

- connections: low / expected / high;
- payload: small / representative / large bounded;
- event rate: idle / expected / burst;
- slow clients: 0% / representative / stress share;
- reconnect: steady / burst;
- replay: none / short gap / maximum supported gap.

Record CPU, RSS, network bytes, delivery percentiles, lag/drop/resync, queue saturation and error rate.

## 13. Forge-specific metrics

For future Forge SSE projection measure separately:

- Activity/Goal/DAG latest-state delivery delay;
- terminal execution/verification event resume correctness;
- Workspace subscriber count;
- broadcast/watch/replay queue depth and lag;
- SQLite replay query latency and rows/bytes;
- reconnect catch-up after backend restart;
- effect on existing execution/runtime/SQLite latency.

Do not optimize SSE throughput at the expense of Forge execution/evidence authority.
