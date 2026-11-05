# Research & Search, Planning, Building

The TUI has three workspaces with independent drafts, reports, transcripts,
progress and DAG lanes. `/space research`, `/space planning` and `/space building`
switch between them; `Alt+1`, `Alt+2`, `Alt+3` are secondary shortcuts. Existing
plain numeric shortcuts and `/plan`, `/dag-plan`, `/dag-run` keep their meanings.
The initial workspace is Building for existing interaction compatibility.

Plain text in Research starts a research request; plain text in Planning starts
a planning proposal. This routing takes precedence over DAG mode. Building keeps
the existing prompt/DAG behavior. Incoming results do not change the selected
workspace. A chain reserves its selected workspaces together; another request
cannot acquire an overlapping workspace while that chain is active. Disjoint
workspaces can run independently.

## Independent and connected work

```text
/research --request-id research-topic -- investigate the topic
/planning --from research-topic -- plan the proposed feature
/building --plan wpa-EXACT-PERSISTED-PLAN-ID

/workflow --through research -- investigate only
/workflow --through planning -- investigate and plan
/workflow --stages planning,building -- plan and develop this feature
/workflow --through building -- investigate, plan and develop this feature
```

`/workflow` defaults to Research then Planning. Building is included only when
explicitly selected, or when `/building --plan <id>` is invoked. A Building-only
request consumes an exact persisted planning artifact; it does not pick a global
latest plan. Planning-only may use a goal without previous research. Invalid,
duplicated, reversed or skipped stage chains are rejected rather than inferred.

The same TUI adapter is available without opening a terminal screen:

```sh
hcodex --workspace /path/to/repo tui -c '/planning -- design a bounded feature'
hcodex --workspace /path/to/repo tui -c '/workflow --through planning -- investigate and plan'
```

`--request-id`, `--planner`, `--planner-effort` and `--timeout-seconds` configure
an explicit workflow. The default has no execution deadline; an explicit
positive deadline is shared by its stages. On cancellation or deadline,
owned work is drained before any further stage can be admitted. Existing research
options remain available through `/research`, including `--file`, `--document`,
`--url`, `--web`, `--no-web` and effect-free `--plan-only`.

## Where work happens

The canonical source repository remains the source of observed code. Research
and Planning model working directories are owned stage directories under the
configured state root, outside that repository. The state root itself must also
be outside the source repository, including for Building-only requests.
Source locators and manifests continue to identify the actual original files;
a separate execution directory does not replace source provenance.

The Store owns workflow records, research reports and immutable planning
artifacts. Stage directories use deterministic identities and ownership markers;
symlinks and foreign existing directories are rejected. They are not agent
worktrees. Creating research or planning artifacts does not dirty the original
repository. No new MCP server, network service or dependency is required.

Planning uses one configurable real model to produce a closed proposal with
requirements, acceptance criteria, scope/non-goals, decisions, risks, blocking
questions and bounded tasks. A plan ID and artifact digest identify the exact
proposal. `BuildReady` means the host validated finite fields, source/reference
identity, paths, dependencies, supported verification commands and the existing
DAG rules. It is not semantic approval, consensus or an executed test result.
A non-development proposal can finish Planning without becoming build-ready.

A known completed research report may still have `incomplete_evidence`; its gaps
are retained in Planning. Running, partial, cancelled or unknown research is not
accepted as a completed handoff. Building resolves the original referenced
research and its digest again and receives bounded evidence, requirements,
scope/non-goals and verification conditions in its actual node prompts.

## Development admission

Building requires all of these before its providers or node worktrees start:

- explicit Building intent and an exact valid persisted plan;
- no blocking planning questions and a mechanically executable task DAG;
- the same original workspace and source manifest used by Planning;
- a Git original with no staged, unstaged or untracked changes.

The existing worktree owner performs its locked clean recheck. No stash, reset,
automatic commit or dirty-source copy bypasses this gate. If Building is blocked,
the preceding research and plan remain available. A later explicit Building-only
request can reuse a still-current plan after a known pre-dispatch dirty block;
it does not rerun Research or Planning. Source drift requires a new plan.

Actual execution uses existing per-node worktrees, source/preimage checks,
dependency handoffs, verification receipts, checkpoints and settlement. Provider
completion is STOPPED; real gates and workspace-manifest proof are required for
SETTLED. Only worktrees with verified closure and ownership are cleaned up.
Unknown or interrupted outcomes retain their original IDs and evidence.

## Readback, restore and cancel

```text
/workflow-status wfl-EXACT-FLOW-ID
/workflow-cancel wfl-EXACT-FLOW-ID
```

Every workspace report retains its flow ID. Explicit status readback can restore
durable stage results into available panes after restarting the TUI. It does not
initialize a database, reconcile jobs, dispatch models or create directories.
Drafts and active tab selection survive pane switches within a running TUI;
they are not claimed as durable backend session state.

Cancellation is accepted only by the Gateway that currently owns that active
workflow. It signals and drains the current stage, preserves partial evidence,
and prevents later-stage admission. Another process reports that it does not
own the workflow rather than claiming it cancelled a remote process. Shutdown
uses the existing supervised cancellation/drain path.

Reusing a flow ID reads its original result; it never reissues models or worktree
execution, including after timeout/unknown. A new explicitly requested flow ID
represents a new run, even when its goal text is unchanged.

Canonical `make ci` uses deterministic fixtures and no web/model/human judge.
Actual provider capability, installed TUI/PTY behavior and semantic research
quality are separate evidence scopes.
