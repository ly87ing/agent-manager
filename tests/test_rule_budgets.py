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
        artifact = (
            ROOT
            / "agent-skill"
            / "repositories"
            / "agent-skills"
            / "artifact-hygiene"
            / "SKILL.md"
        ).read_text(encoding="utf-8")

        phrase = (
            "Do not create or widen `.gitignore`, config conventions, or repository directories just to host "
            "disposable outputs."
        )
        self.assertNotIn(phrase, core)
        self.assertIn(phrase, artifact)

    def test_agent_adapters_stay_within_budget(self):
        for target in ("codex", "claude", "antigravity"):
            content = (ROOT / "agent-rule" / "rules" / "src" / "agents" / target / "adapter.md").read_text(
                encoding="utf-8"
            )
            approx_tokens = round(len(content) / 4)
            self.assertLessEqual(approx_tokens, 220, f"{target} adapter too large: ~{approx_tokens} tokens")
