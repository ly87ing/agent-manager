# Style Guidelines

Apply this rule whenever you write code, touch config or data contracts, or add comments.

## Required Checks

- Use descriptive names that reflect business meaning. Avoid single-letter names and unclear abbreviations outside tiny local scopes.
- Keep comments rare and high signal. Explain invariants, edge cases, or tradeoffs, not obvious line-by-line behavior.
- Validate external input and config fail-close. Reject malformed or unsupported values instead of silently coercing them unless an existing contract requires compatibility.
- Preserve the repository's established formatter, lint, and type-check expectations. New warnings introduced by your change count as regressions.

## Before Completion

- Re-read the diff for accidental churn, dead code, debug leftovers, and misleading names.
