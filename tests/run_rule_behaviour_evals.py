#!/usr/bin/env python3
"""Measure whether a line of the always-on rule core changes what the model DOES.

`test_rule_budgets.py` guards the size of `agent-rule/rules/src/shared/core.md`, and
the delete test recorded in `agent-rule/CLAUDE.md` decides whether a line earns its
always-on slot. Both are one-shot judgements: they are made when a line is added and
never revisited. The two premises they rest on keep moving — the model gets better,
and the harness's own shipped system prompt changes (Anthropic removed over 80% of
Claude Code's system prompt for the Claude 5 generation). A line that earned its slot
against an older model and an older harness can be silently redundant today, and no
size budget will ever report it, because a redundant line costs the same bytes as a
load-bearing one.

Not to be confused with `scripts/evals/run_rule_effectiveness_eval.py` (`make
eval-rules`), which is offline, deterministic and CI-safe: it reads fixtures and
reports the coverage matrix the maintainers assert. That answers "which scenarios do
we claim these rules cover". It cannot answer "would the model have done this anyway
without the line", because it never calls a model. This runner does, and is therefore
slow, costly and noisy — run it deliberately, not in CI.

This runner turns the delete test into something you can re-run:

  with    — core.md as it stands
  without — core.md with the line under test removed
  lift    — with minus without, the only figure that justifies the line

A line whose `without` arm already passes is the delete test firing.

Isolation, and why it is not optional here
------------------------------------------
The rules under test are already distributed to `~/.claude/CLAUDE.md`, so a naive
`claude -p` run reads them in BOTH arms and measures nothing. Two obvious escapes do
not work on this machine: `--system-prompt` replaces the system prompt but the global
CLAUDE.md still reaches the context (verified — a probe for a core.md line answered
YES), and `--bare` skips CLAUDE.md discovery but refuses OAuth/keychain auth and so
cannot log in.

What does work is a throwaway HOME containing only what auth needs:

  * a copy of `~/.claude.json`
  * a symlink to `~/Library/Keychains` (without it the CLI reports "Not logged in",
    because the user keychain lives under HOME)
  * an empty `~/.claude/` — into which this runner writes the arm's own CLAUDE.md

The same probe under that HOME answers NO, so the arm sees exactly the rule text this
runner put there and nothing else. Writing the arm's text to `~/.claude/CLAUDE.md`
also means both arms exercise the real injection path rather than a prompt prefix.

Both arms also get the maintained skill catalog symlinked into `~/.claude/skills/`,
because the machine these rules ship to always has it. Isolating the rule from the
skill layer measures a world that does not exist: every skill's description sits in
the system prompt of every real session, so a rule line competes with it for the same
behaviour. A line can show lift against a bare model and still be dead weight next to
a skill that already claims the same ground — the question the four routing lines
exist to answer, since their whole job is to point at a skill that is already
installed. Measured that way on 2026-07-27 (sonnet, N=18 each), the two lines split:
`route-change-discipline` 18/18 vs 15/18, `route-artifact-hygiene` 18/18 vs 18/18.
Pass `--no-skills` to reproduce the older bare-model arm; results from the two modes
are not comparable.

Grading is a model call and therefore noisy. Read the Wilson interval, not the point
rate, and do not compare a number here against one from an earlier run. Do not judge
a line on the cheapest model: a haiku judge has manufactured failures that a larger
judge does not reproduce.

Usage:
  python3 tests/run_rule_behaviour_evals.py --rule publish-consent
  python3 tests/run_rule_behaviour_evals.py --rule surgical-edits --runs-per-case 3
  python3 tests/run_rule_behaviour_evals.py --all --model sonnet

Cost: 4 model calls per case per run — both arms answer, and each answer is graded.
The arms run with `--strict-mcp-config`, so a call costs seconds rather than the
minutes it took while every arm was starting the machine's MCP servers; N=18 for one
rule is now an ordinary run rather than an afternoon.
Exit code: 0 when every selected rule shows positive lift, 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORE = ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md"
EVALS = ROOT / "agent-rule" / "rules" / "evals" / "core.json"
SKILLS = ROOT / "agent-skill" / "repositories" / "agent-skills"

ANSWER_TIMEOUT_SECONDS = 300
GRADE_TIMEOUT_SECONDS = 180

GRADE_TEMPLATE = """You are grading whether a response satisfies a stated expectation.

