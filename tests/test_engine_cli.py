"""End-to-end CLI tests for the Engine Contract v1 vertical slice (mcp:status).

Sandbox pattern mirrors tests/test_manage.py: the repo is copied into a temp
directory (secrets/, .env and repository caches excluded), config files are
rewritten with hermetic definitions pointing at a temp HOME, and manage.sh is
run as a subprocess with env overrides for lock/log locations.

Every payload is validated against contract/engine-v1/*.schema.json with the
zero-dependency tests/contract_validator.py — the same checker the future Rust
slices must satisfy.
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
STATUS_SCHEMA = load_schema(CONTRACT_DIR / "status.schema.json")
PLAN_SCHEMA = load_schema(CONTRACT_DIR / "plan.schema.json")

# Sentinel values that must never leak into any engine payload (secrets red line).
ALPHA_SECRET = "engine-test-fake-alpha-secret-001"
LIVE_SECRET = "engine-test-fake-live-secret-002"

COPY_IGNORE = shutil.ignore_patterns("repositories", ".git", "__pycache__", "secrets", ".env", "app", "node_modules", "engine-rs")


class EngineCliTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_engine_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        (self.repo / "config" / "manager.local.json").unlink(missing_ok=True)
        (self.repo / "config" / "manager.synced.json").unlink(missing_ok=True)
        self.lock_path = self.temp_dir / "agent_manager.lock"
        self.log_prefix = self.temp_dir / "agent_manager_sync"
        self._write_default_configs()

    # ------------------------------------------------------------------ helpers

    def _write_default_configs(self):
        """Hermetic policy + mcp definitions covering every convergence state.

        alpha   injected, args/env match       -> managed-ok
        beta    enabled, absent from target    -> missing
        gamma   policy enabled=false           -> disabled
        delta   defined but not in policy      -> disabled
        epsilon enabled, agent-b file absent   -> not-materialized
        zeta    injected, args drifted         -> drifted
        eta     enabled for toml agent-b       -> not-materialized until file written
        """
        manager = {
            "agents": {},
            "tools": {},
            "skills": {},
            "mcp": {
                "alpha": {"enabled": True},
                "beta": {"enabled": True},
                "gamma": {"enabled": False},
                "epsilon": {"enabled": True},
                "zeta": {"enabled": True},
                "eta": {"enabled": True},
            },
        }
        (self.repo / "config" / "manager.json").write_text(
            json.dumps(manager, indent=2) + "\n", encoding="utf-8"
        )
        agent_a_target = self.home / ".agent-a.json"
        agent_b_target = self.home / ".agent-b" / "config.toml"
        mcp_def = {
            "servers": {
                "alpha": {
                    "command": "alpha-cmd",
                    "args": ["--x"],
                    "env": {"ALPHA_KEY": "${ENGINE_TEST_ALPHA_KEY}"},
                    "agents": ["agent-a"],
                },
                "beta": {"command": "beta-cmd", "args": [], "agents": ["agent-a"]},
                "gamma": {"command": "gamma-cmd", "args": [], "agents": ["agent-a"]},
                "delta": {"command": "delta-cmd", "args": [], "agents": ["agent-a"]},
                "epsilon": {"command": "epsilon-cmd", "args": [], "agents": ["agent-b"]},
                "zeta": {"command": "zeta-cmd", "args": ["--z"], "agents": ["agent-a"]},
                "eta": {"command": "eta-cmd", "args": ["--e"], "agents": ["agent-b"]},
            },
            "agent_paths": {
                "agent-a": str(agent_a_target),
                "agent-b": {"path": str(agent_b_target), "format": "toml"},
            },
        }
        (self.repo / "config" / "mcp.json").write_text(
            json.dumps(mcp_def, indent=2) + "\n", encoding="utf-8"
        )
        agent_a_target.write_text(
            json.dumps(
                {
                    "mcpServers": {
                        "alpha": {
                            "command": "alpha-cmd",
                            "args": ["--x"],
                            "env": {"ALPHA_KEY": ALPHA_SECRET, "_MANAGED_BY": "agent-manager"},
                        },
                        "zeta": {
                            "command": "zeta-cmd",
                            "args": ["--drifted"],
                            "env": {"_MANAGED_BY": "agent-manager"},
                        },
                    }
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def _write_agent_b_toml(self, servers):
        target = self.home / ".agent-b" / "config.toml"
        target.parent.mkdir(parents=True, exist_ok=True)
        blocks = []
        for name, (command, args) in sorted(servers.items()):
            args_toml = "[" + ", ".join(json.dumps(arg) for arg in args) + "]"
            blocks.append(
                "[mcp_servers.%s]\ncommand = %s\nargs = %s\n\n[mcp_servers.%s.env]\n_MANAGED_BY = \"agent-manager\"\n"
                % (name, json.dumps(command), args_toml, name)
            )
        target.write_text("\n".join(blocks), encoding="utf-8")

    def _env(self, extra=None):
        env = os.environ.copy()
        env.pop("ENGINE_TEST_ALPHA_KEY", None)
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.lock_path)
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.log_prefix)
        if extra:
            env.update(extra)
        return env

    def _run_manage(self, *argv, env_extra=None):
        return subprocess.run(
            [str(self.repo / "manage.sh"), *argv],
            cwd=self.repo,
            env=self._env(env_extra),
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )

    def _run_mcp_status(self, *prefix, env_extra=None):
        result = self._run_manage(*prefix, "mcp:status", "--json", env_extra=env_extra)
        payload = json.loads(result.stdout)  # stdout must be exactly one JSON document
        return result, payload

    def _assert_envelope(self, payload):
        errors = validate(ENVELOPE_SCHEMA, payload)
        self.assertEqual(errors, [], "envelope schema violations: %s" % [str(e) for e in errors])

    def _assert_status_data(self, payload):
        errors = validate(STATUS_SCHEMA, payload["data"])
        self.assertEqual(errors, [], "status schema violations: %s" % [str(e) for e in errors])

    @staticmethod
    def _elements_by_id(payload):
        return {element["id"]: element for element in payload["data"]["elements"]}

    # ------------------------------------------------------- positive scenarios

    def test_mcp_status_output_validates_against_contract_schemas(self):
        result, payload = self._run_mcp_status(env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_envelope(payload)
        self._assert_status_data(payload)

        self.assertEqual(payload["contract"], "agent-manager/engine/v1")
        self.assertEqual(payload["command"], "mcp.status")
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["errors"], [])
        self.assertEqual(payload["warnings"], [])
        self.assertEqual(payload["mode"], {"dry_run": False, "offline": False})
        self.assertEqual(payload["lock"], {"held": False, "pid": None})
        self.assertTrue(payload["run_id"])
        self.assertTrue(str(payload["log_file"]).endswith(".log"))
        self.assertEqual(payload["data"]["domain"], "mcp")

    def test_mcp_status_element_states_cover_the_convergence_axis(self):
        result, payload = self._run_mcp_status(env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        elements = self._elements_by_id(payload)

        self.assertEqual(elements["mcp:alpha"]["state"], "managed-ok")
        self.assertEqual(elements["mcp:beta"]["state"], "missing")
        self.assertEqual(elements["mcp:gamma"]["state"], "disabled")
        self.assertEqual(elements["mcp:delta"]["state"], "disabled")
        self.assertEqual(elements["mcp:zeta"]["state"], "drifted")
        self.assertEqual(elements["mcp:epsilon"]["state"], "not-materialized")

        alpha = elements["mcp:alpha"]
        self.assertEqual(alpha["observed"]["agents"]["agent-a"]["state"], "managed-ok")
        self.assertEqual(alpha["observed"]["agents"]["agent-a"]["target"], str(self.home / ".agent-a.json"))
        self.assertEqual(
            alpha["observed"]["env"]["ALPHA_KEY"],
            {"placeholder": "ENGINE_TEST_ALPHA_KEY", "resolved": True},
        )
        self.assertEqual(alpha["policy"], {"enabled": True, "declared_in_policy": True, "agents": ["agent-a"]})
        self.assertEqual(alpha["freshness"]["state"], "not-applicable")
        self.assertEqual(
            alpha["actions"],
            [{"id": "apply", "risk": "write-local", "needs_lock": True, "offline_ok": True}],
        )
        # delta is defined in mcp.json but absent from the merged policy layer.
        self.assertEqual(elements["mcp:delta"]["policy"]["declared_in_policy"], False)
        self.assertEqual(elements["mcp:delta"]["observed"]["agents"]["agent-a"]["state"], "absent")

    def test_disabled_server_with_managed_leftover_is_drifting_not_disabled(self):
        # 反例：policy 禁用不等于收敛——目标文件里仍有托管残留时必须暴露为 drifted
        # （对齐 doctor「策略已禁用，但仍残留托管服务」的报错语义）。
        target = self.home / ".agent-a.json"
        injected = json.loads(target.read_text(encoding="utf-8"))
        injected["mcpServers"]["gamma"] = {
            "command": "gamma-cmd",
            "args": [],
            "env": {"_MANAGED_BY": "agent-manager"},
        }
        target.write_text(json.dumps(injected, indent=2) + "\n", encoding="utf-8")

        result, payload = self._run_mcp_status(env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_status_data(payload)
        gamma = self._elements_by_id(payload)["mcp:gamma"]
        self.assertEqual(gamma["state"], "drifted")
        self.assertEqual(gamma["policy"]["enabled"], False)
        self.assertEqual(gamma["observed"]["agents"]["agent-a"]["state"], "leftover")

    def test_mcp_plan_translates_non_converged_cells(self):
        result = self._run_manage("mcp:plan", "--json", env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        errors = validate(PLAN_SCHEMA, payload["data"])
        self.assertEqual(errors, [], "plan violations: %s" % [str(e) for e in errors])

        by_pair = {}
        for action in payload["data"]["actions"]:
            by_pair.setdefault(action["element"], []).append(action)
        # 一致/禁用（无残留）的元素不产生动作。
        self.assertNotIn("mcp:alpha", by_pair)
        self.assertNotIn("mcp:gamma", by_pair)
        self.assertNotIn("mcp:delta", by_pair)
        for expected in ("mcp:beta", "mcp:zeta", "mcp:epsilon", "mcp:eta"):
            self.assertIn(expected, by_pair)
        # beta 缺失但目标文件存在 → before 非空；epsilon/eta 目标文件不存在 → before 为 null。
        self.assertIsNotNone(by_pair["mcp:beta"][0]["before_digest"])
        self.assertIsNone(by_pair["mcp:epsilon"][0]["before_digest"])
        for action in payload["data"]["actions"]:
            self.assertEqual(action["op"], "write-file")
            self.assertEqual(action["risk"], "write-local")
        self.assertEqual(
            payload["data"]["summary"]["write-local"], len(payload["data"]["actions"])
        )

    def test_mcp_plan_leftover_removal_has_null_after_digest(self):
        target = self.home / ".agent-a.json"
        injected = json.loads(target.read_text(encoding="utf-8"))
        injected["mcpServers"]["gamma"] = {
            "command": "gamma-cmd",
            "args": [],
            "env": {"_MANAGED_BY": "agent-manager"},
        }
        target.write_text(json.dumps(injected, indent=2) + "\n", encoding="utf-8")

        result = self._run_manage("mcp:plan", "--json", env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        gamma_actions = [a for a in payload["data"]["actions"] if a["element"] == "mcp:gamma"]
        self.assertEqual(len(gamma_actions), 1)
        self.assertIsNone(gamma_actions[0]["after_digest"])
        self.assertIsNotNone(gamma_actions[0]["before_digest"])
        self.assertIn("残留", gamma_actions[0]["note"])

    def test_mcp_status_toml_target_round_trips_managed_ok(self):
        self._write_agent_b_toml({"eta": ("eta-cmd", ["--e"]), "epsilon": ("epsilon-cmd", [])})
        result, payload = self._run_mcp_status(env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_status_data(payload)
        elements = self._elements_by_id(payload)
        self.assertEqual(elements["mcp:eta"]["state"], "managed-ok")
        self.assertEqual(elements["mcp:epsilon"]["state"], "managed-ok")
        self.assertEqual(elements["mcp:eta"]["observed"]["agents"]["agent-b"]["state"], "managed-ok")

    def test_mcp_status_unresolved_placeholder_warns_with_variable_name_only(self):
        result, payload = self._run_mcp_status()  # ENGINE_TEST_ALPHA_KEY deliberately unset
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_envelope(payload)
        alpha_warnings = [w for w in payload["warnings"] if w["code"] == "MCP_ENV_UNRESOLVED"]
        self.assertEqual(len(alpha_warnings), 1)
        self.assertEqual(alpha_warnings[0]["element"], "mcp:alpha")
        self.assertIn("ENGINE_TEST_ALPHA_KEY", alpha_warnings[0]["message"])
        elements = self._elements_by_id(payload)
        self.assertEqual(
            elements["mcp:alpha"]["observed"]["env"]["ALPHA_KEY"],
            {"placeholder": "ENGINE_TEST_ALPHA_KEY", "resolved": False},
        )

    def test_mcp_status_never_emits_secret_values(self):
        # 反例：注入文件与 live 环境变量中的假值都不得出现在 stdout/stderr。
        result, payload = self._run_mcp_status(env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        combined = result.stdout + result.stderr
        self.assertNotIn(ALPHA_SECRET, combined)
        self.assertNotIn(LIVE_SECRET, combined)
        # 只有变量名与 resolved 布尔值允许出现。
        self.assertIn("ENGINE_TEST_ALPHA_KEY", result.stdout)

    def test_offline_and_dry_run_flags_propagate_into_mode(self):
        result, payload = self._run_mcp_status("--offline", "--dry-run", env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(payload["mode"], {"dry_run": True, "offline": True})

    def test_status_does_not_acquire_lock_or_touch_log_pointer(self):
        sleeper = subprocess.Popen(["sleep", "30"])
        self.addCleanup(lambda: (sleeper.kill(), sleeper.wait()))
        lock_dir = Path(str(self.lock_path) + ".d")
        lock_dir.mkdir()
        (lock_dir / "pid").write_text(str(sleeper.pid), encoding="utf-8")
        self.lock_path.write_text(str(sleeper.pid), encoding="utf-8")

        result, payload = self._run_mcp_status(env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        # 他人持有的活锁必须原样保留（只读命令对齐 doctor 豁免语义）。
        self.assertEqual(self.lock_path.read_text(encoding="utf-8"), str(sleeper.pid))
        self.assertFalse(Path(str(self.log_prefix) + ".latest").exists(), "只读命令不得更新 latest 日志指针")

    # ------------------------------------------------------- negative scenarios

    def test_unknown_domain_fails_with_envelope_error_and_exit_2(self):
        result = self._run_manage("foo:status", "--json")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["data"], {})
        self.assertEqual(payload["errors"][0]["code"], "UNKNOWN_DOMAIN")
        self.assertIn("foo", payload["errors"][0]["message"])

    def test_missing_json_flag_fails_with_envelope_error_and_exit_2(self):
        result = self._run_manage("mcp:status")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        self.assertEqual(payload["errors"][0]["code"], "JSON_REQUIRED")
        self.assertIn("--json", result.stderr)

    def test_unimplemented_domains_and_verbs_return_domain_not_implemented(self):
        # 全域 status 已接线；secrets 只读无 plan；profile:status/pull/push 已由
        # Phase 2b 接到 agent-profile/sync_profile.py（不再走引擎内核），
        # profile:plan 仍属后续 Phase。
        for argv in (
            ("secrets:plan", "--json"),
            ("profile:plan", "--json"),
        ):
            with self.subTest(argv=argv):
                result = self._run_manage(*argv)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                payload = json.loads(result.stdout)
                self._assert_envelope(payload)
                self.assertEqual(payload["errors"][0]["code"], "DOMAIN_NOT_IMPLEMENTED")

    def test_invalid_def_file_fails_with_envelope_error_and_exit_1(self):
        (self.repo / "config" / "mcp.json").write_text("{ invalid json\n", encoding="utf-8")
        result = self._run_manage("mcp:status", "--json")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "CONFIG_INVALID")

    # ------------------------------------------- existing behavior is unchanged

    def test_help_lists_engine_command_family_alongside_existing_commands(self):
        result = self._run_manage("help")
        self.assertEqual(result.returncode, 0, result.stderr)
        for token in ("update", "init", "agents:update", "doctor"):
            self.assertIn(token, result.stdout)
        for token in ("mcp:status", "*:status", "*:plan", "system:audit", "system:refresh-upstream"):
            self.assertIn(token, result.stdout)

    def test_update_dry_run_output_and_pipeline_are_unchanged(self):
        # 与本切片前一致的关键行仍必须出现；管线步骤全部按原顺序执行。
        # 恢复原始策略/定义（setUp 的 hermetic 配置会让 rules 目标集为空而跳过）。
        for name in ("manager.json", "mcp.json"):
            shutil.copy(ROOT / "config" / name, self.repo / "config" / name)
        call_log = self.temp_dir / "calls.log"

        def append_stub(rel_path, label):
            stub = self.repo / rel_path
            stub.write_text(
                "#!/usr/bin/env python3\n"
                "import sys\n"
                "open(%r, 'a', encoding='utf-8').write(%r + ' ' + ' '.join(sys.argv[1:]) + '\\n')\n"
                "sys.exit(0)\n" % (str(call_log), label),
                encoding="utf-8",
            )
            stub.chmod(0o755)

        append_stub("agent-lifecycle/sync_agents.py", "agents")
        append_stub("agent-tool/sync_tools.py", "tools")
        append_stub("agent-skill/link_skills.py", "skills")
        append_stub("agent-mcp/sync_mcp.py", "mcp")
        append_stub("agent-rule/scripts/sync-claude-hooks.py", "claude-hooks")
        append_stub("agent-rule/scripts/sync-codex-hooks.py", "codex-hooks")
        append_stub("scripts/sync_browser_google_us.py", "browser")
        rules_stub = self.repo / "agent-rule" / "scripts" / "sync-agent-rules.sh"
        rules_stub.write_text(
            "#!/usr/bin/env bash\nprintf '%%s\\n' \"rules $*\" >> %s\nexit 0\n" % str(call_log),
            encoding="utf-8",
        )
        rules_stub.chmod(0o755)

        result = subprocess.run(
            [str(self.repo / "manage.sh"), "update", "--dry-run"],
            cwd=self.repo,
            env=self._env(),
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)
        self.assertIn("Agent Manager 更新并收敛启动", combined)
        self.assertNotIn("HEALTH CHECK SKIPPED", combined)
        log_lines = call_log.read_text(encoding="utf-8").splitlines()
        for expected in ("tools ", "skills ", "mcp ", "rules ", "claude-hooks ", "codex-hooks "):
            self.assertTrue(
                any(line.startswith(expected) for line in log_lines),
                "管线缺步骤 %r: %s" % (expected, log_lines),
            )

    def test_unknown_command_still_fails_like_before(self):
        result = self._run_manage("frobnicate")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("未知命令 'frobnicate'", result.stdout + result.stderr)

    # ------------------------------------------------- rust slice routing (Phase 3)

    RUST_BIN = ROOT / "engine-rs" / "target" / "debug" / "agent-manager-engine"

    def test_mcp_routes_to_python_fallback_when_no_rust_binary(self):
        # 沙盒拷贝排除 engine-rs → 无 binary 可路由，必须落回 Python（deprecated 内核）。
        result = self._run_manage("mcp:status", "--json", env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("[engine] rust:", result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        self.assertEqual(payload["data"]["domain"], "mcp")

    @unittest.skipUnless(RUST_BIN.exists(), "engine-rs 未构建（cargo build）时跳过")
    def test_mcp_routes_to_rust_binary_via_env_override(self):
        result = self._run_manage(
            "mcp:status", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": LIVE_SECRET, "AGENT_MANAGER_ENGINE_RS": str(self.RUST_BIN)},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[engine] rust:", result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        elements = {e["id"]: e["state"] for e in payload["data"]["elements"]}
        self.assertEqual(elements["mcp:alpha"], "managed-ok")
        self.assertEqual(elements["mcp:zeta"], "drifted")

    def test_rust_route_env_override_accepts_custom_binary(self):
        # env 指定的自定义 binary 优先于仓库 target/（含假 binary 直通验证）。
        canned = {
            "contract": "agent-manager/engine/v1",
            "run_id": "routing-test",
            "command": "mcp.status",
            "mode": {"dry_run": False, "offline": False},
            "lock": {"held": False, "pid": None},
            "log_file": None,
            "ok": True,
            "data": {"domain": "mcp", "elements": []},
            "warnings": [],
            "errors": [],
        }
        fake_bin = self.temp_dir / "fake-engine.sh"
        fake_bin.write_text(
            "#!/bin/sh\ncat <<'JSON'\n%s\nJSON\n" % json.dumps(canned), encoding="utf-8"
        )
        fake_bin.chmod(0o755)
        result = self._run_manage(
            "mcp:status", "--json", env_extra={"AGENT_MANAGER_ENGINE_RS": str(fake_bin)}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[engine] rust:", result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["run_id"], "routing-test")

    @unittest.skipUnless(RUST_BIN.exists(), "engine-rs 未构建（cargo build）时跳过")
    def test_agents_routes_to_rust_binary_via_env_override(self):
        result = self._run_manage(
            "agents:status", "--json", env_extra={"AGENT_MANAGER_ENGINE_RS": str(self.RUST_BIN)},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[engine] rust:", result.stderr)
        payload = json.loads(result.stdout)
        self._assert_envelope(payload)
        self.assertEqual(payload["data"]["domain"], "agents")

    def test_agents_falls_back_to_python_when_no_rust_binary(self):
        result = self._run_manage("agents:status", "--json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("[engine] rust:", result.stderr)
        self.assertEqual(json.loads(result.stdout)["data"]["domain"], "agents")


class EngineDomainSliceTests(unittest.TestCase):
    """Slice C2a: status+plan for agents/tools/rules/hooks/aliases, secrets
    status (read-only), and system.audit. skills stays DOMAIN_NOT_IMPLEMENTED
    on purpose (parallel slice)."""

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_engine_c2a_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        for name in ("manager.local.json", "manager.synced.json"):
            (self.repo / "config" / name).unlink(missing_ok=True)
        self.bindir = self.temp_dir / "bin"
        self.bindir.mkdir()
        self.lock_path = self.temp_dir / "agent_manager.lock"
        self.log_prefix = self.temp_dir / "agent_manager_sync"
        self._write_minimal_configs()

    # ------------------------------------------------------------------ helpers

    def _write_minimal_configs(self):
        self._manager()
        self._write_config("agents.json", {})
        self._write_config("tools.json", {"tools": {}})
        self._write_config("mcp.json", {"servers": {}, "agent_paths": {}})
        self._write_config("skills.json", {"repositories": {}, "agents": {}})
        (self.repo / "config" / "rules-targets.list").write_text("", encoding="utf-8")

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _manager(self, **sections):
        base = {"agents": {}, "tools": {}, "skills": {}, "mcp": {}}
        base.update(sections)
        self._write_config("manager.json", base)

    def _make_bin(self, name, subdir=""):
        target_dir = self.bindir / subdir if subdir else self.bindir
        target_dir.mkdir(parents=True, exist_ok=True)
        path = target_dir / name
        path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        path.chmod(0o755)
        return path

    def _env(self, extra=None, restricted_path=False, prepend_path=False):
        env = os.environ.copy()
        env.pop("ENGINE_TEST_ALPHA_KEY", None)
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.lock_path)
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.log_prefix)
        if restricted_path:
            env["PATH"] = "%s:/usr/bin:/bin" % self.bindir
        elif prepend_path:
            # stub bin 优先于真实 PATH（探测 hermetic），但保留真实 python3 —
            # sync_tools.py 的探测源提取器在 /usr/bin/python3 (3.9) 下不可导入。
            env["PATH"] = "%s:%s" % (self.bindir, env.get("PATH", ""))
        if extra:
            env.update(extra)
        return env

    def _run(self, *argv, env_extra=None, restricted_path=False, prepend_path=False):
        return subprocess.run(
            [str(self.repo / "manage.sh"), *argv],
            cwd=self.repo,
            env=self._env(env_extra, restricted_path, prepend_path),
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )

    def _payload(self, *argv, expect_exit=0, **kw):
        result = self._run(*argv, **kw)
        self.assertEqual(result.returncode, expect_exit, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        errors = validate(ENVELOPE_SCHEMA, payload)
        self.assertEqual(errors, [], "envelope violations: %s" % [str(e) for e in errors])
        return payload

    def _assert_status_data(self, payload):
        errors = validate(STATUS_SCHEMA, payload["data"])
        self.assertEqual(errors, [], "status violations: %s" % [str(e) for e in errors])

    def _assert_plan_data(self, payload):
        errors = validate(PLAN_SCHEMA, payload["data"])
        self.assertEqual(errors, [], "plan violations: %s" % [str(e) for e in errors])

    @staticmethod
    def _by_id(payload):
        return {element["id"]: element for element in payload["data"]["elements"]}

    # ------------------------------------------------------------------ agents

    def _agents_fixture(self):
        self._write_config(
            "agents.json",
            {
                "fake-ok": {
                    "bin": "fake-ok-bin",
                    "providers": {
                        "npm": {
                            "managed": True,
                            "install": "npm install -g @fake/ok",
                            "detect_paths": [str(self.bindir / "fake-ok-bin")],
                        }
                    },
                },
                "fake-missing": {
                    "bin": "fake-missing-bin",
                    "providers": {"npm": {"managed": True, "install": "npm install -g @fake/missing"}},
                },
                "fake-disabled": {
                    "bin": "fake-disabled-bin",
                    "providers": {"npm": {"managed": True, "install": "npm install -g @fake/disabled"}},
                },
                "fake-leftover": {
                    "bin": "fake-leftover-bin",
                    "providers": {
                        "npm": {
                            "managed": True,
                            "install": "npm install -g @fake/leftover",
                            "detect_paths": [str(self.bindir / "fake-leftover-bin")],
                        }
                    },
                },
                "fake-undeclared": {
                    "bin": "fake-undeclared-bin",
                    "providers": {
                        "native": {"managed": False, "detect_paths": [str(self.bindir / "fake-undeclared-bin")]}
                    },
                },
                "fake-external": {
                    "bin": "fake-external-bin",
                    "require_managed_provider": True,
                    "providers": {"npm": {"managed": True, "install": "npm install -g @fake/ext"}},
                },
            },
        )
        self._manager(
            agents={
                "fake-ok": {"enabled": True},
                "fake-missing": {"enabled": True},
                "fake-disabled": {"enabled": False},
                "fake-leftover": {"enabled": False},
                "fake-external": {"enabled": True},
            }
        )
        for name in ("fake-ok-bin", "fake-leftover-bin", "fake-undeclared-bin", "fake-external-bin"):
            self._make_bin(name)

    def test_agents_status_maps_provider_states_and_validates_schema(self):
        self._agents_fixture()
        payload = self._payload("agents:status", "--json", restricted_path=True)
        self._assert_status_data(payload)
        elements = self._by_id(payload)

        self.assertEqual(elements["agents:fake-ok"]["state"], "managed-ok")
        self.assertEqual(elements["agents:fake-missing"]["state"], "missing")
        self.assertEqual(elements["agents:fake-disabled"]["state"], "disabled")
        # 禁用但仍安装 = 期望与物理偏离（对齐 doctor 的漂移语义）。
        self.assertEqual(elements["agents:fake-leftover"]["state"], "drifted")
        # 定义层存在但策略未声明（detect-only 形态）+ 已探测 → unmanaged-detected。
        self.assertEqual(elements["agents:fake-undeclared"]["state"], "unmanaged-detected")
        # 外部安装（无受管 provider 命中）+ require_managed_provider → unmanaged-detected。
        self.assertEqual(elements["agents:fake-external"]["state"], "unmanaged-detected")

        ok = elements["agents:fake-ok"]
        self.assertEqual(ok["observed"]["providers"], ["npm"])
        self.assertTrue(ok["observed"]["installed"])
        self.assertEqual([a["id"] for a in ok["actions"]], ["upgrade", "uninstall"])
        self.assertEqual([a["id"] for a in elements["agents:fake-missing"]["actions"]], ["install"])
        self.assertEqual(elements["agents:fake-disabled"]["actions"], [])
        self.assertEqual(ok["policy"]["preferred_provider"], "npm")
        self.assertTrue(ok["policy"]["declared_in_policy"])
        self.assertFalse(elements["agents:fake-undeclared"]["policy"]["declared_in_policy"])
        self.assertEqual(payload["errors"], [])

    def test_agents_status_deprecated_provider_maps_to_drifted(self):
        brew_stub = self._make_bin("brew")
        brew_stub.write_text(
            "#!/bin/sh\nif [ \"$2\" = \"--cask\" ]; then echo fake-dep; fi\nexit 0\n",
            encoding="utf-8",
        )
        self._write_config(
            "agents.json",
            {
                "fake-dep": {
                    "bin": "fake-dep-bin",
                    "providers": {"brew": {"managed": False, "install": "brew install --cask fake-dep"}},
                }
            },
        )
        self._manager(agents={"fake-dep": {"enabled": True}})
        payload = self._payload("agents:status", "--json", restricted_path=True)
        self._assert_status_data(payload)
        element = self._by_id(payload)["agents:fake-dep"]
        self.assertEqual(element["state"], "drifted")
        self.assertEqual(element["observed"]["classified"]["deprecated"], ["brew-cask"])

    def test_agents_status_unsupported_provider_is_explicit_error(self):
        # 反例：探测到定义层未声明的来源不得静默（提案 §3.1 unsupported）。
        self._make_bin("fake-unsupported-bin", subdir="node_modules/.bin")
        self._write_config(
            "agents.json",
            {
                "fake-unsupported": {
                    "bin": "fake-unsupported-bin",
                    "providers": {"native": {"managed": True, "install": "true"}},
                }
            },
        )
        self._manager(agents={"fake-unsupported": {"enabled": True}})
        npm_style_path = "%s:/usr/bin:/bin" % (self.bindir / "node_modules" / ".bin")
        payload = self._payload(
            "agents:status", "--json", expect_exit=1, restricted_path=True,
            env_extra={"PATH": npm_style_path},
        )
        self._assert_status_data(payload)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "AGENT_PROVIDER_UNSUPPORTED")
        self.assertEqual(self._by_id(payload)["agents:fake-unsupported"]["state"], "unsupported")

    def test_agents_plan_installs_missing_and_respects_auto_install_gate(self):
        self._write_config(
            "agents.json",
            {
                "fake-missing": {
                    "bin": "fake-missing-bin",
                    "providers": {"npm": {"managed": True, "install": "npm install -g @fake/missing"}},
                }
            },
        )
        self._manager(agents={"fake-missing": {"enabled": True}})
        payload = self._payload("agents:plan", "--json", restricted_path=True)
        self._assert_plan_data(payload)
        self.assertEqual(len(payload["data"]["actions"]), 1)
        action = payload["data"]["actions"][0]
        self.assertEqual(action["op"], "install")
        self.assertEqual(action["element"], "agents:fake-missing")
        self.assertEqual(action["cmd"], "npm install -g @fake/missing")
        self.assertEqual(action["risk"], "install-remove")
        self.assertIsNone(action["target"])
        self.assertEqual(payload["data"]["summary"]["install-remove"], 1)

        # 反例：auto_install=false 时 plan 不生成安装动作，只给提示。
        self._manager(agents={"fake-missing": {"enabled": True, "auto_install": False}})
        payload = self._payload("agents:plan", "--json", restricted_path=True)
        self._assert_plan_data(payload)
        self.assertEqual(payload["data"]["actions"], [])
        self.assertEqual(payload["warnings"][0]["code"], "AGENT_INSTALL_DISABLED_BY_POLICY")

    # ------------------------------------------------------------------ tools

    def _tools_fixture(self):
        self._write_config(
            "tools.json",
            {
                "tools": {
                    "fake-tool": {"install": "npm install -g @fake/tool", "ready_check": "bin:fake-tool-bin"},
                    "fake-optional": {
                        "install": "npm install -g @fake/opt",
                        "ready_check": "bin:fake-optional-bin",
                        "doctor_severity": "warning",
                    },
                    "fake-off": {"install": "npm install -g @fake/off", "ready_check": "bin:fake-off-bin"},
                }
            },
        )
        self._manager(
            tools={
                "fake-tool": {"enabled": True},
                "fake-optional": {"enabled": True},
                "fake-off": {"enabled": False},
            }
        )
        self._make_bin("fake-tool-bin")

    def test_tools_status_readiness_mapping_and_optional_warning(self):
        self._tools_fixture()
        payload = self._payload("tools:status", "--json", restricted_path=True)
        self._assert_status_data(payload)
        elements = self._by_id(payload)
        self.assertEqual(elements["tools:fake-tool"]["state"], "managed-ok")
        self.assertEqual(elements["tools:fake-optional"]["state"], "missing")
        self.assertEqual(elements["tools:fake-off"]["state"], "disabled")
        # 未就绪是元素数据不是运行错误；可选工具另配提示级 warning（doctor 惯例）。
        self.assertEqual(payload["errors"], [])
        self.assertEqual([w["code"] for w in payload["warnings"]], ["TOOL_OPTIONAL_NOT_READY"])
        self.assertEqual(payload["warnings"][0]["element"], "tools:fake-optional")
        self.assertEqual([a["id"] for a in elements["tools:fake-tool"]["actions"]], ["upgrade"])
        self.assertEqual([a["id"] for a in elements["tools:fake-optional"]["actions"]], ["install"])

    def test_tools_status_policy_without_definition_is_error(self):
        # 反例：策略启用但定义层缺失必须显式报错。
        self._manager(tools={"ghost": {"enabled": True}})
        payload = self._payload("tools:status", "--json", expect_exit=1, restricted_path=True)
        self._assert_status_data(payload)
        self.assertEqual(payload["errors"][0]["code"], "TOOL_DEFINITION_MISSING")
        self.assertEqual(self._by_id(payload)["tools:ghost"]["state"], "unsupported")

    def test_tools_plan_covers_bin_and_repo_tools(self):
        self._tools_fixture()
        defs = json.loads((self.repo / "config" / "tools.json").read_text(encoding="utf-8"))
        defs["tools"]["fake-repo"] = {"url": "https://example.invalid/fake-repo.git", "setup": "make build"}
        self._write_config("tools.json", defs)
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["tools"]["fake-repo"] = {"enabled": True}
        self._write_config("manager.json", manager)

        payload = self._payload("tools:plan", "--json", restricted_path=True)
        self._assert_plan_data(payload)
        by_element = {a["element"]: a for a in payload["data"]["actions"]}
        self.assertEqual(set(by_element), {"tools:fake-optional", "tools:fake-repo"})
        self.assertEqual(by_element["tools:fake-optional"]["cmd"], "npm install -g @fake/opt")
        self.assertEqual(by_element["tools:fake-repo"]["cmd"], "git clone https://example.invalid/fake-repo.git")
        self.assertIn("make build", by_element["tools:fake-repo"]["note"])
        self.assertEqual(payload["data"]["summary"]["install-remove"], 2)

    # ------------------------------------------------------------------ rules

    def _rules_fixture(self):
        (self.repo / "config" / "rules-targets.list").write_text(
            "claude_global|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/CLAUDE.md\n"
            "claude_drift|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/DRIFT.md\n"
            "claude_missing|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/MISSING.md\n"
            "codex_global|global|rules/dist/codex/AGENTS.md|{{HOME}}/.codex/AGENTS.md\n",
            encoding="utf-8",
        )
        self._manager(agents={"claude-code": {"enabled": True}, "codex": {"enabled": False}})
        dist_src = self.repo / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md"
        ok_target = self.home / ".claude" / "CLAUDE.md"
        ok_target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(dist_src, ok_target)
        (self.home / ".claude" / "DRIFT.md").write_text("drifted content\n", encoding="utf-8")

    def test_rules_status_convergence_and_scope(self):
        self._rules_fixture()
        payload = self._payload("rules:status", "--json")
        self._assert_status_data(payload)
        elements = self._by_id(payload)
        self.assertEqual(elements["rules:claude_global"]["state"], "managed-ok")
        self.assertEqual(elements["rules:claude_drift"]["state"], "drifted")
        self.assertEqual(elements["rules:claude_missing"]["state"], "not-materialized")
        # codex 在策略层禁用 → 目标出作用域。
        self.assertEqual(elements["rules:codex_global"]["state"], "disabled")
        self.assertEqual(elements["rules:codex_global"]["actions"], [])
        self.assertEqual(elements["rules:dist"]["state"], "managed-ok")
        self.assertEqual(payload["errors"], [])

    def test_rules_status_flags_stale_dist_build(self):
        self._rules_fixture()
        src_core = self.repo / "agent-rule" / "rules" / "src" / "shared" / "core.md"
        future = os.path.getmtime(self.repo / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md") + 100
        os.utime(src_core, (future, future))
        payload = self._payload("rules:status", "--json")
        self._assert_status_data(payload)
        elements = self._by_id(payload)
        self.assertEqual(elements["rules:dist"]["state"], "drifted")
        self.assertIn("rules/dist/claude/CLAUDE.md", elements["rules:dist"]["observed"]["stale_products"])
        self.assertTrue(elements["rules:claude_global"]["observed"]["build_stale"])
        self.assertIn("RULE_BUILD_STALE", [w["code"] for w in payload["warnings"]])

    def test_rules_plan_orders_build_before_distribution(self):
        self._rules_fixture()
        src_core = self.repo / "agent-rule" / "rules" / "src" / "shared" / "core.md"
        future = os.path.getmtime(self.repo / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md") + 100
        os.utime(src_core, (future, future))
        payload = self._payload("rules:plan", "--json")
        self._assert_plan_data(payload)
        actions = payload["data"]["actions"]
        self.assertEqual(actions[0]["op"], "run-cmd")
        self.assertEqual(actions[0]["element"], "rules:dist")
        self.assertEqual(actions[0]["risk"], "exec-local")
        self.assertIn("build-agent-rules.py", actions[0]["cmd"])

        by_element = {a["element"]: a for a in actions}
        drift = by_element["rules:claude_drift"]
        self.assertEqual(drift["op"], "write-file")
        self.assertEqual(drift["risk"], "write-local")
        self.assertTrue(drift["before_digest"].startswith("sha256:"))
        self.assertTrue(drift["after_digest"].startswith("sha256:"))
        self.assertNotEqual(drift["before_digest"], drift["after_digest"])
        missing = by_element["rules:claude_missing"]
        self.assertIsNone(missing["before_digest"])
        self.assertIsNotNone(missing["after_digest"])
        # status/plan 一致性：需要收敛的元素都在 plan 里，disabled 不在。
        self.assertNotIn("rules:codex_global", by_element)
        self.assertNotIn("rules:claude_global", by_element)

    def test_rules_manifest_bad_line_is_explicit_error(self):
        # 反例：manifest 非法行不得静默跳过。
        (self.repo / "config" / "rules-targets.list").write_text("broken-line-without-columns\n", encoding="utf-8")
        payload = self._payload("rules:status", "--json", expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "RULES_MANIFEST_INVALID")

    # ------------------------------------------------------------------ hooks

    def _hooks_fixture(self):
        # 沙盒拷贝自带仓库真实的 .claude/settings.json 与 .codex/hooks.json；
        # 删掉以获得可控的“未物化”基线。
        (self.repo / ".claude" / "settings.json").unlink(missing_ok=True)
        (self.repo / ".codex" / "hooks.json").unlink(missing_ok=True)
        self._write_config(
            "claude-hooks.json",
            {
                "settings_path": "../.claude/settings.json",
                "hooks": {
                    "SessionStart": [{"hooks": [{"type": "command", "command": "echo engine-test-claude-hook"}]}]
                },
            },
        )
        self._write_config(
            "codex-hooks.json",
            {
                "hooks_path": "../.codex/hooks.json",
                "hooks": {
                    "PreToolUse": [
                        {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo engine-test-codex-hook"}]}
                    ]
                },
            },
        )

    def _write_claude_settings(self, hooks):
        settings = self.repo / ".claude" / "settings.json"
        settings.parent.mkdir(parents=True, exist_ok=True)
        settings.write_text(json.dumps({"other": True, "hooks": hooks}, indent=2) + "\n", encoding="utf-8")

    def _write_codex_hooks(self, hooks):
        target = self.repo / ".codex" / "hooks.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps({"hooks": hooks}, indent=2) + "\n", encoding="utf-8")

    def test_hooks_status_signature_merge_and_whole_file_semantics(self):
        self._hooks_fixture()
        payload = self._payload("hooks:status", "--json")
        self._assert_status_data(payload)
        elements = self._by_id(payload)
        self.assertEqual(elements["hooks:claude"]["state"], "not-materialized")
        self.assertEqual(elements["hooks:codex"]["state"], "not-materialized")

        # 托管签名存在 + 区块外用户键保留 → managed-ok。
        self._write_claude_settings(
            {"SessionStart": [{"hooks": [{"type": "command", "command": "echo engine-test-claude-hook"}]}]}
        )
        self._write_codex_hooks(
            {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo engine-test-codex-hook"}]}]}
        )
        payload = self._payload("hooks:status", "--json")
        elements = self._by_id(payload)
        self.assertEqual(elements["hooks:claude"]["state"], "managed-ok")
        self.assertEqual(elements["hooks:codex"]["state"], "managed-ok")

        # 托管签名缺失 → drifted；codex 任意多余键 → drifted（整体重写语义）。
        self._write_claude_settings({"SessionStart": []})
        self._write_codex_hooks({})
        payload = self._payload("hooks:status", "--json")
        elements = self._by_id(payload)
        self.assertEqual(elements["hooks:claude"]["state"], "drifted")
        self.assertEqual(elements["hooks:claude"]["observed"]["missing_signatures"], 1)
        self.assertEqual(elements["hooks:codex"]["state"], "drifted")

    def test_hooks_plan_renders_expected_content_digests(self):
        self._hooks_fixture()
        payload = self._payload("hooks:plan", "--json")
        self._assert_plan_data(payload)
        by_element = {a["element"]: a for a in payload["data"]["actions"]}
        self.assertEqual(set(by_element), {"hooks:claude", "hooks:codex"})
        for action in by_element.values():
            self.assertEqual(action["op"], "write-file")
            self.assertEqual(action["risk"], "write-local")
            self.assertIsNone(action["before_digest"])
            self.assertTrue(action["after_digest"].startswith("sha256:"))
        # 用真实 writer 落地后 status 必须回到 managed-ok（渲染语义与引擎一致）。
        for script, flag, target in (
            ("sync-claude-hooks.py", "--settings-path", self.repo / ".claude" / "settings.json"),
            ("sync-codex-hooks.py", "--hooks-path", self.repo / ".codex" / "hooks.json"),
        ):
            subprocess.run(
                [
                    "python3",
                    str(self.repo / "agent-rule" / "scripts" / script),
                    "--config",
                    str(self.repo / "config" / script.replace("sync-", "").replace(".py", ".json")),
                    flag,
                    str(target),
                ],
                check=True,
                capture_output=True,
            )
        payload = self._payload("hooks:status", "--json")
        elements = self._by_id(payload)
        self.assertEqual(elements["hooks:claude"]["state"], "managed-ok")
        self.assertEqual(elements["hooks:codex"]["state"], "managed-ok")
        # 收敛后 plan 为空。
        payload = self._payload("hooks:plan", "--json")
        self.assertEqual(payload["data"]["actions"], [])

    def test_hooks_target_parse_error_is_explicit(self):
        # 反例：目标文件坏 JSON 不得当成“未注入”。
        self._hooks_fixture()
        broken = self.repo / ".codex" / "hooks.json"
        broken.parent.mkdir(parents=True, exist_ok=True)
        broken.write_text("{ invalid\n", encoding="utf-8")
        payload = self._payload("hooks:status", "--json", expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "HOOKS_TARGET_PARSE_ERROR")
        self.assertEqual(payload["errors"][0]["element"], "hooks:codex")
        self.assertEqual(self._by_id(payload)["hooks:codex"]["state"], "drifted")

    # ------------------------------------------------------------------ aliases

    ALIAS_BLOCK_BEGIN = "# >>> agent-manager shell aliases >>>"
    ALIAS_BLOCK_END = "# <<< agent-manager shell aliases <<<"

    def _aliases_fixture(self):
        self._write_config(
            "shell-aliases.json",
            {"timezone": "UTC", "lang": "C", "aliases": {"ll": "ls -l", "gs": "git status"}},
        )

    def _expected_alias_block(self):
        lines = ["alias gs='git status'", "alias ll='ls -l'"]  # sorted by name
        return self.ALIAS_BLOCK_BEGIN + "\n" + "\n".join(lines) + "\n" + self.ALIAS_BLOCK_END

    def test_aliases_status_tracks_managed_block_per_file(self):
        self._aliases_fixture()
        zshrc = self.home / ".zshrc"
        zshrc.write_text("# user content\n\n" + self._expected_alias_block() + "\n", encoding="utf-8")
        payload = self._payload("aliases:status", "--json")
        self._assert_status_data(payload)
        elements = self._by_id(payload)
        self.assertEqual(elements["aliases:zshrc"]["state"], "managed-ok")
        self.assertEqual(elements["aliases:zshrc"]["observed"]["alias_count"], 2)
        self.assertEqual(elements["aliases:zprofile"]["state"], "not-materialized")

        (self.home / ".zprofile").write_text(
            self.ALIAS_BLOCK_BEGIN + "\nalias stale='old'\n" + self.ALIAS_BLOCK_END + "\n",
            encoding="utf-8",
        )
        payload = self._payload("aliases:status", "--json")
        self.assertEqual(self._by_id(payload)["aliases:zprofile"]["state"], "drifted")

    def test_aliases_plan_uses_block_level_digests(self):
        self._aliases_fixture()
        (self.home / ".zprofile").write_text(
            self.ALIAS_BLOCK_BEGIN + "\nalias stale='old'\n" + self.ALIAS_BLOCK_END + "\n",
            encoding="utf-8",
        )
        payload = self._payload("aliases:plan", "--json")
        self._assert_plan_data(payload)
        by_element = {a["element"]: a for a in payload["data"]["actions"]}
        # zshrc 未物化（新建注入）与 zprofile 漂移（改写区块）都应收敛。
        self.assertEqual(set(by_element), {"aliases:zshrc", "aliases:zprofile"})
        action = by_element["aliases:zprofile"]
        self.assertEqual(action["op"], "write-file")
        self.assertEqual(action["risk"], "write-local")
        self.assertTrue(action["target"].endswith(".zprofile"))
        self.assertNotEqual(action["before_digest"], action["after_digest"])
        self.assertIn("区块外", action["note"])
        self.assertIsNone(by_element["aliases:zshrc"]["before_digest"])
        # 反方向：alias 列表清空后已注入区块应被移除（after_digest=null）。
        self._write_config("shell-aliases.json", {"aliases": {}})
        payload = self._payload("aliases:plan", "--json")
        by_element = {a["element"]: a for a in payload["data"]["actions"]}
        self.assertIsNone(by_element["aliases:zprofile"]["after_digest"])

    # ------------------------------------------------------------------ secrets

    def test_secrets_status_is_read_only_and_never_leaks_values(self):
        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {
                        "command": "alpha-cmd",
                        "args": [],
                        "env": {"ALPHA_KEY": "${ENGINE_TEST_ALPHA_KEY}"},
                        "agents": [],
                    }
                },
                "agent_paths": {},
            },
        )
        self._manager(mcp={"alpha": {"enabled": True}})

        payload = self._payload("secrets:status", "--json")
        self._assert_status_data(payload)
        elements = self._by_id(payload)
        self.assertEqual(elements["secrets:repo"]["state"], "not-materialized")
        self.assertEqual(elements["secrets:mcp-env"]["state"], "missing")
        self.assertEqual([w["code"] for w in payload["warnings"]], ["MCP_ENV_UNRESOLVED"])
        variable = elements["secrets:mcp-env"]["observed"]["variables"][0]
        self.assertEqual(variable["placeholder"], "ENGINE_TEST_ALPHA_KEY")
        self.assertFalse(variable["resolved"])

        # 假值只经 secrets/mcp.env 提供；输出只允许变量名 + resolved。
        secrets_dir = self.repo / "secrets"
        (secrets_dir / ".git").mkdir(parents=True)
        (secrets_dir / "mcp.env").write_text("ENGINE_TEST_ALPHA_KEY=%s\n" % LIVE_SECRET, encoding="utf-8")
        result = self._run("secrets:status", "--json")
        self.assertEqual(result.returncode, 0, result.stderr)
        combined = result.stdout + result.stderr
        self.assertNotIn(LIVE_SECRET, combined)
        payload = json.loads(result.stdout)
        elements = self._by_id(payload)
        self.assertEqual(elements["secrets:repo"]["state"], "managed-ok")
        self.assertEqual(elements["secrets:mcp-env"]["state"], "managed-ok")
        self.assertTrue(elements["secrets:mcp-env"]["observed"]["variables"][0]["resolved"])

    # ------------------------------------------------------------------ system.audit

    def test_system_audit_wraps_doctor_report(self):
        payload = self._payload("system:audit", "--json")
        self.assertEqual(payload["command"], "system.audit")
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["errors"], [])
        report = payload["data"]
        self.assertEqual(report["status"], "healthy")
        for section in ("config", "security", "runtime", "storage", "agents", "tools", "mcp", "skills", "rules", "backups"):
            self.assertIn(section, report["sections"])
        self.assertEqual(report["summary"]["errors"], 0)

    def test_system_audit_error_status_maps_to_envelope_errors(self):
        (self.repo / "config" / "agents.json").write_text("{ invalid json\n", encoding="utf-8")
        payload = self._payload("system:audit", "--json", expect_exit=1)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "AUDIT_ERRORS")
        self.assertEqual(payload["data"]["status"], "error")

    def test_system_audit_warning_categories_expand_to_envelope_warnings(self):
        self._write_config(
            "tools.json",
            {"tools": {"fake-opt": {"install": "true", "ready_check": "bin:definitely-missing-tool-xyz", "doctor_severity": "warning"}}},
        )
        self._manager(tools={"fake-opt": {"enabled": True}})
        payload = self._payload("system:audit", "--json")
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["data"]["status"], "warning")
        self.assertEqual([w["code"] for w in payload["warnings"]], ["OPTIONAL_TOOLS"])

    # ------------------------------------------------- upstream freshness (Slice D)

    def _freshness_fixture(self, npm_view_version="2.0.0"):
        """One npm-probed agent + recording stubs for npm/git (zero-network proof)."""
        self._make_bin("fake-agent-bin", ).write_text("#!/bin/sh\necho 1.0.0\n", encoding="utf-8")
        calls = self.temp_dir / "probe-calls.log"
        self._make_bin("npm").write_text(
            "#!/bin/sh\necho \"npm $*\" >> %s\nif [ \"$1\" = \"view\" ]; then echo %s; fi\nexit 0\n"
            % (calls, npm_view_version),
            encoding="utf-8",
        )
        self._make_bin("git").write_text(
            "#!/bin/sh\necho \"git $*\" >> %s\nexit 0\n" % calls,
            encoding="utf-8",
        )
        self._write_config(
            "agents.json",
            {
                "fake-agent": {
                    "bin": "fake-agent-bin",
                    "providers": {"npm": {"managed": True, "install": "npm install -g @fake/agent"}},
                }
            },
        )
        self._manager(agents={"fake-agent": {"enabled": True}})
        cache_file = self.temp_dir / "upstream-cache.json"
        env_extra = {
            "AGENT_MANAGER_UPSTREAM_CACHE": str(cache_file),
            "ENGINE_TEST_CALLS": str(calls),
        }
        return cache_file, calls, env_extra

    def test_refresh_writes_cache_and_status_fills_freshness(self):
        cache_file, calls, env_extra = self._freshness_fixture()
        result = self._run("system:refresh-upstream", "--json", env_extra=env_extra, prepend_path=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        errors = validate(ENVELOPE_SCHEMA, payload)
        self.assertEqual(errors, [], [str(e) for e in errors])
        # 写者路径持锁（refresh 不在锁豁免清单）。
        self.assertEqual(payload["lock"]["held"], True)
        self.assertIsInstance(payload["lock"]["pid"], int)
        self.assertEqual(payload["data"]["summary"], {"probed": 1, "current": 0, "behind": 1, "ahead": 0, "unknown": 0})

        cache = json.loads(cache_file.read_text(encoding="utf-8"))
        entry = cache["entries"]["agents:fake-agent"]
        self.assertEqual(entry["state"], "behind")
        self.assertEqual(entry["current"], "1.0.0")
        self.assertEqual(entry["upstream"], "2.0.0")

        # status 读缓存填充 freshness，且自身零网络（探测 stub 不再被调用）。
        calls.unlink()
        payload = self._payload("agents:status", "--json", env_extra=env_extra, prepend_path=True)
        self._assert_status_data(payload)
        freshness = self._by_id(payload)["agents:fake-agent"]["freshness"]
        self.assertEqual(freshness["state"], "behind")
        self.assertEqual(freshness["current"], "1.0.0")
        self.assertEqual(freshness["upstream"], "2.0.0")
        self.assertIsNotNone(freshness["checked_at"])
        self.assertIsInstance(freshness["cache_age_sec"], int)
        self.assertFalse(calls.exists(), "status 不得触发任何探测命令（零网络）")

    def test_status_without_cache_is_unknown_and_never_probes(self):
        cache_file, calls, env_extra = self._freshness_fixture()
        payload = self._payload("agents:status", "--json", env_extra=env_extra, prepend_path=True)
        freshness = self._by_id(payload)["agents:fake-agent"]["freshness"]
        self.assertEqual(freshness["state"], "unknown")
        self.assertIsNone(freshness["checked_at"])
        self.assertIsNone(freshness["upstream"])
        # 本地 current 仍可观测（bin --version 是本地只读探测）。
        self.assertEqual(freshness["current"], "1.0.0")
        self.assertFalse(cache_file.exists())
        self.assertFalse(calls.exists(), "status 不得触发任何探测命令（零网络）")

    def test_refresh_offline_is_denied_and_writes_nothing(self):
        cache_file, calls, env_extra = self._freshness_fixture()
        result = self._run(
            "--offline", "system:refresh-upstream", "--json", env_extra=env_extra, prepend_path=True
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        errors = validate(ENVELOPE_SCHEMA, payload)
        self.assertEqual(errors, [], [str(e) for e in errors])
        self.assertEqual(payload["errors"][0]["code"], "OFFLINE_NETWORK_DENIED")
        self.assertFalse(cache_file.exists(), "offline 拒绝时不得写缓存")
        self.assertFalse(calls.exists(), "offline 拒绝时不得执行任何探测")

    def test_refresh_only_single_element_and_unknown_element_rejected(self):
        cache_file, calls, env_extra = self._freshness_fixture()
        defs = json.loads((self.repo / "config" / "agents.json").read_text(encoding="utf-8"))
        defs["fake-agent-2"] = {
            "bin": "fake-agent-2-bin",
            "providers": {"npm": {"managed": True, "install": "npm install -g @fake/agent2"}},
        }
        self._write_config("agents.json", defs)
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["agents"]["fake-agent-2"] = {"enabled": True}
        self._write_config("manager.json", manager)

        payload = self._payload(
            "system:refresh-upstream", "--json", "--only", "agents:fake-agent-2",
            env_extra=env_extra, prepend_path=True,
        )
        self.assertEqual([e["id"] for e in payload["data"]["elements"]], ["agents:fake-agent-2"])
        cache = json.loads(cache_file.read_text(encoding="utf-8"))
        self.assertEqual(sorted(cache["entries"]), ["agents:fake-agent-2"])

        result = self._run(
            "system:refresh-upstream", "--json", "--only", "agents:bogus",
            env_extra=env_extra, prepend_path=True,
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["errors"][0]["code"], "USAGE_ERROR")

    def test_status_stale_cache_shows_last_known_with_warning(self):
        cache_file, calls, env_extra = self._freshness_fixture()
        self._run("system:refresh-upstream", "--json", env_extra=env_extra, prepend_path=True)
        calls.unlink()  # refresh 的探测记录清零；随后 status 不得再产生任何探测
        cache = json.loads(cache_file.read_text(encoding="utf-8"))
        cache["entries"]["agents:fake-agent"]["checked_at"] = "2000-01-01T00:00:00Z"
        cache_file.write_text(json.dumps(cache), encoding="utf-8")

        payload = self._payload("agents:status", "--json", env_extra=env_extra, prepend_path=True)
        freshness = self._by_id(payload)["agents:fake-agent"]["freshness"]
        self.assertEqual(freshness["state"], "behind", "超 TTL 仍展示 last-known")
        self.assertGreater(freshness["cache_age_sec"], 365 * 24 * 3600)
        stale = [w for w in payload["warnings"] if w["code"] == "UPSTREAM_PROBE_STALE"]
        self.assertEqual([w["element"] for w in stale], ["agents:fake-agent"])
        self.assertFalse(calls.exists(), "status 不得因缓存 stale 而重新探测")

    # ------------------------------------------------------------------ skills

    def _make_skill_repo(self, repo_name, skill_names):
        repo = self.repo / "agent-skill" / "repositories" / repo_name
        for name in skill_names:
            skill_dir = repo / name
            skill_dir.mkdir(parents=True, exist_ok=True)
            (skill_dir / "SKILL.md").write_text(
                "---\nname: %s\ndescription: engine test\n---\n" % name, encoding="utf-8"
            )
        (repo / ".agent-manager-owned").write_text("{}\n", encoding="utf-8")
        return repo

    def _skills_fixture(self):
        """矩阵集合 coll（skill-a 双 agent、skill-b 仅 claude、skill-c 收件箱）
        + 旧式集合 legacy（skill-x 升格等价=范围内启用）。"""
        coll = self._make_skill_repo("coll", ("skill-a", "skill-b", "skill-c"))
        legacy = self._make_skill_repo("legacy", ("skill-x",))
        claude_skills = self.home / ".claude" / "skills"
        codex_skills = self.home / ".codex" / "skills"
        claude_skills.mkdir(parents=True)
        codex_skills.mkdir(parents=True)

        self._write_config(
            "skills.json",
            {
                "repositories": {
                    "coll": {"url": "https://example.invalid/coll.git", "agents": ["claude-code", "codex"]},
                    "legacy": {"url": "https://example.invalid/legacy.git", "agents": ["claude-code"]},
                },
                "agents": {"claude-code": str(claude_skills), "codex": str(codex_skills)},
            },
        )
        self._manager(
            skills={
                "coll": {
                    "enabled": True,
                    "skills": {
                        "skill-a": {"agents": {"claude-code": True, "codex": True}},
                        "skill-b": {"agents": {"claude-code": True, "codex": False}},
                        "skill-c": {},
                    },
                },
                "legacy": {"enabled": True},
            }
        )
        return coll, legacy, claude_skills, codex_skills

    def test_skills_status_matrix_cells_and_element_aggregation(self):
        coll, _legacy, claude_skills, codex_skills = self._skills_fixture()
        os.symlink(str(coll / "skill-a"), str(claude_skills / "skill-a"))  # linked
        os.symlink(str(coll / "skill-gone"), str(codex_skills / "skill-a"))  # dead-link
        os.symlink(str(coll / "skill-a"), str(claude_skills / "skill-z"))  # 额外受管链接

        payload = self._payload("skills:status", "--json")
        self._assert_status_data(payload)
        elements = self._by_id(payload)

        coll_el = elements["skill:coll"]
        self.assertEqual(coll_el["state"], "drifted", "额外受管链接应让集合偏离")
        self.assertEqual(coll_el["spec"]["matrix_mode"], True)
        self.assertEqual(coll_el["observed"]["discovered"], ["skill-a", "skill-b", "skill-c"])
        self.assertEqual(coll_el["observed"]["discovery_inbox"], ["skill-c"])
        self.assertEqual(coll_el["observed"]["extra_links"][0]["name"], "skill-z")
        self.assertTrue(coll_el["observed"]["owner_marked"])
        self.assertEqual(coll_el["freshness"]["state"], "unknown", "无缓存条目 → unknown")

        skill_a = elements["skill:coll/skill-a"]
        self.assertEqual(skill_a["observed"]["agents"]["claude-code"]["state"], "linked")
        self.assertEqual(skill_a["observed"]["agents"]["codex"]["state"], "dead-link")
        self.assertEqual(skill_a["state"], "drifted")

        skill_b = elements["skill:coll/skill-b"]
        self.assertEqual(skill_b["observed"]["agents"]["claude-code"]["state"], "missing-link")
        self.assertEqual(skill_b["observed"]["agents"]["codex"]["state"], "disabled")
        self.assertEqual(skill_b["state"], "missing")
        self.assertEqual(skill_b["policy"]["agents"], {"claude-code": True, "codex": False})

        skill_c = elements["skill:coll/skill-c"]
        self.assertEqual(skill_c["state"], "disabled")
        self.assertTrue(skill_c["observed"]["discovery_inbox"])
        self.assertEqual(
            skill_c["policy"]["agents"], {"claude-code": False, "codex": False}
        )

        legacy_el = elements["skill:legacy"]
        self.assertEqual(legacy_el["state"], "managed-ok")
        self.assertEqual(legacy_el["spec"]["matrix_mode"], False)
        skill_x = elements["skill:legacy/skill-x"]
        # 旧式定义升格：范围内 agent 默认启用。
        self.assertEqual(skill_x["policy"]["agents"], {"claude-code": True})
        self.assertEqual(skill_x["observed"]["agents"]["claude-code"]["state"], "missing-link")
        self.assertEqual(skill_x["state"], "missing")
        self.assertEqual(payload["errors"], [])

    def test_skills_status_unmaterialized_when_target_dirs_absent(self):
        self._make_skill_repo("fresh", ("skill-y",))
        self._write_config(
            "skills.json",
            {
                "repositories": {"fresh": {"url": "https://example.invalid/fresh.git", "agents": ["claude-code"]}},
                "agents": {"claude-code": str(self.home / ".claude" / "skills")},
            },
        )
        self._manager(skills={"fresh": {"enabled": True}})
        payload = self._payload("skills:status", "--json")
        self._assert_status_data(payload)
        skill_y = self._by_id(payload)["skill:fresh/skill-y"]
        self.assertEqual(skill_y["state"], "not-materialized")
        self.assertEqual(skill_y["observed"]["agents"]["claude-code"]["state"], "missing-link")

    def test_skills_status_invalid_matrix_fails_closed(self):
        # 反例：矩阵引用未发现的 skill → fail-close（对齐 v2 校验语义）。
        self._make_skill_repo("coll", ("skill-a",))
        (self.home / ".claude" / "skills").mkdir(parents=True)
        self._write_config(
            "skills.json",
            {
                "repositories": {"coll": {"url": "https://example.invalid/coll.git", "agents": ["claude-code"]}},
                "agents": {"claude-code": str(self.home / ".claude" / "skills")},
            },
        )
        self._manager(
            skills={"coll": {"enabled": True, "skills": {"ghost-skill": {"agents": {"claude-code": True}}}}}
        )
        payload = self._payload("skills:status", "--json", expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "SKILL_CONFIG_INVALID")
        self.assertEqual(payload["errors"][0]["element"], "skill:coll")
        self.assertEqual(self._by_id(payload)["skill:coll"]["state"], "unsupported")

    def test_skills_status_missing_repo_cache_is_not_materialized(self):
        self._write_config(
            "skills.json",
            {
                "repositories": {"gone": {"url": "https://example.invalid/gone.git", "agents": ["claude-code"]}},
                "agents": {"claude-code": str(self.home / ".claude" / "skills")},
            },
        )
        self._manager(skills={"gone": {"enabled": True}})
        payload = self._payload("skills:status", "--json")
        self._assert_status_data(payload)
        gone = self._by_id(payload)["skill:gone"]
        self.assertEqual(gone["state"], "not-materialized")
        self.assertFalse(gone["observed"]["repo_cached"])
        self.assertEqual([w["code"] for w in payload["warnings"]], ["SKILL_REPO_CACHE_MISSING"])

    def test_skills_status_cross_collection_duplicate_name_is_error(self):
        # 反例：两个集合发现同名 skill 是 spec 层冲突，不得静默（doctor 同语义）。
        self._make_skill_repo("coll-a", ("dup-skill",))
        self._make_skill_repo("coll-b", ("dup-skill",))
        skills_dir = self.home / ".claude" / "skills"
        skills_dir.mkdir(parents=True)
        self._write_config(
            "skills.json",
            {
                "repositories": {
                    "coll-a": {"url": "https://example.invalid/a.git", "agents": ["claude-code"]},
                    "coll-b": {"url": "https://example.invalid/b.git", "agents": ["claude-code"]},
                },
                "agents": {"claude-code": str(skills_dir)},
            },
        )
        self._manager(skills={"coll-a": {"enabled": True}, "coll-b": {"enabled": True}})
        payload = self._payload("skills:status", "--json", expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "SKILL_DUPLICATE_NAME")
        self.assertEqual(payload["errors"][0]["element"], "skill:coll-b/dup-skill")

    def test_skills_plan_translates_matrix_actions(self):
        coll, _legacy, claude_skills, codex_skills = self._skills_fixture()
        os.symlink(str(coll / "skill-a"), str(claude_skills / "skill-a"))
        os.symlink(str(coll / "skill-gone"), str(codex_skills / "skill-a"))
        os.symlink(str(coll / "skill-a"), str(claude_skills / "skill-z"))

        payload = self._payload("skills:plan", "--json")
        self._assert_plan_data(payload)
        actions = payload["data"]["actions"]
        by_note_target = {(a["element"], a["op"], a["target"]): a for a in actions}

        # 死链 → unlink；启用缺链 → symlink；额外链接 → unlink（挂集合）；收件箱 → 无动作。
        dead = [a for a in actions if a["element"] == "skill:coll/skill-a" and a["op"] == "unlink"]
        self.assertEqual(len(dead), 1)
        self.assertEqual(dead[0]["risk"], "exec-local")
        self.assertIn("死链", dead[0]["note"])
        link_b = [a for a in actions if a["element"] == "skill:coll/skill-b"]
        self.assertEqual(len(link_b), 1)
        self.assertEqual(link_b[0]["op"], "symlink")
        self.assertTrue(link_b[0]["target"].endswith(".claude/skills/skill-b"))
        extra = [a for a in actions if a["element"] == "skill:coll" and a["op"] == "unlink"]
        self.assertEqual(len(extra), 1)
        self.assertIn("skill-z", extra[0]["note"])
        self.assertFalse(any(a["element"] == "skill:coll/skill-c" for a in actions))
        self.assertFalse(any(a["element"] == "skill:legacy" for a in actions))
        self.assertEqual(payload["data"]["summary"]["exec-local"], len(actions))

        # status↔plan 一致：missing/drifted 元素必有动作，disabled/managed-ok 没有。
        status_payload = self._payload("skills:status", "--json")
        states = {e["id"]: e["state"] for e in status_payload["data"]["elements"]}
        acted = {a["element"] for a in actions}
        for element_id, state in states.items():
            if state in ("missing", "drifted"):
                self.assertIn(element_id, acted, element_id)
            elif state in ("managed-ok", "disabled"):
                self.assertNotIn(element_id, acted, element_id)

    def test_skills_plan_git_pull_when_collection_cache_behind_upstream(self):
        coll, _legacy, _claude_skills, _codex_skills = self._skills_fixture()
        cache_file = self.temp_dir / "upstream.json"
        cache_file.write_text(
            json.dumps(
                {
                    "version": 1,
                    "entries": {
                        "skill:coll": {
                            "state": "behind",
                            "current": "aaa111",
                            "upstream": "bbb222",
                            "checked_at": "2099-01-01T00:00:00Z",
                            "probe": "git-ls-remote",
                        }
                    },
                }
            ),
            encoding="utf-8",
        )
        env_extra = {"AGENT_MANAGER_UPSTREAM_CACHE": str(cache_file)}
        status_payload = self._payload("skills:status", "--json", env_extra=env_extra)
        freshness = self._by_id(status_payload)["skill:coll"]["freshness"]
        self.assertEqual(freshness["state"], "behind")
        self.assertEqual(freshness["upstream"], "bbb222")

        payload = self._payload("skills:plan", "--json", env_extra=env_extra)
        pulls = [a for a in payload["data"]["actions"] if a["op"] == "git-pull"]
        self.assertEqual([a["element"] for a in pulls], ["skill:coll"])
        self.assertEqual(pulls[0]["risk"], "network")
        self.assertIn("bbb222", pulls[0]["note"])
        self.assertEqual(payload["data"]["summary"]["network"], 1)


if __name__ == "__main__":
    unittest.main()
