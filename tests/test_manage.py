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


COPY_IGNORE = shutil.ignore_patterns(
    "repositories", ".git", ".worktrees", "__pycache__", "secrets", ".env", "app", "node_modules", "target", "manager.local.json", "manager.synced.json"
)


def copy_test_repo(src: Path, dst: Path) -> None:
    shutil.copytree(src, dst, ignore=COPY_IGNORE)
    for candidate in (
        src / "engine-rs" / "target" / "release" / "agent-manager-engine",
        src / "engine-rs" / "target" / "debug" / "agent-manager-engine",
    ):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            target_bin = dst / "engine-rs" / "target" / candidate.parent.name / "agent-manager-engine"
            target_bin.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(candidate, target_bin)
            break


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
        env["AGENT_MANAGER_CHROME_USER_DATA_DIR"] = str(self.runtime_dir / "chrome-user-data")
        env["AGENT_MANAGER_BROWSER_US_CONFIG_DIR"] = str(self.runtime_dir / "browser-us-config")
        env["AGENT_MANAGER_BROWSER_US_NO_OPEN"] = "1"
        env["AGENT_MANAGER_BROWSER_US_SKIP_EXIT_CHECK"] = "1"
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

    def _write_optional_tool_definition(self, temp_root: Path, name: str = "optional-helper"):
        (temp_root / "config" / "tools.json").write_text(
            json.dumps(
                {
                    "tools": {
                        name: {
                            "install": "true",
                            "upgrade": "true",
                            "ready_check": "bin:__missing_optional_helper__",
                            "doctor_severity": "warning",
                        }
                    }
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

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
                copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
            failing_script = temp_root / "agent-tool" / "sync_tools.py"
            failing_script.write_text(
                "#!/usr/bin/env python3\nimport sys\nsys.exit(7)\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "--dry-run", "--offline", "init"],
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
            copy_test_repo(ROOT, temp_root)
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

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
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
            copy_test_repo(ROOT, temp_root)
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

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
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

    def _prepare_merge_capture_repo(self, temp_dir):
        temp_root = Path(temp_dir) / "repo"
        copy_test_repo(ROOT, temp_root)
        call_log = temp_root / "calls.log"
        # update 会跑 maybe_init_secrets；移除 secrets 并 stub init-secrets 保持 hermetic。
        remove_path(temp_root / "secrets")
        self._stub_all_pipeline_scripts(temp_root, call_log)
        capture_path = temp_root / "captured-config.json"
        # tools 是 update 管线里第一个拿到 --config 的步骤，借它把合并产物落盘供断言。
        self._write_stub(
            temp_root / "agent-tool" / "sync_tools.py",
            (
                "#!/usr/bin/env python3\n"
                "import shutil\n"
                "import sys\n"
                f"shutil.copyfile(sys.argv[sys.argv.index('--config') + 1], {str(capture_path)!r})\n"
                "sys.exit(0)\n"
            ),
        )
        return temp_root, call_log, capture_path

    def _run_update_and_capture_merged_config(self, temp_root):
        result = subprocess.run(
            [str(temp_root / "manage.sh"), "update", "--dry-run"],
            cwd=temp_root,
            env=self._managed_env(),
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def test_synced_layer_merges_into_config_when_local_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _call_log, capture_path = self._prepare_merge_capture_repo(temp_dir)
            (temp_root / "config" / "manager.local.json").unlink(missing_ok=True)
            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {},\n'
                '  "skills": {},\n'
                '  "mcp": {},\n'
                '  "probe_base": "base",\n'
                '  "probe_shared": { "base_only": "base", "shared": "base" }\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "manager.synced.json").write_text(
                '{\n'
                '  "probe_synced": "synced",\n'
                '  "probe_shared": { "shared": "synced", "synced_only": "synced" }\n'
                '}\n',
                encoding="utf-8",
            )

            self._run_update_and_capture_merged_config(temp_root)

            merged = json.loads(capture_path.read_text(encoding="utf-8"))
            self.assertEqual(merged["probe_base"], "base")
            self.assertEqual(merged["probe_synced"], "synced")
            self.assertEqual(
                merged["probe_shared"],
                {"base_only": "base", "shared": "synced", "synced_only": "synced"},
            )

    def test_local_layer_overrides_synced_layer_overrides_base(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _call_log, capture_path = self._prepare_merge_capture_repo(temp_dir)
            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {},\n'
                '  "skills": {},\n'
                '  "mcp": {},\n'
                '  "probe_base": "base",\n'
                '  "probe_priority": "base",\n'
                '  "probe_nested": { "base_only": "base", "all_three": "base" }\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "manager.synced.json").write_text(
                '{\n'
                '  "probe_synced": "synced",\n'
                '  "probe_priority": "synced",\n'
                '  "probe_nested": { "synced_only": "synced", "all_three": "synced" }\n'
                '}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "manager.local.json").write_text(
                '{\n'
                '  "probe_local": "local",\n'
                '  "probe_priority": "local",\n'
                '  "probe_nested": { "local_only": "local", "all_three": "local" }\n'
                '}\n',
                encoding="utf-8",
            )

            self._run_update_and_capture_merged_config(temp_root)

            merged = json.loads(capture_path.read_text(encoding="utf-8"))
            self.assertEqual(merged["probe_base"], "base")
            self.assertEqual(merged["probe_synced"], "synced")
            self.assertEqual(merged["probe_local"], "local")
            self.assertEqual(merged["probe_priority"], "local")
            self.assertEqual(
                merged["probe_nested"],
                {
                    "base_only": "base",
                    "synced_only": "synced",
                    "local_only": "local",
                    "all_three": "local",
                },
            )

    def test_missing_synced_and_local_layers_use_baseline_config_directly(self):
        # 反例：synced 与 local 都缺失时不写临时合并文件，CONFIG_FILE 就是基线本身，
        # 与仅 base 的现状逐字节一致（合并产物若经 json.dump 重排格式即破坏该等式）。
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _call_log, capture_path = self._prepare_merge_capture_repo(temp_dir)
            (temp_root / "config" / "manager.synced.json").unlink(missing_ok=True)
            (temp_root / "config" / "manager.local.json").unlink(missing_ok=True)
            manager_json = temp_root / "config" / "manager.json"
            manager_json.write_text(
                '{"agents": {}, "tools": {}, "skills": {}, "mcp": {}}\n',
                encoding="utf-8",
            )

            self._run_update_and_capture_merged_config(temp_root)

            self.assertEqual(capture_path.read_bytes(), manager_json.read_bytes())

    def test_invalid_synced_layer_aborts_before_running_tasks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            (temp_root / "config" / "manager.local.json").unlink(missing_ok=True)
            (temp_root / "config" / "manager.synced.json").write_text("{ invalid json\n", encoding="utf-8")

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

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
                timeout=60,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("共享层配置无效", combined)
            self.assertIn("manager.synced.json", combined)
            self.assertFalse(call_log.exists(), combined)

    def test_doctor_stops_after_config_syntax_errors(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "optional-helper": { "enabled": true }\n  },\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            self._write_optional_tool_definition(temp_root)
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
            self.assertRegex(combined, r"optional-helper\s+: 可选项未就绪")
            self.assertIn("发现 1 个提示项", combined)

    def test_doctor_warns_for_missing_generated_targets_when_agent_is_not_installed(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            copy_test_repo(ROOT, temp_root)
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "sample-skill": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
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
                '{\n  "repositories": {\n    "sample-skill": {\n      "url": "https://example.invalid/sample-skill.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
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
            copy_test_repo(ROOT, temp_root)
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "optional-helper": { "enabled": true }\n  },\n  "skills": {\n    "sample-skill": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            self._write_optional_tool_definition(temp_root)
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
                '{\n  "repositories": {\n    "sample-skill": {\n      "url": "https://example.invalid/sample-skill.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
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
            copy_test_repo(ROOT, temp_root)
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "optional-helper": { "enabled": true }\n  },\n  "skills": {\n    "sample-skill": { "enabled": true }\n  },\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            self._write_optional_tool_definition(temp_root)
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
                '{\n  "repositories": {\n    "sample-skill": {\n      "url": "https://example.invalid/sample-skill.git",\n      "agents": ["claude-code"]\n    }\n  },\n  "agents": {\n    "claude-code": "%s/.claude/skills"\n  }\n}\n'
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

    def test_update_preview_reports_health_caution_for_optional_tool_gaps(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            copy_test_repo(ROOT, temp_root)
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {\n    "optional-helper": { "enabled": true }\n  },\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            self._write_optional_tool_definition(temp_root)
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
                [str(temp_root / "manage.sh"), "update", "--dry-run"],
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


    def test_doctor_skips_node_and_npm_when_policy_does_not_need_them(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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

    def test_update_empty_policy_skips_optional_subengines(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            copy_test_repo(ROOT, temp_root)
            temp_home.mkdir()
            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )

            env = self._managed_env()
            env["HOME"] = str(temp_home)
            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update", "--dry-run"],
                cwd=temp_root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertIn("当前策略未启用任何 Rules 目标", combined)
            self.assertNotIn("计划原子更新 JSON", combined)
            self.assertNotIn("准备下载到托管缓存", combined)
            self.assertNotIn("正在安装 playwright-cli", combined)

    def test_doctor_snapshot_statistics_only_count_managed_backups(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            copy_test_repo(ROOT, temp_root)
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

    def test_manage_without_command_shows_help_and_does_not_run_pipeline(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"

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

    def test_global_flags_without_command_refuse_to_run_a_pipeline(self):
        # 全局 flag 只修饰命令。裸 `--offline` / `--dry-run` 曾默认落到 init（含安装缺失 Agent），
        # 与 `--offline` 对应的 update 语义不符，必须显式报错而不是隐式跑全量管线。
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"

            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")

            for flags in (["--offline"], ["--dry-run"], ["--dry-run", "--offline"]):
                with self.subTest(flags=flags):
                    result = subprocess.run(
                        [str(temp_root / "manage.sh"), *flags],
                        cwd=temp_root,
                        env=self._managed_env(),
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=5,
                    )

                    combined = result.stdout + result.stderr
                    self.assertNotEqual(result.returncode, 0, combined)
                    self.assertIn("只提供了全局参数", combined)
                    self.assertIn("./manage.sh init --dry-run", combined)
                    self.assertIn("./manage.sh --offline update", combined)
                    self.assertIn("./manage.sh help", combined)
                    self.assertFalse(call_log.exists(), combined)

    def test_update_does_not_run_terminal_management(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            # update 会跑 maybe_init_secrets；不隔离会对 secrets/.git 真联网 git pull，
            # 让本用例贴着超时上限抖动。移除 secrets 并 stub init-secrets 使其 hermetic。
            remove_path(temp_root / "secrets")
            self._write_init_secrets_stub(temp_root, call_log)

            (temp_root / "config" / "manager.json").write_text(
                '{\n'
                '  "agents": {},\n'
                '  "tools": {},\n'
                '  "skills": {},\n'
                '  "mcp": {}\n'
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
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
                # sync 现走完整 update 链路(aliases/tools/browser/rules/skills/mcp/hooks),基线 5-7s；
                # 上限用于抓挂死而非性能门禁，给足满负载余量。
                timeout=60,
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
            copy_test_repo(ROOT, temp_root)
            (temp_home / ".codex").mkdir(parents=True)

            call_log = temp_root / "calls.log"
            # update 会跑完整收敛链路：移除 secrets 并 stub init-secrets/skills 链接，
            # 避免对 secrets 仓库与 example.invalid 真联网；rules 同步仍走真实脚本。
            remove_path(temp_root / "secrets")
            self._write_init_secrets_stub(temp_root, call_log)
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

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
                [str(temp_root / "manage.sh"), "update"],
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
            copy_test_repo(ROOT, temp_root)
            (temp_home / ".claude").mkdir(parents=True, exist_ok=True)
            (temp_home / ".codex").mkdir(parents=True, exist_ok=True)

            call_log = temp_root / "calls.log"
            # 同上：update 跑完整链路，stub init-secrets/skills 链接保持 hermetic。
            remove_path(temp_root / "secrets")
            self._write_init_secrets_stub(temp_root, call_log)
            self._write_stub(
                temp_root / "agent-skill" / "link_skills.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )

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
                [str(temp_root / "manage.sh"), "update"],
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

    def test_rules_manifest_has_no_repo_root_targets_by_default(self):
        manifest = (ROOT / "config" / "rules-targets.list").read_text(encoding="utf-8")
        self.assertNotIn("{{REPO_ROOT}}", manifest)

    def _write_rules_stub(self, temp_root, call_log):
        self._write_stub(
            temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
            (
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                "exit 0\n"
            ),
        )

    def _write_browser_stub(self, temp_root, call_log):
        self._write_stub(
            temp_root / "scripts" / "sync_browser_google_us.py",
            (
                "#!/usr/bin/env python3\n"
                "import sys\n"
                f"open({str(call_log)!r}, 'a', encoding='utf-8').write('browser ' + ' '.join(sys.argv[1:]) + '\\n')\n"
                "sys.exit(0)\n"
            ),
        )

    def _write_init_secrets_stub(self, temp_root, call_log):
        self._write_stub(
            temp_root / "scripts" / "init-secrets.sh",
            (
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' init-secrets >> {str(call_log)!r}\n"
                "exit 0\n"
            ),
        )

    def test_update_syncs_runtime_without_version_bump_or_agents(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")
            self._stub_all_pipeline_scripts(temp_root, call_log)

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
            for expected in ("tools ", "skills ", "mcp ", "rules ", "claude-hooks ", "codex-hooks "):
                self.assertTrue(
                    any(line.startswith(expected) for line in log_lines),
                    (expected, log_lines),
                )
            for forbidden in ("versions ", "skill-versions ", "agents "):
                self.assertFalse(
                    any(line.startswith(forbidden) for line in log_lines),
                    (forbidden, log_lines),
                )
            # 两个 hooks 同步步骤位于 MCP 注入之后、skill 引用审计之前。
            mcp_index = next(i for i, line in enumerate(log_lines) if line.startswith("mcp "))
            claude_hooks_index = next(i for i, line in enumerate(log_lines) if line.startswith("claude-hooks "))
            codex_hooks_index = next(i for i, line in enumerate(log_lines) if line.startswith("codex-hooks "))
            self.assertLess(mcp_index, claude_hooks_index, log_lines)
            self.assertLess(claude_hooks_index, codex_hooks_index, log_lines)

    def _stub_all_pipeline_scripts(self, temp_root: Path, call_log: Path):
        self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log)
        self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log)
        self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log)
        self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log)
        self._append_log_stub(temp_root / "agent-rule" / "scripts" / "sync-claude-hooks.py", "claude-hooks", call_log)
        self._append_log_stub(temp_root / "agent-rule" / "scripts" / "sync-codex-hooks.py", "codex-hooks", call_log)
        self._write_rules_stub(temp_root, call_log)
        self._write_browser_stub(temp_root, call_log)
        self._write_init_secrets_stub(temp_root, call_log)

    def test_update_force_refresh_propagates_to_tools_skills_and_mcp(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

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
            self._write_rules_stub(temp_root, call_log)
            self._write_browser_stub(temp_root, call_log)
            self._write_init_secrets_stub(temp_root, call_log)

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
            self.assertTrue(any(line.startswith("tools ") and "--force-refresh" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("skills ") and "--force-refresh" in line for line in log_lines), log_lines)
            self.assertTrue(any(line.startswith("mcp ") and "--force-refresh" in line for line in log_lines), log_lines)

    def test_agents_update_runs_agent_lifecycle_only_without_distribution(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--no-install' in sys.argv and '--async-actions' in sys.argv and '--wait-async-actions' in sys.argv",
            )
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log)
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log)
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log)
            self._write_rules_stub(temp_root, call_log)
            self._write_browser_stub(temp_root, call_log)
            self._write_init_secrets_stub(temp_root, call_log)

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "agents:update"],
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
                    and "--no-install" in line
                    and "--wait-async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )
            for forbidden in ("tools ", "skills ", "mcp ", "rules ", "browser "):
                self.assertFalse(
                    any(line.startswith(forbidden) for line in log_lines),
                    (forbidden, log_lines),
                )
            self.assertFalse(any(line == "init-secrets" for line in log_lines), log_lines)


    def test_init_allows_agent_install_and_waits_for_agent_lifecycle(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(
                temp_root / "agent-lifecycle" / "sync_agents.py",
                "agents",
                call_log,
                "'--no-install' not in sys.argv and '--offline' not in sys.argv and '--wait-async-actions' in sys.argv",
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
                [str(temp_root / "manage.sh"), "init"],
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
                    and "--wait-async-actions" in line
                    for line in log_lines
                ),
                log_lines,
            )


    def test_update_pipeline_skips_browser_google_us_sync_when_disabled(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            remove_path(temp_root / "secrets")

            self._append_log_stub(temp_root / "agent-lifecycle" / "sync_agents.py", "agents", call_log, "True")
            self._append_log_stub(temp_root / "agent-tool" / "sync_tools.py", "tools", call_log, "True")
            self._append_log_stub(temp_root / "agent-skill" / "link_skills.py", "skills", call_log, "True")
            self._append_log_stub(temp_root / "agent-mcp" / "sync_mcp.py", "mcp", call_log, "True")
            self._write_stub(
                temp_root / "scripts" / "sync_browser_google_us.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(91)\n",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh",
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"rules $*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
            )
            (temp_root / "config" / "manager.local.json").write_text(
                '{\n  "browser_us_sync": { "enabled": false }\n}\n',
                encoding="utf-8",
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
            self.assertFalse(any(line.startswith("browser ") for line in log_lines), log_lines)
            self.assertIn("浏览器 US 同步已禁用", combined)

    def test_update_pipeline_pulls_secrets_with_ff_only(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)

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

    def test_update_init_secrets_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)

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

    def test_dry_run_skips_secrets_bootstrap_and_git_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
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
                [str(temp_root / "manage.sh"), "--dry-run", "init"],
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
            copy_test_repo(ROOT, temp_root)
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
                [str(temp_root / "manage.sh"), "--dry-run", "--offline", "init"],
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
            copy_test_repo(ROOT, temp_root)

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
                [str(temp_root / "manage.sh"), "--offline", "init"],
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
            copy_test_repo(ROOT, temp_root)

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
                [str(temp_root / "manage.sh"), "--offline", "init"],
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
            copy_test_repo(ROOT, temp_root)

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
                [str(temp_root / "manage.sh"), "--offline", "init"],
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
            copy_test_repo(ROOT, temp_root)

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
                [str(temp_root / "manage.sh"), "--offline", "init"],
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
            copy_test_repo(ROOT, temp_root)

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
                [str(temp_root / "manage.sh"), "--offline", "init"],
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
                [str(MANAGE), "--dry-run", "--offline", "update"],
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

    def test_help_commands_stay_readable_while_another_run_holds_the_lock(self):
        lock_backup = move_aside(self.lock_path)
        lock_dir_backup = move_aside(self.lock_dir)
        holder = subprocess.Popen(["sleep", "60"])
        try:
            self.lock_dir.mkdir(parents=True)
            (self.lock_dir / "pid").write_text(str(holder.pid), encoding="utf-8")
            for command in ("help", "-h", "--help", "-help"):
                with self.subTest(command=command):
                    result = subprocess.run(
                        [str(MANAGE), command],
                        cwd=ROOT,
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
            restore_path(self.lock_path, lock_backup)
            restore_path(self.lock_dir, lock_dir_backup)

    def test_termination_cleans_up_descendant_processes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)

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

            env = self._managed_env()
            env["CHILD_PID_FILE"] = str(child_pid_file)
            env["READY_FILE"] = str(ready_file)

            proc = subprocess.Popen(
                [str(temp_root / "manage.sh"), "--offline", "agents:update"],
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

    def test_custom_lock_env_ignores_default_global_lock(self):
        global_lock_backup = move_aside(LOCK_PATH)
        global_lock_dir_backup = move_aside(LOCK_DIR)
        holder = subprocess.Popen(["sleep", "60"])
        try:
            LOCK_DIR.mkdir(parents=True)
            (LOCK_DIR / "pid").write_text(str(holder.pid), encoding="utf-8")

            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root = Path(temp_dir) / "repo"
                copy_test_repo(ROOT, temp_root)
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

                result = subprocess.run(
                    [str(temp_root / "manage.sh"), "--dry-run", "--offline", "init"],
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

    def test_pipeline_logs_use_owner_only_permissions(self):
        latest_backup = move_aside(self.latest_log_pointer)
        legacy_backup = move_aside(self.legacy_log_path)
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root = Path(temp_dir) / "repo"
                copy_test_repo(ROOT, temp_root)

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

                result = subprocess.run(
                    [str(temp_root / "manage.sh"), "--offline", "agents:update"],
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

    def test_mcp_sync_prepares_runtime_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
            call_log = temp_root / "calls.log"
            # update 会跑完整收敛链路；这里只验证 MCP 步骤的参数约定，
            # 其余步骤全部 stub，避免真实安装/克隆/写本机。
            remove_path(temp_root / "secrets")

            self._write_stub(
                temp_root / "agent-mcp" / "sync_mcp.py",
                (
                    "#!/usr/bin/env python3\n"
                    "import sys\n"
                    "from pathlib import Path\n"
                    f"Path({str(call_log)!r}).write_text('mcp ' + ' '.join(sys.argv[1:]) + '\\n', encoding='utf-8')\n"
                    "sys.exit(0 if '--prepare-runtime' in sys.argv and '--offline' not in sys.argv else 91)\n"
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
                temp_root / "agent-rule" / "scripts" / "sync-claude-hooks.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_stub(
                temp_root / "agent-rule" / "scripts" / "sync-codex-hooks.py",
                "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
            )
            self._write_rules_stub(temp_root, call_log)
            self._write_browser_stub(temp_root, call_log)
            self._write_init_secrets_stub(temp_root, call_log)

            result = subprocess.run(
                [str(temp_root / "manage.sh"), "update"],
                cwd=temp_root,
                env=self._managed_env(),
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            logged_lines = call_log.read_text(encoding="utf-8").splitlines()
            mcp_lines = [line for line in logged_lines if line.startswith("mcp ")]
            self.assertEqual(len(mcp_lines), 1, logged_lines)
            self.assertIn("--prepare-runtime", mcp_lines[0])
            self.assertNotIn("--offline", mcp_lines[0])

