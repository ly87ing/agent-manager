import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class CodexHooksSyncTests(unittest.TestCase):
    def _copy_repo(self, temp_dir: str) -> Path:
        temp_root = Path(temp_dir) / "repo"
        shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", ".git", ".worktrees"))
        return temp_root

    def _run_sync(self, temp_root: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(temp_root / "agent-rule" / "scripts" / "sync-codex-hooks.py"), *args],
            cwd=temp_root / "agent-rule",
            capture_output=True,
            text=True,
            check=False,
        )

    def _read_hooks(self, temp_root: Path) -> dict:
        return json.loads((temp_root / ".codex" / "hooks.json").read_text(encoding="utf-8"))

    def test_sync_writes_project_codex_hooks_from_managed_definition(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_sync(temp_root)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            hooks = self._read_hooks(temp_root)
            self.assertIn("hooks", hooks)
            self.assertIn("PreToolUse", hooks["hooks"])
            self.assertIn("Stop", hooks["hooks"])
            self.assertTrue(
                any(
                    any("graphify-context-hint.sh" in hook.get("command", "") for hook in entry.get("hooks", []))
                    for entry in hooks["hooks"]["PreToolUse"]
                ),
                hooks,
            )
            self.assertTrue(
                any(
                    any(
                        "check-rules-build.sh" in hook.get("command", "")
                        and "2>/dev/null || pwd" in hook.get("command", "")
                        for hook in entry.get("hooks", [])
                    )
                    for entry in hooks["hooks"]["Stop"]
                ),
                hooks,
            )

    def test_stop_hook_fails_open_outside_git_repo(self):
        hooks = json.loads((ROOT / ".codex" / "hooks.json").read_text(encoding="utf-8"))
        command = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]

        with tempfile.TemporaryDirectory() as temp_dir:
            result = subprocess.run(
                ["bash", "-lc", command],
                cwd=temp_dir,
                input=json.dumps({"hook_event_name": "Stop"}),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_sync_dry_run_does_not_write_project_codex_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            hooks_path = temp_root / ".codex" / "hooks.json"
            before = hooks_path.read_text(encoding="utf-8")

            result = self._run_sync(temp_root, "--dry-run")

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(hooks_path.read_text(encoding="utf-8"), before, result.stdout + result.stderr)

    def test_committed_project_codex_hooks_match_fresh_sync(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_sync(temp_root)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            committed = json.loads((ROOT / ".codex" / "hooks.json").read_text(encoding="utf-8"))
            self.assertEqual(committed, self._read_hooks(temp_root))

    def test_sync_resolves_hooks_path_relative_to_custom_config(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            default_hooks = temp_root / ".codex" / "hooks.json"
            default_hooks.unlink()
            custom_dir = temp_root / "custom-codex-hooks"
            custom_dir.mkdir()
            custom_config = custom_dir / "codex-hooks.json"
            custom_hooks = custom_dir / "nested" / "hooks.json"
            custom_definition = json.loads((temp_root / "config" / "codex-hooks.json").read_text(encoding="utf-8"))
            custom_definition["hooks_path"] = "nested/hooks.json"
            custom_config.write_text(json.dumps(custom_definition, indent=2) + "\n", encoding="utf-8")

            result = self._run_sync(temp_root, "--config", str(custom_config))

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(custom_hooks.exists(), result.stdout + result.stderr)
            self.assertFalse(default_hooks.exists(), result.stdout + result.stderr)

    def test_manage_command_syncs_project_codex_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "codex:hooks:sync"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_root / ".codex" / "hooks.json").exists(), result.stdout + result.stderr)

    def test_make_target_syncs_project_codex_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = subprocess.run(
                ["make", "codex-hooks-sync"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_root / ".codex" / "hooks.json").exists(), result.stdout + result.stderr)

    def test_makefile_exposes_codex_hooks_sync_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("codex-hooks-sync", makefile)
        self.assertIn("codex\\:hooks\\:sync", makefile)
        self.assertIn("\ncodex-hooks-sync:\n\t@./manage.sh codex:hooks:sync\n", makefile)
