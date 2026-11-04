# SSE Engineering

Non-normative Server-Sent Events reference. Governing Realtime and transport-specific Axum rules remain authority.

## 1. Fit

SSE is a one-way server → client event stream over HTTP. Prefer it for live status, progress, notifications, event feeds and resumable UI projections where client mutations can remain ordinary HTTP commands.

Do not use SSE for a true continuously bidirectional session; pair normal HTTP commands with SSE or select WebSocket when duplex semantics are intrinsic.

## 2. Axum shape

Axum 0.8.x exposes `Sse<S>` over a `TryStream` of `Event` values. Keepalive is explicit; `Sse::new(stream)` does not enable keepalive by default.

Conceptual shape:

```rust
Sse::new(stream)
    .keep_alive(KeepAlive::default())
```

Choose the keepalive interval/content from proxy/network idle behavior rather than copying a universal value.

## 3. Event fields

Use SSE fields deliberately:

```text
event: stable event type when named dispatch helps
id: stable replay cursor/event identifier when resume matters
data: encoded payload
retry: reconnect delay hint only
comment: useful for semantic-free keepalive
```

`retry` is not an acknowledgement. A keepalive comment/event must not advance the semantic cursor.

## 4. Resume

When the stream is resumable:

```text
durable event/revision id
→ SSE event id
→ client reconnect Last-Event-ID
→ validate and authorize cursor
→ bounded replay
→ live tail
```

Do not use an in-memory connection sequence if restart-safe resume is promised.

## 5. Browser EventSource authentication caveat

Native browser `EventSource` construction accepts a URL and an optional `withCredentials` flag; it does not provide a general arbitrary-request-header options surface like a server-side HTTP client.

Therefore decide browser authentication explicitly:

- same-origin cookie/session;
- credentialed cross-origin cookie with a reviewed CORS/CSRF model;
- a fetch/streaming client or vetted wrapper when custom headers are required;
- another deployment-specific authenticated channel.

Do not place durable access credentials in query strings merely because native EventSource cannot set a custom authorization header. URL credentials can leak through logs, referrers and diagnostics unless a narrowly reviewed mechanism proves otherwise.

## 6. Connection admission

Authenticate and authorize subscription scope before returning the stream. If permission/session lifetime can change during the connection, define revalidation/revocation semantics under the common Realtime security rules.

## 7. Backpressure and slow clients

The source feeding each SSE response must have explicit bounded, coalesced or replay semantics. Network write backpressure must not grow an unbounded per-client queue.

For latest-state UI projections, `watch` or equivalent coalescing can be appropriate. For semantic events, use bounded live fan-out plus replay/resync when loss is unacceptable.

## 8. Proxy buffering

Reverse proxies may buffer upstream responses. Nginx `proxy_buffering` is on by default; buffering can be disabled by configuration or `X-Accel-Buffering: no` when the deployment permits that header behavior.

Treat immediate flush behavior as a deployment property and verify it end to end. A locally streaming Axum handler does not prove the production proxy emits events promptly.

Also review proxy/read/idle timeouts and rolling-deployment drain behavior for long-lived HTTP responses.

## 9. HTTP protocol and concurrent streams

Browser connection behavior differs between HTTP/1 and multiplexed HTTP/2 deployments. Do not assume protocol version or available concurrent streams; verify the actual ingress/client transport and required number of live subscriptions.

## 10. Cache/content semantics

Serve SSE with the correct event-stream content type and ensure intermediary/cache behavior cannot cache or transform the live event body incorrectly. Compression/proxy transforms are deployment-specific and should be reviewed rather than assumed harmless.

## 11. Disconnect behavior

A client disconnect is normal. The server-side stream/producer observes cancellation or closed output and releases subscriber state promptly. Do not continue expensive producer work after the only subscriber disappears unless another owner needs it.

## 12. Testing

Test, as applicable:

```text
correct content type and event framing
named event/data encoding
keepalive does not alter semantic cursor
Last-Event-ID resume
replay duplicate boundary
expired/unauthorized cursor
slow consumer behavior
disconnect cleanup
permission/session expiry
proxy buffering and idle timeout in deployment integration
server shutdown/drain
```

## 13. Forge pattern

Forge/Orchestration UI default:

```text
HTTP command → Forge application/control owner
                     │
                     ├─ latest projection → coalesced SSE
                     └─ durable transition → resumable SSE
```

Candidate latest projections: queue counts, Agent status, current Goal/DAG, progress. Candidate resumable transitions: execution/verification terminal state, approvals and Writer Lease lifecycle where clients must not silently miss a transition.
