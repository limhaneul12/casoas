"""Unit and contract tests for codex-collab-mcp (routing, receipts, timeouts, consensus, auth)."""

from __future__ import annotations

import asyncio
import contextlib
import json
import sqlite3
import sys
import urllib.error
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import httpx
import pytest

import codex_collab_mcp.server as server_mod
from codex_collab_mcp.agy_bridge import (
    AgyEndpoint,
    extract_json_from_model_text,
    run_agy_model_prompt,
)
from codex_collab_mcp.contracts import (
    adjudicate_collaboration_and_consensus,
    classify_stderr_and_auth_boundaries,
    get_python_runtime_info,
    redact_sensitive_text,
    validate_and_resolve_route,
)
from codex_collab_mcp.evidence.codegraph_slicer import slice_codegraph_symbols
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.prompts import (
    build_adversarial_audit_prompt,
    build_claude_craft_prompt,
    build_differential_candidates_prompt,
    build_rescue_prompt,
    build_targeted_synthesis_prompt,
)
from codex_collab_mcp.runner import (
    build_codex_exec_command,
    build_codex_review_command,
    parse_jsonl_events_with_model,
    run_codex_exec,
    run_http_fallback,
)
from codex_collab_mcp.schemas import (
    ADVERSARIAL_AUDIT_SCHEMA,
    DIFFERENTIAL_CANDIDATES_SCHEMA,
    RESCUE_DIAGNOSIS_SCHEMA,
    TARGETED_SYNTHESIS_SCHEMA,
    JsonObject,
    validate_structured_output,
)
from codex_collab_mcp.server import (
    claude_code_craft,
    codex_adversarial_audit,
    codex_alexandria_decision_pack,
    codex_bridge_health,
    codex_job_status,
    heterarchy_handoff_create,
    heterarchy_triad_orchestrate,
    mcp,
)
from codex_collab_mcp.transports.codex_aux_runners import CodexAuxiliaryRunner


