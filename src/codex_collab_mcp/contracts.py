"""Routing, execution receipt, evidence scope, budget, and adjudication contracts facade."""

from __future__ import annotations

from codex_collab_mcp.evidence.triad_adjudicator import (
    adjudicate_collaboration_and_consensus,
)
from codex_collab_mcp.routing.auth_redaction import (
    classify_stderr_and_auth_boundaries,
    get_python_runtime_info,
    redact_sensitive_text,
)
from codex_collab_mcp.routing.model_specs import (
    ANTIGRAVITY_MODEL_SPECS,
    CODEX_SUPPORTED_MODELS,
)
from codex_collab_mcp.routing.policy_engine import (
    build_execution_receipt,
    build_input_budget_exceeded_response,
    build_rejected_pre_dispatch_response,
    validate_and_resolve_route,
)
from codex_collab_mcp.routing.receipt_budget import evaluate_budget_envelope

__all__ = [
    "ANTIGRAVITY_MODEL_SPECS",
    "CODEX_SUPPORTED_MODELS",
    "adjudicate_collaboration_and_consensus",
    "build_execution_receipt",
    "build_input_budget_exceeded_response",
    "build_rejected_pre_dispatch_response",
    "classify_stderr_and_auth_boundaries",
    "evaluate_budget_envelope",
    "get_python_runtime_info",
    "redact_sensitive_text",
    "validate_and_resolve_route",
]
