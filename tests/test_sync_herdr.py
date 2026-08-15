import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-herdr" / "sync_herdr.py"
SPEC = importlib.util.spec_from_file_location("sync_herdr", MODULE_PATH)
sync_herdr = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(sync_herdr)


class SyncHerdrTests(unittest.TestCase):
    def test_parse_integration_status_preserves_current_version(self):
        statuses = sync_herdr.parse_integration_status(
            "claude: current (v7)\n"
            "codex: not installed (/tmp/.codex/herdr-agent-state.sh)\n"
        )

        self.assertEqual(statuses["claude"], "current (v7)")
        self.assertTrue(sync_herdr.integration_is_current(statuses["claude"]))
        self.assertFalse(sync_herdr.integration_is_current(statuses["codex"]))

    def test_sync_integrations_is_noop_when_both_are_current(self):
        definitions = {
            "claude-code": {"target": "claude", "agent_command": "claude"},
            "codex": {"target": "codex", "agent_command": "codex"},
        }
        policy = {
            "claude-code": {"enabled": True},
            "codex": {"enabled": True},
        }

        with mock.patch.object(
            sync_herdr,
            "run_herdr_command",
            return_value="claude: current (v7)\ncodex: current (v7)",
        ) as run_command:
            sync_herdr.sync_integrations("/tmp/herdr", definitions, policy, dry_run=False)

        run_command.assert_called_once_with("/tmp/herdr", "integration", "status")

    def test_sync_integrations_installs_and_verifies_missing_target(self):
        definitions = {
            "claude-code": {"target": "claude", "agent_command": "claude"},
            "codex": {"target": "codex", "agent_command": "codex"},
        }
        policy = {
            "claude-code": {"enabled": True},
            "codex": {"enabled": True},
        }
        outputs = [
            "claude: current (v7)\ncodex: not installed (/tmp/codex-hook)",
            "",
            "claude: current (v7)\ncodex: current (v7)",
        ]

        with mock.patch.object(sync_herdr, "command_exists", return_value=True):
            with mock.patch.object(sync_herdr, "run_herdr_command", side_effect=outputs) as run_command:
                sync_herdr.sync_integrations("/tmp/herdr", definitions, policy, dry_run=False)

        self.assertEqual(
            run_command.call_args_list,
            [
                mock.call("/tmp/herdr", "integration", "status"),
                mock.call("/tmp/herdr", "integration", "install", "codex"),
                mock.call("/tmp/herdr", "integration", "status"),
            ],
        )

    def test_main_dry_run_does_not_install_missing_binary(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            manager_path = temp_root / "manager.json"
            definition_path = temp_root / "herdr.json"
            manager_path.write_text(
                json.dumps({"herdr": {"enabled": True, "auto_install": True}}),
                encoding="utf-8",
            )
            definition_path.write_text(
                json.dumps(
                    {
                        "binary": {
                            "command": "herdr",
                            "fallback_paths": [],
                            "install": {"posix": "install-herdr"},
                        },
                        "integrations": {},
                    }
                ),
                encoding="utf-8",
            )

            with mock.patch.object(sync_herdr, "find_herdr_binary", return_value=None):
                with mock.patch.object(sync_herdr, "install_binary") as install_binary:
                    result = sync_herdr.main(
                        [
                            "--config",
                            str(manager_path),
                            "--def-file",
                            str(definition_path),
                            "--dry-run",
                        ]
                    )

        self.assertEqual(result, 0)
        install_binary.assert_not_called()

    def test_main_offline_fails_when_binary_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            manager_path = temp_root / "manager.json"
            definition_path = temp_root / "herdr.json"
            manager_path.write_text(
                json.dumps({"herdr": {"enabled": True, "auto_install": True}}),
                encoding="utf-8",
            )
            definition_path.write_text(
                json.dumps(
                    {
                        "binary": {
                            "command": "herdr",
                            "fallback_paths": [],
                            "install": {"posix": "install-herdr"},
                        },
                        "integrations": {},
                    }
                ),
                encoding="utf-8",
            )

            with mock.patch.object(sync_herdr, "find_herdr_binary", return_value=None):
                with mock.patch.object(sync_herdr, "install_binary") as install_binary:
                    result = sync_herdr.main(
                        [
                            "--config",
                            str(manager_path),
                            "--def-file",
                            str(definition_path),
                            "--offline",
                        ]
                    )

        self.assertEqual(result, 1)
        install_binary.assert_not_called()
