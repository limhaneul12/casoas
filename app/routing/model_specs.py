"""Model specifications, explicit aliases, and default routes for Antigravity and Codex."""

from __future__ import annotations

from dataclasses import dataclass

from app.core.collab_enums import (
    AuthContextKind,
    CanonicalModelId,
    EngineKind,
    ProviderKind,
    ReasoningTier,
)
from app.core.model_contract_enums import (
    AntigravityModelPlaceholder,
    ArgonModelAlias,
    Codex6AstraAlias,
    Codex6LunaAlias,
    Codex6SolAlias,
    Codex6TerraAlias,
    Codex61SolAlias,
    Codex61TerraAlias,
    FlashModelAlias,
    ModelCapabilityKind,
    OpusModelAlias,
    SonnetModelAlias,
    TieredModelAlias,
)
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import ModelSpecDict


@dataclass(frozen=True, slots=True)
class ModelSpec:
    """Authoritative capability and routing specification for a single model."""

    provider: str
    engine: str
    model_id: str
    auth_context: str
    reasoning_tier: str
    placeholder: str | None
    capabilities: tuple[str, ...]

    def to_dict(self) -> ModelSpecDict:
        """Serialize the model specification to a TypedDict.

        Returns:
            Serialized ModelSpecDict mapping.
        """
        return ModelSpecDict(
            provider=self.provider,
            engine=self.engine,
            model_id=self.model_id,
            auth_context=self.auth_context,
            reasoning_tier=self.reasoning_tier,
            placeholder=self.placeholder,
            capabilities=list(self.capabilities),
        )


_CRAFT_COUNCIL: tuple[str, ...] = (
    str(ModelCapabilityKind.CODE_CRAFT),
    str(ModelCapabilityKind.ADVISORY_REVIEW),
    str(ModelCapabilityKind.TRIAD_COUNCIL),
)
_CRAFT_ONLY: tuple[str, ...] = (
    str(ModelCapabilityKind.CODE_CRAFT),
    str(ModelCapabilityKind.ADVISORY_REVIEW),
)
_MACRO_COUNCIL: tuple[str, ...] = (
    str(ModelCapabilityKind.MACRO_ARCHITECTURE),
    str(ModelCapabilityKind.ADVISORY_REVIEW),
    str(ModelCapabilityKind.TRIAD_COUNCIL),
)
_MACRO_ONLY: tuple[str, ...] = (
    str(ModelCapabilityKind.MACRO_ARCHITECTURE),
    str(ModelCapabilityKind.ADVISORY_REVIEW),
)
_FAST_CAPS: tuple[str, ...] = (str(ModelCapabilityKind.FAST_REVIEW),)
_CODEX_CAPS: tuple[str, ...] = (
    str(ModelCapabilityKind.ADVERSARIAL_AUDIT),
    str(ModelCapabilityKind.TARGETED_SYNTHESIS),
    str(ModelCapabilityKind.RESCUE),
    str(ModelCapabilityKind.TRIAD_COUNCIL),
)

ANTIGRAVITY_MODEL_SPECS: dict[str, ModelSpec] = {
    str(model_id): ModelSpec(
        provider=str(ProviderKind.ANTIGRAVITY),
        engine=str(EngineKind.ANTIGRAVITY_LOCAL_RPC),
        model_id=str(model_id),
        auth_context=str(AuthContextKind.ANTIGRAVITY_LOCAL_CSRF),
        reasoning_tier=str(tier),
        placeholder=str(placeholder),
        capabilities=caps,
    )
    for model_id, tier, placeholder, caps in (
        (
            CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM,
            ReasoningTier.MEDIUM,
            AntigravityModelPlaceholder.M404,
            _CRAFT_COUNCIL,
        ),
        (
            CanonicalModelId.CLAUDE_SONNET_5_5_HIGH,
            ReasoningTier.HIGH,
            AntigravityModelPlaceholder.M405,
            _CRAFT_COUNCIL,
        ),
        (
            CanonicalModelId.CLAUDE_SONNET_5_5_LOW,
            ReasoningTier.LOW,
            AntigravityModelPlaceholder.M403,
            _CRAFT_ONLY,
        ),
        (
            CanonicalModelId.CLAUDE_OPUS_5_5_MEDIUM,
            ReasoningTier.MEDIUM,
            AntigravityModelPlaceholder.M401,
            _CRAFT_COUNCIL,
        ),
        (
            CanonicalModelId.CLAUDE_OPUS_5_5_HIGH,
            ReasoningTier.HIGH,
            AntigravityModelPlaceholder.M402,
            _CRAFT_COUNCIL,
        ),
        (
            CanonicalModelId.CLAUDE_OPUS_5_5_LOW,
            ReasoningTier.LOW,
            AntigravityModelPlaceholder.M400,
            _CRAFT_ONLY,
        ),
        (
            CanonicalModelId.GEMINI_4_ARGON_HIGH,
            ReasoningTier.HIGH,
            AntigravityModelPlaceholder.M417,
            _MACRO_COUNCIL,
        ),
        (
            CanonicalModelId.GEMINI_4_ARGON_MEDIUM,
            ReasoningTier.MEDIUM,
            AntigravityModelPlaceholder.M418,
            _MACRO_COUNCIL,
        ),
        (
            CanonicalModelId.GEMINI_4_ARGON_LOW,
            ReasoningTier.LOW,
            AntigravityModelPlaceholder.M419,
            _MACRO_ONLY,
        ),
        (
            CanonicalModelId.GEMINI_3_8_FLASH_HIGH,
            ReasoningTier.HIGH,
            AntigravityModelPlaceholder.M318,
            _FAST_CAPS,
        ),
        (
            CanonicalModelId.GEMINI_3_8_FLASH_MEDIUM,
            ReasoningTier.MEDIUM,
            AntigravityModelPlaceholder.M319,
            _FAST_CAPS,
        ),
        (
            CanonicalModelId.GEMINI_3_8_FLASH_LOW,
            ReasoningTier.LOW,
            AntigravityModelPlaceholder.M320,
            _FAST_CAPS,
        ),
    )
}

