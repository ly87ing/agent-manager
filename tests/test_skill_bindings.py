"""Tests for skill_bindings module and immutable skill version distribution."""

import copy
import errno
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = REPO_ROOT / "agent-skill" / "link_skills.py"

from src.skill_bindings import (
    SKILL_BINDINGS_POLICY_KEY,
    SkillBindingError,
    SkillConflictError,
    SkillBindingConflictError,
    SkillSharedTargetError,
    atomic_create_or_replace_link,
    check_not_external_conflict,
    check_shared_agent_target,
    get_binding,
    get_skill_versions_dir,
    is_link_entry,
    is_managed_skill_link,
    load_skill_bindings,
    normalize_path_for_compare,
    remove_binding,
    resolve_link_target,
    safe_remove_skill_link,
    safe_update_skill_link,
    save_skill_bindings,
    set_binding,
    validate_path_identifier,
)
from src.skill_versions import snapshot_skill, verify_skill_version
from src.storage.db import connect_ready_db, get_db_path, init_schema


def _hash_directory(path: Path) -> dict:
    """Compute sha256 checksums of all files in a directory tree."""
    res = {}
    if not path.exists():
        return res
    for root, _dirs, files in os.walk(path):
        for f in sorted(files):
            fp = Path(root) / f
            rel = fp.relative_to(path).as_posix()
            if fp.is_symlink():
                res[rel] = f"symlink:{os.readlink(fp)}"
            elif fp.is_file():
                h = hashlib.sha256()
                with open(fp, "rb") as fh:
                    for chunk in iter(lambda: fh.read(65536), b""):
                        h.update(chunk)
                res[rel] = h.hexdigest()
    return res


def _init_test_db(db_path: Path) -> None:
    """Initialize a compliant manager.db for testing."""
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path))
    init_schema(conn)
    with conn:
        conn.execute("INSERT INTO schema_meta (key, value) VALUES ('schema_version', '1')")
        conn.execute("INSERT INTO schema_meta (key, value) VALUES ('migration_completed_at', '2026-09-08T00:00:00Z')")
    conn.close()


