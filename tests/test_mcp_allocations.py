"""Integration tests for MCP agent allocations and lifecycle workflows."""
from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
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


class McpAgentAllocationTests(McpAllocationBaseTestCase):
    """Integration tests for MCP allocation lifecycle, multi-agent isolation, and formats."""
    def test_immutable_version_isolation_between_two_agents(self):
        """Test requirement: Two agents on v1, definition changes to v2, update Agent A leaves Agent B 100% byte-identical."""
        # 1. Add demo-srv to claude-code and codex at v1
        res1 = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res1.returncode, 0, res1.stderr)
        res2 = self._run_cli(["--action", "add", "--agent", "codex", "--only", "demo-srv"])
        self.assertEqual(res2.returncode, 0, res2.stderr)

        # Snapshot Codex config and hash
        codex_before = self.codex_config.read_bytes()
        codex_before_hash = hashlib.sha256(codex_before).hexdigest()

        # Check DB bindings for both
        bindings = load_mcp_bindings(self.db_path)
        v1 = bindings["demo-srv"]["claude-code"]["version"]
        self.assertEqual(bindings["demo-srv"]["codex"]["version"], v1)

        # 2. Modify definition in mcp.json to v2
        updated_defs = copy.deepcopy(self.base_definitions)
        updated_defs["servers"]["demo-srv"]["args"] = ["demo.js", "--port", "4000", "--v2"]
        self.mcp_json.write_text(json.dumps(updated_defs, indent=2), encoding="utf-8")

        # 3. Explicitly update only claude-code
        res3 = self._run_cli(["--action", "update", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res3.returncode, 0, res3.stderr)

        # 4. Verify Claude received v2
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertEqual(claude_doc["mcpServers"]["demo-srv"]["args"], ["demo.js", "--port", "4000", "--v2"])

        # 5. Verify Codex config file is 100% byte-identical!
        codex_after = self.codex_config.read_bytes()
        codex_after_hash = hashlib.sha256(codex_after).hexdigest()
        self.assertEqual(codex_before_hash, codex_after_hash)
        self.assertEqual(codex_before, codex_after)

        # 6. Verify DB binding for Codex remains at v1 while Claude is at v2
        bindings_after = load_mcp_bindings(self.db_path)
        self.assertEqual(bindings_after["demo-srv"]["codex"]["version"], v1)
        v2 = bindings_after["demo-srv"]["claude-code"]["version"]
        self.assertNotEqual(v1, v2)

        # 7. Running global ordinary sync also preserves Codex at v1
        res_sync = self._run_cli([])
        self.assertEqual(res_sync.returncode, 0, res_sync.stderr)
        self.assertEqual(self.codex_config.read_bytes(), codex_before)
        bindings_sync = load_mcp_bindings(self.db_path)
        self.assertEqual(bindings_sync["demo-srv"]["codex"]["version"], v1)

    def test_lifecycle_and_non_resurrection(self):
        """Test: add -> disable -> enable -> remove lifecycle and non-resurrection during ordinary sync."""
        # 1. Add
        res = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertIn("demo-srv", claude_doc.get("mcpServers", {}))

        # 2. Disable
        res = self._run_cli(["--action", "disable", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", claude_doc.get("mcpServers", {}))
        bindings = load_mcp_bindings(self.db_path)
        self.assertFalse(bindings["demo-srv"]["claude-code"]["enabled"])
        self.assertTrue(bindings["demo-srv"]["claude-code"]["assigned"])

        # Running ordinary sync does NOT resurrect the disabled server
        res_sync = self._run_cli([])
        self.assertEqual(res_sync.returncode, 0, res_sync.stderr)
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", claude_doc.get("mcpServers", {}))

        # 3. Enable
        res = self._run_cli(["--action", "enable", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertIn("demo-srv", claude_doc.get("mcpServers", {}))
        bindings = load_mcp_bindings(self.db_path)
        self.assertTrue(bindings["demo-srv"]["claude-code"]["enabled"])

        # 4. Remove
        res = self._run_cli(["--action", "remove", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", claude_doc.get("mcpServers", {}))
        bindings = load_mcp_bindings(self.db_path)
        self.assertFalse(bindings["demo-srv"]["claude-code"]["assigned"])

        # Running ordinary sync does NOT resurrect the removed server
        res_sync2 = self._run_cli([])
        self.assertEqual(res_sync2.returncode, 0, res_sync2.stderr)
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", claude_doc.get("mcpServers", {}))

    def test_global_policy_toggle_preserves_per_agent_state_in_db(self):
        """Global server toggle (off/on) preserves per-agent enabled state in DB."""
        # Add to claude-code
        self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])

        # Global toggle off in manager.json
        policy = json.loads(self.manager_json.read_text(encoding="utf-8"))
        policy["mcp"]["demo-srv"]["enabled"] = False
        self.manager_json.write_text(json.dumps(policy, indent=2), encoding="utf-8")

        # Run ordinary sync
        self._run_cli([])
        # Removed from file
        claude_doc = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", claude_doc.get("mcpServers", {}))
        # But DB per-agent enabled is still True!
        bindings = load_mcp_bindings(self.db_path)
        self.assertTrue(bindings["demo-srv"]["claude-code"]["enabled"])
        self.assertTrue(bindings["demo-srv"]["claude-code"]["assigned"])

        # Global toggle on again
        policy["mcp"]["demo-srv"]["enabled"] = True
        self.manager_json.write_text(json.dumps(policy, indent=2), encoding="utf-8")
        self._run_cli([])

        # Restored to file
        claude_doc2 = json.loads(self.claude_config.read_text(encoding="utf-8"))
        self.assertIn("demo-srv", claude_doc2.get("mcpServers", {}))

    def test_dry_run_performs_zero_writes(self):
        """--dry-run performs validations and previews with zero disk and zero DB writes."""
        db_before = self.db_path.read_bytes()
        claude_before = self.claude_config.read_bytes()

        res = self._run_cli(["--dry-run", "--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("(预览: 添加 MCP demo-srv)", res.stderr + res.stdout)

        # Zero writes
        self.assertEqual(self.claude_config.read_bytes(), claude_before)
        self.assertEqual(self.db_path.read_bytes(), db_before)

    def test_add_action_rejects_already_assigned_server(self):
        """add 若已有 assigned 必须拒绝，避免把 add 变 update。"""
        res1 = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res1.returncode, 0, res1.stderr)

        res2 = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res2.returncode, 2)
        self.assertIn("无法重复 add", res2.stderr)

    def test_stdio_only_command_required(self):
        """Definitions missing command (non-stdio/HTTP) are rejected."""
        defs = copy.deepcopy(self.base_definitions)
        defs["servers"]["no-cmd-srv"] = {"args": ["abc"]}
        self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")

        res = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "no-cmd-srv"])
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("command", res.stderr)

    def test_opencode_json_format_support(self):
        """OpenCode uses 'mcp' top-level key, command array, and 'environment'."""
        res = self._run_cli(["--action", "add", "--agent", "opencode", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)

        opencode_doc = json.loads(self.opencode_config.read_text(encoding="utf-8"))
        self.assertIn("mcp", opencode_doc)
        srv = opencode_doc["mcp"]["demo-srv"]
        self.assertEqual(srv["type"], "local")
        self.assertEqual(srv["command"], ["node", "demo.js", "--port", "3000"])
        self.assertEqual(srv["environment"]["_MANAGED_BY"], "agent-manager")

        # Disable
        res_dis = self._run_cli(["--action", "disable", "--agent", "opencode", "--only", "demo-srv"])
        self.assertEqual(res_dis.returncode, 0, res_dis.stderr)
        opencode_doc2 = json.loads(self.opencode_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", opencode_doc2.get("mcp", {}))

    def test_toml_codex_format_support(self):
        """Codex TOML support with [mcp_servers.<name>] and startup_timeout_sec."""
        res = self._run_cli(["--action", "add", "--agent", "codex", "--only", "demo-srv"])
        self.assertEqual(res.returncode, 0, res.stderr)

        content = self.codex_config.read_text(encoding="utf-8")
        self.assertTrue('[mcp_servers.demo-srv]' in content or '[mcp_servers."demo-srv"]' in content)
        self.assertIn('command = "node"', content)
        self.assertIn('startup_timeout_sec = 45', content)
        self.assertIn('_MANAGED_BY = "agent-manager"', content)

        # Disable
        res_dis = self._run_cli(["--action", "disable", "--agent", "codex", "--only", "demo-srv"])
        self.assertEqual(res_dis.returncode, 0, res_dis.stderr)
        content2 = self.codex_config.read_text(encoding="utf-8")
        self.assertNotIn('demo-srv', content2)

    def test_agent_filter_without_action(self):
        """CLI --agent without --action strictly filters to that single agent without touching other agents."""
        # Initial state: claude_config, codex_config, opencode_config exist
        claude_bytes_before = self.claude_config.read_bytes()
        codex_bytes_before = self.codex_config.read_bytes()
        opencode_bytes_before = self.opencode_config.read_bytes()

        # 1. Unknown agent must fail with exit code 2
        res_unknown = self._run_cli(["--agent", "unknown-agent-xyz"])
        self.assertEqual(res_unknown.returncode, 2)
        self.assertIn("未知 Agent", res_unknown.stderr)

        # 2. Run --agent claude-code without --action
        res_claude = self._run_cli(["--agent", "claude-code"])
        self.assertEqual(res_claude.returncode, 0, res_claude.stderr)

        # codex and opencode must be 100% byte-identical
        self.assertEqual(self.codex_config.read_bytes(), codex_bytes_before)
        self.assertEqual(self.opencode_config.read_bytes(), opencode_bytes_before)

    def test_source_deleted_bindings_preserved_during_sync(self):
        """Bindings whose definitions were removed from mcp.json are preserved in DB and agent config during ordinary sync."""
        # 1. Add demo-srv to claude-code
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_add.returncode, 0, res_add.stderr)
        self.assertIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))

        # Verify in DB
        loaded = load_mcp_bindings(self.db_path)
        self.assertIn("demo-srv", loaded)
        self.assertTrue(loaded["demo-srv"]["claude-code"]["assigned"])

        # 2. Delete demo-srv from mcp.json
        defs = copy.deepcopy(self.base_definitions)
        del defs["servers"]["demo-srv"]
        self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")

        # 3. Run ordinary sync (no --action)
        res_sync = self._run_cli([])
        self.assertEqual(res_sync.returncode, 0, res_sync.stderr)

        # demo-srv must STILL exist in Claude config
        self.assertIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))

        # demo-srv must STILL exist in DB bindings
        loaded_after = load_mcp_bindings(self.db_path)
        self.assertIn("demo-srv", loaded_after)
        self.assertTrue(loaded_after["demo-srv"]["claude-code"]["assigned"])

        # 4. Explicit remove can still remove it
        res_remove = self._run_cli(["--action", "remove", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_remove.returncode, 0, res_remove.stderr)
        self.assertNotIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))
        loaded_removed = load_mcp_bindings(self.db_path)
        self.assertFalse(loaded_removed["demo-srv"]["claude-code"]["assigned"])

    def test_policy_override_preserves_user_adjustments_and_library_defaults(self):
        """Old policy overrides (args, env placeholders) are preserved in pinned definition, and library servers need no policy entry."""
        # 1. Add custom override in manager.json for demo-srv
        policy = copy.deepcopy(self.base_policy)
        policy["mcp"]["demo-srv"] = {
            "args": ["demo.js", "--port", "8080", "--verbose"],
            "env": {"API_TOKEN": "${CUSTOM_TOKEN}", "EXTRA_FLAG": "true"},
        }
        self.manager_json.write_text(json.dumps(policy, indent=2), encoding="utf-8")
        (self.secrets_dir / "mcp.env").write_text("CUSTOM_TOKEN=custom_override_secret_7777\n", encoding="utf-8")

        # 2. Add demo-srv to claude-code
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_add.returncode, 0, res_add.stderr)

        # 3. Pinned definition in SQLite must reflect the policy override, NOT baseline
        loaded = load_mcp_bindings(self.db_path)
        pinned = loaded["demo-srv"]["claude-code"]["definition"]
        self.assertEqual(pinned["args"], ["demo.js", "--port", "8080", "--verbose"])
        self.assertEqual(pinned["env"]["API_TOKEN"], "${CUSTOM_TOKEN}")
        self.assertEqual(pinned["env"]["EXTRA_FLAG"], "true")
        self.assertNotIn("enabled", pinned)
        self.assertNotIn("agents", pinned)

        # Injected Claude config must have the resolved secret and user args
        claude_content = self.claude_config.read_text(encoding="utf-8")
        self.assertIn("custom_override_secret_7777", claude_content)
        self.assertIn("--port", claude_content)
        self.assertIn("8080", claude_content)

        # 4. Library resource without any policy declaration can be added directly
        defs = copy.deepcopy(self.base_definitions)
        defs["servers"]["lib-only-srv"] = {
            "command": "echo",
            "args": ["hello-from-lib"],
            "env": {"LIB_KEY": "constant_value"},
        }
        self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")

        # Adding without any entry in manager.json policy succeeds (global enabled defaults to True)
        res_lib = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "lib-only-srv"])
        self.assertEqual(res_lib.returncode, 0, res_lib.stderr)
        self.assertIn("lib-only-srv", self.claude_config.read_text(encoding="utf-8"))

        # Explicit policy enabled=false blocks add
        policy["mcp"]["disabled-lib-srv"] = {"enabled": False}
        defs["servers"]["disabled-lib-srv"] = {"command": "echo", "args": ["blocked"]}
        self.manager_json.write_text(json.dumps(policy, indent=2), encoding="utf-8")
        self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")
        res_blocked = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "disabled-lib-srv"])
        self.assertEqual(res_blocked.returncode, 2)
        self.assertIn("全局已禁用", res_blocked.stderr)

    def test_readd_after_remove_succeeds_and_preserves_other_agent(self):
        """add -> remove -> add 周期测试：
        重新 add 使用进入动作前的 old_binding 墓碑快照 CAS，成功将墓碑复活，
        同时另一个 Agent 保持 100% 字节不变且其 DB 绑定不受任何影响。
        覆盖三种格式 (claude-code JSON, codex TOML, opencode)。
        """
        # 1. 给 claude-code, codex, opencode 均添加 demo-srv
        res_a = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_a.returncode, 0, res_a.stderr)
        res_b = self._run_cli(["--action", "add", "--agent", "codex", "--only", "demo-srv"])
        self.assertEqual(res_b.returncode, 0, res_b.stderr)
        res_c = self._run_cli(["--action", "add", "--agent", "opencode", "--only", "demo-srv"])
        self.assertEqual(res_c.returncode, 0, res_c.stderr)

        codex_bytes_snapshot = self.codex_config.read_bytes()
        opencode_bytes_snapshot = self.opencode_config.read_bytes()
        bindings = load_mcp_bindings(self.db_path)
        codex_binding_snapshot = copy.deepcopy(bindings["demo-srv"]["codex"])
        opencode_binding_snapshot = copy.deepcopy(bindings["demo-srv"]["opencode"])

        # 2. remove claude-code 的 demo-srv
        res_rm = self._run_cli(["--action", "remove", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_rm.returncode, 0, res_rm.stderr)
        self.assertNotIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))

        # DB 中 claude-code 变为墓碑
        bindings_after_rm = load_mcp_bindings(self.db_path)
        self.assertFalse(bindings_after_rm["demo-srv"]["claude-code"]["assigned"])
        self.assertFalse(bindings_after_rm["demo-srv"]["claude-code"]["enabled"])

        # 另一个 Agent (codex 和 opencode) 字节及绑定 100% 保持不变
        self.assertEqual(self.codex_config.read_bytes(), codex_bytes_snapshot)
        self.assertEqual(self.opencode_config.read_bytes(), opencode_bytes_snapshot)
        self.assertEqual(bindings_after_rm["demo-srv"]["codex"], codex_binding_snapshot)
        self.assertEqual(bindings_after_rm["demo-srv"]["opencode"], opencode_binding_snapshot)

        # 3. 重新 add claude-code 的 demo-srv（通过 CAS 匹配进入前的墓碑快照）
        res_readd = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_readd.returncode, 0, res_readd.stderr)
        self.assertIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))

        # DB 中 claude-code 成功复活
        bindings_after_readd = load_mcp_bindings(self.db_path)
        self.assertTrue(bindings_after_readd["demo-srv"]["claude-code"]["assigned"])
        self.assertTrue(bindings_after_readd["demo-srv"]["claude-code"]["enabled"])

        # 另一个 Agent 仍然 100% 保持不变
        self.assertEqual(self.codex_config.read_bytes(), codex_bytes_snapshot)
        self.assertEqual(self.opencode_config.read_bytes(), opencode_bytes_snapshot)
        self.assertEqual(bindings_after_readd["demo-srv"]["codex"], codex_binding_snapshot)
        self.assertEqual(bindings_after_readd["demo-srv"]["opencode"], opencode_binding_snapshot)

        # 4. 对 codex 和 opencode 也执行 remove -> re-add 闭环
        for agent, cfg in (("codex", self.codex_config), ("opencode", self.opencode_config)):
            res_rm_i = self._run_cli(["--action", "remove", "--agent", agent, "--only", "demo-srv"])
            self.assertEqual(res_rm_i.returncode, 0, res_rm_i.stderr)
            self.assertNotIn("demo-srv", cfg.read_text(encoding="utf-8"))

            res_readd_i = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
            self.assertEqual(res_readd_i.returncode, 0, res_readd_i.stderr)
            self.assertIn("demo-srv", cfg.read_text(encoding="utf-8"))
            b_i = load_mcp_bindings(self.db_path)
            self.assertTrue(b_i["demo-srv"][agent]["assigned"])
            self.assertTrue(b_i["demo-srv"][agent]["enabled"])

    def test_bootstrap_on_first_add_requires_matching_valid_definition_for_all_formats(self):
        """首次 add 时若配置文件中已存在含 marker 的条目但无历史 binding：
        条目内容必须与当前有效 definition 匹配才允许 bootstrap 接管；
        若内容不一致，仅凭 marker 必须拒绝（返回码非 0），原文件与 DB 均不变。
        覆盖 JSON (claude-code)、TOML (codex)、OpenCode 三种格式。
        """
        targets = [
            ("claude-code", self.claude_config, "json"),
            ("codex", self.codex_config, "toml"),
            ("opencode", self.opencode_config, "opencode"),
        ]

        for agent, cfg_path, fmt in targets:
            with self.subTest(agent=agent, format=fmt):
                # 构造已存在带 marker 但 command 不匹配的条目
                if fmt == "json":
                    cfg_path.write_text(json.dumps({
                        "mcpServers": {
                            "demo-srv": {
                                "command": "different-node-cmd",
                                "args": ["demo.js", "--port", "3000"],
                                "env": {"API_TOKEN": "very_secret_token_val_9999", "_MANAGED_BY": "agent-manager"},
                            }
                        }
                    }, indent=2), encoding="utf-8")
                elif fmt == "opencode":
                    cfg_path.write_text(json.dumps({
                        "mcp": {
                            "demo-srv": {
                                "type": "local",
                                "command": ["different-node-cmd", "demo.js", "--port", "3000"],
                                "environment": {"API_TOKEN": "very_secret_token_val_9999", "_MANAGED_BY": "agent-manager"},
                            }
                        }
                    }, indent=2), encoding="utf-8")
                elif fmt == "toml":
                    cfg_path.write_text(
                        '[mcp_servers.demo-srv]\n'
                        'command = "different-node-cmd"\n'
                        'args = ["demo.js", "--port", "3000"]\n'
                        'startup_timeout_sec = 45\n'
                        '[mcp_servers.demo-srv.env]\n'
                        'API_TOKEN = "very_secret_token_val_9999"\n'
                        '_MANAGED_BY = "agent-manager"\n',
                        encoding="utf-8",
                    )

                original_bytes = cfg_path.read_bytes()
                db_bytes_before = self.db_path.read_bytes()

                # dry-run 必须拒绝
                res_dry = self._run_cli(["--dry-run", "--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertNotEqual(res_dry.returncode, 0)
                self.assertIn("与当前定义不一致且无历史绑定，拒绝仅凭标记接管", res_dry.stderr)
                self.assertEqual(cfg_path.read_bytes(), original_bytes)
                self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

                # 实际 add 必须拒绝
                res_add = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertNotEqual(res_add.returncode, 0)
                self.assertIn("与当前定义不一致且无历史绑定，拒绝仅凭标记接管", res_add.stderr)
                self.assertEqual(cfg_path.read_bytes(), original_bytes)
                self.assertEqual(self.db_path.read_bytes(), db_bytes_before)

                # 将条目修改为与 definition 100% 匹配
                if fmt == "json":
                    cfg_path.write_text(json.dumps({
                        "mcpServers": {
                            "demo-srv": {
                                "command": "node",
                                "args": ["demo.js", "--port", "3000"],
                                "env": {"API_TOKEN": "very_secret_token_val_9999", "_MANAGED_BY": "agent-manager"},
                            }
                        }
                    }, indent=2), encoding="utf-8")
                elif fmt == "opencode":
                    cfg_path.write_text(json.dumps({
                        "mcp": {
                            "demo-srv": {
                                "type": "local",
                                "command": ["node", "demo.js", "--port", "3000"],
                                "environment": {"API_TOKEN": "very_secret_token_val_9999", "_MANAGED_BY": "agent-manager"},
                            }
                        }
                    }, indent=2), encoding="utf-8")
                elif fmt == "toml":
                    cfg_path.write_text(
                        '[mcp_servers.demo-srv]\n'
                        'command = "node"\n'
                        'args = ["demo.js", "--port", "3000"]\n'
                        'startup_timeout_sec = 45\n'
                        '[mcp_servers.demo-srv.env]\n'
                        'API_TOKEN = "very_secret_token_val_9999"\n'
                        '_MANAGED_BY = "agent-manager"\n',
                        encoding="utf-8",
                    )

                # 此时内容完全匹配，add 应该成功 bootstrap
                res_bootstrap = self._run_cli(["--action", "add", "--agent", agent, "--only", "demo-srv"])
                self.assertEqual(res_bootstrap.returncode, 0, res_bootstrap.stderr)
                bindings = load_mcp_bindings(self.db_path)
                self.assertIn("demo-srv", bindings)
                self.assertTrue(bindings["demo-srv"][agent]["assigned"])
                self.assertTrue(bindings["demo-srv"][agent]["enabled"])

    def test_single_only_enforcement_and_unresolved_placeholder_rejection(self):
        """1. args.only 重复同一值（即使 set 后 len=1）必须被拒绝 (exit 2)；
        2. add/update/enable 存在未解析 ${REF} 凭证引用时必须拒绝 (exit 1)，零写配置文件和数据库；
        3. 凭证撤去后，disable 和 remove 仍允许正常执行。
        """
        # 1. 验证重复 --only 必须严格拒绝
        res_dup_only = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv", "--only", "demo-srv"])
        self.assertEqual(res_dup_only.returncode, 2)
        self.assertIn("仅指定单个 server", res_dup_only.stderr)

        # 2. 配置一个含有未解析占位符的 server
        defs = copy.deepcopy(self.base_definitions)
        defs["servers"]["unresolved-srv"] = {
            "command": "node",
            "args": ["server.js"],
            "env": {"API_KEY": "${MISSING_ENV_SECRET}"},
        }
        self.mcp_json.write_text(json.dumps(defs, indent=2), encoding="utf-8")

        claude_before = self.claude_config.read_bytes()
        db_before = self.db_path.read_bytes()

        # dry-run add 拒绝未解析凭证
        res_dry = self._run_cli(["--dry-run", "--action", "add", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertNotEqual(res_dry.returncode, 0)
        self.assertIn("未解析的凭证变量", res_dry.stderr)
        self.assertIn("MISSING_ENV_SECRET", res_dry.stderr)
        self.assertEqual(self.claude_config.read_bytes(), claude_before)
        self.assertEqual(self.db_path.read_bytes(), db_before)

        # 实际 add 拒绝未解析凭证，原文件与 DB 均不变
        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertNotEqual(res_add.returncode, 0)
        self.assertIn("未解析的凭证变量", res_add.stderr)
        self.assertIn("MISSING_ENV_SECRET", res_add.stderr)
        self.assertEqual(self.claude_config.read_bytes(), claude_before)
        self.assertEqual(self.db_path.read_bytes(), db_before)

        # 3. 提供凭证后 add 成功
        (self.secrets_dir / "mcp.env").write_text("DEMO_TOKEN=very_secret_token_val_9999\nMISSING_ENV_SECRET=now_resolved_secret_val_12345\n", encoding="utf-8")
        res_add_ok = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertEqual(res_add_ok.returncode, 0, res_add_ok.stderr)

        # 4. 撤去凭证（模拟凭证被删除/过期）
        (self.secrets_dir / "mcp.env").write_text("DEMO_TOKEN=very_secret_token_val_9999\n", encoding="utf-8")

        # 凭证缺失时 update 必须拒绝
        res_update = self._run_cli(["--action", "update", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertNotEqual(res_update.returncode, 0)
        self.assertIn("未解析的凭证变量", res_update.stderr)

        # 5. 但凭证撤去后，disable 和 remove 仍必须允许正常撤销该服务！
        res_disable = self._run_cli(["--action", "disable", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertEqual(res_disable.returncode, 0, res_disable.stderr)
        self.assertNotIn("unresolved-srv", self.claude_config.read_text(encoding="utf-8"))

        # 凭证缺失时 enable 必须拒绝
        res_enable = self._run_cli(["--action", "enable", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertNotEqual(res_enable.returncode, 0)
        self.assertIn("未解析的凭证变量", res_enable.stderr)

        # 凭证缺失时 remove 仍允许正常移除
        res_remove = self._run_cli(["--action", "remove", "--agent", "claude-code", "--only", "unresolved-srv"])
        self.assertEqual(res_remove.returncode, 0, res_remove.stderr)
        bindings = load_mcp_bindings(self.db_path)
        self.assertFalse(bindings["unresolved-srv"]["claude-code"]["assigned"])

    def test_no_enabled_policy_explicit_add_preserved_on_normal_sync_and_other_agent_not_autobound(self):
        """无 enabled policy 场景：
        manager.json 中 mcp: {}（无 enabled 策略），显式 add demo-srv 给 claude-code。
        随后执行普通 sync (--offline)，claude-code 的配置完整保留，其他 Agent (codex) 不自动添加，
        数据库绑定状态保持一致。
        """
        self.manager_json.write_text(json.dumps({"mcp": {}}, indent=2), encoding="utf-8")
        self.claude_config.write_text("{}", encoding="utf-8")
        self.codex_config.write_text("", encoding="utf-8")
        self.db_path.unlink(missing_ok=True)
        _init_test_db(self.db_path)

        res_add = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_add.returncode, 0, f"add 失败: {res_add.stderr}")
        claude_after_add = self.claude_config.read_bytes()
        self.assertIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))
        self.assertNotIn("demo-srv", self.codex_config.read_text(encoding="utf-8"))

        bindings_after_add = load_mcp_bindings(self.db_path)
        self.assertTrue(bindings_after_add["demo-srv"]["claude-code"]["assigned"])
        self.assertTrue(bindings_after_add["demo-srv"]["claude-code"]["enabled"])

        res_sync = self._run_cli(["--offline"])
        self.assertEqual(res_sync.returncode, 0, f"sync 失败: {res_sync.stderr}")

        self.assertEqual(self.claude_config.read_bytes(), claude_after_add)
        self.assertNotIn("demo-srv", self.codex_config.read_text(encoding="utf-8"))
        bindings_after_sync = load_mcp_bindings(self.db_path)
        self.assertEqual(bindings_after_sync["demo-srv"]["claude-code"], bindings_after_add["demo-srv"]["claude-code"])

    def test_allowed_agents_scope_enforcement_add_update_enable_rejected_disable_remove_allowed(self):
        """定义 agents=[codex] 时：
        1. add --agent claude-code 在 dryrun 和 real 下均 exit 2 且零写入。
        2. 若历史已有越界绑定：update / enable exit 2 拒绝；disable / remove 允许清理。
        3. 普通 sync 面对收紧后的范围：撤下生效入口，但保留 DB 中用户的既有绑定。
        """
        scoped_defs = copy.deepcopy(self.base_definitions)
        scoped_defs["servers"]["demo-srv"]["agents"] = ["codex"]
        self.mcp_json.write_text(json.dumps(scoped_defs, indent=2), encoding="utf-8")
        self.claude_config.write_text("{}", encoding="utf-8")
        self.codex_config.write_text("", encoding="utf-8")
        self.db_path.unlink(missing_ok=True)
        _init_test_db(self.db_path)

        claude_before = self.claude_config.read_bytes()
        db_before = self.db_path.read_bytes()

        # dryrun add 必须 exit 2 且零写
        res_dry = self._run_cli(["--dry-run", "--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_dry.returncode, 2)
        self.assertIn("不在 MCP 服务 'demo-srv' 的允许分配范围", res_dry.stderr)
        self.assertEqual(self.claude_config.read_bytes(), claude_before)
        self.assertEqual(self.db_path.read_bytes(), db_before)

        # 真实 add 必须 exit 2 且零写
        res_real = self._run_cli(["--action", "add", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_real.returncode, 2)
        self.assertIn("不在 MCP 服务 'demo-srv' 的允许分配范围", res_real.stderr)
        self.assertEqual(self.claude_config.read_bytes(), claude_before)
        self.assertEqual(self.db_path.read_bytes(), db_before)

        # 允许范围内的 codex 可以正常 add
        res_codex = self._run_cli(["--action", "add", "--agent", "codex", "--only", "demo-srv"])
        self.assertEqual(res_codex.returncode, 0, res_codex.stderr)
        self.assertIn("demo-srv", self.codex_config.read_text(encoding="utf-8"))

        # 2. 模拟历史越界绑定（如之前分配后上游收紧了 agents 范围）
        bindings = load_mcp_bindings(self.db_path)
        dummy_def = clean_definition(scoped_defs["servers"]["demo-srv"])
        dummy_ver = compute_definition_version(dummy_def)
        set_binding(bindings, "demo-srv", "claude-code", version=dummy_ver, definition=dummy_def, enabled=True, assigned=True, applied_digest="d" * 64)
        save_mcp_bindings(self.db_path, bindings, action="test_setup")

        # update 必须 exit 2
        res_up = self._run_cli(["--action", "update", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_up.returncode, 2)
        self.assertIn("不在 MCP 服务 'demo-srv' 的允许分配范围", res_up.stderr)

        # enable 必须 exit 2
        res_en = self._run_cli(["--action", "enable", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_en.returncode, 2)
        self.assertIn("不在 MCP 服务 'demo-srv' 的允许分配范围", res_en.stderr)

        # 普通 sync 对收紧范围的既有绑定：撤下生效入口（不在 claude 配置中注入），但 DB 绑定保留
        res_sync = self._run_cli(["--offline"])
        self.assertEqual(res_sync.returncode, 0, res_sync.stderr)
        self.assertNotIn("demo-srv", self.claude_config.read_text(encoding="utf-8"))
        bindings_after_sync = load_mcp_bindings(self.db_path)
        self.assertTrue(bindings_after_sync["demo-srv"]["claude-code"]["assigned"])

        # disable 允许清理历史越界绑定
        res_dis = self._run_cli(["--action", "disable", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_dis.returncode, 0, res_dis.stderr)
        bindings_dis = load_mcp_bindings(self.db_path)
        self.assertFalse(bindings_dis["demo-srv"]["claude-code"]["enabled"])

        # remove 允许清理历史越界绑定
        res_rm = self._run_cli(["--action", "remove", "--agent", "claude-code", "--only", "demo-srv"])
        self.assertEqual(res_rm.returncode, 0, res_rm.stderr)
        bindings_rm = load_mcp_bindings(self.db_path)
        self.assertFalse(bindings_rm["demo-srv"]["claude-code"]["assigned"])


if __name__ == "__main__":
    unittest.main()
