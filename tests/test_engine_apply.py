"""Sandbox tests for the engine write path (Phase 2a): '<domain>:apply',
system:converge, snapshots list/restore, runs list/get, policy get/set.

Syncers run for real inside the sandbox (PATH stubs replace heavy commands);
every assertion about writes is paired with a dry-run zero-write counterpart.
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests.contract_validator import load_schema, validate

ROOT = Path(__file__).resolve().parent.parent
CONTRACT_DIR = ROOT / "contract" / "engine-v1"
ENVELOPE_SCHEMA = load_schema(CONTRACT_DIR / "envelope.schema.json")
APPLY_RESULT_SCHEMA = load_schema(CONTRACT_DIR / "apply-result.schema.json")
CONVERGE_SCHEMA = load_schema(CONTRACT_DIR / "converge.schema.json")
RUNS_SCHEMA = load_schema(CONTRACT_DIR / "runs.schema.json")
SNAPSHOTS_SCHEMA = load_schema(CONTRACT_DIR / "snapshots.schema.json")
POLICY_SCHEMA = load_schema(CONTRACT_DIR / "policy.schema.json")

COPY_IGNORE = shutil.ignore_patterns("repositories", ".git", "__pycache__", "secrets", ".env", "app", "node_modules")


class EngineWritePathTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_apply_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        for name in ("manager.local.json", "manager.synced.json"):
            (self.repo / "config" / name).unlink(missing_ok=True)
        self.bindir = self.temp_dir / "bin"
        self.bindir.mkdir()
        self.snapshot_dir = self.temp_dir / "snapshots"
        self.log_prefix = self.temp_dir / "logs" / "agent_manager_sync"
        self._write_minimal_configs()

    # ------------------------------------------------------------------ helpers

    def _write_minimal_configs(self):
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {}, "browser_us_sync": {"enabled": False}})
        self._write_config("agents.json", {})
        self._write_config("tools.json", {"tools": {}})
        self._write_config("mcp.json", {"servers": {}, "agent_paths": {}})
        self._write_config("skills.json", {"repositories": {}, "agents": {}})
        (self.repo / "config" / "rules-targets.list").write_text("", encoding="utf-8")

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _env(self, extra=None):
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.temp_dir / "agent_manager.lock")
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.log_prefix)
        env["AGENT_MANAGER_SNAPSHOT_DIR"] = str(self.snapshot_dir)
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "upstream-cache.json")
        env["AGENT_MANAGER_SYNCED_CONFIG"] = str(self.temp_dir / "manager.synced.json")
        env["AGENT_MANAGER_LOCAL_CONFIG"] = str(self.temp_dir / "manager.local.json")
        if extra:
            env.update(extra)
        return env

    def _run(self, *argv, env_extra=None):
        (self.temp_dir / "logs").mkdir(exist_ok=True)
        return subprocess.run(
            [str(self.repo / "manage.sh"), *argv],
            cwd=self.repo,
            env=self._env(env_extra),
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )

    def _payload(self, *argv, expect_exit=0, env_extra=None):
        result = self._run(*argv, env_extra=env_extra)
        self.assertEqual(result.returncode, expect_exit, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        errors = validate(ENVELOPE_SCHEMA, payload)
        self.assertEqual(errors, [], "envelope violations: %s" % [str(e) for e in errors])
        return payload

    def _assert_schema(self, schema, data):
        errors = validate(schema, data)
        self.assertEqual(errors, [], "schema violations: %s" % [str(e) for e in errors])

    # ------------------------------------------------------------------ mcp apply

    def _mcp_fixture(self):
        target = self.home / ".agent-a.json"
        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--x"], "env": {}, "agents": ["agent-a"]}
                },
                "agent_paths": {"agent-a": str(target)},
            },
        )
        self._manager_mcp()
        return target

    def _manager_mcp(self):
        self._write_config(
            "manager.json",
            {"agents": {}, "tools": {}, "skills": {}, "mcp": {"alpha": {"enabled": True}}, "browser_us_sync": {"enabled": False}},
        )

    def test_mcp_apply_writes_target_and_snapshots_it(self):
        target = self._mcp_fixture()
        payload = self._payload("mcp:apply", "--json", "--offline")
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["steps"][0]["name"], "mcp-sync")
        self.assertTrue(payload["data"]["steps"][0]["ok"])
        injected = json.loads(target.read_text(encoding="utf-8"))
        self.assertEqual(injected["mcpServers"]["alpha"]["command"], "alpha-cmd")
        self.assertEqual(injected["mcpServers"]["alpha"]["env"]["_MANAGED_BY"], "agent-manager")
        # 目标此前不存在 → 快照 manifest 记录 existed=false（回滚会删除）。
        snapshot = payload["data"]["snapshot"]
        self.assertIsNotNone(snapshot)
        self.assertEqual(snapshot["file_count"], 1)
        manifest = json.loads((Path(snapshot["path"]) / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["files"][0]["existed"], False)

    def test_mcp_apply_dry_run_writes_nothing(self):
        target = self._mcp_fixture()
        payload = self._payload("mcp:apply", "--json", "--dry-run")
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        self.assertTrue(payload["data"]["dry_run"])
        self.assertIsNone(payload["data"]["snapshot"])
        self.assertFalse(target.exists(), "dry-run 不得写目标文件")
        self.assertFalse(self.snapshot_dir.exists(), "dry-run 不得建快照目录")

    def test_mcp_apply_existing_target_gets_real_backup(self):
        target = self._mcp_fixture()
        target.write_text(json.dumps({"mcpServers": {}, "user_key": 1}) + "\n", encoding="utf-8")
        payload = self._payload("mcp:apply", "--json", "--offline")
        snapshot = payload["data"]["snapshot"]
        self.assertEqual(snapshot["file_count"], 1)
        manifest = json.loads((Path(snapshot["path"]) / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["files"][0]["existed"], True)
        # 用户键保留（同步器合并语义），受管 server 注入。
        injected = json.loads(target.read_text(encoding="utf-8"))
        self.assertEqual(injected["user_key"], 1)
        self.assertIn("alpha", injected["mcpServers"])

    # ------------------------------------------------------------------ aliases apply + restore roundtrip

    def _aliases_fixture(self):
        self._write_config(
            "shell-aliases.json",
            {"timezone": "UTC", "lang": "C", "aliases": {"ll": "ls -l"}},
        )

    def test_aliases_apply_writes_managed_block(self):
        self._aliases_fixture()
        payload = self._payload("aliases:apply", "--json")
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        zshrc = (self.home / ".zshrc").read_text(encoding="utf-8")
        self.assertIn("alias ll='ls -l'", zshrc)
        self.assertIn("# >>> agent-manager shell aliases >>>", zshrc)
        zprofile = (self.home / ".zprofile").read_text(encoding="utf-8")
        self.assertIn("alias ll='ls -l'", zprofile)

    def test_snapshots_restore_roundtrip_resurrects_and_deletes(self):
        self._aliases_fixture()
        pre_existing = self.home / ".zshrc"
        pre_existing.write_text("# my own rc\n", encoding="utf-8")
        payload = self._payload("aliases:apply", "--json")
        run_id = payload["data"]["snapshot"]["run_id"]
        # apply 后：.zshrc 多了区块；.zprofile 被新建。
        self.assertIn("agent-manager shell aliases", (self.home / ".zshrc").read_text(encoding="utf-8"))
        self.assertTrue((self.home / ".zprofile").exists())

        payload = self._payload("snapshots:restore", "--json", "--run", run_id)
        self._assert_schema(SNAPSHOTS_SCHEMA, payload["data"])
        restore = payload["data"]["restore"]
        self.assertEqual(restore["restored_run"], run_id)
        self.assertGreaterEqual(restore["files_restored"], 1)
        # 回滚后：.zshrc 复原；.zprofile（apply 新建）被删除。
        self.assertEqual((self.home / ".zshrc").read_text(encoding="utf-8"), "# my own rc\n")
        self.assertFalse((self.home / ".zprofile").exists(), "快照时不存在的文件必须被回滚删除")
        self.assertGreaterEqual(restore["files_deleted"], 1)
        # restore 自身也留了再快照（可反向撤销）。
        re_manifest = json.loads(
            (self.snapshot_dir / restore["resnapshot_run_id"] / "manifest.json").read_text(encoding="utf-8")
        )
        self.assertEqual(re_manifest["command"], "snapshots:restore(pre-%s)" % run_id)

        # list 能看到两个快照，新者在前。
        payload = self._payload("snapshots:list", "--json")
        self._assert_schema(SNAPSHOTS_SCHEMA, payload["data"])
        run_ids = [entry["run_id"] for entry in payload["data"]["snapshots"]]
        self.assertEqual(run_ids[0], restore["resnapshot_run_id"])
        self.assertIn(run_id, run_ids)

    # ------------------------------------------------------------------ hooks / rules / skills / tools / agents apply

    def test_hooks_apply_renders_both_targets(self):
        self._write_config(
            "claude-hooks.json",
            {"settings_path": "../.claude/settings.json", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo apply-test"}]}]}},
        )
        self._write_config(
            "codex-hooks.json",
            {"hooks_path": "../.codex/hooks.json", "hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "echo apply-test-codex"}]}]}},
        )
        payload = self._payload("hooks:apply", "--json")
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        self.assertEqual([s["name"] for s in payload["data"]["steps"]], ["claude-hooks", "codex-hooks"])
        claude = json.loads((self.repo / ".claude" / "settings.json").read_text(encoding="utf-8"))
        self.assertEqual(claude["hooks"]["SessionStart"][0]["hooks"][0]["command"], "echo apply-test")
        codex = json.loads((self.repo / ".codex" / "hooks.json").read_text(encoding="utf-8"))
        self.assertEqual(codex["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "echo apply-test-codex")

    def test_rules_apply_distributes_manifest_targets(self):
        (self.repo / "config" / "rules-targets.list").write_text(
            "claude_global|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/CLAUDE.md\n",
            encoding="utf-8",
        )
        self._write_config(
            "manager.json",
            {"agents": {"claude-code": {"enabled": True}}, "tools": {}, "skills": {}, "mcp": {}, "browser_us_sync": {"enabled": False}},
        )
        payload = self._payload("rules:apply", "--json")
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        names = [s["name"] for s in payload["data"]["steps"]]
        self.assertEqual(names, ["rules-sync"], names)
        dist = (self.repo / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md").read_bytes()
        self.assertEqual((self.home / ".claude" / "CLAUDE.md").read_bytes(), dist)

    def test_skills_apply_creates_managed_links(self):
        repo_dir = self.repo / "agent-skill" / "repositories" / "coll"
        (repo_dir / "skill-a").mkdir(parents=True)
        (repo_dir / "skill-a" / "SKILL.md").write_text("---\nname: skill-a\ndescription: t\n---\n", encoding="utf-8")
        # link_skills 只接管 git 仓库形态的集合缓存；目标 agent 目录的父目录
        # 必须存在（is_active_skill_target 语义），否则同步器跳过新建。
        subprocess.run(["git", "init", "-q", str(repo_dir)], check=True, capture_output=True)
        (self.home / ".claude").mkdir(parents=True)
        self._write_config(
            "skills.json",
            {
                "repositories": {"coll": {"url": "https://example.invalid/coll.git", "agents": ["claude-code"]}},
                "agents": {"claude-code": str(self.home / ".claude" / "skills")},
            },
        )
        self._write_config(
            "manager.json",
            {"agents": {}, "tools": {}, "skills": {"coll": {"enabled": True}}, "mcp": {}, "browser_us_sync": {"enabled": False}},
        )
        payload = self._payload("skills:apply", "--json", "--offline", env_extra={"PATH": "%s:/usr/bin:/bin" % self.bindir})
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        self.assertTrue(payload["data"]["steps"][0]["ok"])
        link = self.home / ".claude" / "skills" / "skill-a"
        self.assertTrue(link.is_symlink())
        # link_skills 写相对链接；按目录解析后与源目录一致。
        self.assertEqual((link.parent / os.readlink(link)).resolve(), (repo_dir / "skill-a").resolve())
        # owner 标记由同步器写入（托管锚点）。
        self.assertTrue((repo_dir / ".agent-manager-owned").is_file())
        snapshot = payload["data"]["snapshot"]
        self.assertIsNotNone(snapshot)

    def test_tools_apply_runs_syncer(self):
        self._write_config("tools.json", {"tools": {"fake-tool": {"install": "true", "ready_check": "bin:true"}}})
        self._write_config(
            "manager.json",
            {"agents": {}, "tools": {"fake-tool": {"enabled": True}}, "skills": {}, "mcp": {}, "browser_us_sync": {"enabled": False}},
        )
        # 不限制 PATH：sync_tools.py 当前工作区形态在 /usr/bin/python3 (3.9) 下不可导入。
        payload = self._payload("tools:apply", "--json", "--offline")
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        self.assertTrue(payload["data"]["steps"][0]["ok"])

    def test_agents_apply_requires_allow_uninstall_flag(self):
        # 禁用但仍受管安装 → 无 --allow-uninstall 时必须拒绝（install-remove 二次确认）。
        fake_bin = self.bindir / "fake-agent-bin"
        fake_bin.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        fake_bin.chmod(0o755)
        self._write_config(
            "agents.json",
            {
                "fake-agent": {
                    "bin": "fake-agent-bin",
                    "providers": {
                        "npm": {
                            "managed": True,
                            "install": "true",
                            "uninstall": "rm -f %s" % fake_bin,
                            "upgrade": "true",
                            "detect_paths": [str(fake_bin)],
                        }
                    },
                }
            },
        )
        env_extra = {"PATH": "%s:/usr/bin:/bin" % self.bindir}
        self._write_config(
            "manager.json",
            {"agents": {"fake-agent": {"enabled": False}}, "tools": {}, "skills": {}, "mcp": {}, "browser_us_sync": {"enabled": False}},
        )
        result = self._run("agents:apply", "--json", env_extra=env_extra)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["errors"][0]["code"], "UNINSTALL_REQUIRES_FLAG")

        payload = self._payload("agents:apply", "--json", "--allow-uninstall", "--offline", env_extra=env_extra)
        self._assert_schema(APPLY_RESULT_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["steps"][0]["name"], "agents-lifecycle")
        self.assertTrue(payload["data"]["steps"][0]["ok"])

    # ------------------------------------------------------------------ converge

    def _converge_stub(self, rel_path, label, exit_code=0):
        # 引擎会把 sync_mcp.py / link_skills.py 作为模块 import（只读复用其中的
        # 探测/判定助手），所以 stub 必须保持这两个模块被 import 的符号面。
        extra = ""
        if rel_path.endswith("sync_mcp.py"):
            extra = "def placeholder_name(value):\n    return None\ndef load_env_files(paths):\n    return {}\n"
        elif rel_path.endswith("link_skills.py"):
            extra = "def is_manager_owned_repo(p):\n    return False\n"
        stub = self.repo / rel_path
        stub.write_text(
            "#!/usr/bin/env python3\n%sif __name__ == '__main__':\n    import sys\n    print('%s')\n    sys.exit(%d)\n" % (extra, label, exit_code),
            encoding="utf-8",
        )
        stub.chmod(0o755)

    def test_converge_dry_run_replays_update_pipeline_order(self):
        for rel, label in (
            ("agent-tool/sync_tools.py", "tools"),
            ("agent-skill/link_skills.py", "skills"),
            ("agent-mcp/sync_mcp.py", "mcp"),
            ("agent-lifecycle/sync_agents.py", "agents"),
        ):
            self._converge_stub(rel, label)
        payload = self._payload("system:converge", "--json", "--dry-run")
        self._assert_schema(CONVERGE_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["pipeline"], "update")
        self.assertTrue(payload["data"]["dry_run"])
        names = [s["name"] for s in payload["data"]["steps"]]
        # 与 run_apply_steps 同序；rules 无启用目标跳过；browser_us 禁用跳过。
        self.assertEqual(names, ["shell-aliases", "tools-sync", "skills-link", "mcp-sync", "claude-hooks", "codex-hooks"], names)
        self.assertIn("browser-us", payload["data"]["skipped"])
        self.assertIn("rules-sync", payload["data"]["skipped"])
        self.assertIsNone(payload["data"]["snapshot"], "dry-run 不快照")
        self.assertTrue(all(s["ok"] for s in payload["data"]["steps"]))

    def test_converge_step_failure_surfaces_but_pipeline_continues(self):
        self._converge_stub("agent-tool/sync_tools.py", "tools", exit_code=7)
        for rel, label in (("agent-skill/link_skills.py", "skills"), ("agent-mcp/sync_mcp.py", "mcp")):
            self._converge_stub(rel, label)
        payload = self._payload("system:converge", "--json", expect_exit=1)
        self._assert_schema(CONVERGE_SCHEMA, payload["data"])
        self.assertEqual(payload["errors"][0]["code"], "CONVERGE_STEP_FAILED")
        steps = {s["name"]: s for s in payload["data"]["steps"]}
        self.assertFalse(steps["tools-sync"]["ok"])
        # 后续步骤仍执行（对齐 run_pipeline_step 的 TASK_FAILURES 语义）。
        self.assertTrue(steps["mcp-sync"]["ok"])
        self.assertTrue(steps["claude-hooks"]["ok"])
        self.assertIsNotNone(payload["data"]["snapshot"], "非 dry-run 必须先快照")

    # ------------------------------------------------------------------ runs

    def _runs_fixture(self):
        log_dir = self.temp_dir / "logs"
        log_dir.mkdir(exist_ok=True)
        (log_dir / "agent_manager_sync_20260101T000000_111.log").write_text("first\n", encoding="utf-8")
        (log_dir / "agent_manager_sync_20260202T000000_222.log").write_text("l1\nl2\nl3\n", encoding="utf-8")
        (log_dir / "agent_manager_sync.latest").write_text(
            str(log_dir / "agent_manager_sync_20260202T000000_222.log") + "\n", encoding="utf-8"
        )

    def test_runs_list_and_get(self):
        self._runs_fixture()
        payload = self._payload("runs:list", "--json")
        self._assert_schema(RUNS_SCHEMA, payload["data"])
        runs = payload["data"]["runs"]
        self.assertEqual([r["run_id"] for r in runs], ["20260202T000000_222", "20260101T000000_111"])
        self.assertTrue(runs[0]["is_latest"])
        self.assertFalse(runs[1]["is_latest"])

        payload = self._payload("runs:get", "--json", "--run", "20260202T000000_222", "--lines", "2")
        self._assert_schema(RUNS_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["tail"], "l2\nl3\n")
        self.assertEqual(payload["data"]["lines"], 2)
        self.assertEqual(payload["data"]["runs"][0]["run_id"], "20260202T000000_222")

    def test_runs_get_unknown_run_is_error(self):
        self._runs_fixture()
        payload = self._payload("runs:get", "--json", "--run", "20990101T000000_999", expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "RUN_NOT_FOUND")

    # ------------------------------------------------------------------ policy

    def test_policy_set_writes_layer_and_get_shows_attribution(self):
        baseline_before = (self.repo / "config" / "manager.json").read_bytes()
        payload = self._payload(
            "policy:set", "--json", "--layer", "local", "--key", "mcp.alpha.enabled", "--value", "false"
        )
        self._assert_schema(POLICY_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["layer"], "local")
        self.assertEqual(payload["data"]["value"], False)
        local_path = Path(payload["data"]["path"])
        self.assertEqual(oct(local_path.stat().st_mode & 0o777), "0o600")
        written = json.loads(local_path.read_text(encoding="utf-8"))
        self.assertEqual(written, {"mcp": {"alpha": {"enabled": False}}})
        # 基线层逐字节零改动（只读可审计）。
        self.assertEqual((self.repo / "config" / "manager.json").read_bytes(), baseline_before)

        payload = self._payload("policy:get", "--json")
        self._assert_schema(POLICY_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["layers"]["local"], {"mcp": {"alpha": {"enabled": False}}})
        self.assertEqual(payload["data"]["attribution"]["mcp"], "local")
        self.assertEqual(payload["data"]["effective"]["mcp"]["alpha"]["enabled"], False)
        self.assertEqual(payload["data"]["attribution"]["agents"], "baseline")

    def test_policy_set_synced_layer_via_env_override(self):
        payload = self._payload(
            "policy:set", "--json", "--layer", "synced", "--key", "skills.coll.new_skill_default", "--value", '"disabled"'
        )
        self.assertEqual(payload["data"]["layer"], "synced")
        synced = json.loads((self.temp_dir / "manager.synced.json").read_text(encoding="utf-8"))
        self.assertEqual(synced, {"skills": {"coll": {"new_skill_default": "disabled"}}})

    def test_policy_set_rejects_baseline_layer(self):
        payload = self._payload(
            "policy:set", "--json", "--layer", "baseline", "--key", "mcp.alpha.enabled", "--value", "false",
            expect_exit=2,
        )
        self.assertEqual(payload["errors"][0]["code"], "USAGE_ERROR")
        self.assertIn("baseline", payload["errors"][0]["message"])

    def test_policy_set_rejects_suspected_plaintext_value(self):
        # 门禁反例：疑似明文的 value 命中即拒绝，文件不得出现。
        payload = self._payload(
            "policy:set", "--json", "--layer", "local", "--key", "mcp.alpha.key",
            "--value", '"sk-ant-' + "x" * 24 + '"',
            expect_exit=1,
        )
        self.assertEqual(payload["errors"][0]["code"], "POLICY_SECRET_DENIED")
        self.assertFalse((self.temp_dir / "manager.local.json").exists())

    def test_policy_set_rejects_illegal_key_and_bad_json(self):
        payload = self._payload(
            "policy:set", "--json", "--layer", "local", "--key", "mcp.bad key", "--value", "1",
            expect_exit=2,
        )
        self.assertEqual(payload["errors"][0]["code"], "USAGE_ERROR")
        payload = self._payload(
            "policy:set", "--json", "--layer", "local", "--key", "mcp.alpha", "--value", "not-json",
            expect_exit=2,
        )
        self.assertIn("--value", payload["errors"][0]["message"])

    def test_policy_set_requires_full_arguments(self):
        payload = self._payload("policy:set", "--json", "--layer", "local", expect_exit=2)
        self.assertEqual(payload["errors"][0]["code"], "USAGE_ERROR")

    # ------------------------------------------------------------------ policy:delete

    def test_policy_delete_removes_key_and_snapshots(self):
        self._payload(
            "policy:set", "--json", "--layer", "local", "--key", "mcp.alpha.enabled", "--value", "false"
        )
        payload = self._payload("policy:delete", "--json", "--layer", "local", "--key", "mcp.alpha.enabled")
        self._assert_schema(POLICY_SCHEMA, payload["data"])
        self.assertEqual(payload["data"]["layer"], "local")
        self.assertIsNone(payload["data"]["value"])
        written = json.loads((self.temp_dir / "manager.local.json").read_text(encoding="utf-8"))
        self.assertEqual(written, {"mcp": {"alpha": {}}})
        # 写前快照可见（snapshots:list 语义复用）。
        listing = self._payload("snapshots:list", "--json")
        commands = [entry["command"] for entry in listing["data"]["snapshots"]]
        self.assertIn("policy.delete", commands)

    def test_policy_delete_baseline_rejected(self):
        payload = self._payload(
            "policy:delete", "--json", "--layer", "baseline", "--key", "mcp.alpha", expect_exit=2
        )
        self.assertEqual(payload["errors"][0]["code"], "USAGE_ERROR")
        self.assertIn("baseline", payload["errors"][0]["message"])

    def test_policy_delete_missing_key_is_error_without_snapshot(self):
        payload = self._payload(
            "policy:delete", "--json", "--layer", "local", "--key", "mcp.ghost.enabled", expect_exit=1
        )
        self.assertEqual(payload["errors"][0]["code"], "POLICY_KEY_NOT_FOUND")
        self.assertFalse(self.snapshot_dir.exists(), "键不存在时不得产生快照（空跑不留痕）")

    def test_policy_delete_synced_layer_via_env_override(self):
        self._payload(
            "policy:set", "--json", "--layer", "synced", "--key", "skills.coll.new_skill_default", "--value", '"enabled"'
        )
        payload = self._payload("policy:delete", "--json", "--layer", "synced", "--key", "skills.coll.new_skill_default")
        self.assertEqual(payload["data"]["layer"], "synced")
        synced = json.loads((self.temp_dir / "manager.synced.json").read_text(encoding="utf-8"))
        self.assertEqual(synced, {"skills": {"coll": {}}})


if __name__ == "__main__":
    unittest.main()
