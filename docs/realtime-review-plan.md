# Realtime DAG and Rust launcher review

Scope: preserve the existing dirty Rust migration, public MCP contracts and durable data. Read the Rust harness normal/type/async rules, use the current CodeGraph index for call paths, and separate implementation from independent review.

1. Reproduce the PATH-selected Python shim failure. Migrate only the recognized obsolete hcodex shim to the installed Rust executable; preserve unrelated custom launchers. Cover installation repair with a regression test.
2. Compare upstream OMH at `333e8e323bc3fe6af1537746f8613d4740c64e6d`. Replace layer barriers with bounded dependency-ready execution; retain exclusive shared-workspace writes and evidence-gated settlement. Pass settled upstream handoffs to descendants, including resumed receipt summaries. Cover ordering and artifact delivery with controlled tests.
3. Prune ignored directory subtrees during source discovery. Add the ignored-source regression before changing traversal.
4. Keep running/verification elapsed time visible and show active lanes at ordinary terminal sizes. Cover time progression without sleeps and narrow-terminal visibility before editing.
5. Run focused checks, the canonical `make ci`, install/setup, then exercise the PATH-selected `hcodex tui` in a real PTY and a real multi-provider read-only DAG. Keep runtime receipts and exact source identity. A live lane-state display is distinct from provider token/tool streaming; report remaining differences explicitly.

Ownership: packaging agent owns lifecycle/install; DAG technical lead owns execution/request; main owns planner/TUI and integration. Independent verifier reviews the final diff and evidence. Real Argon/Claude/Codex review is advisory evidence and does not replace compiler/tests or turn WARN/BLOCK into approval.
