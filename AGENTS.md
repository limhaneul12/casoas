# Repository Agent Authority (`codex-collab-mcp`)

Before modifying code in this repository, read and follow the materialized Alexandria Python Engineering Harness under `.agents/`:

1. [`.agents/README.md`](./.agents/README.md)
2. [`.agents/python_dev_harness/PROJECT_PROFILE.md`](./.agents/python_dev_harness/PROJECT_PROFILE.md)
3. [`.agents/python_dev_harness/HARNESS.toml`](./.agents/python_dev_harness/HARNESS.toml)
4. [`.agents/python_dev_harness/rules/README.md`](./.agents/python_dev_harness/rules/README.md)

## Non-Optional Invariants

- Built-in `getattr()`, `hasattr()`, and `setattr()` are **strictly forbidden** in production source (`src/`).
- Every function and method in production source (`src/`) must have explicit parameter and return type annotations (`__init__ -> None`) with zero bare collections (`list`, `dict`, `tuple`, `set`, `frozenset`) and zero unapproved `Any`.
- Symbolic string sets, path/config segment constants, and any `Literal[...]` with $\ge 3$ string items must use `StrEnum` in a `*_enums.py` module (`class X(str, Enum)` is forbidden).
- Do not pass known dictionary shapes as raw `dict[str, Any]` / `dict[str, object]`:
  - External I/O boundaries use **Pydantic v2** (`StrictSchemaModel`: `ConfigDict(extra="forbid", frozen=True, use_enum_values=True, validate_default=True)`).
  - Validated internal I/O / DTOs use **`@dataclass(frozen=True, slots=True)`**.
  - Every `to_dict(self)` method on internal DTOs must return an explicit **`TypedDict`**.
- Avoid generic or stdlib-colliding module names (`json.py`, `http.py`, `utils.py`, `helpers.py`, `common.py`, `misc.py`, `models.py`, `types.py`, `schema.py`, `manager.py`).
- Every function and method in production source (`src/`) must have a Google-style docstring (`"""... Args: ... Returns: ..."""` / `Yields:`).
- Cohesion & Size Limits:
  - Modules with $\ge 6$ functions of the same category/responsibility must group them into a cohesive Class.
  - Every Class in `src/` must have **at most 6 behavioral methods** (excluding `__init__`, dunders, `@property`, and Pydantic validator hooks).
  - Every module in `src/` must have **at most 430 structural lines** (excluding docstrings), and every package directory must have **at most 10 direct `.py` modules**.
- Async functions must never call blocking `subprocess.*`, `time.sleep()`, or unowned `asyncio.create_task()`.

## Canonical Verification

Run all 9 mechanical contract verifiers and quality gates before reporting completion:

```bash
uv run python .agents/python_dev_harness/verify_mechanical_contracts.py
uv run ruff check src tests
uv run pyrefly check
uv run pytest
```
