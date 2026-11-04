# Workspace-local recovery experiment

Scope: one existing regular file, explicit immutable change ID, no automatic
source apply/restore/replay and no production DAG or provider wiring.

## Ownership and sequence

Existing `Store` jobs SQLite owns immutable recovery metadata and exact-ID replay.
`.hcodex/changes/<id>/before` and `after` hold bounded original/intended bytes.
Register the exact intent first, then sync artifact files/directories. A failure
between those steps leaves inspectable intent and incomplete artifacts; it does
not establish that source was changed. Never overwrite a different stored blob.

Inspection verifies record integrity, artifact lengths/hashes and current source.
It reports original bytes observed, intended-result bytes observed but unverified,
or conflicting bytes. Matching bytes are an observation, not a causal execution
receipt, gate proof or SETTLED state. Missing/corrupt artifacts are errors.
Unchanged before/after is rejected because observation could not distinguish them.

Source paths must remain inside the original workspace and cannot target private
`.hcodex` state. That directory is ignored by Git, source discovery and settlement
manifests, and ordinary patches cannot write it. Recovery integrity is checked
separately; excluding private state is not proof that it is undamaged.

This bounded std-filesystem experiment rejects static symlinks/hardlinks and
checks directory identity. It does not promise descriptor-relative confinement
against hostile concurrent directory swaps or hardware/power-loss durability.
Broader source-write admission remains a separate contract.

## Verification

Before implementing, define reopen/exact replay/conflict/corruption cases and real
owned-child termination at three boundaries: DB intent committed, archives synced,
and fixture source synced. Restart must classify bytes without replaying or
restoring anything. Verify historical jobs/handoffs compatibility and make ci.

## Independent development worktrees

Two detached worktrees were created on the external SSD and overlaid with the
current dirty Rust source, preserving the main checkout/index/HEAD. This is
used because HEAD still contains the old Python implementation. Worktrees
own disjoint files; Main integrates only those files after preimage checks. No
commit or push is part of this experiment. Remove only these task-owned copies
after verified byte-for-byte integration; retain source/evidence on uncertainty.

The user then clarified a stricter precondition: the ORIGINAL working checkout
itself, including untracked files, must be clean before creating agent worktrees.
The earlier dirty-snapshot approach does not satisfy this contract. Both already
completed workers' owned files were integrated with preimage/content checks,
and the two task-owned worktrees were removed. Remaining work proceeds in the
original checkout. No stash, reset or commit is used to manufacture cleanliness.
Any later product worktree experiment must first check the original checkout
with untracked files included and reject dirty state.

Product per-agent worktrees remain deferred. Council `cdx-6c7d0bfb87` completed
with Argon High, Claude Opus Medium and Codex Sol, requested/actual models matching,
all WARN: opinion agreement, review_approved=false. The concerns are dirty-source
baselines, downstream dependencies, canonical publication, disk/build overhead,
and retaining unknown/dirty/unjoined work rather than deleting on provider exit.
The council was design review only and ran no tests.

## Executed verification — 2026-11-04

Journal tests passed 3/3, including an actual two-connection race and reopened
immutable intent readback. Artifact tests passed 5/5, including links, partial
publications and conflicts. The integration test launches and kills an owned
child at each of the three specified durable boundaries, then reopens the same
Store and artifacts without replaying source. It also checks private-state
exclusion and patch refusal. All three integration test entries passed.

Final `make ci` exited 0 (rules/manifest integrity, formatting, workspace check,
strict all-target/all-feature Clippy and all tests). Log:
`/Volumes/256.SSD/RustBuild/hcodex-text-patch/ci-recovery-final.log`.
Git diff whitespace validation passed. No commit/push was performed by workers.

For the first portable Rust checkpoint, the required materialized harness/shared
rules and `.cargo/config.toml` alias are included in source. The local caches,
`.hcodex` runtime state, `.omx` reports, `.codex` setup and credentials remain
outside the publication allowlist. This keeps canonical validation runnable on
a clean checkout without silently skipping the manifest/rules gate.
