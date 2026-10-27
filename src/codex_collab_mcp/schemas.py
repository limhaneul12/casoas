"""JSON Schemas and Pydantic v2 closed-boundary models for Codex Collaboration tools."""

from __future__ import annotations

from typing import TYPE_CHECKING, cast

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from codex_collab_mcp.collab_enums import AuditSeverity, AuditVerdict, SchemaName

if TYPE_CHECKING:
    from codex_collab_mcp.evidence.evidence_dicts import (
        EvidenceDimensionsDict,
        EvidenceScopeDict,
        TriadAdjudicationDict,
    )
    from codex_collab_mcp.routing.contract_dicts import (
        BudgetEnvelopeDict,
        ExecutionReceiptDict,
        HandoffTicketDict,
        StderrAuthClassificationDict,
    )
    from codex_collab_mcp.transports.transport_dicts import (
        AgyAuthBoundaryDict,
        CodexCliAuthBoundaryDict,
    )

type JsonValue = str | int | float | bool | list[JsonValue] | dict[str, JsonValue] | None
type JsonObject = dict[str, JsonValue]
type TypedRecordDict = (
    ExecutionReceiptDict
    | BudgetEnvelopeDict
    | StderrAuthClassificationDict
    | AgyAuthBoundaryDict
    | CodexCliAuthBoundaryDict
    | EvidenceScopeDict
    | EvidenceDimensionsDict
    | TriadAdjudicationDict
    | HandoffTicketDict
    | dict[str, EvidenceDimensionsDict]
)


def as_json_object(record: TypedRecordDict | JsonObject) -> JsonObject:
    """Coerce a closed TypedDict record to JsonObject at dynamic payload boundaries without copying.

    Args:
        record: Validated TypedDict instance or JsonObject mapping.

    Returns:
        The same runtime dictionary typed as JsonObject for open payload assembly.
    """
    return cast(JsonObject, record)  # type-contract: allow-cast


class StrictSchemaModel(BaseModel):
    """Canonical Pydantic v2 closed-boundary base model."""

    model_config = ConfigDict(
        extra="forbid",
        frozen=True,
        use_enum_values=True,
        validate_default=True,
    )


class AuditFindingModel(StrictSchemaModel):
    """Validated red-team finding entry."""

    severity: AuditSeverity
    category: str = Field(
        description=(
            "Category e.g. 'race_condition', 'memory_leak', 'logic_bug', "
            "'type_hole', 'edge_case', 'security'"
        )
    )
    location: str = Field(description="Function, class, or line reference")
    flaw_description: str = Field(description="Precise technical explanation of the flaw")
    trigger_scenario: str = Field(
        description="Concrete input, timing, or state sequence that triggers the failure"
    )
    recommended_fix: str = Field(description="Minimal, idiomatic code or architectural fix")


class AdversarialAuditOutput(StrictSchemaModel):
    """Pydantic v2 boundary contract for Codex adversarial audit outputs."""

    verdict: AuditVerdict = Field(
        description="PASS if production-ready, WARN if minor risks exist, BLOCK if critical flaws exist."
    )
    risk_score: int = Field(description="0 (safe) to 100 (critical failure risk)")
    executive_critique: str = Field(
        description="Direct, unfiltered technical summary of what could break in production."
    )
    findings: list[AuditFindingModel]
    failing_test_cases: list[str] = Field(
        description="Concrete pytest/unit test code snippets that would expose the flaws."
    )
    alexandria_decision_note: str = Field(
        description=(
            "Concise architectural takeaway suitable for storing in "
            "Alexandria memory via alexandria_verified_upsert."
        )
    )


class TargetedSynthesisOutput(StrictSchemaModel):
    """Pydantic v2 boundary contract for Codex targeted synthesis outputs."""

    language: str = Field(description="Programming language (e.g., python, rust, typescript)")
    implementation_code: str = Field(
        description="Complete, drop-in, production-ready code with zero placeholders or TODOs."
    )
    unit_tests: str = Field(description="Runnable unit test suite verifying the implementation.")
    edge_cases_handled: list[str] = Field(
        description="List of boundary conditions explicitly guarded in the code."
    )
    complexity_analysis: str = Field(
        description="Time and space complexity + concurrency/lock-contention guarantees."
    )


class DifferentialCandidateModel(StrictSchemaModel):
    """Single optimization-axis candidate implementation."""

    axis: str = Field(
        description=(
            "Optimization axis (e.g. 'Maximal Throughput / Zero-Copy', "
            "'Minimal Simplicity / Stdlib-Only', 'Strict Fault-Isolation')"
        )
    )
    strategy_name: str = Field(description="Short title for this candidate approach")
    code: str = Field(description="Complete, runnable implementation for this candidate")
    pros: list[str]
    cons: list[str]
    estimated_complexity: str = Field(description="Big-O time/memory and operational overhead")


class DifferentialCandidatesOutput(StrictSchemaModel):
    """Pydantic v2 boundary contract for differential candidate exploration."""

    candidates: list[DifferentialCandidateModel]
    argon_selection_guide: str = Field(
        description=(
            "Decision matrix guiding Gemini 4 Argon on which candidate "
            "fits which system constraint."
        )
    )


