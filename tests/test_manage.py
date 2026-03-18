import os
import shutil
import subprocess
import tempfile
import unittest
from getpass import getuser
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANAGE = ROOT / "manage.sh"
USER = getuser()
LOCK_PATH = Path(f"/tmp/agent_manager_{USER}.lock")
LOCK_DIR = Path(f"/tmp/agent_manager_{USER}.lock.d")
LATEST_LOG_POINTER = Path(f"/tmp/agent_manager_sync_{USER}.latest")
LEGACY_LOG_PATH = Path(f"/tmp/agent_manager_sync_{USER}.log")


def move_aside(path: Path):
    if not path.exists() and not path.is_symlink():
        return None
    backup_root = Path(tempfile.gettempdir()) / "agent_manager_test_backups"
    backup_root.mkdir(exist_ok=True)
    backup = backup_root / f"{os.getpid()}_{path.name}"
    if backup.exists() or backup.is_symlink():
        if backup.is_dir() and not backup.is_symlink():
            shutil.rmtree(backup)
        else:
            backup.unlink()
    shutil.move(str(path), str(backup))
    return backup


def remove_path(path: Path):
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
    elif path.is_dir():
        shutil.rmtree(path)


def restore_path(path: Path, backup):
    remove_path(path)
    if backup is not None:
        shutil.move(str(backup), str(path))


