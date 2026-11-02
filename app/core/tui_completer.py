"""Live terminal autocomplete, Codex-style input bar, and suggestion popup for `hcodex tui`."""

from __future__ import annotations

import os
import re
import select
import shutil
import sys
import termios
import tty
from dataclasses import dataclass

from app.core.dag_enums import TuiAnsiStyle, TuiSlashCommand
from app.core.skill_registry import DisplayWidthFormatter, TuiSkillRegistry

_SKILL_TOKEN_RE = re.compile(r"(\$[A-Za-z0-9_.\-]+(?::[A-Za-z0-9_.\-]+)?)")


@dataclass(frozen=True, slots=True)
class TuiSuggestionItem:
    """Single autocomplete recommendation entry for the TUI floating popup."""

    insert_text: str
    display_label: str
    badge: str
    description: str
    keywords: str = ""
    submit_on_enter: bool = False


_STATIC_COMMAND_ITEMS: tuple[TuiSuggestionItem, ...] = tuple(
    TuiSuggestionItem(insert_text=ins, display_label=lbl, badge=b, description=d, keywords=k, submit_on_enter=s)
    for ins, lbl, b, d, k, s in (
        ("/skills", "/skills", "⚙ CMD", "등록된 전체 스킬 카탈로그($skill) 표 보기", "스킬 목록 카탈로그 skills list catalog", True),
        ("/dag-mode", "/dag-mode", "☤ DAG-MODE", "D A G - H E T E R A R C H Y 모드 활성화 (애니메이션)", "다그 모드 헤테라키 울트라 활성화 dag mode heterarchy ultra on", True),
        ("/dag-mode exit", "/dag-mode exit", "☤ DAG-MODE", "D A G - H E T E R A R C H Y 모드 중단 및 해제", "다그 모드 해제 종료 중단 dag mode exit off stop", True),
        ("/effect", "/effect", "⚡ EFFECT", "DAG 에이전트별 Effort(○◔◑◕●)·모델·시각 효과 매트릭스 (/reasoning)", "다그 에이전트 이펙트 에포트 설정 effect effort reasoning matrix", True),
        ("/reasoning", "/reasoning", "⚡ REASON", "OpenAI Codex 스타일 추론 강도(minimal..xhigh) 및 모델 제어", "추론 리즈닝 에포트 reasoning effort xhigh high medium low minimal", True),
        ("/prompts", "/prompts", "☤ PROMPTS", "DAG 4노드 에이전트별 5-Section 프롬프트 전문 조회 (/promt)", "프롬프트 조회 트레이스 어떻게 작업 prompting prompts promt trace", True),
        ("/back", "/back", "↩ BACK", "이전 화면(DAG 워크벤치 등)으로 돌아가기 (/prev · ESC)", "이전 화면 돌아가기 뒤로 복귀 back prev return esc", True),
        ("/diff", "/diff", "◈ DIFF", "세션 기준 워크스페이스 SHA-256 변경 파일 비교 (Codex /diff)", "변경 파일 차이 비교 diff git status modified", True),
        ("/review", "/review", "★ REVIEW", "현재 워크스페이스 변경분 코드 리뷰 및 해저드 점검 (Codex /review)", "코드 리뷰 감사 점검 review audit diff", True),
        ("/compact", "/compact", "⚙ COMPACT", "세션 컨텍스트 토큰 및 뷰 히스토리 압축 (Context % 확보)", "컨텍스트 압축 요약 토큰 compact context memory", True),
        ("/clear", "/clear", "⚙ CLEAR", "세션 히스토리 정리 및 메인 Codex TUI 화면 초기화", "화면 초기화 지우기 클리어 clear reset", True),
        ("/permissions", "/permissions", "⚙ PERM", "권한/승인 모드 조회 및 전환 (`read-only`, `auto-edit`, `yolo`)", "권한 승인 샌드박스 permissions approvals yolo read-only auto-edit", True),
        ("/resume", "/resume", "☤ RESUME", "최근 또는 지정한 DAG 캠페인 체크포인트 재개 (Codex /resume)", "재개 이어하기 복구 resume checkpoint dag", True),
        ("/omh", "/omh", "⚚ OMH", "oh-my-hermes 라이브 DAG 레인 트리 · 태스크 체크리스트 · 요약 조회", "오마이헤르메스 보드 체크리스트 투두 omh board todo plan", True),
        ("/todo", "/todo", "⚚ TODO", "oh-my-hermes 단계별 작업 완료 체크리스트 및 증거 조회", "투두 계획 체크리스트 완료 추적 todo plan omh", True),
        ("/prompts claude", "/prompts claude", "☤ PROMPTS", "Wave 1 Claude 에이전트 프롬프트 및 합성 결과 조회", "프롬프트 클로드 소넷 오퍼스 prompts promt claude", True),
        ("/prompts codex", "/prompts codex", "☤ PROMPTS", "Wave 1 Codex 레드팀 에이전트 프롬프트 및 감사 결과 조회", "프롬프트 코덱스 레드팀 감사 prompts promt codex", True),
        ("/effect codex 6 terra xhigh", "/effect codex 6 terra xhigh", "⚡ EFFECT", "DAG Codex 에이전트를 테라(6) `gpt-6-terra` (● xhigh)로 설정", "코덱스 6 테라 terra gpt-6-terra xhigh effect codex", True),
        ("/effect codex 6 luna xhigh", "/effect codex 6 luna xhigh", "⚡ EFFECT", "DAG Codex 에이전트를 루나(6) `gpt-6-luna` (● xhigh)로 설정", "코덱스 6 루나 luna gpt-6-luna xhigh effect codex", True),
        ("/effect codex 6 sol xhigh", "/effect codex 6 sol xhigh", "⚡ EFFECT", "DAG Codex 에이전트를 솔(6) `gpt-6-sol` (● xhigh)로 설정", "코덱스 6 솔 sol gpt-6-sol xhigh effect codex", True),
        ("/effect codex 6 astra xhigh", "/effect codex 6 astra xhigh", "⚡ EFFECT", "DAG Codex 에이전트를 아스트라(6) `gpt-6-astra` (● xhigh)로 설정", "코덱스 6 아스트라 astra gpt-6-astra xhigh effect codex", True),
        ("/effect codex 6-1 terra xhigh", "/effect codex 6-1 terra xhigh", "⚡ EFFECT", "DAG Codex 에이전트를 테라(6-1) `gpt-6.1-terra` (● xhigh)로 설정", "코덱스 6-1 6.1 테라 terra gpt-6.1-terra xhigh effect codex", True),
        ("/effect codex 6-1 sol xhigh", "/effect codex 6-1 sol xhigh", "⚡ EFFECT", "DAG Codex 에이전트를 솔(6-1) `gpt-6.1-sol` (● xhigh)로 설정", "코덱스 6-1 6.1 솔 sol gpt-6.1-sol xhigh effect codex", True),
        ("/effect codex xhigh", "/effect codex xhigh", "⚡ EFFECT", "DAG Codex 에이전트 Effort를 ● xhigh로 설정", "코덱스 이펙트 에포트 xhigh effect codex", True),
        ("/effect codex high", "/effect codex high", "⚡ EFFECT", "DAG Codex 에이전트 Effort를 ◕ high로 설정", "코덱스 이펙트 에포트 high effect codex", True),
        ("/effect codex low", "/effect codex low", "⚡ EFFECT", "DAG Codex 에이전트 Effort를 ◔ low로 설정", "코덱스 이펙트 에포트 low effect codex", True),
        ("/effect claude opus", "/effect claude opus", "⚡ EFFECT", "DAG Claude 에이전트 모델을 Opus 5.5로 설정", "클로드 오퍼스 이펙트 에포트 effect claude opus", True),
        ("/effect claude sonnet", "/effect claude sonnet", "⚡ EFFECT", "DAG Claude 에이전트 모델을 Sonnet 5.5로 설정", "클로드 소넷 이펙트 에포트 effect claude sonnet", True),
        ("/effect argon flash", "/effect argon flash", "⚡ EFFECT", "DAG Argon 에이전트 모델을 Gemini 3 Flash로 설정", "아르곤 플래시 이펙트 에포트 effect argon flash", True),
        ("/effect style cyber-matrix", "/effect style cyber-matrix", "◈ STYLE", "DAG 시각 효과를 사이버 매트릭스(cyber-matrix)로 전환", "사이버 매트릭스 스타일 테마 effect style cyber matrix", True),
        ("/effect style royal-prism", "/effect style royal-prism", "✧ STYLE", "DAG 시각 효과를 로열 프리즘(royal-prism)으로 전환", "로열 프리즘 스타일 테마 effect style royal prism", True),
        ("/effect style neon-pulse", "/effect style neon-pulse", "✦ STYLE", "DAG 시각 효과를 네온 펄스(neon-pulse)로 전환", "네온 펄스 스타일 테마 effect style neon pulse", True),
        ("/effect reset", "/effect reset", "⚡ EFFECT", "DAG 에이전트 Effect 설정을 기본값으로 초기화", "이펙트 에포트 초기화 리셋 effect reset default", True),
        ("/use argon", "/use argon", "✦ MODEL", "기본 모델을 Gemini 4 Argon High (argon4)로 전환", "아르곤 아르곤4 제미나이 모델 변경 use argon argon4 gemini", True),
        ("/use argon4", "/use argon4", "✦ MODEL", "기본 모델을 Gemini 4 Argon High (`argon4`)로 전환", "아르곤4 제미나이 모델 변경 use argon4 gemini-4-argon-high", True),
        ("/use sonnet", "/use sonnet", "✦ MODEL", "기본 모델을 Claude Sonnet 5.5 Medium으로 전환", "소넷 클로드 모델 변경 use sonnet claude", True),
        ("/use opus", "/use opus", "✦ MODEL", "기본 모델을 Claude Opus 5.5 Medium으로 전환", "오퍼스 클로드 모델 변경 use opus claude", True),
        ("/use flash", "/use flash", "✦ MODEL", "기본 모델을 Gemini 3 Flash로 전환", "플래시 제미나이 모델 변경 use flash gemini", True),
        ("/use gpt-6-astra", "/use gpt-6-astra", "★ MODEL", "기본 모델을 아스트라(6) `gpt-6-astra`로 전환", "코덱스 6 아스트라 모델 변경 use gpt-6-astra codex astra", True),
        ("/use gpt-6-terra", "/use gpt-6-terra", "★ MODEL", "기본 모델을 테라(6) `gpt-6-terra`로 전환", "코덱스 6 테라 모델 변경 use gpt-6-terra codex terra", True),
        ("/use gpt-6-luna", "/use gpt-6-luna", "★ MODEL", "기본 모델을 루나(6) `gpt-6-luna`로 전환", "코덱스 6 루나 모델 변경 use gpt-6-luna codex luna", True),
        ("/use gpt-6-sol", "/use gpt-6-sol", "★ MODEL", "기본 모델을 솔(6) `gpt-6-sol`로 전환", "코덱스 6 솔 모델 변경 use gpt-6-sol codex sol", True),
        ("/use gpt-6.1-terra", "/use gpt-6.1-terra", "★ MODEL", "기본 모델을 테라(6-1) `gpt-6.1-terra`로 전환", "코덱스 6-1 6.1 테라 모델 변경 use gpt-6.1-terra codex", True),
        ("/use gpt-6.1-sol", "/use gpt-6.1-sol", "★ MODEL", "기본 모델을 솔(6-1) `gpt-6.1-sol`로 전환", "코덱스 6-1 6.1 솔 모델 변경 use gpt-6.1-sol codex", True),
        ("/sonnet ", "/sonnet <질문>", "✦ ASK", "Claude Sonnet 5.5 Medium에게 즉시 질문", "소넷 질문 프롬프트 sonnet ask", False),
        ("/opus ", "/opus <질문>", "✦ ASK", "Claude Opus 5.5 Medium에게 심층 아키텍처 질문", "오퍼스 질문 아키텍처 opus ask", False),
        ("/argon ", "/argon <질문>", "✦ ASK", "Gemini 4 Argon High에게 매크로 설계 질문", "아르곤 질문 설계 argon ask", False),
        ("/dag-ulw ", "/dag-ulw <목표>", "☤ DAG", "실시간 3-Wave DAG 캠페인 실행 + 2단계 SHA-256 정산", "다그 그래프 설계 캠페인 실행 dag ulw run plan", False),
        ("/skill default", "/skill default", "⚙ SKILL", "기본 스킬($codex-heterarchy) 단독 활성화로 복원", "스킬 기본 디폴트 헤테라키 skill default codex-heterarchy", True),
        ("/skill core", "/skill core", "⚙ SKILL", "3대 핵심 스킬(codex-heterarchy 등) 일괄 활성화", "스킬 코어 초기화 skill core", True),
        ("/skill clear", "/skill clear", "⚙ SKILL", "현재 활성화된 모든 스킬 해제", "스킬 해제 끄기 초기화 skill clear off", True),
        ("/models", "/models", "⚙ CMD", "3극 모델 라우팅 상태 및 체인 매트릭스 보기", "모델 라우팅 상태 매트릭스 models lanes", True),
        ("/status", "/status", "⚙ CMD", "메인 Codex TUI 화면 새로고침 및 상태 점검", "상태 헬스 점검 새로고침 status health clear", True),
        ("/jobs", "/jobs", "⚙ CMD", "최근 실행된 비동기 잡 및 DAG 캠페인 목록", "잡 작업 목록 기록 jobs history", True),
        ("/inbox", "/inbox", "⚙ CMD", "에이전트 간 핸드오프 티켓 인박스 조회", "인박스 티켓 핸드오프 inbox handoff", True),
        ("/theme", "/theme", "⚙ CMD", "TUI 테마 팔레트 확인 및 전환", "테마 색상 디자인 theme color", True),
        ("/help", "/help", "⚙ CMD", "전체 TUI 명령어 및 단축키 도움말", "도움말 사용법 명령어 help guide", True),
        ("/quit", "/quit", "⚙ CMD", "Heterarchy-Codex TUI 종료", "종료 나가기 끝 quit exit", True),
    )
)
_EXACT_SUBMIT_COMMANDS: frozenset[str] = frozenset((*(str(c) for c in TuiSlashCommand), "/dag-mode exit", "/skill default", "/skill core", "/skill clear", "/use argon", "/use argon4", "/use argon-4", "/use sonnet", "/use opus", "/use flash"))


