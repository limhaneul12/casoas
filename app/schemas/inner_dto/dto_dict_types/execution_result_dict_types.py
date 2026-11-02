"""TypedDict return-type contracts for `execution_result_dtos.py` `.to_dict(self)` methods."""

from __future__ import annotations

from typing import TypedDict

from app.schemas.boundary_schemas import JsonObject, JsonValue


class CodexRejectedExtraFieldsDict(TypedDict):
    """Serialized representation of a `CodexRejectedExtraFields` dataclass."""

    workspace_dir: str
    model: str
    auth_boundary: JsonObject


class CodexBudgetExtraFieldsDict(TypedDict):
    """Serialized representation of a `CodexBudgetExtraFields` dataclass."""

    workspace_dir: str
    model: str


class CodexTimeoutPartialResultDict(TypedDict):
    """Serialized representation of a `CodexTimeoutPartialResult` dataclass."""

    request_id: str
    status: str
    outcome_state: str


class CodexDeduplicatedResponseDict(TypedDict):
    """Serialized representation of a `CodexDeduplicatedResponse` dataclass."""

    ok: bool
    deduplicated_existing_request: bool
    request_id: str
    job_id: str
    status: str
    outcome_state: str
    attempt_count: int
    provider: str
    engine: str
    workspace_dir: str
    model: str
    result: JsonValue
    error: str | None
    receipt: JsonObject | None


class CodexExecTimeoutResultDict(TypedDict):
    """Serialized representation of a `CodexExecTimeoutResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    workspace_dir: str
    model: str
    duration_seconds: float
    error: str
    receipt: JsonObject
    budget: JsonObject
    auth_boundary: JsonObject


class CodexExecCompletedResultDict(TypedDict):
    """Serialized representation of a `CodexExecCompletedResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    exit_code: int | None
    session_id: str | None
    workspace_dir: str
    model: str
    requested_model: str
    duration_seconds: float
    data: JsonValue
    structured_output: JsonObject | None
    stderr: str
    receipt: JsonObject
    budget: JsonObject
    auth_boundary: JsonObject


class CodexGitReviewTimeoutResultDict(TypedDict):
    """Serialized representation of a `CodexGitReviewTimeoutResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    workspace_dir: str
    error: str
    receipt: JsonObject


class CodexGitReviewCompletedResultDict(TypedDict):
    """Serialized representation of a `CodexGitReviewCompletedResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    exit_code: int | None
    workspace_dir: str
    review_output: str
    stderr: str
    receipt: JsonObject
    auth_boundary: JsonObject


class CodexSessionFollowupTimeoutResultDict(TypedDict):
    """Serialized representation of a `CodexSessionFollowupTimeoutResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    mode: str
    parent_session_id: str
    error: str
    receipt: JsonObject


class CodexSessionFollowupCompletedResultDict(TypedDict):
    """Serialized representation of a `CodexSessionFollowupCompletedResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    exit_code: int | None
    mode: str
    parent_session_id: str
    session_id: str
    response: str
    stderr: str
    receipt: JsonObject
    auth_boundary: JsonObject


class HttpChatMessageDict(TypedDict):
    """Serialized representation of an `HttpChatMessage` dataclass."""

    role: str
    content: str


class HttpJsonSchemaSpecDict(TypedDict):
    """Serialized representation of an `HttpJsonSchemaSpec` dataclass."""

    name: str
    strict: bool
    schema: JsonObject


class HttpResponseFormatDict(TypedDict):
    """Serialized representation of an `HttpResponseFormat` dataclass."""

    type: str
    json_schema: HttpJsonSchemaSpecDict


class HttpChatCompletionRequestDict(TypedDict):
    """Serialized representation of an `HttpChatCompletionRequest` dataclass."""

    model: str
    messages: list[JsonValue]


class HttpErrorExecutionResultDict(TypedDict):
    """Serialized representation of an `HttpErrorExecutionResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    model: str
    error: str
    receipt: JsonObject
    budget: JsonObject


class HttpCompletedExecutionResultDict(TypedDict):
    """Serialized representation of an `HttpCompletedExecutionResult` dataclass."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    job_id: str
    provider: str
    engine: str
    model: str
    data: JsonValue
    structured_output: JsonObject | None
    receipt: JsonObject
    budget: JsonObject


__all__ = [
    "CodexBudgetExtraFieldsDict",
    "CodexDeduplicatedResponseDict",
    "CodexExecCompletedResultDict",
    "CodexExecTimeoutResultDict",
    "CodexGitReviewCompletedResultDict",
    "CodexGitReviewTimeoutResultDict",
    "CodexRejectedExtraFieldsDict",
    "CodexSessionFollowupCompletedResultDict",
    "CodexSessionFollowupTimeoutResultDict",
    "CodexTimeoutPartialResultDict",
    "HttpChatCompletionRequestDict",
    "HttpChatMessageDict",
    "HttpCompletedExecutionResultDict",
    "HttpErrorExecutionResultDict",
    "HttpJsonSchemaSpecDict",
    "HttpResponseFormatDict",
]
