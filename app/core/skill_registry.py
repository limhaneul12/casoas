"""Skill discovery, frontmatter parser, and prompt context injector for `hcodex tui`."""

from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass
from pathlib import Path

from app.core.dag_enums import DefaultCoreSkill, SkillScopeKind

_ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


class DisplayWidthFormatter:
    """East-Asian (Korean/CJK) aware terminal display column calculator and formatter."""

    @staticmethod
    def char_width(ch: str) -> int:
        """Return terminal column width (`0`, `1`, or `2`) for a single Unicode character.

        Args:
            ch: Single Unicode character.

        Returns:
            Number of terminal cells occupied by `ch`.
        """
        if not ch:
            return 0
        code = ord(ch)
        if code < 128:
            return 1 if code >= 32 else 0
        if unicodedata.combining(ch):
            return 0
        return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1

    @classmethod
    def text_width(cls, text: str) -> int:
        """Compute total terminal column width of a plain string.

        Args:
            text: Plain string without ANSI escape sequences.

        Returns:
            Total display width in terminal columns.
        """
        return sum(cls.char_width(ch=ch) for ch in text)

    @classmethod
    def visible_width(cls, ansi_text: str) -> int:
        """Compute terminal column width after stripping ANSI SGR escape sequences.

        Args:
            ansi_text: String potentially containing ANSI escape sequences.

        Returns:
            Visible terminal column width.
        """
        clean = _ANSI_ESCAPE_RE.sub("", ansi_text)
        return cls.text_width(text=clean)

    @classmethod
    def fit_columns(cls, text: str, width: int, pad: bool = True) -> str:
        """Truncate or right-pad `text` to fit exact terminal display `width`.

        Args:
            text: Plain string to fit.
            width: Target terminal display columns.
            pad: Whether to right-pad with spaces up to `width`.

        Returns:
            Formatted string occupying at most (or exactly, if `pad=True`) `width` columns.
        """
        if width <= 0:
            return ""
        out_chars: list[str] = []
        used = 0
        for ch in text:
            w = cls.char_width(ch=ch)
            if used + w > width:
                if width - used >= 1 and pad:
                    out_chars.append(" ")
                    used += 1
                break
            out_chars.append(ch)
            used += w
        if pad and used < width:
            out_chars.append(" " * (width - used))
        return "".join(out_chars)

    @classmethod
    def fit_ansi_line(cls, ansi_text: str, max_cols: int) -> str:
        """Clip an ANSI-styled single-line string so its visible display width never exceeds `max_cols`.

        Args:
            ansi_text: Single-line string that may contain ANSI SGR escape sequences.
            max_cols: Maximum allowed visible terminal columns.

        Returns:
            Clipped string guaranteed to occupy at most `max_cols` visible columns.
        """
        if max_cols <= 0:
            return ansi_text
        out: list[str] = []
        used, idx, n = 0, 0, len(ansi_text)
        has_ansi = "\x1b[" in ansi_text
        limit = max(1, max_cols - 1)
        clipped = False
        while idx < n:
            if ansi_text[idx] == "\x1b":
                m = _ANSI_ESCAPE_RE.match(ansi_text, idx)
                if m:
                    out.append(m.group(0))
                    idx = m.end()
                    continue
            ch = ansi_text[idx]
            w = cls.char_width(ch=ch)
            if used + w > limit:
                rest_vis = cls.visible_width(ansi_text=ansi_text[idx:])
                if used + rest_vis <= max_cols:
                    out.append(ansi_text[idx:])
                    break
                out.append("…")
                clipped = True
                break
            out.append(ch)
            used += w
            idx += 1
        if not clipped:
            return ansi_text
        if has_ansi:
            out.append("\x1b[0m")
        return "".join(out)


@dataclass(frozen=True, slots=True)
class TuiSkillEntry:
    """Metadata and file location for a discovered `SKILL.md` customization."""

    name: str
    scope: str
    description: str
    skill_md_path: Path
    plugin_name: str = ""

    @property
    def dollar_token(self) -> str:
        """Return the Codex-style `$skill` or `$plugin:skill` token for this entry.

        Returns:
            Formatted `$skill` or `$plugin:skill` token string.
        """
        if self.plugin_name:
            return f"${self.plugin_name}:{self.name}"
        return f"${self.name}"


def _parse_skill_frontmatter(skill_md: Path, default_name: str) -> tuple[str, str]:
    """Extract `(name, description)` from `SKILL.md` YAML frontmatter.

    Args:
        skill_md: Path to `SKILL.md`.
        default_name: Fallback directory name if frontmatter omits `name`.

    Returns:
        Tuple of `(skill_name, single_line_description)`.
    """
    try:
        text = skill_md.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return default_name, ""
    name_val = default_name
    desc_val = ""
    if text.startswith("---"):
        parts = text.split("---", 2)
        if len(parts) >= 3:
            for raw_line in parts[1].splitlines():
                line = raw_line.strip()
                if line.startswith("name:"):
                    cand = line.removeprefix("name:").strip().strip("\"'")
                    if cand:
                        name_val = cand
                elif line.startswith("description:"):
                    cand_d = line.removeprefix("description:").strip().strip("\"'")
                    if cand_d and cand_d not in (">", "|", ">-", "|-"):
                        desc_val = cand_d
    if not desc_val:
        for raw_line in text.splitlines():
            stripped = raw_line.strip()
            if stripped and not stripped.startswith(("---", "#", "name:", "description:")):
                desc_val = stripped
                break
    return name_val, desc_val[:72]


