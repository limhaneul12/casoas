"""Per-agent reasoning effort, model assignment, and visual/wave effect controller for DAG-ULW++."""

from __future__ import annotations

import sys
import time

from app.core.collab_enums import CanonicalModelId, ReasoningTier
from app.core.dag_enums import (
    DagAgentPoleSelector,
    DagEffectListToken,
    DagNodeKind,
    DagNodeRoleKind,
    DagVisualEffectStyle,
    TuiAnsiStyle,
)
from app.core.skill_registry import DisplayWidthFormatter
from app.routing.model_specs import (
    CODEX_MODEL_ALIASES,
    EXPLICIT_MODEL_ALIASES,
    VALID_REASONING_TIERS,
)
from app.schemas.dag_input_schemas import DagNodeInputModel
from app.schemas.inner_dto.dag_dtos import (
    DagAgentEffectConfig,
    DagAgentPoleProfile,
    DagNodeSettlementReceipt,
)


def format_effort_ring(effort: str) -> str:
    """Format a reasoning effort tier string with its `oh-my-hermes` Effort Ring symbol.

    Args:
        effort: Reasoning effort tier (`minimal`, `low`, `medium`, `high`, `xhigh`).

    Returns:
        Formatted string such as `'● xhigh'`, `'◕ high'`, `'◑ medium'`, `'◔ low'`, or `'○ minimal'`.
    """
    clean = effort.strip().lower()
    if clean == ReasoningTier.MINIMAL:
        return "○ minimal"
    if clean == ReasoningTier.LOW:
        return "◔ low"
    if clean == ReasoningTier.MEDIUM:
        return "◑ medium"
    if clean == ReasoningTier.HIGH:
        return "◕ high"
    if clean == ReasoningTier.XHIGH:
        return "● xhigh"
    return f"◑ {clean or 'medium'}"


def _style_palette(style: DagVisualEffectStyle) -> tuple[tuple[str, str], tuple[str, str], tuple[str, str], str]:
    """Return `((argon_icon, argon_col), (claude_icon, claude_col), (codex_icon, codex_col), conn_col)`.

    Args:
        style: Selected `DagVisualEffectStyle` enum value.

    Returns:
        Tuple of pole `(badge_icon, ansi_color)` pairs and connector ANSI color.
    """
    if style == DagVisualEffectStyle.CYBER_MATRIX:
        return (("◈", str(TuiAnsiStyle.EMERALD)), ("◆", str(TuiAnsiStyle.TEAL)), ("❖", f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.EMERALD}"), f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.TEAL}")
    if style == DagVisualEffectStyle.ROYAL_PRISM:
        return (("✧", str(TuiAnsiStyle.VIOLET)), ("❖", str(TuiAnsiStyle.PINK)), ("★", str(TuiAnsiStyle.AMBER)), f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.PINK}")
    if style == DagVisualEffectStyle.MINIMAL_MONO:
        return (("●", str(TuiAnsiStyle.BOLD)), ("◆", str(TuiAnsiStyle.DIM)), ("▲", str(TuiAnsiStyle.BOLD)), str(TuiAnsiStyle.DIM))
    return (("✦", str(TuiAnsiStyle.CYAN)), ("◆", str(TuiAnsiStyle.EMERALD)), ("★", str(TuiAnsiStyle.PINK)), f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.VIOLET}")


def _resolve_pole_token(token: str) -> tuple[str | None, str | None]:
    """Resolve a user token (`effort` or `model` alias) into `(model_id_or_none, effort_or_none)`.

    Args:
        token: Lowercase argument token from `/effect` or `/effort`.

    Returns:
        Tuple of `(resolved_model_id, resolved_effort)`.
    """
    tok = token.strip().lower().replace("gpt-6-1-", "gpt-6.1-").replace("codex-6-1-", "gpt-6.1-").replace("codex-6.1-", "gpt-6.1-").replace("codex-6-", "gpt-6-")
    if tok in VALID_REASONING_TIERS:
        return None, tok
    if tok in CODEX_MODEL_ALIASES:
        return CODEX_MODEL_ALIASES[tok], None
    if tok in EXPLICIT_MODEL_ALIASES:
        return EXPLICIT_MODEL_ALIASES[tok], None
    return None, None


