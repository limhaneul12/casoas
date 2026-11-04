# Python storage compatibility fixture

Generated once from the repository's Python `JobStore` and `HandoffStore` on
2026-11-02. The fixture is isolated test data and contains no user state.

- `jobs/jobs.db` SHA-256: `a6a263188e7f3d900ed65d1dc9dd0c3ce8f56d782c658c278790d2bf41151f96`
- `handoffs/handoffs.db` SHA-256: `e52671738431e8fa2e3a047a341037f7bd70446e408fec0d13c32c5b873ba068`
- Job ID: `cdx-pythonfx`; metadata includes one string and one numeric value to
  prove Rust preserves Python's behavior of retaining only string metadata.
- Handoffs: one pending ticket and one completed ticket whose
  `resolution_note` is the Python-compatible JSON verdict payload
  `{"verdict":"WARN","summary":"python-reviewed"}`.

Tests copy these databases into a temporary directory before opening them, so
the frozen fixtures remain immutable.
