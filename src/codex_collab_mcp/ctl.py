"""CLI controller (`hcodex`) to toggle, restart, and inspect the Heterarchy Codex MCP."""

from __future__ import annotations

import asyncio
import contextlib
import os
import re
import shutil
import signal
import subprocess
import sys
import tomllib
from pathlib import Path

from codex_collab_mcp.collab_enums import CtlCommand, CtlRelativePath, JobStatusKind, TriadRole
from codex_collab_mcp.contracts import get_python_runtime_info
from codex_collab_mcp.handoff_store import HandoffStore
from codex_collab_mcp.job_store import JobStore
from codex_collab_mcp.transports.agy_rpc_transport import AntigravityRpcClient

REPO_ROOT: Path = Path(__file__).resolve().parents[2]


class _PluginEnvironmentManager:
    """Manages filesystem paths, skill synchronization, and plugin/MCP configuration."""

    def __init__(self, repo_root: Path = REPO_ROOT, home_dir: Path | None = None) -> None:
        """Initialize the plugin environment manager.

        Args:
            repo_root: Repository root directory.
            home_dir: Optional user home directory override.
        """
        self.repo_root: Path = repo_root
        self.home_dir: Path = home_dir or Path.home()

    def resolve_path(self, rel_path: CtlRelativePath) -> Path:
        """Resolve a CtlRelativePath enum member to its absolute Path.

        Args:
            rel_path: Relative path enum identifier.

        Returns:
            Resolved filesystem Path under repo_root or home_dir.
        """
        if rel_path in (
            CtlRelativePath.PLUGIN_SRC,
            CtlRelativePath.SKILL_SRC,
            CtlRelativePath.REPO_SKILL_SRC,
            CtlRelativePath.VENV_DIR,
        ):
            return self.repo_root / str(rel_path)
        return self.home_dir / str(rel_path)

    def find_running_mcp_pids(self) -> list[int]:
        """Return PIDs of currently running `codex_collab_mcp.server` stdio processes.

        Returns:
            List of active OS process IDs excluding the current process.
        """
        res = subprocess.run(
            ["pgrep", "-f", "codex_collab_mcp.server"],  # noqa: S607
            capture_output=True,
            text=True,
            check=False,
        )
        pids: list[int] = []
        my_pid = os.getpid()
        for line in res.stdout.splitlines():
            line_s = line.strip()
            if line_s.isdigit():
                pid = int(line_s)
                if pid != my_pid:
                    pids.append(pid)
        return pids

    def is_codex_mcp_enabled(self) -> bool:
        """Check whether `heterarchy_codex` is enabled in `.codex/config.toml`.

        Returns:
            True when the MCP server entry exists and is not disabled.
        """
        cfg_path = self.resolve_path(CtlRelativePath.CODEX_CONFIG_PATH)
        if not cfg_path.is_file():
            return False
        try:
            data = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
            servers = data.get("mcp_servers")
            if not isinstance(servers, dict):
                return False
            entry = servers.get("heterarchy_codex")
            if not isinstance(entry, dict):
                return False
            return entry.get("enabled") is not False
        except OSError, tomllib.TOMLDecodeError:
            return False

    def set_codex_mcp_enabled(self, enable: bool) -> None:
        """Enable or disable `heterarchy_codex` in `.codex/config.toml`.

        Args:
            enable: Desired enabled flag.
        """
        cfg_path = self.resolve_path(CtlRelativePath.CODEX_CONFIG_PATH)
        if not cfg_path.is_file():
            if not enable:
                return
            cfg_path.parent.mkdir(parents=True, exist_ok=True)
            content = ""
        else:
            content = cfg_path.read_text(encoding="utf-8")
        section_pattern = re.compile(
            r"(\[mcp_servers\.heterarchy_codex\]\r?\n(?:(?!\[).*(?:\r?\n|$))*)",
            re.MULTILINE,
        )
        match = section_pattern.search(content)
        if not match:
            if not enable:
                return
            codex_bin = shutil.which("codex") or "/opt/homebrew/bin/codex"
            block = (
                "\n[mcp_servers.heterarchy_codex]\n"
                f'command = "{self.repo_root}/.venv/bin/python3"\n'
                'args = ["-m", "codex_collab_mcp.server"]\n'
                "enabled = true\n"
                "\n[mcp_servers.heterarchy_codex.env]\n"
                f'PYTHONPATH = "{self.repo_root}/src"\n'
                f'CODEX_CLI_BIN = "{codex_bin}"\n'
            )
            prefix = (content.rstrip() + "\n") if content.strip() else ""
            cfg_path.write_text(prefix + block.lstrip("\n"), encoding="utf-8")
            return

        section_text = match.group(1)
        flag_str = "true" if enable else "false"
        if re.search(r"^enabled\s*=", section_text, flags=re.MULTILINE):
            updated_section = re.sub(
                r"^enabled\s*=.*$",
                f"enabled = {flag_str}",
                section_text,
                flags=re.MULTILINE,
            )
        else:
            updated_section = section_text.rstrip("\n") + f"\nenabled = {flag_str}\n"
        new_content = content[: match.start(1)] + updated_section + content[match.end(1) :]
        cfg_path.write_text(new_content, encoding="utf-8")

    def sync_skills(self) -> None:
        """Synchronize `codex-heterarchy` skills across Antigravity and Codex global roots."""
        skill_src = self.resolve_path(CtlRelativePath.SKILL_SRC)
        if not skill_src.is_dir():
            skill_src = self.resolve_path(CtlRelativePath.REPO_SKILL_SRC)
        if not skill_src.is_dir():
            return
        agy_skill = self.resolve_path(CtlRelativePath.AGY_GLOBAL_SKILL)
        agy_active = self.resolve_path(CtlRelativePath.AGY_PLUGIN_ACTIVE)
        codex_skill = self.resolve_path(CtlRelativePath.CODEX_GLOBAL_SKILL)

        for target in (agy_skill, codex_skill):
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.is_symlink() or target.exists():
                if target.is_dir() and not target.is_symlink():
                    shutil.rmtree(target)
                else:
                    target.unlink()

        target_plugin_skill = agy_active / "skills" / "codex-heterarchy"
        if target_plugin_skill.is_dir():
            agy_skill.symlink_to(target_plugin_skill)
        else:
            shutil.copytree(skill_src, agy_skill)
        shutil.copytree(skill_src, codex_skill)

    def set_agy_plugin_enabled(self, enable: bool) -> None:
        """Enable or disable the Antigravity `heterarchy-codex` plugin bundle.

        Args:
            enable: Desired enabled state.
        """
        venv_dir = self.resolve_path(CtlRelativePath.VENV_DIR)
        if enable and venv_dir.is_dir():
            subprocess.run(  # noqa: S603
                ["chflags", "-R", "nohidden", str(venv_dir)],  # noqa: S607
                capture_output=True,
                check=False,
            )
            for sp in venv_dir.glob("lib/python*/site-packages"):
                sc = sp / "sitecustomize.py"
                if not sc.is_file():
                    sc.write_text(
                        "import sys\nfrom pathlib import Path\n"
                        'sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "src"))\n',
                        encoding="utf-8",
                    )
        plugin_src = self.resolve_path(CtlRelativePath.PLUGIN_SRC)
        agy_active = self.resolve_path(CtlRelativePath.AGY_PLUGIN_ACTIVE)
        agy_disabled = self.resolve_path(CtlRelativePath.AGY_PLUGIN_DISABLED)
        if enable:
            agy_active.parent.mkdir(parents=True, exist_ok=True)
            if agy_active.exists():
                shutil.rmtree(agy_active)
            if plugin_src.is_dir():
                shutil.copytree(plugin_src, agy_active)
            else:
                agy_active.mkdir(parents=True, exist_ok=True)
                repo_skill = self.resolve_path(CtlRelativePath.REPO_SKILL_SRC)
                if repo_skill.is_dir():
                    shutil.copytree(repo_skill, agy_active / "skills" / "codex-heterarchy")
            codex_bin = shutil.which("codex") or "/opt/homebrew/bin/codex"
            mcp_cfg_text = (
                "{\n"
                '  "mcpServers": {\n'
                '    "heterarchy-codex": {\n'
                f'      "command": "{self.repo_root}/.venv/bin/python3",\n'
                '      "args": [\n'
                '        "-m",\n'
                '        "codex_collab_mcp.server"\n'
                "      ],\n"
                '      "env": {\n'
                f'        "PYTHONPATH": "{self.repo_root}/src",\n'
                f'        "CODEX_CLI_BIN": "{codex_bin}"\n'
                "      }\n"
                "    }\n"
                "  }\n"
                "}\n"
            )
            (agy_active / "mcp_config.json").write_text(mcp_cfg_text, encoding="utf-8")
            plugin_manifest = agy_active / "plugin.json"
            if not plugin_manifest.is_file():
                plugin_manifest.write_text(
                    "{\n"
                    '  "name": "heterarchy-codex",\n'
                    '  "displayName": "Heterarchy Tri-Polar Bridge (Argon <-> Claude <-> Codex)",\n'
                    '  "version": "0.2.0",\n'
                    '  "description": "Receipt-verified Tri-Polar Heterarchy collaboration bridge."\n'
                    "}\n",
                    encoding="utf-8",
                )
            if agy_disabled.exists():
                shutil.rmtree(agy_disabled)
            self.sync_skills()
        elif agy_active.exists():
            agy_disabled.parent.mkdir(parents=True, exist_ok=True)
            if agy_disabled.exists():
                shutil.rmtree(agy_disabled)
            shutil.move(str(agy_active), str(agy_disabled))


