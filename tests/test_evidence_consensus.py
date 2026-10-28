"""Regression contracts for review execution versus agreement and decision records."""

from __future__ import annotations

import dataclasses
import hashlib
import json
from pathlib import Path

import pytest

import codex_collab_mcp.server as server
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import CodexJobRecord, JobStore
from codex_collab_mcp.schemas import JsonObject


@pytest.fixture(autouse=True)
def isolate_job_store(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setattr(server, "job_store", JobStore(state_dir=tmp_path / "jobs"))
    monkeypatch.setattr(server, "handoff_store", HandoffStore(state_dir=tmp_path / "handoffs"))


def successful_review(verdict: str = "PASS") -> JsonObject:
    return {
        "ok": True,
        "status": "succeeded",
        "outcome_state": "confirmed_success",
        "structured_output": {"verdict": verdict, "summary": "Reviewed supplied source."},
    }


async def run_council(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    roles: dict[str, JsonObject] | None = None,
    invoke_argon_rpc: bool = True,
) -> dict[str, object]:
    effective_roles = roles or {role: successful_review() for role in ("argon", "sonnet", "codex")}

    async def agy(**kwargs: object) -> JsonObject:
        return effective_roles["argon" if "argon" in str(kwargs.get("model")) else "sonnet"]

    async def codex(**kwargs: object) -> JsonObject:
        return effective_roles["codex"]

    monkeypatch.setattr(server, "run_agy_model_prompt", agy)
    monkeypatch.setattr(server, "_execute_with_engine", codex)
    return json.loads(
        await server.heterarchy_triad_orchestrate(
            task_spec="Bounded review",
            target_code_or_context="def identity(x): return x",
            mode="council",
            workspace_dir=str(tmp_path),
            invoke_argon_rpc=invoke_argon_rpc,
        )
    )


async def pack_for(
    collaboration_id: str | None = None,
    resolution: str = "Decision",
    *,
    workspace_dir: str | None = None,
    expected_scope_digest: str | None = None,
) -> dict[str, object]:
    return json.loads(
        await server.codex_alexandria_decision_pack(
            project_scope="test",
            topic_title="Review outcomes",
            argon_architecture_intent="Argon",
            codex_audit_or_synthesis_summary="Codex",
            sonnet_craft_or_review_summary="Claude",
            final_resolution=resolution,
            collaboration_id=collaboration_id,
            workspace_dir=workspace_dir,
            expected_scope_digest=expected_scope_digest,
        )
    )


async def pack_from_run(result: dict[str, object], workspace: Path) -> dict[str, object]:
    scope = result["evidence_scope"]
    assert isinstance(scope, dict)
    return await pack_for(
        str(result["collaboration_id"]),
        workspace_dir=str(workspace),
        expected_scope_digest=str(scope["scope_digest"]),
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("mode", ["council", "pipeline"])
@pytest.mark.parametrize("verdict", ["PASS", "WARN", "BLOCK"])
async def test_completed_reviews_are_recorded_independently_of_agreement(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, mode: str, verdict: str
) -> None:
    async def response(**kwargs: object) -> JsonObject:
        model = str(kwargs.get("model", ""))
        return {
            "ok": True,
            "status": "succeeded",
            "structured_output": {
                "verdict": verdict if "argon" in model else "PASS",
                "summary": "Inspected the supplied function and its boundary conditions.",
                "crafted_code": "def increment(value: int) -> int: return value + 1",
            },
        }

    monkeypatch.setattr(server, "run_agy_model_prompt", response)
    monkeypatch.setattr(server, "_execute_with_engine", response)
    result = json.loads(
        await server.heterarchy_triad_orchestrate(
            task_spec="Review increment",
            target_code_or_context="def increment(x): return x + 1",
            mode=mode,
            workspace_dir=str(tmp_path),
        )
    )
    assert result["evidence_scope"]["evidence_dimensions"]["static_code_review"] == "verified"
    assert result["consensus_reached"] is (verdict == "PASS")
    for dimension in ("test_execution", "integration_verification", "runtime_confirmation"):
        assert result["evidence_scope"]["evidence_dimensions"][dimension] == "not_run"


@pytest.mark.asyncio
async def test_legacy_decision_pack_cannot_claim_consensus_from_prose() -> None:
    result = json.loads(
        await server.codex_alexandria_decision_pack(
            project_scope="test",
            topic_title="Review outcomes",
            argon_architecture_intent="PASS",
            codex_audit_or_synthesis_summary="PASS",
            sonnet_craft_or_review_summary="PASS",
            final_resolution="Consensus verified!",
        )
    )
    markdown = result["markdown_content"]
    assert markdown.startswith("# [Heterarchy Decision]")
    assert "## 4. Converged" not in markdown
    assert "persist this consensus" not in result["recommended_next_step"]
    assert result["consensus_reached"] is False


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("affected_role", "bad_role"),
    [
        ("argon", {"ok": False, "status": "timed_out_unknown"}),
        ("sonnet", {"ok": False, "status": "timed_out_unknown"}),
        ("codex", {"ok": False, "status": "timed_out_unknown"}),
        (
            "sonnet",
            {"ok": True, "status": "succeeded", "structured_output": {"verdict": "PASS"}},
        ),
        (
            "sonnet",
            {"ok": True, "status": "failed", "structured_output": {"verdict": "PASS"}},
        ),
        (
            "sonnet",
            {"ok": False, "status": "succeeded", "structured_output": {"verdict": "PASS"}},
        ),
        ("sonnet", {"ok": True, "status": "succeeded", "structured_output": {}}),
        (
            "sonnet",
            {
                "ok": True,
                "status": "succeeded",
                "structured_output": {"summary": "No explicit verdict"},
            },
        ),
        (
            "sonnet",
            {"ok": True, "status": "succeeded", "structured_output": {"verdict": "APPROVED"}},
        ),
        (
            "sonnet",
            {
                "ok": True,
                "status": "succeeded",
                "structured_output": {"summary": "No verdict"},
                "data": {"verdict": "PASS"},
            },
        ),
    ],
)
async def test_partial_review_does_not_promote_missing_or_invalid_evidence(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    affected_role: str,
    bad_role: JsonObject,
) -> None:
    roles = {role: successful_review() for role in ("argon", "sonnet", "codex")}
    roles[affected_role] = bad_role
    result = await run_council(
        monkeypatch,
        tmp_path,
        roles,
    )
    scope = result["evidence_scope"]
    assert isinstance(scope, dict)
    assert scope["evidence_dimensions"]["static_code_review"] == "partial"
    per_role = result["role_evidence_dimensions"]
    assert isinstance(per_role, dict)
    for role in roles:
        expected = "not_run" if role == affected_role else "verified"
        assert per_role[role]["static_code_review"] == expected
    assert result["consensus_reached"] is False
    pack = await pack_from_run(result, tmp_path)
    assert pack["consensus_reached"] is False
    assert str(pack["markdown_content"]).startswith("# [Heterarchy Decision]")


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("verdict", "consensus", "approved"),
    [
        ("PASS", True, True),
        ("WARN", True, False),
        ("BLOCK", False, False),
    ],
)
async def test_saved_same_run_consensus_survives_reader_restart(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    verdict: str,
    consensus: bool,
    approved: bool,
) -> None:
    result = await run_council(
        monkeypatch,
        tmp_path,
        {role: successful_review(verdict) for role in ("argon", "sonnet", "codex")},
    )
    assert result["consensus_reached"] is consensus
    assert result["review_approved"] is approved
    # A new owner instance proves disk readback, not just a process-local dictionary.
    monkeypatch.setattr(server, "job_store", JobStore(state_dir=tmp_path / "jobs"))
    pack = await pack_from_run(result, tmp_path)
    assert pack["consensus_reached"] is consensus
    assert pack["review_approved"] is approved
    assert pack["resolution_verified"] is False
    assert pack["consensus_status"] == result["consensus_status"]
    assert pack["collaboration_status"] == result["collaboration_status"]
    prefix = "# [Heterarchy Consensus]" if consensus else "# [Heterarchy Decision]"
    assert str(pack["markdown_content"]).startswith(prefix)
    assert str(result["collaboration_id"]) in str(pack["markdown_content"])


