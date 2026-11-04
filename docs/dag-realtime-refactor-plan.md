# DAG runtime refactor plan

Writer: Technical Lead child agent. Scope: `dag/execution.rs`, `dag/request.rs` and focused DAG regression tests.

Observed defects: the executor waits for every node in a presentation wave before admitting a dependency-ready descendant, and descendants do not receive settled upstream findings. Write lanes already require exclusive access because settlement hashes the full shared workspace.

Contract:

- Keep the compiled DAG, public response fields, model routing and presentation wave indices unchanged.
- Admit a node after all direct dependencies have settled and capacity is available. Unrelated slow read branches do not hold ready descendants behind a presentation layer.
- Bound active work at the configured limit. A write lane owns the shared workspace exclusively; drain read lanes before applying writes. Serial isolation remains serial.
- Forward only settled direct dependency receipts: summary, actual model, manifest identity and executed gate evidence. The same persisted receipt is sufficient when resuming; label summaries as summaries and bound the total prompt through the existing gateway budget.
- Preserve actual event boundaries and distinguish provider STOPPED from gate/manifest SETTLED. Persist checkpoints before exposing settled events.
- On an execution error, stop new admissions and drain already-admitted provider waiters to their bounded durable outcomes. Explicit campaign cancellation and checkpoint persistence failure cooperatively signal active providers/gates and stop later admissions; retain provider job identities and durable outcome rules.

Before changing the scheduler, add controlled readiness/exclusivity and upstream-handoff regression tests. Run focused DAG tests, then canonical `make ci` after integration. Real provider/TUI verification is owned by the main agent.
