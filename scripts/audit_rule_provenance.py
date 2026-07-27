#!/usr/bin/env python3
"""Report how old each always-on rule is and whether anything still checks it.

`tests/test_rule_budgets.py` caps the size of shared/core.md and the delete test in
`agent-rule/CLAUDE.md` decides whether a line earns its always-on slot. Both fire when
a line is ADDED. Neither can fire when a line quietly stops being needed — a line the
harness has since absorbed costs exactly as many bytes as a load-bearing one, so no
size budget will ever flag it. That is the gap this report fills, and it is a report
rather than an assertion on purpose: staleness is a function of the calendar, and a
unittest that turns red as time passes is a broken test, not a working guard.

For each line of core.md it prints:

  introduced  first commit whose diff added this text (a rewrite shows as its own date)
  touched     most recent commit that changed it
  eval        whether agent-rule/rules/evals/core.json has a behaviour case anchored
              to this line — i.e. whether `tests/run_rule_behaviour_evals.py` can
              currently re-run the delete test for it, and re-derive its lift

A line that is old and has no eval is not a defect; it is an unexamined claim. Write a
case for it, measure the lift, and let the number decide whether it keeps its slot.

Usage: python3 scripts/audit_rule_provenance.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORE = ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md"
EVALS = ROOT / "agent-rule" / "rules" / "evals" / "core.json"
CORE_REL = CORE.relative_to(ROOT).as_posix()


def git_dates(needle: str) -> tuple[str, str]:
    """Return (introduced, last touched) short dates for commits changing `needle`."""
    result = subprocess.run(
        ["git", "log", "--format=%ad", "--date=short", "-S", needle, "--", CORE_REL],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    dates = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if not dates:
        return ("unknown", "unknown")
    return (dates[-1], dates[0])


def anchored_rules() -> dict[str, str]:
    """Map each eval anchor to its rule id."""
    if not EVALS.exists():
        return {}
    payload = json.loads(EVALS.read_text(encoding="utf-8"))
    return {rule["anchor"]: rule["id"] for rule in payload.get("rules", [])}


def main() -> int:
    lines = [line for line in CORE.read_text(encoding="utf-8").splitlines() if line.strip()]
    anchors = anchored_rules()

    uncovered = 0
    print(f"{'#':>2}  {'introduced':<11}{'touched':<11}{'eval':<28}rule")
    print("-" * 100)
    for index, line in enumerate(lines, start=1):
        text = line.lstrip("- ").strip()
        # A 48-char slice is long enough to be unique in this file and short enough to
        # survive the tail of a line being reworded.
        introduced, touched = git_dates(text[:48])
        covered = next((rule_id for anchor, rule_id in anchors.items() if anchor in line), None)
        if covered is None:
            uncovered += 1
        summary = text if len(text) <= 58 else text[:55] + "..."
        print(f"{index:>2}  {introduced:<11}{touched:<11}{(covered or '—'):<28}{summary}")

    print("-" * 100)
    print(f"{len(lines)} always-on lines, {len(lines) - uncovered} with a behaviour eval, {uncovered} unexamined")
    if uncovered:
        print("\nUnexamined lines carry no evidence that the current model still needs them.")
        print("Add a case to agent-rule/rules/evals/core.json, then:")
        print("  python3 tests/run_rule_behaviour_evals.py --rule <id> --runs-per-case 3")
    return 0


if __name__ == "__main__":
    sys.exit(main())
