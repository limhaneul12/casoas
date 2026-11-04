# Adapter parity restoration

Compare historical `HEAD:app/core/cli_tui.py`, `app/core/ctl.py`, `app/evidence/dag_agent_effects.py` and the old TUI tests with the current Rust command adapter. Preserve recent core-body loading, real DAG dependencies, owned cancellation, permissions and receipt truth.

Bounded fixes:

- Route CLI skills/asks/DAG commands through the same terminal command boundary where the old CLI did, retaining lifecycle/MCP operations separately. Support positional one-shot TUI input and DAG mode arguments. A bare `dag-ulw` invocation should execute; explicit `--plan-only` should plan.
- Restore valid effect/model/effort aliases and `all`, atomically validate a candidate configuration before committing it. Keep Claude Medium-only and current model routing authority; reject unsupported historical tiers instead of silently weakening contracts.
- Restore `/prompts` as an actual compiled-node inspector: task and selected skill context, deliverable, dependencies/scope/gate, actual returned output/receipt. Clearly distinguish planned instructions from undisclosed transport-generated final prompts. Filters select real node IDs/roles/models without falling back to an unrelated last response.
- Show workspace content changes since the first `/diff` snapshot (the actual historical baseline behavior), including untracked files, with manifest authority instead of a Git-status-only approximation. Do not replace the current real `/review` with the old static review panel.
- Keep `/compact` honest: compact local history only, never invent reclaimed provider tokens. Preserve current safe default permission mode.

Tests target CLI parsing and shared routing, atomic effect failure, effect aliases, planned/settled prompt inspection, and real manifest diff. Context reference resolution integrates only after the context owner supplies a typed contract. Parent owns full CI, installation and native runtime proof.

## Implemented result and evidence

- CLI default is historical `status`; `skills core`, positional `tui /skills core`, `dag-mode` arguments and DAG execution/explicit plan-only route through the actual terminal command boundary. `status`, `jobs` and `inbox` retain their existing object-shaped JSON. Interactive command-style CLI cases return typed TUI update arrays, as `tui -c` does; MCP schemas are unchanged.
- Model aliases and explicit efforts share routing normalization. Effects validate a clone and commit atomically. Gemini tier changes update actual model ID and displayed effort together. `all medium` is supported; `all high` and Claude non-Medium are explicitly rejected under the current routing contract. No unsupported `minimal` tier is substituted.
- `/prompts` shows real saved typed node tasks/skills, deliverable, parent/scope/gate, returned artifact and settlement receipt; live returns/settlement are captured as events arrive. Planned nodes remain explicitly unverified. Unknown selectors do not silently show an unrelated last response. Direct-only history remains inspectable; transport-generated final prompts are not fabricated.
- `/omh` aliases show the current DAG instructions, real source excerpts and gate evidence. `/diff` works outside Git and covers added/modified/deleted untracked files using the shared manifest implementation. Explicit resume IDs resolve their own saved goal rather than borrowing a different current campaign goal.
- Registered `$skill` or `/registered-skill` prefixes preserve the remaining command/question. Skill-only activation performs no provider dispatch. Reserved slash commands keep precedence, and DAG mode restores the discovered codex skill after clearing. Core selection remains atomic. Discovery honors `HCODEX_HOME` consistently with main.
- Focused verification: 16 terminal-adapter tests and the CLI parsing contract passed; strict all-target/all-feature Clippy passed. Parent runs final canonical CI and native proof on the integrated source.

Intentional safety differences: retain `auto-edit` startup rather than silently restoring historical YOLO, retain receipt token totals on `/compact`, and do not copy fabricated successful simulation frames. Parent restores historical `/triad` as the DAG alias and retains the real tri-polar review pipeline under explicit `/triad-review`. The real `/review` is preserved rather than downgraded to an old static panel.

## Integration review repairs

Registered skill extraction now uses lexical ranges rather than split/join: only top-level tokens outside quotes and inline/fenced backticks activate skills, and untouched payload bytes (including double spaces in quoted `@` paths) remain intact. A caller predicate reserves `/help`, `/review` and other UI commands without introducing a registry-to-UI dependency. Explicit `$review` still selects that skill. Explicit plan-only/enter-DAG goals keep their outer action and cannot be rewritten into a direct provider call merely because the remaining goal starts `/ask`.

Validated prefix commands emit an internal presentation-only `ValidatedAction` for DAG mode, clear, back and quit. Actual `TuiApp.apply_update` regression proves these local effects and `?` help work without runtime redispatch or provider jobs. A quoted two-space filename reaches the actual compiled plan scope in the plan-only regression.

`/use codex-default` calls the shared bounded native Codex config resolver and then locks a concrete validated Codex model in the TUI profile. It does not parse configuration itself or start a provider job. A real fake-native-server protocol fixture proves one config probe and concrete direct-request routing; explicit MCP raw-alias provenance remains the gateway's separate contract.