CODEX_MODEL_SPECS: dict[str, ModelSpec] = {
    str(model_id): ModelSpec(
        provider=str(ProviderKind.OPENAI_CODEX),
        engine=str(EngineKind.CODEX_CLI),
        model_id=str(model_id),
        auth_context=str(AuthContextKind.CODEX_CHATGPT_OAUTH),
        reasoning_tier=str(tier),
        placeholder=None,
        capabilities=_CODEX_CAPS,
    )
    for model_id, tier in (
        (CanonicalModelId.GPT_6_TERRA, ReasoningTier.XHIGH),
        (CanonicalModelId.GPT_6_LUNA, ReasoningTier.XHIGH),
        (CanonicalModelId.GPT_6_SOL, ReasoningTier.XHIGH),
        (CanonicalModelId.GPT_6_ASTRA, ReasoningTier.XHIGH),
        (CanonicalModelId.GPT_6_1_TERRA, ReasoningTier.XHIGH),
        (CanonicalModelId.GPT_6_1_SOL, ReasoningTier.XHIGH),
        (CanonicalModelId.CODEX_DEFAULT, ReasoningTier.HIGH),
    )
}

EXPLICIT_MODEL_ALIASES: dict[str, str] = {
    **{str(alias): str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM) for alias in SonnetModelAlias},
    str(TieredModelAlias.SONNET_HIGH): str(CanonicalModelId.CLAUDE_SONNET_5_5_HIGH),
    str(TieredModelAlias.SONNET_LOW): str(CanonicalModelId.CLAUDE_SONNET_5_5_LOW),
    **{str(alias): str(CanonicalModelId.CLAUDE_OPUS_5_5_MEDIUM) for alias in OpusModelAlias},
    str(TieredModelAlias.OPUS_HIGH): str(CanonicalModelId.CLAUDE_OPUS_5_5_HIGH),
    str(TieredModelAlias.OPUS_LOW): str(CanonicalModelId.CLAUDE_OPUS_5_5_LOW),
    **{str(alias): str(CanonicalModelId.GEMINI_4_ARGON_HIGH) for alias in ArgonModelAlias},
    str(TieredModelAlias.ARGON_HIGH): str(CanonicalModelId.GEMINI_4_ARGON_HIGH),
    str(TieredModelAlias.ARGON_MEDIUM): str(CanonicalModelId.GEMINI_4_ARGON_MEDIUM),
    str(TieredModelAlias.ARGON_LOW): str(CanonicalModelId.GEMINI_4_ARGON_LOW),
    **{str(alias): str(CanonicalModelId.GEMINI_3_8_FLASH_HIGH) for alias in FlashModelAlias},
}

CODEX_MODEL_ALIASES: dict[str, str] = {
    **{str(alias): str(CanonicalModelId.GPT_6_TERRA) for alias in Codex6TerraAlias},
    **{str(alias): str(CanonicalModelId.GPT_6_LUNA) for alias in Codex6LunaAlias},
    **{str(alias): str(CanonicalModelId.GPT_6_SOL) for alias in Codex6SolAlias},
    **{str(alias): str(CanonicalModelId.GPT_6_ASTRA) for alias in Codex6AstraAlias},
    **{str(alias): str(CanonicalModelId.GPT_6_1_TERRA) for alias in Codex61TerraAlias},
    **{str(alias): str(CanonicalModelId.GPT_6_1_SOL) for alias in Codex61SolAlias},
}

CODEX_SUPPORTED_MODELS: set[str] = set(CODEX_MODEL_SPECS.keys())
VALID_REASONING_TIERS: set[str] = {tier.value for tier in ReasoningTier}
