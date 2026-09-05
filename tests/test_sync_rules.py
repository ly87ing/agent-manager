import os
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SyncRulesTests(unittest.TestCase):
    def _read_manifest_target(self, name: str) -> str:
        manifest = (ROOT / "config" / "rules-targets.list").read_text(encoding="utf-8")
        for raw_line in manifest.splitlines():
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            entry_name, _group, _source, target = line.split("|", 3)
            if entry_name == name:
                return target
        raise AssertionError(f"missing manifest target: {name}")

    def test_multiple_target_filters_sync_all_requested_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", "app", "node_modules"))

            (temp_home / ".claude").mkdir(parents=True)
            (temp_home / ".codex").mkdir(parents=True)
            claude_target = temp_home / ".claude" / "CLAUDE.md"
            codex_target = temp_home / ".codex" / "AGENTS.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                (
                    f"claude_global|global|rules/dist/claude/CLAUDE.md|{claude_target}\n"
                    f"codex_global|global|rules/dist/codex/AGENTS.md|{codex_target}\n"
                ),
                encoding="utf-8",
            )

            env = os.environ.copy()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [
                    str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"),
                    "--target",
                    "claude_global",
                    "--target",
                    "codex_global",
                ],
                cwd=temp_root / "agent-rule",
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(claude_target.exists(), combined)
            self.assertTrue(codex_target.exists(), combined)
            self.assertEqual(
                claude_target.read_text(encoding="utf-8"),
                (temp_root / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md").read_text(encoding="utf-8"),
            )
            self.assertEqual(
                codex_target.read_text(encoding="utf-8"),
                (temp_root / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md").read_text(encoding="utf-8"),
            )

    def test_sync_supports_repo_root_targets_via_repo_root_placeholder(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", "app", "node_modules"))
            target_path = temp_root / "CLAUDE.generated.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_global|global|rules/dist/claude/CLAUDE.md|{{REPO_ROOT}}/CLAUDE.generated.md\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--target", "claude_global"],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(target_path.exists(), combined)
            self.assertEqual(
                target_path.read_text(encoding="utf-8"),
                (temp_root / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md").read_text(encoding="utf-8"),
            )

    def test_repo_root_targets_replace_existing_files_without_backup(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", "app", "node_modules"))
            target_path = temp_root / "CLAUDE.generated.md"
            target_path.write_text("old content\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_global|global|rules/dist/claude/CLAUDE.md|{{REPO_ROOT}}/CLAUDE.generated.md\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--target", "claude_global"],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("old content", target_path.read_text(encoding="utf-8"))
            self.assertEqual(
                len(list(temp_root.glob("CLAUDE.generated.md.bak.*"))),
                0,
                "overwrite is git-redundant, no .bak",
            )

    def test_codex_sync_supports_native_agents_filename(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            (temp_home / ".codex").mkdir(parents=True)
            target_path = temp_home / ".codex" / "AGENTS.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                f"codex_global|global|rules/dist/codex/AGENTS.md|{target_path}\n",
                encoding="utf-8",
            )

            env = os.environ.copy()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--target", "codex_global"],
                cwd=temp_root / "agent-rule",
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            source_path = temp_root / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md"
            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(target_path.exists(), combined)
            self.assertEqual(target_path.read_text(encoding="utf-8"), source_path.read_text(encoding="utf-8"))

    def test_codex_sync_removes_legacy_global_rule_path(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", "app", "node_modules"))
            (temp_home / ".codex" / "rules").mkdir(parents=True)
            legacy_path = temp_home / ".codex" / "rules" / "global.md"
            legacy_path.write_text("legacy codex rule\n", encoding="utf-8")
            target_path = temp_home / ".codex" / "AGENTS.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                f"codex_global|global|rules/dist/codex/AGENTS.md|{target_path}\n",
                encoding="utf-8",
            )

            env = os.environ.copy()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--target", "codex_global"],
                cwd=temp_root / "agent-rule",
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(target_path.exists(), combined)
            self.assertFalse(legacy_path.exists(), combined)
            self.assertEqual(len(list(legacy_path.parent.glob("global.md.bak.*"))), 1, combined)

    def test_dry_run_does_not_create_directory_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            target_dir = temp_root / "tmp-target" / "rules"
            self.assertFalse(target_dir.exists())
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_rules|global|rules/dist/codex|{target_dir}\n",
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            target_dir = temp_root / "tmp-target" / "rules"
            source_dir = temp_root / "agent-rule" / "rules" / "dist" / "codex"
            shutil.copytree(source_dir, target_dir)
            stale_file = target_dir / "stale.md"
            stale_file.write_text("stale\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_rules|global|rules/dist/codex|{target_dir}\n",
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            target_dir = temp_root / "tmp-target" / "rules"
            source_dir = temp_root / "agent-rule" / "rules" / "dist" / "codex"
            shutil.copytree(source_dir, target_dir)
            stale_backup = target_dir / "AGENTS.md.bak.20260318123628"
            stale_backup.write_text("old backup\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_rules|global|rules/dist/codex|{target_dir}\n",
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            target_dir = temp_root / "tmp-target" / "rules"
            target_dir.parent.mkdir(parents=True)
            target_dir.symlink_to(temp_root / "missing-rules")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_rules|global|rules/dist/codex|{target_dir}\n",
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
            self.assertTrue((target_dir / "AGENTS.md").exists(), result.stdout + result.stderr)
            self.assertEqual(len(list(target_dir.parent.glob("rules.bak.*"))), 0, "overwrite is git-redundant, no .bak")

    def test_copy_mode_replaces_live_directory_symlink_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            real_dir = temp_root / "elsewhere"
            real_dir.mkdir(parents=True)
            target_dir = temp_root / "tmp-target" / "rules"
            target_dir.parent.mkdir(parents=True)
            target_dir.symlink_to(real_dir)
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_rules|global|rules/dist/codex|{target_dir}\n",
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
            self.assertTrue((target_dir / "AGENTS.md").exists(), result.stdout + result.stderr)
            self.assertEqual(list(real_dir.iterdir()), [], result.stdout + result.stderr)
            self.assertEqual(len(list(target_dir.parent.glob("rules.bak.*"))), 0, "overwrite is git-redundant, no .bak")

    def test_symlink_mode_replaces_broken_symlink_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            target_path = temp_root / "tmp-target" / "main.md"
            target_path.parent.mkdir(parents=True)
            target_path.symlink_to(temp_root / "missing-target.md")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_main|global|rules/dist/claude/CLAUDE.md|{target_path}\n",
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

            source_path = temp_root / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md"
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(target_path.is_symlink(), result.stdout + result.stderr)
            self.assertEqual(target_path.resolve(), source_path.resolve())
            self.assertEqual(len(list(target_path.parent.glob("main.md.bak.*"))), 0, "overwrite is git-redundant, no .bak")

    def test_invalid_manifest_target_is_rejected_before_writes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            bad_target = "relative/rules/global.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                f"bad_target|global|rules/dist/claude/CLAUDE.md|{bad_target}\n",
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

    def test_manifest_conflict_detects_repo_root_placeholder_and_absolute_equivalent(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            duplicate_target = (temp_root / "CLAUDE.generated.md").resolve()
            (temp_root / "config" / "rules-targets.list").write_text(
                (
                    "claude_global|global|rules/dist/claude/CLAUDE.md|"
                    "{{REPO_ROOT}}/CLAUDE.generated.md\n"
                    f"duplicate_claude|global|rules/dist/claude/CLAUDE.md|{duplicate_target}\n"
                ),
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
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("规则路径冲突", combined)

    def test_doctor_audits_directory_targets_structurally(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories', "app", "node_modules"))
            target_dir = temp_root / "tmp-target" / "rules"
            source_dir = temp_root / "agent-rule" / "rules" / "dist" / "codex"
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
                f"claude_rules|global|rules/dist/codex|{target_dir}\n",
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
            self.assertIn("claude_rules", combined)

    def test_claude_main_rule_routes_to_rule_derived_skills(self):
        content = (ROOT / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("use the `change-discipline` skill", content)
        self.assertIn("use the `artifact-hygiene` skill", content)
        self.assertNotIn("modules/", content)

    def test_codex_main_rule_routes_to_rule_derived_skills(self):
        content = (ROOT / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md").read_text(encoding="utf-8")
        self.assertIn("use the `frontend-verification` skill", content)
        self.assertIn("use the `reader-facing-writing` skill", content)
        self.assertNotIn("modules/", content)

    def test_manifest_targets_built_rule_outputs(self):
        manifest = (ROOT / "config" / "rules-targets.list").read_text(encoding="utf-8")
        self.assertIn("codex_global|global|rules/dist/codex/AGENTS.md|~/.codex/AGENTS.md", manifest)
        self.assertIn("claude_global|global|rules/dist/claude/CLAUDE.md|~/.claude/CLAUDE.md", manifest)
        self.assertNotIn("rules/dist/modules", manifest)
        self.assertNotIn("_modules|", manifest)
