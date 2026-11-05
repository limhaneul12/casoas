# DAG node worktree mode

Explicit `spawn_plan.isolation_mode="worktree"` or `/dag-run --worktree <goal>`.
Legacy direct/serial modes remain available; no fallback occurs on worktree errors.
Worktree mode requires Unix process-group/root-identity support. Resume is
restricted to retained campaigns with durable ownership and an exact verified
checkpoint frontier. Unknown provider work remains inspect-only under its
original request ID; a continuation never retries it under another ID.

## Named sessions and dependency propagation

`hcodex --worktree-name feature-auth` creates/reopens a persistent detached session
worktree and opens the TUI. From that session, `/dag-mode` and executing DAG
commands default to node worktree isolation without repeating `--worktree`.
Named startup is terminal/DAG-only; it cannot be combined with setup, sync,
prune, recovery or initial resume management. Startup without the flag keeps its
existing defaults.

Names accept 1..64 ASCII letters/digits/hyphen/underscore. Names are scoped by
canonical original repo and configured external state root. A closed, bounded
registration record pins original/common/root identity and Git registration.
Reopening never resets named source or silently refreshes it from parent HEAD.
The original parent must be clean on entry; an existing dirty named workspace may
be opened for review, but starting its next DAG still requires that workspace to
be clean. No new development worktree was created from the currently dirty source.

Completed verified source is accumulated in the named session, not the original
parent checkout. All pending node worktrees are precreated while the session is
clean. For `1 -> 2 -> 3`, node2 receives the complete verified source state after
node1 (including new files), and node3 receives the state after node2. Handoff
summaries complement actual files; they do not replace source propagation.
Nodes are disposable only after canonical verification/storage; named sessions
retain final user work until explicitly managed by the user.

## Lifecycle contract

- Acquire a cross-process canonical-workspace lock, require original Git top-level
  checkout clean (staged, unstaged and untracked), pin HEAD/index and source/private
  metadata, reject unsupported submodules/sparse/symlinks. Readback of an already
  recorded request is separate from starting a new campaign.
- PRECREATE every pending node worktree while original is still clean. No commits,
  stash, reset, merge or hidden copying of pre-existing dirty user work.
- Before each node dispatch, overlay only the expected canonical source snapshot
  into the existing worktree, including verified upstream-created files. Retain
  current writer serialization; isolated trees do not imply parallel publication.
- Provider effects stay read-only/artifact-returning. Host applies declared,
  preimage-bound PatchArtifact in the isolated tree, then verifies it. Canonical
  owner revalidates expected source, exports original bytes/artifact first,
  publishes scoped edits and runs the canonical gate. Only canonical proof may
  produce SETTLED. Gate failure does not blindly restore over newer original work.
- Owned local process-group closure is explicit; missing/false proof, unknown
  provider outcome, cancellation, drift or checkpoint/export failure retains work.
- After all node checkpoints and a durable full-result export in the existing
  Store succeed, verify exact owned worktree/source/private state before removal.
  Final campaign output records removal or retention. No Drop/TTL cleanup.
  A timed-out/lost deletion acknowledgement is `cleanup_unknown`, never asserted
  retained or removed. The campaign DB metadata and lifecycle.json retain the
  exact state path/campaign ID before node dispatch.

Result/evidence storage is outside disposable worktrees under the configured
state root. Worktree paths and lifecycle.json remain discoverable on failure.
Final original source changes remain uncommitted for review.

## Scope and validation

### Continue a verified frontier

```sh
hcodex --workspace /path/to/existing/workspace resume cdx-campaign-id
```

TUI uses `/resume <campaign_id>`. `/dag-run --resume <campaign_id>` uses the same
saved request. Do not append a new goal: a continuation uses the exact stored
compiled plan, models, skill bodies and request configuration. Current model or
skill selections are not injected again. A historical direct/serial checkpoint
can retain its earlier behavior; a historical worktree campaign without ownership
evidence cannot acquire resume authority retroactively.

For named sessions, open the existing session normally and run `/resume` there,
or pass that existing session path as `--workspace`. Initial resume commands
combined with `--worktree-name` are rejected before creating a named workspace.
An already-open interactive named session remains available for resume.

The host reopens only the campaign's retained, precreated node trees. New tree
creation still requires a fully clean original checkout. Resume instead requires
unchanged HEAD/index and an exact match between the current source/private state
and the saved verified frontier, including verified new files. It never uses
stash, reset, automatic commit or dirty-file copying to manufacture a baseline.

Ownership inventories and filesystem/Git identities are bounded in an immutable
`ownership.json` under the existing campaign state directory. Existing campaign
metadata pins its hash and the compact lifecycle frontier. The original workspace
lock is reacquired before preflight. A new continuation has its own campaign ID,
while each node keeps one stable provider ID for the original ownership lifetime.
An existing provider record for a supposedly pending node causes refusal.

Only checkpointed `SETTLED` nodes and untouched pending nodes may continue.
Preparation/admission are durably recorded before their effects. Partial source
publication, admitted-but-uncheckpointed work, changed ownership/source/config,
or a missing frontier is retained and rejected. All-settled export recovery may
reuse verified output without rerunning providers or gates. Partial cleanup does
not become an automatic deletion retry.

The existing plan checkpoint stores one bounded request recipe needed to recover
the exact operation. This is task configuration, not a transcript archive. Node
handoffs and export records remain the evidence authority; no new database or
message queue is introduced.

### Runtime identity observations

