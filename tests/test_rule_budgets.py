import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class RuleBudgetTests(unittest.TestCase):
    def test_shared_core_stays_within_budget(self):
        # 预算 420 -> 440：新增「未经用户明确同意不得发布产物到远端」这条 always-on 协作边界。
        # 它过得了 delete-test：harness 的 Artifact 工具默认「主动发布自己的产物没问题」，删掉此行
        # 该默认立即回归、事故复发，故配占常驻槽位；旧上限只剩 ~10 tokens，容不下任何新边界。
        # 预算 440 -> 545（2026-07-19 用户拍板，三家全量 memory+会话提炼晋升）：新增三条 always-on 边界，
        # 各自过 delete-test——①commit/MR 不带 AI 署名（harness 默认追加 Co-Authored-By，删行即回归）；
        # ②承重结论核一手（4 个项目有凭二手摘要/"已完成"叙述翻车实录，非改码场景无其他承载）；
        # ③打包/导出/发送同属交付动作 + 里程碑后主动提议 push（51 次手动 push 摩擦，并入既有 publish 行）。
        # 这是预期行为变更导致的上限调整，不是为了让断言变绿而放宽。
        # 任一行若下线，此上限必须一并对称回退（见 agent-rule/CLAUDE.md 的 delete-test 护栏）。
        content = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md").read_text(encoding="utf-8")
        approx_tokens = round(len(content) / 4)
        self.assertLessEqual(approx_tokens, 545, f"shared core too large: ~{approx_tokens} tokens")

    def test_shared_core_excludes_artifact_hygiene_detail(self):
        core = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md").read_text(encoding="utf-8")
        phrase = (
            "Do not create or widen `.gitignore`, config conventions, or repository directories just to host "
            "disposable outputs."
        )

        # The independently versioned skill repository owns the detailed clause;
        # this repository owns keeping it out of the always-on core while retaining
        # the route that loads it on demand.
        self.assertNotIn(phrase, core)
        self.assertIn("`artifact-hygiene` skill", core)

    def test_agent_adapters_stay_within_budget(self):
        for target in ("codex", "claude", "antigravity"):
            content = (ROOT / "agent-rule" / "rules" / "src" / "agents" / target / "adapter.md").read_text(
                encoding="utf-8"
            )
            approx_tokens = round(len(content) / 4)
            self.assertLessEqual(approx_tokens, 220, f"{target} adapter too large: ~{approx_tokens} tokens")

    def test_antigravity_adapter_keeps_its_verified_reversed_defaults(self):
        # An adapter may only carry a rule that reverses THAT agent's own default;
        # anything true of every agent belongs in shared core. claude/codex stay
        # empty because their built-in prompts already cover this ground (3a5f110).
        #
        # Provenance, re-established 2026-07-19: these two lines were written for
        # Gemini CLI against gemini-cli#7432 and #24725, then carried to Antigravity
        # by a 100%-similarity rename (bc9604f) that preserved the text while
        # silently voiding its evidence. Re-verified against Antigravity itself:
        # plan-first is NOT its default (opt-in `--mode=plan`; the docs list
        # "request an implementation plan" as a user practice), and this machine
        # sets toolPermission/artifactReviewPolicy to always-proceed, disabling the
        # request-review gates that would otherwise make both lines redundant.
        # Local Antigravity session records show `git add . && git commit && git push`
        # run as one command with no approval recorded. Both lines therefore stay.
        #
        # If the gates are ever restored to Antigravity's shipped defaults, re-run
        # the delete test on both lines rather than keeping them out of habit.
        #
        # The claude/codex emptiness assertion is a dated claim, not an invariant.
        # "Their built-in prompts already cover this ground" was measured against the
        # prompts those runtimes shipped in 2026-04; Anthropic has since removed over
        # 80% of Claude Code's system prompt for the Claude 5 generation, so the
        # covered surface has SHRUNK. The direction of a future correction is
        # therefore more likely to be "claude adapter needs a line back" than "core
        # can absorb one more". Do not read a green run here as evidence that the
        # coverage claim was re-checked — nothing in this file re-checks it. Measure
        # it with tests/run_rule_behaviour_evals.py against the runtime in question,
        # and record the date here when you do.
        adapter = (ROOT / "agent-rule" / "rules" / "src" / "agents" / "antigravity" / "adapter.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("wait for confirmation before modifying files", adapter)
        self.assertIn("never run `git add .`", adapter)
        for empty_target in ("claude", "codex"):
            other = (ROOT / "agent-rule" / "rules" / "src" / "agents" / empty_target / "adapter.md").read_text(
                encoding="utf-8"
            )
            self.assertEqual(other.strip(), "", f"{empty_target} adapter must stay empty unless a reversed default is documented")
