import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SkillExposureContractTests(unittest.TestCase):
    def test_repositories_declare_positive_skill_count(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        for repo_name, repo in skills_config["repositories"].items():
            self.assertIsInstance(repo.get("skill_count"), int, f"{repo_name} must declare skill_count")
            self.assertGreater(repo["skill_count"], 0, f"{repo_name} skill_count must be positive")
            if "skill_paths" in repo:
                self.assertEqual(
                    repo["skill_count"],
                    len(repo["skill_paths"]),
                    f"{repo_name} skill_count must match skill_paths",
                )

    def test_multi_skill_repositories_require_explicit_agents(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        for repo_name, repo in skills_config["repositories"].items():
            if repo["skill_count"] > 1:
                self.assertIsNotNone(repo.get("agents"), f"{repo_name} must explicitly scope agents")

    def test_repository_agent_scopes_stay_intentional(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        expected_scopes = {
            "agent-skills": ["claude-code", "codex", "kimi-code"],
        }

        actual_scopes = {
            repo_name: repo["agents"]
            for repo_name, repo in skills_config["repositories"].items()
            if "agents" in repo
        }
        self.assertEqual(actual_scopes, expected_scopes)

    def test_repository_agent_scopes_reference_declared_targets(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        declared_targets = set(skills_config["agents"])

        for repo_name, repo in skills_config["repositories"].items():
            for agent_name in repo.get("agents", []):
                self.assertIn(agent_name, declared_targets, f"{repo_name} references unknown agent target {agent_name}")
