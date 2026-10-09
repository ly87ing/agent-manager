# Skill Prompt Audit (2026-08-04)

**Conclusion:** the 5 agent-manager skills and 8 xiaoyu-dev skills are generally sound: prohibitions explain why, safety gates are well specified, and pressure language is rare. They do not need wholesale rewriting. The main issues are **full repetition of rules** and **frozen incident narratives**. An operational issue was found and fixed: the installed repository at ~/.xiaoyu-dev lagged its source by 3 commits and lacked the Behavior/Test Focus gate. Three high-priority changes were completed in this round; others were initially queued below.

Method: two read-only subagents read every SKILL.md and reference in both repositories, using Anthropic's prompt-audit distinction between business contracts/safety gates to retain and scaffolding for older models to remove or revise. Target models were Claude Opus 5 / Fable 5. Trigger frequency came from all local Claude Code sessions from 2026-07-27 through 08-04.

## Trigger frequency and optimization priority

Over 9 days: reader-facing-writing **44**, change-discipline 10, xiaoyu-code-submit 10, ones 6, artifact-hygiene 5, gitlab 4, ones-manhour-fill 4, frontend-verification 3, remote-ops 3, safe-merge-review 1, bug-handler 1, **xiaoyu-bug-loop 0**.

- Frequency × body size estimates payoff: reader-facing-writing (44 × 16KB) ranks first.
- All frontmatter descriptions remain in every request regardless of triggering: 12 custom skills total about 4.4k tokens.

## Calibration: do not compress every description indiscriminately

agent-manager has a trigger-evaluation system. The agent-skills README records the host's 1024-character truncation contract, front-loading rule, and empirically rejected compression candidates. **Its five long descriptions (816–1014 characters) are case-tuned local optima and must not be compressed blindly.** The exception is an evaluated safe-merge-review candidate saving about 100 characters, which still requires passing evaluation before adoption. xiaoyu-dev has no such evaluation system; its trigger-phrase lists can be rewritten safely. Drafts for bug-loop and code-submit appear below.

## agent-manager / agent-skills findings

| Skill | Assessment | Findings and changes |
| --- | --- | --- |
| artifact-hygiene (9.1KB) | Sound; minor edits | L17 states one principle three times: reduce to one sentence plus reason. L21 and L49 overlap reuse tests with Hard Boundaries: use one source. Retain secrets-out-of-logs, privilege restoration, and backup-before-filter-repo |
| change-discipline (9.3KB + 2 refs) | Sound; minor edits | L25–27 has three escalation rules: merge 26/27. Six "A real case" narratives: one per rule. Description is an evaluated local optimum; leave it unchanged |
| frontend-verification (7.7KB + 2 refs) | Sound | SKILL.md:34 and visuals-and-decks:42 almost duplicate OS-level full-screen/glare guidance: keep thresholds in this skill. Cut roughly 50% of generic playwright-essentials knowledge, retaining storageState secrecy, mock boundaries, and flaky classification. Keep empirical toolchain failures: INSTALLATION_COMPLETE, 30s silent timeout, exit 137 |
| reader-facing-writing (16KB, near budget) | Moderate edits | Repeat "demote, do not delete" in 4+ places: one source. Halve three reference-index paraphrases, retaining triggers and keywords. Split long L40/44/45 bullets. **Completed below** |
| safe-merge-review (11.5KB + 2 refs) | Moderate edits; most duplication | Field table appears in SKILL.md:125–141, checklist §7, and build_merge_evidence.py: make script authoritative. Checklist §6 repeats workflow §7 completeness: replace with a link. AI-provenance prohibition appears nearly verbatim three times: retain one in the skill and the always-on core.md invariant. Cut about 40% of known merge-base/left-right explanations in workflow §1–3.3, retaining commands. Preserve content-based parallel-baseline judgments in §3.4 and §7.5's three squash methods and cherry-pick-noop protocol |
| agent-rule/ | Sound; leave unchanged | Every core.md line has budget gates and deletion-test support; hooks inject only 1–2 sentences. Halve process narratives such as MCP latency in CLAUDE.md (9.6KB), saving about 1.5KB; preserve commit IDs and dates |

Estimated reduction: about 18–19KB (~25%) of on-demand bodies; no reduction to resident descriptions.

## xiaoyu-dev / skills findings

| Skill | Assessment | Findings and changes |
| --- | --- | --- |
| xiaoyu-bug-loop (SKILL 43KB + refs ~131KB) | Strong structure; heavy repetition | Relative-sprint parsing repeats in G10/Phase 0/D11 (~6KB): retain only D11. Empty-scope gate appears 4 times: one source. Move ~3.5KB Loop controls scheduling/model-selection guidance to README. Remove one-off "nine months apart" measurement examples. mode-fix constraints 7/9 contain full case reviews, 11 duplicates E2, and Experience distilled is over-scaffolded: remove or reduce to 2 lines. mode-verify §1/§3 paraphrases G3/G5: use references. known-pitfalls is a **live contract**, not a memorial: retain ~80-word attachment limits, workflow-dependent transition uuid, unreliable cross-team name_match, and commits lost by --since/--until. Merge admitted Z2/Z7 duplication; reduce D11's 111-sprint table to a principle plus one example. Estimated reductions: SKILL −35%, mode-fix −30%, known-pitfalls −25%, mode-verify −20% |
| xiaoyu-code-submit (15.5KB + refs) | Best adapted to newer models | Replace pressured "Mandatory…no exception" description opening with plain wording. Remove §9 Final self-check's 13 items: nearly 1:1 with Stop Conditions and induces excessive verification in Fable 5; each has an authoritative source elsewhere. Retain secret scanning, AI-provenance removal, server-side `[#id]` + 40-character hook, and code-point/UTF-16 differences. **Completed below** |
| xiaoyu-security-review (20KB + refs ~105KB) | One large concentration | references/ones-defect-evidence.md is **78KB of historical evidence accumulation**, including 833 one-line bug titles. Live value is the distribution/trend tables, section introductions, and lessons in §②. Move evidence to the knowledge base and retain an ~8KB summary. Consolidate three repetitions of reporting only newly introduced defects. Keep confidence ≥8, 14 hard exclusions, TR-over-exclusion arbitration, and remote-data gates |
| xiaoyu-monitoring-gen (23KB) | Heavy duplication | 17 hard rules repeat in phases, including selector three-way splits 2–3 times. Replace Phase 4's handwritten four-step verification loop with validate.py reporting no FAIL. Potential reduction 35–45% |
| xiaoyu-generate-api-detail (23KB) | Most duplication | Step-5 confirmation gate repeats four times, including red-circle emphasis. v1/v2 migration history and dated example branches age quickly. Potential reduction 40–50% |
| xiaoyu-compatibility-review (13KB) | Best of these three | Commands generally explain why; phase five is a real method. Move only phase-one command detail into references. Can serve as a rewrite model. Potential reduction 10–15% |

