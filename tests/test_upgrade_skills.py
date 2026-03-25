import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-skill" / "upgrade_skills.py"
SPEC = importlib.util.spec_from_file_location("upgrade_skills", MODULE_PATH)
upgrade_skills = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(upgrade_skills)


class UpgradeSkillsTests(unittest.TestCase):
    def test_latest_tag_upgrade_writes_new_commit_and_removes_ref(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_repo = temp_root / "source-repo"
            source_repo.mkdir()
            subprocess.run(["git", "init"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=source_repo, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=source_repo, check=True)
            skill_dir = source_repo / "tracked-skill"
            skill_dir.mkdir()
            skill_md = skill_dir / "SKILL.md"
            skill_md.write_text(
                "---\nname: tracked-skill\ndescription: Example\n---\n\n# v1\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "v1"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "tag", "v1.0.0"], cwd=source_repo, check=True)
            skill_md.write_text(
                "---\nname: tracked-skill\ndescription: Example\n---\n\n# v2\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "v2"], cwd=source_repo, capture_output=True, text=True, check=True)
            commit_v2 = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=source_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            subprocess.run(["git", "tag", "v2.0.0"], cwd=source_repo, check=True)

            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"skills": {"tracked-repo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "tracked-repo": {
                                "url": str(source_repo),
                                "ref": "v1.0.0",
                                "upgrade_strategy": "latest-tag",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {"claude-code": str(temp_root / "targets" / "claude")},
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            updated = json.loads(def_path.read_text(encoding="utf-8"))
            repo = updated["repositories"]["tracked-repo"]
            self.assertEqual(repo["commit"], commit_v2)
            self.assertEqual(repo["upgrade_strategy"], "latest-tag")
            self.assertNotIn("ref", repo)

    def test_latest_tag_upgrade_fails_without_tags_and_preserves_config(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_repo = temp_root / "source-repo"
            source_repo.mkdir()
            subprocess.run(["git", "init"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=source_repo, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=source_repo, check=True)
            skill_dir = source_repo / "tracked-skill"
            skill_dir.mkdir()
            (skill_dir / "SKILL.md").write_text(
                "---\nname: tracked-skill\ndescription: Example\n---\n\n# v1\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "v1"], cwd=source_repo, capture_output=True, text=True, check=True)

            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"skills": {"tracked-repo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "tracked-repo": {
                                "url": str(source_repo),
                                "commit": "deadbeef",
                                "upgrade_strategy": "latest-tag",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {"claude-code": str(temp_root / "targets" / "claude")},
                    }
                ),
                encoding="utf-8",
            )
            before = def_path.read_text(encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("未发现可用 tag", combined)
            self.assertEqual(def_path.read_text(encoding="utf-8"), before)

    def test_latest_release_strategy_uses_github_release_tag(self):
        with (
            mock.patch.object(upgrade_skills, "fetch_latest_github_release_tag", return_value="v2.1.0") as fetch_tag,
            mock.patch.object(
                upgrade_skills,
                "resolve_remote_revision_to_commit",
                return_value="0123456789abcdef0123456789abcdef01234567",
            ) as resolve_commit,
        ):
            commit = upgrade_skills.resolve_latest_commit_for_repository(
                {
                    "url": "https://github.com/example/demo.git",
                    "upgrade_strategy": "latest-release",
                },
                offline=False,
            )

        self.assertEqual(commit, "0123456789abcdef0123456789abcdef01234567")
        fetch_tag.assert_called_once_with("https://github.com/example/demo.git", offline=False)
        resolve_commit.assert_called_once_with(
            "https://github.com/example/demo.git",
            "refs/tags/v2.1.0",
            offline=False,
        )
