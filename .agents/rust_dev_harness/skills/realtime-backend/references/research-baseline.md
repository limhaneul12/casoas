# Realtime Research Baseline — 2026-09-11

Research/provenance reference for the Rust Realtime Backend Profile. This document records why the rules and procedures exist; it is not a parallel Rule source.

## 1. Authority order

```text
IETF / WHATWG protocol specifications
→ official Axum/Tokio/Tower documentation and source
→ browser platform documentation (MDN/WHATWG)
→ deployment/server documentation
→ OWASP security guidance
→ community patterns only as secondary hypotheses
```

Repository-local versions, source, deployment and tests remain concrete authority for a project.

## 2. Research baseline

Verified at research time:

- Axum `0.8.9` current docs.
- Tokio `1.53.1` current docs.
- WebSocket wire protocol: RFC 6455.
- Server-Sent Events: HTML/EventSource event-stream semantics.

The Realtime Profile intentionally avoids pinning protocol-independent rules to one framework version.

## 3. Axum SSE findings

Official Axum `0.8.9` SSE docs expose `Sse<S>` over a `TryStream` of `Event` and a `keep_alive` option. Keepalive defaults to **disabled** unless configured.

Primary references:

- https://docs.rs/axum/latest/axum/response/sse/
- https://docs.rs/axum/latest/axum/response/struct.Sse.html

Harness consequences:

- keepalive is an explicit deployment/lifecycle decision;
- the event source feeding `Sse` must own bounded/loss/replay semantics;
- framework streaming does not itself provide durable resume.

## 4. SSE event ID and reconnect findings

Browser EventSource/event-stream semantics define:

- `id` updates the EventSource last-event ID;
- `retry` provides reconnect delay guidance;
- SSE is server → client one-way delivery;
- comments can be used as semantic-free keepalive traffic.

Primary reference:

- https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events

Harness consequences:

- durable/resumable streams map event IDs to canonical durable cursors/revisions;
- `retry` is not an acknowledgement;
- keepalive does not consume/advance a semantic event cursor.

## 5. Native browser EventSource authentication limitation

The standard browser `EventSource` constructor accepts:

```text
EventSource(url)
EventSource(url, { withCredentials })
```

It does not expose a general arbitrary-header options map.

Primary references:

- https://developer.mozilla.org/en-US/docs/Web/API/EventSource/EventSource
- https://developer.mozilla.org/en-US/docs/Web/API/EventSource/withCredentials

Harness consequences:

- browser SSE authentication is an explicit client/deployment contract;
- same-origin/session-cookie, credentialed CORS or fetch-stream style clients are evaluated deliberately;
- durable credentials are not moved into URL query parameters merely for convenience.

## 6. Reverse-proxy buffering

Nginx proxy documentation states `proxy_buffering` defaults to `on`; buffering can be disabled by config, and response header `X-Accel-Buffering: no` can influence it unless ignored by configuration.

Primary reference:

- https://nginx.org/en/docs/http/ngx_http_proxy_module.html

Harness consequence:

Local Axum streaming is not end-to-end delivery evidence. Production SSE verification includes the actual ingress/proxy buffering and idle/read timeout configuration when relevant.

## 7. Axum WebSocket findings

Axum `0.8.9` `WebSocketUpgrade` exposes configurable buffers and limits. Current defaults include:

- read buffer: 128 KiB;
- max message size: 64 MiB;
- max frame size: 16 MiB;
- unmasked client frames rejected by default.

For HTTP/1.1 the extractor uses GET upgrade semantics; Axum documents route handling that can support later HTTP CONNECT-based semantics as well.

Primary references:

- https://docs.rs/axum/latest/axum/extract/struct.WebSocketUpgrade.html
- https://docs.rs/axum/latest/axum/extract/ws/

Harness consequences:

- framework defaults are reviewed, not treated as recommended application limits;
- control-message services normally select smaller workload-derived bounds;
- subprotocol/upgrade behavior is part of the public transport contract when used.

## 8. WebSocket protocol lifecycle

RFC 6455 defines two-way messaging, Ping/Pong control frames, closing handshake/status codes and abnormal-closure reconnect guidance. It specifically warns that immediate persistent reconnect after widespread abnormal closure can contribute to denial of service and recommends randomized/increasing backoff.

Primary reference:

- https://www.rfc-editor.org/rfc/rfc6455.html

Harness consequences:

- heartbeat/idle behavior uses protocol control semantics where appropriate;
- clean close is deliberate;
- reconnect storms are bounded with jitter/backoff;
- transport reconnect does not define application replay or command idempotency.

## 9. Browser WebSocket API findings

The stable browser WebSocket constructor accepts URL plus optional subprotocols. It does not expose arbitrary custom request headers.

