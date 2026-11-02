"""Internal frozen dataclass DTOs for Codex CLI and HTTP execution results."""

from __future__ import annotations

from dataclasses import dataclass

from app.schemas.boundary_schemas import JsonObject, JsonValue
from app.schemas.inner_dto.dto_dict_types import (
    CodexBudgetExtraFieldsDict,
    CodexDeduplicatedResponseDict,
    CodexExecCompletedResultDict,
    CodexExecTimeoutResultDict,
    CodexGitReviewCompletedResultDict,
    CodexGitReviewTimeoutResultDict,
    CodexRejectedExtraFieldsDict,
    CodexSessionFollowupCompletedResultDict,
    CodexSessionFollowupTimeoutResultDict,
    CodexTimeoutPartialResultDict,
    HttpChatCompletionRequestDict,
    HttpChatMessageDict,
    HttpCompletedExecutionResultDict,
    HttpErrorExecutionResultDict,
    HttpJsonSchemaSpecDict,
    HttpResponseFormatDict,
)


@dataclass(frozen=True, slots=True)
class CodexRejectedExtraFields:
    """Internal DTO for extra pre-dispatch rejection fields in Codex CLI."""

    workspace_dir: str
    model: str
    auth_boundary: JsonObject

    def to_dict(self) -> CodexRejectedExtraFieldsDict:
        """Serialize extra rejection fields to a TypedDict.

        Returns:
            Typed `CodexRejectedExtraFieldsDict` mapping.
        """
        return CodexRejectedExtraFieldsDict(
            workspace_dir=self.workspace_dir,
            model=self.model,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class CodexBudgetExtraFields:
    """Internal DTO for extra input-budget rejection fields in Codex CLI."""

    workspace_dir: str
    model: str

    def to_dict(self) -> CodexBudgetExtraFieldsDict:
        """Serialize extra budget rejection fields to a TypedDict.

        Returns:
            Typed `CodexBudgetExtraFieldsDict` mapping.
        """
        return CodexBudgetExtraFieldsDict(workspace_dir=self.workspace_dir, model=self.model)


@dataclass(frozen=True, slots=True)
class CodexTimeoutPartialResult:
    """Internal DTO for a partial result persisted when a Codex CLI execution times out."""

    request_id: str
    status: str
    outcome_state: str

    def to_dict(self) -> CodexTimeoutPartialResultDict:
        """Serialize the timeout partial result to a TypedDict.

        Returns:
            Typed `CodexTimeoutPartialResultDict` mapping.
        """
        return CodexTimeoutPartialResultDict(
            request_id=self.request_id,
            status=self.status,
            outcome_state=self.outcome_state,
        )


@dataclass(frozen=True, slots=True)
class CodexDeduplicatedResponse:
    """Internal DTO for a deduplicated Codex CLI request outcome."""

    ok: bool
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
    deduplicated_existing_request: bool = True

    def to_dict(self) -> CodexDeduplicatedResponseDict:
        """Serialize the deduplicated Codex CLI response to a TypedDict.

        Returns:
            Typed `CodexDeduplicatedResponseDict` mapping.
        """
        return CodexDeduplicatedResponseDict(
            ok=self.ok,
            deduplicated_existing_request=self.deduplicated_existing_request,
            request_id=self.request_id,
            job_id=self.job_id,
            status=self.status,
            outcome_state=self.outcome_state,
            attempt_count=self.attempt_count,
            provider=self.provider,
            engine=self.engine,
            workspace_dir=self.workspace_dir,
            model=self.model,
            result=self.result,
            error=self.error,
            receipt=self.receipt,
        )


@dataclass(frozen=True, slots=True)
class CodexExecTimeoutResult:
    """Internal DTO for a timeout response from `codex exec`."""

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

    def to_dict(self) -> CodexExecTimeoutResultDict:
        """Serialize the `codex exec` timeout result to a TypedDict.

        Returns:
            Typed `CodexExecTimeoutResultDict` mapping.
        """
        return CodexExecTimeoutResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            workspace_dir=self.workspace_dir,
            model=self.model,
            duration_seconds=self.duration_seconds,
            error=self.error,
            receipt=self.receipt,
            budget=self.budget,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class CodexExecCompletedResult:
    """Internal DTO for a completed `codex exec` process outcome."""

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

    def to_dict(self) -> CodexExecCompletedResultDict:
        """Serialize the completed Codex CLI result to a TypedDict.

        Returns:
            Typed `CodexExecCompletedResultDict` mapping.
        """
        return CodexExecCompletedResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            exit_code=self.exit_code,
            session_id=self.session_id,
            workspace_dir=self.workspace_dir,
            model=self.model,
            requested_model=self.requested_model,
            duration_seconds=self.duration_seconds,
            data=self.data,
            structured_output=self.structured_output,
            stderr=self.stderr,
            receipt=self.receipt,
            budget=self.budget,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class CodexGitReviewTimeoutResult:
    """Internal DTO for a timeout outcome from `codex review`."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    workspace_dir: str
    error: str
    receipt: JsonObject

    def to_dict(self) -> CodexGitReviewTimeoutResultDict:
        """Serialize the git review timeout result to a TypedDict.

        Returns:
            Typed `CodexGitReviewTimeoutResultDict` mapping.
        """
        return CodexGitReviewTimeoutResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            workspace_dir=self.workspace_dir,
            error=self.error,
            receipt=self.receipt,
        )


@dataclass(frozen=True, slots=True)
class CodexGitReviewCompletedResult:
    """Internal DTO for a completed `codex review` outcome."""

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

    def to_dict(self) -> CodexGitReviewCompletedResultDict:
        """Serialize the completed git review result to a TypedDict.

        Returns:
            Typed `CodexGitReviewCompletedResultDict` mapping.
        """
        return CodexGitReviewCompletedResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            exit_code=self.exit_code,
            workspace_dir=self.workspace_dir,
            review_output=self.review_output,
            stderr=self.stderr,
            receipt=self.receipt,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class CodexSessionFollowupTimeoutResult:
    """Internal DTO for a timeout outcome from `codex exec resume/fork`."""

    ok: bool
    status: str
    outcome_state: str
    request_id: str
    mode: str
    parent_session_id: str
    error: str
    receipt: JsonObject

    def to_dict(self) -> CodexSessionFollowupTimeoutResultDict:
        """Serialize the session follow-up timeout result to a TypedDict.

        Returns:
            Typed `CodexSessionFollowupTimeoutResultDict` mapping.
        """
        return CodexSessionFollowupTimeoutResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            mode=self.mode,
            parent_session_id=self.parent_session_id,
            error=self.error,
            receipt=self.receipt,
        )


@dataclass(frozen=True, slots=True)
class CodexSessionFollowupCompletedResult:
    """Internal DTO for a completed `codex exec resume/fork` outcome."""

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

    def to_dict(self) -> CodexSessionFollowupCompletedResultDict:
        """Serialize the completed session follow-up result to a TypedDict.

        Returns:
            Typed `CodexSessionFollowupCompletedResultDict` mapping.
        """
        return CodexSessionFollowupCompletedResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            exit_code=self.exit_code,
            mode=self.mode,
            parent_session_id=self.parent_session_id,
            session_id=self.session_id,
            response=self.response,
            stderr=self.stderr,
            receipt=self.receipt,
            auth_boundary=self.auth_boundary,
        )


@dataclass(frozen=True, slots=True)
class HttpChatMessage:
    """Internal DTO for a single OpenAI-compatible chat message entry."""

    role: str
    content: str

    def to_dict(self) -> HttpChatMessageDict:
        """Serialize the chat message to a TypedDict.

        Returns:
            Typed `HttpChatMessageDict` mapping.
        """
        return HttpChatMessageDict(role=self.role, content=self.content)


@dataclass(frozen=True, slots=True)
class HttpJsonSchemaSpec:
    """Internal DTO for a strict JSON schema specification in HTTP chat completions."""

    name: str
    strict: bool
    schema: JsonObject

    def to_dict(self) -> HttpJsonSchemaSpecDict:
        """Serialize the JSON schema spec to a TypedDict.

        Returns:
            Typed `HttpJsonSchemaSpecDict` mapping.
        """
        return HttpJsonSchemaSpecDict(name=self.name, strict=self.strict, schema=self.schema)


@dataclass(frozen=True, slots=True)
class HttpResponseFormat:
    """Internal DTO for a response format envelope in HTTP chat completions."""

    type: str
    json_schema: HttpJsonSchemaSpecDict

    def to_dict(self) -> HttpResponseFormatDict:
        """Serialize the response format envelope to a TypedDict.

        Returns:
            Typed `HttpResponseFormatDict` mapping.
        """
        return HttpResponseFormatDict(type=self.type, json_schema=self.json_schema)


@dataclass(frozen=True, slots=True)
class HttpChatCompletionRequest:
    """Internal DTO for a `/chat/completions` request body."""

    model: str
    messages: list[JsonValue]

    def to_dict(self) -> HttpChatCompletionRequestDict:
        """Serialize the chat completion request to a TypedDict.

        Returns:
            Typed `HttpChatCompletionRequestDict` mapping.
        """
        return HttpChatCompletionRequestDict(model=self.model, messages=self.messages)


@dataclass(frozen=True, slots=True)
class HttpErrorExecutionResult:
    """Internal DTO for an HTTP fallback timeout or error outcome."""

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

    def to_dict(self) -> HttpErrorExecutionResultDict:
        """Serialize the HTTP error execution result to a TypedDict.

        Returns:
            Typed `HttpErrorExecutionResultDict` mapping.
        """
        return HttpErrorExecutionResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            model=self.model,
            error=self.error,
            receipt=self.receipt,
            budget=self.budget,
        )


@dataclass(frozen=True, slots=True)
class HttpCompletedExecutionResult:
    """Internal DTO for a completed HTTP fallback outcome."""

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

    def to_dict(self) -> HttpCompletedExecutionResultDict:
        """Serialize the completed HTTP execution result to a TypedDict.

        Returns:
            Typed `HttpCompletedExecutionResultDict` mapping.
        """
        return HttpCompletedExecutionResultDict(
            ok=self.ok,
            status=self.status,
            outcome_state=self.outcome_state,
            request_id=self.request_id,
            job_id=self.job_id,
            provider=self.provider,
            engine=self.engine,
            model=self.model,
            data=self.data,
            structured_output=self.structured_output,
            receipt=self.receipt,
            budget=self.budget,
        )


__all__ = [
    "CodexBudgetExtraFields",
    "CodexDeduplicatedResponse",
    "CodexExecCompletedResult",
    "CodexExecTimeoutResult",
    "CodexGitReviewCompletedResult",
    "CodexGitReviewTimeoutResult",
    "CodexRejectedExtraFields",
    "CodexSessionFollowupCompletedResult",
    "CodexSessionFollowupTimeoutResult",
    "CodexTimeoutPartialResult",
    "HttpChatCompletionRequest",
    "HttpChatMessage",
    "HttpCompletedExecutionResult",
    "HttpErrorExecutionResult",
    "HttpJsonSchemaSpec",
    "HttpResponseFormat",
]
