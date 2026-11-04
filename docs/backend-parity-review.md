# Backend Python → Rust parity review

Historical baseline: Git `2d5be2d25ed12736c44961534deca0d3ba0db759`, read without restoring Python files or its environment. Completion, command adapters and source-context injection have separate owners. Production ownership in this slice: `routing.rs`, `gateway/contracts.rs`, and parent-authorized bounded guards in `triad/orchestrator.rs`.

## Concrete findings

| Finding | Historical source | Action |
| --- | --- | --- |
| Differential default axes lost for null/empty input | `app/api/codex_router.py::codex_differential_implementations`, `OptimizationAxisKind` | Restored latency/throughput, memory, maintainability defaults; custom axes remain exact |
| Model aliases missing | Model alias enums in `app/core/model_contract_enums.py` | Restored Sonnet/Opus/Argon/Flash and six Codex alias families, Korean/spaced spellings and existing effect-panel spellings |
| Flash canonical identity wrong | Python canonical model and runtime catalogue use Gemini 3.8 Flash | Flash aliases now resolve explicitly to 3.8 high; medium/low canonical models are accepted. Raw requested identity is preserved |
| Provider aliases missing | `RoutingPolicyEngine`: openai/codex, google_antigravity/agy | Added serde aliases to canonical providers, retaining provider/engine mismatch guards |
| Gemini receipt could claim nonexistent xhigh | Python Antigravity route derives tier from model spec | Resolved Gemini model suffix now owns high/medium/low receipt tier; no different model is substituted |
| Cross-mode triad resume fence missing | `TriadExecutionContext.from_inputs` hashes normalized mode | Added normalized mode to task identity and dispatch. Existing records remain readable but old ambiguous identities are not silently reused |
| File workspace not rejected early | Python context factory checks existing directory before invocation | Added orchestrator directory guard before aggregate/provider registration, with negative no-state-created test |
| `codex-default` selection lost | Python CLI route permits configured-default sentinel | Added native effective config/read resolution rather than guessing Astra; Gateway integration preserves raw requested identity, runtime proves concrete actual identity separately |
| Native Git review had no JSONL model proof | Historical review command support; current CLI 0.166 protocol | Switched to supported `codex exec review --json`, with explicit model/effort/sandbox/MCP isolation and captured argv/target/cwd/stdin/session/usage regression; plain output without identity remains failed |
| Engine aliases missing | Python accepts codex-cli, agy, rpc, antigravity-rpc, openai_http | Reported exact spellings to parent/context owner for gateway integration |
| HTTP default changed | Python HTTP default is gpt-6.1-sol | Reported gateway's combined CLI/HTTP Astra default to parent |
| Invalid sandbox leaves false Running job | Installed transport probe, plus gateway options parse order | Reported to parent: sandbox deserialization happens after job registration; invalid input returns tool error but leaves Running/in_flight |

The invalid-sandbox probe used an isolated temporary workspace/state and `CODEX_CLI_BIN=/not-a-provider-sentinel`. It sent one invalid rescue request and queried its local job. No provider was invoked, no home database was touched, and the MCP child exited zero.

## Intentional provider-era difference

Historical Python advertised `minimal`. Read-only inspection of the local Codex 0.166.0 model catalogue found low/medium/high/xhigh/max on current GPT6 profiles, with ultra on some models, and no minimal tier. Keep explicit rejection rather than silently substituting low or pretending support. Current GPT6-only and Claude Medium-only boundaries remain.

## Existing backend mappings

These are source/test mappings; they do not claim fresh live-provider verification.

