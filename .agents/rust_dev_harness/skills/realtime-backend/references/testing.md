# Realtime Testing

Non-normative test design reference. Governing Shared/Rust/Realtime testing rules remain authority.

## 1. Test layers by claim

```text
state/application tests
→ delivery/reducer semantics

channel tests
→ boundedness, lag, coalescing, shutdown

connection-owner tests
→ cancellation, reader/writer lifecycle

transport tests
→ SSE framing / WS handshake and control frames

real network/proxy tests
→ ingress buffering, timeouts, connection behavior

load/fault tests
→ capacity, churn, reconnect storm, recovery
```

Use the narrowest truthful boundary. A real socket is not automatically stronger evidence for domain/event correctness.

## 2. Deterministic slow-consumer test

Construct a producer faster than a controlled receiver and assert the declared policy:

```text
producer fills bounded channel
→ receiver intentionally stalls
→ expected: sender waits / watch coalesces / broadcast Lagged / connection closes
→ memory/queue remains within bound
→ system recovers when receiver resumes or reconnects
```

Do not rely on sleeps alone when a barrier/semaphore/oneshot can coordinate deterministically.

## 3. Broadcast lag

For a small test capacity:

```text
subscribe receiver
send > retention before receiver reads
→ assert RecvError::Lagged(n)
→ execute resync/drop/terminate policy
→ verify next cursor/state
```

If the stream promises no silent loss, verify that `Lagged` cannot be ignored.

## 4. Watch coalescing

Send several revisions before the receiver observes them and assert it sees the newest value and revision. Also prove that no code treats missing intermediate values as missing semantic events.

## 5. Snapshot + tail race

Coordinate exactly:

```text
read snapshot at R
pause
append R+1
establish replay/live boundary
append R+2
resume client
```

Assert R+1/R+2 are neither lost nor incorrectly applied beyond the duplicate policy.

## 6. Disconnect windows

Exercise:

- before event emission;
- after durable commit before live send;
- after server send before client acknowledgement/application;
- during replay;
- during switch from replay to live tail;
- during command execution before result acknowledgement.

These windows reveal duplicate/loss/idempotency errors that happy-path reconnect tests miss.

## 7. Lifecycle/shutdown

For each owned connection:

```text
start connection
start active reader/writer/feed
trigger service shutdown
assert new admission denied/stopped
assert children observe cancellation
assert protocol close/drain policy
join children
assert registry count returns to baseline
```

Test an idle connection separately from a busy connection when shutdown semantics differ.

## 8. Authentication tests

Test initial rejection and long-lived permission lifecycle where applicable:

```text
invalid credential
unauthorized stream/topic
permission revoked while connected
session expires
logout/revocation
reconnect using stale cursor after access removal
```

## 9. SSE tests

At service level verify:

- `text/event-stream` response;
- `event`, `id`, `data`, `retry` framing as used;
- keepalive does not advance event ID;
- `Last-Event-ID`/cursor validation and replay;
- malformed/expired/unauthorized cursor;
- disconnect cleanup.

At deployment level verify proxy buffering/idle timeout only when the actual ingress is part of the claim.

## 10. WebSocket tests

Verify:

- upgrade and subprotocol negotiation;
- invalid Origin/auth;
- message/frame size limits;
- malformed/unknown message;
- inbound rate/concurrency policy;
- Ping/Pong/idle behavior;
- normal close and abrupt transport loss;
- sibling reader/writer task cancellation/join;
- slow outbound consumer;
- duplicate mutating command after reconnect.

## 11. Real listener pattern

When actual network semantics matter:

```text
bind loopback ephemeral port
→ start owned server task
→ connect real client
→ execute scenario
→ trigger graceful cancellation
→ await server and client tasks
```

Avoid fixed ports for parallel tests. Do not leak server tasks after failures.

## 12. Load/capacity tests

Record:

```text
connections
connect/disconnect churn
payload bytes
send/receive rate
queue capacity/depth
slow-client share
replay window
CPU/RSS
p50/p95/p99 delivery delay
lag/drop/resync counts
failure/rejection rates
```

Do not compare two implementations unless delivery semantics and workload are equivalent.

## 13. Completion evidence

Report exact executed lanes:

```text
PASS channel lag test
PASS SSE resume test
PASS shutdown join test
NOT RUN real proxy lane — ingress unchanged
NOT RUN WebSocket — capability inactive
NOT RUN load test — no capacity/performance claim
```

No unexecuted lane is PASS.
