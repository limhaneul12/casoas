# Rust hcodex realtime DAG review — 2026-11-03

## Scope and reference

Reviewed the current Rust implementation under the repository harness using `codex-heterarchy`, `codegraph-operator` and `codebase-refactor-guardian`. Preserved the pre-existing migration work and durable history. No commit/push was performed.

Upstream comparison is pinned to Oh My Hermes [`333e8e323bc3fe6af1537746f8613d4740c64e6d`](https://github.com/rlaope/oh-my-hermes/tree/333e8e323bc3fe6af1537746f8613d4740c64e6d). Its [DAG core](https://github.com/rlaope/oh-my-hermes/blob/333e8e323bc3fe6af1537746f8613d4740c64e6d/src/plugin_bundle/omh/dag_core.py) treats layers as presentation and readiness as satisfied dependencies. Its [run loop](https://github.com/rlaope/oh-my-hermes/blob/333e8e323bc3fe6af1537746f8613d4740c64e6d/skills/dag-ulw/references/run-loop.md) admits ready work within available capacity, requires checked handoffs, and separates verification from child completion. The durable record itself is not a background timer.

## Repairs

| Boundary | Before | Current behavior |
| --- | --- | --- |
| Shell command | `~/.local/bin/hcodex` executed deleted `.venv/bin/python3` | Exact recognized legacy shim migrates atomically to the Rust executable; custom launchers and existing symlinks are preserved |
| Scheduling | Entire wave blocked its descendants | Dependency-ready nodes use free slots immediately; topology layers remain display metadata |
| Shared writes | Whole writing wave was serialized | Each writing node owns the shared workspace exclusively; independent readers can advance across layers |
| Handoffs | Descendants did not receive predecessor findings | Direct settled dependencies supply bounded summaries, actual model, manifest and gate evidence; resumed receipts supply the same context |
| Resume display | Resumed nodes could remain Waiting | Actual settled receipt events initialize resumed lanes |
| Discovery | Ignored directory children were still walked | `filter_entry` prunes ignored subtrees before their files become candidates |
| Failure persistence | Completing then failing a DAG was rejected by terminal CAS | One conditional success/failure transition; interrupted campaigns retain unknown outcomes |
| Cancellation | Campaign cancellation did not reach foreground node work | Owned campaign scope propagates cancellation to admitted nodes/providers/gates; new admission stops and active work drains |
| Child outcomes | Inherited cancellation could store Failed | Cancelled result and receipt are persisted with terminal history preserved |
| Checkpoints | UI settlement could precede storage; storage errors left execution running | Persist before exposing SETTLED; checkpoint failure signals cooperative cancellation and drains work |
| Gate processes | DAG timeout could leave descendants; output was fully buffered | One shared gate effect runner owns process groups, kills/reaps on timeout/cancel and hashes fixed-size output buffers |
| Supervisor | Fast/concurrent tasks could race control registration | Duplicate check, spawn and control insertion share a registration lock boundary |
| TUI | Running clocks froze; narrow cards hid active rows | Actual state events, running/verification clocks, compact active-first rows and bounded update batches; interrupted work stays unverified |

## Verification

Final canonical `make ci` passed: 163 library tests, 2 real stdio MCP integration tests and 2 xtask tests (167 total). Formatting, all-target/all-feature workspace checks, strict Clippy and Rust harness integrity passed. New tests cover readiness without a layer barrier, writer exclusion, error draining, upstream/resumed evidence, failed-result persistence, actual campaign cancellation of child process groups, checkpoint storage failure, cancellation history, supervisor races, ignored sources and narrow/live TUI state. Process output hashing was exercised with 1 MiB of output and a leader exiting before its descendants release pipes.

Plans and writer/reviewer separation: `realtime-review-plan.md`, `dag-realtime-refactor-plan.md`, `launcher-repair-plan.md`, `gate-process-refactor-plan.md`, `cancellation-status-repair-plan.md`. Native implementation owners delivered distinct slices; independent review identified the registration/cancellation/checkpoint defects, and the main agent separately reviewed the reviewer's cancellation patch.

Evidence lives under `.omx/evidence/realtime-review-20261103/` and `.omx/evidence/launcher-repair-20261103/`. `source-manifest-final.json` fences 56 source/config files; all remained unchanged during final runtime verification. `ci-verified.log` is the final canonical gate log.

## Direct installed-runtime use

- Final installed binary: `~/.cargo/bin/hcodex`, SHA-256 `04a605d5af19892364acf0780f5c516810a54e96c1fe42ee965485645b34052c`. PATH-selected `~/.local/bin/hcodex` resolves to it. Setup preserved existing MCP configuration and synchronized skills. Python source/bytecode inventory remains empty.
- Opened the actual `hcodex tui` in a 100×32 PTY, selected read-only permissions and `gpt-6.1-sol:xhigh` for Codex lanes, then audited a disposable Rust fixture. The fixture gate asserts its actual `CARGO_MANIFEST_DIR` as well as arithmetic behavior, preventing a shared-target checkout mix-up.
- Initial campaign `cdx-3d18d48723` ran Argon, Claude and Codex with matching requested/actual models. Three nodes settled; another Codex call returned confirmed model-capacity failure, so its gate was not run and the final dependency was blocked. The campaign correctly persisted Failed; no model substitution or false completion occurred.
- Reopened the final installed TUI and ran `/resume cdx-3d18d48723`. Campaign `cdx-1aef3cd7b0` reused exactly the three settled nodes; only the failed Codex node and previously blocked Argon fan-in ran. All five nodes settled with executed `cargo test --workspace --all-targets` exit 0 and no out-of-scope writes. Original failure history remains.
- Captured the live running clock and final `Completed · Done 5/5` state. Actual use exposed long-goal/card-height clipping and a stale Running header after failure; both were fixed, regression-tested and independently reviewed before the final install/resume proof. Ctrl-C exited 0 and restored alternate screen, bracketed paste and cursor state.
- Fresh stdio MCP against the final binary returned 16 tools, the corrected dependency-ready description and healthy routing. Already-running MCP consumers load a replaced executable only after reconnection; this fresh consumer proof is separate from the original chat's earlier server process.
- Receipts: `verification.json`, `live-jobs.json`, `live-first-summary.json`, `resume-jobs.json`, `tui-live.raw`, `tui-resume.raw`, `tui-resume-running-screen.txt`, `tui-resume-final-screen.txt`, `mcp-final-readback.json`. Text screen reconstructions are bounded ANSI interpretations of the actual PTY capture, not independent GUI screenshots.

## Evidence limits

- Realtime currently means node start/return/STOPPED/settlement events and elapsed verification time. Provider token/tool-body streaming is not implemented; token usage is updated only from returned runtime receipts.
- Shared-workspace writes remain exclusive. OMH-style concurrent write isolation, automatic DAG repair/extend, parked tasks/wake times and editable live topology are not implemented.
- Scheduler capacity is fixed and bounded, not AIMD. The misleading MCP description was corrected without changing its input/output schema.
- The initial real triad review inspected bounded, partly truncated source. Argon/Claude returned BLOCK and Codex timed out unknown (`cdx-dag-realtime-audit-20261103`, Codex child `cdx-0fbc3af212`). Exact-ID readback preserved that unknown result. The review findings were evaluated against source and addressed by targeted fixes; this does not retroactively grant triad approval or consensus.
- A green mechanical gate proves its executed contract. It does not turn a model's WARN/BLOCK into review approval or prove every OMH feature.
