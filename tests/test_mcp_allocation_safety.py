"""Integration tests for MCP allocation conflict safety, tamper detection, and rollback."""
from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import unittest
from unittest import mock

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
from tests.mcp_allocation_support import (
    REPO_ROOT,
    MODULE_PATH,
    _init_test_db,
    McpAllocationBaseTestCase,
)


class McpAllocationSafetyTests(McpAllocationBaseTestCase):
    """Integration tests for conflict safety, tamper detection, rollback, and credential protection."""
    def test_protection_against_unmanaged_external_items(self):
        """Existing unmanaged items and foreign config keys must not be overwritten."""
        # Claude config has an external unmanaged server and other unrelated keys
        external_content = {
            "unrelated_setting": "keep_me",
            "mcpServers": {
                "demo-srv": {
                    "command": "python3",
                    "args": ["external.py"],
                    "env": {"CUSTOM": "1"},  # Notice no _MANAGED_BY
                },
                "other-external-server": {
                    "command": "docker",
                    "args": ["run"],
                },
            },
        }
        self.claude_config.write_text(json.dumps(external_content, indent=2), encoding="utf-8")

        # Action add should be rejected
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertNotEqual(res_add.returncode, 0)
        self.assertIn("已有同名外部项不得覆盖", res_add.stderr)

        # Unrelated keys and external items remain intact
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertEqual(claude_doc["unrelated_setting"], "keep_me")
        self.assertEqual(claude_doc["mcpServers"]["demo-srv"]["command"], "python3")
        self.assertIn("other-external-server", claude_doc["mcpServers"])

    def test_detection_of_external_modifications_to_managed_item(self):
        """External modification to a managed server's command/args triggers conflict error."""
        # 1. Add demo-srv
        self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])

        # 2. External actor manually tampers with the args in ~/.claude.json
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        claude_doc["mcpServers"]["demo-srv"]["args"] = ["tampered.js"]
        self.claude_config.write_text(json.dumps(claude_doc, indent=2), encoding="utf-8")

        # 3. Trying to update fails closed
        res_up = self._run_cli(["--action", "update", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertNotEqual(res_up.returncode, 0)
        self.assertIn("外部文件并发变更: 受管项 'demo-srv' 被外部修改，拒绝覆盖", res_up.stderr)

    def test_detection_of_external_modifications_to_env_value(self):
        """External modification to a managed server's env value triggers conflict error."""
        # 1. Add demo-srv
        self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])

        # 2. External actor tampers with API_TOKEN directly in ~/.claude.json
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        claude_doc["mcpServers"]["demo-srv"]["env"]["API_TOKEN"] = "tampered_token_by_external_actor"
        self.claude_config.write_text(json.dumps(claude_doc, indent=2), encoding="utf-8")

        # 3. Trying to update fails closed
        res_up = self._run_cli(["--action", "update", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertNotEqual(res_up.returncode, 0)
        self.assertIn("外部文件并发变更: 受管项 'demo-srv' 被外部修改，拒绝覆盖", res_up.stderr)

    def test_shared_target_rejected_early(self):
        """Shared target path or symlink between agents is rejected early with McpSharedTargetError."""
        # Point opencode to same path as claude
        defs = copy.deepcopy(self.base_definitions)
        defs["agent_paths"]["opencode"] = str(self.claude_config)
        self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")

        res = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 2)
        self.assertIn("共享", res.stderr)
        self.assertIn("拒绝单 Agent 操作以保证隔离", res.stderr)

    def test_corrupt_db_and_invalid_args_fail_fast(self):
        """Corrupt DB or invalid args fail early with exit code 2."""
        # 1. Missing --agent
        res1 = self._run_cli(["--action", "add", "--only", "demo-srv"])
        self.assertEqual(res1.returncode, 2)

        # 2. Missing --only or multiple --only
        res2 = self._run_cli(["--action", "add", "--agent", "claude-code"])
        self.assertEqual(res2.returncode, 2)

        res3 = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv", "--only", "other-srv"])
        self.assertEqual(res3.returncode, 2)

        # 3. Invalid / unknown agent
        res4 = self._run_cli(["--action", "add", "--agent", "nonexistent-agent", "--only", "demo-srv"])
        self.assertEqual(res4.returncode, 2)

        # 4. Unknown MCP server
        res5 = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "nonexistent-srv"])
        self.assertEqual(res5.returncode, 2)

        # 5. Corrupt DB
        self.db_path.write_text("corrupted sqlite database content", encoding="utf-8")
        res6 = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res6.returncode, 2)
        self.assertIn("数据库未就绪或损坏", res6.stderr)

    def test_zero_plaintext_credentials_in_database(self):
        """Verify secrets are NEVER stored in plaintext in SQLite policy_entries or operation_logs."""
        res = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)

        # Claude config HAS the injected secret
        claude_content = self.claude_config.read_text(encoding="utf-8")
        self.assertIn("very_secret_token_val_9999", claude_content)

        # Now inspect manager.db SQLite binary and dump
        conn = connect_ready_db(self.db_path)
        policy_rows = conn.execute("SELECT * FROM policy_entries").fetchall()
        op_rows = conn.execute("SELECT * FROM operation_logs").fetchall()
        conn.close()

        # Secret value must NEVER appear in policy_entries
        for r in policy_rows:
            val = r["value_json"]
            self.assertNotIn("very_secret_token_val_9999", val)
            self.assertIn("${DEMO_TOKEN}", val)

        # Secret value must NEVER appear in operation_logs
        for r in op_rows:
            detail = r["detail_json"] or ""
            target = r["target"] or ""
            self.assertNotIn("very_secret_token_val_9999", detail)
            self.assertNotIn("very_secret_token_val_9999", target)

        # Raw DB file inspection
        db_raw = self.db_path.read_bytes()
        self.assertNotIn(b"very_secret_token_val_9999", db_raw)

    def test_manage_mcp_config_db_failure_rolls_back_config(self):
        """manage_mcp_config rolls back agent config when DB commit fails during adoption/sync."""
        # Setup Claude config with an existing matching server that triggers adoption
        self.claude_config.write_text(json.dumps({
            "mcpServers": {
                "demo-srv": {
                    "command": "node",
                    "args": ["demo.js", "--port", "3000"],
                    "env": {"API_TOKEN": "very_secret_token_val_9999", "_MANAGED_BY": "agent-manager"}
                }
            }
        }, indent=2), encoding="utf-8")
        original_bytes = self.claude_config.read_bytes()

        agents_cfg = {"claude-code": str(self.claude_config)}
        mcp_servers = {
            "demo-srv": {
                "command": "node",
                "args": ["demo.js", "--port", "3000"],
                "env": {"API_TOKEN": "very_secret_token_val_9999"},
                "enabled": True,
                "_declared_in_policy": True,
            }
        }
        raw_mcp = {
            "demo-srv": {
                "command": "node",
                "args": ["demo.js", "--port", "3000"],
                "env": {"API_TOKEN": "${DEMO_TOKEN}"},
            }
        }
        with mock.patch.object(sync_mcp, "save_mcp_bindings", side_effect=sqlite3.OperationalError("Simulated DB lock")):
            ok = sync_mcp.manage_mcp_config(
                agents_cfg,
                mcp_servers,
                dry_run=False,
                raw_mcp_servers=raw_mcp,
                env_vars={"DEMO_TOKEN": "very_secret_token_val_9999"},
            )
            self.assertFalse(ok)

        # Claude config must be rolled back to original bytes
        self.assertEqual(self.claude_config.read_bytes(), original_bytes)

    def test_execute_agent_action_external_modification_after_parse_aborts_write(self):
        """execute_agent_action aborts and preserves external changes when file is modified after parse."""
        # 1. Test JSON (claude-code)
        self.claude_config.write_text('{"mcpServers": {}}', encoding="utf-8")
        real_resolve = sync_mcp.resolve_mcp_server_args

        def tamper_claude(definition, agent_name):
            # Inject external modification after initial_old_bytes was read and checks passed
            self.claude_config.write_text('{"external_race_content": 999}', encoding="utf-8")
            return real_resolve(definition, agent_name)

        agent_meta = {"claude-code": str(self.claude_config)}
        raw_defs = {"demo-srv": {"command": "node", "args": ["demo.js"], "env": {}}}
        defs = {"demo-srv": {"command": "node", "args": ["demo.js"], "env": {}, "enabled": True}}

        with mock.patch.object(sync_mcp, "resolve_mcp_server_args", side_effect=tamper_claude):
            with self.assertLogs(level="ERROR") as cm:
                ok = sync_mcp.execute_agent_action(
                    action="add",
                    target_agent="claude-code",
                    target_server="demo-srv",
                    raw_definition_cfg=raw_defs,
                    definition_cfg=defs,
                    AGENT_TARGETS=agent_meta,
                    agent_defs={},
                    env_vars={},
                    base_dir=self.temp_dir,
                )
            self.assertFalse(ok)
            self.assertTrue(any("外部文件并发变更" in log for log in cm.output))

        # External content must NOT be overwritten
        self.assertEqual(self.claude_config.read_text(encoding="utf-8"), '{"external_race_content": 999}')
        # DB binding must NOT exist
        loaded = load_mcp_bindings(self.db_path)
        self.assertIsNone(get_binding(loaded, "demo-srv", "claude-code"))

        # 2. Test TOML (codex)
        self.codex_config.write_text('# initial codex config\n', encoding="utf-8")

        def tamper_codex(definition, agent_name):
            self.codex_config.write_text('# concurrent external edit\nexternal_flag = true\n', encoding="utf-8")
            return real_resolve(definition, agent_name)

        agent_meta_codex = {"codex": {"path": str(self.codex_config), "format": "toml"}}

        with mock.patch.object(sync_mcp, "resolve_mcp_server_args", side_effect=tamper_codex):
            with self.assertLogs(level="ERROR") as cm:
                ok = sync_mcp.execute_agent_action(
                    action="add",
                    target_agent="codex",
                    target_server="demo-srv",
                    raw_definition_cfg=raw_defs,
                    definition_cfg=defs,
                    AGENT_TARGETS=agent_meta_codex,
                    agent_defs={},
                    env_vars={},
                    base_dir=self.temp_dir,
                )
            self.assertFalse(ok)
            self.assertTrue(any("外部文件并发变更" in log for log in cm.output))

        # External content must NOT be overwritten
        self.assertIn("concurrent external edit", self.codex_config.read_text(encoding="utf-8"))
        loaded_codex = load_mcp_bindings(self.db_path)
        self.assertIsNone(get_binding(loaded_codex, "demo-srv", "codex"))

    def test_manage_mcp_config_external_modification_after_parse_aborts_sync(self):
        """manage_mcp_config aborts without overwriting when file is modified after old_bytes is captured."""
        # Setup initial DB bindings for both agents so manage_mcp_config actively syncs demo-srv
        bindings = {}
        pinned_def = {"command": "node", "args": ["demo.js"], "env": {}}
        pinned_version = compute_definition_version(pinned_def)
        set_binding(bindings, "demo-srv", "claude-code", version=pinned_version, definition=pinned_def, enabled=True, assigned=True)
        set_binding(bindings, "demo-srv", "codex", version=pinned_version, definition=pinned_def, enabled=True, assigned=True)
        save_mcp_bindings(self.db_path, bindings, action="add")

        # 1. Test JSON
        self.claude_config.write_text('{"mcpServers": {}}', encoding="utf-8")
        real_resolve = sync_mcp.resolve_mcp_server_args

        def tamper_during_sync(definition, agent_name):
            self.claude_config.write_text('{"tampered_in_middle_of_sync": true}', encoding="utf-8")
            return real_resolve(definition, agent_name)

        agents_cfg = {"claude-code": str(self.claude_config)}
        mcp_servers = {
            "demo-srv": {
                "command": "node",
                "args": ["demo.js"],
                "env": {},
                "enabled": True,
                "_declared_in_policy": True,
            }
        }

        with mock.patch.object(sync_mcp, "resolve_mcp_server_args", side_effect=tamper_during_sync):
            with self.assertLogs(level="ERROR") as cm:
                ok = sync_mcp.manage_mcp_config(
                    agents_cfg,
                    mcp_servers,
                    dry_run=False,
                )
            self.assertFalse(ok)
            self.assertTrue(any("外部文件并发变更" in log for log in cm.output))

        # External modification must be preserved intact
        self.assertEqual(self.claude_config.read_text(encoding="utf-8"), '{"tampered_in_middle_of_sync": true}')

        # 2. Test TOML
        self.codex_config.write_text('# initial codex\n', encoding="utf-8")

        def tamper_codex_sync(definition, agent_name):
            self.codex_config.write_text('# externally modified during toml sync\n', encoding="utf-8")
            return real_resolve(definition, agent_name)

        agents_cfg_codex = {"codex": {"path": str(self.codex_config), "format": "toml"}}

        with mock.patch.object(sync_mcp, "resolve_mcp_server_args", side_effect=tamper_codex_sync):
            with self.assertLogs(level="ERROR") as cm:
                ok = sync_mcp.manage_mcp_config(
                    agents_cfg_codex,
                    mcp_servers,
                    dry_run=False,
                )
            self.assertFalse(ok)
            self.assertTrue(any("外部文件并发变更" in log for log in cm.output))

        self.assertIn("externally modified during toml sync", self.codex_config.read_text(encoding="utf-8"))

    def test_execute_agent_action_render_failure_no_temp_no_db(self):
        """渲染在临时文件创建前失败：原文件/DB 不变，目录无 tmp/rb 残留。"""
        self.claude_config.write_text('{"mcpServers": {}}', encoding="utf-8")
        original_bytes = self.claude_config.read_bytes()
        agent_meta = {"claude-code": str(self.claude_config)}
        raw_defs = {"demo-srv": {"command": "node", "args": ["demo.js"], "env": {}}}
        defs = {"demo-srv": {"command": "node", "args": ["demo.js"], "env": {}, "enabled": True}}

        with mock.patch.object(sync_mcp, "render_single_server_json", side_effect=ValueError("simulated render crash")):
            with self.assertLogs(level="ERROR"):
                ok = sync_mcp.execute_agent_action(
                    action="add",
                    target_agent="claude-code",
                    target_server="demo-srv",
                    raw_definition_cfg=raw_defs,
                    definition_cfg=defs,
                    AGENT_TARGETS=agent_meta,
                    agent_defs={},
                    env_vars={},
                    base_dir=self.temp_dir,
                )
            self.assertFalse(ok)

        self.assertEqual(self.claude_config.read_bytes(), original_bytes)
        self.assertIsNone(get_binding(load_mcp_bindings(self.db_path), "demo-srv", "claude-code"))
        leftovers = list(self.home_dir.glob(".claude.json.tmp_*")) + list(self.home_dir.glob(".claude.json.rb_*"))
        self.assertEqual(leftovers, [])

    def test_execute_agent_action_verify_digest_rejects_marker_retained_tamper(self):
        """写入内容保留 marker 但条目被篡改时，读回摘要校验判失败并回滚原文件、不落 DB。"""
        original_content = '{"mcpServers": {}, "keep": 1}'
        self.claude_config.write_text(original_content, encoding="utf-8")
        agent_meta = {"claude-code": str(self.claude_config)}
        raw_defs = {"demo-srv": {"command": "node", "args": ["demo.js"], "env": {}}}
        defs = {"demo-srv": {"command": "node", "args": ["demo.js"], "env": {}, "enabled": True}}
        real_render = sync_mcp.render_single_server_json

        def tampered_render(config_path, agent_name, server_name, settings, old_bytes=UNSET):
            rendered = real_render(config_path, agent_name, server_name, settings, old_bytes=old_bytes)
            doc = json.loads(rendered)
            # 保留 _MANAGED_BY marker，但篡改 args（模拟写出即被外部改动的条目）
            doc["mcpServers"][server_name]["args"] = ["tampered.js"]
            return json.dumps(doc, indent=2, ensure_ascii=False) + "\n"

        with mock.patch.object(sync_mcp, "render_single_server_json", side_effect=tampered_render):
            with self.assertLogs(level="ERROR"):
                ok = sync_mcp.execute_agent_action(
                    action="add",
                    target_agent="claude-code",
                    target_server="demo-srv",
                    raw_definition_cfg=raw_defs,
                    definition_cfg=defs,
                    AGENT_TARGETS=agent_meta,
                    agent_defs={},
                    env_vars={},
                    base_dir=self.temp_dir,
                )
            self.assertFalse(ok)

        # 原文件完整回滚，DB 无绑定
        self.assertEqual(self.claude_config.read_text(encoding="utf-8"), original_content)
        self.assertIsNone(get_binding(load_mcp_bindings(self.db_path), "demo-srv", "claude-code"))

    def test_external_unknown_field_addition_rejected_json_and_opencode(self):
        """受管条目被外部新增未知字段或改动 type（保留 marker）后，update 拒绝覆盖。"""
        res = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)

        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertEqual(claude_doc["mcpServers"]["demo-srv"]["env"]["_MANAGED_BY"], "agent-manager")
        claude_doc["mcpServers"]["demo-srv"]["cwd"] = "/tmp/external-injected"
        self.claude_config.write_text(json.dumps(claude_doc, indent=2), encoding="utf-8")

        res_up = self._run_cli(["--action", "update", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertNotEqual(res_up.returncode, 0)
        self.assertIn("被外部修改，拒绝覆盖", res_up.stderr)
        # 外部新增字段原样保留，未被静默丢弃
        claude_doc_after = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertEqual(claude_doc_after["mcpServers"]["demo-srv"]["cwd"], "/tmp/external-injected")

        res_oc = self._run_cli(["--action", "add", "--agent", "opencode", "--only", "demo-srv"])
        self.assertEqual(res_oc.returncode, 0, res_oc.stderr)
        oc_doc = json.loads(self.opencode_config.read_text(encoding="utf-8"))
        oc_doc["mcp"]["demo-srv"]["type"] = "remote"
        self.opencode_config.write_text(json.dumps(oc_doc, indent=2), encoding="utf-8")

        res_oc_up = self._run_cli(["--action", "update", "--agent", "opencode", "--only", "demo-srv"])
        self.assertNotEqual(res_oc_up.returncode, 0)
        self.assertIn("被外部修改，拒绝覆盖", res_oc_up.stderr)
        oc_doc_after = json.loads(self.opencode_config.read_text(encoding="utf-8"))
        self.assertEqual(oc_doc_after["mcp"]["demo-srv"]["type"], "remote")

    def test_manage_mcp_config_scoped_save_conflict_rolls_back(self):
        """普通 sync 期间同键被并发修改：scoped 事务内 expected 对比失败，配置回滚且不覆盖并发值。"""
        res = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)
        claude_bytes_before = self.claude_config.read_bytes()

        # 使绑定 applied_digest 过期，触发 sync 产生 scoped 变更
        bindings = load_mcp_bindings(self.db_path)
        old_entry = copy.deepcopy(get_binding(bindings, "demo-srv", "claude-code"))
        set_binding(bindings, "demo-srv", "claude-code", version=old_entry["version"], definition=old_entry["definition"], enabled=True, assigned=True, applied_digest="e" * 64)
        save_mcp_bindings(self.db_path, bindings, action="update", target="demo-srv@claude-code", server="demo-srv", agent="claude-code", expected_entry=old_entry)

        real_save = sync_mcp.save_mcp_bindings

        def racing_save(db_path, bindings_arg, **kwargs):
            # 在 agent-a 提交前，并发进程翻转同一键的 enabled
            concurrent = load_mcp_bindings(self.db_path)
            cur = copy.deepcopy(get_binding(concurrent, "demo-srv", "claude-code"))
            set_binding(concurrent, "demo-srv", "claude-code", version=cur["version"], definition=cur["definition"], enabled=False, assigned=True, applied_digest=cur["applied_digest"])
            real_save(self.db_path, concurrent, action="disable", target="demo-srv@claude-code", server="demo-srv", agent="claude-code", expected_entry=cur)
            return real_save(db_path, bindings_arg, **kwargs)

        mcp_servers = {
            "demo-srv": {
                "command": "node",
                "args": ["demo.js", "--port", "3000"],
                "env": {"API_TOKEN": "very_secret_token_val_9999"},
                "enabled": True,
                "_declared_in_policy": True,
            }
        }
        agents_cfg = {"claude-code": str(self.claude_config)}
        with mock.patch.object(sync_mcp, "save_mcp_bindings", side_effect=racing_save):
            with self.assertLogs(level="ERROR") as cm:
                ok = sync_mcp.manage_mcp_config(
                    agents_cfg,
                    mcp_servers,
                    dry_run=False,
                    env_vars={"DEMO_TOKEN": "very_secret_token_val_9999"},
                )
            self.assertFalse(ok)
            self.assertTrue(any("并发冲突" in log for log in cm.output))

        # 配置回滚到 sync 前字节，并发进程的 enabled=False 未被 stale 快照覆盖
        self.assertEqual(self.claude_config.read_bytes(), claude_bytes_before)
        final = load_mcp_bindings(self.db_path)
        self.assertFalse(get_binding(final, "demo-srv", "claude-code")["enabled"])

    def test_manage_mcp_config_bad_nested_db_data_fail_closed(self):
        """DB 中坏嵌套绑定数据：普通 sync fail-closed，不写任何配置文件。"""
        claude_bytes_before = self.claude_config.read_bytes()
        codex_bytes_before = self.codex_config.read_bytes()
        conn = connect_ready_db(self.db_path)
        try:
            with conn:
                conn.execute(
                    "INSERT INTO policy_entries (key, value_json, updated_at) VALUES (?, ?, ?) "
                    "ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
                    (MCP_BINDINGS_POLICY_KEY, json.dumps({"demo-srv": {"claude-code": {"version": "v1", "definition": {"command": "node"}, "enabled": "false", "assigned": True, "applied_digest": ""}}}), "2026-09-08T00:00:00Z"),
                )
        finally:
            conn.close()
        db_bytes_before = self.db_path.read_bytes()

        mcp_servers = {
            "demo-srv": {
                "command": "node",
                "args": ["demo.js", "--port", "3000"],
                "env": {},
                "enabled": True,
                "_declared_in_policy": True,
            }
        }
        agents_cfg = {
            "claude-code": str(self.claude_config),
            "codex": {"path": str(self.codex_config), "format": "toml"},
        }
        with self.assertLogs(level="ERROR"):
            ok = sync_mcp.manage_mcp_config(agents_cfg, mcp_servers, dry_run=False)
        self.assertFalse(ok)
        self.assertEqual(self.claude_config.read_bytes(), claude_bytes_before)
        self.assertEqual(self.codex_config.read_bytes(), codex_bytes_before)
        self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

    def test_external_command_tamper_rejected_across_all_three_formats_and_five_actions(self):
        """add 后外部修改 command 但保留 _MANAGED_BY marker：
        disable / remove / enable / update (含 --dry-run) 均拒绝覆盖（返回码非 0），
        原配置文件和数据库保持原样不变。测试覆盖 JSON (claude-code)、TOML (codex)、OpenCode 三种格式。
        """
        targets = [
            ("claude-code", self.claude_config, "json"),
            ("codex", self.codex_config, "toml"),
            ("opencode", self.opencode_config, "opencode"),
        ]

        for agent, cfg_path, fmt in targets:
            with self.subTest(agent=agent, format=fmt):
                # 1. 正常 add
                res_add = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertEqual(res_add.returncode, 0, f"[{agent}] add 失败: {res_add.stderr}")

                # 2. 外部篡改 command（保留 _MANAGED_BY marker）
                if fmt == "json":
                    doc = json.loads(cfg_path.read_text(encoding="utf-8"))
                    doc["mcpServers"]["demo-srv"]["command"] = "node-tampered"
                    cfg_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")
                elif fmt == "opencode":
                    doc = json.loads(cfg_path.read_text(encoding="utf-8"))
                    doc["mcp"]["demo-srv"]["command"] = ["node-tampered", "demo.js", "--port", "3000"]
                    cfg_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")
                elif fmt == "toml":
                    content = cfg_path.read_text(encoding="utf-8")
                    self.assertIn('command = "node"', content)
                    tampered = content.replace('command = "node"', 'command = "node-tampered"')
                    cfg_path.write_text(tampered, encoding="utf-8")

                tampered_bytes = cfg_path.read_bytes()
                db_bytes_before = self.db_path.read_bytes()

                # 3. 对 4 个修改动作逐一测试 dry-run 与真实执行，必须全部拒绝，且文件/DB 保持未被修改
                for action in ("disable", "remove", "enable", "update"):
                    # dry-run 必须拒绝
                    res_dry = self._run_cli(["--dry-run", "--action", action, "--agent", agent, "--only", "demo-srv"])
                    self.assertNotEqual(res_dry.returncode, 0, f"[{agent}] dry-run {action} 应拒绝但返回 0")
                    self.assertIn("外部文件并发变更", res_dry.stderr)
                    self.assertEqual(cfg_path.read_bytes(), tampered_bytes)
                    self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

                    # 实际执行必须拒绝
                    res_act = self._run_cli(["--action", action, "--agent", agent, "--only", "demo-srv"])
                    self.assertNotEqual(res_act.returncode, 0, f"[{agent}] 实际 {action} 应拒绝但返回 0")
                    self.assertIn("外部文件并发变更", res_act.stderr)
                    self.assertEqual(cfg_path.read_bytes(), tampered_bytes)
                    self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

                # add 也必须拒绝（已分配）
                res_readd_conflict = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertEqual(res_readd_conflict.returncode, 2)
                self.assertEqual(cfg_path.read_bytes(), tampered_bytes)
                self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

    def test_external_null_unknown_field_and_env_type_tamper_rejected_across_five_actions_json_and_opencode(self):
        """主独立真实反例覆盖：
        1. 受管项被外部注入 unknown 字段 user_note: null，保留 marker。
        2. 受管项环境值类型被外部修改为非字符串（如整数 8080），保留 marker。
        对 JSON (claude-code) 与 OpenCode 两种格式：
        disable / remove / enable / update (含 --dry-run) 及 add 均拒绝覆盖（返回码非 0），
        外部文件 bytes 100% 保持不变，数据库亦保持不变。
        """
        cases = [
            ("claude-code", self.claude_config, "json", "unknown_null"),
            ("claude-code", self.claude_config, "json", "env_type"),
            ("opencode", self.opencode_config, "opencode", "unknown_null"),
            ("opencode", self.opencode_config, "opencode", "env_type"),
        ]

        for agent, cfg_path, fmt, tamper_kind in cases:
            with self.subTest(agent=agent, format=fmt, tamper=tamper_kind):
                # 重置环境与数据库
                self.db_path.unlink(missing_ok=True)
                _init_test_db(self.db_path)
                cfg_path.write_text("{}", encoding="utf-8")

                # 1. 正常 add
                res_add = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertEqual(res_add.returncode, 0, f"[{agent}] add 失败: {res_add.stderr}")

                # 2. 外部篡改（保留 _MANAGED_BY marker）
                doc = json.loads(cfg_path.read_text(encoding="utf-8"))
                container_key = "mcp" if fmt == "opencode" else "mcpServers"
                srv_dict = doc[container_key]["demo-srv"]

                if tamper_kind == "unknown_null":
                    # 新增 user_note: null
                    srv_dict["user_note"] = None
                elif tamper_kind == "env_type":
                    # 修改环境值类型为数值
                    env_key = "environment" if fmt == "opencode" else "env"
                    srv_dict[env_key]["PORT"] = 8080

                cfg_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")
                tampered_bytes = cfg_path.read_bytes()
                db_bytes_before = self.db_path.read_bytes()

                # 3. 对 disable / remove / enable / update 测试 dry-run 与真实执行，必须全部拒绝，保留外部 bytes
                for action in ("disable", "remove", "enable", "update"):
                    # dry-run 必须拒绝
                    res_dry = self._run_cli(["--dry-run", "--action", action, "--agent", agent, "--only", "demo-srv"])
                    self.assertNotEqual(res_dry.returncode, 0, f"[{agent}][{tamper_kind}] dry-run {action} 应拒绝但返回 0: {res_dry.stdout}")
                    self.assertEqual(cfg_path.read_bytes(), tampered_bytes)
                    self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

                    # 实际执行必须拒绝
                    res_act = self._run_cli(["--action", action, "--agent", agent, "--only", "demo-srv"])
                    self.assertNotEqual(res_act.returncode, 0, f"[{agent}][{tamper_kind}] 实际 {action} 应拒绝但返回 0: {res_act.stdout}")
                    self.assertEqual(cfg_path.read_bytes(), tampered_bytes)
                    self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

                # add 也必须拒绝（已存在且被篡改 / 已分配）
                res_add_again = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertNotEqual(res_add_again.returncode, 0)
                self.assertEqual(cfg_path.read_bytes(), tampered_bytes)
                self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

    def test_normal_sync_preserves_file_and_db_on_external_tamper_command_null_env(self):
        """普通 sync 遇到外部篡改 (command / null未知字段 / env) fail-closed：
        文件原字节 100% 保留（不被误当孤儿移除，也不被序列化重写），DB 完全不变，返回非 0。
        """
        cases = [
            ("claude-code", self.claude_config, "command"),
            ("claude-code", self.claude_config, "null_field"),
            ("claude-code", self.claude_config, "env_val"),
            ("opencode", self.opencode_config, "command"),
            ("opencode", self.opencode_config, "null_field"),
            ("codex", self.codex_config, "command"),
        ]

        for agent, cfg_path, tamper_kind in cases:
            with self.subTest(agent=agent, tamper=tamper_kind):
                # 重置环境与数据库
                self.db_path.unlink(missing_ok=True)
                _init_test_db(self.db_path)
                cfg_path.write_text("{}", encoding="utf-8") if agent != "codex" else cfg_path.write_text("", encoding="utf-8")

                # 1. 显式添加 demo-srv
                res_add = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertEqual(res_add.returncode, 0, f"[{agent}] add 失败: {res_add.stderr}")

                # 2. 外部篡改受管项（保留 marker）
                if agent == "codex":
                    content = cfg_path.read_text(encoding="utf-8")
                    tampered = content.replace('command = "node"', 'command = "node-tampered"')
                    cfg_path.write_text(tampered, encoding="utf-8")
                elif agent == "opencode":
                    doc = json.loads(cfg_path.read_text(encoding="utf-8"))
                    if tamper_kind == "command":
                        doc["mcp"]["demo-srv"]["command"] = ["node-tampered", "demo.js"]
                    elif tamper_kind == "null_field":
                        doc["mcp"]["demo-srv"]["user_note"] = None
                    elif tamper_kind == "env_val":
                        doc["mcp"]["demo-srv"]["environment"]["PORT"] = "9999"
                    cfg_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")
                else:  # claude-code
                    doc = json.loads(cfg_path.read_text(encoding="utf-8"))
                    if tamper_kind == "command":
                        doc["mcpServers"]["demo-srv"]["command"] = "node-tampered"
                    elif tamper_kind == "null_field":
                        doc["mcpServers"]["demo-srv"]["user_note"] = None
                    elif tamper_kind == "env_val":
                        doc["mcpServers"]["demo-srv"]["env"]["PORT"] = "9999"
                    cfg_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")

                tampered_bytes = cfg_path.read_bytes()
                db_bytes_before = self.db_path.read_bytes()

                # 3. 运行普通 sync（无 --action）
                res_sync = self._run_cli(["--offline", "--agent", agent])
                self.assertNotEqual(res_sync.returncode, 0, f"[{agent}][{tamper_kind}] 普通 sync 应当失败但返回 0")
                self.assertIn("外部文件并发变更", res_sync.stderr)

                # 核心断言：外部 bytes 100% 原样保留，受管项未被当作孤儿移除，DB 也未被更新
                self.assertEqual(cfg_path.read_bytes(), tampered_bytes, f"[{agent}][{tamper_kind}] 文件字节被改动")
                self.assertEqual(self.db_path.read_bytes(), db_bytes_before, f"[{agent}][{tamper_kind}] DB 被改动")

    def test_normal_sync_process_env_only_succeeds_and_preserves_bound_servers(self):
        """只有 process env 提供 DEMO_TOKEN、无 .env / mcp.env 时：
        显式 add 后 normal sync exit 0 且保留 demo-srv，不被当作孤儿删除。
        """
        # 移除 secrets 文件，确保 env_vars 为空
        (self.secrets_dir / "mcp.env").unlink(missing_ok=True)
        test_token = "process_env_only_token_value_abc123"
        env_extra = {"DEMO_TOKEN": test_token}

        # 1. 显式 add demo-srv
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"], env_extra=env_extra)
        self.assertEqual(res_add.returncode, 0, f"add 失败: {res_add.stderr}")

        # 2. 运行普通 sync（无 --action）
        res_sync = self._run_cli(["--offline", "--agent", "claude-code"], env_extra=env_extra)
        self.assertEqual(res_sync.returncode, 0, f"普通 sync 失败: {res_sync.stderr}")

        # 3. 验证 demo-srv 未被当作孤儿删除且凭证正确注入
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertIn("demo-srv", claude_doc.get("mcpServers", {}))
        self.assertEqual(claude_doc["mcpServers"]["demo-srv"]["env"]["API_TOKEN"], test_token)

        # 4. 验证 DB 绑定一致
        bindings = load_mcp_bindings(self.db_path)
        entry = get_binding(bindings, "demo-srv", "claude-code")
        self.assertIsNotNone(entry)
        self.assertTrue(entry.get("assigned"))
        expected_digest = compute_applied_digest(
            claude_doc["mcpServers"]["demo-srv"]["command"],
            claude_doc["mcpServers"]["demo-srv"]["args"],
            claude_doc["mcpServers"]["demo-srv"]["env"],
            claude_doc["mcpServers"]["demo-srv"],
        )
        self.assertEqual(entry.get("applied_digest"), expected_digest)

    def test_normal_sync_credential_rotation_succeeds_without_false_conflict_and_no_secret_leaked(self):
        """凭据轮换场景：
        先用 old_token add，再用 new_token 执行普通 sync。
        必须先对旧 binding 摘要校验外部文件成功，随后计算并写入新摘要；
        不误报外部篡改冲突，文件与 DB 均成功更新，日志绝不泄露凭据明文。
        """
        (self.secrets_dir / "mcp.env").unlink(missing_ok=True)
        old_token = "initial_rotating_secret_val_111"
        new_token = "updated_rotating_secret_val_222"

        # 1. 用 old_token 添加
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"], env_extra={"DEMO_TOKEN": old_token})
        self.assertEqual(res_add.returncode, 0, f"add 失败: {res_add.stderr}")

        bindings_before = load_mcp_bindings(self.db_path)
        old_digest = bindings_before["demo-srv"]["claude-code"]["applied_digest"]

        # 2. 用 new_token 运行普通 sync
        res_sync = self._run_cli(["--offline", "--agent", "claude-code"], env_extra={"DEMO_TOKEN": new_token})
        self.assertEqual(res_sync.returncode, 0, f"普通 sync 凭据轮换失败: {res_sync.stderr}")
        self.assertNotIn("外部文件并发变更", res_sync.stderr)
        self.assertNotIn("拒绝覆盖", res_sync.stderr)

        # 3. 日志与输出绝不泄露任何机密明文
        combined_output = res_sync.stdout + res_sync.stderr
        self.assertNotIn(old_token, combined_output)
        self.assertNotIn(new_token, combined_output)

        # 4. 验证配置文件更新为 new_token
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertEqual(claude_doc["mcpServers"]["demo-srv"]["env"]["API_TOKEN"], new_token)

        # 5. 验证 DB 绑定摘要随之更新
        bindings_after = load_mcp_bindings(self.db_path)
        new_digest = bindings_after["demo-srv"]["claude-code"]["applied_digest"]
        self.assertNotEqual(old_digest, new_digest)
        expected_digest = compute_applied_digest(
            claude_doc["mcpServers"]["demo-srv"]["command"],
            claude_doc["mcpServers"]["demo-srv"]["args"],
            claude_doc["mcpServers"]["demo-srv"]["env"],
            claude_doc["mcpServers"]["demo-srv"],
        )
        self.assertEqual(new_digest, expected_digest)

    def test_normal_sync_credential_lost_rejected_and_preserves_file_and_db(self):
        """凭据丢失场景：
        先用 token add，随后在无 .env/mcp.env 且无系统环境提供该变量时运行普通 sync。
        必须因存在未解析变量而拒绝更新 (exit 1)，文件保持原状（不被移除孤儿），DB 保持原状。
        """
        (self.secrets_dir / "mcp.env").unlink(missing_ok=True)
        token = "temporary_token_for_loss_test_789"

        # 1. 显式 add
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"], env_extra={"DEMO_TOKEN": token})
        self.assertEqual(res_add.returncode, 0, f"add 失败: {res_add.stderr}")

        bytes_before = self.claude_config.read_bytes()
        db_bytes_before = self.db_path.read_bytes()

        # 2. 丢失凭据运行普通 sync
        res_sync = self._run_cli(["--offline", "--agent", "claude-code"], env_extra={})
        self.assertNotEqual(res_sync.returncode, 0, "凭据丢失时普通 sync 应失败")
        # main() 的声明级 fail-fast 门或 manage_mcp_config 的逐 Agent 检查均可拒绝
        self.assertTrue(
            "未解析的占位符" in res_sync.stderr or "未解析的凭证变量" in res_sync.stderr,
            f"缺少未解析变量拒绝日志: {res_sync.stderr}",
        )
        self.assertIn("DEMO_TOKEN", res_sync.stderr)
        self.assertNotIn(token, res_sync.stdout + res_sync.stderr)

        # 3. 验证文件原样保留且 DB 未更新
        self.assertEqual(self.claude_config.read_bytes(), bytes_before)
        self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

    def test_normal_sync_rejects_external_tamper_on_scope_narrowing_paths(self):
        """真实 add → 外部改 command 保留 marker → scope 收紧（binding 禁用 / 全局 off /
        allowlist 越界）→ 普通 sync（含 --dry-run）fail-closed：
        返回非 0、报外部文件并发变更，目标文件与 DB 字节 100% 不变。
        撤下路径只允许撤下与旧 binding 一致的本工具项。
        """
        scenarios = [
            ("claude-code", self.claude_config, "json", "binding_disabled"),
            ("claude-code", self.claude_config, "json", "global_off"),
            ("claude-code", self.claude_config, "json", "allowlist_excluded"),
            ("codex", self.codex_config, "toml", "global_off"),
            ("codex", self.codex_config, "toml", "allowlist_excluded"),
        ]

        for agent, cfg_path, fmt, scenario in scenarios:
            with self.subTest(agent=agent, scenario=scenario):
                # 重置环境、策略与数据库
                self.db_path.unlink(missing_ok=True)
                _init_test_db(self.db_path)
                cfg_path.write_text("" if fmt == "toml" else "{}", encoding="utf-8")
                self.manager_json.write_text(json.dumps(self.base_policy, indent=2), encoding="utf-8")
                self.mcp_json.write_text(json.dumps(self.base_definitions, indent=2), encoding="utf-8")

                # 1. 真实 add
                res_add = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertEqual(res_add.returncode, 0, f"[{agent}] add 失败: {res_add.stderr}")

                # 2. 外部篡改 command（保留 _MANAGED_BY marker）
                if fmt == "toml":
                    content = cfg_path.read_text(encoding="utf-8")
                    self.assertIn('command = "node"', content)
                    cfg_path.write_text(content.replace('command = "node"', 'command = "node-tampered"'), encoding="utf-8")
                else:
                    doc = json.loads(cfg_path.read_text(encoding="utf-8"))
                    doc["mcpServers"]["demo-srv"]["command"] = "node-tampered"
                    cfg_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")
                tampered_bytes = cfg_path.read_bytes()

                # 3. scope 收紧
                if scenario == "binding_disabled":
                    bindings = load_mcp_bindings(self.db_path)
                    entry = copy.deepcopy(get_binding(bindings, "demo-srv", agent))
                    set_binding(bindings, "demo-srv", agent, version=entry["version"], definition=entry["definition"], enabled=False, assigned=True, applied_digest=entry["applied_digest"])
                    save_mcp_bindings(self.db_path, bindings, action="disable", target=f"demo-srv@{agent}", server="demo-srv", agent=agent, expected_entry=entry)
                elif scenario == "global_off":
                    policy = json.loads(self.manager_json.read_text(encoding="utf-8"))
                    policy["mcp"]["demo-srv"]["enabled"] = False
                    self.manager_json.write_text(json.dumps(policy, indent=2), encoding="utf-8")
                elif scenario == "allowlist_excluded":
                    defs = json.loads(self.mcp_json.read_text(encoding="utf-8"))
                    defs["servers"]["demo-srv"]["agents"] = ["codex" if agent != "codex" else "claude-code"]
                    self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")
                db_bytes_before = self.db_path.read_bytes()

                # 4. dry-run 与真实普通 sync 均必须拒绝，文件/DB 字节不变
                for extra in (["--dry-run"], []):
                    res_sync = self._run_cli(extra + ["--offline", "--agent", agent])
                    self.assertNotEqual(res_sync.returncode, 0, f"[{agent}][{scenario}] sync {extra} 应拒绝但返回 0")
                    self.assertIn("外部文件并发变更", res_sync.stderr)
                    self.assertEqual(cfg_path.read_bytes(), tampered_bytes, f"[{agent}][{scenario}] 文件字节被改动")
                    self.assertEqual(self.db_path.read_bytes(), db_bytes_before, f"[{agent}][{scenario}] DB 被改动")

    def test_normal_sync_preserves_marker_orphan_without_historical_binding(self):
        """仅有 marker 但无历史 binding 的孤立项：普通 sync 不得仅凭标记擅删，
        保留原样并告警；覆盖 JSON (claude-code) 与 TOML (codex)。
        """
        self.claude_config.write_text(json.dumps({
            "mcpServers": {
                "ghost-srv": {
                    "command": "ghost",
                    "args": [],
                    "env": {"_MANAGED_BY": "agent-manager"},
                }
            }
        }, indent=2), encoding="utf-8")
        self.codex_config.write_text(
            '[mcp_servers.ghost-srv]\n'
            'command = "ghost"\n'
            'args = []\n'
            '[mcp_servers.ghost-srv.env]\n'
            '_MANAGED_BY = "agent-manager"\n',
            encoding="utf-8",
        )

        res_sync = self._run_cli(["--offline"])
        self.assertEqual(res_sync.returncode, 0, f"普通 sync 失败: {res_sync.stderr}")
        self.assertIn("无历史绑定记录", res_sync.stderr)

        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertIn("ghost-srv", claude_doc.get("mcpServers", {}))
        self.assertEqual(claude_doc["mcpServers"]["ghost-srv"]["command"], "ghost")
        codex_content = self.codex_config.read_text(encoding="utf-8")
        self.assertIn("ghost-srv", codex_content)
        self.assertIn('command = "ghost"', codex_content)

    def test_normal_sync_uninstalled_agent_with_binding_preserves_config_and_db(self):
        """QA 反例：OpenCode 未安装（探测不到程序）时，SQLite 已显式分配且目标文件存在，
        normal sync 不得清空绑定条目：exit 0、配置保留、绑定仍 enabled/assigned。
        legacy（无 DB）未安装清理行为不在此路径。"""
        (self.secrets_dir / "mcp.env").unlink(missing_ok=True)
        token = "process_env_opencode_token_456"
        env_extra = {"DEMO_TOKEN": token}

        # 1. 真实 add 建立绑定（进程环境提供凭证）
        res_add = self._run_cli(["--action", "add", "--agent", "opencode", "--only", "demo-srv"], env_extra=env_extra)
        self.assertEqual(res_add.returncode, 0, f"add 失败: {res_add.stderr}")
        bytes_after_add = self.opencode_config.read_bytes()

        # 2. 普通 sync（本机未安装 opencode，探测 agent_installed=False）
        res_sync = self._run_cli(["--offline", "--agent", "opencode"], env_extra=env_extra)
        self.assertEqual(res_sync.returncode, 0, f"普通 sync 失败: {res_sync.stderr}")

        # 3. 文件条目保留且凭证一致，绑定仍 enabled/assigned
        oc_doc = json.loads(self.opencode_config.read_text(encoding="utf-8"))
        self.assertIn("demo-srv", oc_doc.get("mcp", {}), "已绑定条目不得因未探测到程序被删除")
        self.assertEqual(oc_doc["mcp"]["demo-srv"]["environment"]["API_TOKEN"], token)
        self.assertEqual(self.opencode_config.read_bytes(), bytes_after_add)
        entry = get_binding(load_mcp_bindings(self.db_path), "demo-srv", "opencode")
        self.assertIsNotNone(entry)
        self.assertTrue(entry["enabled"])
        self.assertTrue(entry["assigned"])

    def test_manage_mcp_config_uninstalled_probe_preserves_bound_entries(self):
        """确定性覆盖未安装分支：mock 探测为未安装，normal sync 保留已绑定条目并告警，
        文件与 DB 字节 100% 不变。"""
        (self.secrets_dir / "mcp.env").unlink(missing_ok=True)
        token = "process_env_opencode_token_456"

        res_add = self._run_cli(["--action", "add", "--agent", "opencode", "--only", "demo-srv"], env_extra={"DEMO_TOKEN": token})
        self.assertEqual(res_add.returncode, 0, f"add 失败: {res_add.stderr}")
        bytes_before = self.opencode_config.read_bytes()
        db_bytes_before = self.db_path.read_bytes()

        mcp_servers = {
            "demo-srv": {
                "command": "node",
                "args": ["demo.js", "--port", "3000"],
                "env": {"API_TOKEN": token},
                "enabled": True,
                "_declared_in_policy": True,
            }
        }
        with mock.patch.object(sync_mcp, "is_agent_installed", return_value=False):
            with self.assertLogs(level="WARNING") as cm:
                ok = sync_mcp.manage_mcp_config(
                    {"opencode": str(self.opencode_config)},
                    mcp_servers,
                    dry_run=False,
                    agent_defs={"opencode": {"display_name": "OpenCode"}},
                    env_vars={"DEMO_TOKEN": token},
                )
            self.assertTrue(ok)
            self.assertTrue(any("未探测到程序安装" in log for log in cm.output))

        self.assertEqual(self.opencode_config.read_bytes(), bytes_before)
        self.assertEqual(self.db_path.read_bytes(), db_bytes_before)
        entry = get_binding(load_mcp_bindings(self.db_path), "demo-srv", "opencode")
        self.assertTrue(entry["enabled"])
        self.assertTrue(entry["assigned"])


if __name__ == "__main__":
    unittest.main()
