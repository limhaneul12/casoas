# Rust 2024 migration compatibility inventory

This inventory records the observable contracts that a Rust 2024 `hcodex` rewrite must preserve. It is based on the current Python routers, CLI controller, repositories, and tests, plus the attached migration specification. Names below are compatibility surface, not an implementation prescription.

## MCP tool surface

`app/server.py` registers the following FastMCP tools (the registration test is `tests/test_server.py::test_fastmcp_registered_tools`):

| Tool | Required behavior / key inputs |
|---|---|
| `codex_adversarial_audit` | Read-only Codex red-team audit; target diff, rationale, attack vectors, context, model/effort/provider/engine, evidence scope, idempotency, optional background execution. |
| `codex_targeted_synthesize` | Generate implementation and optional tests from signature, contract, context types, language, model/effort and evidence scope. |
| `codex_differential_implementations` | Generate candidates across optimization axes (latency, memory, maintainability defaults), with receipt and evidence scope. |
| `codex_rescue_sandbox` | Diagnose a failure from trace, target code, prior attempts and reproduction command; honor read-only/workspace-write sandbox and async options. |
| `codex_git_review` | Native review of uncommitted tree, base branch, or commit SHA with custom instructions. |
| `codex_session_followup` | Resume or fork a Codex session by `thr_...` ID. |
| `codex_job_status` | Lookup by job ID or idempotency key; retrieve result/outcome; safe cancellation. |
| `codex_job_list` | List recent async jobs with optional workspace filter and limit. |
| `heterarchy_handoff_create` | Persist queued bidirectional ticket (`codex`, `argon`, `sonnet`, `opus`) with priority. Creation is not completion or consensus. |
| `heterarchy_handoff_inbox` | Filter tickets by recipient, status, workspace, limit; `None`/empty status means all. |
| `heterarchy_handoff_resolve` | Update `in_progress`, `completed`, or `rejected`; preserve resolution note, Alexandria reference, and structured `PASS/WARN/BLOCK` verdict. |
| `claude_code_craft` | Invoke Claude Sonnet/Opus medium tier through local RPC; task/context/rules, language, timeout, idempotency, inspected files/symbols, tests, budget, background mode. |
| `heterarchy_triad_orchestrate` | Coordinate Argon + Claude + Codex in pipeline/council; separate role state from consensus, support handoff, resume, bounded repair rounds, verification gates, async/idempotency. Pending or timed-out required role must never become consensus. |
| `codex_alexandria_decision_pack` | Build deterministic decision bundle from project/topic, Argon intent, Codex synthesis, Sonnet summary, resolution, evidence, collaboration/scope fences. |
| `heterarchy_dag_ulw` | Compile/execute DAG campaign; accept node list/spawn plan/resume ID; plan-only default; dynamic waves, WAW/RAW and CodeGraph hazards, spawn gate, two-stage `STOPPED` then `SETTLED`, workspace SHA-256 manifest. |
| `codex_bridge_health` | Report Codex binary, Antigravity endpoint/model probes, independent auth boundaries, active jobs, and pending handoff counts. |

Every execution result must retain a receipt containing `request_id`, provider, engine, requested/actual model, status, outcome state, output digest, duration, and token counters. Timeout is persisted as `timed_out_unknown`; it is queryable/cancellable and must not silently retry under a new ID.

## Router and service boundaries

- `app/api/codex_router.py`: six Codex tools above; prompt builders in `app/core/prompts.py`; schema dispatch through `McpExecutionGateway`.
- `app/api/triad_router.py`: Claude craft, triad orchestration, decision pack, DAG-ULW. DAG input models are closed Pydantic models (`DagNodeInputModel`, `DagSpawnPlanInputModel`, `DagUlwRequestModel`).
- `app/api/job_handoff_router.py`: job status/list, handoff CRUD, bridge health.
- `app/services/mcp_gateway_service.py`: route validation, pre-dispatch rejection, background task ownership, idempotent registration, schema validation, receipts, cancellation.
- `app/services/mcp_tool_handlers.py`: JSON response serialization for jobs/handoffs/health.
- `app/services/triad_pipeline_service.py` and `triad_execution_context.py`: role lifecycle (`not_started`, `queued`, `running`, `succeeded`, `failed`, `timed_out_unknown`) and consensus states (`consensus_verified`, `incomplete_no_consensus`, `conflict_requires_adjudication`, `blocked_by_red_team`).
- `app/services/dag_ulw_service.py`, `app/evidence/dag_hazard_analyzer.py`, `app/evidence/workspace_digests.py`: Kahn waves, dynamic topology, WAW/RAW/WAR and cycle hazards, scope/write fences, full manifest settlement.
- `app/core/skill_registry.py` and `app/evidence/codegraph_slicer.py`: load real `SKILL.md` bodies and source/symbol excerpts into model prompts; file names alone are insufficient.

