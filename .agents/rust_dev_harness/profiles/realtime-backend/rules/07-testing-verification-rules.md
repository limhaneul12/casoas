# Realtime Testing and Verification Rules

## RT-TEST-001 — Test semantics, not only transport connectivity [MUST]

A connection returning `101 Switching Protocols` or `200 text/event-stream` proves only admission/transport setup. Tests for real-time changes target delivery, ordering, lag, resume, auth, shutdown and resource semantics actually claimed.

## RT-TEST-002 — Deterministic in-process tests are preferred first [SHOULD]

Test channel/state/connection-owner logic without real sockets where possible. Add real Axum/server/client tests for protocol handshake, proxy-sensitive behavior, SSE framing, WebSocket close/ping/pong, connection metadata or interoperability claims.

## RT-TEST-003 — Slow consumers are exercised [MUST]

When buffering/fan-out changes, create a receiver/client slower than the producer and verify the declared behavior: backpressure, coalescing, lag error, disconnect, replay requirement or bounded drop.

## RT-TEST-004 — Disconnect timing is adversarial [MUST when resume/mutations are relevant]

Exercise disconnect before send, after server side effect but before acknowledgement, after event serialization, and during replay/live handoff where applicable. Verify duplicate/loss/idempotency semantics.

## RT-TEST-005 — Shutdown joins owned connection tasks [MUST]

Tests trigger service cancellation/shutdown while connections are idle and active, then prove new admission stops and owned reader/writer/producer tasks terminate within the declared budget or report an explicit failure.

## RT-TEST-006 — Authentication lifetime is tested [MUST when auth can change]

Test invalid admission plus expiry/revocation/permission change during a long-lived connection according to the project policy. A connection established with valid credentials is not assumed valid forever.

## RT-TEST-007 — Resource limits have negative tests [MUST]

Exercise maximum/oversized message or event, connection limit, message-rate/concurrency limit, replay bound and queue saturation. Verify safe rejection/close behavior and recovery after pressure.

## RT-TEST-008 — SSE resume behavior is tested with event IDs [MUST when resumable SSE is active]

Test `Last-Event-ID`/equivalent cursor mapping, replay after disconnect, duplicate boundary event, expired cursor/resync, keepalive not changing semantic cursor and proxy buffering configuration where deployment uses it.

## RT-TEST-009 — WebSocket control frames and close behavior are tested [MUST when WS is active]

Test negotiated subprotocol if used, Ping/Pong/liveness policy, normal close, peer close, protocol/policy/size errors, abrupt transport loss and sibling read/write task termination.

## RT-TEST-010 — Ordering is tested within the declared scope [MUST]

Do not assert stronger order than the contract. Use deterministic producers/revisions to prove per-workspace/entity/global ordering exactly where promised.

## RT-TEST-011 — Capacity/performance tests record semantics [MUST]

Load/stress tests record connections, payload sizes, event/message rate, queue capacities, slow-client policy, persistence/replay involvement, relevant percentiles, CPU/RSS and drop/lag/reconnect counts.

## RT-TEST-012 — Chaos/fault tests are conditional [SHOULD]

For high-value reconnect/failover streams, add controlled faults such as task cancellation, listener restart, DB/replay delay, network close or subscriber lag. Do not add complex chaos infrastructure when focused deterministic tests prove the required claim.

## Canonical verification ladder

```text
focused state/channel test
→ focused connection lifecycle test
→ transport integration test
→ cargo xtask rules/fmt/check
→ cargo xtask test
→ capability-specific reconnect/security/real-socket tests
→ cargo xtask ci
→ stress/fault/load/perf when claims require them
```

No unexecuted lane is PASS.