MDN also notes that the classic browser `WebSocket` interface does not provide receive-side backpressure: when messages arrive faster than the application can process them, buffering/CPU/memory problems can result.

Primary references:

- https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/WebSocket
- https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API

Harness consequences:

- browser authentication must be designed rather than assumed to use arbitrary headers;
- server load tests include slow/browser-like consumers and do not assume client API backpressure protects them.

## 10. WebSocket security findings

OWASP WebSocket guidance highlights:

- WSS/TLS for production sensitive traffic;
- explicit Origin allowlist for browser handshakes;
- authentication plus message/action-level authorization;
- validation of every inbound message;
- connection, message-size and rate limits;
- idle/heartbeat cleanup;
- backpressure controls;
- long-lived session expiry/revocation;
- security/event logging without sensitive full-payload logging.

Primary reference:

- https://cheatsheetseries.owasp.org/cheatsheets/WebSocket_Security_Cheat_Sheet.html

Harness consequence:

The profile encodes the invariants but intentionally does **not** copy OWASP example numbers as universal limits. Capacity and rate values remain threat/workload/project decisions.

## 11. Tokio bounded mpsc

Tokio bounded `mpsc` has finite capacity; senders wait when the buffer is full, providing backpressure. Tokio's unbounded channel has no backpressure and may buffer arbitrarily until system memory is exhausted.

Primary references:

- https://docs.rs/tokio/latest/tokio/sync/mpsc/
- https://docs.rs/tokio/latest/tokio/sync/mpsc/fn.unbounded_channel.html

Harness consequences:

- bounded queues are the production default for producer/consumer handoffs;
- unbounded channels require an external proof of bounded production/lifetime and are not a normal network buffer.

## 12. Tokio broadcast

Tokio `broadcast` is a bounded fan-out channel. When capacity is exceeded, the oldest retained values are overwritten for lagging receivers; those receivers observe `RecvError::Lagged(n)` and may then continue from the oldest retained value.

Primary reference:

- https://docs.rs/tokio/latest/tokio/sync/broadcast/

Harness consequences:

- `broadcast` is live fan-out, not durable replay;
- lag is observable and must map to tolerate/drop/resync/replay/disconnect policy;
- a slow receiver is not automatically disconnected.

## 13. Tokio watch

Tokio `watch` retains only the most recently sent value and notifies receivers of changes. Intermediate states may disappear before observation.

Primary reference:

- https://docs.rs/tokio/latest/tokio/sync/watch/

Harness consequence:

Use it for replaceable latest-state projections such as status/progress/config, not for semantic events that each need independent observation.

## 14. Cancellation / shutdown

Existing Rust async Harness rules already govern structured task ownership, cancellation, bounded concurrency and graceful shutdown. The realtime profile specializes them for long-lived connection owners rather than duplicating Tokio rules.

Relevant references:

- https://tokio.rs/tokio/topics/shutdown
- https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html

Harness consequence:

Reader/writer/feed tasks belong to a connection/service owner; a cancellation signal must be observed and owned tasks joined. Timeout does not prove external/blocking side effects were cancelled.

## 15. Decisions deliberately not universalized

The research does not justify one global value or choice for:

- SSE versus WebSocket in every application;
- heartbeat interval;
- idle timeout;
- per-connection queue capacity;
- WebSocket max message/frame size;
- connection count;
- messages/second;
- replay retention window;
- global versus per-aggregate event order;
- authentication transport;
- event store technology;
- compression;
- reverse proxy/vendor.

These are declared project/profile decisions with measurable evidence.

## 16. Forge-specific research synthesis

Forge's current Project Profile already states:

- Tokio multi-thread runtime;
- latest-value/coalesced UI projections;
- all control/UI handoffs bounded;
- owned task/process shutdown;
- durable Workspace/Activity/Goal/DAG/Agent/Verification authority;
- future HTTP JSON + resumable SSE backend planned but not yet complete.

Therefore the smallest consistent target is:

```text
existing durable Forge state/revisions/events
→ shared latest-state projection + durable replay source
→ bounded live fan-out
→ Axum SSE for server→UI delivery

HTTP JSON/application commands remain command path
```

WebSocket is not the default dashboard transport. It activates later only if Forge owns a true duplex interactive session such as terminal/control streaming.

## 17. Maintenance procedure

When updating this profile:

1. recheck released Axum/Tokio/Tower docs;
2. recheck browser SSE/WebSocket APIs and current ingress behavior;
3. recheck RFC/protocol/security guidance when protocol semantics evolve;
4. separate version-specific mechanics from stable delivery/ownership invariants;
5. update references first;
6. change normative rules only when their semantic premise changes;
7. never distort production architecture to satisfy a stale example or verifier.

Research date: **2026-09-11 (KST)**.
