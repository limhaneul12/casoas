# Restore DAG mode motion

The user reported that the Rust TUI lost its original animation. Historical Git source `app/core/tui_completer.py:364` shows the centered `D A G - H E T E R A R C H Y` badge: a cyan/violet entry sweep at 14 × 40 ms and exit fade at 6 × 40 ms. Read this historical source only; do not restore the retired Python implementation.

Argon advisory `cdx-dag-mode-motion-budget-20261103` completed with actual model `gemini-4-argon-high`, verdict PASS. Its source-only recommendation is bounded tick-driven motion that preserves real execution authority. Initial request `cdx-dag-mode-motion-20261103` was rejected before dispatch for its 60 KB input budget; no provider outcome is being retried or relabeled.

- Focal moment: restore the historical centered composer-border badge and entry/exit timing; keep existing layout and four palettes.
- Continuity: mode transitions interrupt/reverse from current event-loop time; do not block input, resize or Ctrl-C.
- Feedback: low-frequency mode highlight; spinners only on actual active lanes; never synthesize nodes, progress, usage or success.
- Budget: use the existing 16 ms tick, bounded badge characters and existing lane count; no dependencies, new tasks or sleeps. NO_COLOR/TERM=dumb uses static mode indication.
- Mode contract: ordinary text after entering DAG mode dispatches RunDag; explicit `/ask` remains direct. Turning the mode off restores direct ordinary prompts.
- Ownership: main owns model state/input routing; renderer agent owns render.rs. Verify deterministic multi-frame buffers, mode/off/reduced motion and unchanged stage/progress semantics; then canonical CI, install and actual PTY entry/exit typing.

## Result

Restored the centered badge sweep/fade and quiet idle highlight, plus activity glyphs on actual Running/STOPPED lanes. The original four themes and compact/card layout remain. Exact `/dag-mode` submits on the first Enter; partial commands still complete. Mode-enabled plain text routes to RunDag, while explicit `/ask` remains direct.

Final `make ci`: 167 library + 2 stdio + 2 xtask tests = 171 passing tests. Deterministic motion tests prove changing colors with identical text, zero invented nodes/progress, static/reduced motion, small terminals and preserved execution stages. Installed Rust binary SHA-256: `5c1bc98818af9e1234f7e45672b0665e0509f92ab6a23be0812565948cd0c6e5`.

Actual 100×32 PTY use of PATH-selected hcodex confirmed cyan/violet sweep, exit fade, typing during entry, zero fake work, Ctrl-C exit 0 and terminal restoration. All 56 fenced source/config hashes remained unchanged. Evidence: `.omx/evidence/dag-mode-motion-20261103/{argon-advice.json,verification.json,ci-final.log,tui-motion.raw,mode-on-screen.txt,mode-off-screen.txt}`. Argon's PASS is source-only design advice; executed tests and native PTY evidence are separate proof.
