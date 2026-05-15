import json
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
STATUSLINE = ROOT / "scripts" / "claude" / "statusline.sh"
INSTALLER = ROOT / "scripts" / "claude" / "install_statusline.py"


class ClaudeStatuslineScriptTests(unittest.TestCase):
    def _run_statusline(self, payload: dict) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(STATUSLINE)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            check=False,
        )

    def test_statusline_script_is_committed_and_executable(self):
        self.assertTrue(STATUSLINE.is_file(), STATUSLINE)
        mode = stat.S_IMODE(STATUSLINE.stat().st_mode)
        self.assertTrue(mode & stat.S_IXUSR, oct(mode))

    def test_statusline_renders_model_dir_and_branch(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            repo = Path(temp_dir) / "demo-repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "t@e.x"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "t"], cwd=repo, check=True)
            subprocess.run(["git", "checkout", "-q", "-b", "feature/x"], cwd=repo, check=True)
            (repo / "README.md").write_text("seed\n", encoding="utf-8")
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-q", "-m", "seed"], cwd=repo, check=True)

            result = self._run_statusline(
                {"model": {"display_name": "Opus 4.7"}, "workspace": {"current_dir": str(repo)}}
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "Opus 4.7 | demo-repo (feature/x)")

    def test_statusline_omits_branch_outside_git(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            non_git = Path(temp_dir) / "loose-dir"
            non_git.mkdir()

            result = self._run_statusline(
                {"model": {"id": "claude-fast"}, "workspace": {"current_dir": str(non_git)}}
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "claude-fast | loose-dir")


class ClaudeStatuslineInstallerTests(unittest.TestCase):
    def _run_installer(self, target: Path, *extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(INSTALLER), "--target", str(target), *extra],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_installer_writes_executable_copy(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target = Path(temp_dir) / "nested" / "statusline.sh"

            result = self._run_installer(target)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(target.is_file())
            self.assertEqual(target.read_bytes(), STATUSLINE.read_bytes())
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o755)

    def test_installer_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target = Path(temp_dir) / "statusline.sh"

            first = self._run_installer(target)
            second = self._run_installer(target)

            self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
            self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
            self.assertIn("已写入", first.stdout + first.stderr)
            self.assertIn("已是最新", second.stdout + second.stderr)

    def test_installer_dry_run_does_not_write(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target = Path(temp_dir) / "statusline.sh"

            result = self._run_installer(target, "--dry-run")

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(target.exists())
            self.assertIn("预览", result.stdout + result.stderr)

    def test_makefile_and_manage_expose_install_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("claude\\:statusline\\:install", makefile)
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        self.assertIn("claude:statusline:install", manage)
        self.assertIn("install_statusline.py", manage)


class ClaudeSettingsMergeTests(unittest.TestCase):
    """Contract: --apply-settings idempotently patches .statusLine into settings.json."""

    def _run(self, *args, env_home: Path = None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        if env_home is not None:
            env["HOME"] = str(env_home)
        return subprocess.run(
            ["python3", str(INSTALLER), *args],
            capture_output=True, text=True, check=False, env=env,
        )

    def test_creates_settings_when_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings = home / ".claude" / "settings.json"
            target = home / ".claude" / "statusline.sh"
            cmd_path = "/opt/ccstatusline"

            result = self._run(
                "--target", str(target),
                "--apply-settings",
                "--statusline-command", cmd_path,
                env_home=home,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(settings.is_file())
            data = json.loads(settings.read_text())
            self.assertEqual(data["statusLine"]["type"], "command")
            self.assertEqual(data["statusLine"]["command"], cmd_path)

    def test_preserves_other_settings_keys(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings_dir = home / ".claude"
            settings_dir.mkdir()
            settings = settings_dir / "settings.json"
            existing = {
                "model": "claude-opus-4-7",
                "env": {"SECRET_TOKEN": "do-not-touch"},
                "statusLine": {"type": "command", "command": "/old/path", "padding": 0},
            }
            settings.write_text(json.dumps(existing, indent=2), encoding="utf-8")

            result = self._run(
                "--target", str(home / ".claude" / "statusline.sh"),
                "--apply-settings",
                "--statusline-command", "/opt/new",
                env_home=home,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

            data = json.loads(settings.read_text())
            self.assertEqual(data["model"], "claude-opus-4-7")
            self.assertEqual(data["env"]["SECRET_TOKEN"], "do-not-touch")
            self.assertEqual(data["statusLine"]["command"], "/opt/new")
            self.assertEqual(data["statusLine"]["type"], "command")

    def test_idempotent_when_already_correct(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings_dir = home / ".claude"
            settings_dir.mkdir()
            settings = settings_dir / "settings.json"
            settings.write_text(
                json.dumps({"statusLine": {"type": "command", "command": "/opt/x", "padding": 0}}),
                encoding="utf-8",
            )

            first = self._run(
                "--target", str(home / ".claude" / "statusline.sh"),
                "--apply-settings",
                "--statusline-command", "/opt/x",
                env_home=home,
            )
            self.assertEqual(first.returncode, 0)
            self.assertIn("已是最新", first.stdout + first.stderr)

    def test_dry_run_does_not_write_settings(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings = home / ".claude" / "settings.json"
            settings.parent.mkdir()
            settings.write_text(json.dumps({"model": "x"}), encoding="utf-8")
            original = settings.read_text()

            result = self._run(
                "--target", str(home / ".claude" / "statusline.sh"),
                "--apply-settings",
                "--statusline-command", "/opt/y",
                "--dry-run",
                env_home=home,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(settings.read_text(), original)
            self.assertIn("预览", result.stdout + result.stderr)

    def test_invalid_existing_settings_aborts(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings = home / ".claude" / "settings.json"
            settings.parent.mkdir()
            settings.write_text("{ not valid json", encoding="utf-8")

            result = self._run(
                "--target", str(home / ".claude" / "statusline.sh"),
                "--apply-settings",
                "--statusline-command", "/opt/y",
                env_home=home,
            )
            self.assertNotEqual(result.returncode, 0)


class ClaudeThemeMergeTests(unittest.TestCase):
    """`--theme` patches `.theme` alongside `.statusLine`, preserving other keys."""

    def _run(self, *args, env_home: Path = None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        if env_home is not None:
            env["HOME"] = str(env_home)
        return subprocess.run(
            ["python3", str(INSTALLER), *args],
            capture_output=True, text=True, check=False, env=env,
        )

    def test_theme_sets_value_alongside_statusline(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings = home / ".claude" / "settings.json"
            settings.parent.mkdir()
            settings.write_text(
                json.dumps({"env": {"TOKEN": "keep-me"}, "theme": "old"}),
                encoding="utf-8",
            )

            result = self._run(
                "--target", str(home / ".claude" / "statusline.sh"),
                "--apply-settings",
                "--statusline-command", "/opt/ccsl",
                "--theme", "dark-daltonized",
                env_home=home,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            data = json.loads(settings.read_text())
            self.assertEqual(data["theme"], "dark-daltonized")
            self.assertEqual(data["statusLine"]["command"], "/opt/ccsl")
            self.assertEqual(data["env"]["TOKEN"], "keep-me")

    def test_theme_idempotent_when_already_set(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir)
            settings = home / ".claude" / "settings.json"
            settings.parent.mkdir()
            settings.write_text(
                json.dumps({
                    "statusLine": {"type": "command", "command": "/x", "padding": 0},
                    "theme": "dark-daltonized",
                }),
                encoding="utf-8",
            )
            first = self._run(
                "--target", str(home / ".claude" / "statusline.sh"),
                "--apply-settings",
                "--statusline-command", "/x",
                "--theme", "dark-daltonized",
                env_home=home,
            )
            self.assertEqual(first.returncode, 0)
            self.assertIn("已是最新", first.stdout + first.stderr)


class ClaudeStatuslineWireTargetTests(unittest.TestCase):
    """The `claude:statusline:wire` orchestration target only ever points at
    ~/.claude/settings.json."""

    def test_makefile_exposes_wire_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("claude\\:statusline\\:wire", makefile)
        self.assertIn("claude-statusline-wire", makefile)

    def test_manage_sh_wires_only_claude_settings(self):
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        self.assertIn("claude:statusline:wire", manage)
        # Locate the wire branch and ensure it does NOT pass --settings-path
        # pointed elsewhere. The default in install_statusline.py is
        # ~/.claude/settings.json, so omitting --settings-path enforces scope.
        wire_idx = manage.index("claude:statusline:wire)")
        # Look at the next 600 chars after the case match
        wire_block = manage[wire_idx:wire_idx + 800]
        self.assertIn("install_statusline.py", wire_block)
        self.assertIn("--apply-settings", wire_block)
        self.assertNotIn(".codex", wire_block)
        self.assertNotIn(".gemini", wire_block)
        # Must not pass --settings-path (which could redirect elsewhere)
        self.assertNotIn("--settings-path", wire_block)


if __name__ == "__main__":
    unittest.main()
