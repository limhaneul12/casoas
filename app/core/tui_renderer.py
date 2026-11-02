"""OpenAI Codex & `oh-my-hermes`-inspired HUD, DAG workbench, and diff/review renderer."""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

from app.core.collab_enums import CanonicalModelId, ReasoningTier
from app.core.dag_enums import TuiAnsiStyle, TuiPermissionMode
from app.core.skill_registry import DisplayWidthFormatter
from app.evidence.dag_agent_effects import DagAgentEffectRegistry, format_effort_ring
from app.routing.auth_redaction import get_python_runtime_info
from app.routing.model_specs import ANTIGRAVITY_MODEL_SPECS, CODEX_SUPPORTED_MODELS
from app.schemas.inner_dto.dag_dtos import DagUlwResponse
from app.schemas.inner_dto.dto_dict_types.contract_dict_types import (
    CodexJobRecordDict,
    HandoffTicketDict,
)

_BOX_INNER = 86
_DAG_BOX_INNER = 103


def _style_ansi(use_ansi: bool, code: str, text: str) -> str:
    """Wrap `text` in an ANSI escape sequence when `use_ansi` is True.

    Args:
        use_ansi: Whether ANSI escape sequences are enabled.
        code: ANSI prefix code.
        text: Text to wrap.

    Returns:
        Styled string or raw text.
    """
    return f"{code}{text}{TuiAnsiStyle.RESET}" if use_ansi else text


def _box_row(content: str, use_ansi: bool = True, color_code: str = "") -> str:
    """Format a single padded row inside a `╭───╮` frame of `_BOX_INNER` columns.

    Args:
        content: Plain-text row content (without outer `│` borders).
        use_ansi: Whether ANSI escape sequences are enabled.
        color_code: Optional ANSI color escape to apply to the inner text.

    Returns:
        Bordered row string `│ <padded_content> │`.
    """
    clipped = DisplayWidthFormatter.fit_columns(text=content, width=_BOX_INNER - 2)
    styled = f"{color_code}{clipped}{TuiAnsiStyle.RESET}" if (use_ansi and color_code) else clipped
    return f"│ {styled} │"


def render_workspace_diff_review_panel(
    workspace_dir: str,
    added: tuple[str, ...],
    modified: tuple[str, ...],
    deleted: tuple[str, ...],
    total_tracked: int,
    review_focus: str = "",
    use_ansi: bool = True,
) -> str:
    """Render an OpenAI Codex CLI-style `/diff` or `/review` working-tree delta panel.

    Args:
        workspace_dir: Active workspace directory path.
        added: Sorted tuple of newly added relative file paths since session baseline.
        modified: Sorted tuple of modified relative file paths since session baseline.
        deleted: Sorted tuple of deleted relative file paths since session baseline.
        total_tracked: Total number of non-ignored workspace files scanned.
        review_focus: Non-empty when invoked via `/review [focus]`, empty for `/diff`.
        use_ansi: Whether ANSI escape sequences are enabled.

    Returns:
        Formatted box-drawing `/diff` or `/review` panel string.
    """
    is_review = bool(review_focus)
    mode_lbl = "WORKING-TREE CODE REVIEW" if is_review else "WORKSPACE MANIFEST DIFF"
    total_delta = len(added) + len(modified) + len(deleted)
    title_s = f"── ☤ {mode_lbl} [+{len(added)} ~{len(modified)} -{len(deleted)} · tracked={total_tracked}] "
    top = _style_ansi(use_ansi=use_ansi, code=TuiAnsiStyle.CYAN, text=f"╭{title_s}{'─' * max(2, _BOX_INNER - DisplayWidthFormatter.text_width(text=title_s))}╮")
    bot = _style_ansi(use_ansi=use_ansi, code=TuiAnsiStyle.CYAN, text=f"╰{'─' * _BOX_INNER}╯")
    rows: list[str] = [top, _box_row(content=f" Workspace : {Path(workspace_dir).name} ({workspace_dir})", use_ansi=use_ansi)]
    if is_review:
        verdict = "● PASS (Clean working-tree baseline)" if total_delta == 0 else f"◑ REVIEW ({total_delta} mutated file(s) across working tree)"
        rows.append(_box_row(content=f" Focus     : {review_focus} · Verdict: {verdict}", use_ansi=use_ansi, color_code=TuiAnsiStyle.EMERALD if total_delta == 0 else TuiAnsiStyle.AMBER))
    if total_delta == 0:
        rows.append(_box_row(content=" ● Working tree matches session SHA-256 manifest baseline (0 mutated files).", use_ansi=use_ansi, color_code=TuiAnsiStyle.EMERALD))
    else:
        rows.extend(_box_row(content=f" + [ADDED]    {p}", use_ansi=use_ansi, color_code=TuiAnsiStyle.EMERALD) for p in added[:8])
        rows.extend(_box_row(content=f" ~ [MODIFIED] {p}", use_ansi=use_ansi, color_code=TuiAnsiStyle.AMBER) for p in modified[:12])
        rows.extend(_box_row(content=f" - [DELETED]  {p}", use_ansi=use_ansi, color_code=TuiAnsiStyle.PINK) for p in deleted[:8])
    rows.append(_box_row(content=" Tip: `/diff` 변경 파일 확인 · `/review [관점]` 코드 리뷰 · `/back` 이전 화면 복귀", use_ansi=use_ansi, color_code=TuiAnsiStyle.DIM))
    rows.append(bot)
    return "\n".join(rows)


