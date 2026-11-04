# Bounded backend parity repairs

Baseline: historical Python at `2d5be2d25ed12736c44961534deca0d3ba0db759`, read through Git without restoring Python files. Scope: `gateway/contracts.rs` prompt defaults and `routing.rs` model normalization only. Other workers own gateway context assembly, adapters and completion.

1. Restore differential synthesis's three default optimization axes for omitted/null/empty lists; preserve explicit custom axes. Historical `app/api/codex_router.py` used `optimization_axes or default_axes`.
2. Normalize Flash aliases to the actual historical/current Antigravity catalogue family `gemini-3.8-flash-{high,medium,low}`, including the erroneous Rust `gemini-4-flash` spelling as a visible alias to high. Preserve raw requested model in receipts; actual identity still comes from runtime catalogue proof. Add the advertised Argon low canonical spelling.
3. Keep `minimal` rejected for GPT 6/6.1. The local Codex model catalogue (`~/.codex/models_cache.json`, read-only) lists low/medium/high/xhigh/max, with ultra on some models, but no minimal on any current GPT 6 profile. Do not silently substitute low or advertise unsupported effort. Record this as a deliberate provider-era difference from the historical Python vocabulary.

4. Restore the remaining historical model/provider aliases and Gemini canonical-model tier authority. Preserve raw requested identity and current provider/engine/Claude gates.
5. Parent-authorized additional slice: fence normalized triad mode in the source task digest and reject non-directory workspaces before aggregate/provider registration. Old records remain readable; unsafe ambiguous resume reuse is rejected.

Add focused regression tests before changes, run them and strict Clippy. Parent owns final CI and live consumer verification. No provider call, credential mutation, dependency, public MCP schema or persisted-data rewrite is authorized in this slice.

Result: restored differential defaults, historical model/provider aliases, canonical Gemini tier, triad mode digest fence and early workspace-directory validation. Thirteen focused checks and strict all-target/all-feature Clippy passed. The earlier blocked compile attempts were not passing evidence. Remaining gateway-boundary findings are recorded in `backend-parity-review.md` and delivered to the parent owner.
