"""Static WAW/RAW + CodeGraph hazard compiler and two-stage settlement verifier for DAG-ULW++."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path, PurePosixPath, PureWindowsPath

from app.core.collab_enums import AuditSeverity
from app.core.dag_enums import (
    DagHazardCode,
    DagIgnoredDirectory,
    DagIsolationMode,
    DagNodeKind,
    DagNodeRoleKind,
    DagSettlementStage,
)
from app.core.skill_registry import DisplayWidthFormatter
from app.evidence.codegraph_slicer import CodegraphContextSlicer
from app.evidence.dag_agent_effects import build_triad_nodes, format_effort_ring
from app.schemas.dag_input_schemas import (
    DagNodeInputModel,
    DagUlwRequestModel,
)
from app.schemas.inner_dto.dag_dtos import (
    DagCompiledPlan,
    DagHazard,
    DagNodeSettlementReceipt,
    DagWaveSpec,
)

_IGNORED_DIRS: frozenset[str] = frozenset(str(member) for member in DagIgnoredDirectory)
_FILE_DIGEST_CACHE: dict[tuple[str, int, int], str] = {}


def normalize_scope_path(rel_path: str) -> str | None:
    """Normalize a declared scope path to a canonical POSIX relative path, or None if unsafe.

    Args:
        rel_path: Candidate relative file path string.

    Returns:
        Normalized POSIX relative path string, or None when unsafe or invalid.
    """
    cleaned = rel_path.strip()
    if not cleaned or "\x00" in cleaned:
        return None
    win = PureWindowsPath(cleaned)
    if win.drive or win.root or cleaned.startswith(("/", "\\")):
        return None
    posix = PurePosixPath(cleaned.replace("\\", "/"))
    if posix.is_absolute() or ".." in posix.parts:
        return None
    parts = tuple(p for p in posix.parts if p not in ("", "."))
    return "/".join(parts) if parts else None


def _make_hazard(
    code: DagHazardCode, severity: AuditSeverity, node_ids: tuple[str, ...],
    message: str, paths: tuple[str, ...] = (), symbols: tuple[str, ...] = (),
) -> DagHazard:
    """Create a blocking `DagHazard` instance with concise call-site bindings.

    Args:
        code: Machine-readable hazard code.
        severity: Audit severity tier.
        node_ids: Implicated node identifiers.
        message: Human-readable hazard explanation.
        paths: Conflicting relative file paths.
        symbols: Conflicting CodeGraph symbol names.

    Returns:
        Frozen `DagHazard` DTO.
    """
    return DagHazard(code=code, severity=str(severity), blocking=True, node_ids=node_ids, conflicting_paths=paths, conflicting_symbols=symbols, message=message)


class DagTopologyCompiler:
    """Compile a DAG-ULW++ node graph into topological waves with hazard and SpawnPlan gates."""

    def __init__(self) -> None:
        """Initialize the DAG topology compiler with a reusable CodeGraph context slicer."""
        self._slicer: CodegraphContextSlicer = CodegraphContextSlicer()

    def build_default_triad_nodes(self, campaign_goal: str) -> tuple[DagNodeInputModel, ...]:
        """Synthesize a canonical 4-node Tri-Polar DAG when caller omits explicit `nodes`.

        Args:
            campaign_goal: High-level engineering objective to decompose.

        Returns:
            Tuple of 4 `DagNodeInputModel` specifications spanning Argon, Claude, and Codex.
        """
        return build_triad_nodes(campaign_goal=campaign_goal)

    def validate_and_partition_waves(self, nodes: tuple[DagNodeInputModel, ...]) -> tuple[list[tuple[str, ...]], list[DagHazard]]:
        """Validate node IDs, dependencies, and scope paths, then partition into Kahn waves.

        Args:
            nodes: Tuple of DAG node input specifications.

        Returns:
            Tuple of `(wave_node_id_tuples, structural_hazards)`.
        """
        hazards: list[DagHazard] = []
        if not nodes:
            hazards.append(_make_hazard(code=DagHazardCode.DAG_EMPTY_GRAPH, severity=AuditSeverity.CRITICAL, node_ids=(), message="DAG contains zero nodes."))
            return [], hazards

        by_id: dict[str, DagNodeInputModel] = {}
        for node in nodes:
            if node.node_id in by_id:
                hazards.append(_make_hazard(code=DagHazardCode.DAG_DUPLICATE_NODE_ID, severity=AuditSeverity.CRITICAL, node_ids=(node.node_id,), message=f"Duplicate node_id '{node.node_id}' in DAG."))
                continue
            by_id[node.node_id] = node
            bad_paths = tuple(p for p in (*node.read_files, *node.write_files) if normalize_scope_path(rel_path=p) is None)
            if bad_paths:
                hazards.append(_make_hazard(code=DagHazardCode.DAG_INVALID_SCOPE_PATH, severity=AuditSeverity.CRITICAL, node_ids=(node.node_id,), paths=bad_paths, message=f"Node '{node.node_id}' declares unsafe scope path(s): {list(bad_paths)}"))

        in_degree: dict[str, int] = {nid: 0 for nid in by_id}
        dependents: dict[str, list[str]] = {nid: [] for nid in by_id}
        for nid, node in by_id.items():
            for dep in dict.fromkeys(node.depends_on):
                if dep == nid:
                    hazards.append(_make_hazard(code=DagHazardCode.DAG_SELF_DEPENDENCY, severity=AuditSeverity.CRITICAL, node_ids=(nid,), message=f"Node '{nid}' cannot depend on itself."))
                elif dep not in by_id:
                    hazards.append(_make_hazard(code=DagHazardCode.DAG_UNKNOWN_DEPENDENCY, severity=AuditSeverity.CRITICAL, node_ids=(nid, dep), message=f"Node '{nid}' depends on unknown node '{dep}'."))
                else:
                    in_degree[nid] += 1
                    dependents[dep].append(nid)

        if hazards:
            return [], hazards

        remaining = dict(in_degree)
        waves: list[tuple[str, ...]] = []
        while remaining:
            ready = tuple(sorted(nid for nid, deg in remaining.items() if deg == 0))
            if not ready:
                cyc = tuple(sorted(remaining.keys()))
                hazards.append(_make_hazard(code=DagHazardCode.DAG_CYCLE_DETECTED, severity=AuditSeverity.CRITICAL, node_ids=cyc, message=f"Cycle detected among DAG nodes: {list(cyc)}"))
                return [], hazards
            waves.append(ready)
            for nid in ready:
                del remaining[nid]
                for child in dependents[nid]:
                    if child in remaining:
                        remaining[child] -= 1
        return waves, hazards

    def analyze_scope_and_symbol_hazards(self, nodes: tuple[DagNodeInputModel, ...], waves: list[tuple[str, ...]], workspace_dir: str) -> list[DagHazard]:
        """Detect WAW, un-ordered RAW, and CodeGraph 1-hop symbol hazards across waves.

        Args:
            nodes: Tuple of DAG node input specifications.
            waves: Topological wave node ID groups.
            workspace_dir: Resolved workspace directory path.

        Returns:
            List of detected `DagHazard` instances.
        """
        hazards: list[DagHazard] = []
        by_id = {n.node_id: n for n in nodes}
        ancestors: dict[str, set[str]] = {n.node_id: set() for n in nodes}
        for wave in waves:
            for nid in wave:
                for dep in by_id[nid].depends_on:
                    if dep in ancestors:
                        ancestors[nid].add(dep)
                        ancestors[nid].update(ancestors[dep])

        expanded_symbols: dict[str, set[str]] = {}
        for n in nodes:
            seed_syms = [s.strip() for s in n.symbols if s.strip()]
            full_syms = set(seed_syms)
            if seed_syms:
                for sl in self._slicer.slice_symbols(workspace_dir=workspace_dir, symbols=seed_syms):
                    full_syms.update((*sl.callers, *sl.callees))
            expanded_symbols[n.node_id] = full_syms

        norm_writes = {n.node_id: {np for p in n.write_files if (np := normalize_scope_path(rel_path=p)) is not None} for n in nodes}
        norm_reads = {n.node_id: {np for p in n.read_files if (np := normalize_scope_path(rel_path=p)) is not None} for n in nodes}

        for wave in waves:
            for idx, nid_a in enumerate(wave):
                writes_a = norm_writes[nid_a]
                for nid_b in wave[idx + 1 :]:
                    writes_b = norm_writes[nid_b]
                    waw = tuple(sorted(writes_a & writes_b))
                    if waw:
                        hazards.append(
                            _make_hazard(
                                code=DagHazardCode.DAG_WAW_SCOPE_CONFLICT,
                                severity=AuditSeverity.CRITICAL,
                                node_ids=(nid_a, nid_b),
                                paths=waw,
                                message=f"Same-wave WAW conflict between '{nid_a}' and '{nid_b}' on {list(waw)}",
                            )
                        )
                    if writes_a and writes_b:
                        sym_ov = tuple(sorted(expanded_symbols[nid_a] & expanded_symbols[nid_b]))
                        if sym_ov:
                            hazards.append(
                                _make_hazard(
                                    code=DagHazardCode.DAG_WAW_SCOPE_CONFLICT,
                                    severity=AuditSeverity.HIGH,
                                    node_ids=(nid_a, nid_b),
                                    symbols=sym_ov,
                                    message=f"CodeGraph 1-hop symbol write hazard between '{nid_a}' and '{nid_b}': {list(sym_ov)}",
                                )
                            )

        for idx, n_a in enumerate(nodes):
            writes_a, reads_a = norm_writes[n_a.node_id], norm_reads[n_a.node_id]
            for n_b in nodes[idx + 1 :]:
                if (n_a.node_id in ancestors[n_b.node_id]) or (n_b.node_id in ancestors[n_a.node_id]):
                    continue
                writes_b, reads_b = norm_writes[n_b.node_id], norm_reads[n_b.node_id]
                raw_ov = tuple(sorted((writes_a & reads_b) | (writes_b & reads_a) | (writes_a & writes_b)))
                if raw_ov and not (any(n_a.node_id in w and n_b.node_id in w for w in waves) and (writes_a & writes_b)):
                    hazards.append(
                        _make_hazard(
                            code=DagHazardCode.DAG_MISSING_RAW_EDGE,
                            severity=AuditSeverity.HIGH,
                            node_ids=(n_a.node_id, n_b.node_id),
                            paths=raw_ov,
                            message=f"Unordered RAW/WAR/WAW hazard between '{n_a.node_id}' and '{n_b.node_id}' on {list(raw_ov)}",
                        )
                    )
        return hazards

    def render_ascii_dag(
        self,
        nodes: tuple[DagNodeInputModel, ...],
        waves: tuple[DagWaveSpec, ...],
        node_work_map: dict[str, str] | None = None,
    ) -> str:
        """Render an `oh-my-hermes`-style connected Fan-Out / Fan-In box-drawing dependency graph.

        Args:
            nodes: Tuple of DAG node input specifications.
            waves: Tuple of compiled wave specifications.
            node_work_map: Optional mapping of `node_id` to live or settled work status text.

        Returns:
            Multi-line Unicode box-drawing string visualizing waves and dependencies.
        """
        if not waves:
            return "╭──────────────────────────╮\n│  (empty or cyclic graph) │\n╰──────────────────────────╯"
        by_id = {n.node_id: n for n in nodes}
        target_w = 115
        lines: list[str] = []
        for w_idx, wave in enumerate(waves):
            cnt = len(wave.node_ids)
            has_next = w_idx + 1 < len(waves)
            box_w = 85 if cnt == 1 else (56 if cnt == 2 else max(32, (target_w - 3 * (cnt - 1)) // cnt))
            boxes: list[tuple[str, str, str, str]] = []
            for lane_i, nid in enumerate(wave.node_ids):
                nd = by_id[nid]
                raw_m = (nd.preferred_model or "").strip()
                if nd.role in (DagNodeRoleKind.TOPOLOGY_PLANNER, DagNodeRoleKind.VERIFICATION_FAN_IN):
                    ic, m_short, eff_s = "✦", ("flash" if "flash" in raw_m else ("argon" if (not raw_m or "argon" in raw_m) else raw_m[:15])), (nd.reasoning_effort or "high")
                elif nd.role in (DagNodeRoleKind.INTERFACE_CONTRACTOR, DagNodeRoleKind.DISJOINT_BUILDER):
                    ic, m_short, eff_s = "◆", ("opus" if "opus" in raw_m else ("sonnet" if (not raw_m or "sonnet" in raw_m) else raw_m[:15])), (nd.reasoning_effort or "medium")
                else:
                    c_lbl = raw_m or "gpt-6-astra"
                    ic, m_short, eff_s = "★", c_lbl[:15], (nd.reasoning_effort or "xhigh")
                ring_s = format_effort_ring(effort=eff_s)
                left_h = f" Wave {wave.wave_index}" + (f"·L{lane_i + 1} " if cnt > 1 else f" ({wave.isolation_mode}, w={wave.writer_count}) ")
                right_h = f" {ic} {m_short}@{ring_s} "
                left_dashes = max(1, (box_w // 2) - 1 - len(left_h))
                rem_after_mid = max(1, box_w - 2 - len(left_h) - left_dashes - 1)
                r_fit = DisplayWidthFormatter.fit_columns(text=right_h, width=rem_after_mid, pad=False)
                right_dashes = max(0, rem_after_mid - DisplayWidthFormatter.text_width(text=r_fit))
                top_b = f"╭{left_h}{'─' * left_dashes}{'▼' if w_idx > 0 else '─'}{'─' * right_dashes}{r_fit}╮"
                p_fit = DisplayWidthFormatter.fit_columns(text=f"▸ [{nid}] Prompt: {nd.task}", width=box_w - 4)
                work_txt = (node_work_map or {}).get(nid) or f"○ Ready ➔ {nd.deliverable}"
                w_fit = DisplayWidthFormatter.fit_columns(text=f"▸ Work  : {work_txt}", width=box_w - 4)
                half = (box_w - 2) // 2
                if cnt > 1 and box_w >= 34:
                    f_lbl = " · Same frontier "
                    l_rem = max(1, half - len(f_lbl))
                    r_rem = max(1, box_w - 3 - l_rem - len(f_lbl))
                    bot_b = f"╰{'─' * l_rem}{f_lbl}{'┬' if has_next else '─'}{'─' * r_rem}╯"
                else:
                    bot_b = f"╰{'─' * half}┬{'─' * (box_w - 3 - half)}╯" if has_next else f"╰{'─' * (box_w - 2)}╯"
                boxes.append((top_b, f"│ {p_fit} │", f"│ {w_fit} │", bot_b))
            pad = " " * 15 if cnt == 1 else (" " * max(0, (target_w - (cnt * box_w + (cnt - 1) * 3)) // 2))
            for r_i in range(4):
                lines.append(pad + "   ".join(b[r_i] for b in boxes))
            if has_next:
                nxt_cnt = len(waves[w_idx + 1].node_ids)
                if cnt == 1 and nxt_cnt >= 2:
                    lines.append(f"{' ' * 25}┌{'─' * 26}┴{'─' * 26}┐")
                elif cnt >= 2 and nxt_cnt == 1:
                    lines.append(f"{' ' * 25}└{'─' * 26}┬{'─' * 26}┘")
                elif cnt >= 2 and nxt_cnt >= 2:
                    lines.append(f"{' ' * 25}├─── · Same frontier ──────┼────── · Same frontier ───┤")
                else:
                    lines.append(f"{' ' * 52}│")
        return "\n".join(lines)

    def compile_plan(self, req: DagUlwRequestModel, workspace_dir: str, dag_id: str) -> tuple[tuple[DagNodeInputModel, ...], DagCompiledPlan]:
        """Compile a `DagUlwRequestModel` into waves, hazards, and a deterministic plan digest.

        Args:
            req: Validated external DAG-ULW++ request model.
            workspace_dir: Resolved workspace directory path.
            dag_id: Assigned DAG campaign identifier (`dag-...`).

        Returns:
            Tuple of `(effective_nodes, compiled_plan_dto)`.
        """
        eff_nodes = req.nodes or self.build_default_triad_nodes(campaign_goal=req.campaign_goal)
        raw_waves, hazards = self.validate_and_partition_waves(nodes=eff_nodes)
        if raw_waves:
            hazards.extend(self.analyze_scope_and_symbol_hazards(nodes=eff_nodes, waves=raw_waves, workspace_dir=workspace_dir))
        spawn_ack = req.spawn_plan is not None and req.spawn_plan.hazard_acknowledgment
        if len(eff_nodes) > 1 and not req.plan_only and not spawn_ack:
            hazards.append(
                _make_hazard(
                    code=DagHazardCode.DAG_SPAWN_PLAN_REQUIRED,
                    severity=AuditSeverity.HIGH,
                    node_ids=tuple(n.node_id for n in eff_nodes),
                    message="Multi-node execution (`plan_only=False`) requires an explicit `spawn_plan`.",
                )
            )
        by_id = {n.node_id: n for n in eff_nodes}
        max_par = req.spawn_plan.max_parallel_per_wave if req.spawn_plan else 3
        pref_iso = req.spawn_plan.isolation_mode if req.spawn_plan else DagIsolationMode.DIRECT
        wave_specs: list[DagWaveSpec] = []
        for idx, group in enumerate(raw_waves):
            writers = sum(1 for nid in group if by_id[nid].write_files)
            eff_iso = DagIsolationMode.SERIAL if (writers > 1 and pref_iso == DagIsolationMode.DIRECT) else pref_iso
            eff_conc = 1 if eff_iso == DagIsolationMode.SERIAL else min(len(group), max_par)
            wave_specs.append(
                DagWaveSpec(
                    wave_index=idx,
                    node_ids=group,
                    isolation_mode=eff_iso,
                    writer_count=writers,
                    concurrency_limit=eff_conc,
                )
            )
        waves_tuple = tuple(wave_specs)
        hazards_tuple = tuple(hazards)
        digest_src = "|".join(f"{n.node_id}:{n.role}:{n.node_kind}:{','.join(sorted(set(n.depends_on)))}:{','.join(sorted(set(n.write_files)))}" for n in sorted(eff_nodes, key=lambda x: x.node_id))
        plan_digest = f"sha256:{hashlib.sha256(f'{req.campaign_goal}|{digest_src}'.encode()).hexdigest()}"
        plan = DagCompiledPlan(
            dag_id=dag_id,
            campaign_goal=req.campaign_goal,
            plan_digest=plan_digest,
            total_nodes=len(eff_nodes),
            total_waves=len(waves_tuple),
            waves=waves_tuple,
            hazards=hazards_tuple,
            blocking_hazards_count=sum(1 for h in hazards_tuple if h.blocking),
            spawn_plan_acknowledged=spawn_ack,
            ascii_graph=self.render_ascii_dag(nodes=eff_nodes, waves=waves_tuple),
        )
        return eff_nodes, plan


class DagSettlementVerifier:
    """Enforce full-workspace SHA-256 manifest capture and `STOPPED -> SETTLED` verification."""

    def capture_workspace_manifest(self, workspace_dir: str) -> dict[str, str]:
        """Scan `workspace_dir` and compute SHA-256 digests for all non-ignored files.

        Args:
            workspace_dir: Root workspace directory path.

        Returns:
            Sorted mapping of relative POSIX file path to `sha256:<hex>` digest.
        """
        root = Path(workspace_dir).resolve()
        manifest: dict[str, str] = {}
        if not root.is_dir():
            return manifest
        for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
            dirnames[:] = sorted(d for d in dirnames if d not in _IGNORED_DIRS and not d.startswith(".pytest"))
            for fname in sorted(filenames):
                if fname.endswith((".pyc", ".pyo", ".DS_Store")):
                    continue
                fpath = Path(dirpath) / fname
                rel = fpath.relative_to(root).as_posix()
                if fpath.is_symlink():
                    try:
                        target_s = os.readlink(fpath)
                    except OSError as exc:
                        manifest[rel] = f"unreadable:{type(exc).__name__}"
                        continue
                    th = hashlib.sha256(target_s.encode("utf-8", "surrogateescape")).hexdigest()
                    manifest[rel] = f"symlink:sha256:{th}"
                    continue
                if not fpath.is_file():
                    continue
                try:
                    st = fpath.stat(follow_symlinks=False)
                    cache_key = (str(fpath), st.st_mtime_ns, st.st_size)
                    cached_dig = _FILE_DIGEST_CACHE.get(cache_key)
                    if cached_dig is None:
                        raw = fpath.read_bytes()
                        cached_dig = f"sha256:{hashlib.sha256(raw).hexdigest()}"
                        if len(_FILE_DIGEST_CACHE) >= 8192:
                            _FILE_DIGEST_CACHE.clear()
                        _FILE_DIGEST_CACHE[cache_key] = cached_dig
                    manifest[rel] = cached_dig
                except OSError as exc:
                    manifest[rel] = f"unreadable:{type(exc).__name__}"
                    continue
        return manifest

    def compute_manifest_digest(self, manifest: dict[str, str]) -> str:
        """Compute a length-prefixed aggregate SHA-256 digest over a file-manifest mapping.

        Args:
            manifest: Relative path to file digest mapping.

        Returns:
            Aggregate `sha256:<hex>` digest string.
        """
        hasher = hashlib.sha256()
        for k, v in sorted(manifest.items()):
            kb, vb = k.encode("utf-8", "surrogateescape"), v.encode("utf-8")
            hasher.update(len(kb).to_bytes(8, "big") + kb + len(vb).to_bytes(8, "big") + vb)
        return f"sha256:{hasher.hexdigest()}"

    def diff_manifests(self, pre_manifest: dict[str, str], post_manifest: dict[str, str]) -> tuple[str, ...]:
        """Return all relative paths added, modified, or deleted between two manifests.

        Args:
            pre_manifest: Workspace manifest captured before node execution.
            post_manifest: Workspace manifest captured after node execution (`STOPPED`).

        Returns:
            Sorted tuple of mutated relative file paths.
        """
        all_paths = sorted(set(pre_manifest.keys()) | set(post_manifest.keys()))
        return tuple(p for p in all_paths if pre_manifest.get(p) != post_manifest.get(p))

    def settle_node(
        self,
        node: DagNodeInputModel,
        wave_index: int,
        model_id: str,
        duration_seconds: float,
        wave_baseline_manifest: dict[str, str],
        pre_manifest: dict[str, str],
        post_manifest: dict[str, str],
        execution_ok: bool,
        verify_passed: bool,
        reproduction_observed: bool,
        output_summary: str,
        error_text: str | None = None,
    ) -> DagNodeSettlementReceipt:
        """Evaluate two-stage settlement (`STOPPED -> SETTLED`) against scope and gates.

        Args:
            node: Executed DAG node specification.
            wave_index: Topological wave index where the node ran.
            model_id: Resolved model identifier used for execution.
            duration_seconds: Execution duration in seconds.
            wave_baseline_manifest: Workspace manifest captured at the start of the wave.
            pre_manifest: Workspace manifest captured immediately before the node started.
            post_manifest: Workspace manifest captured after the node stopped.
            execution_ok: Whether the underlying model/process call succeeded.
            verify_passed: Whether the node's `verify_cmd` gate passed.
            reproduction_observed: Whether a reproduction node observed expected failure.
            output_summary: Short summary extracted from the node's output.
            error_text: Optional error string from execution or verification.

        Returns:
            Frozen `DagNodeSettlementReceipt` with final `DagSettlementStage`.
        """
        pre_dig = self.compute_manifest_digest(manifest=pre_manifest)
        post_dig = self.compute_manifest_digest(manifest=post_manifest)
        mutated = self.diff_manifests(pre_manifest=pre_manifest, post_manifest=post_manifest)
        allowed_writes = {np for p in node.write_files if (np := normalize_scope_path(rel_path=p)) is not None}
        out_of_scope = tuple(p for p in mutated if p not in allowed_writes)
        dirty_reads = tuple(np for p in node.read_files if (np := normalize_scope_path(rel_path=p)) is not None and wave_baseline_manifest.get(np) != pre_manifest.get(np))
        if dirty_reads:
            stage, err = (
                DagSettlementStage.REJECTED_DIRTY_READ,
                f"Dirty read on {list(dirty_reads)}",
            )
        elif out_of_scope:
            stage, err = (
                DagSettlementStage.REJECTED_OUT_OF_SCOPE_WRITE,
                f"Out-of-scope write on {list(out_of_scope)}",
            )
        elif not execution_ok:
            stage, err = (
                DagSettlementStage.FAILED_EXECUTION,
                (error_text or "Node execution failed."),
            )
        elif node.node_kind == DagNodeKind.REPRODUCTION and not reproduction_observed:
            stage, err = (
                DagSettlementStage.REJECTED_REPRODUCTION_NOT_OBSERVED,
                "ReproductionHold gate not satisfied.",
            )
        elif not verify_passed:
            stage, err = (
                DagSettlementStage.REJECTED_VERIFY_FAILED,
                (error_text or "Verification gate failed."),
            )
        else:
            stage, err = DagSettlementStage.SETTLED, None

        return DagNodeSettlementReceipt(
            node_id=node.node_id,
            role=node.role,
            node_kind=node.node_kind,
            wave_index=wave_index,
            settlement_stage=stage,
            model_id=model_id,
            duration_seconds=duration_seconds,
            pre_manifest_digest=pre_dig,
            post_manifest_digest=post_dig,
            mutated_files=mutated,
            out_of_scope_writes=out_of_scope,
            dirty_read_files=dirty_reads,
            verify_passed=verify_passed,
            reproduction_observed=reproduction_observed,
            output_summary=output_summary,
            error=err,
        )