class SkillBindingsTests(unittest.TestCase):
    def setUp(self):
        self._saved_env = {}
        for k in ("HOME", "AGENT_MANAGER_DATA_DIR", "AGENT_MANAGER_DIR"):
            self._saved_env[k] = os.environ.get(k)

        self.temp_dir = Path(tempfile.mkdtemp(prefix="skill_bindings_test_"))
        self.home_dir = self.temp_dir / "home"
        self.data_dir = self.temp_dir / "data"
        self.home_dir.mkdir()
        self.data_dir.mkdir()

        os.environ["HOME"] = str(self.home_dir)
        os.environ["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)

        self.db_path = self.data_dir / "manager.db"
        _init_test_db(self.db_path)

    def tearDown(self):
        for k, v in self._saved_env.items():
            if v is not None:
                os.environ[k] = v
            else:
                os.environ.pop(k, None)
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_binding_crud_and_audit_log(self):
        bindings = load_skill_bindings(self.db_path)
        self.assertEqual(bindings, {})

        v1_sha = "a" * 64
        set_binding(bindings, "col1", "skill1", "agent1", version=v1_sha, enabled=True, assigned=True)
        save_skill_bindings(
            self.db_path,
            bindings,
            action="add",
            target="col1/skill1@agent1",
            detail={"version": v1_sha},
        )

        loaded = load_skill_bindings(self.db_path)
        self.assertEqual(
            loaded,
            {"col1": {"skill1": {"agent1": {"version": v1_sha, "enabled": True, "assigned": True}}}},
        )

        # Audit log verification
        conn = connect_ready_db(self.db_path, read_only=True)
        rows = conn.execute("SELECT operation_type, target, detail_json FROM operation_logs").fetchall()
        conn.close()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["operation_type"], "skill:binding:add")
        self.assertEqual(rows[0]["target"], "col1/skill1@agent1")
        detail = json.loads(rows[0]["detail_json"])
        self.assertEqual(detail["action"], "add")
        self.assertEqual(detail["version"], v1_sha)

        # remove_binding sets assigned=False
        remove_binding(loaded, "col1", "skill1", "agent1")
        self.assertFalse(loaded["col1"]["skill1"]["agent1"]["assigned"])
        self.assertFalse(loaded["col1"]["skill1"]["agent1"]["enabled"])
        self.assertEqual(loaded["col1"]["skill1"]["agent1"]["version"], v1_sha)

    def test_shared_agent_target_check(self):
        targets = {
            "AgentA": str(self.temp_dir / "shared_dir"),
            "AgentB": str(self.temp_dir / "shared_dir"),
            "AgentC": str(self.temp_dir / "separate_dir"),
        }
        with self.assertRaises(SkillSharedTargetError):
            check_shared_agent_target("AgentA", targets)
        with self.assertRaises(SkillSharedTargetError):
            check_shared_agent_target("AgentB", targets)
        # AgentC is isolated
        check_shared_agent_target("AgentC", targets)

    def test_symlink_unsupported_fails_explicitly(self):
        target = self.temp_dir / "dummy_source"
        target.mkdir()
        link = self.temp_dir / "dummy_link"
        with mock.patch("os.symlink", side_effect=OSError(errno.EPERM, "Operation not permitted")):
            with self.assertRaises(OSError) as ctx:
                atomic_create_or_replace_link(target, link)
            self.assertIn("创建符号链接失败", str(ctx.exception))

    def test_external_conflict_detection(self):
        store_dir = self.data_dir / "skill-versions"
        store_dir.mkdir()
        repo_dir = self.temp_dir / "repo"
        repo_dir.mkdir()

        # Normal managed link
        managed_link = self.temp_dir / "link_managed"
        atomic_create_or_replace_link(store_dir, managed_link)
        self.assertTrue(is_managed_skill_link(managed_link, [repo_dir, store_dir]))
        check_not_external_conflict(managed_link, [repo_dir, store_dir])

        # External directory
        ext_dir = self.temp_dir / "ext_dir"
        ext_dir.mkdir()
        (ext_dir / "user_file.txt").write_text("important")
        self.assertFalse(is_managed_skill_link(ext_dir, [repo_dir, store_dir]))
        with self.assertRaises(SkillConflictError):
            check_not_external_conflict(ext_dir, [repo_dir, store_dir])

        # External symlink
        ext_target = self.temp_dir / "external_target"
        ext_target.mkdir()
        ext_link = self.temp_dir / "ext_link"
        atomic_create_or_replace_link(ext_target, ext_link)
        self.assertFalse(is_managed_skill_link(ext_link, [repo_dir, store_dir]))
        with self.assertRaises(SkillConflictError):
            check_not_external_conflict(ext_link, [repo_dir, store_dir])

    def test_safe_update_link_rollback_on_db_failure(self):
        store_dir = self.data_dir / "skill-versions"
        v1_dir = store_dir / "v1" / "content"
        v2_dir = store_dir / "v2" / "content"
        v1_dir.mkdir(parents=True)
        v2_dir.mkdir(parents=True)

        target_dir = self.temp_dir / "target"
        target_dir.mkdir()
        link_path = target_dir / "skill"
        atomic_create_or_replace_link(v1_dir, link_path)
        self.assertEqual(Path(resolve_link_target(link_path)).resolve(), v1_dir.resolve())

        def failing_db_cb():
            raise sqlite3.OperationalError("Simulated DB lock / crash")

        with self.assertRaises(sqlite3.OperationalError):
            safe_update_skill_link(v2_dir, link_path, failing_db_cb, [store_dir])

        # Link must have rolled back to v1_dir
        self.assertEqual(Path(resolve_link_target(link_path)).resolve(), v1_dir.resolve())

    def test_safe_update_skill_link_db_callback_external_dir_preserved_on_failure(self):
        """反例：DB callback 替换 link 为外部目录后抛错，目录及金丝雀内容必须完整保留，绝不删除或覆盖"""
        store_dir = self.data_dir / "skill-versions"
        v1_dir = store_dir / "v1" / "content"
        v1_dir.mkdir(parents=True)
        v2_dir = store_dir / "v2" / "content"
        v2_dir.mkdir(parents=True)

        target_dir = self.temp_dir / "target_update_cb_preserve"
        target_dir.mkdir()

        # 分支 1：存在 old_target 时，DB callback 将链接替换为外部普通目录后抛错
        link_path = target_dir / "skill_with_old"
        atomic_create_or_replace_link(v1_dir, link_path)
        self.assertEqual(Path(resolve_link_target(link_path)).resolve(), v1_dir.resolve())

        def failing_db_cb_with_external_dir():
            link_path.unlink()
            link_path.mkdir()
            (link_path / "canary.txt").write_text("precious external data", encoding="utf-8")
            raise sqlite3.OperationalError("Simulated DB lock during update")

        with self.assertRaises(sqlite3.OperationalError):
            safe_update_skill_link(v2_dir, link_path, failing_db_cb_with_external_dir, [store_dir])

        # 校验：外部目录及其内部金丝雀文件必须完好无损，绝未被回滚覆盖或删除
        self.assertTrue(link_path.is_dir())
        self.assertFalse(is_link_entry(link_path))
        self.assertEqual((link_path / "canary.txt").read_text(encoding="utf-8"), "precious external data")

        # 分支 2：old_target=None（新建链接）时，DB callback 将链接替换为外部普通目录后抛错
        new_link_path = target_dir / "skill_new"

        def failing_db_cb_with_external_dir_new():
            new_link_path.unlink()
            new_link_path.mkdir()
            (new_link_path / "canary_new.txt").write_text("precious new external data", encoding="utf-8")
            raise sqlite3.OperationalError("Simulated DB lock during new link creation")

        with self.assertRaises(sqlite3.OperationalError):
            safe_update_skill_link(v1_dir, new_link_path, failing_db_cb_with_external_dir_new, [store_dir])

        # 校验：新建链接失败且被外部替换为目录后，绝不能被 shutil.rmtree 递归删除！金丝雀必须保留！
        self.assertTrue(new_link_path.is_dir())
        self.assertFalse(is_link_entry(new_link_path))
        self.assertEqual((new_link_path / "canary_new.txt").read_text(encoding="utf-8"), "precious new external data")

    def test_safe_update_and_atomic_helper_reject_external_modifications_before_and_after(self):
        """反例：普通更新前已存在外部目录或 atomic helper 面对普通目录，必须拒绝覆盖，禁止递归删除"""
        store_dir = self.data_dir / "skill-versions"
        v1_dir = store_dir / "v1" / "content"
        v1_dir.mkdir(parents=True)

        target_dir = self.temp_dir / "target_external_reject"
        target_dir.mkdir()
        ext_dir = target_dir / "external_skill"
        ext_dir.mkdir()
        (ext_dir / "important.txt").write_text("do not delete", encoding="utf-8")

        # 1. safe_update_skill_link 更新前检测到外部目录，必须抛出 SkillConflictError 拒绝更新
        with self.assertRaises(SkillConflictError):
            safe_update_skill_link(v1_dir, ext_dir, None, [store_dir])
        self.assertTrue(ext_dir.is_dir())
        self.assertEqual((ext_dir / "important.txt").read_text(encoding="utf-8"), "do not delete")

        # 2. atomic_create_or_replace_link 面对已有普通目录，必须拒绝递归删除
        with self.assertRaises(SkillConflictError):
            atomic_create_or_replace_link(v1_dir, ext_dir)
        self.assertTrue(ext_dir.is_dir())
        self.assertEqual((ext_dir / "important.txt").read_text(encoding="utf-8"), "do not delete")

    def test_shared_agent_target_check_symlink_alias(self):
        real_dir = self.temp_dir / "real_agent_dir"
        real_dir.mkdir()
        alias_dir = self.temp_dir / "alias_agent_dir"
        os.symlink(real_dir, alias_dir)

        targets = {
            "AgentA": str(real_dir),
            "AgentB": str(alias_dir),
        }
        with self.assertRaises(SkillSharedTargetError):
            check_shared_agent_target("AgentA", targets)
        with self.assertRaises(SkillSharedTargetError):
            check_shared_agent_target("AgentB", targets)

    def test_safe_remove_link_rollback_on_db_failure(self):
        store_dir = self.data_dir / "skill-versions"
        v1_dir = store_dir / "v1" / "content"
        v1_dir.mkdir(parents=True)

        target_dir = self.temp_dir / "target_remove"
        target_dir.mkdir()
        link_path = target_dir / "skill"
        atomic_create_or_replace_link(v1_dir, link_path)
        self.assertEqual(Path(resolve_link_target(link_path)).resolve(), v1_dir.resolve())

        def failing_db_cb():
            raise sqlite3.OperationalError("Simulated DB lock during remove")

        with self.assertRaises(sqlite3.OperationalError):
            safe_remove_skill_link(link_path, failing_db_cb, [store_dir])

        # Link must have rolled back to v1_dir
        self.assertTrue(link_path.exists() or is_link_entry(link_path))
        self.assertEqual(Path(resolve_link_target(link_path)).resolve(), v1_dir.resolve())

    def test_concurrency_conflict_rejection_and_merge(self):
        # 初始写入两个条目
        v1 = "1" * 64
        v2_init = "2" * 64
        v2 = "3" * 64
        v3 = "4" * 64
        v2_updated = "5" * 64
        b1 = load_skill_bindings(self.db_path)
        set_binding(b1, "col1", "skill1", "agent1", version=v1, enabled=True, assigned=True)
        set_binding(b1, "col1", "skill2", "agent2", version=v2_init, enabled=True, assigned=True)
        save_skill_bindings(self.db_path, b1, action="init")

        # 模拟并发进程 A 和进程 B 分别读出相同初始数据
        proc_a_bindings = load_skill_bindings(self.db_path)
        proc_b_bindings = load_skill_bindings(self.db_path)

        # 进程 A 更新 col1/skill1@agent1 为 v2
        old_a_entry = copy.deepcopy(get_binding(proc_a_bindings, "col1", "skill1", "agent1"))
        set_binding(proc_a_bindings, "col1", "skill1", "agent1", version=v2, enabled=True, assigned=True)
        save_skill_bindings(
            self.db_path,
            proc_a_bindings,
            action="update",
            target="col1/skill1@agent1",
            expected_entry=old_a_entry,
        )

        # 进程 B 尝试用旧版本期望更新 col1/skill1@agent1 为 v3，应被冲突拒绝
        old_b_entry = copy.deepcopy(get_binding(proc_b_bindings, "col1", "skill1", "agent1"))
        set_binding(proc_b_bindings, "col1", "skill1", "agent1", version=v3, enabled=True, assigned=True)
        with self.assertRaises(SkillBindingConflictError):
            save_skill_bindings(
                self.db_path,
                proc_b_bindings,
                action="update",
                target="col1/skill1@agent1",
                expected_entry=old_b_entry,
            )

        # 进程 C 更新另一个目标 col1/skill2@agent2，事务内按目标合并，互不覆盖
        proc_c_bindings = load_skill_bindings(self.db_path)
        old_c_entry = copy.deepcopy(get_binding(proc_c_bindings, "col1", "skill2", "agent2"))
        set_binding(proc_c_bindings, "col1", "skill2", "agent2", version=v2_updated, enabled=True, assigned=True)
        save_skill_bindings(
            self.db_path,
            proc_c_bindings,
            action="update",
            target="col1/skill2@agent2",
            expected_entry=old_c_entry,
        )

        # 验证最终 DB 数据包含进程 A 的 v2 与进程 C 的 v2_updated
        final_bindings = load_skill_bindings(self.db_path)
        self.assertEqual(final_bindings["col1"]["skill1"]["agent1"]["version"], v2)
        self.assertEqual(final_bindings["col1"]["skill2"]["agent2"]["version"], v2_updated)

    def test_path_identifier_safety_check(self):
        valid_v = "1" * 64
        # 拒绝 /、\、.、.. 等逃逸字符与空字符
        for bad_id in ("../escape", "foo/bar", "foo\\bar", ".", "..", "foo\x00bar", ""):
            with self.assertRaises(SkillBindingError):
                validate_path_identifier(bad_id)
            with self.assertRaises(SkillBindingError):
                set_binding({}, bad_id, "skill", "agent", version=valid_v)
            with self.assertRaises(SkillBindingError):
                get_binding({}, bad_id, "skill", "agent")
            with self.assertRaises(SkillBindingError):
                remove_binding({}, bad_id, "skill", "agent")

    def test_resolve_link_target_posix_absolute(self):
        # 验证 POSIX 绝对路径软链接回读不会被转成 Windows 反斜线
        target_dir = self.temp_dir / "posix_target"
        target_dir.mkdir()
        link_path = self.temp_dir / "posix_link"
        os.symlink(str(target_dir.resolve()), link_path)
        resolved = resolve_link_target(link_path)
        self.assertNotIn("\\", resolved)
        self.assertEqual(Path(resolved).resolve(), target_dir.resolve())

    def test_save_skill_bindings_expected_entry_none_prevents_duplicate_creation(self):
        v1 = "1" * 64
        v1_other = "2" * 64
        b1 = load_skill_bindings(self.db_path)
        set_binding(b1, "col1", "skill1", "agent1", version=v1, enabled=True, assigned=True)
        save_skill_bindings(
            self.db_path,
            b1,
            action="add",
            target="col1/skill1@agent1",
            expected_entry=None,
        )

        # 第二个并发进程同样期望尚不存在并尝试添加，应被冲突拒绝
        b2 = {}
        set_binding(b2, "col1", "skill1", "agent1", version=v1_other, enabled=True, assigned=True)
        with self.assertRaises(SkillBindingConflictError):
            save_skill_bindings(
                self.db_path,
                b2,
                action="add",
                target="col1/skill1@agent1",
                expected_entry=None,
            )

    def test_save_skill_bindings_bad_json_fails_closed_and_preserves_bytes(self):
        # 损坏 policy_entries 中 skill_bindings 记录
        bad_json_content = "INVALID_JSON_RAW_BYTES_{{{{"
        conn = connect_ready_db(self.db_path)
        try:
            conn.execute(
                "INSERT OR REPLACE INTO policy_entries (key, value_json, updated_at) VALUES (?, ?, ?)",
                (SKILL_BINDINGS_POLICY_KEY, bad_json_content, "2026-09-08T00:00:00Z"),
            )
            conn.commit()
        finally:
            conn.close()

        # save_skill_bindings 必须 fail-closed 抛错，拒绝覆盖
        with self.assertRaises(SkillBindingError):
            save_skill_bindings(self.db_path, {"col1": {}}, action="update")

        # 检查数据库中原始损坏字符串未被覆盖为 {}
        conn = connect_ready_db(self.db_path, read_only=True)
        try:
            row = conn.execute(
                "SELECT value_json FROM policy_entries WHERE key = ?",
                (SKILL_BINDINGS_POLICY_KEY,),
            ).fetchone()
            self.assertEqual(row["value_json"], bad_json_content)
        finally:
            conn.close()

    def test_safe_update_skill_link_failure_during_atomic_replace_or_readback_restores_old_link(self):
        store_dir = self.data_dir / "skill-versions"
        v1_dir = store_dir / "v1" / "content"
        v1_dir.mkdir(parents=True)
        v2_dir = store_dir / "v2" / "content"
        v2_dir.mkdir(parents=True)

        target_dir = self.temp_dir / "target_update_fail"
        target_dir.mkdir()
        link_path = target_dir / "skill"
        atomic_create_or_replace_link(v1_dir, link_path)

        # 模拟原子替换中途发生异常（例如 Windows 上删除原入口后 mklink 失败）
        real_atomic_replace = atomic_create_or_replace_link

        def failing_replace(source, link):
            if Path(source).resolve() == v2_dir.resolve():
                if link.is_symlink() or link.exists():
                    link.unlink()
                raise OSError("Simulated mklink failure")
            return real_atomic_replace(source, link)

        with mock.patch("src.skill_bindings.atomic_create_or_replace_link", side_effect=failing_replace):
            with self.assertRaises(OSError):
                safe_update_skill_link(v2_dir, link_path, None, [store_dir])

        # 验证虽然中途替换失败并删除了原入口，但 rollback 将原链接恢复至 v1_dir
        self.assertTrue(link_path.exists() or is_link_entry(link_path))
        self.assertEqual(Path(resolve_link_target(link_path)).resolve(), v1_dir.resolve())



