# Preserve inherited provider cancellation evidence

When a DAG campaign's cancellation reaches a provider, the runtime returns a cancelled receipt. The child provider job has a different ID and may still be running in storage. Persisting that receipt through the generic failure path incorrectly changes the child to failed/confirmed_failure.

Add a focused regression that starts a real controlled subprocess under a supervised parent, cancels only the parent, and reads the child job through a fresh Store operation. Add a cancellation transition that retains the result and receipt while preserving existing terminal CAS rules. Route only ReceiptStatus::Cancelled to it. Keep existing job schema, statuses, and explicit cancellation API unchanged. Run the focused gateway test and cancellation storage regressions; the root owns canonical CI and independent review of this repair.
