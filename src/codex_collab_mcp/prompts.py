"""Prompt builders tailored for Gemini 4 Argon <-> Claude <-> OpenAI Codex heterarchy workflows."""

from __future__ import annotations

from codex_collab_mcp.collab_enums import CanonicalModelId


def build_adversarial_audit_prompt(
    target_code_or_diff: str,
    design_rationale: str,
    attack_vectors: list[str],
    context_notes: str = "",
) -> str:
    """Build a red-team adversarial audit prompt for OpenAI Codex.

    Args:
        target_code_or_diff: Source code or unified diff under review.
        design_rationale: Architectural intent and invariants from Gemini 4 Argon.
        attack_vectors: Priority failure modes and attack vectors to probe.
        context_notes: Optional supplementary repository context.

    Returns:
        Formatted adversarial audit prompt string.
    """
    vectors_str = (
        ", ".join(attack_vectors)
        if attack_vectors
        else (
            "concurrency/race-conditions, boundary values, security/injection, "
            "resource/memory leaks, error-propagation, type/contract violations"
        )
    )
    rationale = (
        design_rationale
        or "No explicit design rationale provided; infer intended invariants from the code."
    )
    return f"""[ROLE: CODEX ADVERSARIAL RED-TEAM AUDITOR]
You are collaborating in a Heterarchy architecture alongside Gemini 4 Argon (the macro-architect).
Gemini 4 Argon has designed or modified the following code based on its global repository context.
Your mission is NOT to be polite or rubber-stamp the code.
Act as an uncompromising Devil's Advocate and Red-Team Auditor.

### 1. Architect's Design Rationale & Invariants (from Gemini 4 Argon)
{rationale}

### 2. Priority Attack Vectors to Probe
{vectors_str}

### 3. Additional Repository Context
{context_notes or "N/A"}

### 4. Target Code / Diff Under Audit
```
{target_code_or_diff}
```

### Instructions
1. Hunt for concrete failure modes: off-by-one errors, unhandled exceptions, partial state
   mutations, race conditions, silent data corruption, security bypasses, or performance cliffs.
2. Do not report superficial style nitpicks unless they cause real bugs.
3. For every flaw found, provide the exact trigger scenario and a concrete failing test case
   that proves the vulnerability.
4. Set `verdict` to:
   - `BLOCK` if any CRITICAL or HIGH severity flaw exists.
   - `WARN` if only MEDIUM/LOW risks or edge-case hardening items exist.
   - `PASS` only if the implementation rigorously upholds all invariants.
5. Synthesize an `alexandria_decision_note` summarizing the audit outcome for durable storage
   in Heterarchy Alexandria.
6. CRITICAL: Do NOT invoke any shell commands (`rg`, `cat`, `sed`, `pytest`, etc.) or external
   tools. Perform a pure static red-team audit using ONLY the supplied Target Code / Diff and
   CodeGraph Sliced Symbol Context above, and emit your structured JSON output immediately.
"""


def build_targeted_synthesis_prompt(
    signature: str,
    contract_and_constraints: str,
    context_types: str,
    language: str,
    include_unit_tests: bool = True,
) -> str:
    """Build a high-precision code synthesis prompt for OpenAI Codex.

    Args:
        signature: Target function, method, or class signature.
        contract_and_constraints: Pre/post-conditions and invariant rules.
        context_types: Supporting type definitions and imports.
        language: Target programming language name.
        include_unit_tests: Whether to synthesize an accompanying unit test suite.

    Returns:
        Formatted targeted synthesis prompt string.
    """
    test_instruction = (
        "Generate a comprehensive unit test suite covering happy paths and boundary conditions."
        if include_unit_tests
        else "Return an empty string for `unit_tests` unless a minimal smoke test is essential."
    )
    return f"""[ROLE: CODEX HIGH-PRECISION SYNTHESIS ENGINE]
You are operating as the micro-precision implementation engine paired with Gemini 4 Argon.
Gemini 4 Argon has distilled the global repository context into the exact specification below.
Synthesize zero-fluff, production-hardened `{language}` code that strictly satisfies the contract.

### 1. Target Language
{language}

### 2. Target Signature / Interface
```
{signature}
```

### 3. Contract, Pre/Post-Conditions & Constraints
{contract_and_constraints}

### 4. Supporting Context Types & Imports
```
{context_types or "// Standard library types only"}
```

### Instructions
- Produce complete, directly usable implementation code with zero placeholders or `TODO`s.
- Respect all type annotations, error handling rules, and performance constraints.
- {test_instruction}
"""


def build_differential_candidates_prompt(
    problem_spec: str,
    optimization_axes: list[str],
    context_types: str,
    language: str,
) -> str:
    """Build a multi-axis differential algorithm exploration prompt for OpenAI Codex.

    Args:
        problem_spec: Algorithmic or architectural problem description.
        optimization_axes: Distinct optimization dimensions to generate candidates for.
        context_types: Supporting type definitions and constraints.
        language: Target programming language name.

    Returns:
        Formatted differential candidates prompt string.
    """
    axes_formatted = "\n".join(
        f"- Axis {i + 1}: {axis}" for i, axis in enumerate(optimization_axes)
    )
    return f"""[ROLE: CODEX DIFFERENTIAL ALGORITHM EXPLORER]
You are collaborating with Gemini 4 Argon to explore distinct implementation trade-offs
for a critical system component.
For each requested optimization axis, synthesize a complete, distinct candidate in `{language}`.

### 1. Problem Specification
{problem_spec}

### 2. Required Optimization Axes (Generate 1 Candidate per Axis)
{axes_formatted}

### 3. Context Types & Constraints
```
{context_types or "N/A"}
```

### Instructions
- Make each candidate genuinely distinct in algorithmic or structural approach.
- Explicitly analyze pros, cons, and Big-O complexity for each candidate.
- In `argon_selection_guide`, provide a sharp decision matrix so Gemini 4 Argon can choose
  the best candidate based on global system telemetry and architectural priorities.
"""