`status`, `board` and their TUI views distinguish source HEAD/dirty/scoped hashes,
the installed CLI file, the current process, and backend capabilities. Missing
historical startup fields stay unknown. An on-disk binary hash after
replacement is not a loaded-image identity, and matching package versions do not
prove matching code. Source-to-binary linkage and CI remain unverified unless an
actual bound receipt supplies that evidence. Display does not start or restart a
service. The retired `registered_mcp` observation is not a backend-health signal.

Worktrees isolate source working copies, not network, credentials, provider
sessions or shared Git object/ref storage. Cooperative ownership/path fences
are not hostile concurrent-filesystem confinement. Build/cache folders are
regenerable and excluded from source/private cleanup proofs; no speedup is claimed.

Required evidence: real Git dirty/index/untracked preflight; separate node roots;
upstream-created files; canonical drift; isolated-pass/canonical-fail; unknown and
unclosed child retention; durable checkpoint/export before cleanup; unrelated
user worktree preservation; normal successful cleanup. Use deterministic provider
boundary fixtures and actual Git/verification processes, then canonical make ci.

Isolated gates override inherited `CARGO_TARGET_DIR` with the node's own `target/`.
This avoids cross-worktree artifact reuse; verification commands select their
workspace from runtime cwd rather than a cached compile-time directory.
Source/private inputs are bounded to 4,096 files / 64 MiB in this first release.

## Historical verification — 2026-11-04

The following evidence predates the pure-backend cutover. Counts that include an
MCP fixture describe that historical release and do not claim a current MCP
entry point. Current verification is recorded in [backend-cutover.md](backend-cutover.md).

Final local `make ci` exited 0: rules/manifest integrity, formatting, workspace
check, strict all-target/all-feature Clippy, and **278 tests passed**, zero
failed/ignored (268 library + 1 CLI + 2 MCP + 3 recovery + 1 comparison + 3 xtask).
Log: `/Volumes/256.SSD/RustBuild/hcodex-text-patch/ci-dag-worktrees-final-verified.log`.

Coverage includes real Git dirty/index/untracked rejection, precreation of all
nodes, distinct node roots, upstream-created files, original/hidden-state drift,
unrelated user worktree preservation, canonical-gate failure, unknown/unclosed
provider retention, native process-group observation, failed export retention,
exact same-ID readback and cross-workspace ID rejection. The complete request,
controller, checkpoint, export and cleanup path ran with real Git/Cargo processes
and a controlled local provider executable. These are not live external-provider
E2E results.

Integration reproduced and fixed a concurrent-reader index.lock conflict by
replacing `git write-tree` probes with read-only cached-index comparison.
Cached xtask compile-time path reuse was reproduced after an earlier worktree
was deleted; runtime cwd selection and isolated build targets resolve it.

Three development worktrees were created only while the original checkout was
clean. Worker changes were integrated with preimage checks, independently
reviewed, and archived outside the worktrees before those task-owned copies were
removed. Main now contains uncommitted feature changes; no additional developer
worktree was created after it became dirty. No provider/request was replayed to
obtain a passing result, and no installed service was restarted.

### Named session and three-step chain verification

The subsequent named-session implementation passed local canonical `make ci`
with **284 tests**, zero failed/ignored (272 library, 2 CLI, 2 MCP, 1 named CLI
integration, 3 recovery, 1 comparison, 3 xtask). Log:
`/Volumes/256.SSD/RustBuild/hcodex-text-patch/ci-named-worktrees-final.log`.

Real CLI planning-only startup created a named worktree and reopened the same
path while preserving user changes and the clean original. A linked named
workspace successfully hosted disposable node worktrees; cleanup preserved the
named parent. The dependent-node test now covers three distinct worktrees:
node1 changes source and creates a file, node2 verifies those bytes then edits
that file and creates another, and node3 verifies both preceding result states.
This uses controlled provider-boundary responses with actual Git/Cargo gates.

The full suite exposed floating-duration identity drift across JSON persistence
(1.6224532919999999 became 1.622453292). A deterministic reopen regression
reproduced it before enabling the existing serde_json float_roundtrip feature.
No new package was added, and result-equality assertions remain strict.
Source changes are uncommitted and installed services have not been replaced.

### Verified-frontier resume closure — 2026-11-04

- Final `make ci`: **352 passed, zero failed/ignored**; mechanical rules, manifest
  integrity, formatting, workspace checks and strict all-target/all-feature Clippy.
- Twelve actual SIGKILL cutpoints cover planned creation, preparation/admission,
  staging/partial publication, gate completion, checkpoint/frontier boundaries,
  export commit before acknowledgement, and cleanup removal before acknowledgement.
  Controlled transport fixtures retain real Gateway registration, Git, SQLite,
  Cargo and request/checkpoint paths. They are not live RPC proof.
- Real TUI dispatch continued a checkpointed writer through the remaining
  `1→2→3` source/new-file/ancestor chain. Unknown/admitted/colliding provider IDs,
  changed ownership/lock/source/config and late source drift were refused or retained.
- A separate installed-binary canary used two real Sonnet Medium calls and two
  real Codex Sol calls. After an isolated export failure, native `resume` reused
  all four verified nodes and removed their disposable worktrees. Provider records
  were unchanged and the physical gate counter stayed **6→6**. The original run
  used four isolated gates and two canonical source versions; its later canonical
  gates reused the existing source-bound cache.
- The ancestor witness reached the downstream actual-model review. Current
  `board`/TUI identity and a fresh isolated 16-tool MCP connection were also read back.

Evidence: `/Volumes/256.SSD/RustBuild/hcodex-stabilization-goal-20261104/`.
These receipts do not establish power-loss durability, hostile filesystem
confinement, loaded-image identity, or whole-system/native-visual parity.
