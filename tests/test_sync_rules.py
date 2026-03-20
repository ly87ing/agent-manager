import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SyncRulesTests(unittest.TestCase):
    def test_dry_run_does_not_create_directory_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_dir = temp_root / "tmp-target" / "modules"
            self.assertFalse(target_dir.exists())
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_modules|global|rules/global/modules|{target_dir}\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--dry-run"],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(target_dir.exists(), result.stdout + result.stderr)

    def test_directory_sync_removes_stale_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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

    def test_directory_sync_removes_stale_backup_artifacts_without_rebackup(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_dir = temp_root / "tmp-target" / "modules"
            source_dir = temp_root / "agent-rule" / "rules" / "global" / "modules"
            shutil.copytree(source_dir, target_dir)
            stale_backup = target_dir / "frontend.md.bak.20260318123628"
            stale_backup.write_text("old backup\n", encoding="utf-8")
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

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertFalse(stale_backup.exists(), combined)
            self.assertEqual(list(target_dir.glob("*.bak.*")), [], combined)

    def test_copy_mode_replaces_broken_directory_symlink_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_dir = temp_root / "tmp-target" / "modules"
            target_dir.parent.mkdir(parents=True)
            target_dir.symlink_to(temp_root / "missing-modules")
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
            self.assertTrue(target_dir.is_dir(), result.stdout + result.stderr)
            self.assertFalse(target_dir.is_symlink(), result.stdout + result.stderr)
            self.assertTrue((target_dir / "architecture.md").exists(), result.stdout + result.stderr)
            self.assertEqual(len(list(target_dir.parent.glob("modules.bak.*"))), 1)

    def test_copy_mode_replaces_live_directory_symlink_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            real_dir = temp_root / "elsewhere"
            real_dir.mkdir(parents=True)
            target_dir = temp_root / "tmp-target" / "modules"
            target_dir.parent.mkdir(parents=True)
            target_dir.symlink_to(real_dir)
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
            self.assertTrue(target_dir.is_dir(), result.stdout + result.stderr)
            self.assertFalse(target_dir.is_symlink(), result.stdout + result.stderr)
            self.assertTrue((target_dir / "architecture.md").exists(), result.stdout + result.stderr)
            self.assertEqual(list(real_dir.iterdir()), [], result.stdout + result.stderr)
            self.assertEqual(len(list(target_dir.parent.glob("modules.bak.*"))), 1)

    def test_symlink_mode_replaces_broken_symlink_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_path = temp_root / "tmp-target" / "main.md"
            target_path.parent.mkdir(parents=True)
            target_path.symlink_to(temp_root / "missing-target.md")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_main|global|rules/global/main.md|{target_path}\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"),
                    "--mode",
                    "symlink",
                ],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )

            source_path = temp_root / "agent-rule" / "rules" / "global" / "main.md"
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(target_path.is_symlink(), result.stdout + result.stderr)
            self.assertEqual(target_path.resolve(), source_path.resolve())
            self.assertEqual(len(list(target_path.parent.glob("main.md.bak.*"))), 1)

    def test_invalid_manifest_target_is_rejected_before_writes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_dir = temp_root / "tmp-target" / "modules"
            source_dir = temp_root / "agent-rule" / "rules" / "global" / "modules"
            shutil.copytree(source_dir, target_dir)
            (target_dir / "stale.md").write_text("stale\n", encoding="utf-8")

            (temp_root / "config" / "manager.json").write_text(
                json.dumps(
                    {
                        "agents": {"claude-code": {"enabled": True}},
                        "tools": {},
                        "skills": {},
                        "mcp": {},
                    },
                    indent=2,
                ),
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
                f"claude_modules|global|rules/global/modules|{target_dir}\n",
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
            self.assertIn("claude_modules", combined)

    def test_main_rule_points_to_actual_modules_distribution_path(self):
        content = (ROOT / "agent-rule" / "rules" / "global" / "main.md").read_text(encoding="utf-8")
        self.assertIn("~/.<agent>/rules/modules/architecture.md", content)
        self.assertNotIn("~/.<agent>/modules/architecture.md", content)
