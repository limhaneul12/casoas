# Realtime Anti-Patterns

Adversarial reference. Governing Rules remain authority.

1. **Real-time means WebSocket** — transport selected before directionality or delivery semantics.
2. **Socket as source of truth** — workflow/domain state exists only in connection memory.
3. **Per-client unbounded queue** — slow clients turn memory into the capacity policy.
4. **Broadcast equals durable history** — in-memory fan-out is treated as restart-safe replay.
5. **Ignoring `RecvError::Lagged`** — required events disappear silently.
6. **Watch used for semantic events** — intermediate transitions disappear.
7. **Snapshot then subscribe quickly** — no cursor fence closes the race.
8. **Wall clock as total-order ID** — uniqueness/order is assumed without proof.
9. **Timeout equals cancellation** — side effects continue after transport timeout.
10. **Detached reader/writer tasks** — connection children leak or outlive shutdown.
11. **One socket bypasses admission** — unlimited messages/tasks over one connection.
12. **Native browser auth assumptions** — EventSource/WebSocket is treated like a generic header-capable HTTP client.
13. **Credential in URL shortcut** — durable access material leaks through URL surfaces.
14. **No browser WebSocket Origin validation** — cookie-authenticated connection is exposed to cross-site initiation.
15. **Axum WebSocket defaults accepted blindly** — large framework defaults remain without workload review.
16. **Application heartbeat without need** — custom heartbeat duplicates protocol/transport liveness.
17. **Keepalive advances SSE cursor** — resume positions are polluted by heartbeat traffic.
18. **SSE retry treated as acknowledgement** — reconnect hint is mistaken for consumption proof.
19. **Proxy buffering ignored** — locally streaming SSE becomes delayed production bursts.
20. **One polling loop per subscriber** — each client independently polls shared state.
21. **Global order accidentally promised** — local sequencing becomes an undocumented distributed contract.
22. **Reconnect without jitter/backoff** — restart creates a thundering herd.
23. **Unlimited replay** — reconnect triggers unbounded history scans.
24. **Auth only at handshake forever** — long-lived access survives revocation or expiry.
25. **Close reason leaks internals** — sensitive details escape via terminal messages.
26. **Benchmark hides loss** — dropping slow consumers is compared against reliable delivery.
27. **Heartbeat storm** — connection count multiplies overly frequent keepalive cost.
28. **Source scanner proves safety** — keywords are treated as behavioral verification.
29. **WebSocket becomes business router** — message handler reimplements application policy.
30. **All events are durable** — replaceable progress updates explode storage/replay cost.

Use these as review triggers, not mechanical source-pattern bans. Context, accepted architecture and executed evidence remain decisive.
