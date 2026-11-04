# Realtime Security and Admission Rules

## RT-SEC-001 — Long-lived connections have explicit admission policy [MUST]

Authenticate/authorize before granting the long-lived capability appropriate to the deployment. Bound total connections and, where identity exists, per-principal/workspace/session connections. IP-only limits are fallback/threat-model dependent, not universal identity.

## RT-SEC-002 — Authorization is revalidated when its lifetime requires it [MUST]

A successful connection handshake does not grant permanent authority if roles, leases, sessions, tokens or policy can change during a long-lived connection. Define whether authorization is checked per command/event subscription, on token/session expiry, on policy revision, or another bounded trigger.

## RT-SEC-003 — Browser WebSocket Origin is validated [MUST when browser WS is exposed]

For browser WebSocket endpoints, validate `Origin` against an explicit trusted allowlist when cross-origin access is not intended. Origin validation complements authentication; it does not authenticate non-browser clients, which can forge Origin.

## RT-SEC-004 — Messages/frames/events are bounded [MUST]

Define message/frame/event payload bounds, aggregate multipart/binary limits where relevant, decode/decompression limits and per-message processing budgets. Framework defaults are reviewed rather than accepted blindly.

## RT-SEC-005 — Message rate and command concurrency are bounded [MUST]

Persistent clients cannot bypass normal HTTP admission controls by sending unlimited messages over one connection. Apply rate/concurrency/fan-out bounds to inbound commands and expensive subscription changes.

## RT-SEC-006 — Subscription scope is authorized [MUST]

A client may subscribe only to streams/resources it is authorized to observe. Topic/workspace/entity IDs are selectors, not capabilities. Prevent cross-tenant/workspace enumeration and replay leakage.

## RT-SEC-007 — Connection metadata is untrusted input [MUST]

Origin, forwarded IP, subprotocol, query string, resume cursor and client-supplied IDs are validated under their trust model. Do not use forwarded headers as identity without a configured trusted proxy boundary.

## RT-SEC-008 — Secrets are not streamed/logged by default [MUST]

Do not expose credentials, tokens, cookies, secret environment/config values or unrestricted internal debug payloads through live events, close reasons or routine connection/message logs.

## RT-SEC-009 — Binary/protocol decoders are defensive [MUST]

Treat every inbound message as untrusted. Use typed parsers, validate discriminants/schema/lengths, reject unsupported message types/subprotocols and avoid unsafe/eval-like dynamic execution paths.

## RT-SEC-010 — Compression is threat/workload reviewed [SHOULD]

Compression may trade bandwidth for CPU/memory and can create side-channel concerns when secrets and attacker-controlled reflections share compressed contexts. Enable protocol/message compression deliberately rather than by default habit.

## RT-SEC-011 — Failure paths shed load safely [MUST]

Overload, malformed flood, auth failure and slow-client handling close/reject with bounded work. Do not perform expensive logging, body capture, stack formatting or durable writes per malicious message without rate/budget controls.

## RT-SEC-012 — Command replay/idempotency is explicit [MUST]

For mutating WebSocket/stream commands, define command IDs/idempotency keys/sequence fences or another deduplication model where retries/reconnects can repeat messages. Read-only event delivery and mutating command semantics are distinct.

## Evidence

Security tests include unauthorized origin where applicable, missing/expired/revoked auth, cross-scope subscription attempts, oversized messages/events, message flooding, connection limits, replayed mutation commands and secret-free logs/errors.
