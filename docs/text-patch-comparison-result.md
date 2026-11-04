# Text patch: actual model comparison — 2026-11-04

## Result

On three predefined literal-edit tasks in the same real 210-line Rust module,
both output formats produced the exact expected bytes (3/3 each).
Hunk output used **444 bytes versus 23,180 bytes**, a **98.08% reduction** in
the UTF-8 `crafted_code` JSON payload. Three provider-call durations summed to
**14.590 s versus 63.466 s**, **77.01% shorter in this pilot**.

This is evidence for smaller returned payloads and observed latency on these
small edits, not a general throughput, reliability, token-cost or production claim.

| Task | Whole-file payload | Hunk payload | Whole-file duration | Hunk duration | Exact bytes |
|---|---:|---:|---:|---:|---|
| One constant | 7,725 B | 98 B | 22.060 s | 4.396 s | Both exact |
| Two disjoint literals | 7,724 B | 208 B | 20.598 s | 5.508 s | Both exact |
| Three-line block replacement | 7,731 B | 138 B | 20.808 s | 4.686 s | Both exact |

The one-constant whole-file request was a **confirmed bridge schema failure**:
a forbidden `summary_note` was returned in the outer review response. Its
recovered code payload was diagnostically exact, but the role remains failed.
Bridge-accepted exact responses are **2/3 whole-file versus 3/3 hunks**.
This outer-schema error is not attributed to the whole-file patch format.
There were no retries or repairs.

Sensitivity check excluding that failed pair: the two fully accepted pairs
sum to **41.406 s whole-file versus 10.194 s hunks** (75.38% shorter).
This remains a descriptive two-pair observation, not statistical significance.

## Method fixed before dispatch

- Actual model: `claude-sonnet-5-5-medium`; all six actual/requested identities match.
- Full frozen source was supplied to both arms with identical 1-based line listings.
- The task and expected literal replacements were fixed before the first call.
- Order: full/hunks, hunks/full, full/hunks; calls were sequential.
- Exactly one call per arm per task; one model, one source file, one session.
- Whole-file response uses existing closed `PatchArtifact` with the exact source
  path and preimage hash. Hunk proposals contain only original line ranges and
  replacement text; the host derives range hashes from the frozen source, then
  invokes the actual `TextPatch::compile` implementation.
- Comparisons run in memory. No candidate is applied to the working source.
- Source SHA-256: `sha256:b6c9973919a4e9f6b03bdfed3259f27f48da1314c809e8b65a02cde26268e5de`.
- Plan SHA-256: `sha256:dae9414b655003c88e406eaaf8c123710724ff8b25a2de0208bb996da32fe6cc`.

## Measurements and evidence limits

`payload_bytes` includes JSON field names, escaping and formatting inside
`crafted_code`; it is not token count or just changed source bytes.
Durations are provider runtime receipts, not compilation time or full user-task
completion time. Input bytes were **77,808 whole-file versus 78,294 hunks**;
the complete input source remained, and the hunk instructions were slightly longer.
Antigravity reported zero token counters; these are treated as **unavailable**,
not zero consumption. No monetary or token reduction is calculated.

The three tasks are small, exact textual substitutions. There are no repeated
samples, confidence intervals, randomized balanced ordering, complex refactors,
multi-file tasks or different models. Do not infer general accuracy superiority
from a 3/3 result or the one outer-schema failure.

## Reproducible evidence

Raw provider receipts, exact dispatched requests reconstructed from the dispatch
template, frozen source/expected files, the original plan, samples and machine
report are in:

`/Volumes/256.SSD/RustBuild/hcodex-text-patch/comparison-20261104`

The Rust replayer verifies:
- source digest/byte count and requested model against the frozen plan;
- all six unique cells in the original dispatch order;
- expected bytes reconstructed from the planned unique literal substitutions;
- sample fields against original bridge receipts;
- numbered source in each request and context+task digest against the bridge's
  `inline_context_digest` algorithm;
- exact whole-file identity, closed response schemas, and actual compiled hunk output.

Each result row records raw receipt and expected-output SHA-256 digests.
Failed bridge roles remain failed even when their recovered payload is evaluated.

Replay:

```sh
CARGO_TARGET_DIR=/Volumes/256.SSD/RustBuild/hcodex-text-patch \
cargo run -p hcodex --example text_patch_compare -- \
  /Volumes/256.SSD/RustBuild/hcodex-text-patch/comparison-20261104
```

Replayer control tests include wrong path, wrong preimage hash and an invalid
range; these are rejected without repair or fallback. No product prompt,
parser, write authority or installed binary was changed.

## Validation

Final Rust replay exited 0 with all six cells verified. Independent read-only
review recomputed the aggregates and checked plan/source/expected/receipt hashes.
The final `make ci` exited 0: rules/manifest integrity, format, workspace check,
strict all-target/all-feature Clippy and all 246 tests passed, zero ignored.
Log: `/Volumes/256.SSD/RustBuild/hcodex-text-patch/ci-text-patch-comparison-final-verified.log`.
The production compiler hash still equals the frozen source hash above.
