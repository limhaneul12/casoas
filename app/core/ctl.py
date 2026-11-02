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

from app.clients.agy_rpc_transport import (
    AntigravityCircuitBreaker,
    AntigravityRpcClient,
)
from app.core.cli_tui import HeterarchyTuiSession
from app.core.collab_enums import CtlCommand, CtlRelativePath, TriadRole
from app.core.model_contract_enums import CodexBinaryCandidatePath
from app.repositories.handoff_store import HandoffStore
from app.repositories.job_store import JobStore
from app.routing.auth_redaction import get_python_runtime_info

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
        """Return PIDs of currently running `app.server` stdio processes.

        Returns:
            List of active OS process IDs excluding the current process.
        """
        res = subprocess.run(
            args=["pgrep", "-f", "app.server|codex_collab_mcp.server"],  # noqa: S607
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
        cfg_path = self.resolve_path(rel_path=CtlRelativePath.CODEX_CONFIG_PATH)
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
        cfg_path = self.resolve_path(rel_path=CtlRelativePath.CODEX_CONFIG_PATH)
        if not cfg_path.is_file():
            if not enable:
                return
            cfg_path.parent.mkdir(parents=True, exist_ok=True)
            content = ""
        else:
            content = cfg_path.read_text(encoding="utf-8")
        section_pattern = re.compile(
            pattern=r"(\[mcp_servers\.heterarchy_codex\]\r?\n(?:(?!\[).*(?:\r?\n|$))*)",
            flags=re.MULTILINE,
        )
        match = section_pattern.search(content)
        if not match:
            if not enable:
                return
            codex_bin = shutil.which("codex") or str(CodexBinaryCandidatePath.HOMEBREW_BIN)
            block = (
                "\n[mcp_servers.heterarchy_codex]\n"
                f'command = "{self.repo_root}/.venv/bin/python3"\n'
                'args = ["-m", "app.server"]\n'
                "enabled = true\n"
                "\n[mcp_servers.heterarchy_codex.env]\n"
                f'PYTHONPATH = "{self.repo_root}"\n'
                f'CODEX_CLI_BIN = "{codex_bin}"\n'
            )
            prefix = (content.rstrip() + "\n") if content.strip() else ""
            cfg_path.write_text(data=prefix + block.lstrip("\n"), encoding="utf-8")
            return

        section_text = match.group(1)
        flag_str = "true" if enable else "false"
        if re.search(pattern=r"^enabled\s*=", string=section_text, flags=re.MULTILINE):
            updated_section = re.sub(
                pattern=r"^enabled\s*=.*$",
                repl=f"enabled = {flag_str}",
                string=section_text,
                flags=re.MULTILINE,
            )
        else:
            updated_section = section_text.rstrip("\n") + f"\nenabled = {flag_str}\n"
        new_content = content[: match.start(1)] + updated_section + content[match.end(1) :]
        cfg_path.write_text(data=new_content, encoding="utf-8")

    def sync_skills(self) -> None:
        """Synchronize `codex-heterarchy` skills across Antigravity and Codex global roots."""
        skill_src = self.resolve_path(rel_path=CtlRelativePath.SKILL_SRC)
        if not skill_src.is_dir():
            skill_src = self.resolve_path(rel_path=CtlRelativePath.REPO_SKILL_SRC)
        if not skill_src.is_dir():
            return
        agy_skill = self.resolve_path(rel_path=CtlRelativePath.AGY_GLOBAL_SKILL)
        agy_active = self.resolve_path(rel_path=CtlRelativePath.AGY_PLUGIN_ACTIVE)
        codex_skill = self.resolve_path(rel_path=CtlRelativePath.CODEX_GLOBAL_SKILL)

        for target in (agy_skill, codex_skill):
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.is_symlink() or target.exists():
                if target.is_dir() and not target.is_symlink():
                    shutil.rmtree(path=target)
                else:
                    target.unlink()

        target_plugin_skill = agy_active / "skills" / "codex-heterarchy"
        if agy_active.is_dir():
            if target_plugin_skill.is_symlink() or target_plugin_skill.exists():
                if target_plugin_skill.is_dir() and not target_plugin_skill.is_symlink():
                    shutil.rmtree(path=target_plugin_skill)
                else:
                    target_plugin_skill.unlink()
            target_plugin_skill.parent.mkdir(parents=True, exist_ok=True)
            shutil.copytree(src=skill_src, dst=target_plugin_skill)
            agy_skill.symlink_to(target=target_plugin_skill)
        else:
            shutil.copytree(src=skill_src, dst=agy_skill)
        shutil.copytree(src=skill_src, dst=codex_skill)

    def set_agy_plugin_enabled(self, enable: bool) -> None:
        """Enable or disable the Antigravity `heterarchy-codex` plugin bundle.

        Args:
            enable: Desired enabled state.
        """
        venv_dir = self.resolve_path(rel_path=CtlRelativePath.VENV_DIR)
        if enable and venv_dir.is_dir():
            subprocess.run(  # noqa: S603
                args=["chflags", "-R", "nohidden", str(venv_dir)],  # noqa: S607
                capture_output=True,
                check=False,
            )
            for sp in venv_dir.glob("lib/python*/site-packages"):
                sc = sp / "sitecustomize.py"
                if not sc.is_file():
                    sc.write_text(
                        data=(
                            "import sys\nfrom pathlib import Path\n"
                            "sys.path.insert(0, str(Path(__file__).resolve().parents[4]))\n"
                        ),
                        encoding="utf-8",
                    )
        plugin_src = self.resolve_path(rel_path=CtlRelativePath.PLUGIN_SRC)
        agy_active = self.resolve_path(rel_path=CtlRelativePath.AGY_PLUGIN_ACTIVE)
        agy_disabled = self.resolve_path(rel_path=CtlRelativePath.AGY_PLUGIN_DISABLED)
        if enable:
            agy_active.parent.mkdir(parents=True, exist_ok=True)
            if agy_active.exists():
                shutil.rmtree(path=agy_active)
            if plugin_src.is_dir():
                shutil.copytree(src=plugin_src, dst=agy_active)
            else:
                agy_active.mkdir(parents=True, exist_ok=True)
                repo_skill = self.resolve_path(rel_path=CtlRelativePath.REPO_SKILL_SRC)
                if repo_skill.is_dir():
                    shutil.copytree(src=repo_skill, dst=agy_active / "skills" / "codex-heterarchy")
            codex_bin = shutil.which("codex") or str(CodexBinaryCandidatePath.HOMEBREW_BIN)
            mcp_cfg_text = (
                "{\n"
                '  "mcpServers": {\n'
                '    "heterarchy-codex": {\n'
                f'      "command": "{self.repo_root}/.venv/bin/python3",\n'
                '      "args": [\n'
                '        "-m",\n'
                '        "app.server"\n'
                "      ],\n"
                '      "env": {\n'
                f'        "PYTHONPATH": "{self.repo_root}",\n'
                f'        "CODEX_CLI_BIN": "{codex_bin}"\n'
                "      }\n"
                "    }\n"
                "  }\n"
                "}\n"
            )
            (agy_active / "mcp_config.json").write_text(data=mcp_cfg_text, encoding="utf-8")
            plugin_manifest = agy_active / "plugin.json"
            if not plugin_manifest.is_file():
                plugin_manifest.write_text(
                    data=(
                        "{\n"
                        '  "name": "heterarchy-codex",\n'
                        '  "displayName": "Heterarchy Tri-Polar Bridge (Argon <-> Claude <-> Codex)",\n'
                        '  "version": "0.2.0",\n'
                        '  "description": "Receipt-verified Tri-Polar Heterarchy collaboration bridge."\n'
                        "}\n"
                    ),
                    encoding="utf-8",
                )
            if agy_disabled.exists():
                shutil.rmtree(path=agy_disabled)
            self.sync_skills()
        elif agy_active.exists():
            agy_disabled.parent.mkdir(parents=True, exist_ok=True)
            if agy_disabled.exists():
                shutil.rmtree(path=agy_disabled)
            shutil.move(src=str(agy_active), dst=str(agy_disabled))


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
        agy_active = self.env.resolve_path(rel_path=CtlRelativePath.AGY_PLUGIN_ACTIVE)
        agy_skill = self.env.resolve_path(rel_path=CtlRelativePath.AGY_GLOBAL_SKILL)
        codex_skill = self.env.resolve_path(rel_path=CtlRelativePath.CODEX_GLOBAL_SKILL)
        agy_on = agy_active.is_dir()
        codex_on = self.env.is_codex_mcp_enabled()
        agy_skill_ok = (agy_skill / "SKILL.md").is_file()
        codex_skill_ok = (codex_skill / "SKILL.md").is_file()
        pids = self.env.find_running_mcp_pids()
        cache_root = self.env.home_dir / ".cache" / "codex-collab-mcp"
        h_store = HandoffStore(state_dir=cache_root / "handoffs")
        j_store = JobStore(state_dir=cache_root / "jobs")
        roles = (TriadRole.ARGON, TriadRole.SONNET, TriadRole.OPUS, TriadRole.CODEX)
        pending = (
            h_store.count_pending_by_recipient(recipients=roles)
            if h_store.db_path.is_file()
            else {r: 0 for r in roles}
        )
        telemetry = j_store.summarize_telemetry()
        running_jobs = telemetry["running_jobs"]
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
            f"  Job Telemetry      : total={telemetry['total_jobs']} | "
            f"completed={telemetry['completed_jobs']} | "
            f"failed={telemetry['failed_jobs']} | "
            f"timed_out={telemetry['timed_out_unknown_jobs']} | "
            f"avg={telemetry['avg_duration_seconds']}s"
        )
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
        self.env.set_agy_plugin_enabled(enable=enable)
        self.env.set_codex_mcp_enabled(enable=enable)
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
        cache_root = self.env.home_dir / ".cache" / "codex-collab-mcp"
        jobs_deleted = JobStore(state_dir=cache_root / "jobs").prune_completed_jobs(
            max_retained=max_retained,
            wal_checkpoint=True,
        )
        tickets_deleted = HandoffStore(state_dir=cache_root / "handoffs").prune_resolved_tickets(
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
        self.env.set_agy_plugin_enabled(enable=True)
        self.env.set_codex_mcp_enabled(enable=True)
        mcp_cfg = (
            self.env.resolve_path(rel_path=CtlRelativePath.AGY_PLUGIN_ACTIVE) / "mcp_config.json"
        )
        if mcp_cfg.is_file():
            mcp_cfg.touch()
        AntigravityCircuitBreaker.reset()
        with contextlib.suppress(Exception):
            ep = asyncio.run(AntigravityRpcClient.discover_endpoint())
            if ep is not None:
                AntigravityRpcClient.post_connect_rpc_sync(
                    base_url=ep.base_url,
                    csrf_token=ep.csrf_token,
                    method="RefreshMcpServers",
                    payload={},
                    timeout_seconds=10.0,
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
        cache_root = self.env.home_dir / ".cache" / "codex-collab-mcp"
        tickets = HandoffStore(state_dir=cache_root / "handoffs").list_tickets(
            status=None, limit=15
        )
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
        if sub in ("help", "-h", "--help"):
            return HeterarchyTuiSession().run_interactive_loop(initial_command="/help")
        if sub in (CtlCommand.STATUS, "st"):
            return self.cmd_status()
        if sub in (CtlCommand.ON, "enable", "start"):
            return self.cmd_toggle(enable=True)
        if sub in (CtlCommand.OFF, "disable", "stop"):
            return self.cmd_toggle(enable=False)
        if sub in (CtlCommand.RESTART, "reload"):
            return self.cmd_restart()
        if sub in (CtlCommand.INBOX, "tickets"):
            return self.cmd_inbox()
        if sub in ("jobs", "job"):
            return HeterarchyTuiSession().run_interactive_loop(initial_command="/jobs")
        if sub in (CtlCommand.PRUNE, "compact", "clean"):
            return self.cmd_prune()
        if sub in (CtlCommand.TUI, CtlCommand.CHAT):
            tail = args[2:] if len(args) > 2 and args[1] in ("-c", "--command") else args[1:]
            rest = " ".join(tail).strip() if tail else None
            return HeterarchyTuiSession().run_interactive_loop(initial_command=rest)
        if sub in ("dag-mode", "dag-exit"):
            rest_d = " ".join(args[1:]).strip()
            cmd_d = (
                "/dag-exit"
                if sub == "dag-exit"
                else (f"/dag-mode {rest_d}" if rest_d else "/dag-mode")
            )
            return HeterarchyTuiSession().run_interactive_loop(initial_command=cmd_d)
        if sub in ("ask", "prompt", "sonnet", "opus", "argon", "flash"):
            prompt_txt = " ".join(args[1:]).strip()
            slash = "/ask" if sub in ("ask", "prompt") else f"/{sub}"
            return HeterarchyTuiSession().run_interactive_loop(
                initial_command=f"{slash} {prompt_txt}"
            )
        if sub in (CtlCommand.DAG_ULW, "dag"):
            goal = " ".join(args[1:]).strip() or "Verify and refactor module contracts"
            return HeterarchyTuiSession().run_interactive_loop(initial_command=f"/dag-ulw {goal}")
        if sub in ("dag-run", "run"):
            goal = " ".join(args[1:]).strip() or "Verify and refactor module contracts"
            return HeterarchyTuiSession().run_interactive_loop(initial_command=f"/dag-run {goal}")
        if sub in (CtlCommand.MODELS, "lanes", "model"):
            return HeterarchyTuiSession().run_interactive_loop(initial_command="/models")
        if sub in ("skills", "skill"):
            rest_s = " ".join(args[1:]).strip()
            cmd_s = f"/skill {rest_s}" if rest_s else "/skills"
            return HeterarchyTuiSession().run_interactive_loop(initial_command=cmd_s)
        print(
            "Usage: hcodex [tui | skills | ask <prompt> | sonnet <prompt> | opus <prompt> | argon <prompt> | dag-ulw <goal> | dag-run [--live] <goal> | models | status | jobs | inbox | restart]"
        )
        return 1


def main(argv: list[str] | None = None) -> int:
    """CLI entrypoint for `hcodex`.

    Args:
        argv: Optional command-line argument list.

    Returns:
        Process exit status code.
    """
    return HeterarchyCliController().dispatch(argv=argv)


if __name__ == "__main__":
    raise SystemExit(main())