@pytest.fixture(autouse=True)
def _isolate_server_stores(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Ensure server-level JobStore and HandoffStore never mutate the user's home directory."""
    monkeypatch.setattr(server_mod, "job_store", JobStore(state_dir=tmp_path / "_default_jobs"))
    monkeypatch.setattr(
        server_mod, "handoff_store", HandoffStore(state_dir=tmp_path / "_default_handoffs")
    )


def _assert_strict_openai_json_schema(schema: JsonObject) -> None:
    """Recursively verify that every object node in a JSON schema satisfies OpenAI strict mode."""
    if schema.get("type") == "object":
        assert schema.get("additionalProperties") is False, (
            f"Object schema missing additionalProperties=False: {schema}"
        )
        props = schema.get("properties")
        req = schema.get("required")
        assert isinstance(props, dict)
        assert isinstance(req, list)
        assert set(props.keys()) == set(req), (
            f"Object schema required keys {req} != properties keys {list(props.keys())}"
        )
        for sub in props.values():
            if isinstance(sub, dict):
                _assert_strict_openai_json_schema(sub)
    elif schema.get("type") == "array":
        items = schema.get("items")
        if isinstance(items, dict):
            _assert_strict_openai_json_schema(items)


@pytest.mark.parametrize(
    "schema",
    [
        ADVERSARIAL_AUDIT_SCHEMA,
        TARGETED_SYNTHESIS_SCHEMA,
        DIFFERENTIAL_CANDIDATES_SCHEMA,
        RESCUE_DIAGNOSIS_SCHEMA,
    ],
)
def test_schemas_are_strict_openai_compatible(schema: JsonObject) -> None:
    _assert_strict_openai_json_schema(schema)


def test_build_codex_exec_command(tmp_path: Path) -> None:
    schema_path = str(tmp_path / "schema.json")
    cmd = build_codex_exec_command(
        codex_bin="/opt/homebrew/bin/codex",
        workspace_dir=str(tmp_path / "work"),
        output_file=str(tmp_path / "out.txt"),
        schema_file=schema_path,
        sandbox_mode="read-only",
        model="gpt-6.1-sol",
        reasoning_effort="xhigh",
    )
    assert cmd[0] == "/opt/homebrew/bin/codex"
    assert cmd[1] == "exec"
    assert "--ignore-user-config" in cmd
    assert "mcp_servers={}" in cmd
    assert "--output-schema" in cmd
    assert schema_path in cmd
    assert "-m" in cmd
    assert "gpt-6.1-sol" in cmd
    assert 'model_reasoning_effort="xhigh"' in cmd
    assert cmd[-1] == "-"


def test_build_codex_review_command() -> None:
    cmd = build_codex_review_command(
        codex_bin="/opt/homebrew/bin/codex",
        uncommitted=False,
        base_branch="main",
        model="gpt-6.1-sol",
        has_custom_prompt=True,
    )
    assert cmd[:4] == ["/opt/homebrew/bin/codex", "review", "--base", "main"]
    assert "mcp_servers={}" in cmd
    assert cmd[-1] == "-"


def test_parse_jsonl_events() -> None:
    sample_stdout = "\n".join(
        [
            json.dumps({"type": "thread.started", "thread_id": "thr_abc123"}),
            json.dumps(
                {
                    "type": "item.completed",
                    "item": {"type": "agent_message", "text": '{"verdict": "PASS"}'},
                }
            ),
        ]
    )
    session_id, messages, reported_model = parse_jsonl_events_with_model(sample_stdout)
    assert session_id == "thr_abc123"
    assert messages == ['{"verdict": "PASS"}']
    assert reported_model is None


def test_job_store_lifecycle(tmp_path: Path) -> None:
    repo_dir = str(tmp_path / "repo")
    store = JobStore(state_dir=tmp_path)
    rec = store.create_job(
        task_type="adversarial_audit",
        workspace_dir=repo_dir,
        model="gpt-6.1-sol",
    )
    assert rec.status == "running"
    assert rec.outcome_state == "in_flight"
    assert rec.job_id.startswith("cdx-")

    completed = store.complete_job(
        rec.job_id,
        result={"verdict": "PASS"},
        session_id="thr_999",
    )
    assert completed is not None
    assert completed.status == "completed"
    assert completed.outcome_state == "confirmed_success"
    assert completed.session_id == "thr_999"

    persisted = store.get_persisted_job(rec.job_id)
    assert persisted is not None
    assert persisted.status == "completed"
    assert persisted.result == {"verdict": "PASS"}

    fresh_store = JobStore(state_dir=tmp_path)
    loaded = fresh_store.get_job(rec.job_id)
    assert loaded is not None
    assert loaded.status == "completed"
    assert loaded.result == {"verdict": "PASS"}

    lru_store = JobStore(state_dir=tmp_path / "lru", max_cached_jobs=2)
    first = lru_store.create_job(task_type="audit", workspace_dir=repo_dir)
    second = lru_store.create_job(task_type="audit", workspace_dir=repo_dir)
    third = lru_store.create_job(task_type="audit", workspace_dir=repo_dir)
    assert len(lru_store._jobs) == 2
    assert first.job_id not in lru_store._jobs
    assert second.job_id in lru_store._jobs
    assert third.job_id in lru_store._jobs
    reloaded_first = lru_store.get_job(first.job_id)
    assert reloaded_first is not None
    assert reloaded_first.job_id == first.job_id


def test_handoff_store_bidirectional_lifecycle(tmp_path: Path) -> None:
    repo_dir = str(tmp_path / "repo")
    other_dir = str(tmp_path / "other-project")
    store = HandoffStore(state_dir=tmp_path)
    ticket = store.create_ticket(
        sender="codex",
        recipient="argon",
        title="Implement zero-copy frame parser",
        spec_or_payload="Use memoryview slice without copying payload bytes.",
        workspace_dir=repo_dir,
        priority="high",
    )
    assert ticket.ticket_id.startswith("hnd-")
    assert ticket.status == "pending"
    t_dict = ticket.to_dict()
    assert t_dict["collaboration_role_state"] == "queued"
    assert t_dict["execution_completed"] is False
    assert t_dict["consensus_eligible"] is False

    argon_inbox = store.list_tickets(recipient="argon", status="pending")
    assert len(argon_inbox) == 1
    assert argon_inbox[0]["ticket_id"] == ticket.ticket_id
    assert store.count_pending_by_recipient(("argon", "sonnet", "opus", "codex")) == {
        "argon": 1,
        "sonnet": 0,
        "opus": 0,
        "codex": 0,
    }

    # Multi-project workspace filtering check
    assert len(store.list_tickets(workspace_dir=repo_dir)) == 1
    assert len(store.list_tickets(workspace_dir=other_dir)) == 0

    resolved = store.update_ticket_status(
        ticket_id=ticket.ticket_id,
        status="completed",
        resolution_note="Implemented and verified via pytest.",
        alexandria_ref="heterarchy-core/decisions/abc123",
    )
    assert resolved is not None
    assert resolved.status == "completed"
    resolved_dict = resolved.to_dict()
    assert resolved_dict["collaboration_role_state"] == "succeeded"
    assert resolved_dict["execution_completed"] is True
    assert resolved_dict["consensus_eligible"] is True
    assert resolved.alexandria_ref == "heterarchy-core/decisions/abc123"
    assert store.count_pending_by_recipient(("argon", "sonnet", "opus", "codex")) == {
        "argon": 0,
        "sonnet": 0,
        "opus": 0,
        "codex": 0,
    }


def test_prompt_builders() -> None:
    p1 = build_adversarial_audit_prompt("def f(): pass", "must be atomic", ["concurrency"])
    assert "CODEX ADVERSARIAL RED-TEAM AUDITOR" in p1
    assert "concurrency" in p1

    p2 = build_targeted_synthesis_prompt("def parse(x: str) -> int:", "no regex", "", "python")
    assert "CODEX HIGH-PRECISION SYNTHESIS ENGINE" in p2

    p3 = build_differential_candidates_prompt("LRU cache", ["latency", "memory"], "", "rust")
    assert "Axis 1: latency" in p3
    assert "Axis 2: memory" in p3

    p4 = build_rescue_prompt("AssertionError", "def broken(): pass", "tried lock")
    assert "CODEX DEEP RESCUE" in p4

    p5 = build_claude_craft_prompt("Refactor router", "def route(): pass", "no globals", "python")
    assert "CLAUDE SONNET SURGICAL CODE CRAFTSMAN" in p5


def test_extract_json_from_model_text() -> None:
    fenced = '```json\n{"summary": "clean", "crafted_code": "x = 1"}\n```'
    parsed = extract_json_from_model_text(fenced)
    assert parsed == {"summary": "clean", "crafted_code": "x = 1"}


# ---------------------------------------------------------------------------
# Mandatory Contract Verification Tests (Sky Backcraft Failure Reproductions)
# ---------------------------------------------------------------------------


def test_contract_1_routing_validation_and_rejection() -> None:
    """Verify supported routes pass and mismatched routes fail pre-dispatch."""
    # Supported Claude Medium route
    r_claude = validate_and_resolve_route(
        engine="antigravity_local_rpc",
        model="claude-sonnet-5-5-medium",
        provider="antigravity",
    )
    assert r_claude.valid is True
    assert r_claude.provider == "antigravity"
    assert r_claude.engine == "antigravity_local_rpc"
    assert r_claude.resolved_model == "claude-sonnet-5-5-medium"
    assert r_claude.placeholder == "MODEL_PLACEHOLDER_M404"

    # Supported Argon route
    r_argon = validate_and_resolve_route(
        engine="antigravity_local_rpc",
        model="gemini-4-argon-high",
        provider="antigravity",
    )
    assert r_argon.valid is True
    assert r_argon.resolved_model == "gemini-4-argon-high"
    assert r_argon.placeholder == "MODEL_PLACEHOLDER_M417"

    # Supported Codex route
    r_codex = validate_and_resolve_route(
        engine="codex_cli",
        model="gpt-6.1-sol",
        provider="openai_codex",
        reasoning_effort="xhigh",
    )
    assert r_codex.valid is True
    assert r_codex.provider == "openai_codex"
    assert r_codex.engine == "codex_cli"
    assert r_codex.resolved_model == "gpt-6.1-sol"

    # Sky Backcraft reproduction (`argon-mtf-cli-engine-rejection.json`):
    # Passing `gemini-4-argon-high` to `codex_cli` must be rejected pre-dispatch!
    r_mismatch = validate_and_resolve_route(
        engine="cli",
        model="gemini-4-argon-high",
    )
    assert r_mismatch.valid is False
    assert r_mismatch.error_code == "MODEL_NOT_SUPPORTED_BY_ENGINE"

    # Passing `provider="antigravity"` to `codex_cli` must be rejected pre-dispatch!
    r_prov_mismatch = validate_and_resolve_route(
        engine="cli",
        model="gpt-6.1-sol",
        provider="antigravity",
    )
    assert r_prov_mismatch.valid is False
    assert r_prov_mismatch.error_code == "PROVIDER_ENGINE_MISMATCH"

    # Non-Medium Claude tier must be rejected rather than silently rewritten
    r_claude_high = validate_and_resolve_route(
        engine="antigravity_local_rpc",
        model="claude-sonnet-5-5-high",
    )
    assert r_claude_high.valid is False
    assert r_claude_high.error_code == "CLAUDE_TIER_POLICY_VIOLATION"


@pytest.mark.asyncio
async def test_contract_1_and_2_supported_routes_and_receipts_with_mock_transports(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Prove Claude, Argon, and Codex routes produce matching receipts and detect mismatch."""
    store = JobStore(state_dir=tmp_path)
    fake_ep = AgyEndpoint(
        base_url="https://127.0.0.1:59999",
        csrf_token="secret-csrf-token",  # noqa: S106
        pid=11,
    )

    def mock_rpc(
        base_url: str,
        csrf_token: str,
        method: str,
        payload: JsonObject,
        timeout_seconds: float,
    ) -> JsonObject:
        if method == "GetCascadeModelConfigData":
            return {
                "clientModelConfigs": [
                    {
                        "label": "Claude Sonnet 5.5 (Medium)",
                        "modelId": "claude-sonnet-5-5-medium",
                        "modelOrAlias": {"model": "MODEL_PLACEHOLDER_M404"},
                        "quotaInfo": {"remainingFraction": 0.9},
                    },
                    {
                        "label": "Gemini 4 Argon (High)",
                        "modelId": "gemini-4-argon-high",
                        "modelOrAlias": {"model": "MODEL_PLACEHOLDER_M417"},
                        "quotaInfo": {"remainingFraction": 0.8},
                    },
                ]
            }
        return {"response": '{"summary": "ok", "crafted_code": "def x(): return 1"}'}

    claude_res = await run_agy_model_prompt(
        prompt="Craft x",
        model="claude-sonnet-5-5-medium",
        job_store=store,
        endpoint=fake_ep,
        rpc_transport=mock_rpc,
    )
    assert claude_res["ok"] is True
    assert claude_res["status"] == "succeeded"
    c_receipt = claude_res["receipt"]
    assert isinstance(c_receipt, dict)
    assert c_receipt["provider"] == "antigravity"
    assert c_receipt["engine"] == "antigravity_local_rpc"
    assert c_receipt["requested_model"] == "claude-sonnet-5-5-medium"
    assert c_receipt["actual_model"] == "claude-sonnet-5-5-medium"
    assert c_receipt["model_mismatch"] is False
    assert str(c_receipt["output_digest"]).startswith("sha256:")
    assert c_receipt["credentials_redacted"] is True
    assert "secret-csrf-token" not in json.dumps(claude_res)

    argon_res = await run_agy_model_prompt(
        prompt="Review architecture",
        model="gemini-4-argon-high",
        job_store=store,
        endpoint=fake_ep,
        rpc_transport=mock_rpc,
    )
    assert argon_res["ok"] is True
    a_receipt = argon_res["receipt"]
    assert isinstance(a_receipt, dict)
    assert a_receipt["actual_model"] == "gemini-4-argon-high"
    assert a_receipt["model_mismatch"] is False

    # Verify explicit failure when actual_model != requested_model
    def mismatch_rpc(
        base_url: str,
        csrf_token: str,
        method: str,
        payload: JsonObject,
        timeout_seconds: float,
    ) -> JsonObject:
        if method == "GetCascadeModelConfigData":
            return {"clientModelConfigs": []}
        return {
            "modelId": "claude-sonnet-5-5-medium",
            "response": '{"verdict": "PASS"}',
        }

    mismatch_res = await run_agy_model_prompt(
        prompt="Review as Argon",
        model="gemini-4-argon-high",
        job_store=store,
        endpoint=fake_ep,
        rpc_transport=mismatch_rpc,
    )
    assert mismatch_res["ok"] is False
    assert mismatch_res["error_code"] == "ACTUAL_MODEL_MISMATCH"
    m_receipt = mismatch_res["receipt"]
    assert isinstance(m_receipt, dict)
    assert m_receipt["model_mismatch"] is True


@pytest.mark.asyncio
async def test_contract_2_pre_dispatch_rejection_never_calls_codex_cli(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Reproduce `argon-mtf-cli-engine-rejection.json` and prove pre-dispatch rejection."""
    subprocess_calls: list[int] = []

    async def forbidden_subprocess(*args: object, **kwargs: object) -> object:
        subprocess_calls.append(1)
        raise AssertionError("subprocess must NOT be called on pre-dispatch routing rejection")

    monkeypatch.setattr(asyncio, "create_subprocess_exec", forbidden_subprocess)

    raw_json = await codex_adversarial_audit(
        target_code_or_diff="def mtf(): pass",
        model="gemini-4-argon-high",
        engine="cli",
    )
    res = json.loads(raw_json)
    assert len(subprocess_calls) == 0
    assert res["ok"] is False
    assert res["status"] == "rejected_pre_dispatch"
    assert res["error_code"] == "MODEL_NOT_SUPPORTED_BY_ENGINE"
    assert res["receipt"]["status"] == "rejected_pre_dispatch"
    assert res["receipt"]["outcome_state"] == "confirmed_failure"


@pytest.mark.asyncio
async def test_contract_3_timeout_unknown_lookup_idempotency_and_cancel(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Reproduce Argon RPC read timeouts (`argon-mtf-rpc-timeout.json`) as queryable state."""
    isolated_store = JobStore(state_dir=tmp_path)
    monkeypatch.setattr(server_mod, "job_store", isolated_store)
    fake_ep = AgyEndpoint(
        base_url="https://127.0.0.1:59999",
        csrf_token="tok-secret",  # noqa: S106
        pid=22,
    )

    rpc_call_count = 0

    def timeout_rpc(
        base_url: str,
        csrf_token: str,
        method: str,
        payload: JsonObject,
        timeout_seconds: float,
    ) -> JsonObject:
        nonlocal rpc_call_count
        if method == "GetCascadeModelConfigData":
            return {"clientModelConfigs": []}
        rpc_call_count += 1
        raise urllib.error.URLError(TimeoutError("read timed out after 45.13s"))

    res1 = await run_agy_model_prompt(
        prompt="Bounded continuation review",
        model="gemini-4-argon-high",
        timeout_seconds=45.0,
        idempotency_key="sky-backcraft-argon-mtf-01",
        job_store=isolated_store,
        endpoint=fake_ep,
        rpc_transport=timeout_rpc,
    )
    assert rpc_call_count == 1
    assert res1["ok"] is False
    assert res1["status"] == "timed_out_unknown"
    assert res1["outcome_state"] == "outcome_unknown"
    req_id = str(res1["request_id"])
    assert req_id.startswith("cdx-")
    r1_receipt = res1["receipt"]
    r1_budget = res1["budget"]
    assert isinstance(r1_receipt, dict)
    assert isinstance(r1_budget, dict)
    assert r1_receipt["status"] == "timed_out_unknown"
    assert r1_receipt["outcome_state"] == "outcome_unknown"
    assert r1_budget["termination_reason"] == "timeout_budget_reached"

    # Query status by same request_id
    status_json = await codex_job_status(job_id=req_id)
    status_data = json.loads(status_json)
    assert status_data["ok"] is True
    assert status_data["job"]["job_id"] == req_id
    assert status_data["job"]["status"] == "timed_out_unknown"
    assert status_data["outcome_state"] == "outcome_unknown"

    # Query status by idempotency_key
    idem_json = await codex_job_status(idempotency_key="sky-backcraft-argon-mtf-01")
    idem_data = json.loads(idem_json)
    assert idem_data["ok"] is True
    assert idem_data["job"]["job_id"] == req_id

    # Re-calling with same idempotency_key WITHOUT retry_same_request=True must NOT re-dispatch!
    res_dup = await run_agy_model_prompt(
        prompt="Bounded continuation review",
        model="gemini-4-argon-high",
        timeout_seconds=45.0,
        idempotency_key="sky-backcraft-argon-mtf-01",
        job_store=isolated_store,
        endpoint=fake_ep,
        rpc_transport=timeout_rpc,
    )
    assert rpc_call_count == 1  # No duplicate provider dispatch!
    assert res_dup["deduplicated_existing_request"] is True
    assert res_dup["request_id"] == req_id
    assert res_dup["status"] == "timed_out_unknown"

    # Explicit retry with retry_same_request=True reuses the SAME request_id and increments attempt
    res_retry = await run_agy_model_prompt(
        prompt="Bounded continuation review",
        model="gemini-4-argon-high",
        timeout_seconds=45.0,
        idempotency_key="sky-backcraft-argon-mtf-01",
        retry_same_request=True,
        job_store=isolated_store,
        endpoint=fake_ep,
        rpc_transport=timeout_rpc,
    )
    assert rpc_call_count == 2
    assert res_retry["request_id"] == req_id
    retry_receipt = res_retry["receipt"]
    assert isinstance(retry_receipt, dict)
    assert retry_receipt["attempt_count"] == 2

    # Safe cancellation on the same request_id
    cancel_json = await codex_job_status(job_id=req_id, cancel=True)
    cancel_data = json.loads(cancel_json)
    assert cancel_data["ok"] is True
    assert cancel_data["cancel_requested"] is True
    assert cancel_data["job"]["status"] == "cancelled"


@pytest.mark.asyncio
async def test_contract_4_handoff_pending_and_argon_timeout_yield_partial_not_consensus(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Prove pending handoff (`hnd-...`) or Argon timeout produces `partial`, never `consensus`."""
    isolated_jobs = JobStore(state_dir=tmp_path / "jobs")
    isolated_handoffs = HandoffStore(state_dir=tmp_path / "handoffs")
    monkeypatch.setattr(server_mod, "job_store", isolated_jobs)
    monkeypatch.setattr(server_mod, "handoff_store", isolated_handoffs)

    # 1. Create a handoff ticket (reproducing `argon-readonly-handoff.json`)
    hnd_raw = await heterarchy_handoff_create(
        title="Argon read-only review of Sky Backcraft continuation",
        spec_or_payload="Verify window delta invariants",
        sender="codex",
        recipient="argon",
        workspace_dir=str(tmp_path),
    )
    hnd_res = json.loads(hnd_raw)
    assert hnd_res["collaboration_role_state"] == "queued"
    assert hnd_res["execution_completed"] is False
    assert hnd_res["consensus_eligible"] is False
    ticket_id = str(hnd_res["ticket"]["ticket_id"])

    # Mock Sonnet (succeeds) and Codex (succeeds with PASS), while Argon is queued via handoff
    async def mock_agy_prompt(
        prompt: str,
        model: str = "claude-sonnet-5-5-medium",
        expect_json: bool = True,
        timeout_seconds: float = 180.0,
        **kwargs: object,
    ) -> JsonObject:
        if "argon" in model.lower():
            return {
                "ok": False,
                "status": "timed_out_unknown",
                "outcome_state": "outcome_unknown",
                "request_id": "cdx-argontimeout",
                "model_id": model,
                "error": "Antigravity RPC read timed out after 75.12s",
            }
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-sonnetok",
            "model_id": "claude-sonnet-5-5-medium",
            "structured_output": {
                "summary": "Refactored continuation window",
                "crafted_code": "def continuation(): return True",
                "handoff_notes_for_codex_audit": "Check window bounds",
            },
            "raw_output": '{"summary": "Refactored continuation window"}',
        }

    async def mock_codex_engine(
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
        **kwargs: object,
    ) -> JsonObject:
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-codexok",
            "model": "gpt-6.1-sol",
            "structured_output": {
                "verdict": "PASS",
                "failing_test_cases": ["def test_edge(): assert True"],
            },
        }

    monkeypatch.setattr(server_mod, "run_agy_model_prompt", mock_agy_prompt)
    monkeypatch.setattr(server_mod, "_execute_with_engine", mock_codex_engine)

    # Case A: Handoff ticket is still `pending` (`queued`)
    orch_pending_raw = await heterarchy_triad_orchestrate(
        task_spec="Verify continuation window delta",
        workspace_dir=str(tmp_path),
        argon_handoff_ticket_id=ticket_id,
    )
    orch_pending = json.loads(orch_pending_raw)
    assert orch_pending["ok"] is False
    assert orch_pending["collaboration_status"] == "partial"
    assert orch_pending["consensus_status"] == "incomplete_no_consensus"
    assert orch_pending["consensus_reached"] is False
    assert orch_pending["triad_consensus_ready"] is False
    assert orch_pending["adjudication"]["incomplete_roles"] == ["argon"]
    assert orch_pending["adjudication"]["role_states"]["argon"]["state"] == "queued"

    # Case B: Claude and Codex succeed, Argon RPC times out (`timed_out_unknown`)
    orch_timeout_raw = await heterarchy_triad_orchestrate(
        task_spec="Verify continuation window delta",
        workspace_dir=str(tmp_path),
        invoke_argon_rpc=True,
    )
    orch_timeout = json.loads(orch_timeout_raw)
    assert orch_timeout["ok"] is False
    assert orch_timeout["collaboration_status"] == "partial"
    assert orch_timeout["consensus_status"] == "incomplete_no_consensus"
    assert orch_timeout["consensus_reached"] is False
    assert orch_timeout["triad_consensus_ready"] is False
    assert orch_timeout["adjudication"]["incomplete_roles"] == ["argon"]
    assert orch_timeout["adjudication"]["role_states"]["argon"]["state"] == "timed_out_unknown"
    # Partial outputs from Sonnet and Codex are preserved!
    assert orch_timeout["sonnet_craft"]["ok"] is True
    assert orch_timeout["codex_audit"]["ok"] is True


def test_contract_4_conflict_detection_and_decision_authority() -> None:
    """Verify conflicting verdicts across completed roles surface conflict_basis and authority."""
    adj = adjudicate_collaboration_and_consensus(
        required_roles=["argon", "sonnet", "codex"],
        role_states={"argon": "succeeded", "sonnet": "succeeded", "codex": "succeeded"},
        role_verdicts={"argon": "PASS", "sonnet": "PASS", "codex": "BLOCK"},
        role_outputs={"argon": {"ok": True}, "sonnet": {"ok": True}, "codex": {"ok": True}},
        decision_authority="gemini-4-argon",
    )
    assert adj["collaboration_status"] == "complete"
    assert adj["conflict_detected"] is True
    assert adj["consensus_status"] == "conflict_requires_adjudication"
    assert adj["consensus_reached"] is False
    assert "codex=BLOCK" in str(adj["conflict_basis"])
    assert adj["decision_authority"] == "gemini-4-argon"


def test_contract_5_invalid_grant_isolation_and_secret_redaction() -> None:
    """Prove external MCP `invalid_grant` does not alter Antigravity RPC and redacts secrets."""
    sample_stderr = (
        "2026-10-19T13:13:04Z WARN codex_rmcp_client::oauth::refresh_transaction: "
        "failed to refresh OAuth token for server heterarchy-alexandria: invalid_grant\n"
        "Launching language_server --csrf_token super-secret-csrf-12345 "
        "Authorization: Bearer sk-live-9876543210abcdef\n"
        "ERROR: The 'gemini-4-argon-high' model is not supported when using Codex "
        "with a ChatGPT account."
    )
    diag = classify_stderr_and_auth_boundaries(sample_stderr)
    assert diag["antigravity_rpc_affected"] is False
    assert diag["auto_credential_refresh_performed"] is False
    warnings = diag["isolated_external_mcp_oauth_warnings"]
    assert isinstance(warnings, list)
    assert len(warnings) == 1
    first_warn = warnings[0]
    assert isinstance(first_warn, dict)
    assert first_warn["external_mcp_server"] == "heterarchy-alexandria"
    assert first_warn["affects_antigravity_rpc"] is False
    assert first_warn["auto_reauth_triggered"] is False

    serialized = json.dumps(diag)
    assert "super-secret-csrf-12345" not in serialized
    assert "sk-live-9876543210abcdef" not in serialized
    assert "***REDACTED***" in redact_sensitive_text(sample_stderr)


@pytest.mark.asyncio
async def test_contract_6_evidence_scope_digests_and_not_run_dimensions(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Verify CodeGraph snapshot, file SHA-256 digests, and `not_run` test execution dimensions."""
    sample_file = tmp_path / "module_under_test.py"
    sample_file.write_text("def compute_delta(a: int, b: int) -> int:\n    return a - b\n")

    async def mock_codex_engine(
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
        **kwargs: object,
    ) -> JsonObject:
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-evid01",
            "model": "gpt-6.1-sol",
            "structured_output": {
                "verdict": "PASS",
                "executive_critique": "Reviewed the supplied subtraction function and edge cases.",
                "failing_test_cases": ["def test_compute_delta(): assert compute_delta(5, 3) == 2"],
            },
        }

    monkeypatch.setattr(server_mod, "_execute_with_engine", mock_codex_engine)

    audit_raw = await codex_adversarial_audit(
        target_code_or_diff="def compute_delta(a: int, b: int) -> int: return a - b",
        workspace_dir=str(tmp_path),
        files_inspected=["module_under_test.py", "nonexistent_file.py"],
        symbols_inspected=["compute_delta"],
    )
    audit_res = json.loads(audit_raw)
    ev = audit_res["evidence_scope"]
    assert ev["codegraph"]["status"] == "unindexed"
    assert str(ev["scope_digest"]).startswith("sha256:")
    assert "module_under_test.py" in ev["files_inspected_digests"]
    assert "nonexistent_file.py" in ev["unverified_or_missing_files"]
    assert ev["symbols_inspected"] == ["compute_delta"]
    dims = ev["evidence_dimensions"]
    assert dims["static_code_review"] == "verified"
    assert dims["test_generation"] == "verified"
    # Because no test command was actually executed, test_execution MUST remain "not_run"
    assert dims["test_execution"] == "not_run"
    assert dims["integration_verification"] == "not_run"
    assert dims["runtime_confirmation"] == "not_run"


@pytest.mark.asyncio
async def test_contract_7_input_budget_enforcement_and_health_contract(
    tmp_path: Path,
) -> None:
    """Verify input byte budget pre-dispatch rejection and enriched `codex_bridge_health`."""
    raw_craft = await claude_code_craft(
        task_spec="A" * 2000,
        workspace_dir=str(tmp_path),
        max_input_bytes=500,
    )
    craft_res = json.loads(raw_craft)
    assert craft_res["ok"] is False
    assert craft_res["error_code"] == "INPUT_BUDGET_EXCEEDED"
    assert craft_res["budget"]["input_budget_exceeded"] is True
    assert craft_res["budget"]["termination_reason"] == "input_budget_exceeded"

    run_res = await run_codex_exec(
        prompt="B" * 2000,
        workspace_dir=str(tmp_path),
        max_input_bytes=500,
    )
    assert run_res["ok"] is False
    assert run_res["error_code"] == "INPUT_BUDGET_EXCEEDED"


@pytest.mark.asyncio
async def test_alexandria_decision_pack_and_health(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    async def _no_agy_endpoint() -> AgyEndpoint | None:
        return None

    def _no_codex_binary() -> str:
        raise FileNotFoundError("codex binary mocked in unit test")

    monkeypatch.setattr(server_mod, "discover_agy_endpoint", _no_agy_endpoint)
    monkeypatch.setattr(server_mod, "resolve_codex_binary", _no_codex_binary)
    monkeypatch.setattr(server_mod, "handoff_store", HandoffStore(state_dir=tmp_path / "handoffs"))
    pack_json = await codex_alexandria_decision_pack(
        project_scope="heterarchy-core",
        topic_title="Lock-free ring buffer audit",
        argon_architecture_intent="Single-producer single-consumer queue",
        codex_audit_or_synthesis_summary="Identified memory ordering issue on weak architectures",
        final_resolution="Replaced relaxed atomic store with release/acquire semantics",
        sonnet_craft_or_review_summary="Refactored ring index arithmetic for clarity",
        verification_evidence="pytest tests/test_ring.py passed (10,000 iterations)",
    )
    pack = json.loads(pack_json)
    assert pack["ok"] is True
    assert pack["idempotency_key"].startswith("heterarchy-argon-codex-")
    assert "# [Heterarchy Decision] Lock-free ring buffer audit" in pack["markdown_content"]
    assert pack["consensus_reached"] is False
    assert pack["evidence_status"] == "unverified"
    assert pack["review_approved"] is False
    assert "Refactored ring index arithmetic for clarity" in pack["markdown_content"]

    health_json = await codex_bridge_health()
    health = json.loads(health_json)
    assert "verified_at_utc" in health
    assert "codex_binary" in health
    assert "codex_version" in health
    assert "routing_contract" in health
    assert health["routing_contract"]["silent_fallback_enabled"] is False
    assert "auth_boundaries" in health
    assert health["auth_boundaries"]["antigravity_rpc"]["independent_of_codex_cli_oauth"] is True
    assert health["auth_boundaries"]["codex_cli_oauth"]["independent_of_antigravity_rpc"] is True
    assert "pending_argon_handoffs" in health
    assert "pending_sonnet_handoffs" in health


@pytest.mark.asyncio
async def test_fastmcp_registered_tools() -> None:
    tools = await mcp.list_tools()
    tool_names = {t.name for t in tools}
    expected = {
        "codex_adversarial_audit",
        "codex_targeted_synthesize",
        "codex_differential_implementations",
        "codex_rescue_sandbox",
        "codex_git_review",
        "codex_session_followup",
        "codex_job_status",
        "codex_job_list",
        "heterarchy_handoff_create",
        "heterarchy_handoff_inbox",
        "heterarchy_handoff_resolve",
        "claude_code_craft",
        "heterarchy_triad_orchestrate",
        "codex_alexandria_decision_pack",
        "codex_bridge_health",
    }
    assert expected.issubset(tool_names)


@pytest.mark.asyncio
async def test_claude_opus_5_5_medium_routing_prompt_and_receipt(tmp_path: Path) -> None:
    """Verify Claude Opus 5.5 (`claude-opus-5-5-medium` / `opus-5.5`) routing and receipt."""
    for alias in ("opus", "opus-5.5", "opus-5-5", "opus5.5", "claude-opus-5-5-medium"):
        route = validate_and_resolve_route(
            engine="antigravity_local_rpc",
            model=alias,
            provider="antigravity",
        )
        assert route.valid is True
        assert route.resolved_model == "claude-opus-5-5-medium"
        assert route.placeholder == "MODEL_PLACEHOLDER_M401"
        assert route.reasoning_tier == "medium"

    # Non-Medium Opus tiers must be rejected pre-dispatch
    for bad_opus in ("opus-high", "claude-opus-5-5-high", "opus-low", "claude-opus-5-5-low"):
        bad_route = validate_and_resolve_route(
            engine="antigravity_local_rpc",
            model=bad_opus,
            provider="antigravity",
        )
        assert bad_route.valid is False
        assert bad_route.error_code == "CLAUDE_TIER_POLICY_VIOLATION"

    # Prompt builder tailors header for Opus 5.5 vs Sonnet 5.5
    opus_prompt = build_claude_craft_prompt(
        task_spec="Design lock-free ring buffer",
        claude_model="claude-opus-5-5-medium",
    )
    assert "CLAUDE OPUS 5.5 DEEP ARCHITECTURAL & SURGICAL CODE CRAFTSMAN" in opus_prompt
    assert "claude-opus-5-5-medium" in opus_prompt

    # Mock RPC execution for Opus 5.5
    store = JobStore(state_dir=tmp_path / "jobs")
    fake_ep = AgyEndpoint(
        base_url="https://127.0.0.1:59999",
        csrf_token="tok-opus",  # noqa: S106
        pid=33,
    )

    def opus_rpc(
        base_url: str,
        csrf_token: str,
        method: str,
        payload: JsonObject,
        timeout_seconds: float,
    ) -> JsonObject:
        if method == "GetCascadeModelConfigData":
            return {
                "clientModelConfigs": [
                    {
                        "label": "Claude Opus 5.5 (Medium)",
                        "modelId": "claude-opus-5-5-medium",
                        "modelOrAlias": {"model": "MODEL_PLACEHOLDER_M401"},
                        "quotaInfo": {"remainingFraction": 0.95},
                    }
                ]
            }
        assert payload.get("model") == "MODEL_PLACEHOLDER_M401"
        return {
            "modelId": "claude-opus-5-5-medium",
            "response": json.dumps(
                {
                    "verdict": "PASS",
                    "summary": "Opus 5.5 architectural synthesis complete",
                    "crafted_code": "def ring(): return True",
                    "design_highlights": ["Zero-allocation hot path"],
                    "edge_cases_guarded": ["Wrap-around overflow"],
                    "handoff_notes_for_codex_audit": "Verify memory barriers",
                }
            ),
        }

    res = await run_agy_model_prompt(
        prompt=opus_prompt,
        model="opus-5.5",
        job_store=store,
        endpoint=fake_ep,
        rpc_transport=opus_rpc,
    )
    assert res["ok"] is True
    assert res["model_id"] == "claude-opus-5-5-medium"
    assert res["placeholder"] == "MODEL_PLACEHOLDER_M401"
    receipt = res["receipt"]
    assert isinstance(receipt, dict)
    assert receipt["resolved_model"] == "claude-opus-5-5-medium"
    assert receipt["actual_model"] == "claude-opus-5-5-medium"
    assert receipt["model_mismatch"] is False


def test_python_314t_free_threading_runtime_and_lazy_store_init(tmp_path: Path) -> None:
    """Verify Python 3.14.7t free-threading (No-GIL) runtime and lazy directory creation."""
    rt = get_python_runtime_info()
    assert rt["free_threading_build"] is True
    assert rt["gil_enabled"] is False
    assert "t" in rt["abiflags"]
    assert sys._is_gil_enabled() is False

    # Lazy initialization: constructing stores must NOT create directories on disk
    lazy_jobs_dir = tmp_path / "lazy_jobs"
    lazy_handoffs_dir = tmp_path / "lazy_handoffs"
    j_store = JobStore(state_dir=lazy_jobs_dir)
    h_store = HandoffStore(state_dir=lazy_handoffs_dir)
    assert not lazy_jobs_dir.exists()
    assert not lazy_handoffs_dir.exists()

    # Directories are created lazily on first operation
    rec = j_store.create_job(task_type="audit", workspace_dir=str(tmp_path))
    assert lazy_jobs_dir.is_dir()
    assert j_store.get_job(rec.job_id) is not None

    tkt = h_store.create_ticket(
        sender="argon",
        recipient="opus",
        title="Opus deep refactor",
        spec_or_payload="Refactor module boundaries",
        workspace_dir=str(tmp_path),
    )
    assert lazy_handoffs_dir.is_dir()
    assert h_store.get_ticket(tkt.ticket_id) is not None


def test_free_threaded_concurrent_store_stress(tmp_path: Path) -> None:
    """Stress-test JobStore and HandoffStore across 16 parallel OS threads with GIL disabled."""
    assert sys._is_gil_enabled() is False
    j_store = JobStore(state_dir=tmp_path / "concurrent_jobs")
    h_store = HandoffStore(state_dir=tmp_path / "concurrent_handoffs")

    def worker(worker_idx: int) -> tuple[str, str]:
        job = j_store.create_job(
            task_type="parallel_stress",
            workspace_dir=str(tmp_path),
            model="claude-opus-5-5-medium",
            idempotency_key=f"idem-worker-{worker_idx}",
        )
        j_store.complete_job(
            job.job_id,
            result={"worker": worker_idx, "verdict": "PASS"},
        )
        ticket = h_store.create_ticket(
            sender="argon",
            recipient="opus" if worker_idx % 2 == 0 else "sonnet",
            title=f"Task {worker_idx}",
            spec_or_payload=f"Payload {worker_idx}",
            workspace_dir=str(tmp_path),
        )
        h_store.update_ticket_status(
            ticket.ticket_id,
            status="completed",
            resolution_note=f"Resolved by worker {worker_idx}",
        )
        return (job.job_id, ticket.ticket_id)

    with ThreadPoolExecutor(max_workers=16) as pool:
        futures = [pool.submit(worker, idx) for idx in range(48)]
        pairs = [f.result() for f in futures]

    assert len(pairs) == 48
    for idx, (job_id, ticket_id) in enumerate(pairs):
        loaded_job = j_store.get_job(job_id)
        assert loaded_job is not None
        assert loaded_job.status == "completed"
        assert j_store.find_by_idempotency_key(f"idem-worker-{idx}") is not None

        loaded_ticket = h_store.get_ticket(ticket_id)
        assert loaded_ticket is not None
        assert loaded_ticket.status == "completed"
        assert loaded_ticket.to_dict()["execution_completed"] is True


def test_pydantic_v2_closed_boundary_validation() -> None:
    """Verify Pydantic v2 closed-boundary models enforce extra='forbid' and required fields."""
    ok, validated, err = validate_structured_output(
        "claude_craft",
        {
            "verdict": "PASS",
            "summary": "Clean refactor",
            "crafted_code": "def f() -> int: return 1",
            "design_highlights": ["Typed"],
            "edge_cases_guarded": ["None"],
            "handoff_notes_for_codex_audit": "Verify return type",
        },
    )
    assert ok is True
    assert err is None
    assert validated is not None
    assert validated["verdict"] == "PASS"

    # Extra unknown field must be rejected (`extra="forbid"`)
    ok_extra, validated_extra, err_extra = validate_structured_output(
        "claude_craft",
        {
            "verdict": "PASS",
            "summary": "Clean refactor",
            "crafted_code": "def f() -> int: return 1",
            "design_highlights": ["Typed"],
            "edge_cases_guarded": ["None"],
            "handoff_notes_for_codex_audit": "Verify return type",
            "undeclared_field": "forbidden",
        },
    )
    assert ok_extra is False
    assert validated_extra is None
    assert err_extra is not None


@pytest.mark.asyncio
async def test_triad_checkpoint_resume_reuses_succeeded_roles(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Verify `resume_collaboration_id` reuses succeeded Sonnet/Codex roles without re-dispatch."""
    isolated_jobs = JobStore(state_dir=tmp_path / "jobs")
    isolated_handoffs = HandoffStore(state_dir=tmp_path / "handoffs")
    monkeypatch.setattr(server_mod, "job_store", isolated_jobs)
    monkeypatch.setattr(server_mod, "handoff_store", isolated_handoffs)

    sonnet_calls = 0
    codex_calls = 0
    argon_calls = 0

    async def mock_agy_prompt(
        prompt: str,
        model: str = "claude-sonnet-5-5-medium",
        expect_json: bool = True,
        timeout_seconds: float = 180.0,
        **kwargs: object,
    ) -> JsonObject:
        nonlocal sonnet_calls, argon_calls
        if "argon" in model.lower():
            argon_calls += 1
            if argon_calls == 1:
                return {
                    "ok": False,
                    "status": "timed_out_unknown",
                    "outcome_state": "outcome_unknown",
                    "request_id": "cdx-argontimeout01",
                    "model_id": model,
                    "error": "Argon RPC read timed out on first attempt",
                }
            return {
                "ok": True,
                "status": "succeeded",
                "outcome_state": "confirmed_success",
                "request_id": "cdx-argonok02",
                "model_id": model,
                "structured_output": {
                    "verdict": "PASS",
                    "summary": "Argon approved resumed pipeline after checkpoint.",
                },
            }
        sonnet_calls += 1
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-sonnetok01",
            "model_id": model,
            "structured_output": {
                "verdict": "PASS",
                "summary": "Sonnet crafted checkpointable pipeline stage.",
                "crafted_code": "def checkpoint_fn() -> int: return 42",
                "handoff_notes_for_codex_audit": "Verify return constant",
            },
        }

    async def mock_codex_engine(
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
        **kwargs: object,
    ) -> JsonObject:
        nonlocal codex_calls
        codex_calls += 1
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-codexok01",
            "model": "gpt-6.1-sol",
            "structured_output": {
                "verdict": "PASS",
                "executive_critique": "Codex verified checkpoint_fn with zero findings.",
                "failing_test_cases": ["def test_checkpoint_fn(): assert checkpoint_fn() == 42"],
            },
        }

    monkeypatch.setattr(server_mod, "run_agy_model_prompt", mock_agy_prompt)
    monkeypatch.setattr(server_mod, "_execute_with_engine", mock_codex_engine)

    first_raw = await heterarchy_triad_orchestrate(
        task_spec="Implement checkpointable pipeline stage",
        workspace_dir=str(tmp_path),
        mode="pipeline",
    )
    first = json.loads(first_raw)
    assert first["collaboration_status"] == "partial"
    assert first["consensus_status"] == "incomplete_no_consensus"
    assert sonnet_calls == 1
    assert codex_calls == 1
    assert argon_calls == 1
    collab_id = str(first["collaboration_id"])

    second_raw = await heterarchy_triad_orchestrate(
        task_spec="Implement checkpointable pipeline stage",
        workspace_dir=str(tmp_path),
        mode="pipeline",
        resume_collaboration_id=collab_id,
    )
    second = json.loads(second_raw)
    assert second["ok"] is True
    assert second["collaboration_status"] == "complete"
    assert second["consensus_status"] == "consensus_verified"
    assert second["consensus_reached"] is True
    assert second["resumed_roles"] == ["codex", "sonnet"]
    assert second["resumed_from_collaboration_id"] == collab_id
    assert sonnet_calls == 1
    assert codex_calls == 1
    assert argon_calls == 2


