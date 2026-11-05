# hcodex Research Harness Rules v1

This repository-owned research profile is adapted from pinned OMH, Open Deep
Research, STORM and research-evaluation references. It supplements the Rust
normal, type and async rules; it does not change coding DAG or Review Council.

## R-INPUT — Closed brief and bounded effects

A research request declares question, audience, language, sources, models and
finite budgets. It grants no workspace writes, executable verification command,
sandbox expansion or automatic retry. A question-only request defaults to one
bounded native Codex web-discovery turn; explicit sources do not add discovery
unless requested. Discovery can be disabled, in which case a source-free request
returns the missing-input plan. Plan-only requests have no provider, retrieval
or database effects. Only explicit local text files and credential-free public
HTTPS URLs accepted after host acquisition enter the evidence ledger.
Unsupported and failed sources remain explicit gaps. Current captures never
prove historical as-of validity.

## R-DISCOVERY — Candidate discovery is not source evidence

Discovery runs in a fresh empty scratch directory with a read-only sandbox and
known shell, file-execution, MCP and multi-agent feature paths disabled. The host
accepts only exact native `web_search` item events, deduplicated by item ID, and
requires at least one completed event, confirmed actual-model identity and owned
process-group closure. Unexpected tool events, event/byte/time budget exhaustion,
cancellation and unknown process closure fail closed under the original IDs.
The default observable event cap is three and the hard cap is six; this does not
claim a bound on provider-internal query expansion. Candidate URL output is
bounded to six by default and twelve at most.

Search output contains candidate credential-free HTTPS URLs only. Every candidate
must normalize to a `results[].url` observed on a completed `web_search` item;
model prose cannot add candidates. hcodex does not copy raw queries, snippets or
reasoning into its database or report, and never treats them as evidence or
citations. Native Codex sessions may retain provider evidence under their own
policy. The host independently validates redirects and public addresses,
retrieves the body and creates source/excerpt records. Candidate fetch failures
are retained as optional discovery rejections; required explicit-source failures
remain required failures. Investigation starts only after the configured
accepted-source minimum is met.

## R-SOURCE — Observed source records

The host assigns source/excerpt identities after actual bounded acquisition.
Record locator, kind, capture time, body digest/size/media type, redirect chain
and a bounded excerpt with exact digest/range. Persist excerpts, not full raw
bodies or private transcripts. Validate path confinement and source identity;
validate every HTTPS redirect and public DNS address, pin addresses and disable
ambient proxies. For HTML web sources the host may prefer a raw `main`/`article`
region when selecting an excerpt, keeping exact original body byte/line ranges
and hashes. This bounded heuristic does not render pages or prove semantic
coverage. Local code and plain text use the ordinary question window. A URL
proposed by discovery is not an observed source until host acquisition completes.

## R-CLAIM — Evidence linkage and explicit uncertainty

Claims use supported, contested, unresolved or refuted assessments. Supported and
contested claims require captured support; refuted claims require captured
counter-evidence. Resolve each source/excerpt ID and verify quote occurrence in
that exact excerpt. Duplicate identities, digest/range drift and dangling
references fail closed. Distinct URLs do not mechanically prove independent
publishers. Quote fidelity is not semantic entailment or factual truth:
`semantic_verification=NOT_MECHANICALLY_PROVEN`.
Support and counter interpretations of the same quote may both remain visible;
repeating that quote with the same stance is not additional evidence.

## R-FANIN — Parent owns finality

Investigator workers return claims only. The challenger may downgrade known
claims and supply captured counter-evidence; it cannot create supported facts.
It must assess every prior claim exactly once, with a bounded reason. Keeping
the same assessment is allowed; an advisory summary cannot replace claim-level
review. Missing, repeated or foreign claim IDs fail closed.
The synthesizer returns section headings and existing claim IDs only. Free-form
summaries are agent-reported advisory, never the evidence-backed factual body.
The host renders claim text and citations, marks contested findings and retains
unresolved/refuted/omitted findings. A refuted hypothesis with captured counter
quotes may appear in a conclusion, always prefixed `REFUTED HYPOTHESIS / not
asserted`, and remains in the refuted annex. Synthesis accounts for every
supported, contested and refuted finding; unresolved findings remain annex-only.
Agreement and worker count are not truth.

## R-LIFECYCLE — Stable identity and partial outcomes

Parent identity binds the exact brief, explicit source locators, models, budgets
and the materialized discovery configuration. Discovery has a stable ID derived
from the parent and versioned effective configuration; persist its completed
receipt and canonical candidate URLs before fetching them. Capture sources once
after admission; bind each stage ID to parent, source snapshot and phase/lane.
Same-ID reads do not rediscover, reacquire or redispatch. Cancellation
stops admission, signals and drains owned tasks, and persists partial evidence.
Timeout/unknown retains original IDs and is never success. The workspace must
remain unchanged before a successful report is accepted. Research completion is
closure of the bounded workflow, not confirmation of its hypotheses. Refuting a
hypothesis can be a completed outcome. Completion is not coding SETTLED, build
proof, consensus approval or deployment authority.

## R-VERIFY — Deterministic canonical checks

Runtime and fixed-corpus tests use one Rust validator. Canonical CI checks closed
schemas, source/excerpt integrity, claim linkage, assessment/section rules,
budgets and no-write boundaries with local fixtures. It runs no live network,
provider, model judge or human review. Live capability and semantic-quality
review are separately labelled, with actual receipts and budgets. All skipped
checks stay unverified; no external benchmark score becomes hcodex proof.
