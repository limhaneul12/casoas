# Rust migration status — 2026-11-02

Goal remains active: complete feature-preserving Rust 2024 replacement of the Python MCP/CLI/TUI, including all requirements in the attached project specification. This document is a progress checkpoint, not completion evidence.

## Executed current verification

`make ci` passed after the latest gateway persistence and handoff wire compatibility fixes:

- Rust xtask mechanical policy + materialized harness SHA-256 integrity
- workspace formatting
- workspace/all-target/all-feature compiler check
- workspace/all-target/all-feature Clippy with `-D warnings`
- 92 hcodex library tests
- 1 real binary MCP stdio integration test
- 2 xtask tests

Tests include real local HTTP fixtures, subprocess process-group cancellation, historical Python-written SQLite fixtures, CodeGraph 1.6 TEXT identities, actual SQL defenses, dynamic DAG topology/hazards, scoped Claude patch application, durable campaign/resume, truthful settlement, and TestBackend terminal layouts. These do not prove full real-provider or production-install acceptance.

## Collaboration evidence

- Native agents implemented independent storage/evidence, runtime/routing, HTTP, DAG/context, TUI, lifecycle and triad slices. Independent reviewer identified gaps; parent integrated corrections.
- Initial real Claude plan review succeeded; evidence is `.omx/evidence/rust-migration/claude-review.json`. It was a document review and raised concrete acceptance requirements.
- Initial Argon plan review returned `timed_out_unknown` under the original stable request identity; it was not retried or counted as consensus.
- New Rust-binary source reviews are a different task with explicit injected Rust source and stable per-role request identities. Their handles/artifacts must be read before claiming completion.

## Still required before closing goal

- Reconcile fresh independent parity audit against the full original inventory, all 16 MCP tool inputs/outputs and CLI aliases.
- Verify real provider outputs and strict schemas through the Rust binary; keep execution state separate from review/consensus.
- Complete and verify restart/refresh behavior and entrypoint/install/plugin cutover.
- Retire Python production entrypoints/source and temporary oracle only after full parity is evidenced; currently Python is still present and the migration is not complete.
- Verify clean checkout/install/package behavior, final terminal interaction and complete current-source acceptance again after remaining fixes.

No goal completion, deployment, commit or push has been claimed.

## Latest independent audit and live Rust source canaries

A fresh independent audit confirmed the earlier HTTP/evidence/schema/health/CLI/DAG/TUI/cancellation/circuit gaps were corrected. It still requires production package/plugin/docs/skill cutover, actual restart/refresh, relevance-based ordinary prompt context, live provider + temporary-workspace DAG acceptance, atomic config writes, process unregister, MCP version negotiation, and removal of a PID-read test race.

Rust-binary read-only source review calls for Claude and Argon both returned `timed_out_unknown` after 60 seconds. Their original IDs are `rust-source-review-claude-20261102` and `rust-source-review-argon-20261102`. Artifacts are `.omx/evidence/rust-migration/{claude,argon}-rust-source-review.json`; isolated durable job state is `.omx/evidence/rust-migration/rust-source-jobs`. Neither is proof of successful review or consensus. Do not launch duplicate retries with new IDs.

Next bounded lanes are active: lifecycle atomic/config/restart/process ownership; prompt relevance and PID readiness; protocol negotiation research. The full objective remains active; no production Python cutover is claimed yet.

## Subsequent integrated progress

- `make ci` passed with 98 library tests, 1 real binary stdio test and 2 xtask tests (101 total) after protocol negotiation, relevance selection, lifecycle atomic writes/start-identity ownership and refresh/readback integration.
- Installed the Rust binary to an isolated prefix using `cargo install --path crates/hcodex --locked --debug --root .omx/evidence/rust-migration/install-smoke`; installed `--version`, `help` and noninteractive `tui -c /models` exited 0. Readback is in `install-smoke/results.json`.
- Native 80x24 PTY launched the current Rust TUI, rendered fixed regions, accepted help input and exited via Ctrl-C with alternate-screen restoration and exit 0. This is a bounded terminal smoke, not full interactive provider/DAG acceptance.
- Lifecycle refresh success now means only actual RefreshMcpServers RPC plus catalog readback; installed server readiness remains explicitly unverified.
- Rust package/plugin/docs transition lane and distinct sequential live-provider canaries remain in progress. Do not repeat prior unknown source-review IDs.

