# Python → Rust parity closure — 2026-11-03

The earlier migration retained the main backend paths but did not fully restore the original interaction and context contracts. This pass used the exact retired product/test source, not feature-name matching or previous completion statements.

## Historical coverage

Git `2d5be2d25ed12736c44961534deca0d3ba0db759` matches all archived Python application/test bodies. Of 83 manifest entries, 82 SHA-256 hashes match; only retired packaging `pyproject.toml` differs. The trace ledger indexes 71 feature contracts, 845 class/function declarations, 62 test definitions and all 87 statically expanded parameter cases. No historical tests/cases are unmapped; no Python runtime/source tree was restored.

The detailed per-contract source/test mapping is [python-rust-parity-audit.md](python-rust-parity-audit.md), with machine-readable definitions/cases in `python-rust-parity-audit.json`. IMPLEMENTED is a source-contract status, not a claim that every provider/platform combination was newly executed.

## Restored behavior

- Contextual command/model/effort/permission/skill recipes; Korean keyword search; actual global/plugin skills; quoted Unicode/punctuation file completion; selected-row scrolling and inline ghost text. Arrow/Tab/Enter/right acceptance, history, Ctrl-U/W/P/N/D, ESC, multiline paste and inspector paging are connected to real actions.
- Actual `@file`/quoted file/line-range contents, canonical attachment evidence, late source definitions and CodeGraph indexed whole-function/one-hop/diff context. Missing/unsafe/malformed references fail before provider dispatch. Skill example blocks cannot hide arbitrary user text or fabricate attachments.
- Context/usage/selected skills remain visible in dedicated rows. Model-window percentages use runtime receipts and name the originating model; actual grounded request-byte budgets are shown when a provider does not report its window. Delayed foreign-model receipts cannot masquerade as the selected model's metrics.
- Inspector overlays leave the composer/status/menu usable. PageDown reaches long soft-wrapped lines. A provider reply taller than the log remains visible; real code/summary is shown before raw response envelopes. No synthetic token totals or compaction refunds.
- Registered skill prefixes preserve exact quoted-path whitespace and commands/questions. Reserved `/help` and `/review` keep command precedence; skill-only input invokes no provider. Explicit PlanDag/EnterDag/RunDag goals cannot be upgraded into another command, while ordinary DAG-mode input has its own typed origin and can honor an explicit prefixed `/ask`.
- CLI defaults/positional forms/core presets and execute-versus-plan-only behavior, atomic model/effect settings, SHA-256 session-baseline diff including non-Git workspaces, actual five-section node inspectors/source boards, and matching resume-goal lookup. Historical `/triad` remains a DAG alias; `/triad-review` preserves the actual three-role review pipeline.
- Backend default differential axes, historical model/provider/engine aliases, actual Flash 3.8 identity and Gemini tiers, cross-mode triad resume fences, early invalid workspace/sandbox rejection, CodeGraph one-hop write hazards and actual bounded AIMD feedback.
- Explicit `codex-default` uses native read-only `config/read` rather than guessed TOML precedence. Current native review uses `exec review --json` with target-safe instructions; the CLI's invalid target-plus-positional-prompt combination is not copied.

## Current-source verification

`make ci` passed **226 tests**: 221 library, 1 CLI, 2 real stdio integration, 2 xtask. Harness integrity, formatting, workspace/all-target/all-feature checks and strict Clippy passed. Initial unrestricted parallelism made two otherwise unchanged 2-second positive process fixtures time out; both passed in isolation, and the entire suite passed with four threads. Canonical test concurrency is now four. No identity assertions, deadlines, negative timeout tests or test cases were relaxed/skipped.

Five native agent slices delivered separate inventory/DAG, completion/input, context/SQL, adapter/CLI and backend/runtime work. Cross-owner review found reserved-command collisions, plan-only rerouting, malformed quoting, marker filtering, hidden inspector/menu and wrapped-tail issues. Those findings have dedicated regressions and final integration evidence.

Installed binary: `~/.cargo/bin/hcodex` via `~/.local/bin/hcodex`, SHA-256 `387066c247b0e4f2caae84cfdae37fa6a36e3c6d1e68d52d80f3ff85f5cbf926`. All 72 fenced source/config/asset files remained unchanged during final acceptance. No commit/push or unrelated cleanup was performed.

## Actual installed use

- In a real 100×32 PTY, `/use gp` displayed six model choices with descriptions and ghost text; Down/Tab accepted a full value. Ctrl-P restored the exact 263-byte prior input. A paged inspector left the completion menu, composer and context meter visible.
- `skills core` and positional `tui /skills core` both loaded the three real core skill files. In the same interactive session, native default selection resolved the actual project Codex configuration before any model turn.
- With DAG mode on, a registered-skill-prefixed `/ask` dispatched exactly one direct Codex request rather than a DAG. The query referenced `@"src/맥락  sample[late].rs":L220-L240`, including Unicode, two spaces and brackets. Actual `gpt-6.1-sol` returned the exact line-225 marker `PYTHON_PARITY_LATE_6F3A` and named all three received skill bodies. Canonical evidence-file SHA-256 matches the file on disk.
- Final installed reply proof: job `cdx-55360abad8`, confirmed success, requested/actual identity consistent, reported window 475,000/current 47,738 tokens. The terminal visibly displayed the returned code/marker/core names and `Context [gpt-6.1-sol] 89.9% left (ctx 10.1%)`.
- Fresh MCP discovery returned 16 tools. Actual native default readback kept requested `codex-default`, resolved `gpt-6.1-sol`, and deliberately rejected a 1 KiB budget before provider dispatch; actual model remained null and the stored job was terminal `input_budget_exceeded`. Native transport's real `emittedAtMs` extension was added as a typed optional field, retaining unknown-field rejection.
- Ctrl-C exited zero and restored both tested terminal sessions. Model query and plan-only evidence are distinguished; no new whole-DAG/all-provider/full-platform E2E result is invented.

Evidence is under `.omx/evidence/python-parity-20261103/`: `verification.json`, `source-manifest-final.json`, `ci-final-acceptance.log`, `native-default-final-mcp.json`, context provider receipts and native terminal captures/reconstructions.

## Preserved policy differences and live state

The ledger records intentional differences: fake historical telemetry/acceptance and fake token reclaim are not copied; safe auto-edit remains the startup permission; unsupported current-model `minimal` and non-Medium Claude are rejected; omitted/null Codex model retains explicit role policy while `codex-default` is selectable; generated target bounds reject rather than silently ignore extra requested targets. Targeted native review has an explicit 96 KiB escaped instruction-argument cap. Named profiles/custom providers are not guessed or silently discarded by default resolution.

An existing user TUI (PID 52105, started before reinstall) was observed and preserved. Already-running TUI/MCP processes retain their loaded code; installed changes take effect in newly started sessions. Final acceptance used fresh processes, without terminating or overwriting the user's current input/history.
