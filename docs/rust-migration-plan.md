# Full Rust migration execution plan

Goal: replace all Python production functionality with Rust 2024, preserving the 16 MCP tools, CLI, durable job/handoff contracts, real provider execution, DAG settlement and interactive TUI. Attachment `pasted-text-1.txt` is the requested acceptance specification.

## Work lanes and ownership

- Root: Cargo workspace, xtask mechanical contracts, MCP/CLI integration, triad/evidence, cutover, final verification.
- Storage: typed SQLite job/handoff persistence, external SQL, parameter and read-only safety, historical data compatibility.
- DAG/context: dynamic topology, hazards, STOPPED→SETTLED, SHA manifests, actual skills and code excerpts.
- Runtime/routing: strict model routes, receipts, Codex subprocess and Antigravity RPC, cancellation/timeouts.
- TUI: ratatui layout/event loop, keyboard autocomplete, slash commands and live transcript.
- Inventory/review: behavior checklist from actual Python source and tests; independent acceptance review.

## Sequence and proof

1. Record current source contracts and test baseline (Python: 87 passing before rewrite); retain dirty work.
2. Implement independent Rust boundaries with behavior tests; no mock provider completion in production.
3. Integrate all MCP tools/CLI/TUI; port contract fixtures and verify wire/data compatibility.
4. Make Rust gates authoritative: mechanical rules, fmt, strict all-target/all-feature Clippy, tests. Capture actual Argon/Claude reviews with receipts and separate consensus from execution.
5. Verify real MCP framing, process timeout/recovery, TUI small/Unicode rendering and live provider paths. Retire Python production only after all features have Rust evidence.

No narrow subset or green test suite alone closes this goal. Inventory items without executed coverage remain incomplete.
