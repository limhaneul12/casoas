# Shared Agent Execution Contract

Canonical cross-language execution discipline for engineering agents using an adopted Engineering Harness.

## Rule loading order

1. current explicit user task and repository-local `AGENTS.md` / applicable overrides;
2. repository-local `PROJECT_PROFILE` / machine Harness config;
3. shared Harness authority contract;
4. applicable language normal/type rules;
5. optional async/framework/runtime/cross-language rules only when the touched surface activates them;
6. task-relevant Skill procedure;
7. live source/tests/build/CI evidence.

Do not load every Harness document for every task.

## Bounded scope

One work run has one concrete objective and bounded touched responsibilities. “Apply the Harness” is never permission for repository-wide unrelated refactoring.

Before editing, inspect the affected implementation, tests, public/type contracts, effect/resource boundaries, compatibility implications, and canonical verification entrypoints.

## Change discipline

- Preserve unrelated dirty worktree changes.
- Do not reset, revert, clean, or stash unrelated user work as routine cleanup.
- Prefer focused change → focused evidence → required broader gate.
- If a Harness defect blocks requested work, repair the Harness contract rather than distorting production code to satisfy a stale verifier.
- If a Rule and live architecture conflict, determine whether the Rule is stale or the implementation violates an intentional contract before changing either.
- Do not broaden scope merely because adjacent cleanup is possible.

## Multi-agent writing

- One tracked file or authority boundary has at most one active writer.
- Parallel writers require disjoint ownership and a known integration boundary.
- Read-only exploration/context work may run in parallel when useful.
- Independent verification should not rewrite the implementation, tests, fixtures, golden files, config, or acceptance criteria it is evaluating.

## Evidence states

Use: DISCOVERED, PLANNED, IMPLEMENTED, VERIFIED, BLOCKED.

Only actual executed evidence upgrades a claim to VERIFIED. A worker saying PASS is not final evidence.

## Completion report

Report meaningful changes, affected authority/type/runtime boundaries, exact validation performed, failures/unverified gates, and remaining scoped risk. Do not narrate routine exploration.

## Model separation

This contract intentionally does not prescribe a model name or reasoning level. Model routing is owned by repository/user policy and the model-specific agent package.
