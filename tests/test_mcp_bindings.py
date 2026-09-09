"""Comprehensive tests for MCP bindings, version pinning, per-agent allocation, and conflict safety."""

import copy
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
MODULE_PATH = REPO_ROOT / "agent-mcp" / "sync_mcp.py"

from src.mcp_bindings import (
    MANAGED_BY_KEY,
    MANAGED_BY_VALUE,
    MCP_BINDINGS_POLICY_KEY,
    McpBindingConflictError,
    McpBindingError,
    McpConflictError,
    McpScopeError,
    McpSharedTargetError,
    UNSET,
    atomic_rollback_file,
    check_shared_agent_target,
    clean_definition,
    compute_applied_digest,
    compute_definition_version,
    extract_server_from_config,
    get_binding,
    load_mcp_bindings,
    normalize_path_for_compare,
    remove_binding,
    safe_atomic_write_and_verify,
    save_mcp_bindings,
    set_binding,
    validate_binding_entry,
    validate_identifier,
)
from src.storage.db import connect_ready_db, get_db_path, init_schema
from tests.agent_mcp_import import sync_mcp


def _init_test_db(db_path: Path) -> None:
    """Initialize a compliant manager.db for testing."""
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path))
    init_schema(conn)
    with conn:
        conn.execute("INSERT INTO schema_meta (key, value) VALUES ('schema_version', '1')")
        conn.execute("INSERT INTO schema_meta (key, value) VALUES ('migration_completed_at', '2026-09-08T00:00:00Z')")
    conn.close()


