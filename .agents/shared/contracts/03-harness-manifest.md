# Harness Manifest Contract

Canonical identity/provenance contract for materialized Engineering Harness packages.

## Purpose

A repository-local Harness should be discoverable and auditable through an explicit manifest rather than directory-name heuristics alone. The manifest is for humans, coding agents, CI and read-only observers such as FORGE.

It does **not** grant execution permission and does not replace `PROJECT_PROFILE`, Rules, machine config, lockfiles, source/tests, CI, Goal/DAG or Writer Lease authority.

## Canonical filename

Every materialized package uses a package-local `HARNESS_MANIFEST.toml` inside its `.agents` Harness directory. The parent directory name is a discovery hint, not identity authority.

## Schema v1

```toml
schema_version = 1
family = "engineering"
package = "Rust_package"
language = "rust"

[identity]
source = "Alexandria"
source_path = "Jobs/Harness/Engineering Harness/02 Rust/"
materialized_at = "<RFC3339>"

[contracts]
project_profile = "PROJECT_PROFILE.md"
machine_config = "HARNESS.toml"
rules_router = "rules/README.md"

[shared]
contracts = ["authority", "testing", "agent-execution", "solution-minimality"]
skills = ["minimal-engineering"]

[activation]
baseline = ["normal", "type"]
optional = ["async"]

[skills]
entries = ["skills/rust-engineering/SKILL.md"]

[verification]
entrypoints = ["cargo xtask rules", "cargo xtask ci"]

[provenance]
manifest_hash_algorithm = "sha256"
# content_hash = "<GENERATED>"
```

Package-specific manifests may add fields but must not change the meaning of shared fields.

## Shared contract identities

Canonical Shared identifiers:

```text
authority           → Harness Authority and Layering Contract
testing             → Shared Testing and Verification Contract
agent-execution     → Shared Agent Execution Contract
solution-minimality → Solution Minimality Contract
minimal-engineering → Minimal Engineering Skill
```

The common Manifest contract itself is implied by the manifest/schema and does not need to list itself under `[shared].contracts`.

A materializer may embed/copy Shared assets into a common repository Shared Harness location or resolve them through another explicitly supported repository mechanism. It must not silently omit an adopted Shared contract while claiming a complete Harness materialization.

Do not duplicate divergent edited copies of Shared contracts inside every language package. One repository materialization has one canonical Shared owner.

## Required invariants

1. `schema_version`, `family`, `package` and language/composition identity are explicit.
2. Paths are normalized/repository-relative and cannot escape the repository.
3. Activated rule/profile families match project profile and machine config.
4. `shared.contracts` identifies adopted cross-language Shared policy.
5. `shared.skills` identifies shared procedure Skills such as Minimal Engineering.
6. Skill/verifier entrypoints name actual materialized assets/commands.
7. Provenance hashes are generated from actual materialized bytes, not copied from Alexandria Markdown wrappers.
8. Stale/mismatched manifest is a Harness integrity failure; do not guess missing values.
9. Manifest presence never means CI/verifiers have run.
10. Manifest presence never grants source-write/runtime authority.

## Solution Minimality materialization

When `solution-minimality` is adopted, the materialized Harness makes both the normative contract and procedure discoverable:

- `contracts/04-solution-minimality.md`
- `skills/minimal-engineering/SKILL.md`

Minimality remains subordinate to repository requirements, safety/correctness, language Harness invariants and accepted authority boundaries.

## Language packages

### Python
May additionally record async/Pydantic/framework profile activation and Python verifier entrypoints. Shared minimality never changes the dynamic-attribute prohibition or typed-boundary policy.

### Rust
May additionally record toolchain/MSRV, unsafe mode/approved crates, supply-chain lane and xtask surface. Shared minimality never converts ownership/soundness requirements into LOC targets.

### TypeScript
May additionally record TS7/TS6 tooling mode, React/Next activation, package-manager/runtime authority and verifier selection. Shared minimality may prefer native browser/framework capability where semantics permit it.

### Python-Rust
Records child Harness identities plus cross-language profile/config and ABI/runtime verification lanes. Shared minimality reinforces one-authority cutover and does not fork child-language policy.

## FORGE observation contract

Preferred read-only observation:

```text
bounded recursive discovery of HARNESS_MANIFEST.toml
→ parse/validate schema and paths
→ report package identity + shared contracts + activation + integrity
→ optionally inspect referenced project profile/config
```

FORGE remains an observer. It does not mutate Harness packages, silently repair manifests, or treat manifest presence as source-write permission.

## Adoption strategy

Introduce the common identity manifest incrementally. New materializations should record adopted Shared contracts/Skills. Existing language-specific integrity manifests may remain implementation details while `HARNESS_MANIFEST.toml` is the common package identity layer.
