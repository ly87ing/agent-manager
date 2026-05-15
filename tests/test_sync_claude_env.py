import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SyncClaudeEnvTests(unittest.TestCase):
    def _copy_repo(self, temp_dir: str) -> Path:
        temp_root = Path(temp_dir) / "repo"
        shutil.copytree(
            ROOT,
            temp_root,
            ignore=shutil.ignore_patterns("repositories", ".git", ".worktrees"),
        )
        return temp_root

    def _run_sync(self, temp_root: Path, settings_path: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "python3",
                str(temp_root / "scripts" / "claude" / "sync_env.py"),
                "--settings-path",
                str(settings_path),
                *args,
            ],
            cwd=temp_root,
            capture_output=True,
            text=True,
            check=False,
        )

    def _write_secrets(self, temp_root: Path, contents: str) -> Path:
        secrets_path = temp_root / "secrets" / "claude.env"
        secrets_path.parent.mkdir(parents=True, exist_ok=True)
        secrets_path.write_text(contents, encoding="utf-8")
        return secrets_path

    def test_committed_config_declares_anthropic_credentials_with_placeholders(self):
        config = json.loads((ROOT / "config" / "claude-env.json").read_text(encoding="utf-8"))

        self.assertEqual(config["settings_path"], "~/.claude/settings.json")
        env = config["env"]
        self.assertEqual(env["ANTHROPIC_AUTH_TOKEN"], "${ANTHROPIC_AUTH_TOKEN}")
        self.assertEqual(env["ANTHROPIC_BASE_URL"], "${ANTHROPIC_BASE_URL}")

    def test_sync_injects_resolved_env_into_missing_settings_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._write_secrets(
                temp_root,
                "ANTHROPIC_AUTH_TOKEN=tok-abc\nANTHROPIC_BASE_URL=https://relay.example/\n",
            )
            settings_path = temp_root / "home" / ".claude" / "settings.json"

            result = self._run_sync(temp_root, settings_path)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            written = json.loads(settings_path.read_text(encoding="utf-8"))
            self.assertEqual(written["env"]["ANTHROPIC_AUTH_TOKEN"], "tok-abc")
            self.assertEqual(written["env"]["ANTHROPIC_BASE_URL"], "https://relay.example/")

    def test_sync_preserves_other_top_level_fields_and_unmanaged_env_keys(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._write_secrets(
                temp_root,
                "ANTHROPIC_AUTH_TOKEN=tok-abc\nANTHROPIC_BASE_URL=https://relay.example/\n",
            )
            settings_path = temp_root / "home" / ".claude" / "settings.json"
            settings_path.parent.mkdir(parents=True, exist_ok=True)
            settings_path.write_text(
                json.dumps(
                    {
                        "theme": "light",
                        "env": {
                            "ANTHROPIC_AUTH_TOKEN": "stale",
                            "ANTHROPIC_BASE_URL": "stale",
                            "CLAUDE_DEBUG": "1",
                        },
                        "permissions": {"defaultMode": "auto"},
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )

            result = self._run_sync(temp_root, settings_path)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            written = json.loads(settings_path.read_text(encoding="utf-8"))
            self.assertEqual(written["theme"], "light")
            self.assertEqual(written["permissions"], {"defaultMode": "auto"})
            self.assertEqual(written["env"]["ANTHROPIC_AUTH_TOKEN"], "tok-abc")
            self.assertEqual(written["env"]["ANTHROPIC_BASE_URL"], "https://relay.example/")
            self.assertEqual(written["env"]["CLAUDE_DEBUG"], "1")

    def test_sync_errors_when_secrets_file_missing_required_key(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._write_secrets(temp_root, "ANTHROPIC_AUTH_TOKEN=tok-abc\n")
            settings_path = temp_root / "home" / ".claude" / "settings.json"

            env = {k: v for k, v in os.environ.items() if k != "ANTHROPIC_BASE_URL"}
            result = subprocess.run(
                [
                    "python3",
                    str(temp_root / "scripts" / "claude" / "sync_env.py"),
                    "--settings-path",
                    str(settings_path),
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )

            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            combined = result.stdout + result.stderr
            self.assertIn("ANTHROPIC_BASE_URL", combined)
            self.assertFalse(settings_path.exists(), "should not write when resolution fails")

    def test_sync_falls_back_to_process_env_when_secrets_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            secrets_path = temp_root / "secrets" / "claude.env"
            if secrets_path.exists():
                secrets_path.unlink()
            settings_path = temp_root / "home" / ".claude" / "settings.json"

            env = {
                **os.environ,
                "ANTHROPIC_AUTH_TOKEN": "tok-from-env",
                "ANTHROPIC_BASE_URL": "https://from-env.example/",
            }
            result = subprocess.run(
                [
                    "python3",
                    str(temp_root / "scripts" / "claude" / "sync_env.py"),
                    "--settings-path",
                    str(settings_path),
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            written = json.loads(settings_path.read_text(encoding="utf-8"))
            self.assertEqual(written["env"]["ANTHROPIC_AUTH_TOKEN"], "tok-from-env")
            self.assertEqual(written["env"]["ANTHROPIC_BASE_URL"], "https://from-env.example/")

    def test_dry_run_does_not_write_settings(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._write_secrets(
                temp_root,
                "ANTHROPIC_AUTH_TOKEN=tok-abc\nANTHROPIC_BASE_URL=https://relay.example/\n",
            )
            settings_path = temp_root / "home" / ".claude" / "settings.json"

            result = self._run_sync(temp_root, settings_path, "--dry-run")

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(settings_path.exists(), "dry-run must not create file")

    def test_make_target_runs_claude_env_sync(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("claude-env-sync", makefile)
        self.assertIn("claude\\:env\\:sync", makefile)
        self.assertIn("\nclaude-env-sync:\n\t@./manage.sh claude:env:sync\n", makefile)

    def test_pipeline_includes_claude_env_sync_task(self):
        manage_sh = (ROOT / "manage.sh").read_text(encoding="utf-8")
        self.assertIn("function run_claude_env_sync_task()", manage_sh)
        self.assertIn("run_pipeline_step run_claude_env_sync_task", manage_sh)


if __name__ == "__main__":
    unittest.main()
