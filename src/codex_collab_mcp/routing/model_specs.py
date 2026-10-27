"""Model specifications, explicit aliases, and default routes for Antigravity and Codex."""

from __future__ import annotations

from dataclasses import dataclass

from codex_collab_mcp.collab_enums import (
    AuthContextKind,
    CanonicalModelId,
    EngineKind,
    ProviderKind,
    ReasoningTier,
)
from codex_collab_mcp.routing.contract_dicts import ModelSpecDict


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


_CRAFT_COUNCIL = ("code_craft", "advisory_review", "triad_council")
_CRAFT_ONLY = ("code_craft", "advisory_review")
_MACRO_COUNCIL = ("macro_architecture", "advisory_review", "triad_council")
_MACRO_ONLY = ("macro_architecture", "advisory_review")
_FAST_CAPS = ("fast_review",)
_CODEX_CAPS = ("adversarial_audit", "targeted_synthesis", "rescue", "triad_council")
_M = CanonicalModelId
_T = ReasoningTier

ANTIGRAVITY_MODEL_SPECS: dict[str, ModelSpec] = {
    str(m): ModelSpec(
        ProviderKind.ANTIGRAVITY,
        EngineKind.ANTIGRAVITY_LOCAL_RPC,
        str(m),
        AuthContextKind.ANTIGRAVITY_LOCAL_CSRF,
        str(t),
        p,
        c,
    )
    for m, t, p, c in (
        (_M.CLAUDE_SONNET_5_5_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M404", _CRAFT_COUNCIL),
        (_M.CLAUDE_SONNET_5_5_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M405", _CRAFT_COUNCIL),
        (_M.CLAUDE_SONNET_5_5_LOW, _T.LOW, "MODEL_PLACEHOLDER_M403", _CRAFT_ONLY),
        (_M.CLAUDE_OPUS_5_5_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M401", _CRAFT_COUNCIL),
        (_M.CLAUDE_OPUS_5_5_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M402", _CRAFT_COUNCIL),
        (_M.CLAUDE_OPUS_5_5_LOW, _T.LOW, "MODEL_PLACEHOLDER_M400", _CRAFT_ONLY),
        (_M.GEMINI_4_ARGON_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M417", _MACRO_COUNCIL),
        (_M.GEMINI_4_ARGON_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M418", _MACRO_COUNCIL),
        (_M.GEMINI_4_ARGON_LOW, _T.LOW, "MODEL_PLACEHOLDER_M419", _MACRO_ONLY),
        (_M.GEMINI_3_8_FLASH_HIGH, _T.HIGH, "MODEL_PLACEHOLDER_M318", _FAST_CAPS),
        (_M.GEMINI_3_8_FLASH_MEDIUM, _T.MEDIUM, "MODEL_PLACEHOLDER_M319", _FAST_CAPS),
        (_M.GEMINI_3_8_FLASH_LOW, _T.LOW, "MODEL_PLACEHOLDER_M320", _FAST_CAPS),
    )
}

CODEX_MODEL_SPECS: dict[str, ModelSpec] = {
    str(m): ModelSpec(
        ProviderKind.OPENAI_CODEX,
        EngineKind.CODEX_CLI,
        str(m),
        AuthContextKind.CODEX_CHATGPT_OAUTH,
        str(t),
        None,
        _CODEX_CAPS,
    )
    for m, t in (
        (_M.GPT_6_1_SOL, _T.XHIGH),
        (_M.GPT_5_4, _T.HIGH),
        (_M.GPT_5_CODEX, _T.HIGH),
        (_M.GPT_5_1, _T.HIGH),
        (_M.GPT_5, _T.HIGH),
        (_M.GPT_5_NANO, _T.LOW),
        (_M.O3, _T.HIGH),
        (_M.O4_MINI, _T.MEDIUM),
        (_M.CODEX_DEFAULT, _T.HIGH),
    )
}

_SONNET_ALIASES = (
    "sonnet",
    "sonnet-medium",
    "sonnet-5.5",
    "sonnet-5-5",
    "sonnet5.5",
    "claude",
    "claude-sonnet",
    "claude-sonnet-5.5",
    "claude-sonnet-5.5-medium",
)
_OPUS_ALIASES = (
    "opus",
    "opus-medium",
    "opus-5.5",
    "opus-5-5",
    "opus5.5",
    "claude-opus",
    "claude-opus-5.5",
    "claude-opus-5.5-medium",
)
EXPLICIT_MODEL_ALIASES: dict[str, str] = dict(
    (
        *((k, str(_M.CLAUDE_SONNET_5_5_MEDIUM)) for k in _SONNET_ALIASES),
        ("sonnet-high", str(_M.CLAUDE_SONNET_5_5_HIGH)),
        ("sonnet-low", str(_M.CLAUDE_SONNET_5_5_LOW)),
        *((k, str(_M.CLAUDE_OPUS_5_5_MEDIUM)) for k in _OPUS_ALIASES),
        ("opus-high", str(_M.CLAUDE_OPUS_5_5_HIGH)),
        ("opus-low", str(_M.CLAUDE_OPUS_5_5_LOW)),
        ("argon", str(_M.GEMINI_4_ARGON_HIGH)),
        ("gemini-4-argon", str(_M.GEMINI_4_ARGON_HIGH)),
        ("argon-high", str(_M.GEMINI_4_ARGON_HIGH)),
        ("argon-medium", str(_M.GEMINI_4_ARGON_MEDIUM)),
        ("argon-low", str(_M.GEMINI_4_ARGON_LOW)),
    )
)

CODEX_SUPPORTED_MODELS: set[str] = set(CODEX_MODEL_SPECS.keys())
VALID_REASONING_TIERS: set[str] = {tier.value for tier in ReasoningTier}
