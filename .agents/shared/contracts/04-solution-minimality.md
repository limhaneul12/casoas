# Solution Minimality Contract

Canonical cross-language minimal-solution policy for the Engineering Harness. Inspired by Ponytail's YAGNI/native-first/root-cause discipline, adapted to HETERARCHY authority and verification rules.

## Governing principle

> Understand deeply, then create the minimum semantic and authority surface that correctly satisfies the requirement.

Minimality means fewer responsibilities, dependencies, states, branches, ownership boundaries and maintenance obligations. It does **not** mean the fewest characters, fewest files, weakest validation, or smallest-looking diff.

## Precedence

Minimality is subordinate to higher-order contracts:

```text
explicit user/product requirement
→ security / correctness / data integrity / accessibility
→ language Harness invariants
→ repository authority and accepted architecture
→ Solution Minimality
→ convenience / LOC / aesthetic brevity
```

If a shorter solution violates an invariant, weakens a trust boundary, hides runtime authority, or makes debugging/refactoring less reliable, it is not minimal engineering.

## Understand before minimizing

Before choosing a solution:

1. read the affected instructions/profile and live code;
2. trace the relevant control/data/effect flow end to end;
3. identify the actual owner of the behavior/state/effect;
4. distinguish symptom, root cause and unrelated adjacent debt;
5. freeze the acceptance criteria.

Do not optimize for a small diff before understanding the layer that is actually wrong.

## Minimality ladder

Stop at the first rung that correctly satisfies the accepted contract:

1. **Does the change need to exist?** speculative/future-only work is omitted; an already-satisfied requirement produces no code change.
2. **Does the repository already own this capability?** reuse an existing helper, type, component, state machine, adapter, verifier, command, registry, or pattern when it is the correct authority.
3. **Does the language/runtime standard library own it?** prefer standard facilities when they satisfy semantics and operational requirements.
4. **Does the native platform/database/browser/framework own it?** prefer a real native constraint/API/capability to a custom imitation when the native contract is sufficient.
5. **Does an already-approved dependency own it?** reuse an accepted dependency when it cleanly solves the requirement.
6. **Can an existing authority boundary absorb it cleanly?** prefer adding a bounded rule/state/operation to the correct existing owner over creating a new service, manager, coordinator, interface, crate, module, store, abstraction or configuration layer.
7. **Only then create the minimum new authority surface.** create the smallest responsibility-bearing structure that preserves correctness, observability, testability and future modification clarity.

The ladder is a decision aid, not a substitute for repository inspection.

## Root-cause rule

A bug report usually names a symptom. Trace the bounded relevant callers/owners and fix the earliest correct shared authority point.

Prefer one correct fix at the true owner over duplicated guards at sibling call sites. Do not broaden the investigation into unrelated cleanup.

## Surgical scope

Every changed line should trace to the requested behavior, a dependency directly required by that behavior, cleanup made necessary by this change, or verification/evidence required by the adopted Harness.

Do not reformat, rewrite comments, delete unrelated dead code, rename neighboring symbols, or modernize adjacent code merely because it was noticed. Mention unrelated debt separately.

Deletion is preferred only when it removes obsolete authority, duplication, scaffolding, compatibility paths or artifacts made unnecessary by the accepted change. `Deletion over addition` is never permission for unrelated pruning.

## No speculative structure

Default deny interfaces/traits with one implementation and no real substitution boundary, factories for one fixed path, config with no operational need to vary, forwarding-only wrappers, ownerless service/manager/coordinator layers, generic frameworks for one use case, hypothetical scaffolding, and compatibility fallback without product contract.

These structures remain valid when a real responsibility, invariant, lifecycle, dependency inversion, public compatibility or multiple implementation requirement justifies them.

## Files and modules

Do **not** optimize for the fewest files. Use the fewest **new authority surfaces**. File/module/crate boundaries continue to follow the language Harness.

## Brevity

One line is not automatically better than ten. Prefer the shortest implementation that remains explicit about ownership and trust boundaries, readable, statically visible where required, debuggable/refactor-safe, and correct on relevant edge cases.

## Dependencies

New dependencies require a concrete capability gap. Before adding one, verify that the repository, standard library, native platform/framework, or an already-approved dependency does not already own the capability.

## Deliberate simple ceilings

A simple implementation with a known scale/performance limitation may be accepted when it satisfies current requirements and measurement supports the trade-off. Do not knowingly accept correctness, security, accessibility, data-loss or authority defects as a simple ceiling.

## Testing and verification

Minimality does not mean skipping evidence. Use the smallest evidence that proves behavior during iteration, then run the canonical final gates required by the project/Harness.

## Agent execution

Minimality never authorizes overlapping writers, broad cleanup, destructive Git operations, or unrequested scope expansion.

## Relationship to Ponytail

This contract adopts Ponytail's YAGNI, reuse, standard/native features, no speculative abstractions, root-cause fixes, and understanding-before-shortest-diff ideas. It does not adopt fewest-files or unconditional shortest-LOC optimization. HETERARCHY optimizes for **minimum correct semantic/authority surface**.

Reference: https://github.com/dietrichgebert/ponytail

## Completion question

> Did this change create any responsibility, dependency, state, abstraction, configuration or authority surface that the accepted requirement does not actually need?

If yes, remove or justify it before completion.
