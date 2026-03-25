import builtins
import io
import json
import os
import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path
from unittest import mock

from src import interactive


@contextmanager
def working_directory(path):
    previous = Path.cwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(previous)


class AskYesNoTests(unittest.TestCase):
    def test_empty_input_preserves_true_default(self):
        with mock.patch.object(builtins, "input", return_value=""):
            self.assertTrue(interactive.ask_yes_no("启用?", True))

    def test_empty_input_preserves_false_default(self):
        with mock.patch.object(builtins, "input", return_value=""):
            self.assertFalse(interactive.ask_yes_no("启用?", False))


class InteractiveWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        interactive.SYNC_REQUIRED = False
        self.root = Path(self.temp_dir.name)
        config_dir = self.root / "config"
        config_dir.mkdir()
        with open(config_dir / "manager.json", "w", encoding="utf-8") as f:
            json.dump(
                {
                    "agents": {
                        "claude-code": {
                            "enabled": True,
                            "auto_install": True,
                            "auto_update": True,
                            "preferred_provider": "brew",
                        }
                    },
                    "tools": {
                        "playwright-cli": {
                            "enabled": True,
                            "auto_update": True,
                        }
                    },
                    "skills": {},
                    "mcp": {},
                },
                f,
                indent=2,
                ensure_ascii=False,
            )
        with open(config_dir / "agents.json", "w", encoding="utf-8") as f:
            json.dump(
                {
                    "claude-code": {
                        "display_name": "Claude Code",
                        "preferred_provider": "brew",
                        "providers": {
                            "brew": {"install": "brew install --cask claude-code"},
                            "npm": {"install": "npm install -g @anthropic-ai/claude-code"},
                        },
                    }
                },
                f,
                indent=2,
                ensure_ascii=False,
            )
        with open(config_dir / "manager.local.json", "w", encoding="utf-8") as f:
            json.dump({}, f)

    def run_main_with_inputs(self, answers):
        iterator = iter(answers)
        with working_directory(self.root):
            with mock.patch.object(
                builtins,
                "input",
                side_effect=lambda _prompt="": next(iterator),
            ):
                interactive.main()

    def test_exit_without_save_discards_changes(self):
        # 编辑后选择退出，确认提示选 y（确认放弃）
        self.run_main_with_inputs(
            [
                "2",
                "1",
                "n",
                "n",
                "n",
                "",
                "7",
                "y",  # 确认放弃未保存更改
            ]
        )

        local_path = self.root / "config" / "manager.local.json"
        self.assertEqual(json.loads(local_path.read_text(encoding="utf-8")), {})

    def test_exit_without_changes_needs_no_confirmation(self):
        # 未做任何编辑，直接退出不需要确认
        self.run_main_with_inputs(["7"])

        local_path = self.root / "config" / "manager.local.json"
        self.assertEqual(json.loads(local_path.read_text(encoding="utf-8")), {})

    def test_exit_without_save_can_be_cancelled(self):
        # 编辑后选退出，然后选 n 取消，再选 6 保存
        self.run_main_with_inputs(
            [
                "2",
                "1",
                "n",
                "n",
                "n",
                "",
                "7",
                "n",  # 取消放弃，继续留在菜单
                "6",  # 选择保存
            ]
        )

        local_path = self.root / "config" / "manager.local.json"
        saved = json.loads(local_path.read_text(encoding="utf-8"))
        self.assertIn("agents", saved)

    def test_save_persists_only_local_overrides(self):
        self.run_main_with_inputs(
            [
                "2",
                "1",
                "n",
                "n",
                "n",
                "",
                "6",
            ]
        )

        local_path = self.root / "config" / "manager.local.json"
        self.assertEqual(
            json.loads(local_path.read_text(encoding="utf-8")),
            {
                "agents": {
                    "claude-code": {
                        "enabled": False,
                        "auto_install": False,
                        "auto_update": False,
                        "preferred_provider": "brew",
                    }
                }
            },
        )

    def test_save_message_recommends_update_or_bootstrap(self):
        output = io.StringIO()
        with working_directory(self.root):
            iterator = iter(
                [
                    "2",
                    "1",
                    "n",
                    "n",
                    "n",
                    "",
                    "6",
                ]
            )
            with mock.patch.object(
                builtins,
                "input",
                side_effect=lambda _prompt="": next(iterator),
            ):
                with mock.patch("sys.stdout", output):
                    interactive.main()

        rendered = output.getvalue()
        self.assertIn("./manage.sh update", rendered)
        self.assertIn("./manage.sh bootstrap", rendered)
        self.assertNotIn("请运行 ./manage.sh all 以应用更改", rendered)

    def test_save_policy_uses_atomic_json_write(self):
        local_policy = {"agents": {"claude-code": {"enabled": False}}}

        with working_directory(self.root):
            with mock.patch.object(interactive, "atomic_write_json_file") as atomic_write:
                interactive.save_policy(local_policy)

        atomic_write.assert_called_once_with(
            (self.root / "config" / "manager.local.json").resolve(),
            local_policy,
        )

    def test_tool_edit_only_persists_auto_update(self):
        iterator = iter(["1", "n", "n"])
        local_policy = {}
        with working_directory(self.root):
            with mock.patch.object(
                builtins,
                "input",
                side_effect=lambda _prompt="": next(iterator),
            ):
                interactive.edit_section("tools", "Tool", local_policy)

        self.assertEqual(
            local_policy,
            {
                "tools": {
                    "playwright-cli": {
                        "enabled": False,
                        "auto_update": False,
                    }
                }
            },
        )

    def test_agent_edit_persists_preferred_provider(self):
        iterator = iter(["1", "y", "y", "y", "npm"])
        local_policy = {}
        with working_directory(self.root):
            with mock.patch.object(
                builtins,
                "input",
                side_effect=lambda _prompt="": next(iterator),
            ):
                interactive.edit_section("agents", "Agent", local_policy)

        self.assertEqual(
            local_policy,
            {
                "agents": {
                    "claude-code": {
                        "enabled": True,
                        "auto_install": True,
                        "auto_update": True,
                        "preferred_provider": "npm",
                    }
                }
            },
        )

    def test_main_menu_says_save_changes(self):
        output = io.StringIO()
        with working_directory(self.root):
            with mock.patch.object(builtins, "input", return_value="7"):
                with mock.patch("sys.stdout", output):
                    interactive.main()

        self.assertIn("6. 保存更改", output.getvalue())
        self.assertIn("4. 编辑 Skill 策略", output.getvalue())

    def test_main_menu_mentions_local_override_file(self):
        output = io.StringIO()
        with working_directory(self.root):
            with mock.patch.object(builtins, "input", return_value="7"):
                with mock.patch("sys.stdout", output):
                    interactive.main()

        self.assertIn("manager.local.json", output.getvalue())

    def test_invalid_local_override_fails_loudly(self):
        (self.root / "config" / "manager.local.json").write_text("{ invalid json\n", encoding="utf-8")
        output = io.StringIO()

        with working_directory(self.root):
            with self.assertRaises(SystemExit) as exc:
                with mock.patch("sys.stdout", output):
                    interactive.main()

        self.assertEqual(exc.exception.code, 1)
        rendered = output.getvalue()
        self.assertIn("manager.local.json", rendered)
        self.assertIn("无效", rendered)

    def test_invalid_base_config_fails_loudly(self):
        (self.root / "config" / "manager.json").write_text("{ invalid json\n", encoding="utf-8")
        output = io.StringIO()

        with working_directory(self.root):
            with self.assertRaises(SystemExit) as exc:
                with mock.patch("sys.stdout", output):
                    interactive.main()

        self.assertEqual(exc.exception.code, 1)
        rendered = output.getvalue()
        self.assertIn("manager.json", rendered)
        self.assertIn("无效", rendered)

    def test_non_object_base_config_fails_loudly(self):
        (self.root / "config" / "manager.json").write_text("[]\n", encoding="utf-8")
        output = io.StringIO()

        with working_directory(self.root):
            with self.assertRaises(SystemExit) as exc:
                with mock.patch("sys.stdout", output):
                    interactive.main()

        self.assertEqual(exc.exception.code, 1)
        rendered = output.getvalue()
        self.assertIn("manager.json", rendered)
        self.assertIn("顶层必须是对象", rendered)
