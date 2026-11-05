# Pure-backend cutover

The hcodex MCP stdio server and its client-registration lifecycle are retired.
The CLI and TUI now call the same in-process Rust application boundary directly.
This removes a transport layer; it does not replace the Gateway, SQLite store,
DAG scheduler, worktree lifecycle, provider runtimes, or evidence contracts.

## Runtime shape

```text
CLI / TUI
    ↓
Gateway application boundary
    ├── routing and provider runtime
    ├── DAG, verification and settlement
    └── SQLite jobs, handoffs and checkpoints
```

SQLite jobs and checkpoints are the durable authority. TUI event channels are a
bounded soft-realtime projection. They provide responsive node, provider,
verification and settlement updates without creating a second state owner.
`runtime_identity.capabilities` reports `backend="in_process"` and
`mcp_server=false`; it does not report a live MCP registration.

The product does not expose an hcodex HTTP, SSE, WebSocket or MCP server. A local
interactive terminal does not need a network fan-out service. If a bounded UI
channel falls behind, hcodex reports `ProjectionLagged` before the next durable
settlement and returns
`live_projection` counters for `dropped_intermediate_events`,
`undelivered_settlements`, `observer_closed`, and `resync_required`. The client
then uses the durable final result and checkpoints; hcodex does not silently
promote a partial stream to the execution record. A final marker is best effort
when no settlement follows; a completed execution report retains the gap even
if the queue remains full. If execution is interrupted before a report is
produced, projection counters may be unavailable. Existing checkpoints and
`recovery` provide the durable evidence for that unknown outcome.

## Removed surface

- `hcodex mcp`
- automatic hcodex MCP registration
- generated plugin `mcp_config.json`
- MCP enable, disable, refresh and restart commands
- MCP stdio protocol tests and process registration identity

`hcodex setup` and `hcodex sync` still install the managed skills and migrate the
recognized historical launcher. They remove a legacy registration only when its
managed hcodex identity is proven. Unrelated MCP entries, custom same-name
commands, credentials, comments and existing state remain untouched.

The state root keeps its historical `~/.cache/codex-collab-mcp/` name so existing
SQLite jobs, handoffs and checkpoint identities remain readable. The path name is
compatibility data rather than an active transport declaration.

## Real-time contract

The current workload is soft real time: a human should see progress,
cancellation and settlement quickly, while durable correctness must not
depend on the terminal keeping up.

- bounded channels cap memory and action concurrency;
- a settlement checkpoint is committed before awaiting its UI delivery;
- a slow UI may backpressure the bounded notification pipeline;
- intermediate live updates may coalesce or require resync;
- terminal settlement delivery is backed by a previously committed checkpoint;
- shutdown owns and joins tasks and child process groups;
- timeout or lost acknowledgement remains unknown until reconciled by the
  original ID.

This contract does not claim exactly-once event delivery or a replayable network
stream.

## Provider boundary

Codex, Argon and Claude run through owned CLI subprocesses. New Argon and
Claude jobs use Antigravity CLI JSON streams and its own cached credentials.
Historical Antigravity RPC rows remain readable.

hcodex commits the CLI conversation ID before sending the first prompt. The
original job exposes a bounded progress checkpoint (phase, session, model
reported by CLI init, byte count and available usage) without saving the raw
stream. Job status does not send another turn. A CLI conversation follow-up is
a new turn; reattaching to an in-flight turn after losing the owning process is
not supported. The owning process handles local cancellation and reaps its
process group; it does not attest remote or child-agent settlement.

Provider jobs have no default execution deadline. An explicit positive
deadline still applies; startup, stdin and terminal drain have separate I/O
bounds. The Antigravity route accepts artifacts and runs in an isolated scratch
directory. hcodex validates and applies scoped edits. Native Teamwork is not a
supported execution path.

## Verification boundary

The cutover is complete only when all of these checks have fresh evidence:

1. `hcodex --help` omits `mcp`, `on`, `off` and `restart`.
2. Setup and sync install skills and preserve custom/unrelated configuration.
3. Exact managed legacy registration removal has positive and conflict tests.
4. CLI and TUI still route operations through the Gateway.
5. Slow-consumer tests expose resync instead of hiding event loss.
6. Cancellation, shutdown, worktree propagation and durable settlement tests pass.
7. `make ci` passes with no MCP stdio fixture.

Historical migration and parity documents retain the evidence from the releases
they describe. Their MCP-specific counts and observations are not current product
contracts.
