import os
import shutil
import subprocess
import tempfile
import unittest
from getpass import getuser
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANAGE = ROOT / "manage.sh"


class ManageScriptTests(unittest.TestCase):
    def test_doctor_does_not_remove_existing_lock(self):
        lock_path = Path(f"/tmp/agent_manager_{getuser()}.lock")
        original = lock_path.read_text(encoding="utf-8") if lock_path.exists() else None
        try:
            lock_path.write_text("999999", encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "doctor"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(lock_path.exists())
            self.assertEqual(lock_path.read_text(encoding="utf-8"), "999999")
        finally:
            if original is None:
                lock_path.unlink(missing_ok=True)
            else:
                lock_path.write_text(original, encoding="utf-8")

    def test_failed_task_reports_error_and_returns_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            failing_script = temp_root / "agent-tool" / "sync_tools.py"
            failing_script.write_text(
                "#!/usr/bin/env python3\nimport sys\nsys.exit(7)\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("工具失败", combined)
            self.assertIn("规则分发完成", combined)

    def test_logs_command_outputs_log_file(self):
        log_path = Path(f"/tmp/agent_manager_sync_{getuser()}.log")
        original = log_path.read_text(encoding="utf-8") if log_path.exists() else None
        try:
            log_path.write_text("hello log\n", encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "logs"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("hello log", result.stdout)
        finally:
            if original is None:
                log_path.unlink(missing_ok=True)
            else:
                log_path.write_text(original, encoding="utf-8")

    def test_clean_logs_command_removes_log_files(self):
        log_path = Path(f"/tmp/agent_manager_sync_{getuser()}.log")
        rotated_log_path = Path(f"/tmp/agent_manager_sync_{getuser()}.log.1")
        original = log_path.read_text(encoding="utf-8") if log_path.exists() else None
        original_rotated = rotated_log_path.read_text(encoding="utf-8") if rotated_log_path.exists() else None
        try:
            log_path.write_text("hello log\n", encoding="utf-8")
            rotated_log_path.write_text("old log\n", encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "clean-logs"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(log_path.exists())
            self.assertFalse(rotated_log_path.exists())
        finally:
            if original is None:
                log_path.unlink(missing_ok=True)
            else:
                log_path.write_text(original, encoding="utf-8")
            if original_rotated is None:
                rotated_log_path.unlink(missing_ok=True)
            else:
                rotated_log_path.write_text(original_rotated, encoding="utf-8")
