---
name: realtime-backend
description: Production Rust real-time and streaming engineering methodology for live state/event delivery, SSE, WebSocket and long-lived connection systems. Applies the Rust async baseline plus the Realtime Backend Profile, with explicit delivery semantics, bounded backpressure, lifecycle ownership, resume/replay, security and capacity verification.
version: 1.0.1
researched_at: 2026-09-11
---

# Realtime Backend Engineering

Procedure for real-time work under governing Rust + Realtime Rules. Rules are normative authority; this Skill does not redefine policy.

Profile → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/profiles/realtime-backend/README|Rust Realtime Backend Profile]]

## 1. Load authority

Read:

1. closest repository `AGENTS.md` and project instructions;
2. `PROJECT_PROFILE.md`, Cargo/toolchain and runtime configuration;
3. Rust normal + type + async rules;
4. Realtime Backend Profile and relevant rule files;
5. transport profile such as Axum SSE/WebSocket when used;
6. current implementation, tests, persistence contracts and runtime evidence.

Do not add a new transport, channel or dependency before inspecting existing runtime and event/state owners.

## 2. Classify the real-time requirement

Name what must move and in which direction:

- latest state/progress;
- semantic events;
- resumable event history;
- notifications/log tail;
- streaming response;
- interactive commands;
- terminal/session/collaboration;
- internal worker stream.

Then state directionality:

```text
server → client
client → server
bidirectional
internal producer → consumers
```

## 3. Declare delivery semantics first

Choose one or combine deliberately:

```text
latest-state or coalesced
lossy live event
ordered live event
resumable or replayable event
bidirectional command + event
```

Define ordering scope, loss tolerance, duplicate tolerance, resume window and idempotency requirements before choosing SSE, WebSocket or channel primitives.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/architecture|Realtime Architecture]]

## 4. Find canonical state/event authority

Trace:

```text
state transition/effect
→ durable state/event owner
→ live projection/fan-out
→ transport adapter
```

Do not make a socket/session/channel the source of truth. Reuse the same application operation/events across HTTP, MCP, WebSocket and SSE instead of duplicating business logic.

## 5. Select transport with minimality

Default decision:

```text
server-push only, UI status, progress or event feed
→ SSE

ordinary command plus live result/state
→ HTTP command + SSE

continuous bidirectional interactive protocol
→ WebSocket

internal owned work queue
→ bounded mpsc / task owner
```

Do not choose WebSocket solely because the feature is called real-time.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/transport-selection|Realtime Transport Selection]]

## 6. Select Tokio primitive by semantics

```text
bounded mpsc   owned queue with backpressure
broadcast      live fan-out with bounded retention + explicit lag
watch          latest-value state only
oneshot        one completion/result
CancellationToken cooperative cancellation
persistence    replay/durable history
```

For every channel record capacity and full/lag behavior.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/channel-selection|Realtime Channel Selection]]

## 7. Design connection ownership

Identify:

- admission owner;
- connection registry owner if any;
- reader, writer and producer task owners;
- cancellation token hierarchy;
- heartbeat/idle policy;
- slow-consumer policy;
- close/drain/join sequence;
- reconnect behavior.

Reader/writer split is implementation, not ownership. Do not leave both halves as detached tasks.

## 8. Close reconnect/replay gaps

If resume matters:

```text
snapshot/revision R
→ bounded replay after R
→ live tail
```

Define stable cursor/event ID, retention window, expired-cursor behavior, duplicate handling and authorization scope.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/reconnect-replay|Reconnect and Replay]]

## 9. Apply SSE procedure when selected

For Axum-based SSE:

- return `Sse` over a typed event stream;
- choose keepalive deliberately;
- use stable event IDs for resume when required;
- treat `retry` as reconnect guidance only;
- map `Last-Event-ID` to canonical cursor/replay logic;
- verify reverse-proxy buffering and idle timeouts;
- keepalive must not advance the semantic event cursor;
- choose browser authentication based on the actual EventSource/client contract.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/sse|SSE Engineering]]

## 10. Apply WebSocket procedure when selected

For Axum-based WebSocket:

- validate handshake, authentication, Origin and subprotocol according to the trust boundary;
- set message/frame/write-buffer bounds from workload instead of accepting defaults blindly;
- type and validate every inbound message;
- bound message rate and command concurrency;
- supervise read/write halves;
- define Ping/Pong and idle behavior;
- use deliberate close codes/reasons;
- keep reconnect and command deduplication with the application contract.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/websocket|WebSocket Engineering]]

## 11. Review security and resource multiplication

Calculate at least qualitatively:

```text
connections × per-connection buffers/state
+ live fan-out retention
+ payload serialization cost
+ replay backlog
+ heartbeat traffic
+ downstream work
```

Check connection limits, per-principal limits, message/event size, inbound rate, subscription authorization, replay bounds and secret-safe telemetry.

## 12. Test failure modes before performance tuning

Run focused deterministic tests for:

- slow consumer;
- lag/coalescing;
- peer disconnect;
- server cancellation/shutdown;
- reconnect + replay;
- duplicate boundary event/command;
- permission expiry/revocation where relevant;
- oversized/flooded message;
- connection/admission saturation.

Then add real socket/proxy/client tests only for claims requiring them.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/testing|Realtime Testing]]

## 13. Measure the actual workload

Before optimization measure:

- connections;
- event/message rate;
- payload bytes;
- queue wait/depth;
- delivery p50/p95/p99;
- lag/drop/resync;
- CPU/RSS;
- serialization cost;
- replay catch-up;
- downstream pool wait and fan-out.

Do not use hello-world WebSocket/SSE throughput as product evidence.

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/capacity-planning|Realtime Capacity Planning]]

## 14. Adversarial review

Before completion ask:

- Is any socket/channel acting as durable truth?
- Is any queue unbounded?
- What happens to the slowest client?
- Can one connection flood unlimited commands?
- Can reconnect duplicate a mutation?
- Can a lagged broadcast subscriber silently miss a required transition?
- Does snapshot + live subscription have a gap?
- Can permissions become stale on a long-lived connection?
- Do reader/writer tasks terminate together?
- Does shutdown stop new admission before drain?
- Can a proxy buffer SSE or kill idle connections?
- Are performance claims tied to actual delivery semantics?

Reference → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/anti-patterns|Realtime Anti-Patterns]]

## 15. Verification ladder

```text
focused state/event tests
→ channel/backpressure tests
→ lifecycle/reconnect tests
→ transport integration tests
→ cargo xtask rules
→ cargo xtask fmt
→ cargo xtask check
→ cargo xtask test
→ capability-specific security and network tests
→ cargo xtask ci
→ stress/fault/load/perf when required
```

No unexecuted gate is PASS.

Research baseline → [[Jobs/Harness/Engineering Harness/02 Rust/rust_dev_harness/skills/realtime-backend/references/research-baseline|Realtime Research Baseline]]