def build_rescue_prompt(
    error_trace: str,
    target_code: str,
    previous_attempts: str,
    reproduction_command: str = "",
) -> str:
    """Build an isolated root-cause rescue diagnosis prompt for OpenAI Codex.

    Args:
        error_trace: Stack trace or failing test output.
        target_code: Source code under investigation.
        previous_attempts: Summary of prior hypotheses or failed fixes.
        reproduction_command: Optional CLI command that reproduces the failure.

    Returns:
        Formatted rescue diagnosis prompt string.
    """
    return f"""[ROLE: CODEX DEEP RESCUE & ROOT-CAUSE SPECIALIST]
Gemini 4 Argon has encountered a persistent bug or test failure after initial attempts
 and is delegating isolated root-cause analysis and patch synthesis to you.

### 1. Error Trace / Failing Test Output
```
{error_trace}
```

### 2. What Was Already Tried (Avoid Repeating Failed Hypotheses)
{previous_attempts or "No prior attempts logged."}

### 3. Reproduction Command
`{reproduction_command or "N/A"}`

### 4. Target Source Code / Context
```
{target_code}
```

### Instructions
1. Identify why the previous attempts failed and pinpoint the true root cause.
2. Provide a clean, minimal, verifiable fix (as unified diff or drop-in replacement code).
3. List exact verification commands to confirm the fix resolves the issue without regressions.
"""


def build_claude_craft_prompt(
    task_spec: str,
    target_code_or_context: str = "",
    invariants_and_rules: str = "",
    language: str = "python",
    claude_model: str = CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM,
) -> str:
    """Build a structured JSON prompt for Claude Sonnet 5.5 / Opus 5.5 code craft and review.

    Args:
        task_spec: Architectural specification and task instructions.
        target_code_or_context: Existing code or surrounding module context.
        invariants_and_rules: Repository invariants and harness rules to uphold.
        language: Target programming language name.
        claude_model: Requested Claude model identifier (`sonnet` or `opus` Medium).

    Returns:
        Formatted Claude code craft prompt string.
    """
    rules_text = (
        invariants_and_rules
        or "Write clean, strictly typed, idiomatic production code with zero placeholders."
    )
    is_opus = "opus" in claude_model.strip().lower()
    role_banner = (
        "[ROLE: CLAUDE OPUS 5.5 DEEP ARCHITECTURAL & SURGICAL CODE CRAFTSMAN]"
        if is_opus
        else "[ROLE: CLAUDE SONNET SURGICAL CODE CRAFTSMAN & REFACTORING SPECIALIST]"
    )
    node_label = (
        f"opus (`{CanonicalModelId.CLAUDE_OPUS_5_5_MEDIUM}`)"
        if is_opus
        else f"sonnet (`{CanonicalModelId.CLAUDE_SONNET_5_5_MEDIUM}`)"
    )
    craftsman_desc = (
        "- `Claude Opus 5.5 (Medium)`: Deep Architectural Synthesis, Complex Refactoring "
        "& High-Assurance Code Craftsman"
        if is_opus
        else "- `Claude Sonnet 5.5 (Medium)`: Surgical Code Craftsman, Multi-File Refactorer "
        "& Ergonomic API Designer"
    )
    return f"""{role_banner}
You are operating as the precision Code Craftsman node ({node_label}) in a Tri-Polar Heterarchy:
- `Gemini 4 Argon`: Macro-Architect & Long-Context Orchestrator
{craftsman_desc}
- `OpenAI Codex`: Adversarial Red-Team Auditor & Edge-Case Breaker

Your output will be inspected by Gemini 4 Argon and audited by OpenAI Codex.

### 1. Task Specification & Architectural Intent
{task_spec}

### 2. Invariants & Repository Engineering Rules
{rules_text}

### 3. Existing Code / Context (`{language}`)
```
{target_code_or_context or "// Net-new implementation requested"}
```

### Required Output Format (Strict JSON Only)
Respond with ONLY a single valid JSON object (no markdown prose outside JSON) matching:
{{
  "verdict": "PASS | WARN | BLOCK",
  "summary": "Concise summary of the surgical implementation or refactoring decisions",
  "crafted_code": "Complete, drop-in production code in {language}",
  "design_highlights": ["Key structural or ergonomic improvements made"],
  "edge_cases_guarded": ["Specific boundary conditions or invariants protected"],
  "handoff_notes_for_codex_audit": "Specific invariants OpenAI Codex should red-team verify"
}}

Choose `PASS` only when the requested contract is met without an identified risk, `WARN` when
the result has a material limitation or unresolved edge case, and `BLOCK` when a correctness,
security, or contract defect prevents safe use. Always provide this explicit verdict; do not
imply it through the summary, code, or handoff notes.
"""
