# Restore the dependency graph presentation

The previous TUI showed a box-drawing dependency graph with pole colors and explicit fan-out/fan-in. The Rust renderer currently shows unconnected layer cards or a flat compact list. Restore relationships without changing planning/execution authority.

- Render only actual `DagLaneView.dependencies` for loaded plans. Reserve connector gutters between card columns and derive each line from an actual parent/child pair. Do not infer complete bipartite relationships from layers.
- Compact rows keep active/failed lanes first and show actual predecessor IDs with an arrow. Preserve stage labels, elapsed time, model/effort and overflow counts.
- With DAG mode enabled and no loaded plan, show an explicitly labelled routing preview: Argon branches to Claude/Codex and joins at Argon. Animate only that illustration's connectors; never create runtime nodes, progress, Running or SETTLED claims.
- Keep the existing motion policy, four palettes, composer badge, fixed input/status regions and stage distinction. Actual loaded edges animate only when their destination is active.
- Use bounded pure rendering on the existing tick; no dependencies, tasks, sleeps, network or provider calls.
- Verify non-complete dependency shapes, preview/no-work semantics, motion/static frames and compact sizes, then focused renderer tests/Clippy. Root owns canonical CI, install and native PTY verification.

## Implemented and focused evidence

The loaded graph now uses explicit dependency arrows between colored node boxes when they fit. Skipped layers or crossings between unrelated edges fall back to numbered parent pins and an exact edge overview, preventing a shared connector bus from implying unrequested edges. A common fan-out/fan-in shape gets a miniature branching/join drawing in compact mode only when its dependencies exactly match that shape. At 80×24 and 100×32 this stays above the active/failure rows; overflow is explicit. Displayed long IDs are clipped before stages, preserving full STOPPED/SETTLED labels even for valid 64-character IDs.

With no graph, DAG mode shows an animated role-routing schematic labelled `Routing preview · not run`. It does not add nodes, receipts or progress. Existing badge motion and static/reduced-motion behavior remain.

Focused verification: all 16 renderer tests passed, including actual diamond arrows, incomplete crossing/skip-layer edge identity, compact fan-out/join at 80×24 and 100×32, 64-character IDs with three predecessor pins, preserved active/failure stages, and zero-work preview motion. Strict `cargo clippy -p hcodex --all-targets --all-features -- -D warnings` passed. No additional dependency, task or sleep was introduced. Root owns final full CI, installed artifact and native PTY evidence.