@pytest.mark.asyncio
async def test_triad_multi_turn_auto_repair_loop(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Verify `max_rounds=2` runs an automatic Claude repair + Codex re-audit loop on BLOCK."""
    isolated_jobs = JobStore(state_dir=tmp_path / "jobs")
    isolated_handoffs = HandoffStore(state_dir=tmp_path / "handoffs")
    monkeypatch.setattr(server_mod, "job_store", isolated_jobs)
    monkeypatch.setattr(server_mod, "handoff_store", isolated_handoffs)

    sonnet_calls = 0
    codex_calls = 0

    async def mock_agy_prompt(
        prompt: str,
        model: str = "claude-sonnet-5-5-medium",
        expect_json: bool = True,
        timeout_seconds: float = 180.0,
        **kwargs: object,
    ) -> JsonObject:
        nonlocal sonnet_calls
        if "argon" in model.lower():
            return {
                "ok": True,
                "status": "succeeded",
                "outcome_state": "confirmed_success",
                "request_id": "cdx-argonrepair",
                "model_id": model,
                "structured_output": {
                    "verdict": "PASS",
                    "summary": "Argon verified repaired implementation.",
                },
            }
        sonnet_calls += 1
        if sonnet_calls == 1:
            return {
                "ok": True,
                "status": "succeeded",
                "outcome_state": "confirmed_success",
                "request_id": "cdx-sonnet-r1",
                "model_id": model,
                "structured_output": {
                    "verdict": "PASS",
                    "summary": "Initial division helper",
                    "crafted_code": "def safe_div(a: float, b: float) -> float: return a / b",
                    "handoff_notes_for_codex_audit": "Check division behavior",
                },
            }
        assert "Auto-Repair Round 2" in prompt
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-sonnet-r2",
            "model_id": model,
            "structured_output": {
                "verdict": "PASS",
                "summary": "Repaired division helper with zero-denominator guard",
                "crafted_code": (
                    "def safe_div(a: float, b: float) -> float: return 0.0 if b == 0 else a / b"
                ),
                "handoff_notes_for_codex_audit": "Added b == 0 guard",
            },
        }

    async def mock_codex_engine(
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
        **kwargs: object,
    ) -> JsonObject:
        nonlocal codex_calls
        codex_calls += 1
        if codex_calls == 1:
            return {
                "ok": True,
                "status": "succeeded",
                "outcome_state": "confirmed_success",
                "request_id": "cdx-codex-r1",
                "model": "gpt-6.1-sol",
                "structured_output": {
                    "verdict": "BLOCK",
                    "executive_critique": "ZeroDivisionError when b == 0 is unguarded.",
                    "failing_test_cases": ["def test_zero(): safe_div(1.0, 0.0)"],
                },
            }
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-codex-r2",
            "model": "gpt-6.1-sol",
            "structured_output": {
                "verdict": "PASS",
                "executive_critique": "Zero denominator is now guarded properly.",
                "failing_test_cases": ["def test_zero(): assert safe_div(1.0, 0.0) == 0.0"],
            },
        }

    monkeypatch.setattr(server_mod, "run_agy_model_prompt", mock_agy_prompt)
    monkeypatch.setattr(server_mod, "_execute_with_engine", mock_codex_engine)

    res_raw = await heterarchy_triad_orchestrate(
        task_spec="Implement safe_div helper",
        workspace_dir=str(tmp_path),
        mode="pipeline",
        max_rounds=2,
    )
    res = json.loads(res_raw)
    assert res["ok"] is True
    assert res["repair_rounds_executed"] == 1
    assert res["consensus_status"] == "consensus_verified"
    assert res["consensus_reached"] is True
    assert sonnet_calls == 2
    assert codex_calls == 2


@pytest.mark.asyncio
async def test_bridge_executed_verification_gates_promote_test_execution(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Verify `run_verification_gates=True` runs pytest in workspace and promotes test_execution."""
    isolated_jobs = JobStore(state_dir=tmp_path / "jobs")
    isolated_handoffs = HandoffStore(state_dir=tmp_path / "handoffs")
    monkeypatch.setattr(server_mod, "job_store", isolated_jobs)
    monkeypatch.setattr(server_mod, "handoff_store", isolated_handoffs)

    sample_test = tmp_path / "test_sample.py"
    sample_test.write_text(
        "def test_sample_gate_passes() -> None:\n    assert 3 * 7 == 21\n",
        encoding="utf-8",
    )

    async def mock_agy_prompt(
        prompt: str,
        model: str = "claude-sonnet-5-5-medium",
        expect_json: bool = True,
        timeout_seconds: float = 180.0,
        **kwargs: object,
    ) -> JsonObject:
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-gate-agy",
            "model_id": model,
            "structured_output": {
                "verdict": "PASS",
                "summary": "Verified gate implementation.",
                "crafted_code": "def mul() -> int: return 21",
            },
        }

    async def mock_codex_engine(
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
        **kwargs: object,
    ) -> JsonObject:
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-gate-codex",
            "model": "gpt-6.1-sol",
            "structured_output": {
                "verdict": "PASS",
                "executive_critique": "Clean implementation.",
                "failing_test_cases": ["def test_mul(): assert mul() == 21"],
            },
        }

    monkeypatch.setattr(server_mod, "run_agy_model_prompt", mock_agy_prompt)
    monkeypatch.setattr(server_mod, "_execute_with_engine", mock_codex_engine)

    res_raw = await heterarchy_triad_orchestrate(
        task_spec="Verify bridge-executed test gate promotion",
        workspace_dir=str(tmp_path),
        run_verification_gates=True,
    )
    res = json.loads(res_raw)
    assert res["ok"] is True
    ev_scope = res["evidence_scope"]
    assert ev_scope["evidence_dimensions"]["test_execution"] == "verified"
    assert ev_scope["test_execution_evidence_origin"] == "bridge_executed"
    assert len(ev_scope["executed_test_commands"]) == 1
    assert ev_scope["executed_test_commands"][0]["passed"] is True
    assert str(ev_scope["executed_test_commands"][0]["stdout_digest"]).startswith("sha256:")


