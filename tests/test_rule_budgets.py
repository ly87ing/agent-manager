import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class RuleBudgetTests(unittest.TestCase):
    def test_shared_core_stays_within_budget(self):
        content = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md").read_text(encoding="utf-8")
        approx_tokens = round(len(content) / 4)
        self.assertLessEqual(approx_tokens, 420, f"shared core too large: ~{approx_tokens} tokens")

    def test_shared_core_excludes_artifact_hygiene_detail(self):
        core = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md").read_text(encoding="utf-8")
        artifact = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "modules" / "artifact-hygiene.md").read_text(
            encoding="utf-8"
        )

        phrase = (
            "Use project-local ignored output directories only for workflow-consumed artifacts already used by "
            "repository workflows"
        )
        self.assertNotIn(phrase, core)
        self.assertIn(phrase, artifact)

    def test_agent_adapters_stay_within_budget(self):
        for target in ("codex", "claude", "gemini", "ide"):
            content = (ROOT / "agent-rule" / "rules" / "src" / "agents" / target / "adapter.md").read_text(
                encoding="utf-8"
            )
            approx_tokens = round(len(content) / 4)
            self.assertLessEqual(approx_tokens, 220, f"{target} adapter too large: ~{approx_tokens} tokens")
