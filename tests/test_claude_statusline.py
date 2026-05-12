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


if __name__ == "__main__":
    unittest.main()
