# Rust migration verification result

2026-11-03 · `hcodex 0.3.0` · Rust 2024

## Delivered scope

All active application, MCP, CLI, TUI, orchestration, persistence and tests are Rust-native. The former Python source, tests and packaging files were removed from active paths after an 83-file SHA-256 reference archive was verified. The initial source archive was subsequently deleted along with the project Python environment at the user’s explicit request; file hashes and JSON/log evidence are retained under `.omx/evidence/python-retirement-20261103/`. See `python-retirement-result.md`. Shared environments outside this repository and historical job/handoff data were preserved.

| Requirement | Implementation and evidence |
| --- | --- |
| Rust workspace and harness | Repository toolchain `1.98.0`, Rust Edition 2024, Cargo.lock, safe-only workspace lints, actual materialized harness SHA-256 verification in `cargo xtask rules`. |
| All 16 MCP tools | Frozen compatibility schemas, closed argument validation, typed compatibility responses, real stdio integration and external MCP SDK 2.3 consumer. |
| Model routing | GPT-6/6.1 chain, Gemini 4 aliases, Claude Sonnet/Opus medium-only policy; unsupported provider/engine/model/tier rejected before dispatch; no silent model substitution. Availability remains provider-catalog controlled. |
| Runtime and receipts | Tokio-owned bounded tasks, real Codex CLI/HTTP/Antigravity RPC, verified actual-model provenance, attempt/time/path/budget/auth/schema fields, queryable timeout-unknown and same-ID retry, process-group cancellation/reaping. |
| Durable compatibility | Historical Python-written SQLite fixtures; transactional cross-connection/process idempotency, deterministic duplicate ownership without row deletion, atomic terminal transitions, retention/checkpoint and handoffs. |
| SQL safety | Feature SQL files, prepared bindings, NUL/stacked/meta-command defenses, escaped LIKE and encoded read-only/immutable/query-only external CodeGraph connections. |
| Grounded context | Real local/global `SKILL.md` bodies, default codex-heterarchy, prompt/path/symbol relevance, actual numbered source excerpts and CodeGraph one-hop relationships. |
| Dynamic DAG | Generated 4–8 nodes and 2–4 waves, varying start frontiers, WAW/RAW/WAR/cycle/path checks, strict spawn plans, durable checkpoints/resume and source/config fences. |
| Settlement | Actual STOPPED→SETTLED evidence, write scopes, exact-preimage scoped Claude patches, atomic staging/rollback, dirty-read rejection, expected-failure reproduction and successful final gates. |
| Triad | Pipeline/council, independent partial outcomes, compact fenced snapshots, persistence-failure peer preservation, bounded repair with session continuity, exact decision-source/resolution checks. Completion and approval remain separate. |
| Interactive workbench | Ratatui/Crossterm fixed header/DAG/transcript/composer/status/footer; Unicode and small terminals; 45 legacy commands; slash/skill/file completion; themes, effects, permissions, resume and actual runtime integration. |
| Multiline and usage | Hard-newline editing and non-dispatching bracketed paste; soft-wrap/tab/wide-character cursor; per-execution receipt deduplication; unknown/partial token totals; verified current-context occupancy distinct from cumulative usage. |
| Installation | Rust-only distributable plugin and skills; embedded hash-verified setup assets; atomic permission-preserving settings updates; non-Python env preserved; PID/start identity guard; actual refresh RPC and catalog readback. |
| Agent configuration | Exactly five project custom agents. Narrow exploration uses GPT-6 Luna Max; implementation/integration/verification use GPT-6.1 Sol Medium; technical lead and project leader use GPT-6.1 Sol Extra High. |

## Final current-source verification

Evidence root: `.omx/evidence/rust-migration/final-20261103-v3/`.

- `make ci`: **passed** — rules/manifest integrity, formatting, workspace/all-target/all-feature check, Clippy `-D warnings`, **144 library + 2 real binary stdio + 2 xtask tests (148 total)**.
- Current release installed at `/Users/imhaneul/.cargo/bin/hcodex`; Codex and Antigravity managed configurations, plugin and both installed skills read back as the Rust versions. `CODEX_CLI_BIN` was retained; obsolete Python env keys were removed. Configuration/skill backups were preserved before updating.
- Installed/frozen release SHA-256: `fb9cd363b5fd62c3b5c61a70d3e7e7b15ef907586098407b1d26ad235bb6e072`.
- All **66 runtime/package source hashes** matched before and after the final SDK/provider check.
- Actual external MCP SDK negotiated **2026-07-28**, discovered **16 tools**, called health and the final Codex audit through the frozen Rust server.
- Final Codex requested/actual `gpt-6.1-sol`, `schema_valid=true`, durable job `completed`, identity source `codex_turn_context_verified`. Actual context window **475,000**, current occupancy **27,925** (**5.8789%**), reported input/output **27,627 / 298**. These are observations from that execution, not model-wide constants.
- Final native 80×24 PTY smoke: multiline bracketed paste, Ctrl-C exit **0**, raw/alternate-screen/bracketed-paste restoration. Unicode wrapping/cursor and paste-not-submit behavior also have deterministic renderer/input tests.

Exact records include `ci.log`, `install-readback.json`, `source-hashes.json`, `runtime-verification.json`, `codex-context-response.json`, `sdk-health.json`, `restart.json`, `health.json`, `tui-pty.json` and `tui-full-evidence.txt`.

## Live multi-provider behavior and evidence scope

The preceding frozen backend release (`39e6e27bf896817ee79d6192f00b56cde88da1c660172b9a37621b88d52c4572`) was independently exercised with all 66 then-current source hashes unchanged:

- `.omx/evidence/rust-migration/final-20261103/live-dag/`: actual Argon → Claude Sonnet → GPT-6 Astra → Argon execution; all four nodes settled; only declared fixture `src/lib.rs` changed; unchanged test command moved **exit 101 → exit 0**; durable job completed. The auditor's residual `i32::MAX` opinion remains recorded. This proves the prescribed fixture contract and controller/settlement path, not universal arithmetic approval.
- `.omx/evidence/rust-migration/final-20261103/live-triad/`: all three actual models and structured artifacts verified; collaboration recorded complete; PASS/WARN/PASS remained **conflict_requires_adjudication**, without fabricated consensus or approval.
- `.omx/evidence/rust-migration/live-triad/attempt2/`: earlier same-ID recovery reused valid Codex/Sonnet checkpoints and dispatched only the missing Argon role; actual **consensus_verified** and decision-source/resolution verification succeeded. Original attempt artifacts remained unchanged.

Subsequent backend changes only add verified context occupancy; subsequent UI changes fix multiline input, cursor and usage accounting. Their final current-source closure is the 148-test gate plus final SDK/Codex-context/PTY evidence above. AGY/HTTP context limits remain `unknown` when not reported; mixed/ambiguous sessions are not given a fabricated global context percentage.

Refresh success certifies the actual RefreshMcpServers RPC and model-catalog readback. It does not claim every pre-existing client conversation hot-reloaded its tool cache. New real SDK connections to the installed Rust server were verified.

Native independent review compared the attachment, the archived 87-test Python behavior oracle and all 45 original TUI commands. Its actionable findings were corrected and the final focused UI/context/protocol/environment re-review was clear. No commit, push or public publication was performed.
