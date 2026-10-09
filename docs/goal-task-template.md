# /goal Task Templates

> These are generic historical templates, not current project requirements or authorization. The native Rust redesign uses [goal-rust-gui.md](goal-rust-gui.md). Fixed review counts, byte-for-byte compatibility, and repeated full tests below do not automatically apply to this project.

These patterns were extracted from historical /goal task documents through a full session review in 2026-07. Choose the template for the task type; all types share the common structure.

## Common structure

```text
Goal: <one-sentence end state>
Quality bar: <audience-level standard, e.g. "expert-level AI material, substantive, grounded in theory and practice, understandable when read or presented">
Self-review: review and improve at least N times (N≥3 for important deliverables, N≥10 for high-value material), until the quality bar is met
Cadence: commit and push changes promptly; record milestones in project documents rather than leaving them only in conversation
```

## Review tasks (all four elements required)

1. **Criteria first:** enumerate audience × duration × local hard rules, and review each criterion.
2. **Name primary sources for numbers:** give the source path for each number requiring verification (inspect session jsonl entries, measure media duration with ffprobe, count prototype HTML). Do not conclude from the deliverable alone.
3. **Resolved items to omit:** list deliberate or acknowledged decisions with commit hashes, such as "the user intentionally deleted X in 2a843ad; do not suggest restoring it," to prevent repeated noise.
4. **Required report format:** file + line + current state + **specific change**. Omit claims that something could be better without a concrete correction; avoid empty phrases such as "clearer." Specify the sorting dimension.

For iterations, name dimensions that produced no confirmed conclusion last round and make them an independent focus.

## Implementation tasks

- List prohibited changes for the project, such as broadening vocabulary, changing existing delimiters, or adding generic terms.
- Reuse existing mechanisms; add no abstraction.
- Pair every new rule with a negative test proving it did not broaden behavior.
- Keep existing output byte-for-byte unchanged; run tests twice with all passing.
- Reconcile committed state before reporting real additions; explicitly flag behavior changes.
- Do not commit without an instruction.

## Batch classification tasks

- Fix the verdict + pattern enums and classify every item.
- Verify 1:1 coverage with no omissions, additions, deletions, or duplicates; save JSON at the prescribed path.
- Predefine rules against false positives, such as "a batch label is not a security defect."
- Each implementer reports judgment calls worth flagging.

## Traceability and verification tasks

- Give each assertion a three-level verdict (matches / partially mismatches / seriously mismatches), plus the original offset and excerpt.
- Keep evidence read-only and uncontaminated; unpack backups only in a temporary directory.
- Define correctness for instruction-file reviews: factual claims match platform observations, rules agree across files, procedures are unambiguous and executable, and cross-references resolve.