class TuiSuggestionEngine:
    """Rank and format live slash-command, `$skill` token, and model recommendations."""

    def __init__(self, skill_registry: TuiSkillRegistry, git_branch: str = "main") -> None:
        """Initialize the suggestion engine with the active `TuiSkillRegistry` and telemetry.

        Args:
            skill_registry: Discovered skills registry instance.
            git_branch: Active workspace Git branch label for the status bar.
        """
        self.skill_registry: TuiSkillRegistry = skill_registry
        self.git_branch, self.tokens_in, self.tokens_out, self.context_limit = git_branch, 0, 0, 200_000
        self.omh_status_badge: str = "▶▶ accept all on · OMH ready"
        self._catalog_key: tuple[tuple[str, ...], int] | None = None
        self._catalog_cache: list[TuiSuggestionItem] = []

    def build_base_catalog(self) -> list[TuiSuggestionItem]:
        """Build or return the cached catalog of commands, model switches, and `$skill` tokens.

        Returns:
            Ordered list of `TuiSuggestionItem` entries.
        """
        cache_key = (tuple(self.skill_registry.active_skills), len(self.skill_registry.skills))
        if self._catalog_key == cache_key and self._catalog_cache:
            return self._catalog_cache
        active_set = set(self.skill_registry.active_skills)
        active_sk = [TuiSuggestionItem(insert_text=f"{e.dollar_token} ", display_label=e.dollar_token, badge=f"● ON·{e.scope}", description=e.description or f"{s} 스킬 활성화", keywords=f"스킬 {s} /{s} ${s} {e.dollar_token} {e.plugin_name} {e.scope} {e.description}") for s, e in self.skill_registry.skills.items() if s in active_set]
        inactive_sk = [TuiSuggestionItem(insert_text=f"{e.dollar_token} ", display_label=e.dollar_token, badge=f"○ {e.scope}", description=e.description or f"{s} 스킬 활성화", keywords=f"스킬 {s} /{s} ${s} {e.dollar_token} {e.plugin_name} {e.scope} {e.description}") for s, e in self.skill_registry.skills.items() if s not in active_set]
        self._catalog_cache = [_STATIC_COMMAND_ITEMS[0], *active_sk, *_STATIC_COMMAND_ITEMS[1:], *inactive_sk]
        self._catalog_key = cache_key
        return self._catalog_cache

    def suggest(self, buffer: str, cursor_pos: int, force_open: bool = False) -> tuple[list[TuiSuggestionItem], int, str]:
        """Compute live ranked suggestions for the current input `buffer` and `cursor_pos`.

        Args:
            buffer: Full current input line text.
            cursor_pos: Current cursor character offset in `buffer`.
            force_open: Whether to return starter suggestions even when `buffer` is empty.

        Returns:
            Tuple of `(ranked_suggestions, replace_start_offset, active_query_token)`.
        """
        prefix = buffer[:cursor_pos]
        catalog = self.build_base_catalog()
        if not prefix.strip():
            return (catalog, 0, "") if force_open else ([], 0, "")

        lower_prefix = prefix.lower()
        if lower_prefix.startswith(("/dag-mode ", "/use ", "/effect ", "/effort ", "/reasoning ", "/prompts ", "/promt ")):
            pfx_cmd = "/dag-mode" if lower_prefix.startswith("/dag-mode ") else ("/use " if lower_prefix.startswith("/use ") else ("/prompts" if lower_prefix.startswith(("/prompts ", "/promt ")) else "/effect"))
            q_sub = lower_prefix.split(maxsplit=1)[1].strip() if " " in lower_prefix.strip() else ""
            sub_pool = [it for it in catalog if it.insert_text.startswith(pfx_cmd)]
            matched_sub = [it for it in sub_pool if not q_sub or q_sub in it.insert_text[len(pfx_cmd):] or q_sub in it.keywords]
            return sorted(matched_sub, key=lambda it: not it.insert_text[len(pfx_cmd):].strip().startswith(q_sub)), 0, q_sub

        if lower_prefix.startswith("/skill "):
            query = lower_prefix[7:].strip().lstrip("/$")
            sub_items: list[TuiSuggestionItem] = [
                TuiSuggestionItem(insert_text="/skill default", display_label="default", badge="⚙ PRESET", description="기본 스킬($codex-heterarchy) 단독 활성화", submit_on_enter=True),
                TuiSuggestionItem(insert_text="/skill core", display_label="core", badge="⚙ PRESET", description="3대 핵심 스킬 일괄 활성화", submit_on_enter=True),
                TuiSuggestionItem(insert_text="/skill clear", display_label="clear", badge="⚙ PRESET", description="모든 활성 스킬 해제", submit_on_enter=True),
                *(TuiSuggestionItem(insert_text=f"/skill {s}", display_label=e.dollar_token, badge="● ACTIVE" if s in self.skill_registry.active_skills else "○ READY", description=e.description or f"{s} 토글", submit_on_enter=True) for s, e in self.skill_registry.skills.items()),
            ]
            matched_sk = [it for it in sub_items if not query or query in it.display_label.lower() or query in it.insert_text[7:].lower() or query in it.description.lower()]
            return sorted(matched_sk, key=lambda it: (not it.insert_text[7:].lower().startswith(query), not it.display_label.lstrip("$").lower().startswith(query))), 0, query

        token_start = prefix.rfind(" ") + 1
        raw_token = prefix[token_start:]
        if not raw_token:
            prior_tokens = prefix.strip().split()
            if force_open and prior_tokens and all(t.startswith(("/", "$")) for t in prior_tokens):
                used = {t.lstrip("/$").split(":")[-1].lower() for t in prior_tokens}
                return [it for it in catalog if it.display_label.startswith("$") and it.display_label.lstrip("$").split(":")[-1].lower() not in used], token_start, ""
            return [], token_start, ""

        is_dollar, is_slash = raw_token.startswith("$"), raw_token.startswith("/")
        is_trigger = is_dollar or is_slash
        query_clean = raw_token[1:].lower() if is_trigger else raw_token.lower()
        if (token_start > 0 and not is_trigger) or (not is_trigger and not query_clean):
            return [], token_start, raw_token

        prior_parts = prefix[:token_start].split()
        allow_slash_here = token_start == 0 or all(t.startswith("$") for t in prior_parts)
        already_mentioned = {t.lstrip("/$").split(":")[-1].lower() for t in prior_parts if t.startswith(("/", "$"))}
        t_prefix: list[TuiSuggestionItem] = []
        t_label: list[TuiSuggestionItem] = []
        t_kw: list[TuiSuggestionItem] = []
        for item in catalog:
            full_clean = item.display_label.lstrip("/$").lower()
            short_clean = full_clean.split(":")[-1].split()[0]
            is_sk = item.display_label.startswith("$")
            if (is_dollar and not is_sk) or (is_slash and not query_clean and is_sk) or (not allow_slash_here and not is_sk) or (short_clean in already_mentioned and is_sk):
                continue
            if not query_clean or full_clean.startswith(query_clean) or short_clean.startswith(query_clean):
                t_prefix.append(item)
            elif query_clean in full_clean:
                t_label.append(item)
            elif query_clean in item.description.lower() or query_clean in item.keywords.lower():
                t_kw.append(item)
        if is_slash and query_clean:
            t_prefix.sort(key=lambda it: it.display_label.startswith("$"))
        return [*t_prefix, *t_label, *t_kw], token_start, raw_token

    def apply_completion(self, buffer: str, cursor_pos: int, item: TuiSuggestionItem, replace_start: int) -> tuple[str, int]:
        """Replace the token at `[replace_start:cursor_pos]` with `item.insert_text`.

        Args:
            buffer: Current input line buffer.
            cursor_pos: Current cursor character index.
            item: Selected `TuiSuggestionItem`.
            replace_start: Start character index of the token being replaced.

        Returns:
            Tuple of `(new_buffer, new_cursor_pos)`.
        """
        before, after = buffer[:replace_start], buffer[cursor_pos:]
        inserted = item.insert_text[:-1] if (after.startswith(" ") and item.insert_text.endswith(" ")) else item.insert_text
        return before + inserted + after, len(before) + len(inserted)

    def render_dropdown_box(
        self, items: list[TuiSuggestionItem], selected_index: int, max_visible: int = 7, use_ansi: bool = True, term_cols: int = 94
    ) -> list[str]:
        """Render the floating box-drawing recommendation menu above the input line.

        Args:
            items: Ranked suggestion items.
            selected_index: Highlighted item index.
            max_visible: Maximum visible rows in the scrolling window.
            use_ansi: Whether to include ANSI colors.
            term_cols: Available terminal column width.

        Returns:
            List of formatted single-row strings for the dropdown box.
        """
        if not items:
            return []
        total, sel = len(items), max(0, min(selected_index, len(items) - 1))
        win_start = max(0, min(sel - max_visible // 2, max(0, total - max_visible)))
        win_end, box_w = min(total, win_start + max_visible), max(68, min(96, term_cols - 6))
        inner_w = box_w - 2
        hdr_fit = DisplayWidthFormatter.fit_columns(text=f" ☤ 추천 자동완성 ({sel + 1}/{total}) ── [$스킬 · /명령어 · ↑↓ 이동 · Tab 완성 · Enter 실행] ", width=inner_w, pad=False)
        top_b, bot_b = f"╭{hdr_fit}{'─' * max(0, inner_w - DisplayWidthFormatter.text_width(text=hdr_fit))}╮", f"╰{'─' * inner_w}╯"
        rows: list[str] = [f"\033[38;5;141m{top_b}\033[0m" if use_ansi else top_b]
        lbl_w, bdg_w, dsc_w = 34, 13, max(14, inner_w - 54)
        for idx in range(win_start, win_end):
            it, ptr = items[idx], ("▸" if idx == sel else " ")
            lbl = DisplayWidthFormatter.fit_columns(text=it.display_label, width=lbl_w)
            bdg = DisplayWidthFormatter.fit_columns(text=it.badge, width=bdg_w)
            dsc = DisplayWidthFormatter.fit_columns(text=it.description, width=dsc_w)
            row_fit = DisplayWidthFormatter.fit_columns(text=f" {ptr} {lbl} [{bdg}] {dsc} ", width=inner_w)
            rows.append(f"\033[38;5;141m│\033[0m{'\033[48;5;236;1;38;5;117m' if idx == sel else '\033[38;5;252m'}{row_fit}\033[0m\033[38;5;141m│\033[0m" if use_ansi else f"│{row_fit}│")
        rows.append(f"\033[38;5;141m{bot_b}\033[0m" if use_ansi else bot_b)
        return rows

    def render_codex_prompt_bar(
        self, buffer: str, ghost_text: str = "", active_model: str = "gemini-4-argon-high",
        dag_mode: bool = False, use_ansi: bool = True, term_cols: int = 96, badge_override: str | None = None,
    ) -> list[str]:
        """Render the spacious 3-row dark-gray Codex input composer box and 2-line status footer.

        Args:
            buffer: Current user input buffer.
            ghost_text: Optional inline ghost completion string.
            active_model: Currently selected model identifier.
            dag_mode: Whether `D A G - H E T E R A R C H Y` mode is active.
            use_ansi: Whether to emit ANSI color/background codes.
            term_cols: Terminal width in columns.
            badge_override: Optional pre-styled center badge string for shimmer animations.

        Returns:
            Five-line list: `[box_top, body_line_1, body_line_2, status_line_1, status_line_2]`.
        """
        safe_cols, placeholder = max(68, term_cols - 6), "Ask Heterarchy Codex to do anything"
        line_cap = max(40, safe_cols - 6)
        buf_1 = DisplayWidthFormatter.fit_columns(text=buffer, width=line_cap, pad=False) if DisplayWidthFormatter.text_width(text=buffer) > line_cap else buffer
        buf_2 = DisplayWidthFormatter.fit_columns(text=buffer[len(buf_1):], width=line_cap, pad=False) if len(buf_1) < len(buffer) else ""
        used_cols = 4 + DisplayWidthFormatter.text_width(text=(buf_2 or buf_1) or placeholder)
        ghost_fit = DisplayWidthFormatter.fit_columns(text=ghost_text, width=max(0, safe_cols - used_cols - 2), pad=False) if buffer else ""
        extra_cur = 1 if (buffer and not ghost_fit) else 0
        pad_1 = " " * max(1, safe_cols - (4 + DisplayWidthFormatter.text_width(text=buf_1 or placeholder) + (0 if buf_2 else DisplayWidthFormatter.text_width(text=ghost_fit) + extra_cur)))
        pad_2 = " " * max(1, safe_cols - (4 + DisplayWidthFormatter.text_width(text=buf_2) + (DisplayWidthFormatter.text_width(text=ghost_fit) + extra_cur if buf_2 else 0)))
        pad_empty = " " * max(1, safe_cols - 1)
        if not use_ansi:
            box_top = f"▌{pad_empty}"
            body_1 = f"▌ \u203a {buf_1}{'' if buf_2 else f'█{ghost_fit}'}{pad_1}" if buffer else f"▌ \u203a █{placeholder}{pad_1}"
            body_2 = f"▌   {buf_2}█{ghost_fit}{pad_2}" if buf_2 else box_top
        else:
            box_top = f"{TuiAnsiStyle.BG_BAR}\033[38;5;141m▌{TuiAnsiStyle.BG_BAR}{pad_empty}\033[K{TuiAnsiStyle.RESET}"
            cur_part = f"\033[48;5;255;1;38;5;232m{ghost_fit[0]}{TuiAnsiStyle.FG_PLACEHOLDER}{ghost_fit[1:]}" if ghost_fit else str(TuiAnsiStyle.CURSOR_BLOCK)
            styled_1 = _SKILL_TOKEN_RE.sub(f"{TuiAnsiStyle.FG_CYAN_SKILL}\\1{TuiAnsiStyle.FG_TEXT}", buf_1)
            styled_2 = _SKILL_TOKEN_RE.sub(f"{TuiAnsiStyle.FG_CYAN_SKILL}\\1{TuiAnsiStyle.FG_TEXT}", buf_2)
            body_1 = f"{TuiAnsiStyle.BG_BAR}\033[38;5;141m▌\033[38;5;183m \u203a \033[48;5;255;1;38;5;232m{placeholder[0]}{TuiAnsiStyle.FG_PLACEHOLDER}{placeholder[1:]}{pad_1}\033[K{TuiAnsiStyle.RESET}" if not buffer else f"{TuiAnsiStyle.BG_BAR}\033[38;5;141m▌\033[38;5;183m \u203a {TuiAnsiStyle.FG_TEXT}{styled_1}{'' if buf_2 else cur_part}{TuiAnsiStyle.BG_BAR}{pad_1}\033[K{TuiAnsiStyle.RESET}"
            body_2 = f"{TuiAnsiStyle.BG_BAR}\033[38;5;141m▌   {TuiAnsiStyle.FG_TEXT}{styled_2}{cur_part}{TuiAnsiStyle.BG_BAR}{pad_2}\033[K{TuiAnsiStyle.RESET}" if buf_2 else box_top

        used_toks = self.tokens_in + self.tokens_out
        ctx_pct = max(0, min(100, round((1.0 - used_toks / max(1, self.context_limit)) * 100)))
        ctx_meta = f"Context {ctx_pct}% left · {self.tokens_in:,} in · {self.tokens_out:,} out"
        raw_tokens = " ".join(self.skill_registry.skills[s].dollar_token if s in self.skill_registry.skills else f"${s}" for s in self.skill_registry.active_skills) or "(no skills)"
        active_tokens = DisplayWidthFormatter.fit_columns(text=raw_tokens, width=max(16, safe_cols - len(active_model) - len(self.git_branch) - len(ctx_meta) - 12), pad=False)
        status_1 = f" \033[38;5;248m{active_model}\033[0m · \033[38;5;81m{self.git_branch}\033[0m · \033[38;5;244m{ctx_meta} · \033[38;5;211m{active_tokens}\033[0m" if use_ansi else f" {active_model} · {self.git_branch} · {ctx_meta} · {active_tokens}"
        left_hint, right_warn = f" {self.omh_status_badge} · ? shortcuts", "▲ 0 warnings · f2 to view"
        center_badge = "D A G - H E T E R A R C H Y" if (dag_mode or badge_override is not None) else ""
        mid_space = max(2, safe_cols - len(left_hint) - len(right_warn))
        l_pad = max(1, (mid_space - len(center_badge)) // 2) if center_badge else 0
        styled_center = badge_override if badge_override is not None else (f"\033[1;38;5;141m{center_badge}\033[0m" if use_ansi else center_badge)
        mid_seg = f"{' ' * l_pad}{styled_center}{' ' * max(1, mid_space - len(center_badge) - l_pad)}" if center_badge else " " * mid_space
        omh_col = "\033[1;38;5;48m" if "done" in self.omh_status_badge else ("\033[1;38;5;214m" if "running" in self.omh_status_badge else "\033[38;5;215m")
        status_2 = f" {omh_col}{self.omh_status_badge}\033[0;38;5;244m · ? shortcuts\033[0m{mid_seg}\033[38;5;214m▲\033[38;5;244m 0 warnings · \033[1;38;5;250mf2\033[0;38;5;244m to view\033[0m" if use_ansi else f"{left_hint}{mid_seg}{right_warn}"
        return [box_top, body_1, body_2, status_1, status_2]


class TuiInteractivePromptReader:
    """Real-time cbreak-mode terminal prompt reader with Codex-style dark bar and live dropdown."""

    def __init__(self, engine: TuiSuggestionEngine, use_ansi: bool = True) -> None:
        """Initialize the interactive prompt reader.

        Args:
            engine: `TuiSuggestionEngine` providing live suggestions.
            use_ansi: Whether to emit ANSI color/cursor sequences.
        """
        self.engine: TuiSuggestionEngine = engine
        self.use_ansi: bool = use_ansi
        self.history: list[str] = []
        self.active_model: str = "gemini-4-argon-high"
        self.dag_mode: bool = False
        self.viewport_lines: dict[int, str] = {}
        self._prev_dropdown_len: int = 0

    def read_line(self, prompt_text: str) -> str:
        """Read a single line from stdin, using live cbreak dropdown UI when connected to a TTY.

        Args:
            prompt_text: Prompt prefix string rendered before the cursor.

        Returns:
            Committed line string entered by the user.
        """
        if not sys.stdin.isatty() or not sys.stdout.isatty():
            return input(prompt_text)
        try:
            fd = sys.stdin.fileno()
            old_attrs = termios.tcgetattr(fd)
        except OSError, termios.error:
            return input(prompt_text)

        try:
            tty.setcbreak(fd)
            if self.use_ansi:
                sys.stdout.write("\033[?25l")
                sys.stdout.flush()
            line = self._read_tty_loop(fd=fd)
        finally:
            if self.use_ansi:
                sys.stdout.write("\033[?25h")
                sys.stdout.flush()
            termios.tcsetattr(fd, termios.TCSADRAIN, old_attrs)
        if line.strip() and (not self.history or self.history[-1] != line):
            self.history.append(line)
        return line

    def play_dag_mode_shimmer(self, entering: bool) -> None:
        """Animate the centered `D A G - H E T E R A R C H Y` badge in-place on the bottom bar.

        Args:
            entering: True when activating `dag-mode`, False when exiting via `/dag-mode exit`.
        """
        if not (self.use_ansi and sys.stdout.isatty()):
            return
        badge = "D A G - H E T E R A R C H Y"
        term_sz = shutil.get_terminal_size(fallback=(120, 32))
        term_cols, bar_row = max(80, term_sz.columns), max(24, term_sz.lines) - 4
        fade_cols = ("\033[1;38;5;141m", "\033[38;5;141m", "\033[38;5;97m", "\033[38;5;60m", "\033[38;5;240m", "\033[38;5;236m")
        for step in range(14 if entering else 6):
            if entering:
                styled = "".join(f"{'\033[1;38;5;87m' if abs(i - step * 2) <= 1 else ('\033[1;38;5;183m' if abs(i - step * 2) <= 3 else '\033[1;38;5;141m')}{ch}" for i, ch in enumerate(badge)) + "\033[0m"
            else:
                styled = f"{fade_cols[min(step, len(fade_cols) - 1)]}{badge}\033[0m"
            bar = self.engine.render_codex_prompt_bar(buffer="", active_model=self.active_model, dag_mode=entering, use_ansi=True, term_cols=term_cols, badge_override=styled)
            sys.stdout.write("".join(f"\033[{bar_row + i};1H\033[2K{ln}" for i, ln in enumerate(bar)) + f"\033[{bar_row + 1};5H")
            sys.stdout.flush()
            select.select((), (), (), 0.04)

    def _read_tty_loop(self, fd: int) -> str:
        """Run the character-by-character live suggestion and editing loop.

        Args:
            fd: Standard input file descriptor in cbreak mode.

        Returns:
            Final committed input string.
        """
        buffer, cursor_pos, selected_idx, hist_idx = "", 0, 0, len(self.history)
        force_open, dismissed, user_navigated = False, False, False

        while True:
            empty_items: list[TuiSuggestionItem] = []
            items, replace_start, _ = (empty_items, 0, "") if dismissed else self.engine.suggest(buffer=buffer, cursor_pos=cursor_pos, force_open=force_open)
            selected_idx = 0 if selected_idx >= len(items) else selected_idx
            self._render_frame(buffer=buffer, cursor_pos=cursor_pos, items=items, selected_idx=selected_idx, replace_start=replace_start)
            key = self._read_key_sequence(fd=fd)
            if key == "" or (key == "\x04" and not buffer):
                self._clear_frame_and_commit()
                raise EOFError
            if key == "\x03":
                self._clear_frame_and_commit()
                raise KeyboardInterrupt
            if key in ("\r", "\n"):
                if select.select((fd,), (), (), 0.005)[0]:
                    if buffer and not buffer.endswith(" "):
                        buffer, cursor_pos = buffer[:cursor_pos] + " " + buffer[cursor_pos:], cursor_pos + 1
                    continue
                clean_b = buffer.strip()
                single_trig = clean_b.startswith(("/", "$")) and (" " not in clean_b or clean_b.lower().startswith(("/use ", "/dag-mode ", "/skill "))) and clean_b.lower() not in _EXACT_SUBMIT_COMMANDS
                if items and (user_navigated or single_trig):
                    chosen = items[selected_idx]
                    if chosen.submit_on_enter:
                        self._clear_frame_and_commit()
                        return chosen.insert_text.strip()
                    buffer, cursor_pos = self.engine.apply_completion(buffer=buffer, cursor_pos=cursor_pos, item=chosen, replace_start=replace_start)
                    selected_idx, user_navigated, force_open = 0, False, False
                    continue
                self._clear_frame_and_commit()
                return buffer
            if key == "\t":
                if not items:
                    force_open, dismissed = True, False
                else:
                    buffer, cursor_pos = self.engine.apply_completion(buffer=buffer, cursor_pos=cursor_pos, item=items[selected_idx], replace_start=replace_start)
                    selected_idx, user_navigated, force_open = 0, False, buffer.endswith(("/use ", "/skill ", "/dag-mode "))
            elif key in ("\x1b[B", "\x1bOB", "\x0e"):
                force_open, dismissed, selected_idx, user_navigated = (True, False, 0, False) if not items else (force_open, dismissed, (selected_idx + 1) % len(items), True)
            elif key in ("\x1b[A", "\x1bOA", "\x1b[Z", "\x10"):
                if items:
                    selected_idx, user_navigated = (selected_idx - 1) % len(items), True
                elif self.history and hist_idx > 0:
                    hist_idx -= 1
                    buffer, cursor_pos = self.history[hist_idx], len(self.history[hist_idx])
            elif key in ("\x1b[C", "\x1bOC"):
                if cursor_pos < len(buffer):
                    cursor_pos += 1
                elif items:
                    buffer, cursor_pos = self.engine.apply_completion(buffer=buffer, cursor_pos=cursor_pos, item=items[selected_idx], replace_start=replace_start)
                    selected_idx, user_navigated = 0, False
            elif key in ("\x1b[D", "\x1bOD"):
                cursor_pos = max(0, cursor_pos - 1)
            elif key == "\x1b":
                if items:
                    dismissed, force_open, user_navigated = True, False, False
                elif buffer:
                    buffer, cursor_pos = "", 0
                else:
                    self._clear_frame_and_commit()
                    return "/back"
            elif key in ("\x7f", "\x08"):
                if cursor_pos > 0:
                    buffer, cursor_pos = buffer[: cursor_pos - 1] + buffer[cursor_pos:], cursor_pos - 1
                selected_idx, dismissed, user_navigated, force_open = 0, False, False, False
            elif key == "\x15":
                buffer, cursor_pos, selected_idx, dismissed, user_navigated, force_open = "", 0, 0, False, False, False
            elif key == "\x17":
                cut = buffer[:cursor_pos].rstrip(" ").rfind(" ") + 1
                buffer, cursor_pos, selected_idx, dismissed, user_navigated = buffer[:cut] + buffer[cursor_pos:], cut, 0, False, False
            elif not key.startswith("\x1b") and all(ord(ch) >= 32 for ch in key):
                buffer, cursor_pos = buffer[:cursor_pos] + key + buffer[cursor_pos:], cursor_pos + len(key)
                selected_idx, dismissed, user_navigated, force_open = 0, False, False, False

    def _read_key_sequence(self, fd: int) -> str:
        """Read a single UTF-8 character or ANSI escape sequence from `fd`.

        Args:
            fd: Standard input file descriptor.

        Returns:
            Decoded character or escape sequence string.
        """
        raw = os.read(fd, 1)
        if not raw:
            return ""
        b0 = raw[0]
        if b0 == 0x1B:
            if not select.select((fd,), (), (), 0.03)[0]:
                return "\x1b"
            seq = bytearray(raw)
            while select.select((fd,), (), (), 0.01)[0] and (nxt := os.read(fd, 1)):
                seq.extend(nxt)
                if 64 <= nxt[0] <= 126 and len(seq) >= 3:
                    break
            return bytes(seq).decode("utf-8", errors="ignore")
        needed = 1 if 0xC0 <= b0 < 0xE0 else (2 if 0xE0 <= b0 < 0xF0 else (3 if 0xF0 <= b0 < 0xF8 else 0))
        buf = bytearray(raw)
        while needed > 0 and (chunk := os.read(fd, needed)):
            buf.extend(chunk)
            needed -= len(chunk)
        return bytes(buf).decode("utf-8", errors="replace")

    def _render_frame(self, buffer: str, cursor_pos: int, items: list[TuiSuggestionItem], selected_idx: int, replace_start: int) -> None:
        """Render the bottom-docked Codex input bar, footer, and floating popup above the bar.

        Args:
            buffer: Current user input buffer.
            cursor_pos: Current cursor character index.
            items: Active recommendation items.
            selected_idx: Currently highlighted item index.
            replace_start: Start character offset of the token being completed.
        """
        term_sz = shutil.get_terminal_size(fallback=(120, 32))
        term_cols, bar_row = max(80, term_sz.columns), max(24, term_sz.lines) - 4
        ghost = ""
        if buffer and items and cursor_pos == len(buffer):
            top, typed_tok = items[selected_idx], buffer[replace_start:cursor_pos]
            ghost = f"{top.insert_text[len(typed_tok):]}  [{top.badge}: {top.description[:26]}]" if top.insert_text.lower().startswith(typed_tok.lower()) else f"  ➔ {top.display_label} ({top.description[:24]})"
        bar = self.engine.render_codex_prompt_bar(buffer=buffer, ghost_text=ghost, active_model=self.active_model, dag_mode=self.dag_mode, use_ansi=self.use_ansi, term_cols=term_cols)
        dropdown_rows = self.engine.render_dropdown_box(items=items, selected_index=selected_idx, max_visible=7, use_ansi=self.use_ansi, term_cols=term_cols)
        out: list[str] = [
            *(f"\033[{r};1H\033[2K{self.viewport_lines.get(r, '')}" for r in range(max(4, bar_row - self._prev_dropdown_len), max(4, bar_row - len(dropdown_rows))) if self._prev_dropdown_len > len(dropdown_rows)),
            *(f"\033[{max(4, bar_row - len(dropdown_rows)) + i};1H\033[2K {d}" for i, d in enumerate(dropdown_rows)),
            *(f"\033[{bar_row + i};1H\033[2K{ln}" for i, ln in enumerate(bar)),
            f"\033[{bar_row + 1};{5 + DisplayWidthFormatter.text_width(text=buffer[:cursor_pos])}H",
        ]
        self._prev_dropdown_len = len(dropdown_rows)
        sys.stdout.write("".join(out))
        sys.stdout.flush()

    def _clear_frame_and_commit(self) -> None:
        """Erase the floating popup above the bottom bar before executing the committed input."""
        bar_row = max(24, shutil.get_terminal_size(fallback=(120, 32)).lines) - 4
        sys.stdout.write("".join(f"\033[{r};1H\033[2K{self.viewport_lines.get(r, '')}" for r in range(max(4, bar_row - self._prev_dropdown_len), bar_row)))
        self._prev_dropdown_len = 0
        sys.stdout.flush()


__all__ = ["TuiInteractivePromptReader", "TuiSuggestionEngine", "TuiSuggestionItem"]
