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
                "cursor": "~/.cursor/rules",
                "opencode": "~/.config/opencode/skills",
                "kimi-cli": "~/.kimi/skills",
            }
        }

        targets = setup_graphify.build_graphify_targets(skill_cfg)
        rendered = {(target.agent_id, target.kind, target.path.as_posix(), target.skill_file) for target in targets}

        self.assertIn(("claude-code", "skill", "~/.claude/skills/graphify/SKILL.md", "skill.md"), rendered)
        self.assertIn(("codex", "skill", "~/.codex/skills/graphify/SKILL.md", "skill-codex.md"), rendered)
        self.assertIn(("codex", "skill", "~/.agents/skills/graphify/SKILL.md", "skill-codex.md"), rendered)
        self.assertIn(("gemini-cli", "skill", "~/.gemini/skills/graphify/SKILL.md", "skill.md"), rendered)
        self.assertIn(("opencode", "skill", "~/.config/opencode/skills/graphify/SKILL.md", "skill-opencode.md"), rendered)
        self.assertIn(("kimi-cli", "skill", "~/.kimi/skills/graphify/SKILL.md", "skill.md"), rendered)
        self.assertIn(("cursor", "cursor-rule", "~/.cursor/rules/graphify.mdc", ""), rendered)

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

    def test_cursor_rule_is_conditional_on_existing_graph(self):
        self.assertIn("When the current project has a graphify knowledge graph", setup_graphify.CURSOR_RULE)
        self.assertIn("graphify-out/GRAPH_REPORT.md", setup_graphify.CURSOR_RULE)
