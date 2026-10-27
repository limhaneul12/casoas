# `codex-collab-mcp` — Gemini 4 Argon ↔ OpenAI Codex Heterarchy MCP Bridge

Production MCP server enabling multi-polar **Heterarchy collaboration** between **Gemini 4 Argon (Antigravity)** and **OpenAI Codex (`codex-cli`)**, engineered under the **Heterarchy Alexandria Python Engineering Harness (`.agents/python_dev_harness`)**.

---

## Architecture & Role Division

| Node | Role | Core Strengths |
| :--- | :--- | :--- |
| **Gemini 4 Argon (Antigravity)** | Macro-Architect & Orchestrator | Full-repository long-context awareness, cross-file invariants, terminal test execution, Alexandria memory stewardship |
| **OpenAI Codex (`codex-collab-mcp`)** | Adversarial Auditor & High-Speed Synthesizer | Zero-fluff pin-point code generation, red-team boundary auditing, multi-axis algorithm exploration, isolated rescue debugging |

---

## Exposed MCP Tools

1. **`codex_adversarial_audit`**: Red-team critique of code/diffs against Gemini 4 Argon's design rationale; returns structured `verdict` (`PASS` / `WARN` / `BLOCK`), `risk_score`, `findings`, executable `failing_test_cases`, and an `alexandria_decision_note`.
2. **`codex_targeted_synthesize`**: High-precision, zero-fluff code and boundary unit test synthesis from a distilled signature and contract.
3. **`codex_differential_implementations`**: Generates multiple distinct candidate implementations across orthogonal optimization axes (`latency_and_throughput`, `memory_footprint`, `maintainability_and_clarity`) with an `argon_selection_guide`.
4. **`codex_rescue_sandbox`**: Isolated root-cause analysis and patch generation for stubborn bugs or test failures (supports `async_background=True`).
5. **`codex_git_review`**: Native wrapper around `codex review` (`--uncommitted`, `--base`, `--commit`).
6. **`codex_session_followup`**: Multi-turn continuation (`resume`) or branching (`fork`) of an existing Codex session (`session_id`).
7. **`codex_job_status` / `codex_job_list`**: Status and result retrieval for asynchronous background Codex tasks (`cdx-...`).
8. **`codex_alexandria_decision_pack`**: Packages a decision record with a full-content SHA-256 `idempotency_key`. Optional `collaboration_id` resolves a stored single triad run; only verified agreement receives a Consensus title. Calls without evidence remain neutral.
9. **`codex_bridge_health`**: Diagnostics for local `codex` CLI binary, version, and background job store.

---

## Verification & Harness Gates

```bash
uv run python .agents/python_dev_harness/verify_mechanical_contracts.py
uv run ruff check src tests
uv run pyrefly check
uv run pytest
```

## Review evidence and decision records

Triad outputs separate performed review from agreement: `static_code_review` is `verified` when every required role supplies a completed review artifact and explicit verdict, `partial` when only some do, and `not_run` when none do. WARN and BLOCK reviews still count as performed. Test generation, test execution, integration and runtime proof remain separate. A missing verdict or artifact never becomes an implicit PASS. `review_approved` requires all PASS; unanimous WARN can express agreement without approval.

A returned `collaboration_id` points to a compact stored snapshot of that one triad execution. It contains role states/verdicts, artifact and task digests, scope and receipt references, without duplicating raw prompts or outputs. Query it with `codex_job_status`; the enclosing storage job's completion does not imply role completion or agreement. This uses existing JobStore restart persistence, not a transactional crash-safety guarantee.

Pass that same ID, `workspace_dir`, and the original `evidence_scope.scope_digest` as `expected_scope_digest` to `codex_alexandria_decision_pack`. Missing or mismatched workspace/scope fences remain unverified. The reported status belongs to that source run; `resolution_verified=false` is retained for caller-written prose. Legacy calls without an ID and unavailable/invalid references receive a neutral Decision title. Caller-written resolution prose is not certified by a source-run consensus; inspect the recorded scope. Packaging does not upsert Alexandria automatically. Decision idempotency keys include all narrative fields and the evidence snapshot, excluding the display timestamp.

After source updates, existing stdio server processes may still contain old code and schemas. A fresh MCP connection loads the new source. `hcodex restart` syncs configuration/skills but does not reload existing stdio children; never bulk-stop unrelated hosts or active jobs to refresh one connection.

Review artifacts require an explicit verdict plus nonblank `summary` or `executive_critique`; a verdict token alone is insufficient. Caller-supplied `executed_test_commands` retain caller-reported attribution and never upgrade bridge test execution. The v2 compact snapshot records the stronger artifact contract; old v1 jobs remain queryable but cannot be retroactively certified by the new decision pack. A failed final aggregate write returns collected role results with persistence failure, without a usable collaboration ID. Invalid workspace directories are rejected before provider calls. Read-only pipeline audits retain the original source when no crafted code was returned.
