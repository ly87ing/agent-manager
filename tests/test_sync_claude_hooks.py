import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ClaudeHooksSyncTests(unittest.TestCase):
    def _copy_repo(self, temp_dir: str) -> Path:
        temp_root = Path(temp_dir) / "repo"
        shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", ".git", ".worktrees"))
        return temp_root

    def _run_sync(self, temp_root: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(temp_root / "agent-rule" / "scripts" / "sync-claude-hooks.py"), *args],
            cwd=temp_root / "agent-rule",
            capture_output=True,
            text=True,
            check=False,
        )

    def _read_settings(self, temp_root: Path) -> dict:
        return json.loads((temp_root / ".claude" / "settings.json").read_text(encoding="utf-8"))

    def _init_git_repo(self, temp_root: Path) -> None:
        subprocess.run(["git", "init"], cwd=temp_root, capture_output=True, text=True, check=True)
        subprocess.run(["git", "config", "user.email", "tests@example.com"], cwd=temp_root, capture_output=True, text=True, check=True)
        subprocess.run(["git", "config", "user.name", "Tests"], cwd=temp_root, capture_output=True, text=True, check=True)
        subprocess.run(["git", "add", "."], cwd=temp_root, capture_output=True, text=True, check=True)
        subprocess.run(["git", "commit", "-m", "baseline"], cwd=temp_root, capture_output=True, text=True, check=True)

    def test_sync_writes_project_claude_settings_from_managed_definition(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_sync(temp_root)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            settings = self._read_settings(temp_root)
            self.assertIn("hooks", settings)
            self.assertEqual(settings["hooks"], {})

    def test_sync_preserves_unrelated_settings_and_deduplicates_managed_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            settings_path = temp_root / ".claude" / "settings.json"
            settings_path.parent.mkdir(parents=True, exist_ok=True)
            settings_path.write_text(
                json.dumps(
                    {
                        "theme": "dark",
                        "hooks": {
                            "PreToolUse": [
                                {
                                    "matcher": "Bash",
                                    "hooks": [{"type": "command", "command": "echo keep"}],
                                },
                                {
                                    "matcher": "Read",
                                    "hooks": [
                                        {
                                            "type": "command",
                                            "command": 'bash "$CLAUDE_PROJECT_DIR/agent-rule/hooks/block-dist-edits.sh"',
                                        }
                                    ],
                                }
                            ],
                            "Notification": [
                                {
                                    "hooks": [
                                        {
                                            "type": "command",
                                            "command": 'bash "$CLAUDE_PROJECT_DIR/agent-rule/hooks/check-rules-build.sh"',
                                        }
                                    ],
                                }
                            ],
                        },
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )

            first = self._run_sync(temp_root)
            second = self._run_sync(temp_root)

            combined = first.stdout + first.stderr + second.stdout + second.stderr
            self.assertEqual(first.returncode, 0, combined)
            self.assertEqual(second.returncode, 0, combined)
            settings = self._read_settings(temp_root)
            self.assertEqual(settings["theme"], "dark")
            pre_tool_hooks = settings["hooks"].get("PreToolUse", [])
            self.assertEqual(
                sum(
                    1
                    for entry in pre_tool_hooks
                    if any(hook.get("command") == "echo keep" for hook in entry.get("hooks", []))
                ),
                1,
            )
            self.assertEqual(
                sum(
                    1
                    for entry in pre_tool_hooks
                    if entry.get("matcher") == "Read"
                    and any(
                        "block-dist-edits.sh" in hook.get("command", "")
                        for hook in entry.get("hooks", [])
                    )
                ),
                1,
                "non-matching removed signature should be preserved",
            )

    def test_sync_dry_run_does_not_write_project_settings(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            settings_path = temp_root / ".claude" / "settings.json"
            before = settings_path.read_text(encoding="utf-8")

            result = self._run_sync(temp_root, "--dry-run")

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(settings_path.read_text(encoding="utf-8"), before, result.stdout + result.stderr)

    def test_committed_project_settings_match_fresh_sync(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_sync(temp_root)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            committed = json.loads((ROOT / ".claude" / "settings.json").read_text(encoding="utf-8"))
            self.assertEqual(committed, self._read_settings(temp_root))

    def test_sync_strips_removed_signatures(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            subagent_stop_command = 'bash "$CLAUDE_PROJECT_DIR/agent-rule/hooks/check-rules-build.sh"'
            block_dist_command = 'bash "$CLAUDE_PROJECT_DIR/agent-rule/hooks/block-dist-edits.sh"'
            unrelated_command = "echo keep me"
            settings_path = temp_root / ".claude" / "settings.json"
            current = json.loads(settings_path.read_text(encoding="utf-8"))
            current["hooks"]["SubagentStop"] = [
                {
                    "hooks": [{"type": "command", "command": subagent_stop_command}],
                }
            ]
            current["hooks"]["PreToolUse"] = [
                {
                    "matcher": "Edit|MultiEdit|Write",
                    "hooks": [{"type": "command", "command": block_dist_command}],
                },
                {
                    "matcher": "Bash",
                    "hooks": [{"type": "command", "command": unrelated_command}],
                },
            ]
            settings_path.write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8")

            result = self._run_sync(temp_root)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            settings = self._read_settings(temp_root)
            self.assertFalse(
                any(
                    any(subagent_stop_command == hook.get("command") for hook in entry.get("hooks", []))
                    for entry in settings["hooks"].get("SubagentStop", [])
                ),
                f"old SubagentStop hook should be stripped: {settings['hooks'].get('SubagentStop', [])}",
            )
            pre_tool_hooks = settings["hooks"]["PreToolUse"]
            self.assertFalse(
                any(
                    any(block_dist_command == hook.get("command") for hook in entry.get("hooks", []))
                    for entry in pre_tool_hooks
                ),
                f"old block-dist hook should be stripped: {pre_tool_hooks}",
            )
            self.assertTrue(
                any(
                    any(hook.get("command") == unrelated_command for hook in entry.get("hooks", []))
                    for entry in pre_tool_hooks
                ),
                f"unrelated hook should be preserved: {pre_tool_hooks}",
            )

    def test_sync_resolves_settings_path_relative_to_custom_config(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            default_settings = temp_root / ".claude" / "settings.json"
            default_settings.unlink()
            custom_dir = temp_root / "custom-hooks"
            custom_dir.mkdir()
            custom_config = custom_dir / "claude-hooks.json"
            custom_settings = custom_dir / "nested" / "settings.json"
            custom_definition = json.loads((temp_root / "config" / "claude-hooks.json").read_text(encoding="utf-8"))
            custom_definition["settings_path"] = "nested/settings.json"
            custom_config.write_text(json.dumps(custom_definition, indent=2) + "\n", encoding="utf-8")

            result = self._run_sync(temp_root, "--config", str(custom_config))

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(custom_settings.exists(), result.stdout + result.stderr)
            self.assertFalse(default_settings.exists(), result.stdout + result.stderr)

    def test_block_dist_edits_hook_denies_generated_rule_writes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            payload = {
                "tool_input": {
                    "file_path": str(temp_root / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md")
                }
            }

            result = subprocess.run(
                ["bash", str(temp_root / "agent-rule" / "hooks" / "block-dist-edits.sh")],
                cwd=temp_root,
                env={**os.environ, "CLAUDE_PROJECT_DIR": str(temp_root)},
                input=json.dumps(payload),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            response = json.loads(result.stdout)
            self.assertEqual(response["hookSpecificOutput"]["hookEventName"], "PreToolUse")
            self.assertEqual(response["hookSpecificOutput"]["permissionDecision"], "deny")
            self.assertIn("generated output", response["hookSpecificOutput"]["permissionDecisionReason"])

    def _run_block_dist_edits(self, temp_root: Path, payload: dict) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(temp_root / "agent-rule" / "hooks" / "block-dist-edits.sh")],
            cwd=temp_root,
            env={**os.environ, "CLAUDE_PROJECT_DIR": str(temp_root)},
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            check=False,
        )

    def test_block_dist_edits_hook_denies_bash_redirect_writes_to_dist(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_block_dist_edits(
                temp_root,
                {"tool_input": {"command": "echo broken > agent-rule/rules/dist/claude/CLAUDE.md"}},
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            response = json.loads(result.stdout)
            self.assertEqual(response["hookSpecificOutput"]["permissionDecision"], "deny")

    def test_block_dist_edits_hook_denies_bash_rm_of_dist(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_block_dist_edits(
                temp_root,
                {"tool_input": {"command": "rm agent-rule/rules/dist/claude/CLAUDE.md"}},
            )

            response = json.loads(result.stdout)
            self.assertEqual(response["hookSpecificOutput"]["permissionDecision"], "deny")

    def test_block_dist_edits_hook_allows_bash_read_of_dist(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_block_dist_edits(
                temp_root,
                {"tool_input": {"command": "cat agent-rule/rules/dist/claude/CLAUDE.md"}},
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stdout.strip(), "", "read of dist should not deny")

    def test_block_dist_edits_hook_allows_make_rules_build(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_block_dist_edits(
                temp_root,
                {"tool_input": {"command": "make rules-build"}},
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stdout.strip(), "", "managed builder must not be denied")

    def test_block_dist_edits_hook_fast_path_skips_payload_without_dist(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_block_dist_edits(
                temp_root,
                {"tool_input": {"command": "ls -la"}},
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stdout.strip(), "")

    def test_check_rules_build_hook_blocks_stop_when_src_changed_without_dist_refresh(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._init_git_repo(temp_root)
            src_path = temp_root / "agent-rule" / "rules" / "src" / "shared" / "core.md"
            src_path.write_text(src_path.read_text(encoding="utf-8") + "\n# touch\n", encoding="utf-8")

            result = subprocess.run(
                ["bash", str(temp_root / "agent-rule" / "hooks" / "check-rules-build.sh")],
                cwd=temp_root,
                env={**os.environ, "CLAUDE_PROJECT_DIR": str(temp_root)},
                input=json.dumps({"hook_event_name": "Stop"}),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            response = json.loads(result.stdout)
            self.assertEqual(response["decision"], "block")
            self.assertIn("make rules-build", response["reason"])

    def test_check_rules_build_hook_allows_stop_after_dist_changes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._init_git_repo(temp_root)
            src_path = temp_root / "agent-rule" / "rules" / "src" / "shared" / "core.md"
            src_path.write_text(src_path.read_text(encoding="utf-8") + "\n# touch\n", encoding="utf-8")
            build = subprocess.run(
                ["python3", str(temp_root / "agent-rule" / "scripts" / "build-agent-rules.py")],
                cwd=temp_root / "agent-rule",
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(build.returncode, 0, build.stdout + build.stderr)

            result = subprocess.run(
                ["bash", str(temp_root / "agent-rule" / "hooks" / "check-rules-build.sh")],
                cwd=temp_root,
                env={**os.environ, "CLAUDE_PROJECT_DIR": str(temp_root)},
                input=json.dumps({"hook_event_name": "Stop"}),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stdout.strip(), "", result.stdout + result.stderr)

    def test_check_rules_build_hook_blocks_when_dist_is_dirty_but_stale(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            self._init_git_repo(temp_root)
            src_path = temp_root / "agent-rule" / "rules" / "src" / "shared" / "core.md"
            dist_path = temp_root / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md"
            src_path.write_text(src_path.read_text(encoding="utf-8") + "\n# touch\n", encoding="utf-8")
            dist_path.write_text(dist_path.read_text(encoding="utf-8") + "\n# manual touch\n", encoding="utf-8")

            result = subprocess.run(
                ["bash", str(temp_root / "agent-rule" / "hooks" / "check-rules-build.sh")],
                cwd=temp_root,
                env={**os.environ, "CLAUDE_PROJECT_DIR": str(temp_root)},
                input=json.dumps({"hook_event_name": "Stop"}),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            response = json.loads(result.stdout)
            self.assertEqual(response["decision"], "block")
            self.assertIn("make rules-build", response["reason"])

    def test_check_rules_build_hook_blocks_when_manifest_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            manifest_path = temp_root / "config" / "rules-targets.list"
            manifest_path.unlink()

            result = subprocess.run(
                ["bash", str(temp_root / "agent-rule" / "hooks" / "check-rules-build.sh")],
                cwd=temp_root,
                env={**os.environ, "CLAUDE_PROJECT_DIR": str(temp_root)},
                input=json.dumps({"hook_event_name": "Stop"}),
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            response = json.loads(result.stdout)
            self.assertEqual(response["decision"], "block")
            self.assertIn("rules-targets.list", response["reason"])

    def test_manage_command_syncs_project_claude_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "claude:hooks:apply"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_root / ".claude" / "settings.json").exists(), result.stdout + result.stderr)

    def test_make_target_syncs_project_claude_hooks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = subprocess.run(
                ["make", "claude-hooks-sync"],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_root / ".claude" / "settings.json").exists(), result.stdout + result.stderr)

    def test_makefile_exposes_claude_hooks_sync_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("claude-hooks-sync", makefile)
        self.assertIn("claude\\:hooks\\:sync", makefile)
        self.assertIn("@./manage.sh claude:hooks:apply", makefile)
