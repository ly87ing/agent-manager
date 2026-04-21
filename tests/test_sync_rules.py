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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            cursor_target = temp_root / ".cursor" / "rules" / "agent-manager.mdc"
            (temp_root / "config" / "rules-targets.list").write_text(
                "cursor_global|global|rules/dist/cursor/agent-manager.mdc|{{REPO_ROOT}}/.cursor/rules/agent-manager.mdc\n",
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
            self.assertTrue(cursor_target.exists(), combined)
            self.assertEqual(
                cursor_target.read_text(encoding="utf-8"),
                (temp_root / "agent-rule" / "rules" / "dist" / "cursor" / "agent-manager.mdc").read_text(
                    encoding="utf-8"
                ),
            )

    def test_repo_root_targets_replace_existing_files_without_backup_artifacts(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            cursor_target = temp_root / ".cursor" / "rules" / "agent-manager.mdc"
            cursor_target.parent.mkdir(parents=True, exist_ok=True)
            cursor_target.write_text("stale cursor\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                "cursor_global|global|rules/dist/cursor/agent-manager.mdc|{{REPO_ROOT}}/.cursor/rules/agent-manager.mdc\n",
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
            self.assertEqual(
                cursor_target.read_text(encoding="utf-8"),
                (temp_root / "agent-rule" / "rules" / "dist" / "cursor" / "agent-manager.mdc").read_text(
                    encoding="utf-8"
                ),
            )
            self.assertEqual(list(cursor_target.parent.glob("agent-manager.mdc.bak.*")), [], combined)

    def test_codex_sync_supports_native_agents_filename(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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

    def test_kimi_sync_supports_native_agents_filename(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            temp_bin = Path(temp_dir) / "bin"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            (temp_home / ".kimi").mkdir(parents=True)
            target_path = temp_home / ".kimi" / "AGENTS.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                f"kimi_global|global|rules/dist/kimi/AGENTS.md|{target_path}\n",
                encoding="utf-8",
            )
            temp_bin.mkdir(parents=True)
            kimi_bin = temp_bin / "kimi"
            kimi_bin.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            kimi_bin.chmod(0o755)

            env = os.environ.copy()
            env["HOME"] = str(temp_home)
            env["PATH"] = f"{temp_bin}{os.pathsep}{env.get('PATH', '')}"
            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--target", "kimi_global"],
                cwd=temp_root / "agent-rule",
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            source_path = temp_root / "agent-rule" / "rules" / "dist" / "kimi" / "AGENTS.md"
            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(target_path.exists(), combined)
            self.assertEqual(target_path.read_text(encoding="utf-8"), source_path.read_text(encoding="utf-8"))

    def test_codex_sync_removes_legacy_global_rule_path(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_dir = temp_root / "tmp-target" / "modules"
            self.assertFalse(target_dir.exists())
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_modules|global|rules/dist/modules|{target_dir}\n",
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
            source_dir = temp_root / "agent-rule" / "rules" / "dist" / "modules"
            shutil.copytree(source_dir, target_dir)
            stale_file = target_dir / "stale.md"
            stale_file.write_text("stale\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_modules|global|rules/dist/modules|{target_dir}\n",
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
            source_dir = temp_root / "agent-rule" / "rules" / "dist" / "modules"
            shutil.copytree(source_dir, target_dir)
            stale_backup = target_dir / "frontend.md.bak.20260318123628"
            stale_backup.write_text("old backup\n", encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text(
                f"test_modules|global|rules/dist/modules|{target_dir}\n",
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
                f"test_modules|global|rules/dist/modules|{target_dir}\n",
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
                f"test_modules|global|rules/dist/modules|{target_dir}\n",
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
            self.assertEqual(len(list(target_path.parent.glob("main.md.bak.*"))), 1)

    def test_invalid_manifest_target_is_rejected_before_writes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            duplicate_target = (temp_root / ".cursor" / "rules" / "agent-manager.mdc").resolve()
            (temp_root / "config" / "rules-targets.list").write_text(
                (
                    "cursor_global|global|rules/dist/cursor/agent-manager.mdc|"
                    "{{REPO_ROOT}}/.cursor/rules/agent-manager.mdc\n"
                    f"duplicate_cursor|global|rules/dist/cursor/agent-manager.mdc|{duplicate_target}\n"
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            target_dir = temp_root / "tmp-target" / "modules"
            source_dir = temp_root / "agent-rule" / "rules" / "dist" / "modules"
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
                f"claude_modules|global|rules/dist/modules|{target_dir}\n",
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

    def test_claude_main_rule_points_to_relative_modules_path(self):
        content = (ROOT / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("read `modules/architecture.md`", content)
        self.assertNotIn("~/.<agent>/modules/architecture.md", content)

    def test_codex_main_rule_points_to_manifest_modules_path(self):
        content = (ROOT / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md").read_text(encoding="utf-8")
        codex_modules_target = self._read_manifest_target("codex_modules").rstrip("/")
        self.assertIn(f"read `{codex_modules_target}/architecture.md`", content)
        self.assertNotIn("read `modules/architecture.md`", content)

    def test_kimi_main_rule_points_to_manifest_modules_path(self):
        content = (ROOT / "agent-rule" / "rules" / "dist" / "kimi" / "AGENTS.md").read_text(encoding="utf-8")
        kimi_modules_target = self._read_manifest_target("kimi_modules").rstrip("/")
        self.assertIn(f"read `{kimi_modules_target}/architecture.md`", content)
        self.assertNotIn("read `modules/architecture.md`", content)

    def test_manifest_targets_built_rule_outputs(self):
        manifest = (ROOT / "config" / "rules-targets.list").read_text(encoding="utf-8")
        self.assertIn("codex_global|global|rules/dist/codex/AGENTS.md|~/.codex/AGENTS.md", manifest)
        self.assertIn("kimi_global|global|rules/dist/kimi/AGENTS.md|~/.kimi/AGENTS.md", manifest)
        self.assertNotIn("codex_global|global|rules/dist/codex/main.md|~/.codex/rules/global.md", manifest)
        self.assertIn("claude_global|global|rules/dist/claude/CLAUDE.md|~/.claude/CLAUDE.md", manifest)
        self.assertIn("gemini_global|global|rules/dist/gemini/GEMINI.md|~/.gemini/GEMINI.md", manifest)
        self.assertIn(
            "cursor_global|global|rules/dist/cursor/agent-manager.mdc|{{REPO_ROOT}}/.cursor/rules/agent-manager.mdc",
            manifest,
        )
        self.assertIn(
            "cursor_global|global|rules/dist/cursor/agent-manager.mdc|{{REPO_ROOT}}/.cursor/rules/agent-manager.mdc",
            manifest,
        )
        self.assertNotIn("copilot_global|global|rules/dist/copilot/copilot-instructions.md", manifest)
        self.assertNotIn("opencode_global|global|rules/dist/ide/main.md|~/.config/opencode/rules/global.md", manifest)
        self.assertIn("codex_modules|global|rules/dist/modules|~/.codex/modules", manifest)
        self.assertIn("kimi_modules|global|rules/dist/modules|~/.kimi/modules", manifest)
        self.assertIn("claude_modules|global|rules/dist/modules|~/.claude/modules", manifest)
        self.assertIn("gemini_modules|global|rules/dist/modules|~/.gemini/modules", manifest)
        self.assertIn("cursor_modules|global|rules/dist/modules|{{REPO_ROOT}}/.cursor/rules/modules", manifest)
        self.assertNotIn("copilot_modules|global|rules/dist/modules|{{REPO_ROOT}}/.github/modules", manifest)
