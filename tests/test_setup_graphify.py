import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "scripts" / "setup-graphify.py"
SPEC = importlib.util.spec_from_file_location("setup_graphify", MODULE_PATH)
setup_graphify = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(setup_graphify)


class SetupGraphifyTests(unittest.TestCase):
    def test_build_targets_installs_graphify_for_all_declared_agents(self):
        skill_cfg = {
            "agents": {
                "claude-code": "~/.claude/skills",
                "codex": "~/.codex/skills",
                "gemini-cli": "~/.gemini/skills",
            }
        }

        targets = setup_graphify.build_graphify_targets(skill_cfg)
        rendered = {(target.agent_id, target.kind, target.path.as_posix(), target.skill_file) for target in targets}

        self.assertIn(("claude-code", "skill", "~/.claude/skills/graphify/SKILL.md", "skill.md"), rendered)
        self.assertIn(("codex", "skill", "~/.codex/skills/graphify/SKILL.md", "skill-codex.md"), rendered)
        self.assertNotIn(("codex", "skill", "~/.agents/skills/graphify/SKILL.md", "skill-codex.md"), rendered)
        self.assertIn(("gemini-cli", "skill", "~/.gemini/skills/graphify/SKILL.md", "skill.md"), rendered)
        self.assertEqual(len(rendered), 3)

    def test_write_marker_records_installed_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_dir = Path(temp_dir) / "graphify"
            targets = [
                setup_graphify.GraphifyTarget(
                    agent_id="claude-code",
                    kind="skill",
                    path=Path("~/.claude/skills/graphify/SKILL.md"),
                    skill_file="skill.md",
                )
            ]

            setup_graphify.write_marker(marker_dir, targets, version="0.5.4")

            marker = json.loads((marker_dir / "managed-targets.json").read_text(encoding="utf-8"))
            self.assertEqual(marker["version"], "0.5.4")
            self.assertEqual(marker["targets"][0]["agent_id"], "claude-code")
            self.assertEqual(marker["targets"][0]["path"], "~/.claude/skills/graphify/SKILL.md")

    def test_build_targets_ignores_retired_agent_targets(self):
        skill_cfg = {
            "agents": {
                "cursor": "~/.cursor/rules",
                "opencode": "~/.config/opencode/skills",
                "kimi-cli": "~/.kimi/skills",
                "qodercli": "~/.qoder/skills",
            }
        }

        self.assertEqual(setup_graphify.build_graphify_targets(skill_cfg), [])

    def test_prune_stale_targets_removes_old_agent_skill_copy(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_dir = Path(temp_dir) / "graphify"
            stale_skill = Path(temp_dir) / "home" / ".agents" / "skills" / "graphify" / "SKILL.md"
            stale_skill.parent.mkdir(parents=True)
            stale_skill.write_text("---\nname: graphify\n---\n", encoding="utf-8")
            (stale_skill.parent / ".graphify_version").write_text("0.5.4", encoding="utf-8")

            setup_graphify.prune_stale_targets(marker_dir, current_targets=[], deprecated_targets=[stale_skill])

            self.assertFalse(stale_skill.parent.exists())

    def test_prune_stale_targets_preserves_unmarked_user_copy(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_dir = Path(temp_dir) / "graphify"
            user_skill = Path(temp_dir) / "home" / ".agents" / "skills" / "graphify" / "SKILL.md"
            user_skill.parent.mkdir(parents=True)
            user_skill.write_text("---\nname: graphify\n---\n", encoding="utf-8")

            setup_graphify.prune_stale_targets(marker_dir, current_targets=[], deprecated_targets=[user_skill])

            self.assertTrue(user_skill.exists())
