# Realtime Architecture

Non-normative reference. Governing Realtime Rules remain authority.

## 1. Separate state, events, live tail and transport

Preferred conceptual shape:

```text
Domain/Application
      │
      ├─ durable state / revision owner
      ├─ durable event history when required
      │
      └─ live projection/fan-out
               │
        ┌──────┴──────┐
       SSE         WebSocket
   server→client    bidirectional
```

Transport connections are replaceable projections. They do not become domain storage, workflow state or authorization authority simply because they are long-lived.

## 2. State projection versus event stream

Latest-state stream:

```text
state rev 10
state rev 11
state rev 12
slow client sees rev 12
```

Appropriate for progress, status dashboards, queue counts and replaceable projections.

Event stream:

```text
approval.created
execution.started
execution.completed
```

Each transition can matter independently. If loss is unacceptable, pair live delivery with a replay source and stable cursor.

## 3. Snapshot + tail

For a live UI with substantial current state:

```text
GET snapshot @ revision R
        │
        └──── subscribe/replay events > R
                         │
                         └──── live tail
```

The revision/cursor is the gap-closing boundary. Timing alone is not sufficient.

## 4. Command + event separation

For ordinary application operations:

```text
HTTP POST/PUT command
       │
       ▼
Application operation
       │
       ├─ typed immediate response
       └─ durable/live state/event → SSE
```

This is usually simpler than forcing commands into WebSocket. WebSocket becomes valuable when the session itself is continuously interactive and bidirectional.

## 5. Internal fan-out patterns

Latest state:

```text
application projection owner
→ tokio::watch
→ many connection adapters
```

Live lossy event fan-out:

```text
application event producer
→ bounded broadcast
→ subscribers
→ explicit Lagged handling
```

Reliable resumable events:

```text
transaction/state transition
→ durable event/journal
→ live broadcaster
→ subscribers

reconnect cursor
→ durable replay
→ live broadcaster tail
```

Do not use broadcast retention as a durable log.

## 6. Per-connection writer ownership

A robust connection shape:

```text
connection owner
├─ inbound reader (WS only)
├─ outbound writer
├─ subscription/feed task
├─ cancellation token
└─ close/join policy
```

Slow outbound writes must not accumulate unlimited messages. The feed/writer boundary has a bounded/coalesced/disconnect policy.

## 7. Failure model

Plan for:

- client refresh/tab close;
- network loss;
- proxy idle timeout;
- server restart;
- subscriber lag;
- replay store delay/unavailability;
- auth/session expiry;
- task panic/error;
- graceful deployment shutdown;
- overloaded connection admission.

Every failure maps to either reconnect/resync, safe close, bounded retry or explicit unavailable state.

## 8. Forge-oriented topology

For Forge/Orchestration live UI:

```text
Workspace durable truth
Activity / Goal / DAG / Agent / Verification revisions
                    │
             projection owner
          ┌─────────┴─────────┐
        watch              event log/live bus
 latest replaceable        semantic transitions
        │                       │
        └────────── SSE ────────┘

HTTP commands → application/control owners
```

Candidate latest-state data:

- selected Workspace summary;
- current Goal/DAG projection;
- queue counts;
- Agent status;
- auto-approval remaining budget.

Candidate durable/resumable events:

- DAG node lifecycle transition;
- execution terminalization;
- approval creation/decision;
- Writer Lease acquire/release/expiry;
- verification run terminal disposition.

Interactive terminal/control, if added, is a separate WebSocket-capable session rather than a reason to move all UI state into WebSocket.

## 9. Anti-coupling rule of thumb

If deleting SSE/WebSocket transport requires rewriting domain state transitions, transport ownership leaked inward. If replacing broadcast with another fan-out mechanism changes durable event meaning, live delivery became authority accidentally.