class RescueDiagnosisOutput(StrictSchemaModel):
    """Pydantic v2 boundary contract for isolated rescue sandbox diagnosis."""

    root_cause: str = Field(description="Exact root cause identified for the failure or bug.")
    fix_strategy: str = Field(description="Why this fix resolves the issue without regressions.")
    patch_or_replacement_code: str = Field(
        description="Unified diff or complete replacement code block."
    )
    verification_commands: list[str] = Field(
        description="Exact commands Argon or the user should run to verify the fix."
    )


class ClaudeCraftOutput(StrictSchemaModel):
    """Pydantic v2 boundary contract for Claude Sonnet 5.5 / Opus 5.5 code craft."""

    verdict: AuditVerdict
    summary: str
    crafted_code: str
    design_highlights: list[str]
    edge_cases_guarded: list[str]
    handoff_notes_for_codex_audit: str


class ArgonReviewOutput(StrictSchemaModel):
    """Pydantic v2 boundary contract for Gemini 4 Argon architectural adjudication."""

    verdict: AuditVerdict
    summary: str
    architectural_risks: list[str]
    final_recommendation: str


def _resolve_strict_openai_schema_node(
    node: JsonObject,
    defs: dict[str, JsonObject],
) -> JsonObject:
    """Recursively inline $defs and enforce OpenAI strict JSON Schema object constraints.

    Args:
        node: Raw JSON Schema node generated by Pydantic v2.
        defs: Shared $defs mapping extracted from the root Pydantic schema.

    Returns:
        Normalized OpenAI-compatible strict JSON Schema dictionary.
    """
    ref_val = node.get("$ref")
    if isinstance(ref_val, str):
        ref_name = ref_val.rsplit("/", maxsplit=1)[-1]
        target = defs.get(ref_name)
        if isinstance(target, dict):
            return _resolve_strict_openai_schema_node(target, defs)
    out: JsonObject = {}
    for key in ("type", "enum", "description"):
        if key in node:
            out[key] = node[key]
    node_type = node.get("type")
    props_raw = node.get("properties")
    if node_type == "object" and isinstance(props_raw, dict):
        resolved_props: JsonObject = {}
        req_keys: list[JsonValue] = []
        for prop_name, prop_schema in props_raw.items():
            if isinstance(prop_schema, dict):
                resolved_props[prop_name] = _resolve_strict_openai_schema_node(prop_schema, defs)
                req_keys.append(prop_name)
        out["type"] = "object"
        out["additionalProperties"] = False
        out["properties"] = resolved_props
        out["required"] = req_keys
    elif node_type == "array":
        items_raw = node.get("items")
        if isinstance(items_raw, dict):
            out["items"] = _resolve_strict_openai_schema_node(items_raw, defs)
    return out


def _build_strict_openai_schema(model_cls: type[StrictSchemaModel]) -> JsonObject:
    """Derive an OpenAI-strict JSON Schema dictionary from a StrictSchemaModel class.

    Args:
        model_cls: Pydantic v2 StrictSchemaModel subclass.

    Returns:
        Strict JSON Schema dictionary with additionalProperties=False and all keys required.
    """
    raw_obj = model_cls.model_json_schema()
    defs_raw = raw_obj.get("$defs")
    defs: dict[str, JsonObject] = {}
    if isinstance(defs_raw, dict):
        for k, v in defs_raw.items():
            if isinstance(k, str) and isinstance(v, dict):
                defs[k] = v
    return _resolve_strict_openai_schema_node(raw_obj, defs)


ADVERSARIAL_AUDIT_SCHEMA: JsonObject = _build_strict_openai_schema(AdversarialAuditOutput)
TARGETED_SYNTHESIS_SCHEMA: JsonObject = _build_strict_openai_schema(TargetedSynthesisOutput)
DIFFERENTIAL_CANDIDATES_SCHEMA: JsonObject = _build_strict_openai_schema(
    DifferentialCandidatesOutput
)
RESCUE_DIAGNOSIS_SCHEMA: JsonObject = _build_strict_openai_schema(RescueDiagnosisOutput)

_BOUNDARY_MODELS: dict[str, type[StrictSchemaModel]] = {
    SchemaName.ADVERSARIAL_AUDIT: AdversarialAuditOutput,
    SchemaName.TARGETED_SYNTHESIS: TargetedSynthesisOutput,
    SchemaName.DIFFERENTIAL_CANDIDATES: DifferentialCandidatesOutput,
    SchemaName.RESCUE_DIAGNOSIS: RescueDiagnosisOutput,
    SchemaName.CLAUDE_CRAFT: ClaudeCraftOutput,
    SchemaName.ARGON_REVIEW: ArgonReviewOutput,
}


def validate_structured_output(
    schema_name: str,
    payload: JsonObject,
) -> tuple[bool, JsonObject | None, str | None]:
    """Validate structured model output at the boundary using frozen Pydantic v2 models.

    Args:
        schema_name: Registered schema identifier to validate against.
        payload: Raw JSON object returned by the model.

    Returns:
        Tuple of `(is_valid, normalized_payload, error_message)`.
    """
    model_cls = _BOUNDARY_MODELS.get(schema_name)
    if model_cls is None:
        return (True, payload, None)
    try:
        validated = model_cls.model_validate(payload)
        dumped = validated.model_dump(mode="json")
        if isinstance(dumped, dict):
            return (True, dumped, None)
        return (False, None, "Validated output did not serialize to a JSON object.")
    except ValidationError as exc:
        return (False, None, str(exc))
