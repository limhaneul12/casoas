# Realtime Reconnect and Replay

Non-normative implementation reference. Governing Realtime Resume/Replay Rules remain authority.

## 1. Reconnect is normal, not exceptional

Long-lived connections end because of browser navigation, sleep/wake, mobile network changes, proxy idle timeout, deployment restart and transient transport failure. Design recovery before calling a stream reliable.

## 2. Stable cursor model

Use a cursor tied to the event/revision authority, for example:

```text
workspace_id + monotonic revision
aggregate_id + event sequence
append-log offset
opaque durable cursor with server validation
```

Avoid using a connection-local sequence or wall-clock timestamp as a total order without proving uniqueness/order.

## 3. Snapshot + replay + live tail

Robust pattern:

```text
1. read snapshot at revision R
2. subscribe to live source or reserve tail position
3. query replay events > R up to live boundary
4. emit replay in order
5. switch to live tail
```

The exact ordering of subscribe versus replay depends on the event source, but the implementation must close the race between snapshot and live subscription.

## 4. Duplicate boundary events

Assume a reconnect can repeat the last event because the client may disconnect after the server wrote bytes but before the client application persisted/observed them.

Clients should use event IDs/revisions for idempotent application. Mutating command handlers need separate command deduplication/idempotency.

## 5. Gap and retention handling

If cursor C predates retained history:

```text
C < oldest_retained_cursor
→ explicit resync-required / snapshot-required response
```

Do not silently resume at “now” if the client expects a complete stream.

## 6. Bounded replay

Use page/window/byte/time limits. For large gaps:

```text
cursor C
→ page 1
→ next cursor
→ page 2
...
→ live tail
```

Do not `fetch_all` an unbounded history into one connection buffer.

## 7. SSE

Browser `EventSource` tracks the last event ID from SSE `id:` fields and sends it back as `Last-Event-ID` during reconnect according to the SSE protocol. Server-side mapping should validate and authorize that cursor before replay.

`retry:` is reconnect delay guidance, not a delivery acknowledgement and not proof the event was consumed.

Keepalive comment/events should not advance semantic event IDs.

## 8. WebSocket

WebSocket itself does not define application resume/replay. Define a handshake/resume message such as:

```json
{"type":"resume","cursor":"..."}
```

or include the cursor in authenticated connection setup. Validate it exactly like an HTTP/SSE replay cursor.

On abnormal closure, clients should use backoff/jitter rather than immediate tight reconnect loops.

## 9. Authorization during replay

Replay must use current authorization rules or the project's explicit historical-visibility contract. A known cursor/event ID is never sufficient authorization.

If access is revoked while disconnected, reconnect must not replay previously accessible data solely because the cursor is valid.

## 10. Failure matrix

Test:

```text
disconnect before event
server send then immediate disconnect
duplicate last event
expired cursor
future/invalid cursor
cross-workspace cursor
server restart
slow replay store
live events arriving during replay
replay page limit
```

## 11. Forge guidance

Forge already owns revisioned/durable Workspace semantics. Prefer existing durable revisions/evidence/event rows as resume authority instead of inventing a separate SSE-only sequence when a canonical revision can identify the same transition. Where no durable event sequence exists, create one at the application/persistence owner rather than inside the HTTP connection task.
