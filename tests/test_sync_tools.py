import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.agent_tool_import import sync_tools


class SyncToolsTests(unittest.TestCase):
    def test_dirty_git_repo_is_not_reset_hard(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()
            sync_tools.git_cache._cache = {}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == ["git", "status", "--porcelain"]:
                    return True, " M package.json\n"
                if cmd == ["git", "reset", "--hard"]:
                    return True, "reset"
                if cmd == ["git", "pull"]:
                    return True, "pull"
                return True, ""

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd) as run_cmd:
                with mock.patch.object(sync_tools, "apply_overrides"):
                    sync_tools.sync_tool(
                        "playwright-cli",
                        {"url": "https://example.invalid/repo.git"},
                        {"enabled": True, "auto_update": True},
                        Path(temp_dir),
                    )

            reset_calls = [call for call in run_cmd.call_args_list if call.args and call.args[0] == ["git", "reset", "--hard"]]
            self.assertEqual(reset_calls, [])

    def test_dirty_git_repo_does_not_apply_overrides(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()
            sync_tools.git_cache._cache = {}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == ["git", "status", "--porcelain"]:
                    return True, " M package.json\n"
                return True, ""

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "apply_overrides") as apply_overrides:
                    sync_tools.sync_tool(
                        "playwright-cli",
                        {"url": "https://example.invalid/repo.git"},
                        {"enabled": True, "auto_update": True},
                        Path(temp_dir),
                    )

            self.assertEqual(apply_overrides.call_count, 0)

    def test_clone_failure_does_not_apply_overrides(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if isinstance(cmd, list) and cmd[:2] == ["git", "clone"]:
                    return False, "clone failed"
                return True, ""

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "apply_overrides") as apply_overrides:
                    sync_tools.sync_tool(
                        "playwright-cli",
                        {"url": "https://example.invalid/repo.git"},
                        {"enabled": True, "auto_update": True},
                        Path(temp_dir),
                    )

            self.assertEqual(apply_overrides.call_count, 0)

    def test_update_failure_does_not_apply_overrides(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()
            sync_tools.git_cache._cache = {}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == ["git", "status", "--porcelain"]:
                    return True, ""
                if cmd == ["git", "ls-remote", "origin", "HEAD"]:
                    return True, "remotehash\tHEAD\n"
                if cmd == ["git", "rev-parse", "HEAD"]:
                    return True, "localhash\n"
                if cmd == ["git", "pull"]:
                    return False, "pull failed"
                return True, ""

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "apply_overrides") as apply_overrides:
                    sync_tools.sync_tool(
                        "playwright-cli",
                        {"url": "https://example.invalid/repo.git"},
                        {"enabled": True, "auto_update": True},
                        Path(temp_dir),
                    )

            self.assertEqual(apply_overrides.call_count, 0)

    def test_setup_command_is_not_subject_to_global_300_second_timeout(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            calls = []

            def fake_subprocess_run(
                cmd,
                cwd=None,
                check=True,
                capture_output=True,
                text=True,
                shell=False,
                timeout=None,
            ):
                calls.append({"cmd": cmd, "cwd": cwd, "timeout": timeout, "shell": shell})
                if isinstance(cmd, list) and cmd[:2] == ["git", "clone"]:
                    target_dir.mkdir()
                    (target_dir / ".git").mkdir()
                return subprocess.CompletedProcess(cmd, 0, "", "")

            with mock.patch.object(sync_tools.subprocess, "run", side_effect=fake_subprocess_run):
                with mock.patch.object(sync_tools, "apply_overrides"):
                    sync_tools.sync_tool(
                        "playwright-cli",
                        {
                            "url": "https://example.invalid/repo.git",
                            "setup": "npm install && npx playwright install",
                        },
                        {"enabled": True, "auto_update": True},
                        Path(temp_dir),
                    )

            setup_calls = [call for call in calls if call["cmd"] == "npm install && npx playwright install"]
            self.assertEqual(len(setup_calls), 1)
            self.assertIsNone(setup_calls[0]["timeout"])

    def test_sync_tool_applies_real_override_files_when_repo_is_safe_to_write(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            module_dir = temp_root / "module"
            overrides_dir = module_dir / "overrides" / "playwright-cli"
            overrides_dir.mkdir(parents=True)
            (overrides_dir / "config.txt").write_text("override\n", encoding="utf-8")

            target_dir = temp_root / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()
            (target_dir / "config.txt").write_text("original\n", encoding="utf-8")

            with mock.patch.object(sync_tools, "__file__", str(module_dir / "sync_tools.py")):
                sync_tools.sync_tool(
                    "playwright-cli",
                    {"url": "https://example.invalid/repo.git"},
                    {"enabled": True, "auto_update": False},
                    temp_root,
                )

            self.assertEqual((target_dir / "config.txt").read_text(encoding="utf-8"), "override\n")
