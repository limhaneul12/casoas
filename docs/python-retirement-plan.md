# Complete project-local Python retirement

Date: 2026-11-03. The user explicitly requested direct dogfooding followed by complete deletion of deprecated Python remnants. This scope is this repository's superseded Python implementation/environment and test-client code, not the computer's shared Python installations or support for analyzing external Python workspaces.

## Preconditions and evidence

- Fresh connected Rust MCP health succeeds; CLI models/status/permissions/jobs succeed.
- Fresh connected Rust DAG planning compiles eight nodes in three waves with zero blocking hazards.
- A bounded read-only Argon/Claude/Codex retirement review is running under stable request ID `cdx-python-retirement-review-20261103`; query the same ID without duplicate dispatch.
- Active `app/`, root Python tests, pyproject, uv.lock and .python-version are already retired. Rust product/config/plugin entrypoints are installed.

## Exact deletion boundary

After dogfood and independent deletion review:

- `.venv/` — obsolete project Python runtime/dependencies and external SDK test client environment.
- `.pytest_cache/`, `.ruff_cache/` — unused Python gate caches.
- `.omx/backups/python-oracle-before-rust-cutover-20261103/` — obsolete implementation/test/packaging archive; keep only its source hash list in the retirement manifest.
- `.omx/backups/python_dev_harness-before-rust-20261102/` — obsolete Python mechanical harness archive.
- All enumerated `.py`, `.pyc`, `.pyo`, `.pyi` helper artifacts under `.omx/evidence/`, including prior decision-pack and Rust migration MCP clients. Preserve their file hashes and existing JSON/log execution artifacts.

Record exact paths, sizes and hashes before deletion. Reject paths outside the repository, symlink directory roots, tracked runtime data, and non-enumerated candidates. Remove empty Python cache directories after files are deleted.

## Preservation boundary

Keep user job/handoff DBs and histories, all SQLite/SQL compatibility fixtures, `.omx/evidence` JSON/log/manifests, source hash provenance, protected installation configuration backups, Rust toolchains/Cargo data, all source outside the requested Python retirement boundary, and the pre-existing dirty/untracked work unrelated to Python.

## Minimal validation

Check no active Rust/CLI/MCP path depends on deletion candidates; update current documentation to reflect removal, not a still-present archive. Run the canonical `make ci` after deletion, then repeat connected MCP health/job readback and representative CLI commands. Verify candidate absence and zero project Python source/bytecode outside Git/Cargo build output. Preserve the distinction between executed checks and provider review/consensus.

Writer: root. Independent inventory: exploration_steward. Independent review: verification_steward before deletion. No new runtime abstraction or dependency is required.

## Related integrity-boundary repair

Post-deletion CI correctly caught the updated retirement skill text against a duplicate hardcoded SHA-256 in `lifecycle/assets.rs`, even though PROVENANCE.json was updated. Remove the duplicate hash authority: the compiled provenance manifest owns expected hashes, compiled embedded bytes must match it, and repository assets must byte-match the verified embedded bundle. Keep inventory, drift rejection, and all existing integrity gates. Add one negative content-drift regression; no dependencies or data contracts change.