class SkillBindingsIntegrationTests(unittest.TestCase):
    """End-to-end fixture tests ensuring complete isolation and specification conformance."""

    def setUp(self):
        self._saved_env = {}
        for k in ("HOME", "AGENT_MANAGER_DATA_DIR", "AGENT_MANAGER_DIR"):
            self._saved_env[k] = os.environ.get(k)

        self.temp_dir = Path(tempfile.mkdtemp(prefix="skill_e2e_test_"))
        self.home_dir = self.temp_dir / "home"
        self.data_dir = self.temp_dir / "data"
        self.home_dir.mkdir()
        self.data_dir.mkdir()

        os.environ["HOME"] = str(self.home_dir)
        os.environ["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)

        self.db_path = self.data_dir / "manager.db"
        _init_test_db(self.db_path)

        # Setup repo, targets, and configs
        self.base_dir = self.temp_dir / "skills_base"
        self.repo_base_dir = self.base_dir / "repositories"
        self.repo_dir = self.repo_base_dir / "sample-repo"
        self.skill_dir = self.repo_dir / "bundle" / "my-skill"
        self.skill_dir.mkdir(parents=True)
        (self.skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n# Version 1\n", encoding="utf-8")
        (self.skill_dir / "data.txt").write_text("v1 content\n", encoding="utf-8")
        (self.repo_dir / ".git").mkdir(parents=True)
        (self.repo_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")

        self.target_a = self.temp_dir / "targets" / "agent_a"
        self.target_b = self.temp_dir / "targets" / "agent_b"
        self.target_a.mkdir(parents=True)
        self.target_b.mkdir(parents=True)

        self.config_path = self.temp_dir / "manager.json"
        self.def_path = self.temp_dir / "skills.json"
        self.agents_def_path = self.temp_dir / "agents.json"

        self.def_path.write_text(
            json.dumps({
                "repositories": {
                    "sample-repo": {
                        "url": "https://example.invalid/sample-repo.git",
                        "agents": ["AgentA", "AgentB"],
                    }
                },
                "agents": {
                    "AgentA": str(self.target_a),
                    "AgentB": str(self.target_b),
                },
            }),
            encoding="utf-8",
        )
        self.config_path.write_text(
            json.dumps({
                "skills": {
                    "sample-repo": {
                        "enabled": True,
                        "auto_update": False,
                    }
                }
            }),
            encoding="utf-8",
        )
        self.agents_def_path.write_text(
            json.dumps({
                "AgentA": {"display_name": "Agent A"},
                "AgentB": {"display_name": "Agent B"},
            }),
            encoding="utf-8",
        )

    def tearDown(self):
        for k, v in self._saved_env.items():
            if v is not None:
                os.environ[k] = v
            else:
                os.environ.pop(k, None)
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def run_cli(self, extra_args: list[str], expect_code: int = 0) -> subprocess.CompletedProcess:
        cmd = [
            sys.executable,
            str(MODULE_PATH),
            "--config", str(self.config_path),
            "--def-file", str(self.def_path),
            "--base-dir", str(self.base_dir),
            "--offline",
            "--yes",
        ] + extra_args
        env = dict(os.environ)
        env["HOME"] = str(self.home_dir)
        env["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)
        res = subprocess.run(cmd, cwd=REPO_ROOT, env=env, capture_output=True, text=True, check=False)
        self.assertEqual(
            res.returncode,
            expect_code,
            f"CLI failed (code {res.returncode}, expected {expect_code}). Output:\n{res.stdout}\n{res.stderr}",
        )
        return res

    def test_full_immutable_version_and_isolation_lifecycle(self):
        """Proof:
        1. A/B最初v1，源变v2，update A后A=v2/B=v1
        2. 禁用/启用A保留版本，更新已禁用分配不得重启用
        3. remove后普通sync不复活、add可恢复
        4. 集合刷新不越界
        5. 外部同名目录不被改
        6. shared目标拒绝
        7. dryrun全部字节不变
        8. 数据库未初始化拒绝静默降级
        """
        # Step 1: Initial sync does not auto-assign; explicit add-assignment creates links
        self.run_cli([])
        link_a = self.target_a / "my-skill"
        link_b = self.target_b / "my-skill"
        self.assertFalse(link_a.exists())
        self.assertFalse(link_b.exists())
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        self.run_cli(["--agent", "AgentB", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        self.assertTrue(link_a.is_symlink())
        self.assertTrue(link_b.is_symlink())

        target_a_v1 = resolve_link_target(link_a)
        target_b_v1 = resolve_link_target(link_b)
        self.assertEqual(target_a_v1, target_b_v1)
        store_dir = get_skill_versions_dir(self.data_dir).resolve()
        self.assertTrue(Path(target_a_v1).resolve().is_relative_to(store_dir))
        self.assertEqual((link_a / "data.txt").read_text(), "v1 content\n")

        # Step 2: Source changes to v2
        (self.skill_dir / "data.txt").write_text("v2 content\n", encoding="utf-8")

        # Normal sync: neither A nor B should change! (existing assignments pinned)
        self.run_cli([])
        self.assertEqual(resolve_link_target(link_a), target_a_v1)
        self.assertEqual(resolve_link_target(link_b), target_b_v1)
        self.assertEqual((link_a / "data.txt").read_text(), "v1 content\n")
        self.assertEqual((link_b / "data.txt").read_text(), "v1 content\n")

        # Step 3: Update Agent A to v2
        self.run_cli([
            "--update-version",
            "--only", "skill:sample-repo/my-skill",
            "--agent", "AgentA",
        ])
        target_a_v2 = resolve_link_target(link_a)
        target_b_current = resolve_link_target(link_b)
        self.assertNotEqual(target_a_v2, target_a_v1)
        self.assertEqual(target_b_current, target_b_v1)
        self.assertEqual((link_a / "data.txt").read_text(), "v2 content\n")
        self.assertEqual((link_b / "data.txt").read_text(), "v1 content\n")

        # Step 4: Disable Agent A for my-skill
        self.config_path.write_text(
            json.dumps({
                "skills": {
                    "sample-repo": {
                        "enabled": True,
                        "auto_update": False,
                        "disabled": ["my-skill"],
                    }
                }
            }),
            encoding="utf-8",
        )
        self.run_cli([])
        self.assertFalse(link_a.exists())
        self.assertFalse(link_a.is_symlink())
        self.assertFalse(link_b.exists())  # repo-level disabled applies to both in policy

        # Verify DB preserves version and assigned=True, and preserves AgentA enabled=True
        bindings = load_skill_bindings(self.db_path)
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["assigned"])
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["enabled"])

        # Update while collection disabled must be rejected (respects global disable)
        (self.skill_dir / "data.txt").write_text("v3 content\n", encoding="utf-8")
        self.run_cli([
            "--update-version",
            "--only", "skill:sample-repo/my-skill",
            "--agent", "AgentA",
        ], expect_code=2)

        # Re-enable in policy
        self.config_path.write_text(
            json.dumps({
                "skills": {
                    "sample-repo": {
                        "enabled": True,
                        "auto_update": False,
                    }
                }
            }),
            encoding="utf-8",
        )
        self.run_cli([])
        self.assertTrue(link_a.is_symlink())

        # Now update AgentA version while enabled
        self.run_cli([
            "--update-version",
            "--only", "skill:sample-repo/my-skill",
            "--agent", "AgentA",
        ])
        self.assertEqual((link_a / "data.txt").read_text(), "v3 content\n")
        self.assertEqual((link_b / "data.txt").read_text(), "v1 content\n")

        # Step 5: Remove assignment for Agent A
        self.run_cli([
            "--remove-assignment",
            "--only", "skill:sample-repo/my-skill",
            "--agent", "AgentA",
        ])
        self.assertFalse(link_a.exists())
        self.assertFalse(link_a.is_symlink())
        bindings = load_skill_bindings(self.db_path)
        self.assertFalse(bindings["sample-repo"]["my-skill"]["AgentA"]["assigned"])

        # Normal sync does NOT resurrect Agent A!
        self.run_cli([])
        self.assertFalse(link_a.exists())
        self.assertFalse(link_a.is_symlink())

        # Explicit add-assignment restores Agent A!
        self.run_cli([
            "--add-assignment",
            "--only", "skill:sample-repo/my-skill",
            "--agent", "AgentA",
        ])
        self.assertTrue(link_a.is_symlink())
        self.assertEqual((link_a / "data.txt").read_text(), "v3 content\n")

        # Step 6: External directory conflict is NOT overwritten or deleted
        ext_agent = self.temp_dir / "targets" / "agent_ext"
        ext_agent.mkdir(parents=True)
        external_skill_dir = ext_agent / "my-skill"
        external_skill_dir.mkdir()
        (external_skill_dir / "user_secret.txt").write_text("DO NOT DELETE\n", encoding="utf-8")

        self.def_path.write_text(
            json.dumps({
                "repositories": {
                    "sample-repo": {
                        "url": "https://example.invalid/sample-repo.git",
                        "agents": ["AgentExt"],
                    }
                },
                "agents": {
                    "AgentExt": str(ext_agent),
                },
            }),
            encoding="utf-8",
        )
        res = self.run_cli(["--agent", "AgentExt", "--only", "skill:sample-repo/my-skill", "--add-assignment"], expect_code=1)
        self.assertIn("外部非受管内容", res.stderr + res.stdout)
        # External files intact!
        self.assertTrue(external_skill_dir.is_dir())
        self.assertEqual((external_skill_dir / "user_secret.txt").read_text(), "DO NOT DELETE\n")

        # Step 7: Shared target directory rejection
        shared_agent_1 = self.temp_dir / "targets" / "shared_agent"
        self.def_path.write_text(
            json.dumps({
                "repositories": {
                    "sample-repo": {
                        "url": "https://example.invalid/sample-repo.git",
                        "agents": ["Shared1", "Shared2"],
                    }
                },
                "agents": {
                    "Shared1": str(shared_agent_1),
                    "Shared2": str(shared_agent_1),
                },
            }),
            encoding="utf-8",
        )
        res = self.run_cli(["--agent", "Shared1", "--only", "skill:sample-repo/my-skill", "--add-assignment"], expect_code=2)
        self.assertIn("共享", res.stderr + res.stdout)

        # Restore def_path for AgentA / AgentB
        self.def_path.write_text(
            json.dumps({
                "repositories": {
                    "sample-repo": {
                        "url": "https://example.invalid/sample-repo.git",
                        "agents": ["AgentA", "AgentB"],
                    }
                },
                "agents": {
                    "AgentA": str(self.target_a),
                    "AgentB": str(self.target_b),
                },
            }),
            encoding="utf-8",
        )

        # Step 8: Dry-run zero modification
        before_hash = _hash_directory(self.temp_dir)
        self.run_cli(["--dry-run", "--only", "skill:sample-repo/my-skill", "--agent", "AgentA", "--update-version"])
        after_hash = _hash_directory(self.temp_dir)
        self.assertEqual(before_hash, after_hash)

        # Step 9: Uninitialized database refuses silent downgrade
        no_db_data_dir = self.temp_dir / "no_db_dir"
        no_db_data_dir.mkdir()
        env = dict(os.environ)
        env["HOME"] = str(self.home_dir)
        env["AGENT_MANAGER_DATA_DIR"] = str(no_db_data_dir)
        res = subprocess.run([
            sys.executable,
            str(MODULE_PATH),
            "--config", str(self.config_path),
            "--def-file", str(self.def_path),
            "--base-dir", str(self.base_dir),
            "--only", "skill:sample-repo/my-skill",
            "--agent", "AgentA",
            "--add-assignment",
        ], cwd=REPO_ROOT, env=env, capture_output=True, text=True, check=False)
        self.assertEqual(res.returncode, 2)
        self.assertIn("不能静默降级", res.stderr + res.stdout)

    def test_bootstrap_snapshot_from_legacy_cache_link(self):
        """首次遇既有本工具cache链接，必须在clone/pull更新源之前快照并固定其当前版本"""
        # Create a direct legacy link pointing to repo cache
        legacy_link = self.target_a / "my-skill"
        atomic_create_or_replace_link(self.skill_dir, legacy_link)
        self.assertTrue(is_managed_skill_link(legacy_link, [self.repo_base_dir]))

        # DB currently has NO binding
        bindings = load_skill_bindings(self.db_path)
        self.assertIsNone(get_binding(bindings, "sample-repo", "my-skill", "AgentA"))

        # Run normal sync
        self.run_cli([])

        # Link must now point to skill-versions, NOT repo cache
        store_dir = get_skill_versions_dir(self.data_dir).resolve()
        target_resolved = Path(resolve_link_target(legacy_link)).resolve()
        self.assertTrue(target_resolved.is_relative_to(store_dir))

        # DB must record this binding
        bindings = load_skill_bindings(self.db_path)
        assignment = get_binding(bindings, "sample-repo", "my-skill", "AgentA")
        self.assertIsNotNone(assignment)
        self.assertTrue(assignment["assigned"])
        self.assertTrue(assignment["enabled"])
        self.assertTrue(bool(assignment["version"]))

    def test_tampered_version_verification_failure_preserves_old_state(self):
        """失败版本验证不得改变旧入口/记录"""
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        link_a = self.target_a / "my-skill"
        target_v1 = resolve_link_target(link_a)
        bindings_before = load_skill_bindings(self.db_path)

        # Corrupt the version content in store
        content_path = Path(target_v1)
        # Restore write permission on directory and file to tamper
        os.chmod(content_path, 0o755)
        os.chmod(content_path / "data.txt", 0o644)
        (content_path / "data.txt").write_text("TAMPERED")
        os.chmod(content_path / "data.txt", 0o400)
        os.chmod(content_path, 0o500)

        # Run sync: verification must fail, not crashing and not modifying DB
        res = self.run_cli([], expect_code=1)
        self.assertIn("篡改", res.stderr + res.stdout)
        bindings_after = load_skill_bindings(self.db_path)
        self.assertEqual(bindings_before, bindings_after)
        self.assertEqual(resolve_link_target(link_a), target_v1)

    def test_collection_disable_then_re_enable_preserves_agent_enabled_selection(self):
        # 初始显式分配：AgentA 和 AgentB 均分配
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        self.run_cli(["--agent", "AgentB", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        bindings = load_skill_bindings(self.db_path)
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["enabled"])
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentB"]["enabled"])

        # 通过显式 --disable-assignment 停用 AgentB
        self.run_cli(["--agent", "AgentB", "--only", "skill:sample-repo/my-skill", "--disable-assignment"])

        # 检查 AgentB 停用，AgentA 仍启用
        bindings = load_skill_bindings(self.db_path)
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["enabled"])
        self.assertFalse(bindings["sample-repo"]["my-skill"]["AgentB"]["enabled"])
        self.assertTrue((self.target_a / "my-skill").exists())
        self.assertFalse((self.target_b / "my-skill").exists())

        # 集合全局禁用：撤入口，但不得把各 Agent 的 enabled 改为 False，否则丢失 A开/B关 的选择
        disabled_repo_cfg = {
            "skills": {
                "sample-repo": {
                    "enabled": False,
                    "auto_update": False,
                    "skills": {
                        "my-skill": {
                            "agents": {
                                "AgentA": True,
                                "AgentB": False,
                            }
                        }
                    },
                }
            }
        }
        self.config_path.write_text(json.dumps(disabled_repo_cfg), encoding="utf-8")
        self.run_cli([])

        # 两个 Agent 的软链接入口均被撤掉
        self.assertFalse((self.target_a / "my-skill").exists())
        self.assertFalse((self.target_b / "my-skill").exists())

        # 但 DB 中的各 Agent enabled 偏好未被全部覆盖为 False，AgentA 仍为 True，AgentB 仍为 False
        bindings = load_skill_bindings(self.db_path)
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["enabled"])
        self.assertFalse(bindings["sample-repo"]["my-skill"]["AgentB"]["enabled"])

        # 集合重新启用：恢复后保留 A开/B关
        enabled_repo_cfg = {
            "skills": {
                "sample-repo": {
                    "enabled": True,
                    "auto_update": False,
                    "skills": {
                        "my-skill": {
                            "agents": {
                                "AgentA": True,
                                "AgentB": False,
                            }
                        }
                    },
                }
            }
        }
        self.config_path.write_text(json.dumps(enabled_repo_cfg), encoding="utf-8")
        self.run_cli([])

        # AgentA 重新建链，AgentB 保持未建链
        self.assertTrue((self.target_a / "my-skill").exists())
        self.assertFalse((self.target_b / "my-skill").exists())
        bindings = load_skill_bindings(self.db_path)
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["enabled"])
        self.assertFalse(bindings["sample-repo"]["my-skill"]["AgentB"]["enabled"])

    def test_remove_assignment_works_when_upstream_collection_deleted(self):
        # 初始建立分配
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        self.assertTrue((self.target_a / "my-skill").exists())

        # 模拟集合从上游 skills.json 和磁盘彻底删除
        self.def_path.write_text(json.dumps({"repositories": {}, "agents": {"AgentA": str(self.target_a)}}), encoding="utf-8")
        shutil.rmtree(self.repo_dir)

        # 显式 remove-assignment 应当依然能基于 bindings 定位并安全移除链接
        self.run_cli(["--agent", "AgentA", "--remove-assignment", "--only", "skill:sample-repo/my-skill"])
        self.assertFalse((self.target_a / "my-skill").exists())
        bindings = load_skill_bindings(self.db_path)
        self.assertFalse(bindings["sample-repo"]["my-skill"]["AgentA"]["assigned"])

    def test_add_and_update_respect_collection_and_skill_global_disables(self):
        # 初始建立分配
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])

        # 1. 集合全局禁用时，add 和 update 均应被拒绝
        disabled_cfg = {
            "skills": {
                "sample-repo": {
                    "enabled": False,
                    "auto_update": False,
                }
            }
        }
        self.config_path.write_text(json.dumps(disabled_cfg), encoding="utf-8")
        self.run_cli(["--agent", "AgentA", "--update-version", "--only", "skill:sample-repo/my-skill"], expect_code=2)
        self.run_cli(["--agent", "AgentB", "--add-assignment", "--only", "skill:sample-repo/my-skill"], expect_code=2)

        # 2. Skill 全局禁用时，add 和 update 也应被拒绝
        disabled_skill_cfg = {
            "skills": {
                "sample-repo": {
                    "enabled": True,
                    "auto_update": False,
                    "disabled": ["my-skill"],
                }
            }
        }
        self.config_path.write_text(json.dumps(disabled_skill_cfg), encoding="utf-8")
        self.run_cli(["--agent", "AgentA", "--update-version", "--only", "skill:sample-repo/my-skill"], expect_code=2)
        self.run_cli(["--agent", "AgentB", "--add-assignment", "--only", "skill:sample-repo/my-skill"], expect_code=2)

    def test_dry_run_validates_assignment_and_conflict_before_exit(self):
        # 初始显式分配
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])

        # 未分配的 skill 做 update-version --dry-run，应校验失败 (code 2) 而不是直接 exit 0
        self.run_cli(["--agent", "AgentA", "--update-version", "--dry-run", "--only", "skill:sample-repo/nonexistent-skill"], expect_code=2)

        # 外部非受管冲突做 add-assignment --dry-run，应抛出冲突异常并退出
        conflict_agent = self.temp_dir / "targets" / "agent_conflict"
        conflict_agent.mkdir(parents=True)
        external_file = conflict_agent / "my-skill"
        external_file.write_text("manual content")

        def_data = json.loads(self.def_path.read_text(encoding="utf-8"))
        def_data["agents"]["AgentConflict"] = str(conflict_agent)
        self.def_path.write_text(json.dumps(def_data), encoding="utf-8")

        self.run_cli(["--agent", "AgentConflict", "--add-assignment", "--dry-run", "--only", "skill:sample-repo/my-skill"], expect_code=1)

    def test_sqlite_duplicate_skill_across_repos_preserves_existing_assignment(self):
        # 1. 先显式分配 sample-repo/my-skill，建立 AgentA 的正确分配与链接
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        bindings_before = load_skill_bindings(self.db_path)
        v1_sha = bindings_before["sample-repo"]["my-skill"]["AgentA"]["version"]
        link_path = self.target_a / "my-skill"
        self.assertTrue(link_path.exists())
        target_before = resolve_link_target(link_path)

        # 2. 新增一个包含同名 my-skill 的第二个仓库 repo-two
        repo2_dir = self.repo_base_dir / "repo-two"
        repo2_skill_dir = repo2_dir / "bundle" / "my-skill"
        repo2_skill_dir.mkdir(parents=True)
        (repo2_skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n# Repo 2 version\n", encoding="utf-8")
        (repo2_dir / ".git").mkdir(parents=True)
        (repo2_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")

        def_data = json.loads(self.def_path.read_text(encoding="utf-8"))
        def_data["repositories"]["repo-two"] = {
            "url": "https://example.invalid/repo-two.git",
            "agents": ["AgentA"],
        }
        self.def_path.write_text(json.dumps(def_data), encoding="utf-8")

        cfg_data = json.loads(self.config_path.read_text(encoding="utf-8"))
        cfg_data["skills"]["repo-two"] = {"enabled": True, "auto_update": False}
        self.config_path.write_text(json.dumps(cfg_data), encoding="utf-8")

        # 3. 运行同步：检测到跨集合重复同名 skill，应报冲突退出 (code 1)
        res = self.run_cli([], expect_code=1)
        self.assertIn("发现跨仓库重复 skill name: my-skill", res.stderr + res.stdout)

        # 4. 关键验证：SQLite 激活后，不得像旧逻辑一样把已有的正确分配和链接清空！
        self.assertTrue(link_path.exists())
        self.assertEqual(resolve_link_target(link_path), target_before)
        bindings_after = load_skill_bindings(self.db_path)
        self.assertEqual(
            bindings_after["sample-repo"]["my-skill"]["AgentA"]["version"],
            v1_sha,
        )
        self.assertTrue(bindings_after["sample-repo"]["my-skill"]["AgentA"]["enabled"])
        self.assertTrue(bindings_after["sample-repo"]["my-skill"]["AgentA"]["assigned"])

    def test_bootstrap_rollback_on_db_failure(self):
        """测试首次 bootstrap 时若 DB 保存失败，旧 cache 链接必须安全回滚而不丢失"""
        legacy_link = self.target_a / "my-skill"
        atomic_create_or_replace_link(self.skill_dir, legacy_link)
        old_target = resolve_link_target(legacy_link)

        def failing_db_cb():
            raise sqlite3.OperationalError("Simulated bootstrap DB commit failure")

        store_dir = get_skill_versions_dir(self.data_dir)
        snap = snapshot_skill(self.skill_dir, store_dir)

        with self.assertRaises(sqlite3.OperationalError):
            safe_update_skill_link(
                snap["content_path"],
                legacy_link,
                failing_db_cb,
                managed_bases=[self.repo_base_dir, store_dir],
            )

        # 旧链接必须完整回滚至 self.skill_dir
        self.assertEqual(resolve_link_target(legacy_link), old_target)


    def test_new_skill_not_auto_assigned_on_bare_sync(self):
        """反例1：仓库新增技能在 SQLite 模式下普通 sync 不会自动分配给 eligible Agent"""
        # 初始仓库里有 sample-repo/my-skill，但从未显式分配
        self.run_cli([])
        # 两个 Agent 都不应该被自动分配或创建链接
        self.assertFalse((self.target_a / "my-skill").exists())
        self.assertFalse((self.target_b / "my-skill").exists())
        bindings = load_skill_bindings(self.db_path)
        self.assertIsNone(get_binding(bindings, "sample-repo", "my-skill", "AgentA"))
        self.assertIsNone(get_binding(bindings, "sample-repo", "my-skill", "AgentB"))

    def test_upstream_repo_deleted_preserves_old_link_and_entry(self):
        """反例2：上游仓库从配置和磁盘删除后，普通 sync 必须保留旧 immutable 已有分配与链接入口"""
        # 1. 显式分配
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        link_path = self.target_a / "my-skill"
        self.assertTrue(link_path.exists())
        target_version = resolve_link_target(link_path)
        bindings_before = load_skill_bindings(self.db_path)
        self.assertTrue(bindings_before["sample-repo"]["my-skill"]["AgentA"]["assigned"])

        # 2. 从 skills.json 和磁盘彻底删除上游仓库
        self.def_path.write_text(json.dumps({"repositories": {}, "agents": {"AgentA": str(self.target_a)}}), encoding="utf-8")
        shutil.rmtree(self.repo_dir)

        # 3. 运行普通 sync，不可撤销旧链接与绑定
        self.run_cli([])
        self.assertTrue(link_path.exists())
        self.assertEqual(resolve_link_target(link_path), target_version)
        bindings_after = load_skill_bindings(self.db_path)
        self.assertEqual(bindings_before, bindings_after)

    def test_collection_off_then_on_restores_link_even_when_source_deleted(self):
        """反例3：集合 off->on 恢复且源已删除也能从 store_dir 恢复链接入口"""
        # 1. 显式分配
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        link_path = self.target_a / "my-skill"
        self.assertTrue(link_path.exists())
        target_version = resolve_link_target(link_path)

        # 2. 模拟源技能目录被删除
        shutil.rmtree(self.skill_dir)

        # 3. 集合禁用 (off)
        off_cfg = {
            "skills": {
                "sample-repo": {
                    "enabled": False,
                    "auto_update": False,
                }
            }
        }
        self.config_path.write_text(json.dumps(off_cfg), encoding="utf-8")
        self.run_cli([])
        # 链接入口被撤销，但 DB 状态保留
        self.assertFalse(link_path.exists())
        bindings = load_skill_bindings(self.db_path)
        self.assertTrue(bindings["sample-repo"]["my-skill"]["AgentA"]["enabled"])

        # 4. 集合重新启用 (on)，源码目录仍不存在
        on_cfg = {
            "skills": {
                "sample-repo": {
                    "enabled": True,
                    "auto_update": False,
                }
            }
        }
        self.config_path.write_text(json.dumps(on_cfg), encoding="utf-8")
        self.run_cli([])
        # 链接入口必须从 store_dir 成功恢复！
        self.assertTrue(link_path.exists())
        self.assertEqual(resolve_link_target(link_path), target_version)

    def test_five_explicit_cli_operations_and_duplicate_rejection(self):
        """反例4：完整验证 5 个显式操作及 add 重复拒绝（非隐式 update）"""
        link_path = self.target_a / "my-skill"

        # 1. --add-assignment
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        self.assertTrue(link_path.exists())
        v1_target = resolve_link_target(link_path)
        b1 = load_skill_bindings(self.db_path)["sample-repo"]["my-skill"]["AgentA"]
        self.assertTrue(b1["assigned"])
        self.assertTrue(b1["enabled"])

        # 重复 --add-assignment 必须被拒绝 (exit 2)
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"], expect_code=2)

        # 2. --disable-assignment: 撤入口，保留 version/assigned=True, enabled=False
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--disable-assignment"])
        self.assertFalse(link_path.exists())
        b2 = load_skill_bindings(self.db_path)["sample-repo"]["my-skill"]["AgentA"]
        self.assertTrue(b2["assigned"])
        self.assertFalse(b2["enabled"])
        self.assertEqual(b2["version"], b1["version"])

        # 3. --enable-assignment: 恢复旧版本链接入口，enabled=True
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--enable-assignment"])
        self.assertTrue(link_path.exists())
        self.assertEqual(resolve_link_target(link_path), v1_target)
        b3 = load_skill_bindings(self.db_path)["sample-repo"]["my-skill"]["AgentA"]
        self.assertTrue(b3["assigned"])
        self.assertTrue(b3["enabled"])
        self.assertEqual(b3["version"], b1["version"])

        # 4. --update-version: 更新源码后显式升级版本
        (self.skill_dir / "data.txt").write_text("v2 content\n", encoding="utf-8")
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--update-version"])
        self.assertTrue(link_path.exists())
        v2_target = resolve_link_target(link_path)
        self.assertNotEqual(v2_target, v1_target)
        self.assertEqual((link_path / "data.txt").read_text(), "v2 content\n")

        # 5. --remove-assignment: 撤入口，assigned=False
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--remove-assignment"])
        self.assertFalse(link_path.exists())
        b5 = load_skill_bindings(self.db_path)["sample-repo"]["my-skill"]["AgentA"]
        self.assertFalse(b5["assigned"])

    def test_cross_collection_same_name_skill_conflict_rejection(self):
        """反例5：跨集合同名 skill 显式覆盖拒绝与独立安全移除"""
        # 1. 在 sample-repo 下分配 my-skill 给 AgentA
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        link_path = self.target_a / "my-skill"
        self.assertTrue(link_path.exists())
        sample_repo_target = resolve_link_target(link_path)

        # 2. 建立 repo-two，包含同名 my-skill
        repo2_dir = self.repo_base_dir / "repo-two"
        repo2_skill_dir = repo2_dir / "bundle" / "my-skill"
        repo2_skill_dir.mkdir(parents=True)
        (repo2_skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n# Repo 2 version\n", encoding="utf-8")
        (repo2_skill_dir / "data.txt").write_text("repo 2 content\n", encoding="utf-8")
        (repo2_dir / ".git").mkdir(parents=True)
        (repo2_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")

        def_data = json.loads(self.def_path.read_text(encoding="utf-8"))
        def_data["repositories"]["repo-two"] = {
            "url": "https://example.invalid/repo-two.git",
            "agents": ["AgentA"],
        }
        self.def_path.write_text(json.dumps(def_data), encoding="utf-8")

        cfg_data = json.loads(self.config_path.read_text(encoding="utf-8"))
        cfg_data["skills"]["repo-two"] = {"enabled": True, "auto_update": False}
        self.config_path.write_text(json.dumps(cfg_data), encoding="utf-8")

        # 3. 尝试将 repo-two 的 my-skill 也分配给 AgentA：必须拒绝 (exit 1)
        res = self.run_cli(["--agent", "AgentA", "--only", "skill:repo-two/my-skill", "--add-assignment"], expect_code=1)
        self.assertIn("已被集合 sample-repo 分配，拒绝跨集合覆盖", res.stderr + res.stdout)
        # 原有链接不被篡改
        self.assertEqual(resolve_link_target(link_path), sample_repo_target)

        # 4. 手动在 DB 中插入 repo-two 的已禁用 binding，再尝试 --enable-assignment 也必须拒绝 (exit 1)
        bindings = load_skill_bindings(self.db_path)
        store_dir = get_skill_versions_dir(self.data_dir)
        snap2 = snapshot_skill(repo2_skill_dir, store_dir)
        set_binding(bindings, "repo-two", "my-skill", "AgentA", version=snap2["version_id"], enabled=False, assigned=True)
        save_skill_bindings(self.db_path, bindings, action="test_insert", target="repo-two/my-skill@AgentA")

        res = self.run_cli(["--agent", "AgentA", "--only", "skill:repo-two/my-skill", "--enable-assignment"], expect_code=1)
        self.assertIn("已被集合 sample-repo 启用分配，拒绝跨集合覆盖", res.stderr + res.stdout)
        self.assertEqual(resolve_link_target(link_path), sample_repo_target)

        # 5. 对 repo-two 执行 --remove-assignment，不能删除 sample-repo 的有效入口！
        self.run_cli(["--agent", "AgentA", "--only", "skill:repo-two/my-skill", "--remove-assignment"])
        self.assertTrue(link_path.exists())
        self.assertEqual(resolve_link_target(link_path), sample_repo_target)


    def test_prescan_active_collisions_protects_existing_target_from_overwrite(self):
        """反例6：预扫描活跃冲突，确保排在后面的已有入口绝不会在写第一个链接时被覆盖"""
        # 1. 在 sample-repo 和 repo-two 分别准备 my-skill
        store_dir = get_skill_versions_dir(self.data_dir)
        snap1 = snapshot_skill(self.skill_dir, store_dir)

        repo2_dir = self.repo_base_dir / "repo-two"
        repo2_skill_dir = repo2_dir / "bundle" / "my-skill"
        repo2_skill_dir.mkdir(parents=True)
        (repo2_skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n# Repo 2\n", encoding="utf-8")
        (repo2_skill_dir / "data.txt").write_text("repo 2 content\n", encoding="utf-8")
        (repo2_dir / ".git").mkdir(parents=True)
        (repo2_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
        snap2 = snapshot_skill(repo2_skill_dir, store_dir)

        # 2. 配置两个仓库均包含 AgentA
        self.def_path.write_text(json.dumps({
            "repositories": {
                "sample-repo": {"url": "https://example.invalid/sample-repo.git", "agents": ["AgentA"]},
                "repo-two": {"url": "https://example.invalid/repo-two.git", "agents": ["AgentA"]},
            },
            "agents": {"AgentA": str(self.target_a)},
        }), encoding="utf-8")

        # 3. 构造 DB 中两个集合同时对 AgentA 处于活跃分配 (assigned=True, enabled=True)
        bindings = load_skill_bindings(self.db_path)
        set_binding(bindings, "sample-repo", "my-skill", "AgentA", version=snap1["version_id"], enabled=True, assigned=True)
        set_binding(bindings, "repo-two", "my-skill", "AgentA", version=snap2["version_id"], enabled=True, assigned=True)
        save_skill_bindings(self.db_path, bindings, action="test_setup", target="dual_collision")

        # 4. 磁盘上的实际入口先指向 repo-two 的版本
        link_path = self.target_a / "my-skill"
        atomic_create_or_replace_link(snap2["content_path"], link_path)
        expected_repo2_target = resolve_link_target(link_path)

        # 5. 运行普通 sync：必须在发生任何链接写操作前预扫描发现冲突，跳过双方，保持旧入口
        res = self.run_cli([], expect_code=1)
        self.assertIn("跨集合同名冲突", res.stderr + res.stdout)
        self.assertIn("跳过全部变更以保护旧入口", res.stderr + res.stdout)

        # 关键验证：原属于 repo-two 的入口绝不能先被 sample-repo 覆盖再报错！
        self.assertEqual(resolve_link_target(link_path), expected_repo2_target)

    def test_explicit_update_cross_collection_owner_check_and_disabled_isolation(self):
        """反例7：绑定A禁用、B启用同名时，更新A只改A版本不得碰B；A启用时若B已启用必须拒绝更新"""
        store_dir = get_skill_versions_dir(self.data_dir)
        snap_a_v1 = snapshot_skill(self.skill_dir, store_dir)

        repo2_dir = self.repo_base_dir / "repo-two"
        repo2_skill_dir = repo2_dir / "bundle" / "my-skill"
        repo2_skill_dir.mkdir(parents=True)
        (repo2_skill_dir / "SKILL.md").write_text("---\nname: my-skill\n---\n# Repo 2\n", encoding="utf-8")
        (repo2_skill_dir / "data.txt").write_text("repo 2 content\n", encoding="utf-8")
        (repo2_dir / ".git").mkdir(parents=True)
        (repo2_dir / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
        snap_b = snapshot_skill(repo2_skill_dir, store_dir)

        self.def_path.write_text(json.dumps({
            "repositories": {
                "sample-repo": {"url": "https://example.invalid/sample-repo.git", "agents": ["AgentA"]},
                "repo-two": {"url": "https://example.invalid/repo-two.git", "agents": ["AgentA"]},
            },
            "agents": {"AgentA": str(self.target_a)},
        }), encoding="utf-8")

        # A 禁用 (enabled=False)，B 启用 (enabled=True)
        bindings = load_skill_bindings(self.db_path)
        set_binding(bindings, "sample-repo", "my-skill", "AgentA", version=snap_a_v1["version_id"], enabled=False, assigned=True)
        set_binding(bindings, "repo-two", "my-skill", "AgentA", version=snap_b["version_id"], enabled=True, assigned=True)
        save_skill_bindings(self.db_path, bindings, action="test_setup", target="b_enabled_a_disabled")

        # 磁盘上当前入口指向 B
        link_path = self.target_a / "my-skill"
        atomic_create_or_replace_link(snap_b["content_path"], link_path)
        target_b = resolve_link_target(link_path)

        # 源码 A 发生更新为 v2
        (self.skill_dir / "data.txt").write_text("v2 content for A\n", encoding="utf-8")

        # 更新 A 的版本：此时 A 禁用，应成功只改 A 的版本与 DB，绝不触碰 B 的入口与链接
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--update-version"])
        bindings_after = load_skill_bindings(self.db_path)
        a_entry = bindings_after["sample-repo"]["my-skill"]["AgentA"]
        self.assertNotEqual(a_entry["version"], snap_a_v1["version_id"])
        self.assertFalse(a_entry["enabled"])
        self.assertEqual(resolve_link_target(link_path), target_b)

        # 反向分支：若 A 也被设置为启用状态，此时更新 A 必须检测跨集合同名占用并显式拒绝 (exit 1)
        set_binding(bindings_after, "sample-repo", "my-skill", "AgentA", version=a_entry["version"], enabled=True, assigned=True)
        save_skill_bindings(self.db_path, bindings_after, action="test_setup", target="both_enabled")

        (self.skill_dir / "data.txt").write_text("v3 content for A\n", encoding="utf-8")
        res = self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--update-version"], expect_code=1)
        self.assertIn("已被集合 repo-two 启用分配，拒绝跨集合覆盖", res.stderr + res.stdout)
        self.assertEqual(resolve_link_target(link_path), target_b)

    def test_dry_run_validates_tampered_snapshot_and_external_conflict(self):
        """反例8：dry-run 不能跳过验证，坏快照在 enable dry-run 必须报错，外部冲突在普通 sync dry-run 也必须报错"""
        # 1. 显式分配后 disable
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        link_path = self.target_a / "my-skill"
        target_v1 = resolve_link_target(link_path)
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--disable-assignment"])
        self.assertFalse(link_path.exists())

        # 篡改快照内容
        content_path = Path(target_v1)
        os.chmod(content_path, 0o755)
        os.chmod(content_path / "data.txt", 0o644)
        (content_path / "data.txt").write_text("CORRUPTED")
        os.chmod(content_path / "data.txt", 0o400)
        os.chmod(content_path, 0o500)

        # 显式 enable --dry-run 必须校验版本并失败，不可静默返回 0
        res = self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--enable-assignment", "--dry-run"], expect_code=1)
        self.assertIn("篡改", res.stderr + res.stdout)

        # 恢复快照内容
        os.chmod(content_path, 0o755)
        os.chmod(content_path / "data.txt", 0o644)
        (content_path / "data.txt").write_text("v1 content\n")
        os.chmod(content_path / "data.txt", 0o400)
        os.chmod(content_path, 0o500)

        # 2. 重新 enable，随后在目标路径放置外部非受管内容
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--enable-assignment"])
        link_path.unlink()
        link_path.mkdir()
        (link_path / "external.txt").write_text("manual")

        # 普通 sync --dry-run 也必须校验外部冲突并报错退出，不能直接跳过
        res_sync = self.run_cli(["--dry-run"], expect_code=1)
        self.assertIn("外部非受管内容", res_sync.stderr + res_sync.stdout)

    def test_mismatched_managed_version_on_disk_is_rejected_not_overwritten(self):
        """反例9：已有 binding 实际链接指向另一个受管 version 也必须拒绝覆盖并保留；缺入口则允许恢复"""
        store_dir = get_skill_versions_dir(self.data_dir)

        # 1. 建立 AgentA 分配指向 v1
        self.run_cli(["--agent", "AgentA", "--only", "skill:sample-repo/my-skill", "--add-assignment"])
        link_path = self.target_a / "my-skill"
        target_v1 = resolve_link_target(link_path)

        # 2. 制造另一个受管版本 v2
        (self.skill_dir / "data.txt").write_text("v2 content\n", encoding="utf-8")
        snap_v2 = snapshot_skill(self.skill_dir, store_dir)

        # 3. 将磁盘上的链接手动切换指向受管的 v2（但这不匹配 DB 中记录的期望版本 v1）
        atomic_create_or_replace_link(snap_v2["content_path"], link_path)
        self.assertEqual(resolve_link_target(link_path), str(snap_v2["content_path"].resolve()))

        # 4. 运行普通 sync：检测到链接指向非本分配期望版本，拒绝静默覆盖并保留原有状态
        res = self.run_cli([], expect_code=1)
        self.assertIn("现有链接指向非本分配期望版本", res.stderr + res.stdout)
        # 链接入口绝不能被静默篡改回 v1！
        self.assertEqual(resolve_link_target(link_path), str(snap_v2["content_path"].resolve()))

        # 5. 缺入口允许恢复：将链接删除
        link_path.unlink()
        self.assertFalse(link_path.exists())

        # 再次运行普通 sync：成功补全入口并恢复为 DB 期望的 v1
        self.run_cli([])
        self.assertTrue(link_path.exists())
        self.assertEqual(resolve_link_target(link_path), target_v1)


    def test_malformed_db_bindings_escape_path_and_type_rejected_fail_closed(self):
        """反例10：通用策略入口或合成 DB 中恶意/损坏 bindings 拒绝加载，绝不发生路径逃逸，且不覆盖数据库"""
        valid_version = "a" * 64
        canary_outside = self.target_a.parent / "escape"
        if canary_outside.exists():
            if canary_outside.is_dir():
                shutil.rmtree(canary_outside)
            else:
                canary_outside.unlink()

        # 1. 模拟通用 policy 写入含 ../escape 路径逃逸的 skill_bindings
        escape_payload = {
            "sample-repo": {
                "../escape": {
                    "AgentA": {
                        "version": valid_version,
                        "enabled": True,
                        "assigned": True,
                    }
                }
            }
        }
        raw_json = json.dumps(escape_payload)
        conn = connect_ready_db(self.db_path)
        try:
            conn.execute(
                "INSERT OR REPLACE INTO policy_entries (key, value_json, updated_at) VALUES (?, ?, ?)",
                (SKILL_BINDINGS_POLICY_KEY, raw_json, "2026-09-08T00:00:00Z"),
            )
            conn.commit()
        finally:
            conn.close()

        # 2. 直接调用 load_skill_bindings 必须抛出 SkillBindingError
        with self.assertRaises(SkillBindingError):
            load_skill_bindings(self.db_path)

        # 3. 运行普通 sync CLI：必须失败退出 (exit 1)，报错拒绝非法路径逃逸，绝不在目标外目录创建任何文件/软链
        res = self.run_cli([], expect_code=1)
        self.assertIn("加载技能绑定失败", res.stderr + res.stdout)
        self.assertFalse(canary_outside.exists(), "路径逃逸漏洞：目标外目录被非法创建或修改！")

        # 4. 尝试通过 save_skill_bindings 覆盖：必须 fail-closed 抛错，拒绝覆盖损坏/非法的 DB 记录
        with self.assertRaises(SkillBindingError):
            save_skill_bindings(
                self.db_path,
                {"sample-repo": {}},
                action="update",
                target="sample-repo/my-skill@AgentA",
            )

        # 5. 校验数据库原始数据未被覆盖
        conn = connect_ready_db(self.db_path, read_only=True)
        try:
            row = conn.execute(
                "SELECT value_json FROM policy_entries WHERE key = ?",
                (SKILL_BINDINGS_POLICY_KEY,),
            ).fetchone()
            self.assertEqual(row["value_json"], raw_json)
        finally:
            conn.close()

        # 6. 测试各种结构/类型损坏的数据，均必须被 load_skill_bindings 拒绝
        bad_cases = [
            # 嵌套 collection 逃逸
            {"../../col": {"my-skill": {"AgentA": {"version": valid_version, "enabled": True, "assigned": True}}}},
            # 嵌套 agent 逃逸
            {"sample-repo": {"my-skill": {"../../agent": {"version": valid_version, "enabled": True, "assigned": True}}}},
            # skill 不是 dict
            {"sample-repo": {"my-skill": "not_a_dict"}},
            # collection 不是 dict
            {"sample-repo": ["not_a_dict"]},
            # agent 不是 dict
            {"sample-repo": {"my-skill": {"AgentA": 123}}},
            # version 格式非法（非 64 位十六进制）
            {"sample-repo": {"my-skill": {"AgentA": {"version": "bad_digest", "enabled": True, "assigned": True}}}},
            # enabled 非 bool（为字符串）
            {"sample-repo": {"my-skill": {"AgentA": {"version": valid_version, "enabled": "true", "assigned": True}}}},
            # enabled 非 bool（为整数）
            {"sample-repo": {"my-skill": {"AgentA": {"version": valid_version, "enabled": 1, "assigned": True}}}},
            # assigned 非 bool（为整数）
            {"sample-repo": {"my-skill": {"AgentA": {"version": valid_version, "enabled": True, "assigned": 0}}}},
        ]

        for bad in bad_cases:
            bad_json = json.dumps(bad)
            c = connect_ready_db(self.db_path)
            try:
                c.execute(
                    "INSERT OR REPLACE INTO policy_entries (key, value_json, updated_at) VALUES (?, ?, ?)",
                    (SKILL_BINDINGS_POLICY_KEY, bad_json, "2026-09-08T00:00:00Z"),
                )
                c.commit()
            finally:
                c.close()

            with self.assertRaises(SkillBindingError, msg=f"应拒绝异常结构: {bad}"):
                load_skill_bindings(self.db_path)


    def test_new_collection_definition_without_policy_cfg_scans_as_library_resource_without_auto_binding(self):
        """反例11：SQLite 下仅在 definitions 声明新集合库资源，无 policy 额外配置也能正常扫描/更新，不自动分配，global enabled 默认 True"""
        # 1. 在 definitions 中增加新集合 gui-repo，不在 config.json (policy_cfg) 中添加任何条目
        def_data = json.loads(self.def_path.read_text(encoding="utf-8"))
        def_data["repositories"]["gui-repo"] = {
            "url": "https://example.invalid/gui-repo.git",
            "agents": ["AgentA", "AgentB"],
        }
        self.def_path.write_text(json.dumps(def_data), encoding="utf-8")

        # 准备本地缓存目录与技能
        gui_repo_dir = self.repo_base_dir / "gui-repo"
        gui_repo_dir.mkdir(parents=True)
        (gui_repo_dir / ".git").mkdir()
        gui_skill_dir = gui_repo_dir / "gui-skill"
        gui_skill_dir.mkdir()
        (gui_skill_dir / "SKILL.md").write_text("---\nname: gui-skill\n---\n# GUI Skill\n", encoding="utf-8")
        (gui_repo_dir / ".git_marker").write_text("managed", encoding="utf-8")

        # 2. 运行普通 sync：SQLite 下不得因“未声明策略”而跳过，应完成库资源扫描
        res = self.run_cli(["--offline"])
        self.assertNotIn("[gui-repo] 未声明策略，跳过", res.stderr + res.stdout)

        # 3. 严格遵循“新资源不自动分配”：不会为 AgentA 或 AgentB 自动建立链接入口
        target_a_link = self.target_a / "gui-skill"
        target_b_link = self.target_b / "gui-skill"
        self.assertFalse(target_a_link.exists())
        self.assertFalse(target_b_link.exists())

        # 4. 数据库中绝不自动生成 bindings
        bindings = load_skill_bindings(self.db_path)
        self.assertNotIn("gui-repo", bindings)

        # 5. 显式添加分配正常工作：因为资源库已被正确识别与扫描
        res_add = self.run_cli(["--agent", "AgentA", "--only", "skill:gui-repo/gui-skill", "--add-assignment"])
        self.assertIn("已添加分配 gui-repo/gui-skill", res_add.stderr + res_add.stdout)
        self.assertTrue(target_a_link.exists())
        self.assertFalse(target_b_link.exists())

        # 6. 当 definitions 中显式设置 enabled: False 时，全局禁用生效并跳过
        def_data["repositories"]["gui-repo"]["enabled"] = False
        self.def_path.write_text(json.dumps(def_data), encoding="utf-8")
        res_disabled = self.run_cli(["--offline"])
        self.assertIn("[gui-repo] (策略已禁用，跳过)", res_disabled.stderr + res_disabled.stdout)


if __name__ == "__main__":
    unittest.main()