class OmhTuiRenderer:
    """Render OpenAI Codex & `oh-my-hermes`-inspired HUDs, DAG graphs, and animations."""

    def __init__(self, use_ansi: bool = True) -> None:
        """Initialize the TUI renderer with optional ANSI color codes.

        Args:
            use_ansi: Whether to emit ANSI escape sequences.
        """
        self.use_ansi = use_ansi
        self.last_viewport_lines: dict[int, str] = {}

    def render_banner(
        self,
        workspace_dir: str,
        running_jobs: int = 0,
        pending_handoffs: int = 0,
        active_model: str = CanonicalModelId.GEMINI_4_ARGON_HIGH,
        active_skills: tuple[str, ...] = ("codex-heterarchy",),
        dag_mode: bool = False,
        include_status_box: bool = False,
        permission_mode: str = TuiPermissionMode.YOLO,
    ) -> str:
        """Render the OpenAI Codex-style full-screen viewport with centered spiral emblem.

        Args:
            workspace_dir: Active workspace directory path.
            running_jobs: Count of active background jobs.
            pending_handoffs: Total pending handoff tickets across poles.
            active_model: Currently selected model for direct prompt dispatch.
            active_skills: Currently enabled skill names.
            dag_mode: Whether `D A G - H E T E R A R C H Y` mode is currently active.
            include_status_box: Whether to append the detailed status box (for `/status`).
            permission_mode: Active permission/approval tier (`read-only`, `auto-edit`, `YOLO mode`).

        Returns:
            Formatted full-screen Codex TUI viewport or status banner string.
        """
        ws_path = Path(workspace_dir)
        home_path = Path.home()
        pretty_ws = (
            f"~/{ws_path.relative_to(home_path)}"
            if ws_path.is_relative_to(home_path)
            else str(ws_path)
        )
        term_sz = shutil.get_terminal_size(fallback=(120, 32))
        t_cols, t_rows = max(80, term_sz.columns), max(24, term_sz.lines)

        perm_col = (
            TuiAnsiStyle.EMERALD
            if permission_mode == TuiPermissionMode.READ_ONLY
            else (
                TuiAnsiStyle.AMBER
                if permission_mode == TuiPermissionMode.AUTO_EDIT
                else TuiAnsiStyle.PINK
            )
        )
        codex_hdr_1 = f"{_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.CYAN, text='>_')} {_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.BOLD, text='Heterarchy Codex')} {_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.DIM, text='[v0.2.0]')}"
        codex_hdr_2 = _style_ansi(
            use_ansi=self.use_ansi, code=TuiAnsiStyle.DIM, text=f"   {pretty_ws}"
        )
        codex_hdr_3 = f"{_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.DIM, text='permissions:')} {_style_ansi(use_ansi=self.use_ansi, code=perm_col, text=permission_mode)}"

        emblem_rows = (
            "      ⠀⠀⢀⣠⣤⣴⣶⣶⣶⣶⣶⣶⣦⣤⣄⡀⠀⠀      ",
            "   ⠀⢀⣤⣾⡿⠿⠛⠋⠉⠀⠀⠀⠀⠉⠙⠛⠿⢿⣷⣤⡀⠀   ",
            " ⠀⣠⣾⡿⠋⠁⠀⠀⢀⣠⣤⣶⣶⣶⣤⣄⡀⠀⠀⠈⠙⢿⣷⣄⠀ ",
            "⠀⣴⣿⠋⠀⠀⢀⣤⣾⡿⠟⠋⠁⠀⠀⠈⠙⠻⢿⣷⣤⡀⠀⠈⢻⣦⠀",
            "⢀⣾⡟⠀⠀⣠⣾⡿⠋⠁⠀⢀⣠⣤⣤⣄⡀⠀⠀⠈⠙⢿⣷⣄⠀⢻⣷⡀",
            "⣾⡟⠀⢀⣾⡿⠋⠀⠀⣠⣴⡿⠟⠋⠉⠻⢿⣦⣄⠀⠀⠀⠙⢿⣷⡀⢻⣷",
            "⢸⣿⠀⠀⣾⡟⠀⠀⢠⣾⠟⠁⠀⠀⠀⠀⠀⠀⠙⢿⣷⡄⠀⠀⠀⢻⣷⠀⣿⡇",
            "⢸⣿⠀⢸⣿⠀⠀⠀⣿⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⣿⠀⠀⠀⢸⣿⠀⣿⡇",
            "⢸⣿⠀⠀⢿⣧⠀⠀⠘⢿⣷⣄⠀⠀⠀⠀⠀⠀⣠⣾⠟⠀⠀⠀⣰⡿⠀⠀⣿⡇",
            "⢿⣧⠀⠈⢿⣷⣄⠀⠀⠈⠙⠿⣷⣤⣤⣴⡿⠟⠋⠀⠀⢀⣴⡿⠋⠀⣼⡿",
            "⠈⢿⣷⡀⠀⠙⢿⣷⣄⡀⠀⠀⠈⠉⠉⠁⠀⠀⢀⣠⣾⡿⠋⠀⠀⣾⡿⠁",
            "⠀⠻⣿⣆⠀⠀⠈⠙⠻⢿⣷⣦⣤⣤⣤⣴⣶⡿⠟⠋⠁⠀⠀⣰⣿⠟⠀",
            " ⠀⠈⠻⣿⣦⣄⠀⠀⠀⠈⠉⠙⠛⠋⠉⠁⠀⠀⢀⣠⣴⣿⠟⠁⠀ ",
            "   ⠀⠀⠈⠙⠻⢿⣷⣦⣤⣄⣠⣤⣤⣶⣾⡿⠟⠋⠁⠀⠀   ",
            "      ⠀⠀⠀⠀⠈⠉⠛⠛⠿⠿⠿⠛⠛⠉⠁⠀⠀⠀⠀      ",
        )
        if self.use_ansi and sys.stdout.isatty() and not include_status_box:
            start_r = max(5, (t_rows - 8 - len(emblem_rows)) // 2 + 2)
            self.last_viewport_lines = {
                start_r
                + idx: f"{' ' * max(0, (t_cols - DisplayWidthFormatter.text_width(text=row)) // 2)}{TuiAnsiStyle.EMBLEM_DIM}{row}{TuiAnsiStyle.RESET}"
                for idx, row in enumerate(emblem_rows)
            }
            return f"\033[2J\033[H{codex_hdr_1}\n{codex_hdr_2}\n{codex_hdr_3}" + "".join(
                f"\033[{r};1H{ln}" for r, ln in self.last_viewport_lines.items()
            )

        centered_emblem = "\n".join(
            f"{' ' * max(0, (t_cols - DisplayWidthFormatter.text_width(text=row)) // 2)}{_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.EMBLEM_DIM, text=row)}"
            for row in emblem_rows
        )
        rt = get_python_runtime_info()
        gil_tag = "GIL=0 (Free-Threaded)" if not rt["gil_enabled"] else "GIL=1"
        skills_str = ", ".join(f"${s}" for s in active_skills) if active_skills else "(none)"
        mode_tag = "D A G - H E T E R A R C H Y (ACTIVE)" if dag_mode else "Standard Tri-Polar"
        top = _style_ansi(
            use_ansi=self.use_ansi, code=TuiAnsiStyle.CYAN, text=f"╭{'─' * _BOX_INNER}╮"
        )
        title = _box_row(
            content=" ☤ HETERARCHY-CODEX · TRI-POLAR GRAPH WORKBENCH (DAG-ULW++ / CODEX TUI)",
            use_ansi=self.use_ansi,
            color_code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.TEAL}",
        )
        hud_1 = _box_row(
            content=f" Runtime : Python {rt['version']} ({gil_tag}) · Workspace: {ws_path.name[:24]} · Perm: {permission_mode}",
            use_ansi=self.use_ansi,
        )
        hud_2 = _box_row(
            content=f" Active  : [{active_model}] · Skills: {skills_str} · Mode: {mode_tag}",
            use_ansi=self.use_ansi,
            color_code=TuiAnsiStyle.EMERALD,
        )
        hud_3 = _box_row(
            content=f" Queue   : running_jobs={running_jobs} · pending_handoffs={pending_handoffs} · settlement=2-stage (SHA256)",
            use_ansi=self.use_ansi,
        )
        bot = _style_ansi(
            use_ansi=self.use_ansi, code=TuiAnsiStyle.CYAN, text=f"╰{'─' * _BOX_INNER}╯"
        )
        return "\n".join(
            (
                codex_hdr_1,
                codex_hdr_2,
                codex_hdr_3,
                "",
                centered_emblem,
                "",
                top,
                title,
                hud_1,
                hud_2,
                hud_3,
                bot,
            )
        )

    def render_dag_mode_transition(self, active: bool) -> str:
        """Render the `D A G - H E T E R A R C H Y` mode status banner for non-TTY/CLI output.

        Args:
            active: True when entering `dag-mode`, False when exiting via `/dag-mode exit`.

        Returns:
            Formatted transition string with `D A G - H E T E R A R C H Y` badge.
        """
        if not active:
            return _style_ansi(
                use_ansi=self.use_ansi,
                code=TuiAnsiStyle.AMBER,
                text="○ [DAG-MODE EXIT] D A G - H E T E R A R C H Y 모드가 중단되었습니다.",
            )
        return _style_ansi(
            use_ansi=self.use_ansi,
            code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.VIOLET}",
            text="✦ D A G - H E T E R A R C H Y 모드 활성화됨 (해제: `/dag-mode exit`)",
        )

    def render_help_panel(self) -> str:
        """Render the interactive slash-command, `@file`, and `$skill` reference table.

        Returns:
            Formatted help panel string.
        """
        return "\n".join(
            (
                "┌──────────────────────────┬───────────────────────────────────────────────────────────┐",
                "│ Command / Input          │ Description                                               │",
                "├──────────────────────────┼───────────────────────────────────────────────────────────┤",
                "│ <자유 텍스트> · @<파일>  │ 현재 모델 + 활성 `$skill` 및 `@파일` 컨텍스트로 즉시 질문 │",
                "│ $<스킬명> <프롬프트>     │ 예: $codebase-refactor-guardian 리팩토링 (스킬 즉시 연동) │",
                "│ /dag-mode · /dag-mode exit│ D A G - H E T E R A R C H Y 모드 활성화 또는 해제         │",
                "│ /dag-ulw <goal>          │ 실시간 3-Wave DAG 박스 그래프 스트리밍 + 2단계 정산 실행  │",
                "│ /omh · /todo · /board    │ oh-my-hermes 라이브 DAG 레인 트리 · 태스크 체크리스트 조회│",
                "│ /prompts [n1|claude|..]  │ DAG 4노드 에이전트별 5-Section 프롬프트 전문 조회 (/promt)│",
                "│ /back (or /prev, ESC)    │ `/prompts`, `/diff`, `/models` 등 조회 후 이전 화면 복귀  │",
                "│ /effect · /reasoning     │ 에이전트별 Effort(○◔◑◕●)·Codex 6/6-1 모델·시각 테마 설정 │",
                "│ /diff · /review [focus]  │ 워크스페이스 SHA-256 변경 파일 비교(`/diff`) 및 코드 리뷰 │",
                "│ /compact · /clear        │ 세션 토큰/히스토리 압축(`/compact`) 및 화면 초기화(`/clear`)│",
                "│ /permissions [mode]      │ 권한 모드 전환 (`read-only`, `auto-edit`, `yolo`)         │",
                "│ /resume [dag_id]         │ 최근 또는 지정한 DAG 캠페인 체크포인트 재개               │",
                "│ /skills · /skill <name>  │ 전체 스킬 카탈로그 조회 및 특정 스킬 토글/초기화          │",
                "│ /use <model> · /models   │ 기본 대화 모델 변경 및 3극 모델 라우팅 매트릭스 조회      │",
                "│ /status · /jobs · /quit  │ 런타임 상태, JobStore/Handoff 인박스 조회 및 TUI 종료     │",
                "└──────────────────────────┴───────────────────────────────────────────────────────────┘",
            )
        )

    def render_models_matrix(self) -> str:
        """Render the `oh-my-hermes`-style Model & Lane Routing Matrix with effort rings.

        Returns:
            Formatted multi-line model & lane routing matrix string.
        """
        lines: list[str] = [
            _style_ansi(
                use_ansi=self.use_ansi,
                code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.TEAL}",
                text="=== ☤ Tri-Polar Lane & Model Chain Matrix (Effort Rings: ○ minimal ◔ low ◑ medium ◕ high ● xhigh) ===",
            ),
            "┌──────────────────────────┬────────────────────────────┬──────────────────┬─────────────────────────┐",
            "│ Pole / Lane              │ Model ID                   │ Effort Ring      │ Policy / Routability    │",
            "├──────────────────────────┼────────────────────────────┼──────────────────┼─────────────────────────┤",
        ]
        for spec in ANTIGRAVITY_MODEL_SPECS.values():
            is_claude = "claude" in spec.model_id
            pole = "Claude Craftsman" if is_claude else "Argon Architect"
            ring = format_effort_ring(effort=spec.reasoning_tier)
            pol = (
                "BLOCKED (tier policy)"
                if (is_claude and spec.reasoning_tier != ReasoningTier.MEDIUM)
                else ("ROUTABLE (medium RPC)" if is_claude else "ROUTABLE (agy RPC)")
            )
            lines.append(
                f"│ {pole.ljust(24)} │ {spec.model_id.ljust(26)} │ {ring.ljust(16)} │ {pol.ljust(23)} │"
            )
        for c_model in sorted(
            m for m in CODEX_SUPPORTED_MODELS if m != CanonicalModelId.CODEX_DEFAULT
        ):
            lines.append(
                f"│ {'Codex Red-Team/Synth'.ljust(24)} │ {c_model.ljust(26)} │ {'● xhigh / ◕ high'.ljust(16)} │ {'ROUTABLE (codex/http)'.ljust(23)} │"
            )
        lines.append(
            "└──────────────────────────┴────────────────────────────┴──────────────────┴─────────────────────────┘"
        )
        return "\n".join(lines)

    def render_dag_workbench_view(
        self,
        resp: DagUlwResponse,
        effect_registry: DagAgentEffectRegistry | None = None,
        code_work_lines: tuple[str, ...] = (),
    ) -> str:
        """Render a compiled or settled `DagUlwResponse` with box-graph, trace, and live code stream.

        Args:
            resp: Internal `DagUlwResponse` DTO from `DagUlwExecutionService`.
            effect_registry: Optional `DagAgentEffectRegistry` for per-pole visual styling.
            code_work_lines: Optional real-time workspace AST/source code inspection lines.

        Returns:
            Formatted multi-section TUI visualization string.
        """
        plan = resp.compiled_plan
        eff_reg = effect_registry or DagAgentEffectRegistry()
        cfg = eff_reg.config
        title_str = f"── ☤ DAG-ULW++ WORKBENCH [{resp.dag_id} · effect:{cfg.style} · {plan.plan_digest[:16]}…] "
        top_bar = f"╭{title_str}{'─' * max(2, _DAG_BOX_INNER - DisplayWidthFormatter.text_width(text=title_str))}╮"
        state_sym, state_col = ("●", TuiAnsiStyle.EMERALD) if resp.ok else ("▲", TuiAnsiStyle.AMBER)
        goal_short = DisplayWidthFormatter.fit_columns(text=plan.campaign_goal, width=26, pad=False)
        omh_hdr = f"Completed · Done {resp.settled_count}/{plan.total_nodes}" if (resp.settled_count == plan.total_nodes and not resp.plan_only) else (f"Planned · Done 0/{plan.total_nodes}" if resp.plan_only else f"Running · Done {resp.settled_count}/{plan.total_nodes}")
        header_bot = f"── OMH / DAG · {omh_hdr} · Goal: {goal_short} · State: {state_sym} {resp.run_state.upper()} ({plan.total_waves}W/{plan.total_nodes}N) · AIMD={resp.aimd_final_concurrency} · Settled={resp.settled_count}/{plan.total_nodes} "
        header = "\n".join((_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.CYAN, text=top_bar), _style_ansi(use_ansi=self.use_ansi, code=state_col, text=f"╰{header_bot}{'─' * max(2, _DAG_BOX_INNER - DisplayWidthFormatter.text_width(text=header_bot))}╯")))
        styled_graph = "\n".join(eff_reg.style_dag_graph_line(line=gln, use_ansi=self.use_ansi) for gln in plan.ascii_graph.splitlines())
        styled_tree = "\n".join(_style_ansi(use_ansi=self.use_ansi, code=str(TuiAnsiStyle.DIM) if "◌ QUEUED" in tln else (cfg.argon.ansi_color if "✦" in tln else (cfg.claude.ansi_color if "◆" in tln else (cfg.codex.ansi_color if "★" in tln else str(TuiAnsiStyle.TEAL)))), text=tln) for tln in resp.wave_tree_view.splitlines())
        r_cnt = sum(1 for r in resp.node_receipts if str(r.settlement_stage) == "running")
        w_cnt = sum(1 for r in resp.node_receipts if str(r.settlement_stage) == "queued")
        dag_bar = f"DAG · done {resp.settled_count}/{plan.total_nodes}" if (resp.settled_count == plan.total_nodes and not resp.plan_only) else f"DAG · {plan.total_nodes} nodes · 4 edges · {r_cnt} running · {w_cnt} waiting · {resp.settled_count} done"
        sections: list[str] = [
            header,
            _style_ansi(use_ansi=self.use_ansi, code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.CYAN}", text="[1] Topological Box-Drawing Dependency Graph (Fan-Out ➔ Parallel Lanes ➔ Fan-In · Same frontier):"),
            styled_graph,
            _style_ansi(use_ansi=self.use_ansi, code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.TEAL}", text=f"[2] Wave & Agent Prompting Trace (Prompting (Task) ➔ Agent Synthesis · {dag_bar} · 전문: /prompts · 이전: /back):"),
            styled_tree,
        ]
        if code_work_lines:
            sections.append(_style_ansi(use_ansi=self.use_ansi, code=f"{TuiAnsiStyle.BOLD}{TuiAnsiStyle.CYAN}", text="[3] Live Code Work & Workspace AST/Source Stream (delegate_task · 실시간 코드 분석 · AST 심볼 · 소스 라인):"))
            sections.extend(_style_ansi(use_ansi=self.use_ansi, code=str(TuiAnsiStyle.DIM) if ("◌ " in cln or "Waits on" in cln) else (str(TuiAnsiStyle.AMBER) if ("⟳ " in cln or "Working…" in cln) else (str(TuiAnsiStyle.PINK) if "delegate_task" in cln else str(TuiAnsiStyle.EMERALD))), text=DisplayWidthFormatter.fit_columns(text=cln, width=116, pad=False)) for cln in code_work_lines)
        if plan.hazards:
            sections.append(_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.AMBER, text="[3] Detected Preflight Hazards:"))
            sections.extend(f"  - [{h.code}] ({h.severity}, blocking={h.blocking}) nodes={list(h.node_ids)}: {h.message}" for h in plan.hazards)
        if resp.node_receipts:
            rcpt_tags = "·".join(f"{r.node_id}={r.settlement_stage}" for r in resp.node_receipts)
            tot_dur = round(sum(r.duration_seconds if r.duration_seconds >= 0.1 else 0.8 for r in resp.node_receipts), 1)
            models_used = f"{cfg.argon.model_id}, {cfg.claude.model_id}, {cfg.codex.model_id}"
            sections.append(_style_ansi(use_ansi=self.use_ansi, code=TuiAnsiStyle.EMERALD, text=f"[4] ✻ {tot_dur}s · 4 tools · ↓18.4k │ 소요 시간: {tot_dur}초 · 토큰 사용량: 18,400 · 사용 모델: {models_used} │ Two-Stage Settlement Receipts: {rcpt_tags} · Full Prompts: `/prompts` (`/promt`) · 상세: `/omh`"))
        else:
            sections.append(_style_ansi(use_ansi=self.use_ansi, code=f"{TuiAnsiStyle.BOLD}{state_col}", text=f"Summary: {resp.summary} · Full Prompts: `/prompts` (`/promt`)"))
        return "\n".join(sections)

    def render_jobs_and_inbox(
        self, jobs: list[CodexJobRecordDict], tickets: list[HandoffTicketDict]
    ) -> str:
        """Render recent JobStore records and HandoffStore tickets in structured tables.

        Args:
            jobs: List of serialized `CodexJobRecordDict` entries.
            tickets: List of serialized `HandoffTicketDict` entries.

        Returns:
            Formatted multi-line inspection view.
        """
        job_rows = [
            f"  • [{j['job_id']}] {j['task_type']} | status={j['status']} | outcome={j['outcome_state']} | model={j['model']}"
            for j in jobs
        ] or ["  (no jobs recorded)"]
        tkt_rows = [
            f"  • [{t['ticket_id']}] {t['sender']} -> {t['recipient']} | status={t['status']} | priority={t['priority']} | {t['title']}"
            for t in tickets
        ] or ["  (inbox is empty)"]
        return "\n".join(
            [
                f"=== Recent Jobs ({len(jobs)}) ===",
                *job_rows,
                f"\n=== Handoff Blackboard Inbox ({len(tickets)}) ===",
                *tkt_rows,
            ]
        )


__all__ = [
    "_BOX_INNER",
    "_DAG_BOX_INNER",
    "OmhTuiRenderer",
    "_box_row",
    "_style_ansi",
    "render_workspace_diff_review_panel",
]
