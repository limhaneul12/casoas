"""Internal frozen dataclass DTOs for MCP gateway and tool request bundles."""

from __future__ import annotations

from collections.abc import Callable, Coroutine
from dataclasses import dataclass
from typing import TYPE_CHECKING

from app.core.collab_enums import (
    CanonicalModelId,
    EngineKind,
    OrchestrationMode,
    ProviderKind,
    ReasoningTier,
    SchemaName,
)
from app.core.model_contract_enums import (
    EngineAliasKind,
    SandboxModeKind,
    TaskTypeKind,
)
from app.schemas.boundary_schemas import JsonObject

if TYPE_CHECKING:
    from app.repositories.job_store import JobStore

type AsyncEngineCallable = Callable[..., Coroutine[object, object, JsonObject]]


@dataclass(frozen=True, slots=True)
class EngineDispatchRequest:
    """Validated request parameters for synchronous engine execution."""

    prompt: str
    schema_name: str
    output_schema: JsonObject | None
    workspace_dir: str | None = None
    sandbox_mode: str = SandboxModeKind.READ_ONLY
    model: str | None = None
    reasoning_effort: str | None = None
    engine: str = EngineAliasKind.CLI
    timeout_seconds: float = 300.0
    provider: str | None = None
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000


@dataclass(frozen=True, slots=True)
class BackgroundJobDispatchRequest:
    """Validated request parameters for asynchronous background job dispatch."""

    task_type: str
    prompt: str
    schema_name: str
    output_schema: JsonObject | None
    workspace_dir: str
    sandbox_mode: str
    model: str | None
    reasoning_effort: str | None
    engine: str
    timeout_seconds: float
    provider: str | None = None
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000

    @classmethod
    def from_codex_schema_request(
        cls,
        req: CodexSchemaToolRequest,
        prompt: str,
        schema_name: str,
        output_schema: JsonObject | None,
    ) -> BackgroundJobDispatchRequest:
        """Build a background dispatch request from a `CodexSchemaToolRequest`.

        Args:
            req: Validated Codex schema tool request bundle.
            prompt: Effective prompt including optional CodeGraph sliced context.
            schema_name: Registered schema identifier.
            output_schema: Strict JSON schema object or None.

        Returns:
            Initialized `BackgroundJobDispatchRequest` instance.
        """
        return cls(
            task_type=req.task_type,
            prompt=prompt,
            schema_name=schema_name,
            output_schema=output_schema,
            workspace_dir=req.workspace_dir,
            sandbox_mode=req.sandbox_mode,
            model=req.model,
            reasoning_effort=req.reasoning_effort,
            engine=req.engine,
            timeout_seconds=req.timeout_seconds,
            provider=req.provider,
            request_id=req.request_id,
            idempotency_key=req.idempotency_key,
            retry_same_request=req.retry_same_request,
            max_input_bytes=req.max_input_bytes,
        )

    @classmethod
    def from_claude_craft_request(
        cls, req: ClaudeCraftRequest, prompt: str
    ) -> BackgroundJobDispatchRequest:
        """Build a background dispatch request from a `ClaudeCraftRequest`.

        Args:
            req: Validated Claude surgical code craft request bundle.
            prompt: Effective Claude craft prompt string.

        Returns:
            Initialized `BackgroundJobDispatchRequest` instance.
        """
        return cls(
            task_type=str(SchemaName.CLAUDE_CRAFT),
            prompt=prompt,
            schema_name=str(SchemaName.CLAUDE_CRAFT),
            output_schema={},
            workspace_dir=req.workspace_dir,
            sandbox_mode=str(SandboxModeKind.READ_ONLY),
            model=req.claude_model,
            reasoning_effort=None,
            engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
            timeout_seconds=req.timeout_seconds,
            provider=str(ProviderKind.ANTIGRAVITY),
            request_id=req.request_id,
            idempotency_key=req.idempotency_key,
            retry_same_request=req.retry_same_request,
            max_input_bytes=req.max_input_bytes,
        )

    @classmethod
    def from_triad_request(cls, req: TriadOrchestrationRequest) -> BackgroundJobDispatchRequest:
        """Build a background dispatch request from a `TriadOrchestrationRequest`.

        Args:
            req: Validated Tri-Polar orchestration request bundle.

        Returns:
            Initialized `BackgroundJobDispatchRequest` instance.
        """
        return cls(
            task_type=str(TaskTypeKind.TRIAD_ORCHESTRATION),
            prompt=req.task_spec,
            schema_name=str(TaskTypeKind.TRIAD_ORCHESTRATION),
            output_schema=None,
            workspace_dir=req.workspace_dir,
            sandbox_mode=str(SandboxModeKind.READ_ONLY),
            model=req.claude_model,
            reasoning_effort=None,
            engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
            timeout_seconds=req.timeout_seconds,
            provider=str(ProviderKind.ANTIGRAVITY),
            request_id=req.request_id,
            idempotency_key=req.idempotency_key,
            retry_same_request=req.retry_same_request,
            max_input_bytes=req.max_input_bytes,
        )


