# Realtime Transport Selection

Non-normative decision reference.

## Default decision ladder

Choose the simplest transport that satisfies the contract:

| Requirement | Default |
| --- | --- |
| server → browser live status/progress | SSE |
| server → browser semantic event feed with resume | SSE + event IDs/replay |
| ordinary client mutation + live result | HTTP command + SSE |
| one streaming HTTP response | HTTP body stream / SSE depending semantics |
| continuous bidirectional commands/events | WebSocket |
| terminal/PTY-style interactive session | WebSocket |
| binary duplex protocol | WebSocket or protocol-native transport |
| internal owned task queue | bounded Tokio channel, not network transport |

## Prefer SSE when

- server is the primary producer;
- browser/client only needs subscriptions and automatic reconnect;
- normal HTTP authorization, routing and observability are useful;
- text/event-stream representation is sufficient;
- connection state is disposable and replay comes from an application/durable cursor.

## Prefer HTTP command + SSE over WebSocket when

Client messages are ordinary application commands such as approve, stop, update settings, start work or change selection. Keeping commands on normal HTTP preserves request/response status, retries, idempotency and independent authorization, while SSE projects live state/events.

## Prefer WebSocket when

- both sides send frequently after one handshake;
- interaction is session-oriented rather than independent HTTP commands;
- terminal/editor/collaboration semantics need continuous duplex traffic;
- protocol-specific message framing/subprotocol negotiation is useful;
- connection identity/session state is genuinely part of the transport contract.

## Do not choose by mythology

Avoid:

- “WebSocket is always faster, therefore use it”;
- “SSE cannot scale” without infrastructure/load evidence;
- “real-time means bidirectional”;
- “one socket is simpler” when it creates custom command acknowledgements, auth refresh, retries and reconnect semantics that normal HTTP already solves.

## Infrastructure review

For either transport verify:

- reverse proxy/CDN supports long-lived connections;
- idle/read/write timeouts fit heartbeat policy;
- deployment drain/rolling restart behavior is understood;
- connection limits and file descriptors are capacity-budgeted;
- TLS termination and trusted proxy headers are defined.

For SSE additionally verify buffering is disabled/controlled on intermediaries where immediate delivery is required.

For WebSocket verify HTTP Upgrade/CONNECT support and Origin/auth policy for browser clients.

## Forge recommendation

Use SSE for live Activity, Goal/DAG, Agent, Queue, Writer Lease and Verification projections. Keep mutations as HTTP/application operations. Add WebSocket only for true interactive duplex features such as a future terminal/session stream or collaborative command channel.
