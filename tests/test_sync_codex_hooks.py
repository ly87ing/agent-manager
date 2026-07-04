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
            self.assertEqual(hooks["hooks"], {})

    def test_stop_hook_fails_open_outside_git_repo(self):
        hooks = json.loads((ROOT / ".codex" / "hooks.json").read_text(encoding="utf-8"))
        self.assertEqual(hooks["hooks"], {}, "hooks should be empty when disabled")

    def test_pre_tool_hook_denies_codex_apply_patch_to_generated_rules(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            command = "\n".join(
                [
                    "*** Begin Patch",
                    "*** Update File: agent-rule/rules/dist/codex/AGENTS.md",
                    "@@",
                    "+bad manual edit",
                    "*** End Patch",
                ]
            )

            result = subprocess.run(
                ["bash", str(temp_root / "agent-rule" / "hooks" / "block-dist-edits.sh")],
                cwd=temp_root,
                input=json.dumps({"tool_input": {"command": command}}),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            response = json.loads(result.stdout)
            self.assertEqual(response["hookSpecificOutput"]["hookEventName"], "PreToolUse")
            self.assertEqual(response["hookSpecificOutput"]["permissionDecision"], "deny")
            self.assertIn("generated output", response["hookSpecificOutput"]["permissionDecisionReason"])

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
                [str(temp_root / "manage.sh"), "codex:hooks:apply"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_root / ".codex" / "hooks.json").exists(), result.stdout + result.stderr)

    def test_make_target_applies_project_codex_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = subprocess.run(
                ["make", "codex-hooks-apply"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_root / ".codex" / "hooks.json").exists(), result.stdout + result.stderr)

    def test_makefile_exposes_codex_hooks_apply_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("codex-hooks-apply", makefile)
        self.assertIn("codex\\:hooks\\:apply", makefile)
        self.assertIn("@./manage.sh codex:hooks:apply", makefile)
