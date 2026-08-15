import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-skill" / "link_skills.py"
SPEC = importlib.util.spec_from_file_location("link_skills", MODULE_PATH)
link_skills = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(link_skills)


# Python 3.14's argparse colorizes --help output (respecting NO_COLOR/FORCE_COLOR),
# which inserts SGR escapes between flag tokens. Strip them so text assertions test
# the documented content, not the coloring.
_ANSI_SGR_RE = re.compile(r"\x1b\[[0-9;]*m")


def _strip_ansi(text):
    return _ANSI_SGR_RE.sub("", text)


class LinkSkillsTests(unittest.TestCase):
    def test_help_lists_force_refresh_short_flag(self):
        result = subprocess.run(
            ["python3", str(MODULE_PATH), "-h"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)
        self.assertIn("-f, --force-refresh", _strip_ansi(combined))

    def test_codex_is_globally_active_without_lifecycle_definition(self):
        self.assertTrue(link_skills.is_active_skill_target("codex", "/tmp/.codex/skills", {}))

    def test_main_allows_scoped_repo_targets_that_are_declared_but_inactive(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            policy_root = {"skills": {"scoped-repo": {"enabled": False}}}
            full_def = {
                "repositories": {
                    "scoped-repo": {
                        "url": "https://example.invalid/repo.git",
                        "agents": ["claude-code", "codex"],
                    }
                },
                "agents": {
                    "claude-code": str(temp_root / "targets" / "claude"),
                    "codex": str(temp_root / "targets" / "codex"),
                },
            }

            with (
                mock.patch.object(
                    link_skills,
                    "load_json_object_file",
                    side_effect=[policy_root, full_def, {"claude-code": {}, "codex": {}}],
                ),
                mock.patch.object(
                    link_skills,
                    "is_active_skill_target",
                    side_effect=lambda agent_name, *_args: agent_name == "claude-code",
                ),
                mock.patch.object(
                    sys,
                    "argv",
                    [
                        str(MODULE_PATH),
                        "--config",
                        str(temp_root / "config.json"),
                        "--def-file",
                        str(temp_root / "defs.json"),
                        "--base-dir",
                        str(temp_root / "skills"),
                        "--dry-run",
                        "--yes",
                    ],
                ),
            ):
                link_skills.main()

    def test_resolve_repo_link_targets_includes_globally_active_codex_for_unscoped_repo(self):
        active_targets = {
            "claude-code": "/tmp/claude",
            "codex": "/tmp/.codex/skills",
        }
        all_targets = {
            "claude-code": "/tmp/claude",
            "codex": "/tmp/.codex/skills",
        }

        unscoped_targets = link_skills.resolve_repo_link_targets(active_targets, all_targets, None)
        scoped_targets = link_skills.resolve_repo_link_targets(active_targets, all_targets, ["codex"])

        self.assertEqual(unscoped_targets, active_targets)
        self.assertEqual(scoped_targets, {"codex": "/tmp/.codex/skills"})

    def test_resolve_repo_link_targets_reports_explicit_codex_targets(self):
        active_targets = {"claude-code": "/tmp/claude"}
        all_targets = {
            "claude-code": "/tmp/claude",
            "codex": "/tmp/.codex/skills",
        }

        scoped_targets = link_skills.resolve_repo_link_targets(active_targets, all_targets, ["claude-code", "codex"])

        self.assertEqual(set(scoped_targets), {"claude-code", "codex"})

    def test_invalid_policy_config_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text("{ invalid json\n", encoding="utf-8")
            def_path.write_text('{"repositories": {}, "agents": {}}', encoding="utf-8")

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

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("config.json", combined)
            self.assertIn("无效", combined)

    def test_invalid_definition_config_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text("{ invalid json\n", encoding="utf-8")

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

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("defs.json", combined)
            self.assertIn("无效", combined)

    def test_non_object_definition_section_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text('{"repositories": [], "agents": {}}', encoding="utf-8")

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

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("defs.json.repositories", combined)
            self.assertIn("必须是对象", combined)

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

    def test_clone_or_update_repo_skips_pull_when_probe_reports_latest(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            managed_root = Path(temp_dir) / "repositories"
            repo_path = managed_root / "demo-skill"
            (repo_path / ".git").mkdir(parents=True)

            with mock.patch.object(
                link_skills,
                "probe_git_remote_state",
                return_value={"status": "latest", "remote_head": "abc", "local_remote_head": "abc"},
            ):
                with mock.patch.object(link_skills, "repo_head_is_detached", return_value=False):
                    with mock.patch.object(link_skills, "run_cmd") as run_cmd:
                        repo, ok = link_skills.clone_or_update_repo(
                            "demo-skill",
                            "https://example.invalid/repo.git",
                            managed_root,
                        )

        self.assertEqual(repo, repo_path)
        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_count, 0)

    def test_clone_or_update_repo_probe_failure_skips_pull_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            managed_root = Path(temp_dir) / "repositories"
            repo_path = managed_root / "demo-skill"
            (repo_path / ".git").mkdir(parents=True)

            with mock.patch.object(link_skills, "probe_git_remote_state", return_value={"status": "unknown"}):
                with mock.patch.object(link_skills, "repo_head_is_detached", return_value=False):
                    with mock.patch.object(link_skills, "run_cmd") as run_cmd:
                        repo, ok = link_skills.clone_or_update_repo(
                            "demo-skill",
                            "https://example.invalid/repo.git",
                            managed_root,
                        )

        self.assertEqual(repo, repo_path)
        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_count, 0)

    def test_clone_or_update_repo_probe_failure_force_refresh_runs_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            managed_root = Path(temp_dir) / "repositories"
            repo_path = managed_root / "demo-skill"
            (repo_path / ".git").mkdir(parents=True)

            with mock.patch.object(link_skills, "probe_git_remote_state", return_value={"status": "unknown"}):
                with mock.patch.object(link_skills, "repo_head_is_detached", return_value=False):
                    with mock.patch.object(link_skills, "run_cmd", return_value=(True, "updated")) as run_cmd:
                        repo, ok = link_skills.clone_or_update_repo(
                            "demo-skill",
                            "https://example.invalid/repo.git",
                            managed_root,
                            force_refresh=True,
                        )

        self.assertEqual(repo, repo_path)
        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_args_list, [mock.call(["git", "pull", "--ff-only"], cwd=repo_path)])

    def test_write_owner_marker_adds_marker_to_local_git_exclude(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_path = Path(temp_dir) / "repositories" / "demo-skill"
            (repo_path / ".git" / "info").mkdir(parents=True)
            (repo_path / ".git" / "info" / "exclude").write_text("# local excludes\n", encoding="utf-8")

            link_skills.write_owner_marker(repo_path, "demo-skill")

            marker = json.loads((repo_path / ".agent-manager-owned").read_text(encoding="utf-8"))
            exclude_lines = (repo_path / ".git" / "info" / "exclude").read_text(encoding="utf-8").splitlines()
            self.assertEqual(marker, {"managed_by": "agent-manager", "repository": "demo-skill"})
            self.assertIn(".agent-manager-owned", exclude_lines)

    def test_write_owner_marker_does_not_duplicate_local_git_exclude(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_path = Path(temp_dir) / "repositories" / "demo-skill"
            (repo_path / ".git" / "info").mkdir(parents=True)

            link_skills.write_owner_marker(repo_path, "demo-skill")
            link_skills.write_owner_marker(repo_path, "demo-skill")

            exclude_lines = (repo_path / ".git" / "info" / "exclude").read_text(encoding="utf-8").splitlines()
            self.assertEqual(exclude_lines.count(".agent-manager-owned"), 1)

    def test_missing_policy_repository_is_ignored(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "ignored-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(temp_root / "targets" / "claude"),
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
                    "--dry-run",
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("未声明策略，跳过", combined)
            self.assertNotIn("准备下载到托管缓存", combined)

    def test_dry_run_missing_repo_with_skill_paths_does_not_fail(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            target_dir = temp_root / "targets" / "codex"
            target_dir.mkdir(parents=True)
            config_path.write_text(
                json.dumps({"skills": {"demo-skill": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-skill": {
                                "url": "https://example.invalid/repo.git",
                                "skill_paths": ["bundle/frontend-skill"],
                                "agents": ["codex"],
                            }
                        },
                        "agents": {
                            "codex": str(target_dir),
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
                    str(base_dir),
                    "--dry-run",
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("本地仓库缺失，跳过 skill 枚举", combined)

    def test_missing_policy_repository_removes_existing_managed_links(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "ignored-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            os.symlink(skill_dir, target_dir / "my-skill")
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "ignored-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertFalse((target_dir / "my-skill").exists())
            self.assertIn("未声明策略，跳过", combined)
            self.assertIn("移除来自失效仓库的 skill 链接: my-skill", combined)

    def test_validate_repo_disabled_rejects_non_list(self):
        with self.assertRaises(ValueError):
            link_skills.validate_repo_disabled("demo", {"disabled": "safe-merge-review"})
        with self.assertRaises(ValueError):
            link_skills.validate_repo_disabled("demo", {"disabled": ["", "ok"]})
        # 缺省与合法数组都不报错
        link_skills.validate_repo_disabled("demo", {})
        link_skills.validate_repo_disabled("demo", {"disabled": ["a", "b"]})

    def test_partition_disabled_skills_filters_named_skill(self):
        skills = {"keep": Path("/x/keep"), "drop": Path("/x/drop")}
        kept, active = link_skills.partition_disabled_skills(
            "demo", {"disabled": ["drop"]}, skills
        )
        self.assertEqual(set(kept), {"keep"})
        self.assertEqual(active, {"drop"})

    def test_partition_disabled_skills_absent_keeps_all(self):
        skills = {"a": Path("/x/a"), "b": Path("/x/b")}
        kept, active = link_skills.partition_disabled_skills("demo", {}, skills)
        self.assertEqual(kept, skills)
        self.assertEqual(active, set())

    def test_partition_disabled_skills_unknown_name_is_ignored(self):
        skills = {"a": Path("/x/a")}
        kept, active = link_skills.partition_disabled_skills(
            "demo", {"disabled": ["ghost"]}, skills
        )
        self.assertEqual(kept, skills)
        self.assertEqual(active, set())

    def test_disabled_skill_removes_existing_link_and_keeps_others(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_path = base_dir / "repositories" / "demo"
            agent_target = temp_root / "targets" / "claude"
            (repo_path / ".git").mkdir(parents=True)
            for name in ("skill-keep", "skill-drop"):
                (repo_path / name).mkdir()
                (repo_path / name / "SKILL.md").write_text(
                    f"---\nname: {name}\ndescription: demo\n---\n", encoding="utf-8"
                )
            agent_target.mkdir(parents=True)
            # 预置旧链：两个都曾被链接，本次把 skill-drop 禁用
            os.symlink(repo_path / "skill-drop", agent_target / "skill-drop")
            config_path.write_text(
                json.dumps(
                    {"skills": {"demo": {"enabled": True, "disabled": ["skill-drop"]}}}
                ),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "agents": ["claude-code"],
                            }
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
                    str(base_dir),
                    "--offline",
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((agent_target / "skill-keep").is_symlink(), combined)
            self.assertFalse((agent_target / "skill-drop").exists(), combined)
            self.assertIn("本机禁用 skill", combined)

    def test_missing_policy_repository_preserves_unmanaged_alias_link(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "ignored-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            alias_link = target_dir / "alias-skill"
            alias_link.symlink_to(skill_dir)
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "ignored-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(alias_link.exists(), combined)
            self.assertTrue(alias_link.is_symlink(), combined)

    def test_missing_policy_repository_preserves_same_name_unmanaged_live_link(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "ignored-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            external_skill = temp_root / "external" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            external_skill.mkdir(parents=True)
            live_link = target_dir / "my-skill"
            live_link.symlink_to(external_skill)
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "ignored-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(live_link.exists(), combined)
            self.assertTrue(live_link.is_symlink(), combined)

    def test_managed_skill_name_replaces_local_directory_without_touching_other_names(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "demo-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            (target_dir / "my-skill").mkdir()
            (target_dir / "my-skill" / "local.txt").write_text("keep? no\n", encoding="utf-8")
            (target_dir / "user-skill").mkdir()
            (target_dir / "user-skill" / "local.txt").write_text("keep\n", encoding="utf-8")
            config_path.write_text(
                json.dumps({"skills": {"demo-skill": {"enabled": True, "auto_update": False}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            managed_link = target_dir / "my-skill"
            self.assertTrue(managed_link.is_symlink(), combined)
            self.assertEqual(managed_link.resolve(), skill_dir.resolve())
            self.assertTrue((target_dir / "user-skill").is_dir(), combined)
            self.assertTrue((target_dir / "user-skill" / "local.txt").exists(), combined)

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

    def test_create_link_handles_var_private_var_path_aliases(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_dir = temp_root / "skills" / "repositories" / "demo" / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            source_dir.mkdir(parents=True)
            target_dir.mkdir(parents=True)
            link_path = target_dir / "my-skill"

            link_skills.create_link(source_dir, link_path, dry_run=False)

            self.assertTrue(link_path.is_symlink())
            self.assertTrue(link_path.exists())
            self.assertEqual(link_path.resolve(), source_dir.resolve())

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

    def test_prune_orphan_repos_removes_owned_directory_when_tty_confirms_yes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            cache_dir = Path(temp_dir) / "repositories"
            owned_orphan = cache_dir / "owned-orphan"
            owned_orphan.mkdir(parents=True)
            (owned_orphan / ".agent-manager-owned").write_text("true\n", encoding="utf-8")

            with (
                mock.patch.object(sys.stdin, "isatty", return_value=True),
                mock.patch("builtins.input", return_value="y"),
            ):
                planned_removed = link_skills.prune_orphan_repos(
                    cache_dir,
                    managed_names=set(),
                    dry_run=False,
                    force_yes=False,
                )

            self.assertFalse(owned_orphan.exists())
            self.assertEqual(planned_removed, [owned_orphan.resolve(strict=False)])

    def test_prune_orphan_repos_preserves_owned_directory_when_tty_declines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            cache_dir = Path(temp_dir) / "repositories"
            owned_orphan = cache_dir / "owned-orphan"
            owned_orphan.mkdir(parents=True)
            (owned_orphan / ".agent-manager-owned").write_text("true\n", encoding="utf-8")

            with (
                mock.patch.object(sys.stdin, "isatty", return_value=True),
                mock.patch("builtins.input", return_value="n"),
            ):
                planned_removed = link_skills.prune_orphan_repos(
                    cache_dir,
                    managed_names=set(),
                    dry_run=False,
                    force_yes=False,
                )

            self.assertTrue(owned_orphan.exists())
            self.assertEqual(planned_removed, [])

    def test_removed_repository_definition_scrubs_dead_links_in_same_run(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "orphan-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            os.symlink(skill_dir, target_dir / "my-skill")
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {},
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertFalse(repo_dir.exists())
            self.assertFalse((target_dir / "my-skill").exists())
            self.assertIn("发现孤儿仓库: orphan-skill", combined)
            self.assertIn("移除死链: my-skill", combined)

    def test_dry_run_previews_orphan_repository_cleanup(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "orphan-skill"
            repo_dir.mkdir(parents=True)
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {},
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
                    str(base_dir),
                    "--dry-run",
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(repo_dir.exists())
            self.assertIn("发现孤儿仓库: orphan-skill", combined)

    def test_dry_run_previews_dead_link_cleanup_after_orphan_repository_removal(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "orphan-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            os.symlink(skill_dir, target_dir / "my-skill")
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {},
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--dry-run",
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(repo_dir.exists())
            self.assertTrue((target_dir / "my-skill").exists())
            self.assertIn("发现孤儿仓库: orphan-skill", combined)
            self.assertIn("移除死链: my-skill", combined)

    def test_dry_run_without_yes_does_not_preview_dead_link_cleanup_after_orphan_repository_removal(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "orphan-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            target_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            os.symlink(skill_dir, target_dir / "my-skill")
            config_path.write_text('{"skills": {}}', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {},
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--dry-run",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(repo_dir.exists())
            self.assertTrue((target_dir / "my-skill").exists())
            self.assertIn("发现孤儿仓库: orphan-skill", combined)
            self.assertNotIn("移除死链: my-skill", combined)

    def test_dry_run_previews_dead_link_cleanup_for_currently_managed_skill(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "demo-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            target_dir = temp_root / "targets" / "claude"
            foreign_dir = temp_root / "foreign"
            target_dir.mkdir(parents=True)
            foreign_dir.mkdir()
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            os.symlink(foreign_dir / "missing-skill", target_dir / "my-skill")
            config_path.write_text(
                json.dumps({"skills": {"demo-skill": {"enabled": True, "auto_update": False}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
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
                    str(base_dir),
                    "--dry-run",
                    "--offline",
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((target_dir / "my-skill").is_symlink())
            self.assertIn("(预览: 计划加载 my-skill)", combined)
            self.assertIn("移除死链: my-skill", combined)

    def test_enabled_repository_removes_links_from_disallowed_agents(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "demo-skill"
            skill_dir = repo_dir / "bundle" / "my-skill"
            claude_dir = temp_root / "targets" / "claude"
            codex_dir = temp_root / "targets" / "codex"
            claude_dir.mkdir(parents=True)
            codex_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            subprocess.run(["git", "init"], cwd=repo_dir, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo_dir, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=repo_dir, check=True)
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n", encoding="utf-8")
            subprocess.run(["git", "add", "."], cwd=repo_dir, check=True)
            subprocess.run(["git", "commit", "-m", "seed"], cwd=repo_dir, capture_output=True, text=True, check=True)
            os.symlink(skill_dir, claude_dir / "my-skill")
            os.symlink(skill_dir, codex_dir / "my-skill")
            config_path.write_text(
                json.dumps({"skills": {"demo-skill": {"enabled": True, "auto_update": False, "agents": ["claude-code"]}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-skill": {
                                "url": "https://example.invalid/repo.git",
                                "agents": None,
                            }
                        },
                        "agents": {
                            "claude-code": str(claude_dir),
                            "codex": str(codex_dir),
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
                    str(base_dir),
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((claude_dir / "my-skill").exists())
            self.assertFalse((codex_dir / "my-skill").exists())
            self.assertIn("移除来自失效仓库的 skill 链接: my-skill", combined)

    def test_codex_target_can_receive_links_without_lifecycle_agent_definition(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_repo = temp_root / "source-repo"
            source_repo.mkdir()
            subprocess.run(["git", "init"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=source_repo, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=source_repo, check=True)
            skill_dir = source_repo / "skills" / ".curated" / "frontend-skill"
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text(
                "---\nname: frontend-skill\ndescription: OpenAI\n---\n\n# Frontend Skill\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "seed"], cwd=source_repo, capture_output=True, text=True, check=True)

            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            codex_target = temp_root / "targets" / "codex" / "skills"
            config_path.write_text(
                json.dumps({"skills": {"openai-frontend-skill": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "openai-frontend-skill": {
                                "url": str(source_repo),
                                "skill_paths": ["skills/.curated/frontend-skill"],
                                "agents": ["codex"],
                            }
                        },
                        "agents": {
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
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(codex_target.parent.exists(), combined)
            self.assertTrue((codex_target / "frontend-skill").is_symlink(), combined)

    def test_unscoped_repository_keeps_codex_link(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            base_dir = temp_root / "skills"
            repo_dir = base_dir / "repositories" / "shared-repo"
            skill_dir = repo_dir / "bundle" / "shared-skill"
            codex_dir = temp_root / "targets" / "codex" / "skills"
            codex_dir.mkdir(parents=True)
            repo_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text("---\nname: shared-skill\n---\n", encoding="utf-8")
            (codex_dir / "shared-skill").symlink_to(skill_dir)
            config_path.write_text(
                json.dumps({"skills": {"shared-repo": {"enabled": True, "auto_update": False}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "shared-repo": {
                                "url": "https://example.invalid/repo.git",
                                "agents": None,
                            }
                        },
                        "agents": {
                            "codex": str(codex_dir),
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
                    str(base_dir),
                    "--yes",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((codex_dir / "shared-skill").exists(), combined)

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

    def test_local_skill_repository_pin_is_honored_in_managed_cache(self):
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
            skill_md.write_text(
                "---\nname: pinned-skill\ndescription: Example\n---\n\n# v2\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "v2"], cwd=source_repo, capture_output=True, text=True, check=True)

            config_path = temp_root / "config.json"
            def_path = temp_root / "skills.json"
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
            (temp_root / "version-pins.local.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "version_locks": {},
                        "skill_repositories": {"pinned-repo": commit_v1},
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
            head = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=managed_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            self.assertEqual(head, commit_v1)
            self.assertTrue((agent_target / "pinned-skill").is_symlink())

    def test_detached_managed_repo_uses_fetch_instead_of_pull(self):
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
                                "commit": commit_v1,
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

            first = subprocess.run(
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
            first_output = first.stdout + first.stderr
            self.assertEqual(first.returncode, 0, first_output)

            second = subprocess.run(
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

            second_output = second.stdout + second.stderr
            self.assertEqual(second.returncode, 0, second_output)
            self.assertNotIn("You are not currently on a branch.", second_output)

    def test_unpinned_repository_switches_detached_cache_back_to_default_branch_latest(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_repo = temp_root / "source-repo"
            source_repo.mkdir()
            subprocess.run(["git", "init"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=source_repo, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=source_repo, check=True)
            default_branch = subprocess.run(
                ["git", "branch", "--show-current"],
                cwd=source_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            skill_dir = source_repo / "tracked-skill"
            skill_dir.mkdir()
            skill_md = skill_dir / "SKILL.md"
            skill_md.write_text(
                "---\nname: tracked-skill\ndescription: Example\n---\n\n# v1\n",
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

            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            agent_target.parent.mkdir(parents=True)
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
                                "commit": commit_v1,
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

            first = subprocess.run(
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
            first_output = first.stdout + first.stderr
            self.assertEqual(first.returncode, 0, first_output)

            managed_repo = temp_root / "skills" / "repositories" / "tracked-repo"
            detached_head = subprocess.run(
                ["git", "symbolic-ref", "-q", "HEAD"],
                cwd=managed_repo,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(detached_head.returncode, 0, detached_head.stdout + detached_head.stderr)

            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "tracked-repo": {
                                "url": str(source_repo),
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

            second = subprocess.run(
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

            second_output = second.stdout + second.stderr
            self.assertEqual(second.returncode, 0, second_output)
            head = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=managed_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            current_branch = subprocess.run(
                ["git", "branch", "--show-current"],
                cwd=managed_repo,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            self.assertEqual(head, commit_v2)
            self.assertEqual(current_branch, default_branch)
            self.assertTrue((agent_target / "tracked-skill").is_symlink())

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

    def test_selected_skill_paths_limit_repo_exposure(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            source_repo = temp_root / "source-repo"
            source_repo.mkdir()
            subprocess.run(["git", "init"], cwd=source_repo, capture_output=True, text=True, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=source_repo, check=True)
            subprocess.run(["git", "config", "user.name", "Test User"], cwd=source_repo, check=True)
            selected_dir = source_repo / "skills" / ".curated" / "frontend-skill"
            other_dir = source_repo / "skills" / "other-skill"
            selected_dir.mkdir(parents=True)
            other_dir.mkdir(parents=True)
            (selected_dir / "SKILL.md").write_text(
                "---\nname: frontend-skill\ndescription: Selected\n---\n\n# Selected\n",
                encoding="utf-8",
            )
            (other_dir / "SKILL.md").write_text(
                "---\nname: other-skill\ndescription: Unselected\n---\n\n# Other\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=source_repo, check=True)
            subprocess.run(["git", "commit", "-m", "seed"], cwd=source_repo, capture_output=True, text=True, check=True)

            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            agent_target = temp_root / "targets" / "claude"
            agent_target.parent.mkdir(parents=True)
            config_path.write_text(
                json.dumps({"skills": {"openai-frontend-skill": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "openai-frontend-skill": {
                                "url": str(source_repo),
                                "skill_paths": ["skills/.curated/frontend-skill"],
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

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((agent_target / "frontend-skill").is_symlink(), combined)
            self.assertFalse((agent_target / "other-skill").exists(), combined)

    def test_scoped_repository_only_links_declared_skill_paths(self):
        self.test_selected_skill_paths_limit_repo_exposure()

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

    def test_scrub_dead_links_preserves_broken_relative_link_without_managed_ownership(self):
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

            self.assertTrue(link.is_symlink())

    def test_scrub_dead_links_removes_broken_relative_link_for_current_agent_managed_skill(self):
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

            link_skills.scrub_dead_links(
                {"TestAgent": str(agent_dir)},
                managed_dir,
                managed_skill_names_by_agent={"TestAgent": {"skill"}},
            )

            self.assertFalse(link.is_symlink())

    def test_scrub_dead_links_preserves_broken_link_outside_managed_dir_with_same_prefix(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            managed_dir = temp_root / "managed"
            foreign_dir = temp_root / "managed-foreign"
            agent_dir = temp_root / "agent" / "skills"
            managed_dir.mkdir()
            foreign_dir.mkdir()
            agent_dir.mkdir(parents=True)

            link = agent_dir / "skill"
            link.symlink_to(foreign_dir / "missing-skill")

            link_skills.scrub_dead_links({"TestAgent": str(agent_dir)}, managed_dir)

            self.assertTrue(link.is_symlink())

    def test_scrub_dead_links_preserves_broken_link_for_disabled_repo_name(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            managed_dir = temp_root / "managed"
            repo_dir = managed_dir / "disabled-repo"
            skill_dir = repo_dir / "bundle" / "skill"
            foreign_dir = temp_root / "foreign"
            agent_dir = temp_root / "agent" / "skills"
            repo_dir.mkdir(parents=True)
            skill_dir.mkdir(parents=True)
            foreign_dir.mkdir()
            agent_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (skill_dir / "SKILL.md").write_text("---\nname: skill\n---\n", encoding="utf-8")

            link = agent_dir / "skill"
            link.symlink_to(foreign_dir / "missing-skill")

            link_skills.scrub_dead_links({"TestAgent": str(agent_dir)}, managed_dir)

            self.assertTrue(link.is_symlink())

    def test_scrub_dead_links_removes_broken_link_for_currently_managed_skill_name(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            managed_dir = temp_root / "managed"
            repo_dir = managed_dir / "demo-repo"
            skill_dir = repo_dir / "bundle" / "skill"
            foreign_dir = temp_root / "managed-foreign"
            agent_dir = temp_root / "agent" / "skills"
            repo_dir.mkdir(parents=True)
            skill_dir.mkdir(parents=True)
            foreign_dir.mkdir()
            agent_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (skill_dir / "SKILL.md").write_text("---\nname: skill\n---\n", encoding="utf-8")

            link = agent_dir / "skill"
            link.symlink_to(foreign_dir / "missing-skill")

            link_skills.scrub_dead_links(
                {"TestAgent": str(agent_dir)},
                managed_dir,
                managed_skill_names_by_agent={"TestAgent": {"skill"}},
            )

            self.assertFalse(link.is_symlink())

    def test_scrub_dead_links_preserves_broken_link_for_skill_managed_by_other_agent(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            managed_dir = temp_root / "managed"
            repo_dir = managed_dir / "codex-repo"
            skill_dir = repo_dir / "bundle" / "frontend-skill"
            foreign_dir = temp_root / "foreign"
            agent_dir = temp_root / "agent" / "skills"
            repo_dir.mkdir(parents=True)
            skill_dir.mkdir(parents=True)
            foreign_dir.mkdir()
            agent_dir.mkdir(parents=True)
            (repo_dir / ".git").mkdir()
            (repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (skill_dir / "SKILL.md").write_text("---\nname: frontend-skill\n---\n", encoding="utf-8")

            link = agent_dir / "frontend-skill"
            link.symlink_to(foreign_dir / "missing-skill")

            link_skills.scrub_dead_links(
                {"claude-code": str(agent_dir)},
                managed_dir,
                managed_skill_names_by_agent={"codex": {"frontend-skill"}},
            )

            self.assertTrue(link.is_symlink())

    def test_is_path_within_base_accepts_windows_substitution_path(self):
        managed_path = r"\\?\C:\repo\agent-skill\repositories\demo-repo\bundle\frontend-skill"
        base_path = r"C:\repo\agent-skill\repositories"

        self.assertTrue(link_skills.is_path_within_base(managed_path, base_path))

    def test_default_skill_catalog_includes_agent_skills_repo_with_explicit_agents(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        repo = skills_config["repositories"].get("agent-skills")

        self.assertIsNotNone(repo)
        self.assertEqual(repo["url"], "https://github.com/ly87ing/agent-skills.git")
        self.assertEqual(
            repo["skill_paths"],
            [
                "safe-merge-review",
                "change-discipline",
                "frontend-verification",
                "artifact-hygiene",
                "reader-facing-writing",
            ],
        )
        self.assertEqual(repo["skill_count"], 5)
        self.assertEqual(repo["agents"], ["claude-code", "codex"])
        self.assertNotIn("ignored_skill_paths", repo)
        self.assertRegex(repo["commit"], r"^[0-9a-f]{40}$")
        self.assertEqual(repo["upgrade_strategy"], "latest-branch")
        self.assertEqual(repo["upgrade_ref"], "refs/heads/main")
        self.assertNotIn("ref", repo)

    def test_default_skill_catalog_does_not_reference_removed_playwright_skill(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        self.assertIsNone(skills_config["repositories"].get("playwright-best-practices-skill"))

    def test_default_manager_policy_explicitly_enables_agent_skills_repo(self):
        manager_config = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertEqual(
            manager_config["skills"].get("agent-skills"),
            {"enabled": True},
        )

    def test_default_skill_catalog_does_not_reference_removed_openai_frontend_skill(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        repo = skills_config["repositories"].get("openai-frontend-skill")

        self.assertIsNone(repo)

    def test_default_skill_catalog_does_not_reference_removed_anthropic_frontend_design_skill(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        self.assertIsNone(skills_config["repositories"].get("anthropic-frontend-design-skill"))

    def test_default_manager_policy_does_not_enable_removed_openai_frontend_skill(self):
        manager_config = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertIsNone(manager_config["skills"].get("openai-frontend-skill"))

    def test_default_manager_policy_does_not_enable_removed_anthropic_frontend_design_skill(self):
        manager_config = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertIsNone(manager_config["skills"].get("anthropic-frontend-design-skill"))