class TuiSkillRegistry:
    """Discover, toggle, and inject `SKILL.md` instructions inside `hcodex tui`."""

    def __init__(self, workspace_dir: str = ".", home_dir: Path | None = None) -> None:
        """Initialize the skill registry by scanning global, plugin, harness, and builtin roots.

        Args:
            workspace_dir: Active workspace directory path.
            home_dir: Optional user home directory override.
        """
        self.workspace_root: Path = Path(workspace_dir).resolve()
        self.home_root: Path = home_dir or Path.home()
        self._excerpt_cache: dict[tuple[str, int], str] = {}
        self.skills: dict[str, TuiSkillEntry] = self.discover_all_skills()
        default_skill = str(DefaultCoreSkill.CODEX_HETERARCHY)
        self.active_skills: list[str] = [default_skill]

    def discover_all_skills(self) -> dict[str, TuiSkillEntry]:
        """Scan global, plugin, workspace `.agents`, and builtin directories for `SKILL.md`.

        Returns:
            Sorted mapping of skill name to `TuiSkillEntry`.
        """
        found: dict[str, TuiSkillEntry] = {}
        roots: tuple[tuple[SkillScopeKind, Path, str], ...] = (
            (
                SkillScopeKind.PLUGIN,
                self.home_root / ".gemini" / "config" / "plugins",
                "*/skills/*/SKILL.md",
            ),
            (SkillScopeKind.GLOBAL, self.home_root / ".gemini" / "config" / "skills", "*/SKILL.md"),
            (SkillScopeKind.HARNESS, self.workspace_root / ".agents", "**/skills/*/SKILL.md"),
            (
                SkillScopeKind.BUILTIN,
                self.home_root / ".gemini" / "antigravity" / "builtin" / "skills",
                "*/SKILL.md",
            ),
        )
        for scope, base_dir, pattern in roots:
            if not base_dir.is_dir():
                continue
            for md_path in sorted(base_dir.glob(pattern)):
                if not md_path.is_file():
                    continue
                dir_name = md_path.parent.name
                plugin_name = (
                    md_path.parent.parent.parent.name if scope == SkillScopeKind.PLUGIN else ""
                )
                parsed_name, desc = _parse_skill_frontmatter(
                    skill_md=md_path, default_name=dir_name
                )
                norm_key = (parsed_name or dir_name).strip().lower()
                if plugin_name == norm_key or norm_key in tuple(DefaultCoreSkill):
                    plugin_name = ""
                if norm_key and norm_key not in found:
                    found[norm_key] = TuiSkillEntry(
                        name=norm_key,
                        scope=str(scope),
                        description=desc,
                        skill_md_path=md_path,
                        plugin_name=plugin_name,
                    )
        return dict(sorted(found.items()))

    def _resolve_skill_key(self, candidate: str) -> str | None:
        """Resolve a raw skill identifier (`name` or `plugin:name`) to its canonical key.

        Args:
            candidate: Lowercase candidate token without leading `/` or `$`.

        Returns:
            Canonical skill key in `self.skills`, or None if not found.
        """
        if candidate in self.skills:
            return candidate
        if ":" in candidate:
            plug_part, skill_part = candidate.split(":", 1)
            entry = self.skills.get(skill_part)
            if entry is not None and entry.plugin_name.lower() == plug_part:
                return skill_part
            if skill_part in self.skills:
                return skill_part
        return None

    def extract_inline_skill_mentions(self, raw_line: str) -> tuple[list[str], str]:
        """Extract `$skill`, `$plugin:skill`, or `/<skill>` mentions and activate them.

        Args:
            raw_line: Raw user input line from the TUI prompt.

        Returns:
            Tuple of `(matched_skill_names, remaining_text_without_skill_tokens)`.
        """
        tokens = raw_line.strip().split()
        matched: list[str] = []
        kept: list[str] = []
        for tok in tokens:
            if tok.startswith(("/", "$")) and len(tok) > 1:
                resolved = self._resolve_skill_key(candidate=tok[1:].lower())
                if resolved is not None:
                    matched.append(resolved)
                    if resolved not in self.active_skills:
                        self.active_skills.append(resolved)
                    continue
            kept.append(tok)
        return matched, " ".join(kept).strip()

    def handle_skill_command(self, arg: str) -> str:
        """Handle `/skill <subcommand_or_name>` to toggle, reset, or clear active skills.

        Args:
            arg: Argument string passed after `/skill`.

        Returns:
            Human-readable status message.
        """
        cleaned = arg.strip().lower()
        if not cleaned or cleaned == "list":
            return self.render_skills_table()
        if cleaned in ("clear", "none", "off", "reset"):
            self.active_skills.clear()
            return "○ 모든 활성 스킬을 해제했습니다. (`/skill default` 또는 `/skill core`로 재활성화 가능)"
        if cleaned == "default":
            def_s = str(DefaultCoreSkill.CODEX_HETERARCHY)
            self.active_skills = [def_s] if def_s in self.skills else []
            return f"● 기본 스킬(`$codex-heterarchy`) 활성화 완료: {self.active_skills}"
        if cleaned == "core":
            self.active_skills = [str(s) for s in DefaultCoreSkill if str(s) in self.skills]
            return f"● 기본 핵심 스킬 활성화 완료: {self.active_skills}"
        targets = [t.lstrip("/$") for t in cleaned.split() if t.strip()]
        toggled_on: list[str] = []
        toggled_off: list[str] = []
        unknown: list[str] = []
        for t in targets:
            resolved = self._resolve_skill_key(candidate=t)
            if resolved is None:
                unknown.append(t)
            elif resolved in self.active_skills:
                self.active_skills.remove(resolved)
                toggled_off.append(resolved)
            else:
                self.active_skills.append(resolved)
                toggled_on.append(resolved)
        parts: list[str] = []
        if toggled_on:
            parts.append(f"● 활성화: {toggled_on}")
        if toggled_off:
            parts.append(f"○ 비활성화: {toggled_off}")
        if unknown:
            parts.append(f"▲ 알 수 없는 스킬: {unknown} (`/skills`로 목록 확인)")
        parts.append(f"현재 활성 스킬 ({len(self.active_skills)}개): {self.active_skills}")
        return " | ".join(parts)

    def build_active_skills_context(self, max_bytes_per_skill: int = 16_000) -> str:
        """Build a bounded prompt context block containing active `SKILL.md` instructions.

        Args:
            max_bytes_per_skill: Maximum UTF-8 byte excerpt per active skill.

        Returns:
            Formatted Markdown string with active skill instructions, or empty string.
        """
        if not self.active_skills:
            return ""
        blocks: list[str] = [f"=== ACTIVE REGISTERED SKILLS ({', '.join(self.active_skills)}) ==="]
        for s_name in self.active_skills:
            entry = self.skills.get(s_name)
            if entry is None:
                continue
            cache_key = (s_name, max_bytes_per_skill)
            cached = self._excerpt_cache.get(cache_key)
            if cached is None:
                try:
                    raw_md = entry.skill_md_path.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                excerpt = raw_md.encode("utf-8")[:max_bytes_per_skill].decode(
                    "utf-8", errors="ignore"
                )
                cached = f"### Skill: `{entry.name}` ({entry.scope})\n{excerpt}"
                self._excerpt_cache[cache_key] = cached
            blocks.append(cached)
        return "\n\n".join(blocks)

    def render_skills_table(self) -> str:
        """Render the `oh-my-hermes`-style Registered Skills table.

        Returns:
            Multi-line box-drawing table of discovered and active skills.
        """
        lines: list[str] = [
            f"=== ☤ Registered Skills Catalog (active={len(self.active_skills)}/{len(self.skills)}) ===",
            "┌────────────┬────────────────────────────────────────┬──────────┬───────────────────────────────────┐",
            "│ State      │ Skill Token ($skill or /skill)         │ Scope    │ Description                       │",
            "├────────────┼────────────────────────────────────────┼──────────┼───────────────────────────────────┤",
        ]
        scope_order = tuple(str(s) for s in SkillScopeKind)
        ordered = sorted(
            self.skills.items(),
            key=lambda kv: (
                kv[0] not in self.active_skills,
                scope_order.index(kv[1].scope) if kv[1].scope in scope_order else 4,
                kv[0],
            ),
        )
        for name, entry in ordered:
            state = DisplayWidthFormatter.fit_columns(
                text="● ACTIVE" if name in self.active_skills else "○ READY", width=10
            )
            token_s = DisplayWidthFormatter.fit_columns(text=entry.dollar_token, width=38)
            scope_s = DisplayWidthFormatter.fit_columns(text=entry.scope, width=8)
            desc = DisplayWidthFormatter.fit_columns(
                text=entry.description or "(no description)", width=33
            )
            lines.append(f"│ {state} │ {token_s} │ {scope_s} │ {desc} │")
        lines.append(
            "└────────────┴────────────────────────────────────────┴──────────┴───────────────────────────────────┘"
        )
        lines.append(
            "Tip: `$` 또는 `/` 입력 시 실시간 추천 팝업이 뜹니다! (`↑↓` 이동, `Tab` 자동완성, `Enter` 실행)"
        )
        return "\n".join(lines)


__all__ = ["DisplayWidthFormatter", "TuiSkillEntry", "TuiSkillRegistry"]
