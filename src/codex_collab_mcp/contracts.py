"""Routing, execution receipt, evidence scope, budget, and adjudication contracts facade."""

from __future__ import annotations

from codex_collab_mcp.domain.evidence_tracker import (
    attach_evidence_scope,
    canonical_sha256,
    compact_evidence_scope,
    compute_task_digest,
    prepare_triad_role_evidence,
    resolve_workspace_dir,
)
from codex_collab_mcp.domain.routing_catalog import (
    ANTIGRAVITY_MODEL_SPECS,
    CODEX_SUPPORTED_MODELS,
    classify_stderr_and_auth_boundaries,
    evaluate_budget_envelope,
    get_python_runtime_info,
    redact_sensitive_text,
)
from codex_collab_mcp.domain.routing_policy import (
    build_execution_receipt,
    build_input_budget_exceeded_response,
    build_rejected_pre_dispatch_response,
    validate_and_resolve_route,
)
from codex_collab_mcp.domain.triad_adjudicator import (
    TRIAD_REQUIRED_ROLES,
    TRIAD_RESULT_SCHEMA_VERSION,
    adjudicate_collaboration_and_consensus,
    extract_verdict,
    map_response_to_role_state,
    recompute_saved_triad_adjudication,
)

__all__ = [
    "ANTIGRAVITY_MODEL_SPECS",
    "CODEX_SUPPORTED_MODELS",
    "TRIAD_REQUIRED_ROLES",
    "TRIAD_RESULT_SCHEMA_VERSION",
    "adjudicate_collaboration_and_consensus",
    "attach_evidence_scope",
    "build_execution_receipt",
    "build_input_budget_exceeded_response",
    "build_rejected_pre_dispatch_response",
    "canonical_sha256",
    "classify_stderr_and_auth_boundaries",
    "compact_evidence_scope",
    "compute_task_digest",
    "evaluate_budget_envelope",
    "extract_verdict",
    "get_python_runtime_info",
    "map_response_to_role_state",
    "prepare_triad_role_evidence",
    "recompute_saved_triad_adjudication",
    "redact_sensitive_text",
    "resolve_workspace_dir",
    "validate_and_resolve_route",
]
