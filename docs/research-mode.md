# Research mode

Research is a separate read-only workflow. The existing development DAG,
worktrees, build gates and three-role Review Council retain their contracts.
Research produces a source-linked report rather than a patch or a code
`SETTLED` receipt.

## Run from the CLI

```bash
hcodex --workspace /path/to/repo research \
  --file src/lib.rs \
  --document docs/spec.md \
  --url https://example.com/spec \
  --request-id research-contract-comparison \
  -- compare the implementation and documented contract

hcodex --workspace /path/to/repo research-status research-contract-comparison
```

A question-only request runs bounded native web discovery and passes the
resulting candidates into the existing research pipeline.

```bash
hcodex --workspace /path/to/repo research -- \
  compare the current Rust 2024 migration guidance
```

`--file` selects repository/code evidence and `--document` selects a local
technical document. Paths are relative to `--workspace`; the reader rejects
escapes, symlinks, nonregular files and credential paths. `--url` accepts
explicit public HTTPS textual sources. `--web` combines declared sources with
discovered sources. `--no-web` disables automatic discovery, including for a
question-only request. `--search-model` enables discovery and selects its model.
Every redirect and resolved address is
checked, and the connection uses the checked addresses. Requests do not use an
ambient HTTP proxy.

Discovery output is a candidate list, not evidence. hcodex accepts only bounded,
credential-free HTTPS candidates, fetches the pages itself, and gives research
stages only the captured source/excerpt records. Search snippets and model prose
never become citations. The configured `max_search_events` bounds distinct native
web-search events observed by hcodex; it cannot claim a count for provider-internal
query batching. The raw search event log is not retained as a new archive.
This statement applies to the hcodex database and report. The Codex CLI may
retain its own provider session under its separate session-retention policy.

V1 does not crawl sites, parse PDFs or prove a historical as-of claim from a
current page. `--no-web` with no explicit sources returns the plan and missing
inputs without fetching sources, invoking models or creating a research record.

Inspect the plan explicitly with:

```bash
hcodex --workspace /path/to/repo research --plan-only \
  --file src/lib.rs -- compare error handling
```

Plan-only does not read the source bodies, call a provider or initialize the
state database. It also does not run automatic discovery, even when the request
would normally enable it. Research commands reject `--worktree-name` before
creating a named workspace. Research can inspect an existing workspace through
`--workspace`; it never creates a worktree for investigation.

## Select research models

The default is one Argon High investigator, a Claude Sonnet Medium challenger
and a Codex 6.1 Sol synthesizer. These are research phase defaults, not a required
three-provider quorum.

```bash
hcodex --workspace /path/to/repo research \
  --document docs/spec.md \
  --investigator gemini-4-argon-high \
  --investigator gemini-4-argon-high \
  --challenger claude-opus-5-5-medium \
  --synthesizer gpt-6.1-sol \
  --max-parallel 2 --timeout-seconds 180 \
  -- compare guarantees and counter-evidence
```

Repeated investigators receive distinct lane and job identities even when they
use the same model. One to three investigators may run within the declared
parallel budget. The challenger follows investigation, and synthesis follows
the challenge. Available models and quotas remain subject to actual routing
receipts; the workflow never silently substitutes another model.

The CLI accepts `--audience human|coding_agent`, `--language`, `--as-of`,
`--max-sources`, `--max-parallel`, `--timeout-seconds`, `--request-id` and
`--idempotency-key`. The backend owns validation and hard resource bounds.
Omitting the execution deadline or setting it to zero leaves model execution
unbounded by time. Web fetches and subprocess startup retain separate I/O bounds.
`--as-of` declares a constraint, not proof that a current capture existed at that
date; unsupported historical evidence remains an explicit limitation.

For per-lane focus/effort and the full source/excerpt budgets, use
`--request-file path/to/request.json` containing the closed `ResearchRequest`.
The file must include `question` and `workspace_dir`; its canonical workspace
must match `--workspace`. Its remaining fields follow the defaults and schema in
[the closed research types](../crates/hcodex/src/research/types.rs).
`--request-file` cannot be mixed with inline question/source/model/budget
options. `--plan-only` is allowed as a safe preview override.

The optional closed `discovery` object controls question-to-source discovery.
Omitting it preserves source-seeded request identity: requests with explicit
sources do not search, while question-only requests receive the bounded default.
Use `{"kind":"disabled"}` to opt out, or configure the web mode explicitly:

