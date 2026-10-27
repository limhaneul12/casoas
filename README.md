# `codex-collab-mcp` — Gemini 4 Argon ↔ Claude Sonnet/Opus 5.5 ↔ OpenAI Codex Heterarchy MCP Bridge

Production MCP server enabling multi-polar **Heterarchy collaboration** between **Gemini 4 Argon (Antigravity)**, **Claude Sonnet 5.5 / Opus 5.5 Medium**, and **OpenAI Codex (`codex-cli`)**, engineered with receipt-verified routing and evidence scope tracking.

> **📖 한글 상세 사용 가이드 및 에이전트 스킬**: [`skills/codex-heterarchy/SKILL.md`](./skills/codex-heterarchy/SKILL.md)

---

## Quick Start (설치 및 스킬·MCP 원클릭 등록)

```bash
# 1. 저장소 클론 및 가상환경 설치 (uv 필요)
git clone https://github.com/limhaneul12/casoas.git
cd casoas
uv sync

# 2. Antigravity & Codex 글로벌 스킬(codex-heterarchy) 및 MCP 서버 자동 등록
uv run hcodex on

# 3. 런타임, 플러그인, 스킬, 활성 PID 및 핸드오프 인박스 상태 확인
uv run hcodex status
```

`uv run hcodex on` 실행 시 현재 클론된 경로를 기준으로 다음 항목이 자동 설정됩니다:
- **Antigravity 글로벌 스킬**: `~/.gemini/config/skills/codex-heterarchy/SKILL.md`
- **Codex 글로벌 스킬**: `~/.codex/skills/codex-heterarchy/SKILL.md`
- **Antigravity MCP 플러그인**: `~/.gemini/config/plugins/heterarchy-codex/mcp_config.json`
- **Codex MCP 설정**: `~/.codex/config.toml` (`[mcp_servers.heterarchy_codex]`)

---

## Architecture & Role Division

| Node | Role | Core Strengths |
| :--- | :--- | :--- |
| **Gemini 4 Argon (Antigravity)** | Macro-Architect & Orchestrator | Full-repository long-context awareness, cross-file invariants, terminal test execution, consensus adjudication |
| **Claude Sonnet 5.5 / Opus 5.5 Medium** | Surgical Code Craftsman | Clean module implementation, deep architectural refactoring, ergonomic API/UI design (`claude_code_craft`) |
| **OpenAI Codex (`codex-collab-mcp`)** | Adversarial Auditor & High-Speed Synthesizer | Zero-fluff pin-point code generation, red-team boundary auditing, multi-axis algorithm exploration, isolated rescue debugging |

---

## Exposed MCP Tools (15 Tools)

1. **`heterarchy_triad_orchestrate`**: End-to-end 3-pole orchestration (`mode="pipeline"` or `"council"`) with checkpoint resume (`resume_collaboration_id`), multi-round auto-repair (`max_rounds=1..3`), and bridge-executed verification gates (`run_verification_gates=True`).
2. **`claude_code_craft`**: Invokes Claude Sonnet 5.5 or Opus 5.5 Medium via local Antigravity RPC with strict tier policy enforcement (`CLAUDE_TIER_POLICY_VIOLATION` on non-Medium tiers).
3. **`codex_adversarial_audit`**: Red-team critique of code/diffs against Gemini 4 Argon's design rationale; returns structured `verdict` (`PASS` / `WARN` / `BLOCK`), `risk_score`, `findings`, executable `failing_test_cases`, and an `alexandria_decision_note`.
4. **`codex_targeted_synthesize`**: High-precision, zero-fluff code and boundary unit test synthesis from a distilled signature and contract.
5. **`codex_differential_implementations`**: Generates multiple distinct candidate implementations across orthogonal optimization axes (`latency_and_throughput`, `memory_footprint`, `maintainability_and_clarity`) with an `argon_selection_guide`.
6. **`codex_rescue_sandbox`**: Isolated root-cause analysis and patch generation for stubborn bugs or test failures (supports `async_background=True`).
7. **`codex_git_review`**: Native wrapper around `codex review` (`--uncommitted`, `--base`, `--commit`).
8. **`codex_session_followup`**: Multi-turn continuation (`resume`) or branching (`fork`) of an existing Codex session (`session_id`).
9. **`codex_job_status` / `codex_job_list`**: Status, receipt lookup, and safe cancellation (`cancel=True`) for asynchronous background and `timed_out_unknown` Codex tasks (`cdx-...`).
10. **`heterarchy_handoff_create` / `heterarchy_handoff_inbox` / `heterarchy_handoff_resolve`**: Shared blackboard ticket system across `argon`, `sonnet`, `opus`, and `codex`.
11. **`codex_alexandria_decision_pack`**: Packages a decision record with a full-content SHA-256 `idempotency_key`. Optional `collaboration_id` resolves a stored single triad run; only verified agreement receives a Consensus title. Calls without evidence remain neutral.
12. **`codex_bridge_health`**: Diagnostics for local `codex` CLI binary, Antigravity RPC routability (`routable_*`), authentication boundaries, and background job store.

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
