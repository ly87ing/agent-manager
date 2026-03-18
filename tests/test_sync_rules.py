import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SyncRulesTests(unittest.TestCase):
    def test_directory_sync_removes_stale_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            target_dir = temp_root / "tmp-target" / "modules"
            source_dir = temp_root / "agent-rule" / "rules" / "global" / "modules"
            shutil.copytree(source_dir, target_dir)
            stale_file = target_dir / "stale.md"
            stale_file.write_text("stale\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_modules|global|rules/global/modules|{target_dir}\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh")],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(stale_file.exists(), result.stdout + result.stderr)

    def test_invalid_manifest_target_is_rejected_before_writes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            bad_target = "relative/rules/global.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                f"bad_target|global|rules/global/main.md|{bad_target}\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh")],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_doctor_audits_directory_targets_structurally(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            target_dir = temp_root / "tmp-target" / "modules"
            source_dir = temp_root / "agent-rule" / "rules" / "global" / "modules"
            shutil.copytree(source_dir, target_dir)
            (target_dir / "stale.md").write_text("stale\n", encoding="utf-8")

            (temp_root / "config" / "manager.json").write_text(
                json.dumps({"agents": {}, "tools": {}, "skills": {}, "mcp": {}}, indent=2),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                json.dumps({"agent_paths": {}, "servers": {}}, indent=2),
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                json.dumps({"repositories": {}, "agents": {}}, indent=2),
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_modules|global|rules/global/modules|{target_dir}\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("test_modules", combined)

    def test_main_rule_points_to_actual_modules_distribution_path(self):
        content = (ROOT / "agent-rule" / "rules" / "global" / "main.md").read_text(encoding="utf-8")
        self.assertIn("~/.<agent>/rules/modules/architecture.md", content)
        self.assertNotIn("~/.<agent>/modules/architecture.md", content)
