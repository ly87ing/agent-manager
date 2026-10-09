# Skill Authoring Standards

Applies to all managed skills in `agent-skills` (ly87ing/agent-skills). That repository's `tests/test_skill_contracts.py` encodes the structural contract: naming, frontmatter, English body, a single `agents/openai.yaml`, ≤500 lines and ≤16000 characters, and evals. This document covers authoring judgments that tests cannot encode.

Sources: Anthropic's official Agent Skills best practices (docs.claude.com → Agent Skills best practices) and OpenAI's official Codex Skills documentation (developers.openai.com/codex/skills). Both emphasize: descriptions determine triggering and must state capabilities, when to use, and when not to use; SKILL.md is an index, with details in `references/` and deterministic logic in `scripts/` (execute rather than load them; only outputs enter context); instructions are self-contained with verification and positive/negative examples; time-sensitive facts belong in references with verification dates, not core files.

## Two additional governance standards

1. **Supplement rather than replace.** A domain skill consisting only of local knowledge can displace general engineering judgment. This was a reproducible evaluation failure: the first security-shift-left skill used only company cases and performed worse than the no-skill control. Combine general judgment with local evidence. SKILL.md must explicitly state what it supplements and what it does not replace.
2. **Local evidence takes precedence over generic templates.** Calibrate checklists, weights, and attention to actual historical distributions rather than copying generic industry proportions such as OWASP. Give a traceable original issue number for every piece of evidence; adversarial review targets disagreement between summaries and the evidence base. When incident-derived local knowledge (TR) conflicts with industry defaults, local knowledge wins: defaults are uninformed priors; TR is a posterior earned through real incidents.

## Leading cause of missed triggers: descriptions omit existing capabilities (observed 2026-07-19)

The first full scan tested 110 cases against all 16 installed skills: 97/110 passed. Of 13 failures, 7 selected `none`, rather than a wrong skill. Reading the bodies showed that capabilities were present but **descriptions did not advertise covered scenarios**. Four examples shared this cause:

| Case | Already covered in the body | Absent from the description |
| --- | --- | --- |
| artifact-hygiene #12 | Do not create .bak/timestamp copies of version-controlled files | Backup copies missing from artifact types |
| safe-merge-review #9 | Parallel releases make is-ancestor/commit-id/issue checks falsely report missing; compare content | Trigger list covered only merge mechanics |
| safe-merge-review #14 | Simulate re-pin/bump/apply for a tracked ref | No go/no-go for unpushed cross-repository dependencies |
| reader-facing-writing #8 | Give an outline or options before a large subjective rewrite with no target | Scenario unmentioned |

**Whenever a body clause is added, check that the description advertises it.** This can be judged deterministically by reading body and description, without noisy measurement or waiting for trigger rates to fall. The reverse matters too: claiming unsupported capabilities attracts the wrong tasks. safe-merge-review once mentioned "a merge request link" with no MR guidance in body or references. ones-manhour-fill once said "safe add-only writes," implying no deletion or modification and routing deletion requests around the guard requiring read-back and UTC+8 date verification before deletion.

**A description change is not local: all skills in the catalog compete for the same selection.** On 2026-07-19, adding projected full-screen rendering to frontend-verification caused reader-facing-writing #19 (a dark welcome screen for a venue) to fail and select fv, despite no case change. The fix was ownership by **verb**: design intent belongs to reader-facing-writing; checking an existing page's rendering in venue full-screen conditions belongs to frontend-verification.

Retesting after a description change must include **neighboring skills' cases**, not just its own. Otherwise a local fix can silently break adjacent selection.

**Check two hard budgets before expanding a description:**

1. Each description ≤1024 characters, enforced by `tests/test_skill_contracts.py`. On 2026-07-30, three of this repository's five descriptions were near the limit (1008/1014/1006); make room before adding.
2. Total name+description length for all installed skills. Community observations indicate Claude Code truncates by a character budget: **excess skills are silently dropped with no warning**, while the system prompt prohibits using unlisted skills. On 2026-07-30, 11 installed skills totaled 8003 characters, including 4816 from this repository's five descriptions. The reported default budget of about 15000 left roughly 7000. Measure totals before changes to avoid excluding other skills.

Filling description gaps still requires retesting; trigger success is not 100%. The four 2026-07-19 fixes were retested serially with the full catalog (N=5): artifact-hygiene #12 and safe-merge-review #9 rose from 0.00 to 1.00; reader-facing-writing #8 rose from 0.00 to 0.80 but its interval crossed the threshold, so the result was inconclusive. safe-merge-review #14 still lost all 5/5 to change-discipline.

**If a case consistently selects another skill after filling the description gap, question ownership before continuing to tune wording.** #14 asked whether a branch could merge, but its blocker was a cross-repository contract; both skills were defensible. Record this as a known disagreement. Do not change expectations merely to pass tests. This differs from a malformed case, such as change-discipline #5 using a graded expectation in a binary harness: that is a structural defect, while the former is a judgment disagreement.

## Proving that a clause should exist (established 2026-07-19)

Start with the deletion test, then triggering. Official guidance first asks whether the model would err without the instruction. `tests/run_behaviour_evals.py` answers this by running the same prompt with and without the skill body, independently scoring against expected_output, and comparing lift. **A passing without-skill run activates the deletion test; it is not success.** An ineffective clause costs context on every trigger. For example, the bare model already kept credentials out of command output in artifact-hygiene's case, so trigger tuning offered no benefit.

`tests/run_trigger_evals.py` answers only whether the skill loads, not the behavior question. When using it:

1. **Include neighbors in the catalog.** A catalog containing only this repository's skills is systematically optimistic; the real agent chooses among all installed skills. Use `--catalog-dir ~/.claude/skills`. Losing to a skill outside this repository is a real operating condition.
2. **Compare intervals, not point estimates.** A single verdict says little. Use `--runs-per-query` with Wilson intervals; `?` means the interval crosses the threshold and this N establishes nothing, not a pass. One configuration produced 0.17/0.58/0.40 across three runs; a homogeneity test gave p≈0.11, consistent with binomial noise at N≈10 rather than instrument drift.
3. **Pair and interleave serially.** Compare descriptions within one run, alternating candidates per case and rotating order. Running all A then all C confounds time with the candidate. Observed `--jobs` concurrency shifted judgments: a worse candidate scored 1.00 concurrently but only 0.10 in interleaved serial runs, nearly leading to deployment on false evidence.
4. **If the effect is smaller than dispersion, report it as unmeasurable and leave it unchanged.** Three candidates had heavily overlapping intervals and would need 150+ samples per arm to distinguish them; that cost was disproportionate.

Two counterintuitive observations:

- **Negative clauses can backfire.** "Not for X" introduces X's keywords and can attract selection. Define boundaries with positive criteria for ownership instead.
- **Do not tune the scorer to agree with you.** An apparently unfair first behavior FAIL prompted a looser scoring draft; rereading the expectation showed the scorer was right: the answer diagnosed but did not remediate. Allow wording differences or equivalent commands, never missing steps.

## Change workflow

Edit the development repository (`~/Documents/github/agent-skills`) → add eval cases → run repository contract tests → commit in repository style without AI attribution → push with user consent → update the source on Agent Manager Skills and confirm enablement per assistant → spot-check what each agent actually loads.
