# Harness Authority and Layering Contract

Shared canonical authority model for all Engineering Harness packages.

## Responsibility model

### Repository concrete authority

The live repository owns concrete execution facts:
- project-local `AGENTS.md` / closest applicable instructions;
- project profile and machine config;
- package/lock/toolchain/build configuration;
- source/tests/fixtures and public contracts;
- CI/runtime/deployment evidence;
- explicitly adopted non-optional Harness invariants.

Shared Jobs documents are reusable canonical methodology, not a substitute for live project facts.

### Rules

Normative engineering policy. Rules define what is required, allowed, forbidden, or conditionally permitted.

A Rule owns semantics. A verifier, Skill, AGENTS file, or model overlay must not silently create a stronger conflicting Rule.

### Project Profile / Harness config

Applicability and concrete project decisions. Profiles/config record project shape, activated optional rule families, runtime/build/toolchain policy, ownership, verification gates, and explicit exceptions only where the governing Rule permits them.

### Framework/runtime Profiles

Reusable specialization for real capabilities. They add to, but do not fork, the language baseline.

### Skills

Procedure and methodology. Skills explain **how** to execute work under governing Rules/Profile. They do not redefine policy.

### Solution Minimality

`04 Solution Minimality Contract` is a Shared normative decision contract applied **inside** accepted project/language authority.

It answers:

> What is the minimum correct semantic and authority surface needed to satisfy the already-accepted requirement and invariants?

It does not authorize changing the requirement, weakening security/correctness/accessibility, merging distinct authority boundaries, or optimizing for raw LOC/file count.

Procedure lives in `skills/minimal-engineering/SKILL.md`.

### Mechanical Verifiers

Bounded evidence implementations. They prove only what is reliably machine-checkable.

A verifier must not invent stricter policy than its governing Rule/Profile. If it does, that is Harness drift; production code is not distorted merely to satisfy the stale verifier.

### Tests / compiler / runtime evidence

Executed evidence of behavior/type/build/runtime claims. Existing commands or test files do not imply PASS unless actually run successfully.

### AGENTS / model-specific guidance

Loading, routing and orchestration. AGENTS selects the applicable Harness and Skills, defines approval/delegation behavior, and remains compact.

GPT-6 Astra/GPT-5.6 model guidance changes orchestration, not engineering semantics.

### FORGE

Runtime authority for Goal/DAG/Writer Lease/effect/evidence/ChangeSet/integration when FORGE is used. Harness policy never grants write/runtime authority by itself.

## Precedence model

For an engineering change:

```text
platform / safety / permissions
→ explicit user/product requirement
→ repository authority and accepted architecture
→ adopted language/cross-language Rules and non-optional invariants
→ Project Profile / machine configuration
→ Shared Solution Minimality within those constraints
→ task-relevant Skills / procedures
→ Verifiers and executed evidence
→ conventions inferred from live code
```

This is responsibility precedence, not permission to ignore a closer repository instruction that legitimately specializes a broader shared template.

## Shared invariants

- One concern has one canonical production owner after cutover.
- Preserve unrelated user changes.
- Do not use broad reset/clean/revert/stash as routine cleanup.
- Do not broaden scope merely because adjacent cleanup is visible.
- Evidence claims correspond to checks that actually ran.
- Missing required evidence is UNVERIFIED/BLOCKED, never synthetic PASS.
- Worker self-report is evidence input, not independent final acceptance.
- Rules remain authoritative over their mechanical verifier implementation.
- Skills do not become parallel Rule sources.
- Solution Minimality runs after understanding the actual flow and cannot weaken accepted invariants.
- Minimality optimizes semantic/authority surface, not raw LOC, file count, or shortest-looking diff.

## Minimality relation to other layers

```text
Requirement says WHAT must be true.
Rules/Profile say WHAT engineering constraints apply.
Minimality selects the smallest correct solution inside those constraints.
Skill defines HOW to search for that solution.
Verifier/tests prove WHAT can actually be evidenced.
```

If a one-line solution violates a typed/ownership/trust/accessibility contract, it loses. If an extra module/crate/service owns a real independent responsibility, it may be required even though it adds files.

## Language packages

- Python Engineering Harness Bundle
- Rust Engineering Harness Bundle
- TypeScript Engineering Harness Bundle
- Python and Rust Engineering Harness Bundle

All inherit Shared Authority, Testing, Agent Execution, Manifest and Solution Minimality contracts.