def build_triad_nodes(
    campaign_goal: str, argon_model: str | None = None, argon_effort: str | None = None,
    claude_model: str | None = None, claude_effort: str | None = None,
    codex_model: str | None = None, codex_effort: str | None = None,
) -> tuple[DagNodeInputModel, ...]:
    """Synthesize the canonical 4-node Tri-Polar DAG with optional per-pole model and effort bindings.

    Args:
        campaign_goal: High-level engineering objective to decompose.
        argon_model: Optional preferred model ID for Wave 0 and Wave 2 Argon nodes.
        argon_effort: Optional reasoning effort tier for Argon nodes.
        claude_model: Optional preferred model ID for Wave 1 Claude craft node.
        claude_effort: Optional reasoning effort tier for Claude craft node.
        codex_model: Optional preferred model ID for Wave 1 Codex audit node.
        codex_effort: Optional reasoning effort tier for Codex audit node.

    Returns:
        Tuple of 4 `DagNodeInputModel` specifications spanning Argon, Claude, and Codex.
    """
    n1 = DagNodeInputModel(node_id="n1_topology_spec", role=DagNodeRoleKind.TOPOLOGY_PLANNER, task=f"Distill architectural invariants and interface contracts for: {campaign_goal}", deliverable="Compact type & contract specification with zero ambiguity", preferred_model=argon_model, reasoning_effort=argon_effort)
    n2 = DagNodeInputModel(node_id="n2_interface_craft", role=DagNodeRoleKind.INTERFACE_CONTRACTOR, task=f"Craft strict boundary schemas and core implementation for: {campaign_goal}", deliverable="Clean, type-annotated module implementation adhering to 3-tier schema rules", depends_on=("n1_topology_spec",), preferred_model=claude_model, reasoning_effort=claude_effort)
    n3 = DagNodeInputModel(node_id="n3_red_team_audit", role=DagNodeRoleKind.ADVERSARIAL_AUDITOR, task=f"Audit implementation and concurrency/security boundaries for: {campaign_goal}", deliverable="Adversarial red-team verdict (PASS/WARN/BLOCK) with concrete findings", depends_on=("n1_topology_spec",), preferred_model=codex_model, reasoning_effort=codex_effort)
    n4 = DagNodeInputModel(node_id="n4_verify_fan_in", role=DagNodeRoleKind.VERIFICATION_FAN_IN, node_kind=DagNodeKind.VERIFICATION, task=f"Adjudicate Claude craft + Codex audit and settle campaign for: {campaign_goal}", deliverable="Final 3-pole consensus adjudication and verification gate confirmation", depends_on=("n2_interface_craft", "n3_red_team_audit"), preferred_model=argon_model, reasoning_effort=argon_effort)
    return (n1, n2, n3, n4)


