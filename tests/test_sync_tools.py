import json
import os
import subprocess
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.agent_tool_import import sync_tools

ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-tool" / "sync_tools.py"


class SyncToolsTests(unittest.TestCase):
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
        self.assertIn("-f, --force-refresh", combined)

    def test_committed_auggie_policy_tracks_latest_channel(self):
        manager = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))
        tool_defs = json.loads((ROOT / "config" / "tools.json").read_text(encoding="utf-8"))["tools"]

        self.assertTrue(manager["tools"]["auggie"]["enabled"])
        self.assertTrue(manager["tools"]["auggie"]["auto_update"])
        self.assertEqual(tool_defs["auggie"]["install"], "npm install -g @augmentcode/auggie@latest")
        self.assertEqual(tool_defs["auggie"]["upgrade"], "npm install -g @augmentcode/auggie@latest")

    def test_committed_browser_tool_policy_tracks_latest_channels(self):
        manager = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))
        tool_defs = json.loads((ROOT / "config" / "tools.json").read_text(encoding="utf-8"))["tools"]

        self.assertTrue(manager["tools"]["playwright-cli"]["enabled"])
        self.assertTrue(manager["tools"]["agent-browser"]["enabled"])
        self.assertEqual(tool_defs["playwright-cli"]["install"], "npm install -g @playwright/cli@latest")
        self.assertEqual(tool_defs["playwright-cli"]["upgrade"], "npm install -g @playwright/cli@latest")
        self.assertEqual(tool_defs["playwright-cli"]["setup"], "playwright-cli install-browser")
        self.assertEqual(tool_defs["playwright-cli"]["ready_check"], "bin:playwright-cli")
        self.assertIn("~/Library/Caches/ms-playwright/chromium-*", tool_defs["playwright-cli"]["runtime_ready_globs_any"])
        self.assertEqual(tool_defs["agent-browser"]["install"], "npm install -g agent-browser@latest")
        self.assertEqual(tool_defs["agent-browser"]["upgrade"], "npm install -g agent-browser@latest")
        self.assertEqual(tool_defs["agent-browser"]["setup"], "agent-browser install")
        self.assertEqual(tool_defs["agent-browser"]["ready_check"], "bin:agent-browser")
        self.assertIn("app:/Applications/Google Chrome.app", tool_defs["agent-browser"]["runtime_ready_checks_any"])
        self.assertIn("~/Library/Caches/ms-playwright/chromium-*", tool_defs["agent-browser"]["runtime_ready_globs_any"])

    def test_install_based_tool_with_setup_requires_setup_marker(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            ready = sync_tools.detect_tool_readiness(
                "agent-browser",
                {
                    "install": "npm install -g agent-browser@latest",
                    "setup": "agent-browser install",
                    "ready_check": "bin:agent-browser",
                },
                temp_dir,
            )

        self.assertFalse(ready)

    def test_install_based_tool_runs_setup_and_marks_ready(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "agent-browser"
            state = {"binary_ready": False, "runtime_ready": False}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == "npm install -g agent-browser@latest":
                    state["binary_ready"] = True
                    return True, "installed"
                if cmd == "agent-browser install":
                    state["runtime_ready"] = True
                    return True, "setup ok"
                return True, ""

            def fake_check_ready(name, ready_check):
                return state["binary_ready"]

            def fake_detect_tool_readiness(name, definition, repo_root, inventory=None):
                return state["binary_ready"] and state["runtime_ready"] and (
                    target_dir / sync_tools.TOOL_READY_MARKER
                ).exists()

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "_check_ready", side_effect=fake_check_ready):
                    with mock.patch.object(sync_tools, "detect_tool_readiness", side_effect=fake_detect_tool_readiness):
                        ok = sync_tools.sync_tool(
                            "agent-browser",
                            {
                                "install": "npm install -g agent-browser@latest",
                                "upgrade": "npm install -g agent-browser@latest",
                                "setup": "agent-browser install",
                                "ready_check": "bin:agent-browser",
                            },
                            {"enabled": True, "auto_update": False},
                            Path(temp_dir),
                        )

            self.assertTrue(ok)
            self.assertTrue((target_dir / sync_tools.TOOL_READY_MARKER).exists())
            self.assertFalse((target_dir / sync_tools.TOOL_BROKEN_MARKER).exists())

    def test_install_based_tool_skips_setup_when_runtime_is_already_available_after_upgrade(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            state = {"binary_ready": True}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == "agent-browser install":
                    raise AssertionError("setup should not run when runtime is already available")
                return True, "ok"

            def fake_check_ready(name, ready_check):
                return state["binary_ready"]

            def fake_detect_tool_readiness(name, definition, repo_root, inventory=None):
                return True

            with mock.patch.object(sync_tools, "probe_package_update_state", return_value="stale"):
                with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd) as run_cmd:
                    with mock.patch.object(sync_tools, "_check_ready", side_effect=fake_check_ready):
                        with mock.patch.object(sync_tools, "detect_tool_readiness", side_effect=fake_detect_tool_readiness):
                            ok = sync_tools.sync_tool(
                                "agent-browser",
                                {
                                    "install": "npm install -g agent-browser@latest",
                                    "upgrade": "npm install -g agent-browser@latest",
                                    "setup": "agent-browser install",
                                    "ready_check": "bin:agent-browser",
                                    "runtime_ready_checks_any": ["app:/Applications/Google Chrome.app"],
                                },
                                {"enabled": True, "auto_update": True},
                                Path(temp_dir),
                            )

            self.assertTrue(ok)
            self.assertEqual(run_cmd.call_args_list, [mock.call("npm install -g agent-browser@latest")])

    def test_probe_package_update_state_reports_latest_for_matching_npm_versions(self):
        def fake_run_cmd(cmd, cwd=None, timeout=300):
            if cmd == ["npm", "list", "-g", "@augmentcode/auggie", "--depth=0", "--json"]:
                return True, json.dumps({"dependencies": {"@augmentcode/auggie": {"version": "0.24.0"}}})
            if cmd == ["npm", "view", "@augmentcode/auggie", "version"]:
                return True, "0.24.0\n"
            raise AssertionError(f"unexpected command: {cmd}")

        with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
            state = sync_tools.probe_package_update_state(
                "auggie",
                {
                    "install": "npm install -g @augmentcode/auggie@latest",
                    "upgrade": "npm install -g @augmentcode/auggie@latest",
                },
            )

        self.assertEqual(state, "latest")

    def test_install_based_tool_skips_upgrade_when_package_probe_reports_latest(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            with mock.patch.object(sync_tools, "probe_package_update_state", return_value="latest"):
                with mock.patch.object(sync_tools, "_check_ready", return_value=True):
                    with mock.patch.object(sync_tools, "detect_tool_readiness", return_value=True):
                        with mock.patch.object(sync_tools, "run_cmd") as run_cmd:
                            ok = sync_tools.sync_tool(
                                "auggie",
                                {
                                    "install": "npm install -g @augmentcode/auggie@latest",
                                    "upgrade": "npm install -g @augmentcode/auggie@latest",
                                    "ready_check": "bin:auggie",
                                },
                                {"enabled": True, "auto_update": True},
                                Path(temp_dir),
                            )

        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_count, 0)

    def test_install_based_tool_probe_failure_skips_upgrade_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            with mock.patch.object(sync_tools, "probe_package_update_state", return_value="unknown"):
                with mock.patch.object(sync_tools, "_check_ready", return_value=True):
                    with mock.patch.object(sync_tools, "detect_tool_readiness", return_value=True):
                        with mock.patch.object(sync_tools, "run_cmd") as run_cmd:
                            ok = sync_tools.sync_tool(
                                "auggie",
                                {
                                    "install": "npm install -g @augmentcode/auggie@latest",
                                    "upgrade": "npm install -g @augmentcode/auggie@latest",
                                    "ready_check": "bin:auggie",
                                },
                                {"enabled": True, "auto_update": True},
                                Path(temp_dir),
                            )

        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_count, 0)

    def test_install_based_tool_probe_failure_force_refresh_runs_upgrade(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            with mock.patch.object(sync_tools, "probe_package_update_state", return_value="unknown"):
                with mock.patch.object(sync_tools, "_check_ready", return_value=True):
                    with mock.patch.object(sync_tools, "detect_tool_readiness", return_value=True):
                        with mock.patch.object(sync_tools, "run_cmd", return_value=(True, "upgraded")) as run_cmd:
                            ok = sync_tools.sync_tool(
                                "auggie",
                                {
                                    "install": "npm install -g @augmentcode/auggie@latest",
                                    "upgrade": "npm install -g @augmentcode/auggie@latest",
                                    "ready_check": "bin:auggie",
                                },
                                {"enabled": True, "auto_update": True},
                                Path(temp_dir),
                                force_refresh=True,
                            )

        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_args_list, [mock.call("npm install -g @augmentcode/auggie@latest")])

    def test_install_based_tool_skips_setup_when_runtime_glob_is_already_available_after_upgrade(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            state = {"binary_ready": True}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == "playwright-cli install-browser":
                    raise AssertionError("setup should not run when runtime globs already match")
                return True, "ok"

            def fake_check_ready(name, ready_check):
                return state["binary_ready"]

            with mock.patch.object(sync_tools, "probe_package_update_state", return_value="stale"):
                with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd) as run_cmd:
                    with mock.patch.object(sync_tools, "_check_ready", side_effect=fake_check_ready):
                        with mock.patch.object(sync_tools, "detect_tool_readiness", return_value=True):
                            ok = sync_tools.sync_tool(
                                "playwright-cli",
                                {
                                    "install": "npm install -g @playwright/cli@latest",
                                    "upgrade": "npm install -g @playwright/cli@latest",
                                    "setup": "playwright-cli install-browser",
                                    "ready_check": "bin:playwright-cli",
                                    "runtime_ready_globs_any": ["~/Library/Caches/ms-playwright/chromium-*"],
                                },
                                {"enabled": True, "auto_update": True},
                                Path(temp_dir),
                            )

        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_args_list, [mock.call("npm install -g @playwright/cli@latest")])

    def test_install_based_tool_setup_failure_marks_broken(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "agent-browser"
            state = {"binary_ready": False}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == "npm install -g agent-browser@latest":
                    state["binary_ready"] = True
                    return True, "installed"
                if cmd == "agent-browser install":
                    return False, "setup failed"
                return True, ""

            def fake_check_ready(name, ready_check):
                return state["binary_ready"]

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "_check_ready", side_effect=fake_check_ready):
                    ok = sync_tools.sync_tool(
                        "agent-browser",
                        {
                            "install": "npm install -g agent-browser@latest",
                            "upgrade": "npm install -g agent-browser@latest",
                            "setup": "agent-browser install",
                            "ready_check": "bin:agent-browser",
                        },
                        {"enabled": True, "auto_update": False},
                        Path(temp_dir),
                    )

            self.assertFalse(ok)
            self.assertTrue((target_dir / sync_tools.TOOL_BROKEN_MARKER).exists())
            self.assertFalse((target_dir / sync_tools.TOOL_READY_MARKER).exists())

    def test_install_based_tool_success_without_runtime_probe_marks_broken(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            state = {"binary_ready": False}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == "npm install -g @playwright/cli@latest":
                    state["binary_ready"] = True
                    return True, "installed"
                if cmd == "playwright-cli install-browser":
                    return True, "setup ok"
                return True, ""

            def fake_check_ready(name, ready_check):
                return state["binary_ready"]

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "_check_ready", side_effect=fake_check_ready):
                    with mock.patch.object(sync_tools, "detect_tool_readiness", return_value=False):
                        ok = sync_tools.sync_tool(
                            "playwright-cli",
                            {
                                "install": "npm install -g @playwright/cli@latest",
                                "upgrade": "npm install -g @playwright/cli@latest",
                                "setup": "playwright-cli install-browser",
                                "ready_check": "bin:playwright-cli",
                                "runtime_ready_globs_any": ["~/Library/Caches/ms-playwright/chromium-*"],
                            },
                            {"enabled": True, "auto_update": False},
                            Path(temp_dir),
                        )

            self.assertFalse(ok)
            self.assertTrue((target_dir / sync_tools.TOOL_BROKEN_MARKER).exists())
            self.assertFalse((target_dir / sync_tools.TOOL_READY_MARKER).exists())

    def test_invalid_policy_config_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text("{ invalid json\n", encoding="utf-8")
            def_path.write_text('{"tools": {}}', encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(Path(sync_tools.__file__).resolve()),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                ],
                cwd=temp_root,
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
            config_path.write_text('{"tools": {}}', encoding="utf-8")
            def_path.write_text("{ invalid json\n", encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(Path(sync_tools.__file__).resolve()),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("defs.json", combined)
            self.assertIn("无效", combined)

    def test_non_object_definition_config_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"tools": {}}', encoding="utf-8")
            def_path.write_text("[]\n", encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(Path(sync_tools.__file__).resolve()),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("defs.json", combined)
            self.assertIn("顶层必须是对象", combined)

    def test_default_base_dir_does_not_prune_current_working_directory(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            repo_root = temp_root / "repo"
            shutil.copytree(ROOT, repo_root, ignore=shutil.ignore_patterns('repositories'))
            module_path = repo_root / "agent-tool" / "sync_tools.py"
            working_dir = temp_root / "cwd"
            victim_dir = working_dir / "agent-lifecycle"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            working_dir.mkdir()
            victim_dir.mkdir()
            config_path.write_text('{"tools": {}}', encoding="utf-8")
            def_path.write_text('{"tools": {}}', encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(module_path),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--yes",
                ],
                cwd=working_dir,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(victim_dir.exists(), result.stdout + result.stderr)

    def test_missing_policy_tool_does_not_prune_unmanaged_directories(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            tools_dir = temp_root / "tools"
            foreign_dir = tools_dir / "foreign-dir"
            tools_dir.mkdir()
            foreign_dir.mkdir()
            config_path.write_text('{"tools": {}}', encoding="utf-8")
            def_path.write_text(
                '{"tools": {"playwright-cli": {"url": "https://example.invalid/repo.git"}}}',
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(Path(sync_tools.__file__).resolve()),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(tools_dir),
                    "--yes",
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(foreign_dir.exists(), combined)
            self.assertIn("跳过非托管工具目录: foreign-dir", combined)

    def test_prune_orphan_tools_only_removes_manager_owned_directories(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            base_dir = Path(temp_dir)
            owned_dir = base_dir / "owned-orphan"
            foreign_dir = base_dir / "foreign-orphan"
            owned_dir.mkdir()
            foreign_dir.mkdir()
            sync_tools.write_owner_marker(owned_dir, "owned-orphan")

            sync_tools.prune_orphan_tools(base_dir, managed_names=set(), dry_run=False, force_yes=True)

            self.assertFalse(owned_dir.exists())
            self.assertTrue(foreign_dir.exists())

    def test_log_level_environment_enables_debug_output(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"tools": {}}', encoding="utf-8")
            def_path.write_text('{"tools": {}}', encoding="utf-8")
            env = os.environ.copy()
            env["LOG_LEVEL"] = "DEBUG"

            result = subprocess.run(
                [
                    "python3",
                    str(Path(sync_tools.__file__).resolve()),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "tools"),
                ],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("[DEBUG] Logging initialized at level DEBUG", combined)

    def test_missing_policy_tool_is_ignored(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"tools": {}}', encoding="utf-8")
            def_path.write_text(
                '{"tools": {"playwright-cli": {"url": "https://example.invalid/repo.git"}}}',
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(Path(sync_tools.__file__).resolve()),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "tools"),
                    "--dry-run",
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("未声明策略，跳过", combined)
            self.assertNotIn("正在安装 playwright-cli", combined)

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

    def test_offline_mode_does_not_clone_missing_repo_tool(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            with (
                mock.patch.object(sync_tools, "run_cmd") as run_cmd,
                self.assertLogs(level="ERROR") as logs,
            ):
                ok = sync_tools.sync_tool(
                    "playwright-cli",
                    {"url": "https://example.invalid/repo.git"},
                    {"enabled": True, "auto_update": False},
                    Path(temp_dir),
                    offline=True,
                )

        self.assertFalse(ok)
        run_cmd.assert_not_called()
        self.assertTrue(any("离线模式且本地仓库缺失" in line for line in logs.output), logs.output)

    def test_dry_run_offline_mode_reports_missing_repo_tool_as_blocker(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertLogs(level="ERROR") as logs:
                ok = sync_tools.sync_tool(
                    "playwright-cli",
                    {"url": "https://example.invalid/repo.git"},
                    {"enabled": True, "auto_update": False},
                    Path(temp_dir),
                    dry_run=True,
                    offline=True,
                )

        self.assertFalse(ok)
        self.assertTrue(any("离线模式且本地仓库缺失" in line for line in logs.output), logs.output)

    def test_offline_mode_does_not_install_missing_install_based_tool(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            with (
                mock.patch.object(sync_tools, "run_cmd") as run_cmd,
                mock.patch.object(sync_tools, "_check_ready", return_value=False),
                mock.patch.object(sync_tools, "detect_tool_readiness", return_value=False),
                self.assertLogs(level="ERROR") as logs,
            ):
                ok = sync_tools.sync_tool(
                    "agent-browser",
                    {
                        "install": "npm install -g agent-browser@latest",
                        "setup": "agent-browser install",
                        "ready_check": "bin:agent-browser",
                    },
                    {"enabled": True, "auto_update": False},
                    Path(temp_dir),
                    offline=True,
                )

        self.assertFalse(ok)
        run_cmd.assert_not_called()
        self.assertTrue(any("离线模式且工具未安装" in line for line in logs.output), logs.output)

    def test_successful_setup_marks_repo_ready(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if isinstance(cmd, list) and cmd[:2] == ["git", "clone"]:
                    target_dir.mkdir()
                    (target_dir / ".git").mkdir()
                    (target_dir / "node_modules").mkdir()
                    return True, "cloned"
                if cmd == "npm install && npx playwright install":
                    return True, "setup ok"
                return True, ""

            with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                with mock.patch.object(sync_tools, "apply_overrides"):
                    ok = sync_tools.sync_tool(
                        "playwright-cli",
                        {
                            "url": "https://example.invalid/repo.git",
                            "setup": "npm install && npx playwright install",
                            "ready_paths": ["node_modules"],
                        },
                        {"enabled": True, "auto_update": True},
                        Path(temp_dir),
                    )

            self.assertTrue(ok)
            self.assertTrue((target_dir / sync_tools.TOOL_READY_MARKER).exists())
            self.assertFalse((target_dir / sync_tools.TOOL_BROKEN_MARKER).exists())

    def test_failed_setup_marks_repo_broken_and_clears_ready_marker(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()
            (target_dir / "node_modules").mkdir()
            (target_dir / sync_tools.TOOL_READY_MARKER).write_text("ready\n", encoding="utf-8")
            sync_tools.git_cache._cache = {}

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == ["git", "status", "--porcelain"]:
                    return True, ""
                if cmd == ["git", "pull", "--ff-only"]:
                    return True, "pulled"
                if cmd == "npm install && npx playwright install":
                    return False, "setup failed"
                return True, ""

            with mock.patch.object(sync_tools, "probe_git_remote_state", return_value={"status": "stale"}):
                with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd):
                    with mock.patch.object(sync_tools, "apply_overrides"):
                        ok = sync_tools.sync_tool(
                            "playwright-cli",
                            {
                                "url": "https://example.invalid/repo.git",
                                "setup": "npm install && npx playwright install",
                                "ready_paths": ["node_modules"],
                            },
                            {"enabled": True, "auto_update": True},
                            Path(temp_dir),
                        )

            self.assertFalse(ok)
            self.assertFalse((target_dir / sync_tools.TOOL_READY_MARKER).exists())
            self.assertTrue((target_dir / sync_tools.TOOL_BROKEN_MARKER).exists())

    def test_auto_update_false_preserves_broken_repo_state(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()
            (target_dir / sync_tools.TOOL_BROKEN_MARKER).write_text("broken\n", encoding="utf-8")

            with mock.patch.object(sync_tools, "apply_overrides"):
                ok = sync_tools.sync_tool(
                    "playwright-cli",
                    {
                        "url": "https://example.invalid/repo.git",
                        "setup": "npm install && npx playwright install",
                        "ready_paths": ["node_modules"],
                    },
                    {"enabled": True, "auto_update": False},
                    Path(temp_dir),
                )

            self.assertFalse(ok)
            self.assertTrue((target_dir / sync_tools.TOOL_BROKEN_MARKER).exists())
            self.assertFalse((target_dir / sync_tools.TOOL_READY_MARKER).exists())

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
                if cmd == ["git", "pull", "--ff-only"]:
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

    def test_repo_backed_tool_probe_failure_skips_pull_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == ["git", "status", "--porcelain"]:
                    return True, ""
                raise AssertionError(f"unexpected command: {cmd}")

            with mock.patch.object(sync_tools, "probe_git_remote_state", return_value={"status": "unknown"}):
                with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd) as run_cmd:
                    with mock.patch.object(sync_tools, "apply_overrides") as apply_overrides:
                        ok = sync_tools.sync_tool(
                            "playwright-cli",
                            {"url": "https://example.invalid/repo.git"},
                            {"enabled": True, "auto_update": True},
                            Path(temp_dir),
                        )

        self.assertTrue(ok)
        self.assertIn(mock.call(["git", "status", "--porcelain"], cwd=target_dir, timeout=5), run_cmd.call_args_list)
        self.assertEqual(apply_overrides.call_count, 1)

    def test_repo_backed_tool_probe_failure_force_refresh_runs_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "playwright-cli"
            target_dir.mkdir()
            (target_dir / ".git").mkdir()

            def fake_run_cmd(cmd, cwd=None, timeout=300):
                if cmd == ["git", "status", "--porcelain"]:
                    return True, ""
                if cmd == ["git", "pull", "--ff-only"]:
                    return True, "updated"
                raise AssertionError(f"unexpected command: {cmd}")

            with mock.patch.object(sync_tools, "probe_git_remote_state", return_value={"status": "unknown"}):
                with mock.patch.object(sync_tools, "run_cmd", side_effect=fake_run_cmd) as run_cmd:
                    with mock.patch.object(sync_tools, "apply_overrides") as apply_overrides:
                        ok = sync_tools.sync_tool(
                            "playwright-cli",
                            {"url": "https://example.invalid/repo.git"},
                            {"enabled": True, "auto_update": True},
                            Path(temp_dir),
                            force_refresh=True,
                        )

        self.assertTrue(ok)
        self.assertIn(mock.call(["git", "status", "--porcelain"], cwd=target_dir, timeout=5), run_cmd.call_args_list)
        self.assertIn(mock.call(["git", "pull", "--ff-only"], cwd=target_dir), run_cmd.call_args_list)
        self.assertEqual(apply_overrides.call_count, 1)

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