def _seed_test_codegraph_db(ws: Path) -> None:
    """Create a self-contained `.codegraph/codegraph.db` fixture in `ws` for CI and local tests."""
    src_file = ws / "src" / "codex_collab_mcp" / "domain" / "routing_policy.py"
    src_file.parent.mkdir(parents=True, exist_ok=True)
    src_file.write_text(
        "class RoutingPolicyEngine:\n    def validate(self) -> bool:\n        return True\n",
        encoding="utf-8",
    )
    cg_dir = ws / ".codegraph"
    cg_dir.mkdir(parents=True, exist_ok=True)
    db_path = cg_dir / "codegraph.db"
    with contextlib.closing(sqlite3.connect(db_path)) as conn, conn:
        conn.executescript(
            """
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL
            );
            CREATE TABLE edges (
                source TEXT NOT NULL,
                target TEXT NOT NULL
            );
            INSERT INTO nodes VALUES (
                'n1', 'class', 'RoutingPolicyEngine',
                'codex_collab_mcp.domain.routing_policy.RoutingPolicyEngine',
                'src/codex_collab_mcp/domain/routing_policy.py', 1, 3
            );
            INSERT INTO nodes VALUES (
                'n2', 'function', 'validate_and_resolve_route',
                'codex_collab_mcp.domain.routing_policy.validate_and_resolve_route',
                'src/codex_collab_mcp/domain/routing_policy.py', 1, 3
            );
            INSERT INTO edges VALUES ('n2', 'n1');
            """
        )


