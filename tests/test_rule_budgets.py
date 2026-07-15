import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class RuleBudgetTests(unittest.TestCase):
    def test_shared_core_stays_within_budget(self):
        # 预算 420 -> 440：新增「未经用户明确同意不得上传产物到远端」这条 always-on 协作边界。
        # 它属于 core 该承载的层级（触发条件/协作边界/最小行为约束），且旧上限只剩 ~10 tokens 余量，
        # 容不下任何新边界。这是预期行为变更导致的上限调整，不是为了让断言变绿而放宽。
        content = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md").read_text(encoding="utf-8")
        approx_tokens = round(len(content) / 4)
        self.assertLessEqual(approx_tokens, 440, f"shared core too large: ~{approx_tokens} tokens")

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
