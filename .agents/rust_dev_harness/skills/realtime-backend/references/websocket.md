# WebSocket Engineering

Non-normative WebSocket reference. Governing Realtime and Axum transport rules remain authority.

## 1. Fit

Use WebSocket when the application needs a genuinely continuous bidirectional session: terminal/PTY traffic, collaborative editing/control, high-frequency command/event exchange, or another protocol whose interaction is connection-oriented rather than independent HTTP requests.

Do not move ordinary CRUD/approval/start/stop commands into WebSocket solely to reduce HTTP request count.

## 2. Axum handshake

Axum 0.8.x exposes `WebSocketUpgrade` and `WebSocket`. For HTTP/1.1 the upgrade is GET-based; newer HTTP versions use CONNECT semantics, so Axum documents `routing::any` when one endpoint must support both.

Subprotocol negotiation is explicit through `protocols(...)` when the application defines one.

## 3. Review framework defaults

Axum 0.8.x defaults include a 128 KiB read buffer, 64 MiB maximum message size and 16 MiB maximum frame size. These are framework defaults, not recommended limits for every application.

For small JSON/control messages, select substantially smaller workload-appropriate bounds. For binary/terminal streams, choose limits from actual protocol requirements and capacity measurements.

Also review write-buffer limits and compression/extensions where used.

## 4. Browser authentication constraints

The native browser `WebSocket` constructor exposes URL plus optional subprotocols, not an arbitrary custom-header configuration surface.

Choose browser authentication deliberately, such as:

- existing secure same-origin/session cookie with CSWSH defenses;
- a short-lived connection ticket obtained over authenticated HTTP and validated once;
- application-level authentication after upgrade with strict pre-auth limits and timeout;
- another reviewed client/deployment-specific mechanism.

Do not put durable credentials in query strings by convenience; URLs are commonly logged and surfaced in diagnostics.

## 5. Origin validation

For browser-exposed WebSocket, validate the handshake `Origin` against an explicit trusted allowlist when cross-origin use is not intended. Origin is a browser CSWSH defense, not proof of identity for arbitrary non-browser clients.

Authentication and message-level authorization remain required according to the application trust model.

## 6. Typed message protocol

Prefer a bounded tagged envelope:

```rust
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Subscribe { workspace_id: WorkspaceId },
    Ack { event_id: EventId },
    Command { command_id: CommandId, payload: CommandPayload },
}
```

Validate every field and reject unknown/unsupported operations according to the protocol compatibility policy. Do not deserialize directly into privileged domain commands without an authorization/application boundary.

## 7. Inbound rate/concurrency

One established socket can send indefinitely. Bound:

```text
message bytes
messages per policy window
concurrent command executions
subscription count
fan-out per command
```

Do not let WebSocket bypass resource/admission controls that normal HTTP endpoints would receive.

## 8. Reader/writer ownership

Axum allows splitting the socket into sender/receiver halves for concurrent work. Treat this as implementation only:

```text
connection owner
├─ reader task
├─ writer task
├─ application/feed task
└─ cancellation token
```

When one critical half exits, decide whether siblings cancel/drain and then join them. Do not spawn both halves and forget their handles.

## 9. Outbound backpressure

A bounded server-side queue sits in front of the writer when decoupling producers from network writes. Select slow-client behavior from stream semantics: coalesce, backpressure, disconnect/resync, or durable replay.

The browser `WebSocket` API itself does not provide receive-side backpressure, so a fast server can also overwhelm a browser application. Capacity/load tests should include slow clients, not only fast loopback consumers.

## 10. Ping/Pong and liveness

RFC 6455 defines Ping/Pong control frames for keepalive/responsiveness. Application liveness policy still decides:

- who sends heartbeat;
- heartbeat interval;
- Pong/last-activity tracking if needed;
- idle versus dead distinction;
- timeout/close behavior.

Do not send application JSON "ping" messages when protocol Ping/Pong is sufficient unless the application needs semantic heartbeat data.

## 11. Close semantics

Select close behavior appropriate to normal shutdown, policy violation, protocol error, oversized input, planned restart or temporary unavailability. Use protocol-defined codes where applicable.

Keep close reasons short and do not expose internal database paths, stack traces, secrets or sensitive security details. After close starts, stop normal message processing and coordinate sibling task shutdown.

## 12. Reconnect

WebSocket does not define application resume. On abnormal closure clients should use randomized/backoff reconnect behavior rather than tight loops. Resume cursor/subscriptions/authentication are application protocol concerns.

Mutating commands need command IDs/idempotency if reconnect can repeat a command whose result was lost.

## 13. Compression

WebSocket extensions such as per-message compression can reduce bandwidth but increase CPU/memory and may interact with secret-reflection side channels. Enable only when workload/security analysis justifies it and measure the actual payload mix.

## 14. Testing

Test, as applicable:

```text
handshake and selected subprotocol
unauthorized Origin/auth
message/frame oversize
unknown/malformed message
message-rate/concurrency limit
subscription authorization
reader/writer sibling termination
Ping/Pong/idle behavior
normal and abnormal close
slow client/outbound queue saturation
server shutdown
reconnect + duplicate command
```

## 15. Forge guidance

Do not use WebSocket for the initial Forge live dashboard migration. Prefer HTTP JSON commands + resumable/coalesced SSE. Introduce WebSocket as a separately reviewed capability if Forge later owns an interactive terminal or other true duplex session; it must reuse existing application/execution authority rather than becoming a new command policy owner.