| Historical contract/tests | Current owner and existing checks |
| --- | --- |
| Strict OpenAI schemas, closed input, 16 MCP tools | `catalog.rs`, `mcp-tools.json`, gateway normalization, real `tests/mcp_stdio.rs` |
| Codex exec/review construction, JSONL parsing | `runtime.rs`; real subprocess and session identity tests |
| Resume/fork session continuity | `codex_session_followup_with_options`; identifier checks and native CLI arguments |
| Provider rejection, actual identity, auth isolation/redaction | `routing.rs`, runtime RPC catalogue proof, HTTP mismatch and auth-noise tests |
| Timeout unknown, cancel, idempotency, CAS | gateway supervisor, Store transactions, process-group timeout/cancel tests |
| Concurrent/durable storage, old SQLite readability | Store cross-process registration test and historical fixtures |
| Handoff pending versus verified completion | typed handoff storage, structured verdict resolution and triad inspection |
| Pipeline/council, bounded repair, prior findings/session | `triad/orchestrator.rs` and repair-prompt continuity tests |
| Review completion separate from consensus | `triad/adjudication.rs`, role snapshots, missing narrative/WARN/BLOCK tests |
| Compact aggregates, scope/tamper fences, exact decision resolution | orchestrator snapshot hashing and `triad/decision.rs` tests |
| Caller tests never promoted to bridge execution | `evidence/builder.rs`, `only_result_side_bridge_receipts_promote_execution` |
| Actual verification process ownership | `triad/gates.rs`, shared `verification.rs`, timeout/descendant/stream hashing tests |
| CodeGraph/source evidence and digest boundaries | `context/source.rs`, readonly SQL reader, evidence slices/symlink tests; current context worker owns range restoration |
| HTTP fallback registration/schema/model/usage/timeout | `http_runtime.rs` local HTTP fixtures plus gateway persistence |
| Background craft/triad, telemetry and retention | gateway background/idempotency tests, Store telemetry/pruning prepared SQL |

## Verification boundary

Focused regressions were added for defaults, aliases/providers, actual Gemini tier, mode separation and invalid workspace. Initial attempts were blocked by intermediate completion/adapter edits and are not counted as passing. Final executed results follow after compilation stabilizes. Parent owns canonical CI, installation, live consumer checks and edits outside the assigned backend files.

### Final executed results for this slice

After concurrent compile errors were resolved, `cargo test -p hcodex --lib routing::tests::` passed 5 tests; the differential-default regression passed 1; `triad::orchestrator::tests::` passed 7. Strict `cargo clippy -p hcodex --all-targets --all-features -- -D warnings` passed. These 13 focused checks include five added regressions. No live provider call was made in this slice. Source is frozen for the parent's independent review and integrated canonical CI.

Parent-authorized runtime follow-up added three native-config protocol/process tests and one actual captured-review-argv regression. They passed individually; later whole-runtime reruns were temporarily blocked by the shared adapter context-usage test signature changing from a pair to a triple. Those blocked reruns are not passing evidence. The real native config/read probe used no provider turn; see `codex-default-resolution-proposal.md` for its precise scope and profile limitation.

Final runtime follow-up: all 21 runtime/HTTP tests passed in 5.02 seconds and strict all-target/all-feature Clippy passed. The retained process-group helper is shared by config preflight and Codex execution. An initially shorter timeout test exposed a parallel-bootstrap fixture race; the final test records PIDs before handshake, uses the actual five-second native cap, requires a timeout error and still checks that no descendant runs afterward.

Native parser validation also found that target flags cannot be combined with positional review instructions: real `codex exec review --uncommitted ... -` exited 2 before dispatch. The repaired targeted path keeps native target flags and uses the verified `developer_instructions` config field, with a 96 KiB limit on the escaped argument and typed InvalidInput before spawning if exceeded. No-target custom review still uses stdin. Captured argv tests cover all targets, precedence, cwd, exact instructions, JSONL model/session/usage, oversized rejection/no child, and refusal to turn plain unproven output into success. Real target-safe native argv parsed successfully and was stopped by a deliberately non-Git workspace preflight; no provider was called.

## Final harness resource diagnosis

The parent's unrestricted canonical test run passed 216/218 library tests but the two positive subprocess fixtures returned TimedOutUnknown at their unchanged two-second deadlines. Each passed unchanged in isolation: native review in 0.46 seconds, exec JSONL in 0.39 seconds. The full runtime/HTTP group passed 21 tests with `--test-threads=4`, and the full workspace/all-target/all-feature suite passed 218 library + 1 main + 2 stdio + 2 xtask tests (223 total) with the same four-thread cap. Library time was 6.29 seconds.

No runtime source, deadline, identity assertion or negative-timeout expectation was changed for this diagnosis. The results are consistent with parallel process/filesystem contention, not a reproduced isolated functional failure. The parent received the minimal recommendation to bound canonical Rust harness concurrency to four, retaining all proof assertions and test coverage. Logs are under `.omx/evidence/python-parity-20261103/`: `review-isolated.log`, `exec-isolated.log`, `runtime-threads4.log`, `full-threads4.log`.
