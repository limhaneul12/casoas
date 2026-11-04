# Native `codex-default` resolution proposal

The read-only investigation below was accepted by the parent. Implementation is now in `runtime/config.rs`, with trusted requested-model preservation in `ExecutionOptions` and Gateway integration owned by the context worker. No public MCP field was added.

## Primary local authority

Installed Codex CLI 0.166.0 exposes `codex app-server --stdio` and `config/read`. Its own `app-server generate-json-schema` output defines `ConfigReadParams.cwd` as the directory used to resolve project configuration layers, and the response includes effective `config.model`, `config.model_provider`, model reasoning effort and per-key origins. The [official app-server API](https://learn.chatgpt.com/docs/app-server#api-overview) also documents `config/read` as effective on-disk configuration after layering.

A fresh local probe sent only initialize, initialized and config/read, with `explicitGatewayOauth=true`. It started no thread or turn and dispatched no model request. For this checkout the response was:

- Effective model: `gpt-6.1-sol`; reasoning: `xhigh`; model provider: null/default.
- Model origin: project `.codex`, version `sha256:77e62402b39086711f67cab46a3166dd9af613b7449d67711234d53c38142b78`.
- User layer model was `gpt-6.1-terra`, proving that reading only the global TOML would resolve the wrong model.
- The ephemeral native process exited cleanly in roughly 0.46 seconds. Full configuration contents were not logged.

## Bounded implementation contract

1. Only when the caller explicitly requests the `codex-default` sentinel, run one supervised native config/read probe before job registration/provider dispatch. Use the configured Codex executable and the canonical target workspace, preserving CODEX_HOME and native managed/user/project precedence.
2. Bound startup, reads and total duration; bound stdout size. Send no thread/start, turn/start, login or configuration mutation. Parse only the required effective-model/provider/origin projection; never log the entire configuration or credentials.
3. Require a nonempty concrete current GPT6 model. Reject a null/missing model, recursive sentinel, unsupported model, native configuration error, malformed response or timeout before provider dispatch. Do not guess Astra or a catalogue default.
4. Preserve raw `requested_model=codex-default` and record the native config origin/resolved concrete ID. Execute with explicit `-m <resolved model>`; runtime receipts must still prove the actual model independently. The existing `--ignore-user-config` execution isolation can remain after the configured choice has been resolved explicitly.
5. If the effective model provider is custom, do not silently drop it by executing under ignored user configuration with OpenAI's provider. Either support that provider through the existing explicit engine contract or reject this sentinel resolution with a clear pre-dispatch configuration error.
6. Keep the caller/tool reasoning-effort argument authoritative; historical tool defaults already explicitly passed that setting. A model-default sentinel is not permission to silently replace an explicit effort.

Gateway can resolve an effective concrete request before validation and preserve raw requested identity when finalizing the receipt/decision. Keep filesystem/process effects out of the pure routing normalizer. Avoid a new public MCP field or persisted schema change merely to carry the internal resolution proof.

## Profile boundary

Current hcodex has no explicit Codex profile argument; historical hcodex exec likewise passed none. Native Codex rejects `-p <name> app-server` because app-server is not among its profile-enabled runtime commands. A synthetic legacy `profile="speed"` configuration made config/read return an explicit unsupported-legacy-profile error directing callers to `--profile speed` with `speed.config.toml`. Do not ignore that error or guess which profile an unrelated parent Codex session used.

`codex debug prompt-input` accepts a named profile but returns only model-visible input items, not a selected model, so it is not model-identity proof. Named runtime-profile support would need a separately verified read surface or an explicit reviewed profile-resolution contract. This proposal covers native effective configuration with no unexposed profile flag and fails closed for unsupported/ambiguous profile settings.

## Minimum acceptance proof

- Managed/user/project native precedence with differing models, including an untrusted/disabled project layer.
- Sentinel requested identity preserved while the concrete resolved model and actual receipt agree.
- Missing model, unsupported/current-non-GPT6 model, custom provider, native config error, oversized output, timeout and closed child pipe all reject before dispatch and leave no false Running job.
- Config read never starts a provider turn; explicit models bypass the probe.
- Real local config/readback for this checkout remains separate from fake-binary negative tests and later actual consumer verification.

## Implementation evidence

The native reader has a five-second total cap, 512 KiB stdout and 64 KiB diagnostics bounds, inherited environment/config precedence and no thread/turn invocation. Errors, invalid models, custom providers and discarded OpenAI provider overrides fail closed. The original child process-group identity is retained through leader exit; runtime and probe share kill/reap cleanup.

Three fake-executable tests passed for exact initialize/initialized/config-read protocol, canonical model plus raw requested alias without invented actual identity, missing/unsupported/custom provider inputs, closed/oversized output and timeout with a descendant holding pipes after leader exit. The Gateway owner separately passed sync/async sentinel dispatch and explicit-model bypass using captured fake native config and exec output. Final integrated CI and installed native probing remain parent-owned evidence.

The final runtime/HTTP suite passed all 21 tests; strict all-target/all-feature Clippy passed. Default preflight and Codex execution reuse the same retained-process-group cleanup helper. No named profile support or implicit model fallback has been invented.

## Actual native transport timestamp correction

The parent's first installed native probe rejected a real startup notification carrying `emittedAtMs`, before registering a job or invoking a provider. The transport envelope now declares that optional field as `u64` while retaining `deny_unknown_fields`. Fake native fixtures include timestamped notifications and responses; malformed timestamps and unrelated unknown metadata still reject. All four config tests and strict all-target/all-feature Clippy passed.

A freshly built debug hcodex then completed the real native configuration read using Codex 0.166.0. Its deliberately tiny input budget stopped before model dispatch: requested `codex-default`, resolved `gpt-6.1-sol`, actual null, durable job status `input_budget_exceeded`, and project `.codex` origin/hash matched. Evidence: `.omx/evidence/python-parity-20261103/native-config-timestamp-debug.json`. This debug-source proof is separate from the parent's subsequent release installation/readback.