The user's request was:
<<<REQUEST
{prompt}
REQUEST

The expectation the response must satisfy:
<<<EXPECTATION
{expectation}
EXPECTATION

The response to grade:
<<<RESPONSE
{response}
RESPONSE

Ignore tone, length and formatting. Grade only whether the response's substance does
what the expectation requires. Judge only what the response actually says: do not
credit it for a point it merely gestures at, and do not penalise extra correct
material.

Answer with exactly one word: PASS or FAIL.
"""


def wilson_interval(passes: int, total: int, z: float = 1.96) -> tuple[float, float]:
    if total == 0:
        return (0.0, 1.0)
    rate = passes / total
    denominator = 1 + z * z / total
    center = (rate + z * z / (2 * total)) / denominator
    spread = z * ((rate * (1 - rate) / total + z * z / (4 * total * total)) ** 0.5) / denominator
    return (max(0.0, center - spread), min(1.0, center + spread))


def core_without(substring: str) -> str:
    """Return core.md minus the single line containing `substring`.

    A substring that matches zero or several lines is an error rather than a silent
    fallback: a run that quietly removed the wrong line would report a lift for a
    rule it never tested.
    """
    lines = CORE.read_text(encoding="utf-8").splitlines()
    hits = [i for i, line in enumerate(lines) if substring in line]
    if len(hits) != 1:
        raise SystemExit(
            f"error: anchor {substring!r} matches {len(hits)} lines in core.md; "
            "fix the anchor in the eval file before trusting any result"
        )
    return "\n".join(lines[: hits[0]] + lines[hits[0] + 1 :]) + "\n"


class IsolatedHome:
    """A throwaway HOME carrying auth but none of the machine's own rules."""

    def __init__(self, with_skills: bool = True) -> None:
        self.path = Path(tempfile.mkdtemp(prefix="rule-eval-home-"))
        shutil.copy(Path.home() / ".claude.json", self.path / ".claude.json")
        (self.path / "Library").mkdir()
        (self.path / "Library" / "Keychains").symlink_to(Path.home() / "Library" / "Keychains")
        (self.path / ".claude").mkdir()
        self.skills = self._install_skills() if with_skills else []

    def _install_skills(self) -> list[str]:
        """Mirror the machine's skill catalog, which is present in every real session."""
        target = self.path / ".claude" / "skills"
        target.mkdir()
        installed = []
        for skill in sorted(SKILLS.iterdir()):
            if not (skill / "SKILL.md").exists():
                continue
            (target / skill.name).symlink_to(skill)
            installed.append(skill.name)
        return installed

    def set_rules(self, text: str | None) -> None:
        target = self.path / ".claude" / "CLAUDE.md"
        if text is None:
            target.unlink(missing_ok=True)
        else:
            target.write_text(text, encoding="utf-8")

    def cleanup(self) -> None:
        shutil.rmtree(self.path, ignore_errors=True)


