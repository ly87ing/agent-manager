"""Definition overlay tests (P4a): policy layers carry definition fragments
(mcp.definitions.<name> / skills.repositories.<name>) deep-merged over the
committed definition files, with `_delete: true` tombstones and fail-close
shape validation. Consumers covered: engine status/plan kernels, sync_mcp.py
(real injection), link_skills.py (real linking), manage_doctor.py expected sets.
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANAGE = ROOT / "manage.sh"
COPY_IGNORE = shutil.ignore_patterns(
    "repositories", ".git", "__pycache__", "secrets", ".env", "app", "node_modules", "engine-rs", "manager.local.json", "manager.synced.json"
)


class DefinitionOverlayTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_overlay_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        (self.repo / "config" / "manager.local.json").unlink(missing_ok=True)
        (self.repo / "config" / "manager.synced.json").unlink(missing_ok=True)
        self.agent_a = self.home / ".agent-a.json"
        # agent-a 须可被探测为「已安装」，否则 sync_mcp 走「未安装仅清理」语义
        # （is_agent_installed 的 detect_paths 通道，指向一个常备标记文件）。
        self.agent_marker = self.temp_dir / "agent-a.installed"
        self.agent_marker.write_text("installed\n", encoding="utf-8")
        self._write_config("agents.json", {"agent-a": {"bin": "__missing_agent_a__", "providers": {"native": {"managed": False, "detect_paths": [str(self.agent_marker)]}}}})
        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--x"], "env": {}, "agents": ["agent-a"]},
                    "gamma": {"command": "gamma-cmd", "args": [], "env": {}, "agents": ["agent-a"]},
                },
                "agent_paths": {"agent-a": str(self.agent_a)},
            },
        )
        self._write_config(
            "manager.json",
            {"agents": {}, "tools": {}, "skills": {}, "mcp": {"alpha": {"enabled": True}, "gamma": {"enabled": True}}},
        )

    # ------------------------------------------------------------------ helpers

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _env(self, extra=None):
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.temp_dir / "lock")
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.temp_dir / "logs" / "sync")
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "no-cache.json")
        env["AGENT_MANAGER_SYNCED_CONFIG"] = str(self.temp_dir / "manager.synced.json")
        env["AGENT_MANAGER_LOCAL_CONFIG"] = str(self.temp_dir / "manager.local.json")
        if extra:
            env.update(extra)
        return env

    def _run(self, *argv, env_extra=None):
        (self.temp_dir / "logs").mkdir(exist_ok=True)
        return subprocess.run(
            [str(self.repo / "manage.sh"), *argv],
            cwd=self.repo, env=self._env(env_extra), capture_output=True, text=True, check=False, timeout=60,
        )

    def _status_elements(self, *argv, env_extra=None):
        result = self._run(*argv, env_extra=env_extra)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        return payload, {e["id"]: e for e in payload["data"]["elements"]}

    def _write_overlay(self, layer, content):
        """layer: 'synced'|'local' — 写到 env 重定向的层文件。"""
        path = self.temp_dir / f"manager.{layer}.json"
        path.write_text(json.dumps(content, indent=2) + "\n", encoding="utf-8")

    # ------------------------------------------------------------------ mcp 新增/编辑/tombstone

    def test_source_commands_save_readback_and_reject_stale_revision(self):
        self._write_config("shell-aliases.json", {"aliases": {"old": "echo old"}, "timezone": "UTC"})
        result = self._run("sources:get", "--key", "aliases", "--json")
        self.assertEqual(result.returncode, 0, result.stderr)
        source = json.loads(result.stdout)["data"]["sources"][0]
        value = json.dumps({"revision": source["revision"], "value": {"new": "echo new"}})
        saved = self._run("sources:set", "--key", "aliases.shell", "--value", value, "--json")
        self.assertEqual(saved.returncode, 0, saved.stdout + saved.stderr)
        self.assertTrue(json.loads(saved.stdout)["ok"])
        document = json.loads((self.repo / "config/shell-aliases.json").read_text())
        self.assertEqual(document, {"aliases": {"new": "echo new"}, "timezone": "UTC"})
        stale = self._run("sources:set", "--key", "aliases.shell", "--value", value, "--json")
        self.assertNotEqual(stale.returncode, 0)
        self.assertIn("重新加载", json.loads(stale.stdout)["errors"][0]["message"])

    def test_agent_definition_add_edit_remove_uses_effective_install_command(self):
        definition = {"bin": "__missing_custom_agent__", "providers": {"npm": {"managed": True, "install": "echo install-custom", "upgrade": "echo upgrade-custom"}}}
        self._write_overlay("local", {"agent_definitions": {"custom": definition}, "agents": {"custom": {"enabled": True}}})
        _, elements = self._status_elements("agents:status", "--json")
        self.assertEqual(elements["agents:custom"]["spec"]["bin"], "__missing_custom_agent__")
        plan = self._run("agents:plan", "--only", "agents:custom", "--json")
        self.assertEqual(plan.returncode, 0, plan.stderr)
        self.assertIn("echo install-custom", json.dumps(json.loads(plan.stdout)["data"]["actions"]))
        self._write_overlay("local", {"agent_definitions": {"custom": {"_delete": True}}, "agents": {"custom": {"enabled": True}}})
        _, elements = self._status_elements("agents:status", "--json")
        self.assertNotIn("agents:custom", elements)

    def test_agent_full_definition_replaces_lower_provider_keys_and_rejects_invalid_save(self):
        self._write_overlay("synced", {"agent_definitions": {"custom": {"bin": "__missing_custom__", "providers": {"native": {"install": "echo old"}}}}})
        self._write_overlay("local", {"agent_definitions": {"custom": {"_replace": True, "bin": "__missing_custom__", "providers": {"npm": {"install": "echo new"}}}}})
        _, elements = self._status_elements("agents:status", "--json")
        self.assertEqual(elements["agents:custom"]["spec"]["providers"], {"npm": {"install": "echo new"}})
        before = (self.temp_dir / "manager.local.json").read_bytes()
        result = self._run("policy:set", "--layer", "local", "--key", "agent_definitions.invalid", "--value", '{"bin":""}', "--json")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.temp_dir / "manager.local.json").read_bytes(), before)

    def test_tool_definition_add_remove_has_no_phantom_policy_error(self):
        self._write_config("tools.json", {"tools": {}})
        self._write_overlay("local", {"tool_definitions": {"custom": {"install": "echo install-tool", "bin": "__missing_custom_tool__"}}, "tools": {"custom": {"enabled": True}}})
        _, elements = self._status_elements("tools:status", "--json")
        self.assertIn("tools:custom", elements)
        self._write_overlay("local", {"tool_definitions": {"custom": {"_delete": True}}, "tools": {"custom": {"enabled": True}}})
        payload, elements = self._status_elements("tools:status", "--json")
        self.assertNotIn("tools:custom", elements)
        self.assertEqual(payload["errors"], [])

    def test_mcp_overlay_add_new_server_end_to_end(self):
        # 仅本机层定义出新 server + 策略启用 → status 出现元素 + plan 有注入动作 +
        # 真跑 sync_mcp 后沙盒目标文件被注入。
        # 同步器对「未安装且无配置文件」的 agent 跳过注入——预建空目标文件使流程可达。
        self.agent_a.write_text('{"mcpServers": {}}\n', encoding="utf-8")
        self._write_overlay("local", {
            "mcp": {
                "definitions": {"local-srv": {"command": "local-cmd", "args": ["--local"], "agents": ["agent-a"]}},
                "local-srv": {"enabled": True},
            }
        })
        payload, elements = self._status_elements("mcp:status", "--json")
        self.assertIn("mcp:local-srv", elements)
        self.assertEqual(elements["mcp:local-srv"]["state"], "missing", "目标文件已预建（同步器门槛），元素为缺注入")
        self.assertEqual(elements["mcp:local-srv"]["spec"]["command"], "local-cmd")

        plan = self._run("mcp:plan", "--json")
        self.assertEqual(plan.returncode, 0, plan.stderr)
        actions = json.loads(plan.stdout)["data"]["actions"]
        self.assertIn("mcp:local-srv", [a["element"] for a in actions])

        apply_result = self._run("mcp:apply", "--json", "--offline")
        self.assertEqual(apply_result.returncode, 0, apply_result.stdout + apply_result.stderr)
        injected = json.loads(self.agent_a.read_text(encoding="utf-8"))
        self.assertEqual(injected["mcpServers"]["local-srv"]["command"], "local-cmd")

    def test_mcp_overlay_edit_keeps_baseline_keys(self):
        # 覆盖 args 键，基线的 command/env/agents 保留（深合并语义）。
        self._write_overlay("local", {"mcp": {"definitions": {"alpha": {"args": ["--edited"]}}}})
        _payload, elements = self._status_elements("mcp:status", "--json")
        alpha = elements["mcp:alpha"]
        self.assertEqual(alpha["spec"]["args"], ["--edited"], "覆盖层 args 生效")
        self.assertEqual(alpha["spec"]["command"], "alpha-cmd", "基线 command 保留")
        # 生效定义漂移判定也用合并后语义：注入旧 args 的文件应判 drifted。
        self.agent_a.write_text(
            json.dumps({"mcpServers": {"alpha": {"command": "alpha-cmd", "args": ["--x"], "env": {"_MANAGED_BY": "agent-manager"}}}}) + "\n",
            encoding="utf-8",
        )
        _payload, elements = self._status_elements("mcp:status", "--json")
        self.assertEqual(elements["mcp:alpha"]["state"], "drifted", "生效定义=基线⊕覆盖，旧注入应判漂移")

    def test_mcp_overlay_tombstone_removes_baseline_server(self):
        # tombstone：删除定义后目标文件保留可验证残留时，status 呈现为 drifted/disabled policy，
        # 且仅生成该元素的清理计划；通过 --only 离线清理后，目标文件移除该 server、status 不再出现、doctor 正常。
        before_target_cfg = {
            "mcpServers": {
                "alpha": {"command": "alpha-cmd", "args": ["--x"], "env": {"_MANAGED_BY": "agent-manager"}},
                "gamma": {"command": "gamma-cmd", "args": [], "env": {"_MANAGED_BY": "agent-manager"}},
            }
        }
        self.agent_a.write_text(
            json.dumps(before_target_cfg) + "\n",
            encoding="utf-8",
        )
        alpha_before = json.loads(json.dumps(before_target_cfg["mcpServers"]["alpha"]))
        self._write_overlay("local", {"mcp": {"definitions": {"gamma": {"_delete": True}}}})
        _payload, elements = self._status_elements("mcp:status", "--json")
        self.assertIn("mcp:gamma", elements, "清理前保留可验证受管残留供精确清理")
        self.assertEqual(elements["mcp:gamma"]["state"], "drifted")
        self.assertFalse(elements["mcp:gamma"]["policy"]["enabled"])
        self.assertFalse(elements["mcp:gamma"]["policy"]["declared_in_policy"])
        self.assertIn("mcp:alpha", elements)

        plan_res = self._run("mcp:plan", "--json", "--offline", "--only", "mcp:gamma")
        self.assertEqual(plan_res.returncode, 0, plan_res.stdout + plan_res.stderr)
        plan_data = json.loads(plan_res.stdout)["data"]
        self.assertTrue(plan_data["actions"], "应生成 gamma 的清理动作")
        self.assertEqual({a["element"] for a in plan_data["actions"]}, {"mcp:gamma"})

        apply_result = self._run("mcp:apply", "--json", "--offline", "--only", "mcp:gamma")
        self.assertEqual(apply_result.returncode, 0, apply_result.stdout + apply_result.stderr)
        injected = json.loads(self.agent_a.read_text(encoding="utf-8"))
        self.assertNotIn("gamma", injected["mcpServers"])
        self.assertIn("alpha", injected["mcpServers"])
        self.assertEqual(injected["mcpServers"]["alpha"], alpha_before, "未选对象 alpha 完整对象（含 env 及其他字段）原样保留")

        _payload_after, elements_after = self._status_elements("mcp:status", "--json")
        self.assertNotIn("mcp:gamma", elements_after, "清理后元素彻底消失")
        self.assertIn("mcp:alpha", elements_after)

        doctor = self._run("doctor", "--json")
        self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)
        report = json.loads(doctor.stdout)
        mcp_entries = report["sections"]["mcp"]
        self.assertFalse(
            any(e["status"] == "error" for e in mcp_entries),
            "doctor 不得对 tombstone 后的集合误报漂移: %s" % mcp_entries,
        )

    def test_mcp_overlay_shape_fail_close(self):
        # 反例：非 tombstone 片段缺必填键 command → fail-close。
        self._write_overlay("local", {"mcp": {"definitions": {"bad": {"args": []}}}})
        result = self._run("mcp:status", "--json")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["errors"][0]["code"], "CONFIG_INVALID")
        self.assertIn("command", payload["errors"][0]["message"])

    def test_mcp_overlay_layer_priority_local_over_synced(self):
        # 三层优先级：local 覆盖 synced 覆盖基线。
        self._write_overlay("synced", {"mcp": {"definitions": {"alpha": {"args": ["--synced"]}}}})
        self._write_overlay("local", {"mcp": {"definitions": {"alpha": {"args": ["--local"]}}}})
        _payload, elements = self._status_elements("mcp:status", "--json")
        self.assertEqual(elements["mcp:alpha"]["spec"]["args"], ["--local"])

    def test_mcp_overlay_unknown_keys_warn_not_reject(self):
        self._write_overlay("local", {"mcp": {"definitions": {"alpha": {"bogus_key": 1}}}})
        payload, elements = self._status_elements("mcp:status", "--json")
        self.assertIn("mcp:alpha", elements)
        self.assertIn("DEFINITION_OVERLAY_UNKNOWN_KEYS", [w["code"] for w in payload["warnings"]])

    # ------------------------------------------------------------------ skills 新增/tombstone

    def _skills_fixture(self):
        coll = self.repo / "agent-skill" / "repositories" / "coll"
        (coll / "skill-a").mkdir(parents=True)
        (coll / "skill-a" / "SKILL.md").write_text("---\nname: skill-a\ndescription: t\n---\n", encoding="utf-8")
        legacy = self.repo / "agent-skill" / "repositories" / "legacy"
        (legacy / "skill-x").mkdir(parents=True)
        (legacy / "skill-x" / "SKILL.md").write_text("---\nname: skill-x\ndescription: t\n---\n", encoding="utf-8")
        claude_skills = self.home / ".claude" / "skills"
        claude_skills.mkdir(parents=True)
        self._write_config("skills.json", {
            "repositories": {
                "coll": {"url": "https://example.invalid/coll.git", "agents": ["claude-code"]},
                "legacy": {"url": "https://example.invalid/legacy.git", "agents": ["claude-code"]},
            },
            "agents": {"claude-code": str(claude_skills)},
        })
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["skills"] = {"coll": {"enabled": True}, "legacy": {"enabled": True}}
        self._write_config("manager.json", manager)
        return claude_skills

    def test_skills_overlay_tombstone_and_add(self):
        claude_skills = self._skills_fixture()
        # link_skills 只接管 git 仓库形态的集合缓存——fixture 的集合目录补 .git。
        for name in ("coll", "legacy"):
            repo_dir = self.repo / "agent-skill" / "repositories" / name
            subprocess.run(["git", "init", "-q", str(repo_dir)], check=True, capture_output=True)
        # 新增集合：仅 local 层定义（仓库缓存目录手工放置，等价于已拉取）。
        local_coll = self.repo / "agent-skill" / "repositories" / "local-coll"
        (local_coll / "skill-l").mkdir(parents=True)
        (local_coll / "skill-l" / "SKILL.md").write_text("---\nname: skill-l\ndescription: t\n---\n", encoding="utf-8")
        subprocess.run(["git", "init", "-q", str(local_coll)], check=True, capture_output=True)
        self._write_overlay("local", {
            "skills": {
                "repositories": {
                    "local-coll": {"url": "https://example.invalid/local.git", "agents": ["claude-code"]},
                    "legacy": {"_delete": True},
                },
                "local-coll": {"enabled": True},
            }
        })
        _payload, elements = self._status_elements("skills:status", "--json")
        self.assertIn("skill:local-coll", elements)
        self.assertNotIn("skill:legacy", elements, "tombstone 后集合元素不再出现")
        self.assertNotIn("skill:legacy/skill-x", elements)

        apply_result = self._run("skills:apply", "--json", "--offline", env_extra={
            "PATH": os.environ.get("PATH", ""),
        })
        self.assertEqual(apply_result.returncode, 0, apply_result.stdout + apply_result.stderr)
        self.assertTrue((claude_skills / "skill-l").is_symlink(), "新增集合的 skill 应被链接")
        self.assertFalse((claude_skills / "skill-x").exists(), "tombstone 集合不得产生链接")

        doctor = self._run("doctor", "--json")
        self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)
        report = json.loads(doctor.stdout)
        skills_entries = report["sections"]["skills"]
        self.assertFalse(
            any(e["status"] == "error" for e in skills_entries),
            "doctor 不得对 tombstone 后的集合误报: %s" % skills_entries,
        )


if __name__ == "__main__":
    unittest.main()
