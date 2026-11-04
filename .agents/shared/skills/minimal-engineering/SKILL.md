---
name: minimal-engineering
description: Cross-language procedure for finding the smallest correct solution after understanding the real flow. Use during implementation, bug fixing, refactoring, review, dependency selection, and architecture decisions to prevent speculative abstractions and unnecessary authority surfaces.
version: 1.0.0
researched_at: 2026-09-07
---

# Minimal Engineering

Procedure Skill implementing the Shared Solution Minimality Contract. Inspired by Ponytail, adapted for HETERARCHY Engineering Harness.

This Skill is a procedure. It does not override repository Rules, Project Profile, security/correctness requirements, or language Harness invariants.

## 1. Load the governing contract

Before minimizing, read the closest repository instructions, applicable project profile/machine config, Shared Solution Minimality Contract, affected language/runtime Rules, and Shared Testing/Agent Execution contracts as needed. Do not load unrelated Harnesses.

## 2. Restate the real requirement

Write the required observable outcome and constraints/non-negotiable invariants. Separate explicit requirements from speculative future possibilities. If the repository already satisfies the requirement, stop with no implementation change.

## 3. Trace before choosing

Read the affected flow end to end at bounded scope: entry point → decision/owner → state/effect boundary → output/observable behavior. For bugs, inspect callers/consumers sufficiently to locate the shared root cause.

## 4. Run the minimality ladder

```text
Need it at all?
↓
Existing repository capability?
↓
Standard library/runtime capability?
↓
Native platform/database/browser/framework capability?
↓
Already-approved dependency?
↓
Existing authority boundary can absorb it?
↓
Minimum new authority surface
```

Stop at the first rung that meets all accepted requirements and Harness invariants.

## 5. Challenge new structure

For every proposed new artifact, ask what distinct responsibility it owns. Challenge especially services/managers/coordinators, interfaces/traits, factories, configuration flags, wrappers/adapters, state/store/cache, modules/crates/packages, dependencies, compatibility paths, and test scaffolding. Future-flexibility alone is not sufficient justification.

## 6. Prefer native ownership

Subject to project authority, prefer database constraints for DB invariants, native browser APIs before custom widgets, Rust std/core/existing crates before new dependencies, explicit Python typed models/mappings/registries before reflection, and installed framework capabilities before custom imitation. Reject native options that fail required semantics, compatibility, security, accessibility or operability.

## 7. Keep the write surface surgical

Change only requested behavior, directly required dependencies/callers, code made obsolete by this change, and required tests/verifiers/evidence. Do not opportunistically reformat, rename, comment-edit, delete unrelated code, or modernize adjacent components.

## 8. Implement the minimum semantic surface

Prefer explicit code over clever compression; one authority over duplicate owners; one root-cause fix over sibling patches; deletion of obsolete authority over compatibility shims; existing data/state models over parallel representations; bounded additions to the correct existing owner over new layers.

## 9. Record deliberate ceilings

If the simplest correct solution has a meaningful scale/performance ceiling, record the assumption/limit, why acceptable now, and measurable trigger for redesign. Never use this for security, correctness, accessibility, data-integrity or authority defects.

## 10. Verify proportionally

During iteration run the smallest deterministic evidence that can fail when the changed behavior is wrong, then run canonical final gates. A minimal solution without required evidence is incomplete.

## 11. Minimality review

Ask whether any new responsibility, dependency, state/config, or authority can disappear; whether an existing authority can be reused; whether unrelated lines were touched; whether brevity hid ownership/debuggability; and whether correctness/security/accessibility contracts remain intact.

## Anti-patterns

Do not choose the smallest-looking diff before tracing the flow, delete unrelated code because deletion is preferred, merge responsibilities to reduce file count, replace explicit typed contracts with dynamic tricks, add familiar dependencies without need, build generic abstractions for one case, skip validation/error handling/security/accessibility as overengineering, keep old and new production authorities just in case, or add broad tests solely to justify implementation details.

## Upstream inspiration

Ponytail: https://github.com/dietrichgebert/ponytail
