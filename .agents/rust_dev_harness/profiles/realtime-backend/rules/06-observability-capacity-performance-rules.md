# Realtime Observability Capacity and Performance Rules

## RT-OBS-001 — Connection lifecycle is observable [MUST]

Record bounded structured signals for admitted/rejected/opened/closed connections, normal versus abnormal termination, duration, transport type, authenticated scope and safe close/error category. Do not rely only on the initial HTTP upgrade/access log.

## RT-OBS-002 — Saturation is observable [MUST]

Expose enough telemetry to diagnose connection count, active subscribers, queue depth/capacity, lag/drop/resync count, send wait time, slow-client disconnects, inbound command rate, rejected admission and replay backlog.

## RT-OBS-003 — High-cardinality identities stay out of metric labels [MUST]

Workspace/user/session/connection IDs may belong in logs/traces where needed, but avoid arbitrary IDs/topic names/message contents as metric labels. Use bounded categories and aggregate counts for metrics.

## RT-OBS-004 — Payload logging is opt-in [MUST]

Do not log full event/message payloads by default. If payload capture is required for diagnostics, bound size, redact secrets/private data and define retention/access.

## RT-OBS-005 — Capacity is modeled end-to-end [MUST]

Capacity review includes:

```text
connection count
× per-connection state/buffers
+ live fan-out buffer
+ replay query/cache cost
+ serialization allocations
+ heartbeat traffic
+ downstream pool/fan-out work
```

Do not size only the Tokio channel and call the system bounded.

## RT-OBS-006 — Tail latency matters [MUST]

For real-time paths, measure delivery delay/queue wait and relevant percentiles under realistic concurrency, not only messages/sec or average latency.

## RT-OBS-007 — Heartbeats are excluded/classified correctly [SHOULD]

Do not let heartbeat/keepalive traffic distort business event throughput or message rate alerts. Track heartbeat failures/RTT separately when used for liveness.

## RT-OBS-008 — Replay performance is separate from live-tail performance [MUST]

Measure reconnect catch-up/history scanning independently from steady live fan-out. A fast live channel can still collapse when many clients replay large histories after restart.

## RT-OBS-009 — Serialization is measured before custom optimization [MUST]

Profile payload encoding/cloning/compression before introducing shared-byte caches, custom codecs, unsafe buffers or manual framing. Optimize the actual dominant path.

## RT-OBS-010 — Performance evidence names semantics [MUST]

Any benchmark/load result states transport, payload size, delivery guarantee, connection count, event rate, queue policy, persistence/replay involvement, machine/runtime config and failure/lag behavior. A benchmark that silently drops slow consumers is not comparable to a replayable stream.

## RT-OBS-011 — Connection churn is a first-class workload [SHOULD]

Where clients reconnect frequently, load tests include connect/auth/subscribe/close churn, not only steady-state established connections.

## RT-OBS-012 — Capacity breach has a safe outcome [MUST]

When limits are reached, behavior is explicit: reject new connection/subscription, shed/drop according to stream semantics, disconnect slow client, or require replay. Do not transition into hidden memory growth.

## Evidence

Capacity/performance claims require representative load or stress tests plus runtime telemetry sufficient to confirm boundedness, lag behavior and recovery after saturation.