class ManageScriptTests(unittest.TestCase):
    def _write_stub(self, path: Path, content: str):
        path.write_text(content, encoding="utf-8")
        path.chmod(0o755)

    def _append_log_stub(self, path: Path, label: str, log_path: Path, extra_condition: str = "True"):
        content = (
            "#!/usr/bin/env python3\n"
            "import sys\n"
            f"open({str(log_path)!r}, 'a', encoding='utf-8').write({label!r} + ' ' + ' '.join(sys.argv[1:]) + '\\n')\n"
            f"sys.exit(0 if ({extra_condition}) else 91)\n"
        )
        self._write_stub(path, content)

    def test_doctor_does_not_remove_existing_lock(self):
        original = LOCK_PATH.read_text(encoding="utf-8") if LOCK_PATH.exists() else None
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root = Path(temp_dir) / "repo"
                temp_home = Path(temp_dir) / "home"
                temp_home.mkdir()
                shutil.copytree(ROOT, temp_root)
                (temp_root / "config" / "manager.json").write_text(
                    '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                    encoding="utf-8",
                )
                (temp_root / "config" / "mcp.json").write_text(
                    '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                    encoding="utf-8",
                )
                (temp_root / "config" / "skills.json").write_text(
                    '{\n  "repositories": {},\n  "agents": {}\n}\n',
                    encoding="utf-8",
                )
                (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

                LOCK_PATH.write_text("999999", encoding="utf-8")
                env = os.environ.copy()
                env["HOME"] = str(temp_home)
                result = subprocess.run(
                    [str(temp_root / "manage.sh"), "doctor"],
                    cwd=temp_root,
                    env=env,
                    capture_output=True,
                    text=True,
                    check=False,
                )

                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertTrue(LOCK_PATH.exists())
                self.assertEqual(LOCK_PATH.read_text(encoding="utf-8"), "999999")
        finally:
            if original is None:
                LOCK_PATH.unlink(missing_ok=True)
            else:
                LOCK_PATH.write_text(original, encoding="utf-8")

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

    def test_sync_runs_only_local_config_tasks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "False",
            )
            self._append_log_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "tools",
                call_log,
                "False",
            )
            self._append_log_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "skills",
                call_log,
                "'--offline' in sys.argv and '--dry-run' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--offline' in sys.argv and '--prepare-runtime' not in sys.argv",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 13\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertIn("rules ", log_lines[0])
            self.assertTrue(any(line.startswith("skills ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") for line in log_lines), log_lines)
            self.assertFalse(any(line.startswith("agents ") for line in log_lines), log_lines)
            self.assertFalse(any(line.startswith("tools ") for line in log_lines), log_lines)
            self.assertFalse(any(line == "init-secrets" for line in log_lines), log_lines)

    def test_update_refreshes_resources_without_agent_install(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--no-install' in sys.argv and '--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "tools",
                call_log,
                "'--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "skills",
                call_log,
                "'--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--prepare-runtime' in sys.argv and '--offline' not in sys.argv",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    "mkdir -p \"$PWD/../secrets/.git\"\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertIn("init-secrets", log_lines, log_lines)
            self.assertTrue(any(line.startswith("agents ") and "--no-install" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("tools ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("rules ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("skills ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") and "--prepare-runtime" in line for line in log_lines), log_lines)

    def test_bootstrap_allows_agent_install(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--no-install' not in sys.argv and '--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "tools",
                call_log,
                "'--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "skills",
                call_log,
                "'--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--prepare-runtime' in sys.argv and '--offline' not in sys.argv",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    "mkdir -p \"$PWD/../secrets/.git\"\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "bootstrap"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(any(line.startswith("agents ") and "--no-install" not in line for line in log_lines), log_lines)

    def test_self_update_only_runs_repo_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"git $*\" >> {str(call_log)!r}\n"
                    "echo 'Already up to date'\n"
                    "exit 0\n"
                ),
            )
            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "False")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "False")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "False")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "False")
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 99\n")

            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "self-update"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(log_lines), 1, log_lines)
            self.assertIn(f"git -C {temp_root.resolve()} pull", log_lines[0])

    def test_plan_runs_bootstrap_pipeline_in_dry_run_mode(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--no-install' not in sys.argv and '--dry-run' in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "tools",
                call_log,
                "'--dry-run' in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "skills",
                call_log,
                "'--dry-run' in sys.argv and '--offline' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--prepare-runtime' in sys.argv and '--dry-run' in sys.argv",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "case \"$*\" in\n"
                    "  *--dry-run*) exit 0 ;;\n"
                    "  *) exit 23 ;;\n"
                    "esac\n"
                ),
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 17\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "plan"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("空转模式，跳过 secrets 检查", combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertFalse(any(line == "init-secrets" for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("agents ") and "--dry-run" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("tools ") and "--dry-run" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("rules ") and "--dry-run" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("skills ") and "--dry-run" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") and "--dry-run" in line for line in log_lines), log_lines)

    def test_dry_run_skips_secrets_bootstrap_and_git_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            remove_path(temp_root / "secrets")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("空转模式，跳过 secrets 检查", combined)
            self.assertNotIn("正在克隆凭证仓库", combined)

    def test_dry_run_offline_combination_respects_offline_flag(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            remove_path(temp_root / "secrets")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("离线模式，跳过 secrets 检查", combined)
            self.assertNotIn("正在克隆凭证仓库", combined)

    def test_offline_is_forwarded_to_networked_subtasks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)

            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0 if '--offline' in sys.argv else 9)\n",
            )
            self._write_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0 if '--offline' in sys.argv else 9)\n",
            )
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0 if '--offline' in sys.argv else 9)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0 if '--offline' in sys.argv else 9)\n",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                "#!/usr/bin/env bash\nexit 0\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)

    def test_progress_output_preserves_utf8_log_lines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)

            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys, time\n"
                    "print('[AGENT: Claude Code] 状态一致，正在尝试检查更新并验证远端元数据完整性...', flush=True)\n"
                    "time.sleep(1)\n"
                    "sys.exit(0)\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 0\n")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("\ufffd", combined)

    def test_progress_output_reports_elapsed_time_for_long_running_task(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)

            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys, time\n"
                    "print('[AGENT: Claude Code] 状态一致，正在尝试检查更新...', flush=True)\n"
                    "time.sleep(1)\n"
                    "sys.exit(0)\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 0\n")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertRegex(combined, r"已运行 [0-9]+s")

    def test_progress_output_does_not_reuse_previous_task_log_lines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)

            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys, time\n"
                    "print('[AGENT: Claude Code] 状态一致，正在尝试检查更新...', flush=True)\n"
                    "time.sleep(1)\n"
                    "sys.exit(0)\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "#!/usr/bin/env python3\nimport sys, time\ntime.sleep(1)\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 0\n")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotRegex(combined, r"同步全局工具: .*Claude Code")

    def test_progress_output_does_not_redraw_identical_message_every_poll(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)

            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys, time\n"
                    "print('[AGENT: Claude Code] 状态一致，正在尝试检查更新...', flush=True)\n"
                    "time.sleep(1.2)\n"
                    "sys.exit(0)\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 0\n")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertLessEqual(combined.count("对齐 Agent 生命周期: [AGENT: Claude Code] 状态一致"), 2, combined)

    def test_atomic_lock_directory_blocks_new_run(self):
        lock_backup = move_aside(LOCK_PATH)
        lock_dir_backup = move_aside(LOCK_DIR)
        holder = subprocess.Popen(["sleep", "60"])
        try:
            LOCK_DIR.mkdir(parents=True)
            (LOCK_DIR / "pid").write_text(str(holder.pid), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "--dry-run", "--offline"],
                cwd=ROOT,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("另一个管理任务", combined)
        finally:
            holder.terminate()
            holder.wait(timeout=5)
            restore_path(LOCK_PATH, lock_backup)
            restore_path(LOCK_DIR, lock_dir_backup)

    def test_unlock_removes_atomic_lock_artifacts(self):
        lock_backup = move_aside(LOCK_PATH)
        lock_dir_backup = move_aside(LOCK_DIR)
        try:
            LOCK_PATH.write_text("999999", encoding="utf-8")
            LOCK_DIR.mkdir(parents=True)
            (LOCK_DIR / "pid").write_text("999999", encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "unlock"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(LOCK_PATH.exists())
            self.assertFalse(LOCK_DIR.exists())
        finally:
            restore_path(LOCK_PATH, lock_backup)
            restore_path(LOCK_DIR, lock_dir_backup)

    def test_logs_command_outputs_latest_run_log(self):
        latest_backup = move_aside(LATEST_LOG_POINTER)
        legacy_backup = move_aside(LEGACY_LOG_PATH)
        unique_log = Path(f"/tmp/agent_manager_sync_{USER}_{os.getpid()}_latest.log")
        unique_backup = move_aside(unique_log)
        try:
            unique_log.write_text("hello latest log\n", encoding="utf-8")
            LATEST_LOG_POINTER.write_text(str(unique_log), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "logs"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("hello latest log", result.stdout)
        finally:
            restore_path(LATEST_LOG_POINTER, latest_backup)
            restore_path(LEGACY_LOG_PATH, legacy_backup)
            restore_path(unique_log, unique_backup)

    def test_clean_logs_command_removes_log_files(self):
        latest_backup = move_aside(LATEST_LOG_POINTER)
        legacy_backup = move_aside(LEGACY_LOG_PATH)
        unique_log = Path(f"/tmp/agent_manager_sync_{USER}_{os.getpid()}_cleanup.log")
        unique_backup = move_aside(unique_log)
        try:
            LEGACY_LOG_PATH.write_text("hello legacy log\n", encoding="utf-8")
            unique_log.write_text("hello unique log\n", encoding="utf-8")
            LATEST_LOG_POINTER.write_text(str(unique_log), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "clean-logs"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(LEGACY_LOG_PATH.exists())
            self.assertFalse(unique_log.exists())
            self.assertFalse(LATEST_LOG_POINTER.exists())
        finally:
            restore_path(LATEST_LOG_POINTER, latest_backup)
            restore_path(LEGACY_LOG_PATH, legacy_backup)
            restore_path(unique_log, unique_backup)

    def test_mcp_sync_defaults_to_config_only_mode(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root)
            agent_config_path = temp_root / "tmp-agent.json"
            (temp_root / "config" / "mcp.json").write_text(
                """
{
  "agent_paths": {
    "claude-code": "%s"
  },
  "servers": {
    "bad-mcp": {
      "url": "https://invalid.example.invalid/nope.git",
      "command": "echo",
      "args": [],
      "env": {}
    }
  }
}
                """
                % agent_config_path,
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "mcp:sync"],
                cwd=temp_root,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(agent_config_path.exists(), combined)