@pytest.mark.asyncio
async def test_failed_execution_does_not_create_review_evidence(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    result = await run_council(
        monkeypatch,
        tmp_path,
        {
            role: {"ok": False, "status": "timed_out_unknown"}
            for role in ("argon", "sonnet", "codex")
        },
    )
    scope = result["evidence_scope"]
    assert isinstance(scope, dict)
    assert scope["evidence_dimensions"]["static_code_review"] == "not_run"
    assert result["consensus_reached"] is False
    pack = await pack_from_run(result, tmp_path)
    assert pack["consensus_status"] == "incomplete_no_consensus"
    assert pack["consensus_reached"] is False


@pytest.mark.asyncio
@pytest.mark.parametrize("selector", [None, "", "../outside", "cdx-0000000000"])
async def test_absent_or_invalid_aggregate_is_neutral(selector: str | None) -> None:
    pack = await pack_for(selector)
    assert pack["consensus_reached"] is False
    assert str(pack["markdown_content"]).startswith("# [Heterarchy Decision]")


@pytest.mark.asyncio
async def test_non_triad_job_cannot_certify_consensus() -> None:
    job = server.job_store.create_job("adversarial_audit", ".")
    server.job_store.complete_job(
        job.job_id,
        {
            "collaboration_status": "complete",
            "consensus_status": "consensus_verified",
            "consensus_reached": True,
        },
    )
    pack = await pack_for(job.job_id)
    assert pack["consensus_reached"] is False
    assert str(pack["markdown_content"]).startswith("# [Heterarchy Decision]")


@pytest.mark.asyncio
async def test_decision_idempotency_uses_entire_resolution() -> None:
    prefix = "x" * 220
    first = await pack_for(resolution=prefix + " first")
    replay = await pack_for(resolution=prefix + " first")
    changed = await pack_for(resolution=prefix + " changed")
    assert first["idempotency_key"] == replay["idempotency_key"]
    assert first["markdown_content"] == replay["markdown_content"]
    assert first["idempotency_key"] != changed["idempotency_key"]


@pytest.mark.asyncio
@pytest.mark.parametrize("wrong_fence", ["workspace", "scope", "missing"])
async def test_argon_cross_scope_replay_is_not_approved(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    wrong_fence: str,
) -> None:
    result = await run_council(monkeypatch, tmp_path)
    scope = result["evidence_scope"]
    assert isinstance(scope, dict)
    pack = await pack_for(
        str(result["collaboration_id"]),
        workspace_dir=str(tmp_path / "another") if wrong_fence == "workspace" else str(tmp_path),
        expected_scope_digest=(
            None
            if wrong_fence == "missing"
            else "sha256:" + "0" * 64
            if wrong_fence == "scope"
            else str(scope["scope_digest"])
        ),
    )
    assert pack["consensus_reached"] is False
    assert pack["review_approved"] is False
    assert pack["resolution_verified"] is False
    assert str(pack["markdown_content"]).startswith("# [Heterarchy Decision]")


@pytest.mark.asyncio
async def test_inflight_aggregate_is_unverified(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    result = await run_council(monkeypatch, tmp_path)
    job = server.job_store.get_job(str(result["collaboration_id"]))
    assert job is not None
    running_job = dataclasses.replace(job, status="running")
    server.job_store._cache_put(running_job)
    server.job_store._persist(running_job)
    # The valid snapshot and matching fences remain; only job status differs.
    pack = await pack_from_run(result, tmp_path)
    assert pack["consensus_reached"] is False
    assert pack["review_approved"] is False


@pytest.mark.asyncio
async def test_not_started_role_remains_readable_in_saved_aggregate(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    result = await run_council(
        monkeypatch,
        tmp_path,
        {role: successful_review() for role in ("argon", "sonnet", "codex")},
        invoke_argon_rpc=False,
    )
    assert result["consensus_status"] == "incomplete_no_consensus"
    pack = await pack_from_run(result, tmp_path)
    assert pack["consensus_status"] == "incomplete_no_consensus"
    assert pack["collaboration_status"] == "partial"


@pytest.mark.asyncio
async def test_consensus_evidence_changes_decision_key(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    first = await run_council(
        monkeypatch, tmp_path, {role: successful_review() for role in ("argon", "sonnet", "codex")}
    )
    second = await run_council(
        monkeypatch,
        tmp_path,
        {
            "argon": successful_review("BLOCK"),
            "sonnet": successful_review(),
            "codex": successful_review(),
        },
    )
    first_pack = await pack_from_run(first, tmp_path)
    second_pack = await pack_from_run(second, tmp_path)
    assert first_pack["idempotency_key"] != second_pack["idempotency_key"]
    assert first_pack["consensus_reached"] is True
    assert second_pack["consensus_reached"] is False


@pytest.mark.asyncio
async def test_stored_aggregate_is_compact_and_corruption_is_not_approved(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    result = await run_council(monkeypatch, tmp_path)
    job = server.job_store.get_job(str(result["collaboration_id"]))
    assert job is not None and isinstance(job.result, dict)
    encoded = json.dumps(job.result)
    assert "Bounded review" not in encoded
    assert "Reviewed supplied source." not in encoded
    roles = job.result["roles"]
    assert isinstance(roles, dict) and set(roles) == {"argon", "sonnet", "codex"}
    for role in roles.values():
        assert isinstance(role, dict)
        assert str(role["review_artifact_digest"]).startswith("sha256:")
    job.result["source_task_digest"] = "sha256:" + "0" * 64
    pack = await pack_from_run(result, tmp_path)
    assert pack["review_approved"] is False
    assert pack["consensus_reached"] is False


@pytest.mark.asyncio
async def test_caller_test_report_is_not_bridge_execution(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    async def response(**kwargs: object) -> JsonObject:
        return successful_review()

    monkeypatch.setattr(server, "run_agy_model_prompt", response)
    monkeypatch.setattr(server, "_execute_with_engine", response)
    result = json.loads(
        await server.heterarchy_triad_orchestrate(
            task_spec="Review only",
            target_code_or_context="def f(): return 1",
            mode="council",
            workspace_dir=str(tmp_path),
            executed_test_commands=[{"command": "pytest imaginary.py", "passed": True}],
        )
    )
    assert result["evidence_scope"]["evidence_dimensions"]["test_execution"] == "not_run"
    assert result["evidence_scope"]["test_execution_evidence_origin"] == "caller_reported"
    pack = await pack_from_run(result, tmp_path)
    scope = pack["evidence_scope"]
    assert isinstance(scope, dict)
    assert scope["evidence_dimensions"]["test_execution"] == "not_run"
    assert scope["test_execution_evidence_origin"] == "caller_reported"


@pytest.mark.asyncio
@pytest.mark.parametrize("bad_workspace", ["", "missing-directory"])
async def test_invalid_workspace_rejected_before_dispatch(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    bad_workspace: str,
) -> None:
    calls = []

    async def response(**kwargs: object) -> JsonObject:
        calls.append(kwargs)
        return successful_review()

    monkeypatch.setattr(server, "run_agy_model_prompt", response)
    monkeypatch.setattr(server, "_execute_with_engine", response)
    workspace = "" if not bad_workspace else str(tmp_path / bad_workspace)
    with pytest.raises(ValueError):
        await server.heterarchy_triad_orchestrate(task_spec="Review", workspace_dir=workspace)
    assert calls == []


@pytest.mark.asyncio
async def test_persistence_error_keeps_completed_peer_results(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def disk_full(*args: object, **kwargs: object) -> None:
        raise OSError("simulated full disk")

    monkeypatch.setattr(server.job_store, "create_job", disk_full)
    result = await run_council(monkeypatch, tmp_path)
    assert not result.get("collaboration_id")
    assert result["collaboration_persistence"] == "failed"
    for role in ("argon_review", "sonnet_craft", "codex_audit"):
        role_result = result[role]
        assert isinstance(role_result, dict)
        assert role_result["status"] == "succeeded"


@pytest.mark.asyncio
async def test_readonly_pipeline_reviews_source_not_response_json(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    source = "def original_target() -> int: return 42"
    captured = []

    async def agy(**kwargs: object) -> JsonObject:
        artifact: JsonObject = {
            "verdict": "PASS",
            "summary": "Reviewed source.",
            "crafted_code": "",
        }
        return {
            "ok": True,
            "status": "succeeded",
            "structured_output": artifact,
            "raw_output": json.dumps(artifact),
        }

    async def codex(**kwargs: object) -> JsonObject:
        captured.append(str(kwargs["prompt"]))
        return successful_review()

    monkeypatch.setattr(server, "run_agy_model_prompt", agy)
    monkeypatch.setattr(server, "_execute_with_engine", codex)
    result = json.loads(
        await server.heterarchy_triad_orchestrate(
            task_spec="Read-only source review",
            target_code_or_context=source,
            mode="pipeline",
            workspace_dir=str(tmp_path),
        )
    )
    assert source in captured[0]
    assert (
        result["evidence_scope"]["inline_context_digest"]
        == "sha256:" + hashlib.sha256(source.encode()).hexdigest()
    )


@pytest.mark.asyncio
async def test_synthesis_is_not_static_review(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    async def response(**kwargs: object) -> JsonObject:
        return {
            "ok": True,
            "status": "succeeded",
            "structured_output": {
                "implementation": "def f(): return 1",
                "unit_tests": "def test_f(): assert f() == 1",
            },
        }

    monkeypatch.setattr(server, "_execute_with_engine", response)
    result = json.loads(
        await server.codex_targeted_synthesize(
            signature="def f() -> int",
            contract_and_constraints="Return one.",
            workspace_dir=str(tmp_path),
        )
    )
    assert result["evidence_scope"]["evidence_dimensions"]["static_code_review"] == "not_run"
    assert result["evidence_scope"]["evidence_dimensions"]["test_generation"] == "verified"


@pytest.mark.asyncio
async def test_failed_final_write_cannot_certify_cached_completed_record(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    original_persist = server.job_store._persist

    def fail_completed_write(record: object) -> None:
        assert isinstance(record, CodexJobRecord)
        if record.status == "completed":
            raise OSError("simulated final write failure")
        original_persist(record)

    monkeypatch.setattr(server.job_store, "_persist", fail_completed_write)
    result = await run_council(monkeypatch, tmp_path)
    assert result["collaboration_persistence"] == "failed"
    assert not result.get("collaboration_id")
    unconfirmed = str(result["unconfirmed_collaboration_id"])
    scope = result["evidence_scope"]
    assert isinstance(scope, dict)
    pack = await pack_for(
        unconfirmed, workspace_dir=str(tmp_path), expected_scope_digest=str(scope["scope_digest"])
    )
    assert pack["consensus_reached"] is False
    assert pack["review_approved"] is False
