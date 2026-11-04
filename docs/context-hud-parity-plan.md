# Context HUD and completion display restoration

The Python HUD always put context beside the composer, but used a hard-coded 200,000-token limit and accumulated/estimated tokens. Preserve visibility and useful context feedback without reinstating invented model-window values or fake compaction.

- Keep exact model context percentage when a runtime receipt provides current context and window.
- Also display the actual grounded request bytes / admitted byte budget from the runtime envelope, including when a provider does not expose its model window. Label request bytes separately from model tokens.
- Put context, runtime token usage, selected skills and runtime configuration on visible status rows rather than after a long clipped prefix.
- Restore the descriptive completion menu and ghost text using the completion owner's command metadata; window the menu around the selected item and preserve composer position/Unicode cursor behavior.
- Regression checks must prove context remains visible on 80-column terminals, missing provider metrics are not rendered as 100% free, actual request-byte evidence is shown, selected completion remains on screen, and ghost text does not enter submitted text.

Root owns render.rs, TokenUsage display/fields and terminal_adapter usage extraction helpers only. Other agents own input catalog/state, source grounding, adapter command semantics, backend aliases and exhaustive historical traceability.
