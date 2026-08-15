import json
import os
import re
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
STATUSLINE = ROOT / "scripts" / "claude" / "statusline.sh"
INSTALLER = ROOT / "scripts" / "claude" / "install_statusline.py"

_ANSI = re.compile(r"\x1b\[[0-9;]*m")


def _strip(text: str) -> str:
    """Drop ANSI color codes — the statusline always emits color, so tests
    assert on the plain text it carries."""
    return _ANSI.sub("", text)


class ClaudeStatuslineScriptTests(unittest.TestCase):
    """Contract of the lean pure-bash statusline. It derives everything from
    stdin JSON (+ local git HEAD): no jq, no git fork, no network, no
    transcript IO. Segments are ` · `-joined; missing fields drop their
    segment; it always exits 0."""

    def _run_statusline(
        self, payload: dict, env: dict = None
    ) -> subprocess.CompletedProcess[str]:
        run_env = {**os.environ, **env} if env is not None else None
        return subprocess.run(
            ["bash", str(STATUSLINE)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            check=False,
            env=run_env,
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
                {"model": {"display_name": "Opus 4.7"}, "cwd": str(repo)}
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            out = _strip(result.stdout)
            self.assertIn("Opus 4.7", out)
            # 分支从 .git/HEAD 读出（不 fork git），目录后括号内
            self.assertIn("demo-repo (feature/x)", out)

    def test_statusline_renders_branch_inside_git_worktree(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            repo = Path(temp_dir) / "demo-repo"
            repo.mkdir()
            worktree = Path(temp_dir) / "linked-worktree"
            subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "t@e.x"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "t"], cwd=repo, check=True)
            (repo / "README.md").write_text("seed\n", encoding="utf-8")
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-q", "-m", "seed"], cwd=repo, check=True)
            subprocess.run(
                ["git", "worktree", "add", "-q", "-b", "feature/worktree", str(worktree)],
                cwd=repo,
                check=True,
            )

            result = self._run_statusline(
                {"model": {"display_name": "Opus 4.7"}, "cwd": str(worktree)}
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("linked-worktree (feature/worktree)", _strip(result.stdout))

    def test_statusline_omits_branch_outside_git(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            non_git = Path(temp_dir) / "loose-dir"
            non_git.mkdir()

            result = self._run_statusline(
                {"model": {"display_name": "claude-fast"}, "cwd": str(non_git)}
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(_strip(result.stdout), "claude-fast · loose-dir")

    def test_statusline_includes_effort_and_output_style(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self._run_statusline({
                "model": {"display_name": "Opus 4.7"},
                "effort": {"level": "xhigh"},
                "output_style": {"name": "Explanatory"},
                "cwd": temp_dir,
            })
            self.assertEqual(result.returncode, 0, result.stderr)
            out = _strip(result.stdout)
            self.assertIn("effort:xhigh", out)
            self.assertIn("[Explanatory]", out)

    def test_statusline_omits_default_output_style(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self._run_statusline({
                "model": {"display_name": "Opus 4.7"},
                "output_style": {"name": "default"},
                "cwd": temp_dir,
            })
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("default", _strip(result.stdout))

    def test_statusline_shows_context_percentage(self):
        """ctx% comes from context_window.used_percentage (the one followed by
        remaining_percentage), never confused with rate-limit percentages."""
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self._run_statusline({
                "model": {"display_name": "Opus 4.7"},
                "cwd": temp_dir,
                "context_window": {"used_percentage": 50, "remaining_percentage": 50},
                "rate_limits": {
                    "five_hour": {"used_percentage": 18},
                    "seven_day": {"used_percentage": 2},
                },
            })
            self.assertEqual(result.returncode, 0, result.stderr)
            out = _strip(result.stdout)
            self.assertIn("ctx 50%", out)
            self.assertIn("5h 18%", out)
            self.assertIn("7d 2%", out)
            # 无 resets_at 时只显示百分比，不渲染 ↻ 重置时刻（也不 fork date）
            self.assertNotIn("↻", out)

    def test_statusline_shows_rate_limit_reset_countdown(self):
        """rate_limits.*.resets_at（Unix epoch 秒，已在 stdin 内，无需 API）渲染为
        各用量段后的「还剩多久重置」倒计时：5h 用 Xh Ym、7d 用 Xd Yh。脚本仅 fork
        一次 date +%s 取当前时刻（两窗口共用）；这里用 CC_STATUSLINE_NOW 注入固定
        now 以得到确定结果——now=1781058000 时 5h 差 3h0m、7d 差 5d12h40m。"""
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self._run_statusline(
                {
                    "model": {"display_name": "Opus 4.8"},
                    "cwd": temp_dir,
                    "rate_limits": {
                        "five_hour": {"used_percentage": 51, "resets_at": 1781068800},
                        "seven_day": {"used_percentage": 5, "resets_at": 1781535600},
                    },
                },
                env={"CC_STATUSLINE_NOW": "1781058000"},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            out = _strip(result.stdout)
            self.assertIn("5h 51% ↻3h0m", out)
            self.assertIn("7d 5% ↻5d12h", out)

    def test_statusline_shows_lines_changed_and_cost(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self._run_statusline({
                "model": {"display_name": "Opus 4.7"},
                "cwd": temp_dir,
                "cost": {
                    "total_cost_usd": 0.42,
                    "total_lines_added": 12,
                    "total_lines_removed": 3,
                },
            })
            self.assertEqual(result.returncode, 0, result.stderr)
            out = _strip(result.stdout)
            self.assertIn("+12/-3", out)
            self.assertIn("$0.42", out)

    def test_statusline_empty_stdin_exits_zero(self):
        result = subprocess.run(
            ["bash", str(STATUSLINE)],
            input="",
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_statusline_ignores_transcript_path(self):
        """Zero transcript IO contract: pointing transcript_path at a path
        that does not exist must NOT affect output or cause failure — the
        helper never touches that file (it reads only stdin)."""
        with tempfile.TemporaryDirectory() as temp_dir:
            payload = {
                "model": {"display_name": "Opus 4.7"},
                "cwd": temp_dir,
                "transcript_path": "/dev/null/definitely-does-not-exist",
            }
            result = self._run_statusline(payload)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Opus 4.7", _strip(result.stdout))


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

    def test_makefile_and_manage_expose_statusline_apply_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("claude\\:statusline\\:apply", makefile)
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        self.assertIn("claude:statusline:apply", manage)
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
    """`--theme` patches `.theme` alongside `.statusLine`, preserving other keys.

    Note: the `claude:statusline:wire` manage.sh target intentionally omits
    `--theme` so the user's existing `theme` (e.g. `auto`) is preserved; these
    tests exercise the installer's `--theme` capability directly."""

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
        # Look at the next 800 chars after the case match
        wire_block = manage[wire_idx:wire_idx + 800]
        self.assertIn("install_statusline.py", wire_block)
        self.assertIn("--apply-settings", wire_block)
        self.assertNotIn(".codex", wire_block)
        # Must not pass --settings-path (which could redirect elsewhere)
        self.assertNotIn("--settings-path", wire_block)


if __name__ == "__main__":
    unittest.main()