def test_codegraph_context_slicer_and_persisted_idempotency_index(
    tmp_path: Path,
) -> None:
    """Verify CodeGraph symbol slicing and SQLite WAL `jobs.db` indexed idempotency lookup."""
    _seed_test_codegraph_db(tmp_path)
    sliced_md = slice_codegraph_symbols(str(tmp_path), ["RoutingPolicyEngine"])
    assert "### CodeGraph Symbol Slices" in sliced_md
    assert "RoutingPolicyEngine" in sliced_md
    assert "src/codex_collab_mcp/domain/routing_policy.py" in sliced_md
    assert "callers=" in sliced_md
    assert "callees=" in sliced_md

    jobs_dir = tmp_path / "persisted_index_jobs"
    store1 = JobStore(state_dir=jobs_dir)
    created = store1.create_job(
        task_type="adversarial_audit",
        workspace_dir=str(tmp_path),
        idempotency_key="persisted-idem-key-001",
    )
    store1.complete_job(created.job_id, result={"verdict": "PASS"})
    assert (jobs_dir / "jobs.db").is_file()

    fresh_store = JobStore(state_dir=jobs_dir)
    resolved = fresh_store.find_by_idempotency_key("persisted-idem-key-001")
    assert resolved is not None
    assert resolved.job_id == created.job_id
    assert resolved.status == "completed"