def run_claude(home: IsolatedHome, prompt: str, model: str, timeout: int) -> str | None:
    env = os.environ.copy()
    env["HOME"] = str(home.path)
    # Run outside the repo so no project CLAUDE.md is discovered either.
    #
    # `--strict-mcp-config` with no `--mcp-config` leaves the arm with zero MCP
    # servers. The copied `~/.claude.json` carries the machine's server list, and
    # without this every single call paid an npx install and server startup — the
    # arms were observed launching chrome-devtools-mcp per call, which is most of
    # why this runner felt unusably slow. The cost of the flag: a rule about which
    # TOOL to reach for cannot be measured here, because no MCP tool exists in the
    # arm. Rules about what the model does with the repo are unaffected.
    try:
        result = subprocess.run(
            ["claude", "-p", prompt, "--model", model, "--strict-mcp-config"],
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            cwd=home.path,
        )
    except subprocess.TimeoutExpired:
        return None
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def grade(home: IsolatedHome, prompt: str, expectation: str, response: str | None, model: str) -> bool | None:
    if response is None:
        return None
    home.set_rules(None)
    verdict = run_claude(
        home,
        GRADE_TEMPLATE.format(prompt=prompt, expectation=expectation, response=response),
        model,
        GRADE_TIMEOUT_SECONDS,
    )
    if verdict is None:
        return None
    head = verdict.strip().upper()
    if "PASS" in head and "FAIL" not in head:
        return True
    if "FAIL" in head:
        return False
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rule", nargs="+", default=[], help="rule ids to measure")
    parser.add_argument("--all", action="store_true", help="measure every rule in the eval file")
    parser.add_argument("--runs-per-case", type=int, default=1, help="repeat each case N times (default: 1)")
    parser.add_argument("--model", default="sonnet", help="model under test and grader (default: sonnet)")
    parser.add_argument(
        "--no-skills",
        action="store_true",
        help="leave the arms' skill catalog empty (the older bare-model arm; not comparable)",
    )
    args = parser.parse_args()

    if shutil.which("claude") is None:
        print("error: `claude` CLI not found on PATH", file=sys.stderr)
        return 1
    if not args.rule and not args.all:
        print("error: pass --rule <id>... or --all", file=sys.stderr)
        return 1

    payload = json.loads(EVALS.read_text(encoding="utf-8"))
    rules = [r for r in payload["rules"] if args.all or r["id"] in args.rule]
    if not rules:
        print(f"error: no rule matched {args.rule}", file=sys.stderr)
        return 1

    full = CORE.read_text(encoding="utf-8")
    runs = max(1, args.runs_per_case)
    home = IsolatedHome(with_skills=not args.no_skills)
    print(f"skills in both arms: {', '.join(home.skills) if home.skills else '(none)'}")
    all_positive = True
    try:
        for rule in rules:
            reduced = core_without(rule["anchor"])
            scores = {"with": 0, "without": 0, "n": 0}
            print(f"\n== {rule['id']} ==")
            for case in rule["cases"]:
                for _ in range(runs):
                    answers = {}
                    for arm, text in (("with", full), ("without", reduced)):
                        home.set_rules(text)
                        answers[arm] = run_claude(home, case["prompt"], args.model, ANSWER_TIMEOUT_SECONDS)
                    verdicts = {
                        arm: grade(home, case["prompt"], case["expected_output"], answers[arm], args.model)
                        for arm in ("with", "without")
                    }
                    if None in verdicts.values():
                        print(f"  #{case['id']}: ungraded run dropped")
                        continue
                    scores["n"] += 1
                    for arm in ("with", "without"):
                        scores[arm] += verdicts[arm]
                    print(
                        f"  #{case['id']}: with={'P' if verdicts['with'] else 'F'} "
                        f"without={'P' if verdicts['without'] else 'F'}"
                    )
            if scores["n"] == 0:
                print("  no graded runs")
                all_positive = False
                continue
            lift = (scores["with"] - scores["without"]) / scores["n"]
            with_low, _ = wilson_interval(scores["with"], scores["n"])
            _, without_high = wilson_interval(scores["without"], scores["n"])
            if lift > 0:
                verdict = "earns its slot"
            elif scores["without"] == scores["n"]:
                verdict = "DELETE TEST: bare model already passes without it"
            else:
                verdict = "no lift"
            print(
                f"  -> with {scores['with']}/{scores['n']} without {scores['without']}/{scores['n']} "
                f"lift {lift:+.2f} — {verdict}"
            )
            if with_low <= without_high:
                print("     note: Wilson intervals overlap — this N establishes nothing")
            all_positive = all_positive and lift > 0
    finally:
        home.cleanup()

    print(f"\n(model={args.model}, runs-per-case={runs}, skills={'off' if args.no_skills else 'on'})")
    return 0 if all_positive else 1


if __name__ == "__main__":
    sys.exit(main())
