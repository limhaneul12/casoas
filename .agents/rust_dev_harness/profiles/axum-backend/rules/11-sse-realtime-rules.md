# Axum SSE Realtime Rules

These rules activate when Axum serves Server-Sent Events. The Rust Realtime Backend Profile is also required; it owns generic delivery/backpressure/lifecycle/replay semantics.

## AXUM-SSE-001 — Realtime baseline is active [MUST]

Axum SSE code follows Rust normal/type/async rules plus the Realtime Backend Profile. Do not define a second SSE-only model for queue ownership, replay or cancellation.

## AXUM-SSE-002 — SSE is server-push transport [MUST]

Use SSE for server → client live delivery. Ordinary application commands remain HTTP operations unless the application genuinely requires a bidirectional session.

## AXUM-SSE-003 — Keepalive is explicit [MUST for long-lived production streams]

Axum `Sse::new` has no keepalive by default. Configure `KeepAlive` when deployment/proxy idle behavior requires it, and document the interval rationale. Keepalive traffic must not advance the semantic event cursor.

## AXUM-SSE-004 — Event IDs map to canonical resume authority [MUST when resumable]

`Event::id` carries a stable application/durable cursor or revision when resume is promised. Do not generate connection-local IDs that cannot survive restart.

Incoming `Last-Event-ID` or equivalent cursor is parsed, bounded, authenticated/authorized and mapped to bounded replay before joining the live tail.

## AXUM-SSE-005 — `retry` is guidance only [MUST]

SSE `retry` may guide browser reconnect timing. It is not an acknowledgement, delivery guarantee or server-side backoff substitute.

## AXUM-SSE-006 — Native browser client auth is designed explicitly [MUST]

The standard browser `EventSource` constructor does not expose arbitrary request headers; it primarily supports URL plus credentialed-cookie mode. Choose same-origin/session credentials, reviewed credentialed CORS, fetch-stream style client, or another explicit client contract.

Do not put durable credentials in the event-stream URL merely to work around browser API limitations.

## AXUM-SSE-007 — Proxy buffering/idle behavior is part of production verification [MUST]

If the service is deployed behind a reverse proxy/CDN, verify immediate streaming, buffering policy, read/idle timeout and rolling-drain behavior. Local Axum tests alone do not prove end-to-end SSE latency.

## AXUM-SSE-008 — Stream source is bounded [MUST]

The stream fed to `Sse` has explicit latest-state, bounded fan-out or durable replay semantics. A per-client unbounded channel is prohibited under the Realtime rules.

## AXUM-SSE-009 — Disconnect releases subscriber work [MUST]

When the response future/stream is dropped because the peer disconnects or the server shuts down, connection-owned producer/subscription work terminates or is released promptly. Shared application producers may continue only because another owner requires them.

## AXUM-SSE-010 — Errors after headers require stream semantics [MUST]

Once SSE response headers are sent, later application failures cannot be turned into an ordinary HTTP status response. Decide whether to emit a typed terminal event, close and require reconnect/resync, or another stream-level outcome without leaking internal details.

## Evidence

Test SSE framing/content type, keepalive behavior, cursor/replay, slow consumer, disconnect cleanup, auth/cursor isolation and actual proxy behavior when that deployment layer is part of the claim.