### Description drafts (pending evaluation / adopted)

bug-loop (~890→~520 characters; not applied at the first audit, with trigger verification recommended):

> Batch-process one developer's ONES bugs. Full mode analyzes New/In Progress bugs and comments + transits them; code changes, commits and Resolved happen only on explicit automatic-fix requests. Verify mode proves deployed Resolved fixes with UI evidence matching each bug's exact scenario, then routes passing bugs to their filing QA — no matching evidence, no routing. Scope defaults to all the current user's actionable bugs across sprints; another named person is read-only unless writes are authorized. Single-bug root cause → bug-handler; ONES-only ops → ones.

code-submit: applied below, retaining the trigger phrase "including small 'just commit this' changes."

## Completed (2026-08-04)

1. **Synchronize installed repository:** ~/.xiaoyu-dev `git pull --ff-only` reached origin (3936f25), aligned 4 drifted files, and `diff -rq` reported no differences. New source changes from this round still require commit+push and another pull to reach the installed repository.
2. **Deduplicate reader-facing-writing** in agent-skills: 9 edits, 16060→15150 bytes. Demotion guidance moved to one source at L37; prose-style/revision-pass/visuals pointer paragraphs were halved. Triggers remain in SKILL.md, respecting the README's 2026-07-29 observation that reference-only rules may not take effect. Three long bullets about document sets, named entities, and organizational logic lost internal repetition. All 23 offline contract tests passed. Behavior evals were not run: descriptions and the rules themselves were unchanged, so trigger evals were unaffected.
3. **xiaoyu-code-submit** in xiaoyu-dev: description shortened from 843 to about 600 characters and pressure language removed; §9 Final self-check deleted because every item already exists in SKILL.md/other sections. Updated its contents line and SKILL.md:69 reference. All repository check targets passed.

## Second batch (2026-08-04; original items 1–5 complete)

Three parallel editing agents implemented changes; combined-state verification preceded per-skill commits and pushes. agent-skills e7db98e..46b9d71 contains four commits, xiaoyu-dev 16dd2ba..39b49e1 four; ~/.xiaoyu-dev was synchronized.

- **safe-merge-review:** one source for fields (script for structure, SKILL.md for semantics); checklist §6/§7 duplication replaced with pointers; provenance guidance consolidated; conceptual workflow explanations removed. Section bytes fell 20%, below the literal 40% target because the rest was retained checklist content.
- **bug-loop:** description draft applied; SKILL.md 43.1→36.1KB (−16%). Relative-sprint parsing centralized at D11, empty-scope gate at G10, Loop controls moved to README. mode-fix −2.2KB by shortening case reviews and Experience distilled to 2 lines. known-pitfalls Z2/Z7 merged and D11 measurement table reduced to a principle. mode-verify paraphrases became references.
- **security-review:** 78KB evidence moved with git mv into docs/ to preserve history; package retains a 13.6KB summary of distributions/trends, representative cases, and lessons. Description 340→165 characters; newly-introduced-only guidance consolidated.
- **generate-api-detail:** confirmation gate centralized in step 5; red-circle emphasis removed, migration history reduced to one sentence, dated examples replaced with placeholders (−0.9KB).
- **monitoring-gen:** hard-rule/phase duplication consolidated; Phase 4 uses validate.py with zero FAIL as its criterion, retaining the prohibition against removing checks to pass (−1.0KB).
- **playwright-essentials:** −33%, short of 50% because three empirical blocks were retained. **change-discipline:** L26/27 merged and one case per rule. **artifact-hygiene:** three repeated statements condensed. **visuals-and-decks:** projection guidance consolidated with frontend-verification.

Verification: all 23 agent-skills contract tests and xiaoyu-dev checks passed in the combined final state. Safety-gate/hard-constraint spot checks passed: five stop markers, G10, no residual Z7, and the monitoring safety gate.

## Remaining optional work

- Halve process narratives in agent-rule/CLAUDE.md (−1.5KB).
- If bug-loop remains unused long-term, add `.default-off` to remove its resident description; frequency above was 0 over 9 days.
- Evaluate the safe-merge-review description candidate before applying it.
- Rerun behavior evals (`run_behaviour_evals.py` and xiaoyu output-quality evals, requiring model calls) to confirm no regression from compression.

Execution convention: agent-skills changes must pass its tests/ and run_trigger_evals; change one xiaoyu-dev skill at a time, retaining old wording in Git for rollback. Do not remove: the ONES state machine and stop gates, G1–G10, secret scanning/provenance removal, known-pitfalls environment contracts, all agent-rule/core.md, or frontend-verification's empirical failure modes.