@dataclass(frozen=True, slots=True)
class CodexSchemaToolRequest:
    """Validated request bundle for structured Codex schema tool dispatch."""

    task_type: str
    prompt: str
    provided_context_or_code: str
    workspace_dir: str = "."
    sandbox_mode: str = SandboxModeKind.READ_ONLY
    model: str | None = None
    reasoning_effort: str | None = None
    async_background: bool = False
    engine: str = EngineAliasKind.CLI
    timeout_seconds: float = 300.0
    provider: str | None = None
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000
    files_inspected: list[str] | None = None
    symbols_inspected: list[str] | None = None
    executed_test_commands: list[JsonObject] | None = None


@dataclass(frozen=True, slots=True)
class ClaudeCraftRequest:
    """Validated request bundle for Claude Sonnet/Opus surgical code craft."""

    task_spec: str
    target_code_or_context: str = ""
    invariants_and_rules: str = ""
    language: str = "python"
    claude_model: str = CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM
    timeout_seconds: float = 180.0
    workspace_dir: str = "."
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False
    max_input_bytes: int = 256_000
    files_inspected: list[str] | None = None
    symbols_inspected: list[str] | None = None
    executed_test_commands: list[JsonObject] | None = None
    async_background: bool = False


@dataclass(frozen=True, slots=True)
class TriadOrchestrationRequest:
    """Validated request bundle for Tri-Polar Heterarchy orchestration."""

    task_spec: str
    target_code_or_context: str = ""
    invariants_and_rules: str = ""
    language: str = "python"
    mode: str = OrchestrationMode.PIPELINE
    workspace_dir: str = "."
    claude_model: str = CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM
    codex_model: str | None = None
    codex_reasoning_effort: str = ReasoningTier.XHIGH
    argon_model: str = CanonicalModelId.GEMINI_4_ARGON_HIGH
    invoke_argon_rpc: bool = True
    argon_handoff_ticket_id: str | None = None
    timeout_seconds: float = 300.0
    files_inspected: list[str] | None = None
    symbols_inspected: list[str] | None = None
    executed_test_commands: list[JsonObject] | None = None
    max_input_bytes: int = 256_000
    resume_collaboration_id: str | None = None
    max_rounds: int = 1
    run_verification_gates: bool = False
    async_background: bool = False
    request_id: str | None = None
    idempotency_key: str | None = None
    retry_same_request: bool = False


@dataclass(frozen=True, slots=True)
class DecisionPackRequest:
    """Validated request bundle for Alexandria decision pack synthesis."""

    project_scope: str
    topic_title: str
    argon_architecture_intent: str
    codex_audit_or_synthesis_summary: str
    final_resolution: str
    sonnet_craft_or_review_summary: str = ""
    verification_evidence: str = ""
    collaboration_id: str | None = None
    workspace_dir: str | None = None
    expected_scope_digest: str | None = None


@dataclass(frozen=True, slots=True)
class ServerGatewayBindings:
    """Runtime store and transport callables injected from the server module."""

    job_store: JobStore
    run_agy_fn: AsyncEngineCallable
    run_http_fn: AsyncEngineCallable
    run_codex_fn: AsyncEngineCallable


__all__ = [
    "AsyncEngineCallable",
    "BackgroundJobDispatchRequest",
    "ClaudeCraftRequest",
    "CodexSchemaToolRequest",
    "DecisionPackRequest",
    "EngineDispatchRequest",
    "ServerGatewayBindings",
    "TriadOrchestrationRequest",
]
