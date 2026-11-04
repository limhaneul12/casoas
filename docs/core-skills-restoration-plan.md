# Restore core skill selection

Historical `DefaultCoreSkill` declares exactly `codex-heterarchy`, `codegraph-operator`, and `codebase-refactor-guardian`. Restore `/skills core` and `/skill core` to replace selected skills with the discovered core set; repeated selection is idempotent. Default selects only the discovered codex skill. Clear/none/off/reset clear the selected set. Unknown selections leave it unchanged, and missing preset members are explicitly reported.

Keep discovery precedence local workspace, user Codex skills, user `.agents/skills`, then packaged/plugin/Gemini roots. Sort traversal inside each root so duplicate packaged skills have a deterministic winner. Never add an active label for a missing file.

Direct prompts already include selected skill bodies. DAG requests must include the same bounded, disk-read bodies in each serialized node task, while the short displayed goal remains unchanged. Persisted node/config identities therefore fence the actual instructions. Preserve the bridge's separate canonical codex skill authority; do not expand into a gateway redesign.

Ownership: core restoration worker owns `context/skills.rs` and `terminal_adapter.rs`; parent owns slash routing and graph presentation. Add focused preset/discovery/body-injection regressions, run focused tests and strict Clippy, then parent integrates canonical CI and native TUI proof.

## Worker result

Implemented the exact three-skill core preset, deterministic discovered-file priority and `.agents/skills` fallback including symlinked skills. Missing members are reported rather than represented as active. Core/default selection serializes an isolated registry selection, reads actual disk bodies in `spawn_blocking`, and commits/publishes only after successful preflight. Each loaded member has its own transcript line and path. A deleted discovered body preserves the prior selection and publishes no success update.

Direct requests and every serialized DAG node request now carry actual selected skill bodies; the displayed node goal stays short. Oversized skill excerpts are explicitly marked with UTF-8 byte truncation boundaries. Existing bridge-level canonical skill context remains separate.

Focused checks passed: four registry tests and four `core_` tests (two overlapping, six distinct contracts total), including default/clear/unknown selection, missing files, local precedence, symlink cycles, actual body delivery to direct/DAG request arguments, and selection rollback on failed preflight. Strict all-target/all-feature Clippy passed. Root owns final canonical CI, installed command and native TUI verification.
