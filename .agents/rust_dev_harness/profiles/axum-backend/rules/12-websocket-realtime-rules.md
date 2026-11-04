# Axum WebSocket Realtime Rules

These rules activate when Axum serves WebSocket. The Rust Realtime Backend Profile is also required and owns generic delivery/backpressure/lifecycle/replay semantics.

## AXUM-WS-001 — WebSocket requires true duplex justification [MUST]

Use WebSocket when continuous bidirectional/session-oriented traffic is a real requirement. Do not migrate ordinary HTTP commands and server-push status to WebSocket merely because it is called real-time.

## AXUM-WS-002 — Upgrade/auth policy is explicit [MUST]

Authenticate and authorize the connection/subscription scope before granting privileged long-lived access according to the client/deployment model. Browser connections additionally validate `Origin` against a trusted allowlist when cross-origin access is not intended.

## AXUM-WS-003 — Native browser auth limitations are considered [MUST]

The browser `WebSocket` constructor provides URL and optional subprotocols, not arbitrary custom request headers. Select a reviewed cookie/session, short-lived connection ticket, application-level auth handshake, or other explicit mechanism. Do not leak durable credentials through URL query parameters by convenience.

## AXUM-WS-004 — Framework size/buffer defaults are reviewed [MUST]

Axum 0.8.x WebSocket defaults include a 128 KiB read buffer, 64 MiB max message size and 16 MiB max frame size. These are not workload recommendations. Set message/frame/write-buffer limits appropriate to the actual protocol and capacity model.

## AXUM-WS-005 — Subprotocol negotiation is typed contract [MUST when used]

When `Sec-WebSocket-Protocol` is used, enumerate supported protocols deliberately, select only an expected protocol and test negotiation/failure. Do not overload subprotocol strings as an unreviewed credential transport.

## AXUM-WS-006 — Inbound messages are typed untrusted input [MUST]

Decode into bounded tagged transport messages; validate size, message type, fields and authorization before invoking application operations. Unknown/invalid messages follow a deliberate protocol error/close response.

## AXUM-WS-007 — Inbound rate and work concurrency are bounded [MUST]

One connected socket cannot bypass application admission controls. Bound message rate, simultaneous command work, subscription count and fan-out. Do not spawn one unrestricted task per inbound message.

## AXUM-WS-008 — Reader/writer halves are supervised [MUST]

Axum supports split read/write processing, but both halves remain children of one connection owner. Define sibling failure, cancellation, close/drain and join semantics; do not detach meaningful tasks.

## AXUM-WS-009 — Outbound slow-client policy is explicit [MUST]

If producers and the socket writer are decoupled, use bounded/coalesced/replayable delivery according to the Realtime profile. No unbounded per-client outbound queue.

## AXUM-WS-010 — Ping/Pong and idle semantics are deliberate [MUST when heartbeat is used]

Use RFC WebSocket control frames when transport heartbeat is sufficient. Define heartbeat sender, activity/Pong observation, idle/dead distinction and timeout/close policy. Application-level heartbeat messages require a separate semantic reason.

## AXUM-WS-011 — Close semantics are safe and intentional [MUST]

Use appropriate protocol/policy/size/restart/unavailable close behavior and stop normal message processing after close begins. Close reasons remain short and do not expose internal error detail or secrets.

## AXUM-WS-012 — Reconnect/resume/idempotency are application concerns [MUST]

WebSocket transport does not supply durable replay. Abnormal reconnect uses backoff/jitter. If session state/events resume, validate a canonical cursor; if commands may repeat, use command ID/idempotency/deduplication according to the application contract.

## AXUM-WS-013 — Classic browser client backpressure is not assumed [MUST]

Do not assume the browser `WebSocket` API will backpressure inbound data to protect the client application. Server capacity tests include slow consumers and bounded sending behavior.

## Evidence

Test handshake/auth/Origin, subprotocol if used, size/rate limits, typed invalid messages, reader/writer lifecycle, Ping/Pong/idle, close, slow client, shutdown and reconnect/duplicate command semantics where applicable.
