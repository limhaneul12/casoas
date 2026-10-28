"""Connect-RPC transport, process endpoint discovery, and model catalog for Antigravity."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import re
import socket
import ssl
import threading
import time
import urllib.error
import urllib.request
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Final

from codex_collab_mcp.collab_enums import EngineKind, ProviderKind, ReasoningTier
from codex_collab_mcp.routing.model_specs import ANTIGRAVITY_MODEL_SPECS
from codex_collab_mcp.routing.policy_engine import validate_and_resolve_route
from codex_collab_mcp.schemas import JsonObject
from codex_collab_mcp.transports.transport_dicts import (
    AgyAuthBoundary,
    AgyAuthBoundaryDict,
    AgyModelConfigEntry,
    AgyModelConfigEntryDict,
)

RpcTransportFn = Callable[[str, str, str, JsonObject, float], JsonObject]

_CSRF_TOKEN_RE: Final[re.Pattern[str]] = re.compile(r"--csrf_token(?:=|\s+)([^\s]+)")
_LISTEN_PORT_RE: Final[re.Pattern[str]] = re.compile(r"127\.0\.0\.1:(\d+)\s+\(LISTEN\)")
_FENCED_JSON_RE: Final[re.Pattern[str]] = re.compile(
    r"```(?:json)?\s*(\{.*\})\s*```", flags=re.DOTALL
)


def _make_unverified_ssl_context() -> ssl.SSLContext:
    """Create a reusable unverified SSLContext for local loopback HTTPS Connect-RPC.

    Returns:
        An SSLContext configured without hostname verification or certificate validation.
    """
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx


_UNVERIFIED_SSL_CONTEXT: ssl.SSLContext = _make_unverified_ssl_context()


@dataclass(frozen=True, slots=True)
class AgyEndpoint:
    """Connection parameters for the local Antigravity LanguageServer RPC endpoint."""

    base_url: str
    csrf_token: str
    pid: int | None = None


DiscoverEndpointFn = Callable[[], Awaitable[AgyEndpoint | None]]


class AntigravityRpcClient:
    """Low-level Connect-RPC client and local language_server process endpoint discovery."""

    _cached_endpoint: tuple[AgyEndpoint, float] | None = None
    _cache_lock: threading.Lock = threading.Lock()
    _ENDPOINT_CACHE_TTL: float = 15.0

    @staticmethod
    def post_connect_rpc_sync(
        base_url: str,
        csrf_token: str,
        method: str,
        payload: JsonObject,
        timeout_seconds: float,
    ) -> JsonObject:
        """Execute a synchronous HTTPS/HTTP Connect-RPC POST to the local language server.

        Args:
            base_url: Loopback HTTP or HTTPS base URL for the language server.
            csrf_token: Codeium CSRF token required by the local language server.
            method: Connect-RPC method name on LanguageServerService.
            payload: JSON-serializable request payload dictionary.
            timeout_seconds: Socket read timeout in seconds.

        Returns:
            Parsed JSON object returned by the Connect-RPC endpoint.
        """
        url = f"{base_url}/exa.language_server_pb.LanguageServerService/{method}"
        body_bytes = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(  # noqa: S310
            url,
            data=body_bytes,
            headers={
                "Content-Type": "application/json",
                "x-codeium-csrf-token": csrf_token,
            },
            method="POST",
        )
        ctx: ssl.SSLContext | None = _UNVERIFIED_SSL_CONTEXT if url.startswith("https://") else None

        with urllib.request.urlopen(req, timeout=timeout_seconds, context=ctx) as resp:  # noqa: S310
            raw_text = resp.read().decode("utf-8", errors="replace")

        parsed = json.loads(raw_text) if raw_text.strip() else {}
        if isinstance(parsed, dict):
            return parsed
        return {"raw": parsed}

    @staticmethod
    async def run_probe_cmd(*args: str, probe_timeout: float = 5.0) -> str:
        """Run a local process probe (`ps` or `lsof`) with guaranteed cleanup on timeout/cancel.

        Args:
            args: Command and positional arguments to execute.
            probe_timeout: Maximum execution time in seconds before killing the probe process.

        Returns:
            Decoded stdout text from the probe process, or an empty string on failure.
        """
        try:
            proc = await asyncio.create_subprocess_exec(
                *args,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
        except OSError:
            return ""
        try:
            stdout_bytes, _ = await asyncio.wait_for(proc.communicate(), timeout=probe_timeout)
        except BaseException as exc:
            if proc.returncode is None:
                with contextlib.suppress(ProcessLookupError):
                    proc.kill()
                await proc.wait()
            if isinstance(exc, (TimeoutError, OSError)):
                return ""
            raise
        return stdout_bytes.decode("utf-8", errors="replace")

    @staticmethod
    def extract_candidates_from_ps(
        ps_output: str,
        env_csrf: str,
    ) -> list[tuple[int, str]]:
        """Extract candidate `(pid, csrf_token)` pairs from `ps -axo pid,command` output.

        Args:
            ps_output: Raw stdout text from the `ps` command.
            env_csrf: Fallback CSRF token from the `ANTIGRAVITY_CSRF_TOKEN` environment variable.

        Returns:
            Ordered list of `(pid, csrf_token)` tuples for candidate language_server processes.
        """
        candidates: list[tuple[int, str]] = []
        for line in ps_output.splitlines():
            if "language_server" not in line or "--csrf_token" not in line:
                continue
            stripped = line.strip()
            parts = stripped.split(None, 1)
            if len(parts) < 2 or not parts[0].isdigit():
                continue
            pid = int(parts[0])
            cmd_str = parts[1]
            match = _CSRF_TOKEN_RE.search(cmd_str)
            token = match.group(1) if match else env_csrf
            if token:
                candidates.append((pid, token))
        return candidates

    @staticmethod
    def extract_listen_ports(lsof_text: str) -> list[int]:
        """Extract loopback TCP listen ports from `lsof` output.

        Args:
            lsof_text: Raw stdout text from `lsof -nP -a -p <pid> -iTCP -sTCP:LISTEN`.

        Returns:
            List of loopback TCP port numbers in listen state.
        """
        ports: list[int] = []
        for lsof_line in lsof_text.splitlines():
            port_match = _LISTEN_PORT_RE.search(lsof_line)
            if port_match:
                ports.append(int(port_match.group(1)))
        return ports

    @classmethod
    async def discover_endpoint(
        cls,
        rpc_transport: RpcTransportFn | None = None,
    ) -> AgyEndpoint | None:
        """Discover the live Antigravity `language_server` HTTPS/HTTP endpoint and CSRF token.

        Args:
            rpc_transport: Optional synchronous Connect-RPC transport callable override.

        Returns:
            Discovered `AgyEndpoint` instance if a live server responds, otherwise `None`.
        """
        if rpc_transport is None:
            with cls._cache_lock:
                cached = cls._cached_endpoint
            if cached is not None and (time.monotonic() - cached[1]) < cls._ENDPOINT_CACHE_TTL:
                return cached[0]

        transport = rpc_transport or cls.post_connect_rpc_sync
        env_csrf = os.environ.get("ANTIGRAVITY_CSRF_TOKEN", "").strip()
        ps_output = await cls.run_probe_cmd("ps", "-axo", "pid,command", probe_timeout=5.0)
        candidates = cls.extract_candidates_from_ps(ps_output, env_csrf)

        for pid, token in candidates:
            lsof_text = await cls.run_probe_cmd(
                "lsof",
                "-nP",
                "-a",
                "-p",
                str(pid),
                "-iTCP",
                "-sTCP:LISTEN",
                probe_timeout=5.0,
            )
            ports = cls.extract_listen_ports(lsof_text)

            for port in ports:
                for scheme in ("https", "http"):
                    base_url = f"{scheme}://127.0.0.1:{port}"
                    try:
                        res: JsonObject | None = await asyncio.to_thread(
                            transport,
                            base_url,
                            token,
                            "GetCascadeModelConfigData",
                            {},
                            2.5,
                        )
                    except Exception:
                        res = None
                    if res is not None and "clientModelConfigs" in res:
                        endpoint = AgyEndpoint(base_url=base_url, csrf_token=token, pid=pid)
                        if rpc_transport is None:
                            with cls._cache_lock:
                                cls._cached_endpoint = (endpoint, time.monotonic())
                        return endpoint

        if rpc_transport is None:
            with cls._cache_lock:
                cls._cached_endpoint = None
        return None

    @staticmethod
    def is_timeout_exception(exc: BaseException) -> bool:
        """Return True if `exc` represents a read, socket, or HTTP timeout after dispatch.

        Args:
            exc: Exception raised during Connect-RPC transport execution.

        Returns:
            True if the exception indicates a timeout condition, False otherwise.
        """
        if isinstance(exc, (TimeoutError, socket.timeout)):
            return True
        if isinstance(exc, urllib.error.URLError):
            reason = exc.reason
            if isinstance(reason, (TimeoutError, socket.timeout)):
                return True
            if "timed out" in str(reason).lower():
                return True
        return "timed out" in str(exc).lower()


class AntigravityModelCatalog:
    """Model catalog parser, placeholder resolver, and JSON extractor for Antigravity RPC."""

    @staticmethod
    def build_auth_boundary(connected: bool) -> AgyAuthBoundaryDict:
        """Build the Antigravity local CSRF authentication boundary TypedDict.

        Args:
            connected: Whether the local Antigravity LanguageServer RPC connection was established.

        Returns:
            Typed AgyAuthBoundaryDict for Antigravity RPC responses.
        """
        return AgyAuthBoundary(antigravity_rpc_connected=connected).to_dict()

    @staticmethod
    def parse_model_config_entry(item: JsonObject) -> AgyModelConfigEntryDict:
        """Normalize a single `clientModelConfigs` dictionary entry from LanguageServer.

        Args:
            item: Raw model configuration dictionary from `GetCascadeModelConfigData`.

        Returns:
            Typed AgyModelConfigEntryDict with routability and quota fields.
        """
        label = item.get("label")
        model_id = str(item.get("modelId") or "")
        alias_obj = item.get("modelOrAlias")
        quota_obj = item.get("quotaInfo")
        placeholder = str(alias_obj.get("model") or "") if isinstance(alias_obj, dict) else ""
        remaining = quota_obj.get("remainingFraction") if isinstance(quota_obj, dict) else None
        remaining_val = float(remaining) if isinstance(remaining, (int, float)) else None
        spec = ANTIGRAVITY_MODEL_SPECS.get(model_id.lower())
        is_disallowed_claude = (
            spec is not None
            and spec.model_id.startswith("claude-")
            and spec.reasoning_tier != ReasoningTier.MEDIUM
        )
        routable = bool(
            placeholder
            and (remaining_val is None or remaining_val > 0.0)
            and not is_disallowed_claude
        )
        return AgyModelConfigEntry(
            label=str(label or ""),
            model_id=model_id,
            placeholder=placeholder,
            remaining_quota=remaining_val,
            routable=routable,
        ).to_dict()

    @classmethod
    async def list_models(
        cls,
        endpoint: AgyEndpoint | None = None,
        rpc_transport: RpcTransportFn | None = None,
        discover_fn: DiscoverEndpointFn | None = None,
    ) -> list[AgyModelConfigEntryDict]:
        """Return available models and quotas from the local Antigravity LanguageServer.

        Args:
            endpoint: Optional pre-discovered `AgyEndpoint` to query.
            rpc_transport: Optional synchronous Connect-RPC transport callable override.
            discover_fn: Optional async endpoint discovery callable override.

        Returns:
            List of normalized AgyModelConfigEntryDict items.
        """
        discover = discover_fn or AntigravityRpcClient.discover_endpoint
        resolved = endpoint or await discover()
        if resolved is None:
            return []

        transport = rpc_transport or AntigravityRpcClient.post_connect_rpc_sync
        try:
            data = await asyncio.to_thread(
                transport,
                resolved.base_url,
                resolved.csrf_token,
                "GetCascadeModelConfigData",
                {},
                5.0,
            )
        except Exception:
            if endpoint is None and rpc_transport is None:
                with AntigravityRpcClient._cache_lock:
                    AntigravityRpcClient._cached_endpoint = None
            return []

        raw_configs = data.get("clientModelConfigs")
        if not isinstance(raw_configs, list):
            return []

        models: list[AgyModelConfigEntryDict] = []
        for item in raw_configs:
            if isinstance(item, dict):
                models.append(cls.parse_model_config_entry(item))
        return models

    @classmethod
    async def resolve_model_placeholder(
        cls,
        requested_model: str,
        endpoint: AgyEndpoint | None = None,
        rpc_transport: RpcTransportFn | None = None,
        discover_fn: DiscoverEndpointFn | None = None,
    ) -> tuple[str, str]:
        """Strictly resolve a validated Antigravity model ID or alias to `(model_id, placeholder)`.

        Args:
            requested_model: Model identifier or alias requested by the caller.
            endpoint: Optional pre-discovered `AgyEndpoint` instance.
            rpc_transport: Optional synchronous Connect-RPC transport callable override.
            discover_fn: Optional async endpoint discovery callable override.

        Returns:
            Tuple of `(resolved_model_id, placeholder_token)`.
        """
        route = validate_and_resolve_route(
            engine=EngineKind.ANTIGRAVITY_LOCAL_RPC,
            model=requested_model,
            provider=ProviderKind.ANTIGRAVITY,
            enforce_claude_medium_policy=True,
        )
        if not route.valid:
            raise ValueError(
                route.error_message or f"Invalid Antigravity model '{requested_model}'"
            )

        target_id = route.resolved_model
        spec = ANTIGRAVITY_MODEL_SPECS.get(target_id)
        if endpoint is not None and rpc_transport is None and spec is not None and spec.placeholder:
            return (spec.model_id, spec.placeholder)

        catalog = await cls.list_models(
            endpoint=endpoint,
            rpc_transport=rpc_transport,
            discover_fn=discover_fn,
        )
        for entry in catalog:
            mid = entry["model_id"].lower()
            ph = entry["placeholder"]
            if mid == target_id and ph:
                return (mid, ph)

        if spec is None or not spec.placeholder:
            raise ValueError(f"No verified placeholder found for Antigravity model '{target_id}'.")
        return (spec.model_id, spec.placeholder)

    @staticmethod
    def extract_json_from_model_text(raw_text: str) -> JsonObject | None:
        """Extract a JSON object from model response text, stripping markdown fences if present.

        Args:
            raw_text: Raw text output returned by the language model.

        Returns:
            Parsed JSON object dictionary if valid JSON is found, otherwise `None`.
        """
        cleaned = raw_text.strip()
        if cleaned.startswith("```"):
            fence_match = _FENCED_JSON_RE.search(cleaned)
            if fence_match:
                cleaned = fence_match.group(1).strip()

        try:
            parsed = json.loads(cleaned)
            if isinstance(parsed, dict):
                return parsed
        except json.JSONDecodeError:
            start_idx = cleaned.find("{")
            end_idx = cleaned.rfind("}")
            if start_idx != -1 and end_idx > start_idx:
                try:
                    sub_parsed = json.loads(cleaned[start_idx : end_idx + 1])
                    if isinstance(sub_parsed, dict):
                        return sub_parsed
                except json.JSONDecodeError:
                    return None
        return None