## CLI and interactive command compatibility

`app/core/ctl.py::HeterarchyCliController.dispatch` accepts these top-level forms and aliases:

`help`, `status|st`, `on|enable|start`, `off|disable|stop`, `restart|reload`, `inbox|tickets`, `jobs|job`, `prune|compact|clean`, `tui|chat [-c|--command]`, `dag-mode`, `dag-exit`, `ask|prompt`, `sonnet`, `opus`, `argon`, `flash`, `dag-ulw|dag`, `dag-run|run`, `models|lanes|model`, and `skills|skill`.

Interactive `HeterarchyTuiSession` commands covered by `tests/test_dag_ulw_and_tui.py` include `/help`, `/dag-ulw`, `/dag-mode`, `/dag-exit`, `/omh`, `/prompts` and typo-compatible `/promt`, `/back`, `/models`, `/model`, `/skills`, `/skill`, `/effect`, `/effort`, `/status`, `/diff`, `/review`, `/clear`, `/quit`, `/jobs`, `/ask`, `/sonnet`, `/opus`, `/argon`, `/flash`, and `/dag-run`. Preserve completion for `/`, `$` skill mentions, and `@` workspace file mentions.

TUI behavior is contractual: fixed header, dynamic DAG cards, live delegate/tool transcript, permanently fixed multiline composer/status bar, truthful context/token counters, and readable Unicode width. Do not regress to a static four-node diamond or numbered text wall.

## Persistence and SQL compatibility

`JobStore`/`SqliteJobEngine` (`app/repositories/job_store.py`, `sqlite_job_engine.py`, `sql/job_store.sql`) must preserve lazy initialization, idempotent registration, CAS cancellation, stale-running reconciliation, result/receipt persistence, workspace filtering, retention pruning, WAL checkpoint, and lookup by job ID/idempotency key. The `jobs` table stores `job_id`, workspace, serialized record, status/index metadata, timestamps, and idempotency index.

`HandoffStore`/`SqliteHandoffEngine` (`handoff_store.py`, `sqlite_handoff_engine.py`, `sql/handoff_store.sql`) must preserve ticket IDs (`hnd-...`), sender/recipient/title/payload, priority/status, workspace, resolution note, verdict, Alexandria reference, timestamps, filtering, bidirectional inbox, retention pruning, and WAL checkpoint. Pending tickets are not consensus evidence.

The Rust rewrite must keep SQL in feature files (`sql/job_store.sql`, `sql/handoff_store.sql`, `sql/codegraph_slicer.sql`), use prepared bindings only, reject stacked statements and dangerous pragmas, reject NUL input, escape LIKE wildcards, and open external CodeGraph SQLite read-only/immutable with query-only mode.

## Test-derived acceptance checklist

- `tests/test_server.py`: strict JSON schemas; command builders and JSONL parser; job/handoff lifecycle; route rejection before provider call; timeout-unknown lookup/cancel; partial triad when Argon pending; conflict/decision authority; grant isolation and secret redaction; evidence scope and `not_run`; budget/health; decision pack; registered tools; Opus medium routing; lazy/free-threaded concurrent stores; closed Pydantic boundaries; checkpoint/resume; bridge-executed verification; CodeGraph slicing and persisted idempotency; retention; cancellation CAS/stale PID/process-group handling; triad resolution/multi-round repair; decision-pack/diff/slicer/session/async triad telemetry.
- `tests/test_dag_ulw_and_tui.py`: plan-only dynamic waves; WAW/RAW/cycle hazards; two-stage settlement and out-of-scope write rejection; reproduction hold/checkpoint resume; slash command and CLI entrypoints; live prompt/tree trace; effect/effort configuration.
- Rust gates required by the specification: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`, with focused tests for dynamic DAGs, actual skill/source injection, SQL injection defenses, and TUI layout width/fixed-footer behavior.

## Migration risk points

The highest-risk compatibility edges are closed schema/enum values, receipt and timeout semantics, lazy SQLite state paths, idempotency/CAS behavior, handoff-vs-consensus separation, dynamic DAG settlement, and the exact MCP/CLI names above. A Rust implementation should add contract fixtures against these Python outputs before deleting the Python runtime.