@pytest.mark.asyncio
async def test_http_fallback_job_store_registration_and_gateway_codegraph_slicing(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Verify HTTP fallback registers in JobStore and gateway tools slice CodeGraph symbols."""

    class _FakeResponse:
        def raise_for_status(self) -> None:
            return None

        def json(self) -> dict[str, object]:
            return {
                "model": "gpt-6.1-sol",
                "choices": [{"message": {"content": '{"verdict": "PASS"}'}}],
            }

    class _FakeAsyncClient:
        def __init__(self, *, timeout: float) -> None:
            self.timeout = timeout

        async def __aenter__(self) -> _FakeAsyncClient:
            return self

        async def __aexit__(self, *args: object) -> None:
            return None

        async def post(self, url: str, **kwargs: object) -> _FakeResponse:
            return _FakeResponse()

    monkeypatch.setattr(httpx, "AsyncClient", _FakeAsyncClient)
    store = JobStore(state_dir=tmp_path / "http_jobs")

    first = await run_http_fallback(
        prompt="Verify HTTP job registration",
        schema_name="adversarial_audit",
        output_schema=None,
        model="gpt-6.1-sol",
        request_id="cdx-http000001",
        idempotency_key="http-idem-01",
        workspace_dir=str(tmp_path),
        job_store=store,
    )
    assert first["ok"] is True
    persisted = store.get_persisted_job("cdx-http000001")
    assert persisted is not None
    assert persisted.status == "completed"
    assert persisted.outcome_state == "confirmed_success"

    dedup = await run_http_fallback(
        prompt="Verify HTTP job registration",
        schema_name="adversarial_audit",
        output_schema=None,
        model="gpt-6.1-sol",
        idempotency_key="http-idem-01",
        workspace_dir=str(tmp_path),
        job_store=store,
    )
    assert dedup.get("deduplicated_existing_request") is True
    assert dedup["request_id"] == "cdx-http000001"

    captured_prompts: list[str] = []

    async def fake_agy(prompt: str, **kwargs: object) -> JsonObject:
        captured_prompts.append(prompt)
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "structured_output": {
                "verdict": "PASS",
                "summary": "Sliced symbol verified.",
                "crafted_code": "x = 1",
                "design_highlights": ["Clean slice"],
                "edge_cases_guarded": ["None"],
                "handoff_notes_for_codex_audit": "ok",
            },
        }

    monkeypatch.setattr(server_mod, "run_agy_model_prompt", fake_agy)
    _seed_test_codegraph_db(tmp_path)
    craft_raw = await claude_code_craft(
        task_spec="Check symbol slicing",
        target_code_or_context="def f(): pass",
        workspace_dir=str(tmp_path),
        symbols_inspected=["RoutingPolicyEngine"],
    )
    craft_res = json.loads(craft_raw)
    assert craft_res["ok"] is True
    assert craft_res["schema_valid"] is True
    assert len(captured_prompts) == 1
    assert "[CodeGraph Sliced Symbol Context]:" in captured_prompts[0]
    assert "RoutingPolicyEngine" in captured_prompts[0]


@pytest.mark.asyncio
async def test_triad_role_snapshot_model_ids_and_job_telemetry(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    """Verify triad role snapshots persist `model_id` and `triad_orchestration` job telemetry."""
    isolated_jobs = JobStore(state_dir=tmp_path / "jobs")
    isolated_handoffs = HandoffStore(state_dir=tmp_path / "handoffs")
    monkeypatch.setattr(server_mod, "job_store", isolated_jobs)
    monkeypatch.setattr(server_mod, "handoff_store", isolated_handoffs)

    async def mock_agy_prompt(
        prompt: str,
        model: str = "claude-opus-5-5-medium",
        expect_json: bool = True,
        timeout_seconds: float = 180.0,
        **kwargs: object,
    ) -> JsonObject:
        if "argon" in model.lower():
            return {
                "ok": True,
                "status": "succeeded",
                "outcome_state": "confirmed_success",
                "request_id": "cdx-argon-tel01",
                "model_id": "gemini-4-argon",
                "structured_output": {
                    "verdict": "PASS",
                    "summary": "Argon macro review verified.",
                },
            }
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-opus-tel01",
            "model_id": "claude-opus-5-5-medium",
            "structured_output": {
                "verdict": "PASS",
                "summary": "Opus 5.5 review verified.",
                "crafted_code": "def f() -> int: return 1",
                "handoff_notes_for_codex_audit": "Verify return value",
            },
        }

    async def mock_codex_engine(
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
        **kwargs: object,
    ) -> JsonObject:
        return {
            "ok": True,
            "status": "succeeded",
            "outcome_state": "confirmed_success",
            "request_id": "cdx-codex-tel01",
            "model": "gpt-6.1-sol",
            "structured_output": {
                "verdict": "PASS",
                "executive_critique": "Codex red-team verified.",
                "failing_test_cases": ["def test_f(): assert f() == 1"],
            },
        }

    monkeypatch.setattr(server_mod, "run_agy_model_prompt", mock_agy_prompt)
    monkeypatch.setattr(server_mod, "_execute_with_engine", mock_codex_engine)

    res_raw = await heterarchy_triad_orchestrate(
        task_spec="Verify role snapshot model_id and job telemetry",
        workspace_dir=str(tmp_path),
        mode="council",
        claude_model="claude-opus-5-5-medium",
        codex_model="gpt-6.1-sol",
        argon_model="gemini-4-argon",
    )
    res = json.loads(res_raw)
    assert res["ok"] is True
    collab_id = str(res["collaboration_id"])
    persisted_job = isolated_jobs.get_persisted_job(collab_id)
    assert persisted_job is not None
    assert persisted_job.provider == "heterarchy_triad"
    assert persisted_job.engine == "council"
    assert persisted_job.model == "gemini-4-argon+claude-opus-5-5-medium+gpt-6.1-sol"
    assert persisted_job.completed_at is not None
    assert persisted_job.completed_at >= persisted_job.created_at
    assert isinstance(persisted_job.result, dict)
    roles = persisted_job.result["roles"]
    assert isinstance(roles, dict)
    argon_snap = roles["argon"]
    sonnet_snap = roles["sonnet"]
    codex_snap = roles["codex"]
    assert isinstance(argon_snap, dict)
    assert isinstance(sonnet_snap, dict)
    assert isinstance(codex_snap, dict)
    assert argon_snap["model_id"] == "gemini-4-argon"
    assert sonnet_snap["model_id"] == "claude-opus-5-5-medium"
    assert codex_snap["model_id"] == "gpt-6.1-sol"

    for bad_val in ("--help", "-c", " --dangerously-bypass-approvals-and-sandbox "):
        with pytest.raises(ValueError, match="must not start with '-'"):
            build_codex_exec_command(
                codex_bin="/opt/homebrew/bin/codex",
                workspace_dir=str(tmp_path),
                output_file=str(tmp_path / "out.txt"),
                model=bad_val,
            )
        with pytest.raises(ValueError, match="must not start with '-'"):
            build_codex_exec_command(
                codex_bin="/opt/homebrew/bin/codex",
                workspace_dir=str(tmp_path),
                output_file=str(tmp_path / "out.txt"),
                reasoning_effort=bad_val,
            )
        with pytest.raises(ValueError, match="must not start with '-'"):
            build_codex_review_command(
                codex_bin="/opt/homebrew/bin/codex",
                reasoning_effort=bad_val,
            )
        with pytest.raises(ValueError, match="not starting with '-'"):
            await CodexAuxiliaryRunner.run_session_followup(
                session_id=bad_val,
                prompt="continue",
                workspace_dir=str(tmp_path),
                resolve_binary_fn=lambda: "/opt/homebrew/bin/codex",
            )