## Current integration checkpoint — 2026-11-03

Rust package verification (`cargo package --locked -p hcodex --allow-dirty`) completed successfully, and Rust-only plugin/install/docs artifacts have been prepared and unignored for review. The Python source oracle is still present; retirement remains outstanding.

Real provider canaries returned terminal failures (not unknown): Codex lacked a JSONL model identifier; Claude and Argon returned invalid closed-schema outputs. The original failed receipts are immutable. Codex now has a stronger optional runtime identity proof fenced by exact session ID, cwd, engine turn settings and assistant output digest. Antigravity prompts now include the exact server-owned JSON schema and explicitly exclude receipt/evidence fields. One bounded same-ID retry per confirmed-failed canary is authorized after the build stabilizes; no unknown source-review request is retried.

An exhaustive Python TuiSlashCommand scan found additional missing historical commands including permission/resume/compact/theme/triad aliases. Their TUI port is actively being integrated. During that edit, a full CI build stopped on temporarily missing handler methods; do not report the earlier 101-test green run as current-source acceptance. Runtime canary retry was blocked before dispatch and must resume only after the TUI slice compiles. Internal sandbox policy is carried by Gateway.with_sandbox rather than adding hidden public MCP input fields.

## Full behavioral parity follow-up

The exhaustive original Python-test audit revealed additional contracts not covered by the earlier smaller inventory: cross-connection SQLite idempotency/CAS, detailed job/budget/error wire fields, compact triad persistence with partial-role preservation, exact evidence dimensions, repair-session continuity, DAG reproduction and dirty-read enforcement, auth-warning isolation, and additional command semantics. These are active implementation lanes, not deferred scope or optional polish. Some audit assertions conflict with current Python parameterized tests (notably unanimous WARN consensus); owners must follow actual current source/tests rather than blindly adopt the audit wording.

Current ownership: storage handles transactional/CAS persistence and atomic budget terminal transitions; gateway contract lane handles wire DTOs, budgets, schema_valid and specialized prompts; triad lane handles peer/persistence/evidence/decision/session parity; DAG lane handles reproduction/dirty reads/compatibility responses; TUI lane restores all 45 historical commands. Root added bounded first-object JSON decoding and receipt start/end/call-path/attempt fields. Current-source final CI is unproven while these edits are integrating.

Live acceptance progress: same-ID authorized attempt2 independently succeeded for Codex, Claude and Argon; original failure artifacts remain unchanged. These prove provider routing/output acceptance only, not triad consensus, complete DAG write settlement or full migration completion. The actual temporary-workspace DAG acceptance still remains required after semantic integration.

## Final verification closure

See `docs/rust-migration-result.md` for the current requirement-by-requirement result and exact evidence scope. Active Python paths are retired with a verified 83-file archive. The final installed/frozen Rust release is `fb9cd363...e072`; all 66 runtime inputs stayed unchanged through the actual MCP SDK/Codex-context check. Final `make ci` passed 148 tests with strict Rust gates. Native PTY paste/restoration passed. Managed Codex/Antigravity configs and installed skills now point to Rust and preserve non-Python environment values. Earlier backend immutable-head DAG/triad proofs and their exact source/binary identities remain preserved; no timed-out or conflicting review was relabeled as success.

## Explicit complete Python deletion follow-up — 2026-11-03

At the user's request, the installed/connected Rust MCP and CLI were dogfooded again, followed by complete removal of project `.venv`, Python caches, implementation/harness source backups and obsolete Python evidence helpers. 2,404 files removed; zero project Python source/bytecode files remain. All 158 protected DB/fixture/evidence/config files retained identical hashes. Full post-cleanup Rust CI passed 149 tests, and installed/connected MCP plus CLI readback still worked without `.venv`. Source hash metadata and JSON/log receipts remain; Python code backups no longer exist. See `docs/python-retirement-result.md` for exact scope and evidence.
