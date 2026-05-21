import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "scripts" / "claude" / "ccstatusline.settings.json"
INSTALLER = ROOT / "scripts" / "claude" / "install_ccstatusline_config.py"


class CcstatuslineSnapshotContractTests(unittest.TestCase):
    """The repo carries a canonical ccstatusline settings.json that the
    installer pushes to the user's config directory."""

    def test_snapshot_is_committed(self):
        self.assertTrue(SNAPSHOT.is_file(), SNAPSHOT)

    def test_snapshot_is_valid_json_with_expected_shape(self):
        data = json.loads(SNAPSHOT.read_text(encoding="utf-8"))
        self.assertEqual(data.get("version"), 3)
        self.assertIsInstance(data.get("lines"), list)
        self.assertGreaterEqual(len(data["lines"]), 1)
        # First line carries widgets; second/third are intentionally empty.
        self.assertIsInstance(data["lines"][0], list)
        self.assertTrue(
            any(widget.get("type") == "model" for widget in data["lines"][0]),
            "首行应包含 model widget",
        )


class CcstatuslineInstallerTests(unittest.TestCase):
    def _run(
        self,
        *args,
        env_home: Path | None = None,
        env_xdg: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        if env_home is not None:
            env["HOME"] = str(env_home)
        if env_xdg is not None:
            env["XDG_CONFIG_HOME"] = str(env_xdg)
        else:
            env.pop("XDG_CONFIG_HOME", None)
        return subprocess.run(
            ["python3", str(INSTALLER), *args],
            capture_output=True,
            text=True,
            check=False,
            env=env,
        )

    def test_writes_snapshot_to_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target = Path(temp_dir) / "nested" / "settings.json"
            result = self._run("--target", str(target))
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(target.is_file())
            self.assertEqual(target.read_bytes(), SNAPSHOT.read_bytes())

    def test_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target = Path(temp_dir) / "settings.json"
            first = self._run("--target", str(target))
            second = self._run("--target", str(target))
            self.assertEqual(first.returncode, 0)
            self.assertEqual(second.returncode, 0)
            self.assertIn("已写入", first.stdout + first.stderr)
            self.assertIn("已是最新", second.stdout + second.stderr)

    def test_dry_run_does_not_write(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target = Path(temp_dir) / "settings.json"
            result = self._run("--target", str(target), "--dry-run")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(target.exists())
            self.assertIn("预览", result.stdout + result.stderr)

    def test_xdg_config_home_drives_default_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            xdg = Path(temp_dir) / "xdg"
            home = Path(temp_dir) / "home"
            home.mkdir()
            result = self._run(env_home=home, env_xdg=xdg)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            expected = xdg / "ccstatusline" / "settings.json"
            self.assertTrue(expected.is_file(), expected)
            self.assertEqual(expected.read_bytes(), SNAPSHOT.read_bytes())

    def test_default_target_falls_back_to_dot_config(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            home = Path(temp_dir) / "home"
            home.mkdir()
            result = self._run(env_home=home)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            expected = home / ".config" / "ccstatusline" / "settings.json"
            self.assertTrue(expected.is_file(), expected)


class CcstatuslineManageWiringTests(unittest.TestCase):
    """The orchestration surface exposes the new sync target and the existing
    `wire` target now also pushes the ccstatusline config."""

    def test_manage_sh_exposes_config_target(self):
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        self.assertIn("claude:statusline:config)", manage)
        self.assertIn("install_ccstatusline_config.py", manage)

    def test_wire_target_now_syncs_ccstatusline_config(self):
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        wire_idx = manage.index("claude:statusline:wire)")
        wire_block = manage[wire_idx:wire_idx + 1200]
        self.assertIn("install_statusline.py", wire_block)
        self.assertIn("install_ccstatusline_config.py", wire_block)

    def test_makefile_exposes_config_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("claude\\:statusline\\:config", makefile)
        self.assertIn("claude-statusline-config", makefile)


class CcstatuslinePipelineWiringTests(unittest.TestCase):
    """`make update` / `make all` / `make sync` reconcile the ccstatusline
    config by running install_ccstatusline_config.py inside the standard
    pipelines."""

    def setUp(self) -> None:
        self.manage = (ROOT / "manage.sh").read_text(encoding="utf-8")

    def _pipeline_block(self, fn_name: str) -> str:
        start = self.manage.index(f"function {fn_name}()")
        end = self.manage.index("\n}\n", start)
        return self.manage[start:end]

    def test_task_helper_is_defined(self):
        self.assertIn("function run_ccstatusline_config_sync_task()", self.manage)
        self.assertIn("install_ccstatusline_config.py", self.manage)

    def test_update_pipeline_includes_config_sync(self):
        block = self._pipeline_block("run_update_pipeline")
        self.assertIn("run_ccstatusline_config_sync_task", block)

    def test_sync_pipeline_includes_config_sync(self):
        block = self._pipeline_block("run_sync_pipeline")
        self.assertIn("run_ccstatusline_config_sync_task", block)


if __name__ == "__main__":
    unittest.main()
