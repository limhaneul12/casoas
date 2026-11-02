"""External Pydantic v2 boundary schemas (`app.schemas`), internal DTOs (`app.schemas.inner_dto`), and TypedDict return contracts (`app.schemas.inner_dto.dto_dict_types`)."""

from __future__ import annotations

from app.schemas.boundary_schemas import (
    ADVERSARIAL_AUDIT_SCHEMA,
    DIFFERENTIAL_CANDIDATES_SCHEMA,
    RESCUE_DIAGNOSIS_SCHEMA,
    TARGETED_SYNTHESIS_SCHEMA,
    AdversarialAuditOutput,
    ArgonReviewOutput,
    AuditFindingModel,
    ClaudeCraftOutput,
    DifferentialCandidateModel,
    DifferentialCandidatesOutput,
    JsonObject,
    JsonValue,
    RescueDiagnosisOutput,
    StrictSchemaModel,
    TargetedSynthesisOutput,
    TypedRecordDict,
    as_json_object,
    validate_structured_output,
)
from app.schemas.dag_input_schemas import (
    DagNodeInputModel,
    DagSpawnPlanInputModel,
    DagUlwRequestModel,
)

__all__ = [
    "ADVERSARIAL_AUDIT_SCHEMA",
    "DIFFERENTIAL_CANDIDATES_SCHEMA",
    "RESCUE_DIAGNOSIS_SCHEMA",
    "TARGETED_SYNTHESIS_SCHEMA",
    "AdversarialAuditOutput",
    "ArgonReviewOutput",
    "AuditFindingModel",
    "ClaudeCraftOutput",
    "DagNodeInputModel",
    "DagSpawnPlanInputModel",
    "DagUlwRequestModel",
    "DifferentialCandidateModel",
    "DifferentialCandidatesOutput",
    "JsonObject",
    "JsonValue",
    "RescueDiagnosisOutput",
    "StrictSchemaModel",
    "TargetedSynthesisOutput",
    "TypedRecordDict",
    "as_json_object",
    "validate_structured_output",
]