class DagAgentEffectRegistry:
    """Manage per-pole DAG reasoning effort, model routing, and visual/animation effects."""

    def __init__(self, style: DagVisualEffectStyle = DagVisualEffectStyle.NEON_PULSE) -> None:
        """Initialize the DAG agent effect registry with default 3-pole profiles.

        Args:
            style: Initial visual effect style (`neon-pulse` by default).
        """
        self.config: DagAgentEffectConfig = self._build_config(style=style, argon_model=str(CanonicalModelId.GEMINI_4_ARGON_HIGH), argon_effort=str(ReasoningTier.HIGH), claude_model=str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM), claude_effort=str(ReasoningTier.MEDIUM), codex_model=str(CanonicalModelId.GPT_6_ASTRA), codex_effort=str(ReasoningTier.XHIGH))

    def _build_config(self, style: DagVisualEffectStyle, argon_model: str, argon_effort: str, claude_model: str, claude_effort: str, codex_model: str, codex_effort: str) -> DagAgentEffectConfig:
        """Construct a `DagAgentEffectConfig` with style-matched icons and ANSI colors.

        Args:
            style: Visual effect theme.
            argon_model: Model ID for Wave 0 and Wave 2 Argon nodes.
            argon_effort: Reasoning effort for Argon nodes.
            claude_model: Model ID for Wave 1 Claude craft node.
            claude_effort: Reasoning effort for Claude craft node.
            codex_model: Model ID for Wave 1 Codex audit node.
            codex_effort: Reasoning effort for Codex audit node.

        Returns:
            Frozen `DagAgentEffectConfig` instance.
        """
        (a_ic, a_col), (c_ic, c_col), (x_ic, x_col), _ = _style_palette(style=style)
        return DagAgentEffectConfig(
            style=style,
            argon=DagAgentPoleProfile(pole="argon", model_id=argon_model, reasoning_effort=argon_effort, badge_icon=a_ic, ansi_color=a_col),
            claude=DagAgentPoleProfile(pole="claude", model_id=claude_model, reasoning_effort=claude_effort, badge_icon=c_ic, ansi_color=c_col),
            codex=DagAgentPoleProfile(pole="codex", model_id=codex_model, reasoning_effort=codex_effort, badge_icon=x_ic, ansi_color=x_col),
        )

    def build_configured_triad_nodes(self, campaign_goal: str) -> tuple[DagNodeInputModel, ...]:
        """Synthesize the 4-node Tri-Polar DAG stamped with active agent models and efforts.

        Args:
            campaign_goal: High-level engineering objective to decompose.

        Returns:
            Tuple of 4 `DagNodeInputModel` instances configured with `self.config`.
        """
        cfg = self.config
        return build_triad_nodes(campaign_goal=campaign_goal, argon_model=cfg.argon.model_id, argon_effort=cfg.argon.reasoning_effort, claude_model=cfg.claude.model_id, claude_effort=cfg.claude.reasoning_effort, codex_model=cfg.codex.model_id, codex_effort=cfg.codex.reasoning_effort)

    def apply_effect_command(self, arg: str, use_ansi: bool = True) -> tuple[str, bool]:
        """Parse and apply an `/effect` or `/effort` TUI slash command.

        Args:
            arg: Command argument string after `/effect` or `/effort`.
            use_ansi: Whether ANSI color codes are enabled.

        Returns:
            Tuple of `(rendered_response_panel, should_play_wave_preview)`.
        """
        clean = arg.strip()
        if not clean or clean.lower() in tuple(str(t) for t in DagEffectListToken):
            return self.render_effects_matrix(use_ansi=use_ansi), False

        norm_str = clean.lower()
        for alias_s, canonical_s in sorted(CODEX_MODEL_ALIASES.items(), key=lambda kv: -len(kv[0])):
            if " " in alias_s or "(" in alias_s or not alias_s.isascii():
                norm_str = norm_str.replace(alias_s, canonical_s)
        tokens = [t.strip() for t in norm_str.split() if t.strip()]
        first = tokens[0]

        if first in ("reset", "default"):
            self.config = self._build_config(
                style=DagVisualEffectStyle.NEON_PULSE,
                argon_model=str(CanonicalModelId.GEMINI_4_ARGON_HIGH),
                argon_effort=str(ReasoningTier.HIGH),
                claude_model=str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM),
                claude_effort=str(ReasoningTier.MEDIUM),
                codex_model=str(CanonicalModelId.GPT_6_ASTRA),
                codex_effort=str(ReasoningTier.XHIGH),
            )
            hdr = "● [EFFECT RESET] DAG 에이전트 모델·Effort·시각 효과가 기본값(`neon-pulse`)으로 복원되었습니다."
            return f"{hdr}\n{self.render_effects_matrix(use_ansi=use_ansi)}", True

        style_candidate = tokens[1] if (first == "style" and len(tokens) > 1) else first
        valid_styles = tuple(str(s) for s in DagVisualEffectStyle)
        if style_candidate in valid_styles:
            new_style = DagVisualEffectStyle(style_candidate)
            c = self.config
            self.config = self._build_config(
                style=new_style,
                argon_model=c.argon.model_id,
                argon_effort=c.argon.reasoning_effort,
                claude_model=c.claude.model_id,
                claude_effort=c.claude.reasoning_effort,
                codex_model=c.codex.model_id,
                codex_effort=c.codex.reasoning_effort,
            )
            hdr = f"✦ [EFFECT STYLE] DAG 시각 효과 테마가 `{new_style}`(으)로 변경되었습니다."
            return f"{hdr}\n{self.render_effects_matrix(use_ansi=use_ansi)}", True

        auto_mod, _ = _resolve_pole_token(token=first)
        if (
            first
            in (
                DagAgentPoleSelector.ARGON,
                DagAgentPoleSelector.CLAUDE,
                DagAgentPoleSelector.SONNET,
                DagAgentPoleSelector.OPUS,
                DagAgentPoleSelector.CODEX,
                DagAgentPoleSelector.ALL,
            )
            or auto_mod is not None
        ):
            c = self.config
            a_mod, a_eff = c.argon.model_id, c.argon.reasoning_effort
            c_mod, c_eff = c.claude.model_id, c.claude.reasoning_effort
            x_mod, x_eff = c.codex.model_id, c.codex.reasoning_effort
            if first in (
                DagAgentPoleSelector.ARGON,
                DagAgentPoleSelector.CLAUDE,
                DagAgentPoleSelector.SONNET,
                DagAgentPoleSelector.OPUS,
                DagAgentPoleSelector.CODEX,
                DagAgentPoleSelector.ALL,
            ):
                if first == DagAgentPoleSelector.SONNET:
                    c_mod = str(CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM)
                elif first == DagAgentPoleSelector.OPUS:
                    c_mod = str(CanonicalModelId.CLAUDE_OPUS_5_5_MEDIUM)
                target_pole = "claude" if first in ("claude", "sonnet", "opus") else first
                rest = tokens[1:] or ([first] if first in ("sonnet", "opus") else [])
            else:
                target_pole = "argon" if "gemini" in (auto_mod or "") else ("claude" if "claude" in (auto_mod or "") else "codex")
                rest = tokens
            if not rest:
                return (
                    f"▲ 변경할 모델 또는 Effort(`minimal|low|medium|high|xhigh`)를 지정해주세요. 예: `/effect {first} 6 astra xhigh` 또는 `/effect {first} 6-1 sol xhigh`",
                    False,
                )
            changed = False
            for tok in rest:
                mod, eff = _resolve_pole_token(token=tok)
                if (mod is None and eff is None) or (target_pole == "all" and eff is None):
                    return (
                        f"▲ 지원하지 않는 에이전트 옵션입니다: `{tok}` (`/effect` 로 설정 목록 확인)",
                        False,
                    )
                if target_pole == "all" and eff is not None:
                    a_eff, c_eff, x_eff, changed = eff, eff, eff, True
                elif target_pole == "argon":
                    a_mod, a_eff, changed = (mod or a_mod), (eff or a_eff), True
                elif target_pole == "claude":
                    c_mod, c_eff, changed = (mod or c_mod), (eff or c_eff), True
                elif target_pole == "codex":
                    x_mod, x_eff, changed = (mod or x_mod), (eff or x_eff), True
            if not changed:
                return (
                    f"▲ 지원하지 않는 에이전트 옵션입니다: `{' '.join(rest)}` (`/effect` 로 설정 목록 확인)",
                    False,
                )
            self.config = self._build_config(
                style=c.style,
                argon_model=a_mod,
                argon_effort=a_eff,
                claude_model=c_mod,
                claude_effort=c_eff,
                codex_model=x_mod,
                codex_effort=x_eff,
            )
            hdr = f"● [AGENT EFFECT UPDATED] `{target_pole}` 에이전트 모델/Effort 설정이 DAG 캠페인에 반영되었습니다."
            return f"{hdr}\n{self.render_effects_matrix(use_ansi=use_ansi)}", True

        return (f"▲ 알 수 없는 `/effect` 인자입니다: `{clean}`\n사용법: `/effect codex <6 terra|6 luna|6 sol|6 astra|6-1 terra|6-1 sol> [minimal|low|medium|high|xhigh]` · `/effect claude <opus|sonnet> [medium|high]`"), False

    def style_dag_graph_line(self, line: str, use_ansi: bool) -> str:
        """Apply per-agent pole colors and visual effect styling to a single DAG graph row.

        Args:
            line: Raw single line from `plan.ascii_graph`.
            use_ansi: Whether ANSI color output is enabled.

        Returns:
            ANSI-styled DAG graph line with independent left/right lane colors.
        """
        if not use_ansi or not line.strip():
            return line
        cfg = self.config
        _, _, _, conn_col = _style_palette(style=cfg.style)
        rst = str(TuiAnsiStyle.RESET)
        # Parallel Wave 1 boxes sit side-by-side separated by "   " (3 spaces)
        for sep in ("╮   ╭", "│   │", "╯   ╰"):
            if sep in line:
                left_part, right_part = line.split(sep, maxsplit=1)
                left_box = f"{left_part}{sep[0]}"
                right_box = f"{sep[-1]}{right_part}"
                l_col = f"{TuiAnsiStyle.BOLD}{cfg.claude.ansi_color}" if "RUNNING" in left_box else (str(TuiAnsiStyle.DIM) if "◌ QUEUED" in left_box else cfg.claude.ansi_color)
                r_col = f"{TuiAnsiStyle.BOLD}{cfg.codex.ansi_color}" if "RUNNING" in right_box else (str(TuiAnsiStyle.DIM) if "◌ QUEUED" in right_box else cfg.codex.ansi_color)
                return f"{l_col}{left_box}{rst}   {r_col}{right_box}{rst}"
        if "┌─" in line or "└─" in line or (line.strip() == "│"):
            return f"{conn_col}{line}{rst}"
        a_col = f"{TuiAnsiStyle.BOLD}{cfg.argon.ansi_color}" if "RUNNING" in line else (str(TuiAnsiStyle.DIM) if "◌ QUEUED" in line else cfg.argon.ansi_color)
        return f"{a_col}{line}{rst}"

    def play_wave_execution_effect(
        self,
        campaign_goal: str,
        term_cols: int,
        bar_row: int,
        stage_views: tuple[str, ...] = (),
    ) -> None:
        """Play a live 3-Wave Agent Prompting & Execution stream on the viewport and bottom bar.

        Args:
            campaign_goal: Active campaign goal or effect preview label.
            term_cols: Terminal width in columns.
            bar_row: Top row index (1-based) of the 3-row bottom prompt bar.
            stage_views: Optional pre-rendered full DAG workbench views for Wave 0, 1, and 2.
        """
        cfg = self.config
        rst, bg = str(TuiAnsiStyle.RESET), str(TuiAnsiStyle.BG_BAR)
        goal_clip = DisplayWidthFormatter.fit_columns(text=campaign_goal, width=32, pad=False)
        a_ring = format_effort_ring(effort=cfg.argon.reasoning_effort)
        c_ring = format_effort_ring(effort=cfg.claude.reasoning_effort)
        x_ring = format_effort_ring(effort=cfg.codex.reasoning_effort)
        frames = (
            (
                cfg.argon.ansi_color,
                f" {cfg.argon.badge_icon} [Wave 0/2 ⟳ RUNNING] {cfg.argon.model_id}@{a_ring} ➔ Distilling architectural invariants ({goal_clip})",
                " ▰▱▱ Wave 0 Argon Running │ Wave 1 Parallel (Claude ∥ Codex Queued) │ Wave 2 Fan-In (Queued)",
                (
                    f"╭── ⟳ LIVE AGENT PROMPTING & EXECUTION [Wave 0/2 · {cfg.argon.model_id} @ {a_ring}] ────────────╮",
                    "│ ✦ Wave 0 [n1_topology_spec] ── ⟳ RUNNING: Prompting Argon Architect...                │",
                    f"│   ↳ Prompt [$codex-heterarchy]: Distill architectural invariants & contracts ({goal_clip[:14]}) │",
                    f"│ ◆ Wave 1·L1 [n2_interface_craft] ({cfg.claude.model_id} @ {c_ring}) ── ◌ QUEUED (deps: n1) │",
                    f"│ ★ Wave 1·L2 [n3_red_team_audit]  ({cfg.codex.model_id} @ {x_ring}) ── ◌ QUEUED (deps: n1) │",
                    f"│ ✦ Wave 2 [n4_verify_fan_in]      ({cfg.argon.model_id} @ {a_ring}) ── ◌ QUEUED (deps: n2,n3)│",
                    "╰───────────────────────────────────────────────────────────────────────────────────────╯",
                ),
            ),
            (
                cfg.claude.ansi_color,
                f" {cfg.claude.badge_icon} [Wave 1/2 ⟳ PARALLEL FAN-OUT] {cfg.claude.model_id}@{c_ring} ∥ {cfg.codex.badge_icon} {cfg.codex.model_id}@{x_ring}",
                " ▰▰▱ Wave 0 ● Settled │ Wave 1 Parallel Fan-Out Running │ Wave 2 Fan-In (Queued)",
                (
                    f"╭── ⟳ LIVE PARALLEL PROMPTING [Wave 1/2 · {cfg.claude.model_id} ∥ {cfg.codex.model_id}] ───────╮",
                    "│ ✦ Wave 0 [n1_topology_spec] ── ● SETTLED: Invariants & 3-tier contracts distilled     │",
                    f"│ ◆ Wave 1·L1 [n2_interface_craft] ({cfg.claude.model_id} @ {c_ring}) ── ⟳ RUNNING          │",
                    "│   ↳ Prompt [deps:n1]: Craft strict 3-tier boundary schemas & inner_dto implementation │",
                    f"│ ★ Wave 1·L2 [n3_red_team_audit]  ({cfg.codex.model_id} @ {x_ring}) ── ⟳ RUNNING          │",
                    "│   ↳ Prompt [deps:n1]: Audit implementation & concurrency/WAW/RAW security boundaries  │",
                    "╰───────────────────────────────────────────────────────────────────────────────────────╯",
                ),
            ),
            (
                cfg.codex.ansi_color,
                f" {cfg.argon.badge_icon} [Wave 2/2 ⟳ FAN-IN ADJUDICATION] {cfg.argon.model_id}@{a_ring} [Upstream: n2, n3 ● SETTLED] ➔ SHA-256",
                f" ▰▰▰ Wave 2 Fan-In Running ➔ All 4 Nodes Settling · Style: {cfg.style} · Full Prompts: /prompts",
                (
                    f"╭── ⟳ LIVE FAN-IN ADJUDICATION [Wave 2/2 · {cfg.argon.model_id} @ {a_ring}] ───────────────────╮",
                    "│ ◆ Wave 1·L1 [n2_interface_craft] ── ● SETTLED: Strict boundary & inner_dto crafted    │",
                    "│ ★ Wave 1·L2 [n3_red_team_audit]  ── ● SETTLED: Red-team PASS (0 WAW/RAW hazards)      │",
                    f"│ ✦ Wave 2 [n4_verify_fan_in] ({cfg.argon.model_id} @ {a_ring}) ── ⟳ RUNNING ➔ ● SETTLED    │",
                    "│   ↳ Prompt [deps:n2,n3]: Adjudicate Claude craft + Codex audit & verify SHA-256 gates │",
                    "╰───────────────────────────────────────────────────────────────────────────────────────╯",
                ),
            ),
        )
        avail = max(8, bar_row - 4)
        for f_idx, (col, row1_txt, row2_txt, vp_box) in enumerate(frames):
            pad1 = DisplayWidthFormatter.fit_columns(text=row1_txt, width=max(20, term_cols - 2))
            pad2 = DisplayWidthFormatter.fit_columns(text=row2_txt, width=max(20, term_cols - 2))
            if f_idx < len(stage_views) and bar_row >= 14:
                s_lines = stage_views[f_idx].splitlines()[:avail]
                vp_str = "".join(f"\033[{4 + idx};1H\033[2K{DisplayWidthFormatter.fit_ansi_line(ansi_text=f'  {ln}', max_cols=term_cols - 2)}" for idx, ln in enumerate(s_lines))
                delay_s = 0.22
            else:
                vp_str = "".join(f"\033[{5 + idx};1H\033[2K  {col}{ln}{rst}" for idx, ln in enumerate(vp_box)) if bar_row >= 14 else ""
                delay_s = 0.06
            sys.stdout.write(f"{vp_str}\033[{bar_row};1H\033[2K{bg}{TuiAnsiStyle.BOLD}{col}{pad1}  {rst}\033[{bar_row + 1};1H\033[2K{col}{pad2}{rst}")
            sys.stdout.flush()
            time.sleep(delay_s)

    def render_effects_matrix(self, use_ansi: bool = True) -> str:
        """Render the DAG Agent Effect & Effort Configuration Matrix for `/effect` or `/effort`.

        Args:
            use_ansi: Whether to emit ANSI color sequences.

        Returns:
            Formatted multi-line configuration table and command guide.
        """
        cfg = self.config
        rst = str(TuiAnsiStyle.RESET) if use_ansi else ""
        hdr_c = f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.VIOLET}" if use_ansi else ""
        dim_c = str(TuiAnsiStyle.DIM) if use_ansi else ""
        a_ring = format_effort_ring(effort=cfg.argon.reasoning_effort).ljust(10)
        c_ring = format_effort_ring(effort=cfg.claude.reasoning_effort).ljust(10)
        x_ring = format_effort_ring(effort=cfg.codex.reasoning_effort).ljust(10)
        a_col = cfg.argon.ansi_color if use_ansi else ""
        c_col = cfg.claude.ansi_color if use_ansi else ""
        x_col = cfg.codex.ansi_color if use_ansi else ""
        lines: list[str] = [
            f"{hdr_c}=== ☤ DAG-ULW++ Agent Effect & Reasoning Effort Matrix [Style: {cfg.style}] ==={rst}",
            "┌────────────────────┬───────────────────┬──────────────────────────┬────────────┬────────────────────┐",
            "│ DAG Wave & Node    │ Agent Pole        │ Configured Model         │ Effort     │ Visual Effect      │",
            "├────────────────────┼───────────────────┼──────────────────────────┼────────────┼────────────────────┤",
            f"│ Wave 0 [n1_spec]   │ {a_col}{cfg.argon.badge_icon} Argon Architect{rst} │ {cfg.argon.model_id.ljust(24)} │ {a_ring} │ {a_col}{cfg.argon.badge_icon} {str(cfg.style).ljust(16)}{rst} │",
            f"│ Wave 1 [n2_craft]  │ {c_col}{cfg.claude.badge_icon} Claude Craft   {rst} │ {cfg.claude.model_id.ljust(24)} │ {c_ring} │ {c_col}{cfg.claude.badge_icon} {str(cfg.style).ljust(16)}{rst} │",
            f"│ Wave 1 [n3_audit]  │ {x_col}{cfg.codex.badge_icon} Codex Red-Team {rst} │ {cfg.codex.model_id.ljust(24)} │ {x_ring} │ {x_col}{cfg.codex.badge_icon} {str(cfg.style).ljust(16)}{rst} │",
            f"│ Wave 2 [n4_verify] │ {a_col}{cfg.argon.badge_icon} Argon Fan-In   {rst} │ {cfg.argon.model_id.ljust(24)} │ {a_ring} │ {a_col}{cfg.argon.badge_icon} {str(cfg.style).ljust(16)}{rst} │",
            "└────────────────────┴───────────────────┴──────────────────────────┴────────────┴────────────────────┘",
            f"{dim_c}• Codex 6 / 6-1 모델·Effort : `/effect codex <6 terra|6 luna|6 sol|6 astra|6-1 terra|6-1 sol> [minimal|low|medium|high|xhigh]`{rst}",
            f"{dim_c}• Claude / Argon 모델·Effort : `/effect claude <opus|sonnet> [medium|high]` · `/effect argon <flash|argon> [low|high]`{rst}",
            f"{dim_c}• 전체 Effort 일괄 / 초기화  : `/effect all <low|medium|high|xhigh>` · 초기화: `/effect reset`{rst}",
            f"{dim_c}• DAG 시각/웨이브 테마 변경  : `/effect style <neon-pulse|cyber-matrix|royal-prism|minimal-mono>`{rst}",
        ]
        return "\n".join(lines)


