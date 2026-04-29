import os
import json
import shutil
import signal
import stat
import subprocess
import tempfile
import time
import unittest
from getpass import getuser
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANAGE = ROOT / "manage.sh"
USER = getuser()
LOCK_PATH = Path(f"/tmp/agent_manager_{USER}.lock")
LOCK_DIR = Path(f"/tmp/agent_manager_{USER}.lock.d")


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
    def setUp(self):
        self.runtime_dir = Path(tempfile.mkdtemp(prefix="agent_manager_manage_test_"))
        self.lock_path = self.runtime_dir / "agent_manager.lock"
        self.lock_dir = Path(f"{self.lock_path}.d")
        self.log_prefix = self.runtime_dir / "agent_manager_sync"
        self.latest_log_pointer = Path(f"{self.log_prefix}.latest")
        self.legacy_log_path = Path(f"{self.log_prefix}.log")
        self.addCleanup(shutil.rmtree, self.runtime_dir, ignore_errors=True)

    def _managed_env(self, base_env=None):
        env = dict(base_env or os.environ.copy())
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.lock_path)
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.log_prefix)
        return env

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

    def _await_probe_child_pid(self, proc: subprocess.Popen, child_pid_file: Path, ready_file: Path, message: str):
        child_pid = None
        deadline = time.time() + 10
        while time.time() < deadline:
            if ready_file.exists() and child_pid_file.exists():
                child_pid = int(child_pid_file.read_text(encoding="utf-8").strip())
                break
            if proc.poll() is not None:
                break
            time.sleep(0.1)

        self.assertIsNotNone(child_pid, message)
        os.kill(child_pid, 0)
        return child_pid

    def _close_process_pipes(self, proc: subprocess.Popen):
        if proc.stdout is not None:
            proc.stdout.close()
        if proc.stderr is not None:
            proc.stderr.close()

    def _assert_descendant_exits_after_parent_termination(self, proc: subprocess.Popen, child_pid: int, message: str):
        proc.terminate()
        proc.wait(timeout=10)
        self._close_process_pipes(proc)

        deadline = time.time() + 3
        child_alive = True
        while time.time() < deadline:
            try:
                os.kill(child_pid, 0)
            except OSError:
                child_alive = False
                break
            time.sleep(0.1)

        self.assertFalse(child_alive, message)

    def test_doctor_does_not_remove_existing_lock(self):
        original = self.lock_path.read_text(encoding="utf-8") if self.lock_path.exists() else None
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root = Path(temp_dir) / "repo"
                temp_home = Path(temp_dir) / "home"
                temp_home.mkdir()
                shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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

                self.lock_path.write_text("999999", encoding="utf-8")
                env = self._managed_env()
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
                self.assertTrue(self.lock_path.exists())
                self.assertEqual(self.lock_path.read_text(encoding="utf-8"), "999999")
        finally:
            if original is None:
                self.lock_path.unlink(missing_ok=True)
            else:
                self.lock_path.write_text(original, encoding="utf-8")

    def test_failed_task_reports_error_and_returns_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            failing_script = temp_root / "agent-tool" / "sync_tools.py"
            failing_script.write_text(
                "#!/usr/bin/env python3\nimport sys\nsys.exit(7)\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "--offline"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("工具失败", combined)
            self.assertIn("规则分发完成", combined)
            self.assertIn("HEALTH CHECK SKIPPED", combined)
            self.assertNotIn("CERTIFIED HEALTHY", combined)

    def test_invalid_local_override_aborts_before_running_tasks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            (temp_root / "config" / "manager.local.json").write_text("{ invalid json\n", encoding="utf-8")

            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' rules >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("本地覆盖配置无效", combined)
            self.assertFalse(call_log.exists(), combined)

    def test_non_object_local_override_aborts_before_running_tasks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            (temp_root / "config" / "manager.local.json").write_text("[]\n", encoding="utf-8")

            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' rules >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("本地覆盖配置无效", combined)
            self.assertIn("manager.local.json 顶层必须是对象", combined)
            self.assertFalse(call_log.exists(), combined)

    def test_detect_fails_on_invalid_agent_definitions(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_root / "config" / "agents.json").write_text("{ invalid json\n", encoding="utf-8")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "detect"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("agents.json", combined)
            self.assertIn("无效", combined)

    def test_detect_fails_on_non_object_agent_definition_entry(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_root / "config" / "agents.json").write_text('{"claude-code": []}\n', encoding="utf-8")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "detect"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("agents.json.claude-code", combined)
            self.assertIn("必须是对象", combined)

    def test_detect_fails_on_non_object_tool_definitions(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_root / "config" / "tools.json").write_text('{"tools": []}\n', encoding="utf-8")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "detect"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("tools.json.tools", combined)
            self.assertIn("必须是对象", combined)

    def test_detect_repo_backed_tool_requires_managed_git_repo(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {"playwright-cli": {"enabled": true, "auto_update": true}},\n'
                '  "skills": {},\n'
                '  "mcp": {}\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "tools.json").write_text(
                '{\n'
                '  "tools": {\n'
                '    "playwright-cli": {\n'
                '      "url": "https://github.com/microsoft/playwright-cli.git",\n'
                '      "setup": "npm install && npx playwright install",\n'
                '      "ready_paths": ["node_modules"]\n'
                '    }\n'
                '  }\n'
                '}\n',
                encoding="utf-8",
            )
            repo_tool_dir = temp_root / "agent-tool" / "playwright-cli"
            remove_path(repo_tool_dir)
            repo_tool_dir.mkdir(parents=True, exist_ok=True)

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "detect"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertRegex(combined, r"playwright-cli\s+\(工具就绪:否\)")

    def test_detect_repo_backed_tool_requires_readiness_artifacts_after_clone(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {"playwright-cli": {"enabled": true, "auto_update": true}},\n'
                '  "skills": {},\n'
                '  "mcp": {}\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "tools.json").write_text(
                '{\n'
                '  "tools": {\n'
                '    "playwright-cli": {\n'
                '      "url": "https://github.com/microsoft/playwright-cli.git",\n'
                '      "setup": "npm install && npx playwright install",\n'
                '      "ready_paths": ["node_modules"]\n'
                '    }\n'
                '  }\n'
                '}\n',
                encoding="utf-8",
            )
            repo_tool_dir = temp_root / "agent-tool" / "playwright-cli"
            remove_path(repo_tool_dir)
            (repo_tool_dir / ".git").mkdir(parents=True, exist_ok=True)

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "detect"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertRegex(combined, r"playwright-cli\s+\(工具就绪:否\)")

    def test_doctor_stops_after_config_syntax_errors(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_root / "config" / "agents.json").write_text("{ invalid json\n", encoding="utf-8")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("agents.json: 语法错误", combined)
            self.assertNotIn("--- Agent 状态与策略一致性 ---", combined)
            self.assertNotIn("Claude Code         : 漂移", combined)

    def test_doctor_stops_after_config_shape_errors(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_root / "config" / "tools.json").write_text('{"tools": []}\n', encoding="utf-8")

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("tools.json: 格式正确", combined)
            self.assertIn("结构非法", combined)
            self.assertIn("tools.json.tools 必须是对象", combined)
            self.assertNotIn("--- Agent 状态与策略一致性 ---", combined)

    def test_doctor_fails_when_required_definition_file_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "tools.json").unlink()
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

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("tools.json: 缺失", combined)
            self.assertNotIn("环境状态完美", combined)

    def test_doctor_fails_when_rules_manifest_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "rules-targets.list").unlink()
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

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("rules-targets.list: 缺失", combined)
            self.assertNotIn("环境状态完美", combined)

    def test_doctor_warns_when_configured_skill_target_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {"missing-skill": {"enabled": true}},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {\n    "missing-skill": {"url": "https://example.invalid/repo.git", "agents": ["claude-code"]}\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("--- Skills 链接审计 ---", combined)
            self.assertIn("Claude Code", combined)
            self.assertIn("尚未建立 skills 目录", combined)
            self.assertIn("发现 1 个提示项", combined)

    def test_doctor_fails_when_enabled_tool_is_not_ready(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "playwright-cli": { "enabled": true }\n  },\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "tools.json").write_text(
                '{\n'
                '  "tools": {\n'
                '    "playwright-cli": {\n'
                '      "url": "https://github.com/microsoft/playwright-cli.git",\n'
                '      "setup": "npm install && npx playwright install",\n'
                '      "ready_paths": ["node_modules"]\n'
                '    }\n'
                '  }\n'
                '}\n',
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
            repo_tool_dir = temp_root / "agent-tool" / "playwright-cli"
            remove_path(repo_tool_dir)
            (repo_tool_dir / ".git").mkdir(parents=True, exist_ok=True)

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("--- Tools 就绪审计 ---", combined)
            self.assertRegex(combined, r"playwright-cli\s+: 未就绪")

    def test_doctor_warns_but_returns_zero_for_optional_tool_gap(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "tpm": { "enabled": true }\n  },\n  "skills": {},\n  "mcp": {}\n}\n',
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

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("--- Tools 就绪审计 ---", combined)
            self.assertRegex(combined, r"tpm\s+: 可选项未就绪")
            self.assertIn("发现 1 个提示项", combined)

    def test_doctor_warns_for_missing_generated_targets_when_agent_is_not_installed(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "superpowers": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "__missing_claude__"\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s/.claude.json"\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["claude-code"]\n    }\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {\n    "superpowers": {\n      "url": "https://example.invalid/superpowers.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_global|global|rules/dist/claude/CLAUDE.md|%s/.claude/CLAUDE.md\n" % temp_home,
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertRegex(combined, r"Claude Code: Agent 未安装，未生成 MCP 配置")
            self.assertRegex(combined, r"Claude Code\s+: Agent 未安装，未建立 skills 目录")
            self.assertRegex(combined, r"claude_global: Agent 未安装，未分发")
            self.assertIn("发现 3 个提示项", combined)

    def test_doctor_groups_warning_categories_in_summary(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "tpm": { "enabled": true }\n  },\n  "skills": {\n    "superpowers": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "__missing_claude__"\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s/.claude.json"\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["claude-code"]\n    }\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {\n    "superpowers": {\n      "url": "https://example.invalid/superpowers.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_global|global|rules/dist/claude/CLAUDE.md|%s/.claude/CLAUDE.md\n" % temp_home,
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("[ 提示项分类 ]", combined)
            self.assertIn("可选工具未就绪: 1 项", combined)
            self.assertIn("托管产物尚未生成: 3 项", combined)
            self.assertIn("发现 4 个提示项", combined)

    def test_doctor_json_output_is_machine_readable(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "tpm": { "enabled": true }\n  },\n  "skills": {\n    "superpowers": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "__missing_claude__"\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s/.claude.json"\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["claude-code"]\n    }\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {\n    "superpowers": {\n      "url": "https://example.invalid/superpowers.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_global|global|rules/dist/claude/CLAUDE.md|%s/.claude/CLAUDE.md\n" % temp_home,
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor", "--json"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            payload = json.loads(result.stdout)
            self.assertEqual(payload["status"], "warning")
            self.assertEqual(payload["summary"]["warnings"], 4)
            self.assertEqual(payload["summary"]["warning_categories"]["optional_tools"], 1)
            self.assertEqual(payload["summary"]["warning_categories"]["unmaterialized"], 3)
            self.assertEqual(payload["summary"]["errors"], 0)
            self.assertIn("tools", payload["sections"])
            self.assertIn("mcp", payload["sections"])
            self.assertIn("skills", payload["sections"])
            self.assertIn("rules", payload["sections"])

    def test_sync_preview_reports_health_caution_for_optional_tool_gaps(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "tpm": { "enabled": true }\n  },\n  "skills": {},\n  "mcp": {}\n}\n',
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
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync", "--dry-run"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=20,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("HEALTH CAUTION", combined)
            self.assertIn("可选工具未就绪 1 项", combined)
            self.assertNotIn("HEALTH WARNING", combined)

    def test_sync_preview_reports_health_caution_for_missing_generated_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "superpowers": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "__missing_claude__"\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s/.claude.json"\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["claude-code"]\n    }\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {\n    "superpowers": {\n      "url": "https://example.invalid/superpowers.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_global|global|rules/dist/claude/CLAUDE.md|%s/.claude/CLAUDE.md\n" % temp_home,
                encoding="utf-8",
            )
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync", "--dry-run"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=20,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("HEALTH CAUTION", combined)
            self.assertNotIn("HEALTH WARNING", combined)

    def test_doctor_skips_node_and_npm_when_policy_does_not_need_them(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

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

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("node: 缺失", combined)
            self.assertNotIn("npm: 缺失", combined)

    def test_doctor_skips_tool_setup_commands_when_repo_tool_is_ready_and_auto_update_disabled(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {"playwright-cli": {"enabled": true, "auto_update": false}},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "tools.json").write_text(
                '{\n'
                '  "tools": {\n'
                '    "playwright-cli": {\n'
                '      "url": "https://github.com/microsoft/playwright-cli.git",\n'
                '      "setup": "npm install && npx playwright install",\n'
                '      "ready_paths": ["node_modules"]\n'
                '    }\n'
                '  }\n'
                '}\n',
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
            tool_repo = temp_root / "agent-tool" / "playwright-cli"
            remove_path(tool_repo)
            tool_repo.mkdir(parents=True, exist_ok=True)
            (tool_repo / ".git").mkdir(exist_ok=True)
            (tool_repo / "node_modules").mkdir(exist_ok=True)

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("npm: 缺失", combined)
            self.assertNotIn("npx: 缺失", combined)
            self.assertIn("环境状态完美", combined)

    def test_doctor_empty_policy_ignores_default_mcp_skill_and_rules_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("--- MCP 注入审计 ---\n  ✗", combined)
            self.assertNotIn("--- Skills 链接审计 ---\n  ✗", combined)
            self.assertNotIn("--- Rules 内容/结构审计 ---\n  ✗", combined)
            self.assertIn("环境状态完美", combined)

    def test_sync_empty_policy_skips_optional_subengines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync", "--dry-run"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("cursor_global", combined)
            self.assertNotIn("copilot_global", combined)
            self.assertNotIn("当前策略未启用任何 Rules 目标", combined)
            self.assertNotIn("计划原子更新 JSON", combined)
            self.assertNotIn("准备下载到托管缓存", combined)
            self.assertNotIn("正在安装 playwright-cli", combined)

    def test_doctor_snapshot_statistics_only_count_managed_backups(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            managed_target = temp_home / ".claude.json"
            managed_target.write_text("{}", encoding="utf-8")
            managed_backup = temp_home / ".claude.json.bak.20260318"
            unrelated_backup = temp_home / "Documents" / "report.txt.bak.20260318"
            unrelated_backup.parent.mkdir(parents=True)
            managed_backup.write_text("managed\n", encoding="utf-8")
            unrelated_backup.write_text("private\n", encoding="utf-8")

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {"bad-mcp": {"enabled": true}}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % managed_target,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "doctor"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("有效历史快照: 1 个", combined)
            self.assertNotIn("有效历史快照: 2 个", combined)

    def test_ui_command_launches_interactive_menu(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "ui"],
                cwd=temp_root,
                env=self._managed_env(),
                input="7\n",
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("Agent Manager 交互式配置中心", combined)
            self.assertIn("7. 退出", combined)

    def test_ui_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "ui-child.pid"
            ready_file = Path(temp_dir) / "ui-ready"

            self._write_stub(
                temp_root / "src" / "interactive.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "from pathlib import Path\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "Path(os.environ['CHILD_PID_FILE']).write_text(str(child.pid), encoding='utf-8')\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                ),
            )

            env = self._managed_env()
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "ui"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                child_pid = self._await_probe_child_pid(
                    proc,
                    child_pid_file,
                    ready_file,
                    "ui 未按预期启动子进程",
                )
                self._assert_descendant_exits_after_parent_termination(
                    proc,
                    child_pid,
                    f"ui 退出后子进程仍存活: {child_pid}",
                )
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                self._close_process_pipes(proc)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_manage_without_command_shows_help_and_does_not_run_pipeline(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"

            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._append_log_stub(temp_root / "agent-terminal" / "sync_terminal.py", "terminal", call_log, "True")
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' rules >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh")],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
                timeout=5,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("Agent Manager 帮助手册", combined)
            self.assertIn("-f, --force-refresh", combined)
            self.assertIn("--wait-agent-lifecycle", combined)
            self.assertFalse(call_log.exists(), combined)

    def test_make_default_help_lists_cc_bridge_runtime_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            result = subprocess.run(
                ["make"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
                timeout=5,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("cc-bridge-install", combined)
            self.assertIn("cc-bridge-start", combined)
            self.assertIn("cc-bridge-stop", combined)
            self.assertIn("cc-bridge-status", combined)

    def test_sync_refreshes_skills_and_mcp_without_agent_or_tool_lifecycle(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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
                "'--offline' not in sys.argv and '--dry-run' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--offline' not in sys.argv and '--prepare-runtime' in sys.argv",
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                env=self._managed_env(),
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

    def test_sync_skips_terminal_when_terminal_policy_disabled(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"

            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {},\n'
                '  "skills": {},\n'
                '  "mcp": {},\n'
                '  "terminal": { "enabled": false }\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._append_log_stub(temp_root / "agent-terminal" / "sync_terminal.py", "terminal", call_log, "True")
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "sync"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(any(line.startswith("skills ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") for line in log_lines), log_lines)
            self.assertFalse(any(line.startswith("terminal ") for line in log_lines), log_lines)

    def test_rules_sync_uses_built_rule_layout(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_home / ".codex").mkdir(parents=True)

            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {},\n'
                '  "skills": {\n'
                '    "openai-frontend-skill": { "enabled": true }\n'
                '  },\n'
                '  "mcp": {}\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n'
                '  "repositories": {\n'
                '    "openai-frontend-skill": {\n'
                '      "url": "https://example.invalid/openai-frontend-skill.git",\n'
                '      "agents": ["codex"]\n'
                '    }\n'
                '  },\n'
                '  "agents": {\n'
                '    "codex": "%s/.codex/skills"\n'
                '  }\n'
                '}\n' % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "codex_global|global|rules/dist/codex/AGENTS.md|%s/.codex/AGENTS.md\n" % temp_home,
                encoding="utf-8",
            )
            (temp_home / ".codex").mkdir(parents=True, exist_ok=True)
            legacy_path = temp_home / ".codex" / "rules" / "global.md"
            legacy_path.parent.mkdir(parents=True, exist_ok=True)
            legacy_path.write_text("legacy codex rule\n", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "rules:sync"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((temp_home / ".codex" / "AGENTS.md").exists(), result.stdout + result.stderr)
            self.assertFalse(legacy_path.exists(), result.stdout + result.stderr)
            self.assertEqual(len(list(legacy_path.parent.glob("global.md.bak.*"))), 1, result.stdout + result.stderr)

    def test_rules_sync_updates_all_active_rule_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            (temp_home / ".claude").mkdir(parents=True, exist_ok=True)
            (temp_home / ".codex").mkdir(parents=True, exist_ok=True)

            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {\n'
                '    "claude-code": { "enabled": true }\n'
                '  },\n'
                '  "tools": {},\n'
                '  "skills": {\n'
                '    "openai-frontend-skill": { "enabled": true }\n'
                '  },\n'
                '  "mcp": {}\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n'
                '  "repositories": {\n'
                '    "openai-frontend-skill": {\n'
                '      "url": "https://example.invalid/openai-frontend-skill.git",\n'
                '      "agents": ["codex"]\n'
                '    }\n'
                '  },\n'
                '  "agents": {\n'
                '    "codex": "%s/.codex/skills"\n'
                '  }\n'
                '}\n' % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                (
                    "claude_global|global|rules/dist/claude/CLAUDE.md|%s/.claude/CLAUDE.md\n"
                    "codex_global|global|rules/dist/codex/AGENTS.md|%s/.codex/AGENTS.md\n"
                )
                % (temp_home, temp_home),
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "rules:sync"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((temp_home / ".claude" / "CLAUDE.md").exists(), combined)
            self.assertTrue((temp_home / ".codex" / "AGENTS.md").exists(), combined)

    def test_rules_sync_materializes_repo_root_targets_without_active_agent_enablement(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "cursor_global|global|rules/dist/cursor/agent-manager.mdc|{{REPO_ROOT}}/.cursor/rules/agent-manager.mdc\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "rules:sync"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((temp_root / ".cursor" / "rules" / "agent-manager.mdc").exists(), combined)

    def test_skills_upgrade_dispatches_upgrade_script(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"

            self._write_stub(
                temp_root / "agent-skill" / "upgrade_skills.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys\n"
                    f"open({str(call_log)!r}, 'a', encoding='utf-8').write('upgrade ' + ' '.join(sys.argv[1:]) + '\\n')\n"
                    "sys.exit(0 if '--config' in sys.argv and '--def-file' in sys.argv and '--dry-run' in sys.argv else 92)\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "skills:upgrade"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(log_lines), 1, log_lines)
            self.assertTrue(log_lines[0].startswith("upgrade "), log_lines)
            self.assertIn("--config", log_lines[0])
            self.assertIn("--def-file", log_lines[0])
            self.assertIn("--dry-run", log_lines[0])

    def test_skills_upgrade_forwards_requested_repositories(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"

            self._write_stub(
                temp_root / "agent-skill" / "upgrade_skills.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys\n"
                    f"open({str(call_log)!r}, 'a', encoding='utf-8').write('upgrade ' + ' '.join(sys.argv[1:]) + '\\n')\n"
                    "expected = ['--repo', 'repo-a', '--repo', 'repo-b']\n"
                    "sys.exit(0 if all(item in sys.argv for item in expected) else 93)\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "skills:upgrade", "repo-a", "repo-b"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(log_lines), 1, log_lines)
            self.assertIn("--repo repo-a", log_lines[0])
            self.assertIn("--repo repo-b", log_lines[0])

    def test_update_refreshes_resources_without_agent_install_and_waits_for_agent_lifecycle(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertIn("init-secrets", log_lines, log_lines)
            self.assertTrue(
                any(
                    line.startswith("agents ")
                    and "--no-install" in line
                    and "--async-actions" in line
                    and "--wait-async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )
            self.assertTrue(any(line.startswith("tools ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("rules ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("skills ") for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") and "--prepare-runtime" in line for line in log_lines), log_lines)

    def test_update_force_refresh_propagates_to_tools_skills_and_mcp(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--force-refresh' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "tools",
                call_log,
                "'--force-refresh' in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "skills",
                call_log,
                "'--force-refresh' in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--prepare-runtime' in sys.argv and '--force-refresh' in sys.argv",
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                [str(temp_root / "manage.sh"), "--force-refresh", "update"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    line.startswith("agents ")
                    and "--force-refresh" not in line
                    and "--async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )
            self.assertTrue(any(line.startswith("tools ") and "--force-refresh" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("skills ") and "--force-refresh" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") and "--force-refresh" in line for line in log_lines), log_lines)

    def test_update_short_force_refresh_flag_propagates_to_tools_skills_and_mcp(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--force-refresh' not in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-tool" / "sync_tools.py",
                "tools",
                call_log,
                "'--force-refresh' in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "skills",
                call_log,
                "'--force-refresh' in sys.argv",
            )
            self._append_log_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                "mcp",
                call_log,
                "'--prepare-runtime' in sys.argv and '--force-refresh' in sys.argv",
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                [str(temp_root / "manage.sh"), "-f", "update"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    line.startswith("agents ")
                    and "--force-refresh" not in line
                    and "--async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )
            self.assertTrue(any(line.startswith("tools ") and "--force-refresh" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("skills ") and "--force-refresh" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") and "--force-refresh" in line for line in log_lines), log_lines)

    def test_bootstrap_allows_agent_install(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    line.startswith("agents ")
                    and "--no-install" not in line
                    and "--async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )

    def test_update_wait_agent_lifecycle_forwards_wait_flag(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--async-actions' in sys.argv and '--wait-async-actions' in sys.argv and '--no-install' in sys.argv",
            )
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                [str(temp_root / "manage.sh"), "--wait-agent-lifecycle", "update"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    line.startswith("agents ")
                    and "--wait-async-actions" in line
                    and "--async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )

    def test_self_update_only_runs_repo_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"git $*\" >> {str(call_log)!r}\n"
                    "if [ \"$3\" = \"status\" ] && [ \"$4\" = \"--porcelain\" ]; then\n"
                    "  exit 0\n"
                    "fi\n"
                    "echo 'Already up to date'\n"
                    "exit 0\n"
                ),
            )
            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "False")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "False")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "False")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "False")
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 99\n")
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
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
            self.assertEqual(len(log_lines), 2, log_lines)
            self.assertIn(f"git -C {temp_root.resolve()} status --porcelain", log_lines[0])
            self.assertIn(f"git -C {temp_root.resolve()} pull --ff-only", log_lines[1])

    def test_self_update_status_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "self-update-status-child.pid"
            ready_file = Path(temp_dir) / "self-update-status-ready"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()

            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    "if [ \"$1\" = \"-C\" ]; then\n"
                    "  shift 2\n"
                    "fi\n"
                    "if [ \"$1\" = \"status\" ] && [ \"$2\" = \"--porcelain\" ]; then\n"
                    "  python3 - <<'PY'\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "from pathlib import Path\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "Path(os.environ['CHILD_PID_FILE']).write_text(str(child.pid), encoding='utf-8')\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                    "PY\n"
                    "fi\n"
                    "echo \"unexpected git args: $*\" >&2\n"
                    "exit 2\n"
                ),
            )

            env = self._managed_env()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "self-update"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                child_pid = self._await_probe_child_pid(
                    proc,
                    child_pid_file,
                    ready_file,
                    "self-update status 未按预期启动子进程",
                )
                self._assert_descendant_exits_after_parent_termination(
                    proc,
                    child_pid,
                    f"self-update status 退出后子进程仍存活: {child_pid}",
                )
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                self._close_process_pipes(proc)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_update_pipeline_pulls_secrets_with_ff_only(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            (temp_root / "secrets" / ".git").mkdir(parents=True, exist_ok=True)

            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"git $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._write_stub(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh", "#!/usr/bin/env bash\nexit 0\n")
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(line == f"git -C {temp_root.resolve() / 'secrets'} pull --ff-only" for line in log_lines),
                log_lines,
            )

    def test_update_pipeline_secrets_pull_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "secrets-pull-child.pid"
            ready_file = Path(temp_dir) / "secrets-pull-ready"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            (temp_root / "secrets" / ".git").mkdir(parents=True, exist_ok=True)

            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    "if [ \"$1\" = \"-C\" ]; then\n"
                    "  shift 2\n"
                    "fi\n"
                    "if [ \"$1\" = \"pull\" ] && [ \"$2\" = \"--ff-only\" ]; then\n"
                    "  python3 - <<'PY'\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "from pathlib import Path\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "Path(os.environ['CHILD_PID_FILE']).write_text(str(child.pid), encoding='utf-8')\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                    "PY\n"
                    "fi\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                "#!/usr/bin/env bash\nexit 0\n",
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                child_pid = self._await_probe_child_pid(
                    proc,
                    child_pid_file,
                    ready_file,
                    "update secrets pull 未按预期启动子进程",
                )
                self._assert_descendant_exits_after_parent_termination(
                    proc,
                    child_pid,
                    f"update secrets pull 退出后子进程仍存活: {child_pid}",
                )
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                self._close_process_pipes(proc)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_self_update_skips_dirty_worktree(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"git $*\" >> {str(call_log)!r}\n"
                    "if [ \"$3\" = \"status\" ] && [ \"$4\" = \"--porcelain\" ]; then\n"
                    "  printf ' M manage.sh\\n'\n"
                    "  exit 0\n"
                    "fi\n"
                    "echo 'pull should not run'\n"
                    "exit 0\n"
                ),
            )

            env = self._managed_env()
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
            self.assertIn("跳过自更新以保护工作区", result.stdout + result.stderr)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(log_lines), 1, log_lines)
            self.assertIn(f"git -C {temp_root.resolve()} status --porcelain", log_lines[0])

    def test_self_update_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "self-update-child.pid"
            ready_file = Path(temp_dir) / "self-update-ready"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()

            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    "if [ \"$1\" = \"-C\" ]; then\n"
                    "  shift 2\n"
                    "fi\n"
                    "if [ \"$1\" = \"status\" ] && [ \"$2\" = \"--porcelain\" ]; then\n"
                    "  exit 0\n"
                    "fi\n"
                    "if [ \"$1\" = \"pull\" ] && [ \"$2\" = \"--ff-only\" ]; then\n"
                    "  python3 - <<'PY'\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "from pathlib import Path\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "Path(os.environ['CHILD_PID_FILE']).write_text(str(child.pid), encoding='utf-8')\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                    "PY\n"
                    "fi\n"
                    "echo \"unexpected git args: $*\" >&2\n"
                    "exit 2\n"
                ),
            )

            env = self._managed_env()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "self-update"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                child_pid = self._await_probe_child_pid(
                    proc,
                    child_pid_file,
                    ready_file,
                    "self-update 未按预期启动子进程",
                )
                self._assert_descendant_exits_after_parent_termination(
                    proc,
                    child_pid,
                    f"self-update 退出后子进程仍存活: {child_pid}",
                )
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                self._close_process_pipes(proc)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_update_init_secrets_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "init-secrets-child.pid"
            ready_file = Path(temp_dir) / "init-secrets-ready"
            remove_path(temp_root / "secrets")

            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    "python3 - <<'PY'\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "from pathlib import Path\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "Path(os.environ['CHILD_PID_FILE']).write_text(str(child.pid), encoding='utf-8')\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                    "PY\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                "#!/usr/bin/env bash\nexit 0\n",
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                child_pid = self._await_probe_child_pid(
                    proc,
                    child_pid_file,
                    ready_file,
                    "init-secrets 未按预期启动子进程",
                )
                self._assert_descendant_exits_after_parent_termination(
                    proc,
                    child_pid,
                    f"init-secrets 退出后子进程仍存活: {child_pid}",
                )
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                self._close_process_pipes(proc)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_fetch_models_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "fetch-models-child.pid"
            ready_file = Path(temp_dir) / "fetch-models-ready"

            self._write_stub(
                temp_root / "scripts" / "fetch_models.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "from pathlib import Path\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "Path(os.environ['CHILD_PID_FILE']).write_text(str(child.pid), encoding='utf-8')\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                ),
            )

            env = self._managed_env()
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "fetch-models"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                child_pid = self._await_probe_child_pid(
                    proc,
                    child_pid_file,
                    ready_file,
                    "fetch-models 未按预期启动子进程",
                )
                self._assert_descendant_exits_after_parent_termination(
                    proc,
                    child_pid,
                    f"fetch-models 退出后子进程仍存活: {child_pid}",
                )
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                self._close_process_pipes(proc)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_plan_runs_bootstrap_pipeline_in_dry_run_mode(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                env=self._managed_env(),
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
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            remove_path(temp_root / "secrets")

            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"git $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "'--dry-run' in sys.argv")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "'--dry-run' in sys.argv")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "'--dry-run' in sys.argv")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "'--dry-run' in sys.argv")
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
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 17\n"
                ),
            )

            env = self._managed_env()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("空转模式，跳过 secrets 检查", combined)
            self.assertNotIn("正在克隆凭证仓库", combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertFalse(any(line == "init-secrets" for line in log_lines), log_lines)
            self.assertFalse(
                any(line == f"git -C {temp_root.resolve() / 'secrets'} pull --ff-only" for line in log_lines),
                log_lines,
            )

    def test_dry_run_offline_combination_respects_offline_flag(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"
            fake_bin = temp_root / "fake-bin"
            fake_bin.mkdir()
            remove_path(temp_root / "secrets")

            self._write_stub(
                fake_bin / "git",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"git $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "scripts" / "init-secrets.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                    "exit 17\n"
                ),
            )

            env = self._managed_env()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "--offline"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("离线模式，跳过 secrets 检查", combined)
            self.assertNotIn("正在克隆凭证仓库", combined)
            log_lines = call_log.read_text(encoding="utf-8").splitlines()
            self.assertFalse(any(line == "init-secrets" for line in log_lines), log_lines)
            self.assertFalse(
                any(line == f"git -C {temp_root.resolve() / 'secrets'} pull --ff-only" for line in log_lines),
                log_lines,
            )

    def test_offline_is_forwarded_to_networked_subtasks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

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
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)

    def test_progress_output_preserves_utf8_log_lines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

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
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("\ufffd", combined)

    def test_progress_output_reports_elapsed_time_for_long_running_task(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

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
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertRegex(combined, r"已运行 [0-9]+s")

    def test_progress_output_does_not_reuse_previous_task_log_lines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

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
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotRegex(combined, r"同步全局工具: .*Claude Code")

    def test_progress_output_does_not_redraw_identical_message_every_poll(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

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
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--offline"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertEqual(result.returncode, 0, combined)
            self.assertLessEqual(combined.count("对齐 Agent 生命周期: [AGENT: Claude Code] 状态一致"), 2, combined)

    def test_atomic_lock_directory_blocks_new_run(self):
        lock_backup = move_aside(self.lock_path)
        lock_dir_backup = move_aside(self.lock_dir)
        holder = subprocess.Popen(["sleep", "60"])
        try:
            self.lock_dir.mkdir(parents=True)
            (self.lock_dir / "pid").write_text(str(holder.pid), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "--dry-run", "--offline"],
                cwd=ROOT,
                env=self._managed_env(),
                capture_output=True,
                check=False,
            )

            combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("另一个管理任务", combined)
        finally:
            holder.terminate()
            holder.wait(timeout=5)
            restore_path(self.lock_path, lock_backup)
            restore_path(self.lock_dir, lock_dir_backup)

    def test_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "child.pid"
            ready_file = Path(temp_dir) / "ready"

            self._write_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "with open(os.environ['CHILD_PID_FILE'], 'w', encoding='utf-8') as fh:\n"
                    "    fh.write(str(child.pid))\n"
                    "Path = __import__('pathlib').Path\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
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
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                "#!/usr/bin/env bash\nexit 0\n",
            )
            self._write_stub(
                temp_root / "agent-terminal" / "sync_terminal.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

            env = self._managed_env()
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "--offline", "update"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                deadline = time.time() + 10
                while time.time() < deadline:
                    if ready_file.exists() and child_pid_file.exists():
                        child_pid = int(child_pid_file.read_text(encoding="utf-8").strip())
                        break
                    if proc.poll() is not None:
                        break
                    time.sleep(0.1)

                self.assertIsNotNone(child_pid, "生命周期任务未按预期启动子进程")
                os.kill(child_pid, 0)

                proc.terminate()
                proc.wait(timeout=10)
                proc.communicate(timeout=1)

                deadline = time.time() + 3
                child_alive = True
                while time.time() < deadline:
                    try:
                        os.kill(child_pid, 0)
                    except OSError:
                        child_alive = False
                        break
                    time.sleep(0.1)

                self.assertFalse(child_alive, f"manage.sh 退出后子进程仍存活: {child_pid}")
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.communicate(timeout=5)
                else:
                    proc.stdout.close()
                    proc.stderr.close()
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass

    def test_unlock_removes_atomic_lock_artifacts(self):
        lock_backup = move_aside(self.lock_path)
        lock_dir_backup = move_aside(self.lock_dir)
        try:
            self.lock_path.write_text("999999", encoding="utf-8")
            self.lock_dir.mkdir(parents=True)
            (self.lock_dir / "pid").write_text("999999", encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "unlock"],
                cwd=ROOT,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(self.lock_path.exists())
            self.assertFalse(self.lock_dir.exists())
        finally:
            restore_path(self.lock_path, lock_backup)
            restore_path(self.lock_dir, lock_dir_backup)

    def test_unlock_dry_run_preserves_atomic_lock_artifacts(self):
        lock_backup = move_aside(self.lock_path)
        lock_dir_backup = move_aside(self.lock_dir)
        try:
            self.lock_path.write_text("999999", encoding="utf-8")
            self.lock_dir.mkdir(parents=True)
            (self.lock_dir / "pid").write_text("999999", encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "--dry-run", "unlock"],
                cwd=ROOT,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )
            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("计划清理原子锁目录与遗留锁文件", combined)
            self.assertTrue(self.lock_path.exists(), combined)
            self.assertTrue(self.lock_dir.exists(), combined)
        finally:
            restore_path(self.lock_path, lock_backup)
            restore_path(self.lock_dir, lock_dir_backup)

    def test_custom_lock_env_ignores_default_global_lock(self):
        global_lock_backup = move_aside(LOCK_PATH)
        global_lock_dir_backup = move_aside(LOCK_DIR)
        holder = subprocess.Popen(["sleep", "60"])
        try:
            LOCK_DIR.mkdir(parents=True)
            (LOCK_DIR / "pid").write_text(str(holder.pid), encoding="utf-8")

            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root = Path(temp_dir) / "repo"
                shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
                self._write_stub(
                    temp_root / "agent-lifecycle" / "sync_agents.py",
                    "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                self._write_stub(
                    temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                    "#!/usr/bin/env bash\nexit 0\n",
                )
                self._write_stub(
                    temp_root / "agent-terminal" / "sync_terminal.py",
                    "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
                )

                result = subprocess.run(
                    [str(temp_root / "manage.sh"), "--dry-run", "--offline"],
                    cwd=temp_root,
                    env=self._managed_env(),
                    capture_output=True,
                    check=False,
                )

                combined = (result.stdout + result.stderr).decode("utf-8", errors="replace")
                self.assertEqual(result.returncode, 0, combined)
                self.assertNotIn("另一个管理任务", combined)
        finally:
            holder.terminate()
            holder.wait(timeout=5)
            restore_path(LOCK_PATH, global_lock_backup)
            restore_path(LOCK_DIR, global_lock_dir_backup)

    def test_logs_command_outputs_latest_run_log(self):
        latest_backup = move_aside(self.latest_log_pointer)
        legacy_backup = move_aside(self.legacy_log_path)
        unique_log = self.runtime_dir / "latest.log"
        unique_backup = move_aside(unique_log)
        try:
            unique_log.write_text("hello latest log\n", encoding="utf-8")
            self.latest_log_pointer.write_text(str(unique_log), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "logs"],
                cwd=ROOT,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("hello latest log", result.stdout)
        finally:
            restore_path(self.latest_log_pointer, latest_backup)
            restore_path(self.legacy_log_path, legacy_backup)
            restore_path(unique_log, unique_backup)

    def test_clean_logs_command_removes_log_files(self):
        latest_backup = move_aside(self.latest_log_pointer)
        legacy_backup = move_aside(self.legacy_log_path)
        unique_log = Path(f"{self.log_prefix}_cleanup.log")
        unique_backup = move_aside(unique_log)
        try:
            self.legacy_log_path.write_text("hello legacy log\n", encoding="utf-8")
            unique_log.write_text("hello unique log\n", encoding="utf-8")
            self.latest_log_pointer.write_text(str(unique_log), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "clean-logs"],
                cwd=ROOT,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(self.legacy_log_path.exists())
            self.assertFalse(unique_log.exists())
            self.assertFalse(self.latest_log_pointer.exists())
        finally:
            restore_path(self.latest_log_pointer, latest_backup)
            restore_path(self.legacy_log_path, legacy_backup)
            restore_path(unique_log, unique_backup)

    def test_clean_logs_dry_run_preserves_log_files(self):
        latest_backup = move_aside(self.latest_log_pointer)
        legacy_backup = move_aside(self.legacy_log_path)
        unique_log = Path(f"{self.log_prefix}_cleanup.log")
        unique_backup = move_aside(unique_log)
        try:
            self.legacy_log_path.write_text("hello legacy log\n", encoding="utf-8")
            unique_log.write_text("hello unique log\n", encoding="utf-8")
            self.latest_log_pointer.write_text(str(unique_log), encoding="utf-8")
            result = subprocess.run(
                [str(MANAGE), "--dry-run", "clean-logs"],
                cwd=ROOT,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )
            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("计划清理 latest 指针与所有按次运行日志", combined)
            self.assertTrue(self.legacy_log_path.exists(), combined)
            self.assertTrue(unique_log.exists(), combined)
            self.assertTrue(self.latest_log_pointer.exists(), combined)
        finally:
            restore_path(self.latest_log_pointer, latest_backup)
            restore_path(self.legacy_log_path, legacy_backup)
            restore_path(unique_log, unique_backup)

    def test_pipeline_logs_use_owner_only_permissions(self):
        latest_backup = move_aside(self.latest_log_pointer)
        legacy_backup = move_aside(self.legacy_log_path)
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root = Path(temp_dir) / "repo"
                shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

                self._write_stub(
                    temp_root / "agent-lifecycle" / "sync_agents.py",
                    "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
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
                self._write_stub(
                    temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                    "#!/usr/bin/env bash\nexit 0\n",
                )
                self._write_stub(
                    temp_root / "agent-terminal" / "sync_terminal.py",
                    "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
                )

                result = subprocess.run(
                    [str(temp_root / "manage.sh"), "--offline", "update"],
                    cwd=temp_root,
                    env=self._managed_env(),
                    capture_output=True,
                    text=True,
                    check=False,
                )

                combined = result.stdout + result.stderr
                self.assertEqual(result.returncode, 0, combined)
                self.assertTrue(self.latest_log_pointer.exists(), combined)

                latest_log = Path(self.latest_log_pointer.read_text(encoding="utf-8").strip())
                self.assertTrue(latest_log.exists(), combined)
                self.assertEqual(stat.S_IMODE(latest_log.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(self.latest_log_pointer.stat().st_mode), 0o600)
        finally:
            restore_path(self.latest_log_pointer, latest_backup)
            restore_path(self.legacy_log_path, legacy_backup)

    def test_backups_restore_restores_directory_backup_with_spaces(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            backup_dir = temp_home / "Library" / "Application Support" / "Agent Rules.bak.20260318"
            target_dir = temp_home / "Library" / "Application Support" / "Agent Rules"
            backup_dir.mkdir(parents=True)
            (backup_dir / "rule.md").write_text("# restored\n", encoding="utf-8")

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude|copy|global|%s\n" % target_dir,
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "backups:restore"],
                cwd=temp_root,
                env=env,
                input="1\n",
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue((target_dir / "rule.md").exists(), combined)
            self.assertEqual((target_dir / "rule.md").read_text(encoding="utf-8"), "# restored\n")

    def test_backups_restore_restores_plain_bak_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            target_file = temp_home / ".claude.json"
            backup_file = temp_home / ".claude.json.bak"
            backup_file.write_text('{"restored": true}\n', encoding="utf-8")
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % target_file,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "backups:restore"],
                cwd=temp_root,
                env=env,
                input="1\n",
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertEqual(target_file.read_text(encoding="utf-8"), '{"restored": true}\n')

    def test_backups_restore_dry_run_does_not_modify_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            target_file = temp_home / ".claude.json"
            target_file.write_text('{"current": true}\n', encoding="utf-8")
            backup_file = temp_home / ".claude.json.bak"
            backup_file.write_text('{"restored": true}\n', encoding="utf-8")
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % target_file,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "backups:restore"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("以下备份可用于恢复", combined)
            self.assertIn(str(backup_file), combined)
            self.assertEqual(target_file.read_text(encoding="utf-8"), '{"current": true}\n')

    def test_backups_restore_respects_global_lock(self):
        lock_backup = move_aside(self.lock_path)
        lock_dir_backup = move_aside(self.lock_dir)
        holder = subprocess.Popen(["sleep", "60"])
        try:
            self.lock_dir.mkdir(parents=True)
            (self.lock_dir / "pid").write_text(str(holder.pid), encoding="utf-8")
            with tempfile.TemporaryDirectory() as temp_dir:
                temp_home = Path(temp_dir) / "home"
                backup_file = temp_home / "config.json.bak.20260318"
                backup_file.parent.mkdir(parents=True)
                backup_file.write_text("{}", encoding="utf-8")
                env = self._managed_env()
                env["HOME"] = str(temp_home)

                result = subprocess.run(
                    [str(MANAGE), "backups:restore"],
                    cwd=ROOT,
                    env=env,
                    input="1\n",
                    capture_output=True,
                    text=True,
                    check=False,
                )

                combined = result.stdout + result.stderr
                self.assertNotEqual(result.returncode, 0, combined)
                self.assertIn("另一个管理任务", combined)
        finally:
            holder.terminate()
            holder.wait(timeout=5)
            restore_path(self.lock_path, lock_backup)
            restore_path(self.lock_dir, lock_dir_backup)

    def test_backups_restore_returns_non_zero_when_restore_fails(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            target_file = temp_home / ".claude.json"
            backup_file = temp_home / ".claude.json.bak"
            backup_file.write_text('{"restored": true}\n', encoding="utf-8")
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % target_file,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)

            temp_home.chmod(0o500)
            try:
                result = subprocess.run(
                    [str(temp_root / "manage.sh"), "backups:restore"],
                    cwd=temp_root,
                    env=env,
                    input="1\n",
                    capture_output=True,
                    text=True,
                    check=False,
                )
            finally:
                temp_home.chmod(0o700)

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("恢复失败", combined)
            self.assertFalse(target_file.exists(), combined)

    def test_backups_list_ignores_unmanaged_home_backups(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            managed_target = temp_home / ".claude.json"
            managed_backup = temp_home / ".claude.json.bak.20260318"
            unrelated_backup = temp_home / "Documents" / "report.txt.bak.20260318"
            unrelated_backup.parent.mkdir(parents=True)
            managed_backup.write_text("{}", encoding="utf-8")
            unrelated_backup.write_text("private\n", encoding="utf-8")

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % managed_target,
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "backups:list"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn(str(managed_backup), combined)
            self.assertNotIn(str(unrelated_backup), combined)

    def test_backups_list_orders_newest_entries_first(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            (temp_home / "A").mkdir(parents=True)
            (temp_home / "Z").mkdir(parents=True)

            older_target = temp_home / "Z" / "zeta.json"
            newer_target = temp_home / "A" / "alpha.json"
            older_backup = temp_home / "Z" / "zeta.json.bak.202603181200"
            newer_backup = temp_home / "A" / "alpha.json.bak.202603201200"
            older_backup.write_text("older\n", encoding="utf-8")
            newer_backup.write_text("newer\n", encoding="utf-8")
            os.utime(older_backup, (1710763200, 1710763200))
            os.utime(newer_backup, (1710936000, 1710936000))

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s",\n    "gemini-cli": "%s"\n  },\n  "servers": {}\n}\n'
                % (older_target, newer_target),
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "backups:list"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            lines = [line for line in result.stdout.splitlines() if line.strip()]
            self.assertGreaterEqual(len(lines), 2, combined)
            self.assertEqual(lines[0], str(newer_backup), combined)
            self.assertEqual(lines[1], str(older_backup), combined)

    def test_backups_list_fails_when_rules_manifest_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "rules-targets.list").unlink()

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "backups:list"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("rules-targets.list 缺失", combined)

    def test_backups_restore_fails_when_managed_scope_enumeration_fails(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()
            (temp_root / "config" / "rules-targets.list").unlink()

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "backups:restore"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("rules-targets.list 缺失", combined)

    def test_cleanup_only_removes_target_scoped_temp_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            target_dir = temp_home / ".claude"
            target_dir.mkdir(parents=True)
            managed_target = target_dir / "settings.json"
            managed_target.write_text("{}", encoding="utf-8")
            managed_tmp = target_dir / "settings.json.tmp_123"
            unrelated_tmp = target_dir / "other.tmp_keep"
            managed_tmp.write_text("tmp", encoding="utf-8")
            unrelated_tmp.write_text("keep", encoding="utf-8")

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % managed_target,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(managed_tmp.exists(), result.stdout + result.stderr)
            self.assertTrue(unrelated_tmp.exists(), result.stdout + result.stderr)

    def test_cleanup_removes_plain_bak_for_managed_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            managed_target = temp_home / ".claude.json"
            plain_backup = temp_home / ".claude.json.bak"
            plain_backup.write_text("{}", encoding="utf-8")

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % managed_target,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(plain_backup.exists(), result.stdout + result.stderr)

    def test_cleanup_removes_directory_backups_for_managed_rules_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            rule_target = temp_home / ".claude" / "rules" / "modules"
            rule_backup = temp_home / ".claude" / "rules" / "modules.bak.20260318"
            rule_backup.mkdir(parents=True)
            (rule_backup / "style.md").write_text("# backup\n", encoding="utf-8")

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "claude_modules|modules|rules/dist/modules|%s\n" % rule_target,
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(rule_backup.exists(), result.stdout + result.stderr)

    def test_cleanup_expands_repo_root_rule_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            repo_native_backup = temp_root / ".kimi" / "AGENTS.md.bak.20260423"
            repo_native_backup.parent.mkdir(parents=True, exist_ok=True)
            repo_native_backup.write_text("old repo-native backup\n", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(repo_native_backup.exists(), result.stdout + result.stderr)

    def test_cleanup_removes_legacy_rule_backups(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            legacy_backups = [
                temp_home / ".codex" / "rules" / "global.md.bak.20260329",
                temp_home / ".claude" / "rules" / "modules.bak.20260329",
                temp_home / ".config" / "opencode" / "rules" / "global.md.bak.20260329",
                temp_home / ".config" / "opencode" / "oh-my-opencode.jsonc.bak",
            ]
            for backup in legacy_backups:
                backup.parent.mkdir(parents=True, exist_ok=True)
                backup.write_text("old managed backup\n", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            for backup in legacy_backups:
                self.assertFalse(backup.exists(), result.stdout + result.stderr)

    def test_cleanup_removes_gemini_invalid_json_tmp_backups(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            invalid_backup = (
                temp_home
                / ".gemini"
                / "tmp"
                / "session"
                / "logs.json.invalid_json.1766550378705.bak"
            )
            invalid_backup.parent.mkdir(parents=True)
            invalid_backup.write_text("{invalid\n", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(invalid_backup.exists(), result.stdout + result.stderr)

    def test_cleanup_dry_run_preserves_managed_artifacts(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            temp_home.mkdir()

            target_dir = temp_home / ".claude"
            target_dir.mkdir(parents=True)
            managed_target = target_dir / "settings.json"
            managed_target.write_text("{}", encoding="utf-8")
            managed_tmp = target_dir / "settings.json.tmp_123"
            managed_backup = target_dir / "settings.json.bak.20260318"
            managed_tmp.write_text("tmp", encoding="utf-8")
            managed_backup.write_text("bak", encoding="utf-8")
            latest_log = temp_home / "latest.log"
            latest_log.write_text("log\n", encoding="utf-8")

            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "claude-code": "%s"\n  },\n  "servers": {}\n}\n'
                % managed_target,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            env["AGENT_MANAGER_LOG_PREFIX"] = str(temp_home / "agent_manager_sync_test")
            env["AGENT_MANAGER_LATEST_LOG_POINTER"] = str(latest_log)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "cleanup"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("计划清理以下受管备份、临时文件与运行日志", combined)
            self.assertTrue(managed_tmp.exists(), combined)
            self.assertTrue(managed_backup.exists(), combined)
            self.assertTrue(latest_log.exists(), combined)

    def test_mcp_sync_prepares_runtime_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))
            call_log = temp_root / "calls.log"

            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys\n"
                    "from pathlib import Path\n"
                    f"Path({str(call_log)!r}).write_text(' '.join(sys.argv[1:]) + '\\n', encoding='utf-8')\n"
                    "sys.exit(0 if '--prepare-runtime' in sys.argv and '--offline' not in sys.argv else 91)\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "mcp:sync"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            logged = call_log.read_text(encoding="utf-8")
            self.assertIn("--prepare-runtime", logged)
            self.assertNotIn("--offline", logged)

    def test_atomic_mcp_sync_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns('repositories'))

            child_pid_file = Path(temp_dir) / "mcp-child.pid"
            ready_file = Path(temp_dir) / "mcp-ready"

            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import os\n"
                    "import signal\n"
                    "import subprocess\n"
                    "import sys\n"
                    "import time\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                    "with open(os.environ['CHILD_PID_FILE'], 'w', encoding='utf-8') as fh:\n"
                    "    fh.write(str(child.pid))\n"
                    "Path = __import__('pathlib').Path\n"
                    "Path(os.environ['READY_FILE']).write_text('ready\\n', encoding='utf-8')\n"
                    "signal.signal(signal.SIGTERM, lambda *_args: sys.exit(143))\n"
                    "while True:\n"
                    "    time.sleep(1)\n"
                ),
            )

            env = self._managed_env()
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "--offline", "mcp:sync"],
                cwd=temp_root,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            child_pid = None
            try:
                deadline = time.time() + 10
                while time.time() < deadline:
                    if ready_file.exists() and child_pid_file.exists():
                        child_pid = int(child_pid_file.read_text(encoding="utf-8").strip())
                        break
                    if proc.poll() is not None:
                        break
                    time.sleep(0.1)

                self.assertIsNotNone(child_pid, "mcp:sync 未按预期启动子进程")
                os.kill(child_pid, 0)

                proc.terminate()
                proc.wait(timeout=10)
                proc.stdout.close()
                proc.stderr.close()

                deadline = time.time() + 3
                child_alive = True
                while time.time() < deadline:
                    try:
                        os.kill(child_pid, 0)
                    except OSError:
                        child_alive = False
                        break
                    time.sleep(0.1)

                self.assertFalse(child_alive, f"mcp:sync 退出后子进程仍存活: {child_pid}")
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)
                    proc.stdout.close()
                    proc.stderr.close()
                else:
                    proc.stdout.close()
                    proc.stderr.close()
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except OSError:
                        pass
