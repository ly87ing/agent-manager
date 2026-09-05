import json
import unittest
from pathlib import Path

from src.agent_ids import canonicalize_agent_id as _canonicalize_agent_id


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

    def test_new_skill_default_values_are_valid_when_present(self):
        # v2（提案 §7.1）：new_skill_default 只接受 "disabled" / "enabled"，缺省 disabled
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        manager_config = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        sources = [("skills.json", repo) for repo in skills_config["repositories"].values()]
        sources += [("manager.json", policy) for policy in manager_config.get("skills", {}).values()]
        for source_name, config in sources:
            if "new_skill_default" in config:
                self.assertIn(
                    config["new_skill_default"],
                    ("disabled", "enabled"),
                    f"{source_name} new_skill_default must be disabled|enabled",
                )

    def test_policy_skill_matrix_shape_is_valid_when_present(self):
        # v2 启用矩阵形状契约：skills.<repo>.skills.<name>.agents = null | {agent: bool}
        manager_config = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        declared_targets = set(skills_config["agents"])

        for source in (manager_config.get("skills", {}),):
            for repo_name, policy in source.items():
                matrix = policy.get("skills")
                if matrix is None:
                    continue
                self.assertIsInstance(matrix, dict, f"{repo_name} skills matrix must be a mapping")
                for skill_name, entry in matrix.items():
                    self.assertIsInstance(skill_name, str, f"{repo_name} matrix keys must be skill names")
                    self.assertIsInstance(entry, dict, f"{repo_name}.{skill_name} entry must be an object")
                    self.assertLessEqual(set(entry), {"agents"}, f"{repo_name}.{skill_name} has unknown keys")
                    agents = entry.get("agents")
                    if agents is None:
                        continue
                    self.assertIsInstance(agents, dict, f"{repo_name}.{skill_name}.agents must be null or a mapping")
                    for agent_name, switch in agents.items():
                        self.assertIn(
                            _canonicalize_agent_id(agent_name),
                            declared_targets,
                            f"{repo_name}.{skill_name}.agents references unknown agent target {agent_name}",
                        )
                        self.assertIsInstance(switch, bool, f"{repo_name}.{skill_name}.agents.{agent_name} must be bool")
