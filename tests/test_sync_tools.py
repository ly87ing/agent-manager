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