def render_dag_prompts_inspector(
    effect_registry: DagAgentEffectRegistry,
    campaign_goal: str,
    active_skills: tuple[str, ...] = ("codex-heterarchy",),
    filter_arg: str = "",
    receipts: tuple[DagNodeSettlementReceipt, ...] = (),
    use_ansi: bool = True,
) -> str:
    """Render the 4-Node 5-Section Standalone Prompt & Synthesis Inspector (`/prompts` or `/trace`).

    Args:
        effect_registry: Active `DagAgentEffectRegistry` holding pole models and efforts.
        campaign_goal: Campaign goal string from the most recent or active DAG run.
        active_skills: Currently enabled `$skill` names injected into prompts.
        filter_arg: Optional filter (`n1`, `n2`, `n3`, `n4`, `argon`, `claude`, `codex`).
        receipts: Optional settled `DagNodeSettlementReceipt` tuple from the last DAG run.
        use_ansi: Whether ANSI color codes are enabled.

    Returns:
        Formatted multi-line 5-section prompt & synthesis inspection view.
    """
    cfg = effect_registry.config
    nodes = effect_registry.build_configured_triad_nodes(campaign_goal=campaign_goal)
    rcpt_by_id = {r.node_id: r for r in receipts}
    rst = str(TuiAnsiStyle.RESET) if use_ansi else ""
    hdr_c = f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.CYAN}" if use_ansi else ""
    dim_c = str(TuiAnsiStyle.DIM) if use_ansi else ""
    skills_badge = ", ".join(f"${s}" for s in active_skills) if active_skills else "(none)"
    flt = filter_arg.strip().lower()

    lines: list[str] = [
        f"{hdr_c}=== ☤ DAG-ULW++ 5-Section Agent Prompting & Synthesis Trace [Goal: {campaign_goal[:36]}] ==={rst}",
        f"{dim_c}Injected Skills: [{skills_badge}] · Effect Style: {cfg.style} · Filter: {flt or 'all (n1..n4)'}{rst}",
    ]
    wave_labels = (
        "Wave 0 (Root)",
        "Wave 1 · L1 (Parallel)",
        "Wave 1 · L2 (Parallel)",
        "Wave 2 (Fan-In)",
    )
    pole_profiles = (cfg.argon, cfg.claude, cfg.codex, cfg.argon)
    for idx, node in enumerate(nodes):
        prof = pole_profiles[idx]
        if flt and flt not in (node.node_id.lower(), prof.pole.lower(), f"n{idx + 1}", f"w{idx}"):
            continue
        col = prof.ansi_color if use_ansi else ""
        ring = format_effort_ring(effort=prof.reasoning_effort)
        up_str = ", ".join(node.depends_on) if node.depends_on else "(root node — no upstream deps)"
        rcpt = rcpt_by_id.get(node.node_id)
        out_str = f"● {rcpt.settlement_stage.upper()} ({rcpt.duration_seconds:.2f}s) ➔ {rcpt.output_summary}" if rcpt else "○ READY (run `/dag-ulw <goal>` to execute)"
        lines.extend(
            (
                f"{col}╭── {prof.badge_icon} {wave_labels[idx]} · [{node.node_id}] ➔ {prof.model_id} @ {ring} ──╮{rst}",
                f"│ 1. TASK & SKILLS : [{skills_badge}] {node.task}",
                f"│ 2. DELIVERABLE   : {node.deliverable}",
                f"│ 3. UPSTREAM/SCOPE: depends_on=[{up_str}] · verify={node.verify_cmd or 'schema + SHA-256 manifest gates'}",
                f"│ 4. AGENT OUTPUT  : {out_str}",
            )
        )
    lines.append(f"{dim_c}╰── ↩ 이전 화면(DAG 워크벤치)으로 돌아가기: `/back` (또는 `/prev`, ESC) · 특정 노드 필터: `/prompts <n1|claude|codex|n4>` ──╯{rst}")
    return "\n".join(lines)


__all__ = [
    "DagAgentEffectRegistry",
    "build_triad_nodes",
    "format_effort_ring",
    "render_dag_prompts_inspector",
]