class McpBindingsUnitTests(unittest.TestCase):
    """Unit tests for src/mcp_bindings.py logic."""

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="mcp_bindings_unit_"))
        self.data_dir = self.temp_dir / "data"
        self.data_dir.mkdir()
        self.db_path = self.data_dir / "manager.db"
        _init_test_db(self.db_path)

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_validate_identifier(self):
        # Valid identifiers
        validate_identifier("server-1")
        validate_identifier("claude_code")
        validate_identifier("server.v2")

        # Invalid identifiers
        with self.assertRaises(McpBindingError):
            validate_identifier("")
        with self.assertRaises(McpBindingError):
            validate_identifier(".")
        with self.assertRaises(McpBindingError):
            validate_identifier("..")
        with self.assertRaises(McpBindingError):
            validate_identifier("srv/sub")
        with self.assertRaises(McpBindingError):
            validate_identifier("srv\\sub")
        with self.assertRaises(McpBindingError):
            validate_identifier("srv\x00test")

    def test_check_shared_agent_target(self):
        cfg_file_a = self.temp_dir / "agent_a.json"
        cfg_file_b = self.temp_dir / "agent_b.json"
        cfg_file_a.touch()
        cfg_file_b.touch()

        # Distinct targets - OK
        agent_targets = {
            "agent_a": str(cfg_file_a),
            "agent_b": str(cfg_file_b),
        }
        check_shared_agent_target("agent_a", agent_targets)

        # Shared physical file - raises McpSharedTargetError
        agent_targets_shared = {
            "agent_a": str(cfg_file_a),
            "agent_b": str(cfg_file_a),
        }
        with self.assertRaises(McpSharedTargetError):
            check_shared_agent_target("agent_a", agent_targets_shared)

        # Symlink pointing to same file - raises McpSharedTargetError
        symlink_b = self.temp_dir / "agent_b_symlink.json"
        symlink_b.symlink_to(cfg_file_a)
        agent_targets_symlink = {
            "agent_a": str(cfg_file_a),
            "agent_b": str(symlink_b),
        }
        with self.assertRaises(McpSharedTargetError):
            check_shared_agent_target("agent_a", agent_targets_symlink)

    def test_clean_definition_and_version(self):
        raw_def = {
            "command": "npx",
            "args": ["-y", "my-srv@1.0.0"],
            "env": {"API_KEY": "${MY_API_KEY}"},
            "_declared_in_policy": True,
            "_sync_ready": True,
            "_repo_path": "/tmp/some/path",
        }
        cleaned = clean_definition(raw_def)
        self.assertEqual(
            cleaned,
            {
                "command": "npx",
                "args": ["-y", "my-srv@1.0.0"],
                "env": {"API_KEY": "${MY_API_KEY}"},
            },
        )
        version1 = compute_definition_version(raw_def)
        version2 = compute_definition_version(cleaned)
        self.assertEqual(version1, version2)
        self.assertEqual(len(version1), 64)

        # Changes to internal metadata do not affect version
        raw_def_modified_meta = copy.deepcopy(raw_def)
        raw_def_modified_meta["_declared_in_policy"] = False
        self.assertEqual(compute_definition_version(raw_def_modified_meta), version1)

        # Changes to definition content change version
        raw_def_modified_content = copy.deepcopy(raw_def)
        raw_def_modified_content["args"] = ["-y", "my-srv@2.0.0"]
        self.assertNotEqual(compute_definition_version(raw_def_modified_content), version1)

    def test_compute_applied_digest_detects_env_value_changes(self):
        digest1 = compute_applied_digest(
            command="node",
            args=["index.js"],
            env={"API_KEY": "secret_1", "HOST": "localhost", MANAGED_BY_KEY: MANAGED_BY_VALUE},
            extra={"startup_timeout_sec": 60},
        )
        digest2 = compute_applied_digest(
            command="node",
            args=["index.js"],
            env={"API_KEY": "secret_DIFFERENT", "HOST": "localhost", MANAGED_BY_KEY: MANAGED_BY_VALUE},
            extra={"startup_timeout_sec": 60},
        )
        # External modification to env value must change digest to detect tampering
        self.assertNotEqual(digest1, digest2)

        # Internal MANAGED_BY tag changes do NOT affect digest
        digest_managed = compute_applied_digest(
            command="node",
            args=["index.js"],
            env={"API_KEY": "secret_1", "HOST": "localhost"},
            extra={"startup_timeout_sec": 60},
        )
        self.assertEqual(digest1, digest_managed)

        # Changing command or args changes the digest
        digest3 = compute_applied_digest(
            command="node2",
            args=["index.js"],
            env={"API_KEY": "secret_1", "HOST": "localhost"},
            extra={"startup_timeout_sec": 60},
        )
        self.assertNotEqual(digest1, digest3)

        # Null unknown field in extra must change digest (not ignored!)
        digest_with_null_extra = compute_applied_digest(
            command="node",
            args=["index.js"],
            env={"API_KEY": "secret_1", "HOST": "localhost"},
            extra={"startup_timeout_sec": 60, "user_note": None},
        )
        self.assertNotEqual(digest1, digest_with_null_extra)

        # Env value type changes (string vs int, string vs bool) must change digest
        digest_str_env = compute_applied_digest(
            command="node",
            args=["index.js"],
            env={"PORT": "8080"},
        )
        digest_int_env = compute_applied_digest(
            command="node",
            args=["index.js"],
            env={"PORT": 8080},
        )
        self.assertNotEqual(digest_str_env, digest_int_env)

    def test_definition_secret_scan_rejects_plaintext_credentials(self):
        # A definition containing a real GitHub token must be rejected by validate_un_injected_definition / set_binding
        bad_def = {
            "command": "node",
            "args": ["server.js"],
            "env": {"GITHUB_TOKEN": "".join(["g", "h", "p", "_"]) + "mocksecretfortest1234567890"},
        }
        bindings = {}
        with self.assertRaises(McpBindingError) as ctx:
            set_binding(bindings, "gh-srv", "agent1", version="v1", definition=bad_def)
        self.assertIn("敏感机密凭据", str(ctx.exception))

    def test_save_and_load_mcp_bindings_lifecycle(self):
        bindings = load_mcp_bindings(self.db_path)
        self.assertEqual(bindings, {})

        # Set binding
        def_v1 = {"command": "npx", "args": ["pkg1"]}
        v1 = compute_definition_version(def_v1)
        set_binding(
            bindings,
            "srv1",
            "agent1",
            version=v1,
            definition=def_v1,
            enabled=True,
            assigned=True,
            applied_digest="a" * 64,
        )

        saved = save_mcp_bindings(self.db_path, bindings, action="add", target="srv1@agent1")
        self.assertIn("srv1", saved)
        self.assertIn("agent1", saved["srv1"])

        # Reload from DB
        loaded = load_mcp_bindings(self.db_path)
        entry = get_binding(loaded, "srv1", "agent1")
        self.assertIsNotNone(entry)
        self.assertEqual(entry["version"], v1)
        self.assertTrue(entry["enabled"])
        self.assertTrue(entry["assigned"])

        # Check operation_logs audit
        conn = connect_ready_db(self.db_path)
        rows = conn.execute("SELECT * FROM operation_logs WHERE target = 'srv1@agent1'").fetchall()
        conn.close()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["operation_type"], "mcp:binding:add")

        # Concurrency check: expected_entry mismatch raises McpBindingConflictError
        with self.assertRaises(McpBindingConflictError):
            save_mcp_bindings(
                self.db_path,
                bindings,
                action="update",
                target="srv1@agent1",
                expected_entry={"version": "wrong_version"},
            )

        # Remove binding (tombstone assigned=False)
        remove_binding(bindings, "srv1", "agent1")
        save_mcp_bindings(self.db_path, bindings, action="remove", target="srv1@agent1")
        loaded2 = load_mcp_bindings(self.db_path)
        entry2 = get_binding(loaded2, "srv1", "agent1")
        self.assertFalse(entry2["assigned"])
        self.assertFalse(entry2["enabled"])

    def test_safe_atomic_write_render_failure_leaves_no_temp_no_db(self):
        """content_writer 渲染失败：无临时文件/FD 残留，原文件与 DB 均不变。"""
        target_file = self.temp_dir / "config.json"
        original = '{"initial": true}'
        target_file.write_text(original, encoding="utf-8")
        db_cb = mock.Mock()

        def failing_writer():
            raise ValueError("simulated render crash")

        with self.assertRaises(ValueError):
            safe_atomic_write_and_verify(
                target_file,
                content_writer=failing_writer,
                verify_cb=lambda p: True,
                db_commit_cb=db_cb,
            )

        self.assertEqual(target_file.read_text(encoding="utf-8"), original)
        db_cb.assert_not_called()
        leftovers = list(self.temp_dir.glob("config.json.tmp_*")) + list(self.temp_dir.glob("config.json.rb_*"))
        self.assertEqual(leftovers, [])

    def test_validate_binding_entry_fail_closed(self):
        """坏嵌套数据（字符串布尔、非法摘要、版本与定义不匹配、非对象记录）一律 fail-closed。"""
        bindings = {}
        good_def = {"command": "node", "args": ["a.js"]}
        good_version = compute_definition_version(good_def)

        # 字符串 "false" 不得被强转为布尔
        with self.assertRaises(McpBindingError):
            set_binding(bindings, "srv", "agent", version=good_version, definition=good_def, enabled="false", assigned=True)
        with self.assertRaises(McpBindingError):
            set_binding(bindings, "srv", "agent", version=good_version, definition=good_def, enabled=True, assigned=1)

        # 版本必须与未注入定义摘要匹配
        with self.assertRaises(McpBindingError):
            set_binding(bindings, "srv", "agent", version="v1", definition=good_def, enabled=True, assigned=True)

        # applied_digest 必须是 64 位 hex 或空
        with self.assertRaises(McpBindingError):
            set_binding(bindings, "srv", "agent", version=good_version, definition=good_def, enabled=True, assigned=True, applied_digest="not-a-digest")

        # assigned=True 时 definition 或 version 为空必须被拒绝
        with self.assertRaises(McpBindingError):
            set_binding(bindings, "srv_empty_def", "agent", version="", definition={}, enabled=True, assigned=True)
        with self.assertRaises(McpBindingError):
            validate_binding_entry("srv", "agent", {
                "version": "",
                "definition": good_def,
                "enabled": True,
                "assigned": True,
                "applied_digest": "",
            })
        with self.assertRaises(McpBindingError):
            validate_binding_entry("srv", "agent", {
                "version": "",
                "definition": {},
                "enabled": True,
                "assigned": True,
                "applied_digest": "",
            })

        # 合法条目 + 显式空墓碑均通过
        entry = set_binding(bindings, "srv", "agent", version=good_version, definition=good_def, enabled=True, assigned=True)
        self.assertEqual(entry["version"], good_version)
        tomb = set_binding(bindings, "srv2", "agent", version="", definition={}, enabled=False, assigned=False)
        self.assertEqual(tomb["version"], "")
        validated_tomb = validate_binding_entry("srv2", "agent", tomb)
        self.assertFalse(validated_tomb["assigned"])
        self.assertEqual(validated_tomb["version"], "")

    def _write_raw_bindings(self, payload: dict) -> None:
        conn = connect_ready_db(self.db_path)
        try:
            with conn:
                conn.execute(
                    "INSERT INTO policy_entries (key, value_json, updated_at) VALUES (?, ?, ?) "
                    "ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
                    (MCP_BINDINGS_POLICY_KEY, json.dumps(payload), "2026-09-08T00:00:00Z"),
                )
        finally:
            conn.close()

    def test_load_and_save_reject_bad_nested_data_fail_closed(self):
        """DB 中坏嵌套数据：load 抛错，save 拒绝覆盖且 DB 字节不变。"""
        good_def = {"command": "node", "args": ["a.js"]}
        good_version = compute_definition_version(good_def)
        good_entry = {
            "version": good_version,
            "definition": good_def,
            "enabled": True,
            "assigned": True,
            "applied_digest": "b" * 64,
        }
        bad_variants = [
            {"srv": {"agent": dict(good_entry, enabled="false")}},      # 字符串布尔
            {"srv": {"agent": dict(good_entry, applied_digest="x")}},   # 非法摘要
            {"srv": {"agent": dict(good_entry, version="v9")}},         # 版本与定义不匹配
            {"srv": {"agent": ["not-a-dict"]}},                          # 非对象记录
            {"srv": ["not-a-dict"]},                                     # 非对象 server 映射
            {"bad/srv": {"agent": good_entry}},                          # 非法标识符
            {"srv": {"agent": dict(good_entry, unknown_field=1)}},       # 未知字段
        ]
        for payload in bad_variants:
            self._write_raw_bindings(payload)
            with self.assertRaises(McpBindingError):
                load_mcp_bindings(self.db_path)
            db_bytes_before = self.db_path.read_bytes()
            with self.assertRaises(McpBindingError):
                save_mcp_bindings(self.db_path, {}, action="update", target="srv@agent")
            self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

    def test_scoped_changes_preserve_concurrent_other_agent(self):
        """scoped_changes 仅应用本次变更：其它 Agent 并发修改不被 stale 整树覆盖；同键冲突 fail-closed。"""
        def_a = {"command": "node", "args": ["a.js"]}
        bindings = {}
        set_binding(bindings, "srv", "agent-a", version=compute_definition_version(def_a), definition=def_a, enabled=True, assigned=True)
        set_binding(bindings, "srv", "agent-b", version=compute_definition_version(def_a), definition=def_a, enabled=True, assigned=True)
        save_mcp_bindings(self.db_path, bindings, action="add")

        # agent-a 进程基于旧快照工作；期间 agent-b 并发翻转了自己的 enabled
        stale_snapshot = load_mcp_bindings(self.db_path)
        concurrent = load_mcp_bindings(self.db_path)
        old_b = copy.deepcopy(get_binding(concurrent, "srv", "agent-b"))
        set_binding(concurrent, "srv", "agent-b", version=old_b["version"], definition=old_b["definition"], enabled=False, assigned=True)
        save_mcp_bindings(self.db_path, concurrent, action="disable", target="srv@agent-b", server="srv", agent="agent-b", expected_entry=old_b)

        # agent-a 提交自己的 scoped 变更（applied_digest 更新），不得影响 agent-b 的并发值
        old_a = copy.deepcopy(get_binding(stale_snapshot, "srv", "agent-a"))
        new_a = dict(old_a, applied_digest="c" * 64)
        save_mcp_bindings(
            self.db_path,
            stale_snapshot,
            action="sync",
            target="agent-a",
            scoped_changes=[("srv", "agent-a", old_a, new_a)],
        )
        loaded = load_mcp_bindings(self.db_path)
        self.assertEqual(get_binding(loaded, "srv", "agent-a")["applied_digest"], "c" * 64)
        self.assertFalse(get_binding(loaded, "srv", "agent-b")["enabled"])

        # 同键并发冲突：expected 旧值已过期，fail-closed 且 DB 不变
        conflict_old = copy.deepcopy(get_binding(loaded, "srv", "agent-a"))
        raced = dict(conflict_old, applied_digest="d" * 64)
        with self.assertRaises(McpBindingConflictError) as cm:
            save_mcp_bindings(
                self.db_path,
                loaded,
                action="sync",
                target="agent-a",
                scoped_changes=[("srv", "agent-a", old_a, raced)],
            )
        self.assertNotIn("c" * 64, str(cm.exception))
        loaded_after = load_mcp_bindings(self.db_path)
        self.assertEqual(get_binding(loaded_after, "srv", "agent-a"), conflict_old)

        # scoped 删除：new_entry=None 删除该键
        cur_a = copy.deepcopy(get_binding(loaded_after, "srv", "agent-a"))
        save_mcp_bindings(
            self.db_path,
            loaded_after,
            action="remove",
            target="agent-a",
            scoped_changes=[("srv", "agent-a", cur_a, None)],
        )
        self.assertIsNone(get_binding(load_mcp_bindings(self.db_path), "srv", "agent-a"))

    def test_safe_atomic_write_and_verify_rollback_on_db_failure(self):
        target_file = self.temp_dir / "config.json"
        target_file.write_text('{"initial": true}', encoding="utf-8")

        def writer():
            return '{"modified": true}'

        def verify(p):
            return True

        def failing_db():
            raise RuntimeError("Database simulated commit crash")

        with self.assertRaises(RuntimeError):
            safe_atomic_write_and_verify(
                target_file,
                content_writer=writer,
                verify_cb=verify,
                db_commit_cb=failing_db,
            )

        # Target file must be rolled back to original content
        self.assertEqual(target_file.read_text(encoding="utf-8"), '{"initial": true}')

    def test_cas_save_mcp_bindings_sentinel(self):
        """CAS: expected_entry=None expects entry absent; sentinel UNSET skips concurrency check."""
        bindings = {}
        srv = "my-srv"
        agent = "my-agent"
        def_v1 = {"command": "echo", "args": ["hi"], "env": {"SECRET_KEY": "pass123"}}
        v1 = compute_definition_version(def_v1)
        set_binding(bindings, srv, agent, version=v1, definition=def_v1, enabled=True, assigned=True)

        # 1. First add with expected_entry=None succeeds because DB currently has no entry
        save_mcp_bindings(self.db_path, bindings, action="add", target=f"{srv}@{agent}", expected_entry=None)

        # 2. Second add with expected_entry=None must fail because DB now HAS an entry
        with self.assertRaises(McpBindingConflictError) as cm:
            save_mcp_bindings(self.db_path, bindings, action="add", target=f"{srv}@{agent}", expected_entry=None)
        err_msg = str(cm.exception)
        self.assertIn("并发冲突", err_msg)
        self.assertIn("期望旧状态: None", err_msg)
        self.assertIn(f"version={v1[:12]}", err_msg)
        # Verify secret token / definition details are NOT leaked in conflict error message
        self.assertNotIn("pass123", err_msg)
        self.assertNotIn("SECRET_KEY", err_msg)

        # 3. Sentinel UNSET skips concurrency check
        def_v2 = {"command": "echo", "args": ["v2"]}
        set_binding(bindings, srv, agent, version=compute_definition_version(def_v2), definition=def_v2, enabled=True, assigned=True)
        save_mcp_bindings(self.db_path, bindings, action="update", target=f"{srv}@{agent}", expected_entry=UNSET)
        loaded = load_mcp_bindings(self.db_path)
        self.assertEqual(get_binding(loaded, srv, agent)["version"], compute_definition_version(def_v2))

    def test_safe_atomic_write_and_verify_external_changes(self):
        """safe_atomic_write_and_verify detects pre-replace changes, verify_cb exceptions, and post-write edits."""
        target_file = self.temp_dir / "target.json"
        target_file.write_text('{"v": 1}', encoding="utf-8")

        # 1. Target file modified between read and replace triggers McpConflictError
        def writer_tamper():
            target_file.write_text('{"tampered_externally": true}', encoding="utf-8")
            return '{"v": 2}'

        with self.assertRaises(McpConflictError):
            safe_atomic_write_and_verify(
                target_file,
                content_writer=writer_tamper,
                verify_cb=lambda p: True,
            )
        # Content remains the external tamper because replace was aborted
        self.assertEqual(target_file.read_text(encoding="utf-8"), '{"tampered_externally": true}')

        # 2. verify_cb raising exception triggers rollback
        target_file.write_text('{"v": 10}', encoding="utf-8")
        def verify_raising(p):
            raise ValueError("Corrupt write verification failure")

        with self.assertRaises(McpBindingError):
            safe_atomic_write_and_verify(
                target_file,
                content_writer=lambda: '{"v": 20}',
                verify_cb=verify_raising,
            )
        self.assertEqual(target_file.read_text(encoding="utf-8"), '{"v": 10}')

        # 3. atomic_rollback_file does not overwrite if file was modified after write
        atomic_rollback_file(
            target_file,
            old_bytes=b'{"v": 999}',
            expected_current_bytes=b'{"v": 888}',  # Actual on disk is {"v": 10}
        )
        self.assertEqual(target_file.read_text(encoding="utf-8"), '{"v": 10}')

    def test_extract_server_from_config_fail_closed(self):
        """extract_server_from_config raises McpConflictError on corrupt or non-dict structures."""
        # 1. Invalid JSON syntax
        json_file = self.temp_dir / "corrupt.json"
        json_file.write_text('{invalid_json: true', encoding="utf-8")
        with self.assertRaises(McpConflictError):
            extract_server_from_config(json_file, "claude-code", "json", "srv")

        # 2. JSON root is not a dict
        json_file.write_text('["item1", "item2"]', encoding="utf-8")
        with self.assertRaises(McpConflictError):
            extract_server_from_config(json_file, "claude-code", "json", "srv")

        # 3. JSON mcpServers is not a dict
        json_file.write_text('{"mcpServers": "string_not_dict"}', encoding="utf-8")
        with self.assertRaises(McpConflictError):
            extract_server_from_config(json_file, "claude-code", "json", "srv")

        # 4. JSON server entry is not a dict
        json_file.write_text('{"mcpServers": {"srv": 12345}}', encoding="utf-8")
        with self.assertRaises(McpConflictError):
            extract_server_from_config(json_file, "claude-code", "json", "srv")

        # 5. Invalid TOML syntax
        toml_file = self.temp_dir / "corrupt.toml"
        toml_file.write_text('key = [[invalid toml', encoding="utf-8")
        with self.assertRaises(McpConflictError):
            extract_server_from_config(toml_file, "codex", "toml", "srv")

        # 6. TOML server entry is not a dict
        toml_file.write_text('[mcp_servers]\nsrv = "not_table"\n', encoding="utf-8")
        with self.assertRaises(McpConflictError):
            extract_server_from_config(toml_file, "codex", "toml", "srv")




if __name__ == "__main__":
    unittest.main()
