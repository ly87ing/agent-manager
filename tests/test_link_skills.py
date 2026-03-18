import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-skill" / "link_skills.py"
SPEC = importlib.util.spec_from_file_location("link_skills", MODULE_PATH)
link_skills = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(link_skills)


class LinkSkillsTests(unittest.TestCase):
    def test_offline_mode_uses_existing_repo_without_git_update(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            repo_path = temp_root / "skills" / "repositories" / "offline-skill"
            agent_target = temp_root / "targets" / "claude"
            (repo_path / ".git").mkdir(parents=True)
            (repo_path / "example-skill").mkdir()
            (repo_path / "example-skill" / "SKILL.md").write_text(
                "---\nname: example-skill\ndescription: Example\n---\n\n# Example\n",
                encoding="utf-8",
            )
            agent_target.parent.mkdir(parents=True)
            config_path.write_text(
                json.dumps({"skills": {"offline-skill": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "offline-skill": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(agent_target),
                        },
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
                    "--base-dir",
                    str(temp_root / "skills"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((agent_target / "example-skill").is_symlink())

    def test_find_skills_uses_metadata_name_from_skill_md(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            skill_dir = temp_root / "repo" / "directory-name"
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text(
                "---\nname: canonical-skill\ndescription: Example\n---\n\n# Example\n",
                encoding="utf-8",
            )

            skills = link_skills.find_skills_recursively(temp_root / "repo")

            self.assertIn("canonical-skill", skills)
            self.assertEqual(skills["canonical-skill"].resolve(), skill_dir.resolve())
            self.assertNotIn("directory-name", skills)

    def test_find_skills_rejects_duplicate_metadata_names(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            first_skill = temp_root / "repo" / "skill-one"
            second_skill = temp_root / "repo" / "skill-two"
            first_skill.mkdir(parents=True)
            second_skill.mkdir(parents=True)
            first_skill.joinpath("SKILL.md").write_text(
                "---\nname: duplicate-skill\ndescription: First\n---\n\n# First\n",
                encoding="utf-8",
            )
            second_skill.joinpath("SKILL.md").write_text(
                "---\nname: duplicate-skill\ndescription: Second\n---\n\n# Second\n",
                encoding="utf-8",
            )

            with self.assertRaises(ValueError):
                link_skills.find_skills_recursively(temp_root / "repo")

    def test_prune_orphan_repos_only_removes_manager_owned_directories(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            cache_dir = Path(temp_dir) / "repositories"
            owned_orphan = cache_dir / "owned-orphan"
            foreign_orphan = cache_dir / "foreign-orphan"
            owned_orphan.mkdir(parents=True)
            foreign_orphan.mkdir()
            (owned_orphan / ".agent-manager-owned").write_text("true\n", encoding="utf-8")

            link_skills.prune_orphan_repos(cache_dir, managed_names=set(), dry_run=False, force_yes=True)

            self.assertFalse(owned_orphan.exists())
            self.assertTrue(foreign_orphan.exists())

    def test_ref_pin_is_honored_in_managed_cache(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_repo = temp_root / "source-repo"
            source_repo.mkdir()
            subprocess.run(["git", "init"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=source_repo, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=source_repo, check=True)
            skill_dir = source_repo / "pinned-skill"
            skill_dir.mkdir()
            skill_md = skill_dir / "SKILL.md"
            skill_md.write_text(
                "---\nname: pinned-skill\ndescription: Example\n---\n\n# v1\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "v1"], cwd=source_repo, capture_output=True, text=True, check=True)
            commit_v1 = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=source_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            subprocess.run(["git", "tag", "v1"], cwd=source_repo, check=True)
            skill_md.write_text(
                "---\nname: pinned-skill\ndescription: Example\n---\n\n# v2\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "v2"], cwd=source_repo, capture_output=True, text=True, check=True)

            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            agent_target.parent.mkdir(parents=True)
            config_path.write_text(
                json.dumps({"skills": {"pinned-repo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "pinned-repo": {
                                "url": str(source_repo),
                                "ref": "v1",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(agent_target),
                        },
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
                    "--base-dir",
                    str(temp_root / "skills"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            managed_repo = temp_root / "skills" / "repositories" / "pinned-repo"
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(managed_repo.exists())
            head = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=managed_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            self.assertEqual(head, commit_v1)
            self.assertTrue((agent_target / "pinned-skill").is_symlink())

    def test_auto_update_false_uses_existing_managed_cache_repo_without_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            repo_path = temp_root / "skills" / "repositories" / "manual-skill"
            agent_target = temp_root / "targets" / "claude"
            (repo_path / ".git").mkdir(parents=True)
            (repo_path / "stable-skill").mkdir()
            (repo_path / "stable-skill" / "SKILL.md").write_text(
                "---\nname: stable-skill\ndescription: Example\n---\n\n# Example\n",
                encoding="utf-8",
            )
            agent_target.parent.mkdir(parents=True)
            config_path.write_text(
                json.dumps({"skills": {"manual-skill": {"enabled": True, "auto_update": False}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "manual-skill": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(agent_target),
                        },
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
                    "--base-dir",
                    str(temp_root / "skills"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((agent_target / "stable-skill").is_symlink())

    def test_clone_failure_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"skills": {"bad-repo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "bad-repo": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "agents": [],
                            }
                        },
                        "agents": {},
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
                    "--base-dir",
                    str(temp_root / "skills"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_duplicate_skill_names_across_repositories_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            repo_a = temp_root / "skills" / "repositories" / "repo-a"
            repo_b = temp_root / "skills" / "repositories" / "repo-b"
            for repo_path in (repo_a, repo_b):
                (repo_path / ".git").mkdir(parents=True)
            (repo_a / "alpha").mkdir()
            (repo_b / "beta").mkdir()
            (repo_a / "alpha" / "SKILL.md").write_text(
                "---\nname: duplicate-skill\ndescription: Alpha\n---\n\n# Alpha\n",
                encoding="utf-8",
            )
            (repo_b / "beta" / "SKILL.md").write_text(
                "---\nname: duplicate-skill\ndescription: Beta\n---\n\n# Beta\n",
                encoding="utf-8",
            )
            agent_target.parent.mkdir(parents=True)
            config_path.write_text(
                json.dumps(
                    {
                        "skills": {
                            "repo-a": {"enabled": True},
                            "repo-b": {"enabled": True},
                        }
                    }
                ),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo-a": {"url": "https://example.invalid/a.git", "agents": ["claude-code"]},
                            "repo-b": {"url": "https://example.invalid/b.git", "agents": ["claude-code"]},
                        },
                        "agents": {"claude-code": str(agent_target)},
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
                    "--base-dir",
                    str(temp_root / "skills"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("duplicate-skill", combined)
            self.assertFalse((agent_target / "duplicate-skill").exists())

    def test_duplicate_skill_names_remove_existing_links_for_conflicted_skill(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            repo_a = temp_root / "skills" / "repositories" / "repo-a"
            repo_b = temp_root / "skills" / "repositories" / "repo-b"
            for repo_path in (repo_a, repo_b):
                (repo_path / ".git").mkdir(parents=True)
            (repo_a / "alpha").mkdir()
            (repo_b / "beta").mkdir()
            (repo_a / "alpha" / "SKILL.md").write_text(
                "---\nname: duplicate-skill\ndescription: Alpha\n---\n\n# Alpha\n",
                encoding="utf-8",
            )
            (repo_b / "beta" / "SKILL.md").write_text(
                "---\nname: duplicate-skill\ndescription: Beta\n---\n\n# Beta\n",
                encoding="utf-8",
            )
            agent_target.mkdir(parents=True)
            existing_link = agent_target / "duplicate-skill"
            existing_link.symlink_to(repo_a / "alpha")
            config_path.write_text(
                json.dumps(
                    {
                        "skills": {
                            "repo-a": {"enabled": True},
                            "repo-b": {"enabled": True},
                        }
                    }
                ),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo-a": {"url": "https://example.invalid/a.git", "agents": ["claude-code"]},
                            "repo-b": {"url": "https://example.invalid/b.git", "agents": ["claude-code"]},
                        },
                        "agents": {"claude-code": str(agent_target)},
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
                    "--base-dir",
                    str(temp_root / "skills"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertFalse(existing_link.exists(), combined)
            self.assertFalse(existing_link.is_symlink(), combined)

    def test_internal_duplicate_skill_names_do_not_remove_other_agent_links(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            claude_target = temp_root / "targets" / "claude"
            codex_target = temp_root / "targets" / "codex"
            repo_a = temp_root / "skills" / "repositories" / "repo-a"
            external_skill = temp_root / "external" / "dup-a"
            (repo_a / ".git").mkdir(parents=True)
            (repo_a / "a1").mkdir()
            (repo_a / "a2").mkdir()
            (repo_a / "a1" / "SKILL.md").write_text(
                "---\nname: dup-a\ndescription: Alpha\n---\n\n# Alpha\n",
                encoding="utf-8",
            )
            (repo_a / "a2" / "SKILL.md").write_text(
                "---\nname: dup-a\ndescription: Beta\n---\n\n# Beta\n",
                encoding="utf-8",
            )
            external_skill.mkdir(parents=True)
            claude_target.mkdir(parents=True)
            codex_target.mkdir(parents=True)
            (claude_target / "dup-a").symlink_to(repo_a / "a1")
            other_agent_link = codex_target / "dup-a"
            other_agent_link.symlink_to(external_skill)
            config_path.write_text(
                json.dumps({"skills": {"repo-a": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo-a": {"url": "https://example.invalid/a.git", "agents": ["claude-code"]},
                        },
                        "agents": {
                            "claude-code": str(claude_target),
                            "codex": str(codex_target),
                        },
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
                    "--base-dir",
                    str(temp_root / "skills"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertFalse((claude_target / "dup-a").exists(), combined)
            self.assertTrue(other_agent_link.exists(), combined)
            self.assertTrue(other_agent_link.is_symlink(), combined)
            self.assertEqual(other_agent_link.resolve(), external_skill.resolve())

    def test_internal_duplicate_skill_names_remove_all_conflicted_links(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            repo_a = temp_root / "skills" / "repositories" / "repo-a"
            (repo_a / ".git").mkdir(parents=True)
            for directory, skill_name in (
                ("a1", "dup-a"),
                ("a2", "dup-a"),
                ("b1", "dup-b"),
                ("b2", "dup-b"),
            ):
                (repo_a / directory).mkdir()
                (repo_a / directory / "SKILL.md").write_text(
                    f"---\nname: {skill_name}\ndescription: Example\n---\n\n# Example\n",
                    encoding="utf-8",
                )
            agent_target.mkdir(parents=True)
            link_a = agent_target / "dup-a"
            link_b = agent_target / "dup-b"
            link_a.symlink_to(repo_a / "a1")
            link_b.symlink_to(repo_a / "b1")
            config_path.write_text(
                json.dumps({"skills": {"repo-a": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo-a": {"url": "https://example.invalid/a.git", "agents": ["claude-code"]},
                        },
                        "agents": {"claude-code": str(agent_target)},
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
                    "--base-dir",
                    str(temp_root / "skills"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("dup-a", combined)
            self.assertIn("dup-b", combined)
            self.assertFalse(link_a.exists(), combined)
            self.assertFalse(link_b.exists(), combined)

    def test_internal_duplicate_skill_names_remove_existing_non_conflicted_links_from_invalid_repo(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            repo_a = temp_root / "skills" / "repositories" / "repo-a"
            (repo_a / ".git").mkdir(parents=True)
            (repo_a / "dup1").mkdir()
            (repo_a / "dup2").mkdir()
            (repo_a / "stable").mkdir()
            (repo_a / "dup1" / "SKILL.md").write_text(
                "---\nname: dup-a\ndescription: First\n---\n\n# First\n",
                encoding="utf-8",
            )
            (repo_a / "dup2" / "SKILL.md").write_text(
                "---\nname: dup-a\ndescription: Second\n---\n\n# Second\n",
                encoding="utf-8",
            )
            (repo_a / "stable" / "SKILL.md").write_text(
                "---\nname: stable-skill\ndescription: Stable\n---\n\n# Stable\n",
                encoding="utf-8",
            )
            agent_target.mkdir(parents=True)
            stable_link = agent_target / "stable-skill"
            stable_link.symlink_to(repo_a / "stable")
            config_path.write_text(
                json.dumps({"skills": {"repo-a": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo-a": {"url": "https://example.invalid/a.git", "agents": ["claude-code"]},
                        },
                        "agents": {"claude-code": str(agent_target)},
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
                    "--base-dir",
                    str(temp_root / "skills"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertFalse(stable_link.exists(), combined)
            self.assertFalse(stable_link.is_symlink(), combined)

    def test_scrub_dead_links_removes_broken_relative_link(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            managed_dir = temp_root / "managed"
            agent_dir = temp_root / "agent" / "skills"
            managed_dir.mkdir()
            agent_dir.mkdir(parents=True)

            target = managed_dir / "repo" / "skill"
            target.parent.mkdir(parents=True)
            target.mkdir()
            link = agent_dir / "skill"
            link.symlink_to(os.path.relpath(target, agent_dir))

            target.rmdir()
            target.parent.rmdir()

            link_skills.scrub_dead_links({"TestAgent": str(agent_dir)}, managed_dir)

            self.assertFalse(link.is_symlink())

    def test_default_skill_catalog_includes_ui_ux_pro_max_repo_for_all_agents(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        repo = skills_config["repositories"].get("ui-ux-pro-max-skill")

        self.assertIsNotNone(repo)
        self.assertEqual(repo["url"], "https://github.com/nextlevelbuilder/ui-ux-pro-max-skill")
        self.assertEqual(repo["commit"], "07f4ef3ac2568c25a3b0c8ef5165a86abc3e56e4")
        self.assertIsNone(repo["agents"])

    def test_default_manager_policy_explicitly_enables_ui_ux_pro_max_repo(self):
        manager_config = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertEqual(
            manager_config["skills"].get("ui-ux-pro-max-skill"),
            {"enabled": True},
        )