```json
{
  "kind": "web",
  "model": "gpt-6.1-sol",
  "effort": "high",
  "max_search_events": 3,
  "max_candidates": 6,
  "min_accepted_sources": 2,
  "timeout_seconds": 60
}
```

For example, the omitted model and budget fields retain their backend defaults:

```json
{
  "question": "Compare the guarantees and unresolved limits",
  "workspace_dir": "/path/to/repo",
  "sources": [
    {"kind": "repository", "path": "src/lib.rs"},
    {"kind": "technical_document", "path": "docs/spec.md"},
    {"kind": "web", "url": "https://example.com/spec"}
  ],
  "investigators": [
    {
      "lane_id": "source-review",
      "focus": "Check source-backed guarantees and counter-evidence",
      "model": "gemini-4-argon-high",
      "effort": "high"
    }
  ]
}
```

## Use from the TUI

```text
/research --file src/lib.rs --document docs/spec.md --plan-only -- compare guarantees
/research --url https://example.com/spec -- assess the documented limits
/research -- compare current public guidance and counter-evidence
/research --no-web --file src/lib.rs -- inspect only the local source
/research-status research-contract-comparison
```

Research slash commands use the same argument-to-request adapter as the CLI.
TUI options use whitespace-separated paths; use the native CLI for quoted paths
containing spaces. Running `/research` does not toggle or alter development
DAG mode, model effect settings or Council behavior.

The research view displays the host-rendered Markdown and saved observation
metadata. Model advisory prose and raw provider output are not used as a
fallback report body.

## Source and report evidence

hcodex directly reads admitted sources once and assigns source/excerpt IDs.
The observed source record contains the locator, UTC capture time, raw-body
SHA-256, media type and size; a bounded excerpt has its own digest and exact
location. Only bounded excerpts and structured research artifacts are retained;
full web bodies and model transcripts are not a new log archive.

For HTML pages, excerpt selection prefers a raw `main` or `article` region
when present, so long navigation markup does not consume the excerpt budget.
The selected text remains an exact slice of the captured body with original
byte and line positions. This is a bounded selection heuristic; hcodex does not
render JavaScript pages or claim complete document extraction.

The first source snapshot digest binds all stage request identities. Reusing an
existing request ID reads the original record without fetching changed source
content or dispatching a new model request. Use a new identity for a new study.

Investigators return source-linked claims. The challenger may retain or downgrade
claims when evidence is contested, unresolved or refuted. Synthesis selects known
claim IDs for report sections. The host renders those claims and citations,
including the unresolved/refuted annex; it does not trust free-form model prose
as verified report text.

The harness checks citation integrity: source/excerpt identity, digest, range,
quote occurrence and reference validity. **Semantic truth and entailment remain
`NOT_MECHANICALLY_PROVEN`.** Agent agreement or extra lanes do not make a claim
true. Current capture time also does not establish historical validity.

## Completion, interruption and readback

Research completion is separate from the durable job state. A finished job may
contain a complete report, an incomplete-evidence report or a partial report.
No valid captured excerpts means no analysis model calls. Missing sources,
budgets, invalid artifacts and provider outcomes remain visible.

A fully refuted hypothesis can be a complete research outcome when captured
counter-evidence and a nonempty conclusion account for it. The host labels the
hypothesis as refuted instead of asserting its text as true. Contested or
unresolved claims keep the report incomplete; completion never proves semantic
truth or exhaustive coverage.

Foreground CLI `Ctrl-C` signals a cancellation latch, stops admission and awaits
the same owned run while admitted children drain. TUI shutdown signals its
owning Gateway supervisor. The parent retains a bounded partial report and
original stage IDs. A timeout or unknown provider result is not retried under a
new request ID. Research status from a different process is observational and
does not claim to cancel another process's supervisor.

`research-status` reads existing state through the diagnostic read-only path.
It does not initialize a database, reconcile stale jobs, fetch sources, invoke
models or resume the run.

## Dedicated harness and verification

The policy, manifest and fixed corpus live in
[the project research assets](../crates/hcodex/research-assets.toml).
The runtime validator is also the corpus validator. `make harness` checks
harness/fixture hashes and `make ci` executes deterministic local tests. Neither
requires a web connection, a model call, a human judge or an API key.

The benchmark basis and adopted/rejected mechanisms are recorded in
[the reference comparison](research-reference-benchmark.md). Optional live tests
are separate capability evidence: they record actual model/job receipts,
captured sources, latency and usage. They do not imply a measured research
quality or speed improvement over another system.
