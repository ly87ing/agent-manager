import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SkillExposureContractTests(unittest.TestCase):
    def test_multi_skill_repositories_require_explicit_agents(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        for repo_name, repo in skills_config["repositories"].items():
            repo_path = ROOT / "agent-skill" / "repositories" / repo_name
            raw_paths = repo.get("skill_paths")
            search_paths = [repo_path / raw_path for raw_path in raw_paths] if raw_paths else [repo_path]

            skill_count = 0
            for search_path in search_paths:
                skill_count += sum(1 for _ in search_path.rglob("SKILL.md"))

            if skill_count > 1:
                self.assertIsNotNone(repo.get("agents"), f"{repo_name} must explicitly scope agents")
