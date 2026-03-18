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
        self.run_main_with_inputs(
            [
                "2",
                "1",
                "n",
                "n",
                "n",
                "7",
            ]
        )

        local_path = self.root / "config" / "manager.local.json"
        self.assertEqual(json.loads(local_path.read_text(encoding="utf-8")), {})

    def test_save_persists_only_local_overrides(self):
        self.run_main_with_inputs(
            [
                "2",
                "1",
                "n",
                "n",
                "n",
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

    def test_main_menu_says_save_changes(self):
        output = io.StringIO()
        with working_directory(self.root):
            with mock.patch.object(builtins, "input", return_value="7"):
                with mock.patch("sys.stdout", output):
                    interactive.main()

        self.assertIn("6. 保存更改", output.getvalue())
        self.assertIn("4. 编辑 Skill 策略", output.getvalue())