class HeterarchyCliController:
    """Executes user-facing CLI subcommands (`status`, `on`, `off`, `restart`, `inbox`)."""

    def __init__(self, env_mgr: _PluginEnvironmentManager | None = None) -> None:
        """Initialize the CLI controller.

        Args:
            env_mgr: Optional environment manager instance.
        """
        self.env: _PluginEnvironmentManager = env_mgr or _PluginEnvironmentManager()

    def cmd_status(self) -> int:
        """Print runtime, plugin, skill, job, and handoff status.

        Returns:
            Zero exit code.
        """
        agy_active = self.env.resolve_path(CtlRelativePath.AGY_PLUGIN_ACTIVE)
        agy_skill = self.env.resolve_path(CtlRelativePath.AGY_GLOBAL_SKILL)
        codex_skill = self.env.resolve_path(CtlRelativePath.CODEX_GLOBAL_SKILL)
        agy_on = agy_active.is_dir()
        codex_on = self.env.is_codex_mcp_enabled()
        agy_skill_ok = (agy_skill / "SKILL.md").is_file()
        codex_skill_ok = (codex_skill / "SKILL.md").is_file()
        pids = self.env.find_running_mcp_pids()
        h_store = HandoffStore()
        j_store = JobStore()
        roles = (TriadRole.ARGON, TriadRole.SONNET, TriadRole.OPUS, TriadRole.CODEX)
        pending = (
            h_store.count_pending_by_recipient(roles)
            if h_store.db_path.is_file()
            else {r: 0 for r in roles}
        )
        recent_jobs = j_store.list_jobs(limit=5) if j_store.db_path.is_file() else []
        running_jobs = sum(1 for j in recent_jobs if j.get("status") == JobStatusKind.RUNNING)
        rt = get_python_runtime_info()

        print("=== Heterarchy Tri-Polar MCP Status (Argon + Sonnet/Opus 5.5 + Codex) ===")
        print(
            f"  Python Runtime     : {rt['version']} "
            f"(free_threading={rt['free_threading_build']}, gil_enabled={rt['gil_enabled']})"
        )
        print(f"  Antigravity Plugin : {'ON (Enabled)' if agy_on else 'OFF (Disabled)'}")
        print(f"  GPT / Codex MCP    : {'ON (Enabled)' if codex_on else 'OFF (Disabled)'}")
        print(
            f"  Registered Skills  : Antigravity={'OK' if agy_skill_ok else 'MISSING'} "
            f"| Codex={'OK' if codex_skill_ok else 'MISSING'}"
        )
        print(f"  Active Stdio PIDs  : {pids if pids else 'None (spawns on demand)'}")
        print(f"  Running Async Jobs : {running_jobs}")
        print(
            f"  Handoff Inbox      : Argon={pending.get(TriadRole.ARGON, 0)} | "
            f"Sonnet={pending.get(TriadRole.SONNET, 0)} | "
            f"Opus={pending.get(TriadRole.OPUS, 0)} | "
            f"Codex={pending.get(TriadRole.CODEX, 0)} pending"
        )
        return 0

    def cmd_toggle(self, enable: bool) -> int:
        """Enable or disable plugin, skills, and MCP server for Antigravity and Codex.

        Args:
            enable: True to enable (`on`), False to disable and stop (`off`).

        Returns:
            Status command exit code.
        """
        self.env.set_agy_plugin_enabled(enable)
        self.env.set_codex_mcp_enabled(enable)
        if enable:
            print("[hcodex] Enabled Heterarchy Codex MCP & skills for Antigravity and GPT/Codex.")
        else:
            pids = self.env.find_running_mcp_pids()
            for pid in pids:
                with contextlib.suppress(ProcessLookupError):
                    os.kill(pid, signal.SIGTERM)
            print(f"[hcodex] Disabled Heterarchy Codex MCP (terminated PIDs: {pids or 'none'}).")
        return self.cmd_status()

    def cmd_prune(self, max_retained: int = 1000) -> int:
        """Prune old terminal SQLite records and truncate WAL journals for jobs and handoffs.

        Args:
            max_retained: Maximum number of newest records to retain per table.

        Returns:
            Zero exit code.
        """
        jobs_deleted = JobStore().prune_completed_jobs(
            max_retained=max_retained,
            wal_checkpoint=True,
        )
        tickets_deleted = HandoffStore().prune_resolved_tickets(
            max_retained=max_retained,
            wal_checkpoint=True,
        )
        print(
            f"[hcodex] Pruned SQLite WAL stores "
            f"(jobs_deleted={jobs_deleted}, handoffs_deleted={tickets_deleted}, "
            f"max_retained={max_retained})."
        )
        return 0

    def cmd_restart(self) -> int:
        """Terminate old MCP stdio PIDs, sync plugin/skills, and invoke RefreshMcpServers RPC.

        Returns:
            Status command exit code.
        """
        pids = self.env.find_running_mcp_pids()
        for pid in pids:
            with contextlib.suppress(ProcessLookupError):
                os.kill(pid, signal.SIGTERM)
        self.env.set_agy_plugin_enabled(True)
        self.env.set_codex_mcp_enabled(True)
        mcp_cfg = self.env.resolve_path(CtlRelativePath.AGY_PLUGIN_ACTIVE) / "mcp_config.json"
        if mcp_cfg.is_file():
            mcp_cfg.touch()
        with contextlib.suppress(Exception):
            ep = asyncio.run(AntigravityRpcClient.discover_endpoint())
            if ep is not None:
                AntigravityRpcClient.post_connect_rpc_sync(
                    ep.base_url, ep.csrf_token, "RefreshMcpServers", {}, 10.0
                )
        print(
            f"[hcodex] Restarted Heterarchy Codex MCP "
            f"(terminated old PIDs: {pids or 'none'}, refreshed LanguageServer MCP)."
        )
        return self.cmd_status()

    def cmd_inbox(self) -> int:
        """Display recent tickets from the HandoffStore inbox.

        Returns:
            Zero exit code.
        """
        tickets = HandoffStore().list_tickets(status=None, limit=15)
        if not tickets:
            print("[hcodex] Handoff inbox is empty.")
            return 0
        print(f"=== Recent Handoff Tickets ({len(tickets)}) ===")
        for t in tickets:
            print(
                f"  [{t['ticket_id']}] {t['sender']} -> {t['recipient']} "
                f"| status={t['status']} | priority={t['priority']} | {t['title']}"
            )
        return 0

    def dispatch(self, argv: list[str] | None = None) -> int:
        """Parse CLI arguments and dispatch to the matching subcommand.

        Args:
            argv: Optional argument list override.

        Returns:
            Exit code from the executed command.
        """
        args = argv if argv is not None else sys.argv[1:]
        sub = args[0].lower() if args else CtlCommand.STATUS.value
        if sub in (CtlCommand.STATUS, "st"):
            return self.cmd_status()
        if sub in (CtlCommand.ON, "enable", "start"):
            return self.cmd_toggle(True)
        if sub in (CtlCommand.OFF, "disable", "stop"):
            return self.cmd_toggle(False)
        if sub in (CtlCommand.RESTART, "reload"):
            return self.cmd_restart()
        if sub in (CtlCommand.INBOX, "tickets"):
            return self.cmd_inbox()
        if sub in (CtlCommand.PRUNE, "compact", "clean"):
            return self.cmd_prune()
        print("Usage: hcodex [status | on | off | restart | inbox | prune]")
        return 1


def main(argv: list[str] | None = None) -> int:
    """CLI entrypoint for `hcodex`.

    Args:
        argv: Optional command-line argument list.

    Returns:
        Process exit status code.
    """
    return HeterarchyCliController().dispatch(argv)


if __name__ == "__main__":
    raise SystemExit(main())
