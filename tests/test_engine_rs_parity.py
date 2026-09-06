"""Dual-implementation parity: Python engine (src/engine_v1.py) vs the Rust
slice (engine-rs, binary 'agent-manager-engine') on a shared sandbox corpus.

Every scenario runs both engines with identical env (fixed run_id / log_file /
lock state / cache path), parses both stdout envelopes, normalizes the
documented volatile keys (run_id, log_file, lock.pid, cache_age_sec, and the
parser-embedded message of MCP_TARGET_PARSE_ERROR), and asserts deep equality
plus identical exit codes. A negative control proves the harness goes red when
one side diverges.

Skipped when cargo is unavailable (AGENTS.md promises python3+make suffice).
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO = shutil.which("cargo")
ENGINE_RS = ROOT / "engine-rs"
DEBUG_BIN = ENGINE_RS / "target" / "debug" / "agent-manager-engine"

COPY_IGNORE = shutil.ignore_patterns("repositories", ".git", "__pycache__", "secrets", ".env", "app", "node_modules", "engine-rs")

VOLATILE_KEYS = {"run_id", "log_file", "cache_age_sec"}
# 解析器诊断文本随实现语言而异；契约只对齐 code/element。
MESSAGE_MASKED_CODES = {"MCP_TARGET_PARSE_ERROR", "HOOKS_TARGET_PARSE_ERROR"}


def normalize(node):
    if isinstance(node, dict):
        out = {}
        for key, value in node.items():
            if key in VOLATILE_KEYS:
                continue
            if key == "lock" and isinstance(value, dict):
                out[key] = {"held": value.get("held")}
                continue
            if key == "message" and isinstance(value, str):
                out[key] = value
                continue
            out[key] = normalize(value)
        return out
    if isinstance(node, list):
        return [normalize(item) for item in node]
    return node


def normalize_envelope(payload):
    payload = normalize(payload)
    for bucket in ("warnings", "errors"):
        for entry in payload.get(bucket, []):
            if entry.get("code") in MESSAGE_MASKED_CODES:
                entry["message"] = "<masked>"
    return payload


def deep_diff(left, right, path="$"):
    diffs = []
    if type(left) is not type(right):
        return ["%s: type %s vs %s" % (path, type(left).__name__, type(right).__name__)]
    if isinstance(left, dict):
        for key in sorted(set(left) | set(right)):
            if key not in left:
                diffs.append("%s.%s: missing in python" % (path, key))
            elif key not in right:
                diffs.append("%s.%s: missing in rust" % (path, key))
            else:
                diffs += deep_diff(left[key], right[key], "%s.%s" % (path, key))
    elif isinstance(left, list):
        if len(left) != len(right):
            diffs.append("%s: len %d vs %d" % (path, len(left), len(right)))
        for index, (x, y) in enumerate(zip(left, right)):
            diffs += deep_diff(x, y, "%s[%d]" % (path, index))
    elif left != right:
        diffs.append("%s: %r vs %r" % (path, left, right))
    return diffs


@unittest.skipUnless(CARGO, "cargo 不可用，跳过 rust parity（AGENTS.md 承诺 python3+make 即可跑检查）")
class EngineRsParityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=ENGINE_RS, capture_output=True, text=True, check=False
        )
        if build.returncode != 0:
            raise unittest.SkipTest("cargo build 失败: %s" % build.stderr[-500:])
        if not DEBUG_BIN.exists():
            raise unittest.SkipTest("rust binary 缺失: %s" % DEBUG_BIN)

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_rs_parity_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        (self.repo / "config" / "manager.local.json").unlink(missing_ok=True)
        (self.repo / "config" / "manager.synced.json").unlink(missing_ok=True)
        self._write_corpus()

    # ------------------------------------------------------------------ corpus

    def _write_corpus(self):
        """alpha=managed-ok beta=missing gamma=disabled delta=未声明
        zeta=drifted epsilon/eta=agent-b(toml) not-materialized。"""
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
        (self.repo / "config" / "manager.json").write_text(json.dumps(manager, indent=2) + "\n", encoding="utf-8")
        self.agent_a = self.home / ".agent-a.json"
        self.agent_b = self.home / ".agent-b" / "config.toml"
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
                "zeta": {"command": "npx", "args": ["-y", "zeta-pkg", "--z"], "agents": ["agent-a"]},
                "eta": {"command": "eta-cmd", "args": ["--e"], "agents": ["agent-b"]},
            },
            "agent_paths": {
                "agent-a": str(self.agent_a),
                "agent-b": {"path": str(self.agent_b), "format": "toml"},
            },
        }
        (self.repo / "config" / "mcp.json").write_text(json.dumps(mcp_def, indent=2) + "\n", encoding="utf-8")
        self.agent_a.write_text(
            json.dumps(
                {
                    "mcpServers": {
                        "alpha": {
                            "command": "alpha-cmd",
                            "args": ["--x"],
                            "env": {"ALPHA_KEY": "engine-rs-fake-alpha-secret-001", "_MANAGED_BY": "agent-manager"},
                        },
                        "zeta": {
                            "command": "npx",
                            "args": ["-y", "zeta-pkg", "--drifted"],
                            "env": {"_MANAGED_BY": "agent-manager"},
                        },
                    }
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def _env(self, extra=None):
        env = os.environ.copy()
        env.pop("ENGINE_TEST_ALPHA_KEY", None)
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_RUN_ID"] = "parity-run-0001"
        env["AGENT_MANAGER_LOG_FILE"] = str(self.temp_dir / "parity.log")
        env["AGENT_MANAGER_LOCK_HELD"] = "false"
        env["AGENT_MANAGER_ROOT_DIR"] = str(self.repo)
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "upstream-cache.json")
        if extra:
            env.update(extra)
        return env

    def _run_python(self, *argv, env_extra=None):
        return subprocess.run(
            ["python3", str(self.repo / "src" / "engine_v1.py"), *argv],
            cwd=self.repo, env=self._env(env_extra), capture_output=True, text=True, check=False, timeout=60,
        )

    def _run_rust(self, *argv, env_extra=None, binary=None):
        return subprocess.run(
            [str(binary or DEBUG_BIN), *argv],
            cwd=self.repo, env=self._env(env_extra), capture_output=True, text=True, check=False, timeout=60,
        )

    def _assert_parity(self, *argv, env_extra=None):
        py = self._run_python(*argv, env_extra=env_extra)
        rs = self._run_rust(*argv, env_extra=env_extra)
        self.assertEqual(
            py.returncode, rs.returncode,
            "exit 码不一致: python=%d rust=%d\npy stderr: %s\nrs stderr: %s"
            % (py.returncode, rs.returncode, py.stderr[-500:], rs.stderr[-500:]),
        )
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(rs.stdout)))
        self.assertEqual(diffs, [], "envelope 不一致:\n%s" % "\n".join(diffs))
        return json.loads(py.stdout)

    # ------------------------------------------------------------------ scenarios

    def test_status_parity_base_corpus(self):
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "engine-rs-fake-live-002"},
        )
        states = {e["id"]: e["state"] for e in payload["data"]["elements"]}
        self.assertEqual(
            states,
            {
                "mcp:alpha": "managed-ok",
                "mcp:beta": "missing",
                "mcp:gamma": "disabled",
                "mcp:delta": "disabled",
                "mcp:zeta": "drifted",
                "mcp:epsilon": "not-materialized",
                "mcp:eta": "not-materialized",
            },
        )

    def test_status_parity_leftover_and_toml_roundtrip(self):
        injected = json.loads(self.agent_a.read_text(encoding="utf-8"))
        injected["mcpServers"]["gamma"] = {"command": "gamma-cmd", "args": [], "env": {"_MANAGED_BY": "agent-manager"}}
        self.agent_a.write_text(json.dumps(injected, indent=2) + "\n", encoding="utf-8")
        self.agent_b.parent.mkdir(parents=True, exist_ok=True)
        self.agent_b.write_text(
            "[mcp_servers.epsilon]\ncommand = \"epsilon-cmd\"\nargs = []\n\n[mcp_servers.epsilon.env]\n_MANAGED_BY = \"agent-manager\"\n",
            encoding="utf-8",
        )
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"},
        )
        states = {e["id"]: e["state"] for e in payload["data"]["elements"]}
        self.assertEqual(states["mcp:gamma"], "drifted")
        self.assertEqual(states["mcp:epsilon"], "managed-ok")

    def test_status_parity_unresolved_placeholder_warning(self):
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json"
        )
        warnings = [w for w in payload["warnings"] if w["code"] == "MCP_ENV_UNRESOLVED"]
        self.assertEqual(len(warnings), 1)
        self.assertEqual(warnings[0]["element"], "mcp:alpha")
        # 红线：假值永不出现在任何一侧输出。
        self.assertNotIn("engine-rs-fake-alpha-secret-001", payload["warnings"][0]["message"])

    def test_status_parity_offline_mode(self):
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json", "--offline",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"},
        )
        self.assertEqual(payload["mode"], {"dry_run": False, "offline": True})

    def test_status_parity_freshness_from_shared_cache(self):
        (self.temp_dir / "upstream-cache.json").write_text(
            json.dumps(
                {
                    "version": 1,
                    "entries": {
                        "mcp:zeta": {
                            "state": "behind",
                            "current": "1.0.0",
                            "upstream": "2.0.0",
                            "checked_at": "2099-01-01T00:00:00Z",
                            "probe": "npm-view",
                        }
                    },
                }
            ),
            encoding="utf-8",
        )
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"},
        )
        freshness = {e["id"]: e["freshness"] for e in payload["data"]["elements"]}
        self.assertEqual(freshness["mcp:zeta"]["state"], "behind")
        self.assertEqual(freshness["mcp:zeta"]["upstream"], "2.0.0")
        self.assertEqual(freshness["mcp:alpha"]["state"], "not-applicable")

    def test_status_parity_stale_cache_warns_identically(self):
        (self.temp_dir / "upstream-cache.json").write_text(
            json.dumps(
                {
                    "version": 1,
                    "entries": {
                        "mcp:zeta": {"state": "behind", "current": "1.0.0", "upstream": "2.0.0", "checked_at": "2000-01-01T00:00:00Z", "probe": "npm-view"}
                    },
                }
            ),
            encoding="utf-8",
        )
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"},
        )
        self.assertIn("UPSTREAM_PROBE_STALE", [w["code"] for w in payload["warnings"]])

    def test_plan_parity_digests_are_byte_identical(self):
        payload = self._assert_parity(
            "mcp:plan", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"},
        )
        actions = payload["data"]["actions"]
        self.assertTrue(actions)
        for action in actions:
            self.assertEqual(action["op"], "write-file")
            if action["after_digest"] is not None:
                self.assertTrue(action["after_digest"].startswith("sha256:"))
        self.assertEqual(payload["data"]["summary"]["write-local"], len(actions))

    def test_status_parity_parse_error_exit_1(self):
        self.agent_a.write_text("{ broken json\n", encoding="utf-8")
        payload = self._assert_parity(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json"
        )
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "MCP_TARGET_PARSE_ERROR")

    def test_usage_error_parity(self):
        for argv in (
            ("mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json"),  # 缺 --json
            ("foo:status", "--config", "config/manager.json", "--json"),  # 未知域
            ("profile:status", "--config", "config/manager.json", "--json"),  # 两侧均未实现
            ("mcp:status", "--json"),  # 缺 --config
        ):
            with self.subTest(argv=argv):
                self._assert_parity(*argv)

    def test_harness_goes_red_when_one_side_diverges(self):
        # 反例：包装 binary 翻转一个元素状态，比较器必须发现。
        wrapper = self.temp_dir / "mutating-engine.sh"
        wrapper.write_text(
            "#!/bin/sh\n"
            "%s \"$@\" | python3 -c \"\n"
            "import json, sys\n"
            "p = json.load(sys.stdin)\n"
            "els = p.get('data', {}).get('elements', [])\n"
            "if els:\n"
            "    els[0]['state'] = 'unsupported'\n"
            "print(json.dumps(p, indent=2, ensure_ascii=False))\n"
            "\"\n" % DEBUG_BIN,
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        py = self._run_python(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"},
        )
        mutated = self._run_rust(
            "mcp:status", "--config", "config/manager.json", "--def-file", "config/mcp.json", "--json",
            env_extra={"ENGINE_TEST_ALPHA_KEY": "x"}, binary=wrapper,
        )
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(mutated.stdout)))
        self.assertTrue(any("state" in diff for diff in diffs), "比较器必须红: %s" % diffs)


@unittest.skipUnless(CARGO, "cargo 不可用，跳过 rust parity")
class EngineRsAgentsParityTests(unittest.TestCase):
    """agents 域双实现对齐：探测层（PATH stub + brew stub）与状态映射全覆盖。"""

    @classmethod
    def setUpClass(cls):
        build = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=ENGINE_RS, capture_output=True, text=True, check=False
        )
        if build.returncode != 0:
            raise unittest.SkipTest("cargo build 失败: %s" % build.stderr[-500:])
        if not DEBUG_BIN.exists():
            raise unittest.SkipTest("rust binary 缺失: %s" % DEBUG_BIN)

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_rs_agents_parity_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        (self.repo / "config" / "manager.local.json").unlink(missing_ok=True)
        (self.repo / "config" / "manager.synced.json").unlink(missing_ok=True)
        self.bindir = self.temp_dir / "bin"
        self.bindir.mkdir()
        self._write_corpus()

    def _make_bin(self, name, body="#!/bin/sh\nexit 0\n", subdir=""):
        target_dir = self.bindir / subdir if subdir else self.bindir
        target_dir.mkdir(parents=True, exist_ok=True)
        path = target_dir / name
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)
        return path

    def _write_corpus(self):
        """fake-ok managed-ok / fake-missing missing / fake-disabled disabled /
        fake-leftover drifted(禁用仍装) / fake-undeclared unmanaged-detected /
        fake-external unmanaged-detected(system+require_managed) /
        fake-deprecated drifted(brew-cask) / fake-pref managed-ok(偏好偏离) /
        fake-gap drifted+AGENT_OFFICIAL_GAP（禁用仍装且官方无 uninstall）。"""
        self._write_config("manager.json", {
            "agents": {
                "fake-ok": {"enabled": True},
                "fake-missing": {"enabled": True},
                "fake-disabled": {"enabled": False},
                "fake-leftover": {"enabled": False},
                "fake-external": {"enabled": True},
                "fake-deprecated": {"enabled": True},
                "fake-pref": {"enabled": True, "preferred_provider": "npm"},
                "fake-gap": {"enabled": False},
            },
            "tools": {}, "skills": {}, "mcp": {},
        })
        defs = {
            "fake-ok": {"bin": "fake-ok-bin", "providers": {"npm": {"managed": True, "install": "npm install -g @fake/ok", "detect_paths": [str(self.bindir / "fake-ok-bin")]}}},
            "fake-missing": {"bin": "fake-missing-bin", "providers": {"npm": {"managed": True, "install": "npm install -g @fake/missing"}}},
            "fake-disabled": {"bin": "fake-disabled-bin", "providers": {"npm": {"managed": True, "install": "npm install -g @fake/disabled"}}},
            "fake-leftover": {"bin": "fake-leftover-bin", "providers": {"npm": {"managed": True, "install": "npm install -g @fake/leftover", "detect_paths": [str(self.bindir / "fake-leftover-bin")]}}},
            "fake-undeclared": {"bin": "fake-undeclared-bin", "providers": {"native": {"managed": False, "detect_paths": [str(self.bindir / "fake-undeclared-bin")]}}},
            "fake-external": {"bin": "fake-external-bin", "require_managed_provider": True, "providers": {"npm": {"managed": True, "install": "npm install -g @fake/ext"}}},
            "fake-deprecated": {"bin": "fake-dep-bin", "providers": {"brew": {"managed": False, "install": "brew install --cask fake-dep"}}},
            "fake-pref": {
                "bin": "fake-pref-bin",
                "preferred_provider": "npm",
                "providers": {
                    "native": {"managed": True, "install": "true", "detect_paths": [str(self.bindir / "fake-pref-bin")]},
                    "npm": {"managed": True, "install": "npm install -g @fake/pref"},
                },
            },
            "fake-gap": {
                "bin": "fake-gap-bin",
                "providers": {
                    "npm": {
                        "managed": True,
                        "install": "npm install -g @fake/gap",
                        "uninstall": "npm uninstall -g @fake/gap",
                        "unsupported_actions": ["uninstall"],
                        "detect_paths": [str(self.bindir / "fake-gap-bin")],
                    }
                },
            },
        }
        self._write_config("agents.json", defs)
        self._make_bin("fake-ok-bin", "#!/bin/sh\necho 1.0.0\n")
        for name in ("fake-leftover-bin", "fake-undeclared-bin", "fake-external-bin", "fake-pref-bin", "fake-gap-bin"):
            self._make_bin(name)
        # brew stub：formula 列表为空，cask 列表含 fake-dep。
        self._make_bin("brew", "#!/bin/sh\nif [ \"$2\" = \"--cask\" ]; then echo fake-dep; fi\nexit 0\n")

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _env(self, extra=None):
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        # bindir 优先（探测 hermetic），保留真实 PATH 其余部分：受限 PATH 会让
        # python 落到 /usr/bin/python3 (3.9)，sync_tools 在那里不可导入，Python
        # 引擎的探测源分类随之降级——两侧须都在完整能力下对齐。
        env["PATH"] = "%s:%s" % (self.bindir, env.get("PATH", ""))
        env["AGENT_MANAGER_RUN_ID"] = "parity-run-agents"
        env["AGENT_MANAGER_LOG_FILE"] = str(self.temp_dir / "parity.log")
        env["AGENT_MANAGER_LOCK_HELD"] = "false"
        env["AGENT_MANAGER_ROOT_DIR"] = str(self.repo)
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "upstream-cache.json")
        if extra:
            env.update(extra)
        return env

    def _run_python(self, *argv, env_extra=None):
        return subprocess.run(
            ["python3", str(self.repo / "src" / "engine_v1.py"), *argv],
            cwd=self.repo, env=self._env(env_extra), capture_output=True, text=True, check=False, timeout=60,
        )

    def _run_rust(self, *argv, env_extra=None, binary=None):
        return subprocess.run(
            [str(binary or DEBUG_BIN), *argv],
            cwd=self.repo, env=self._env(env_extra), capture_output=True, text=True, check=False, timeout=60,
        )

    def _assert_parity(self, *argv, env_extra=None):
        py = self._run_python(*argv, env_extra=env_extra)
        rs = self._run_rust(*argv, env_extra=env_extra)
        self.assertEqual(
            py.returncode, rs.returncode,
            "exit 码不一致: python=%d rust=%d\npy stderr: %s\nrs stderr: %s"
            % (py.returncode, rs.returncode, py.stderr[-500:], rs.stderr[-500:]),
        )
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(rs.stdout)))
        self.assertEqual(diffs, [], "envelope 不一致:\n%s" % "\n".join(diffs))
        return json.loads(py.stdout)

    def test_agents_status_parity_full_state_matrix(self):
        payload = self._assert_parity(
            "agents:status", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json"
        )
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["agents:fake-ok"]["state"], "managed-ok")
        self.assertEqual(elements["agents:fake-missing"]["state"], "missing")
        self.assertEqual(elements["agents:fake-disabled"]["state"], "disabled")
        self.assertEqual(elements["agents:fake-leftover"]["state"], "drifted")
        self.assertEqual(elements["agents:fake-undeclared"]["state"], "unmanaged-detected")
        self.assertEqual(elements["agents:fake-external"]["state"], "unmanaged-detected")
        self.assertEqual(elements["agents:fake-deprecated"]["state"], "drifted")
        self.assertEqual(elements["agents:fake-pref"]["state"], "managed-ok")
        self.assertFalse(elements["agents:fake-pref"]["observed"]["preferred_active"])
        self.assertEqual(elements["agents:fake-gap"]["state"], "drifted")
        self.assertIn("AGENT_OFFICIAL_GAP", [w["code"] for w in payload["warnings"]])
        # 本地版本探测（bin --version 输出解析）两侧一致。
        self.assertEqual(elements["agents:fake-ok"]["freshness"]["current"], "1.0.0")
        self.assertEqual(elements["agents:fake-ok"]["freshness"]["state"], "unknown")

    def test_agents_status_parity_unsupported_is_error_exit_1(self):
        self._make_bin("fake-unsupported-bin", subdir="node_modules/.bin")
        defs = json.loads((self.repo / "config" / "agents.json").read_text(encoding="utf-8"))
        defs["fake-unsupported"] = {
            "bin": "fake-unsupported-bin",
            "providers": {"native": {"managed": True, "install": "true"}},
        }
        self._write_config("agents.json", defs)
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["agents"]["fake-unsupported"] = {"enabled": True}
        self._write_config("manager.json", manager)
        npm_path = "%s:%s:%s" % (
            self.bindir / "node_modules" / ".bin",
            self.bindir,
            os.environ.get("PATH", ""),
        )
        payload = self._assert_parity(
            "agents:status", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json",
            env_extra={"PATH": npm_path},
        )
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "AGENT_PROVIDER_UNSUPPORTED")
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["agents:fake-unsupported"]["state"], "unsupported")

    def test_agents_status_parity_freshness_cache_fill(self):
        (self.temp_dir / "upstream-cache.json").write_text(
            json.dumps({
                "version": 1,
                "entries": {
                    "agents:fake-ok": {
                        "state": "behind", "current": "1.0.0", "upstream": "2.0.0",
                        "checked_at": "2099-01-01T00:00:00Z", "probe": "npm-view",
                    }
                },
            }),
            encoding="utf-8",
        )
        payload = self._assert_parity(
            "agents:status", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json"
        )
        freshness = {e["id"]: e["freshness"] for e in payload["data"]["elements"]}
        self.assertEqual(freshness["agents:fake-ok"]["state"], "behind")
        self.assertEqual(freshness["agents:fake-ok"]["upstream"], "2.0.0")
        self.assertEqual(freshness["agents:fake-missing"]["state"], "unknown")

    def test_agents_plan_parity_install_actions(self):
        payload = self._assert_parity(
            "agents:plan", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json"
        )
        actions = payload["data"]["actions"]
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["element"], "agents:fake-missing")
        self.assertEqual(actions[0]["op"], "install")
        self.assertEqual(actions[0]["cmd"], "npm install -g @fake/missing")
        self.assertEqual(actions[0]["risk"], "install-remove")
        self.assertEqual(payload["data"]["summary"]["install-remove"], 1)

    def test_agents_plan_parity_policy_gates(self):
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["agents"]["fake-missing"]["auto_install"] = False
        self._write_config("manager.json", manager)
        payload = self._assert_parity(
            "agents:plan", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json"
        )
        self.assertEqual(payload["data"]["actions"], [])
        self.assertIn("AGENT_INSTALL_DISABLED_BY_POLICY", [w["code"] for w in payload["warnings"]])

    def test_agents_plan_parity_platform_command_resolution(self):
        # 平台映射：darwin 键优先，缺失则 posix→default；全无 → AGENT_INSTALL_UNAVAILABLE。
        defs = json.loads((self.repo / "config" / "agents.json").read_text(encoding="utf-8"))
        defs["fake-missing"]["providers"]["npm"]["install"] = {
            "windows": "win-install", "posix": "posix-install", "default": "default-install"
        }
        defs["fake-missing-2"] = {
            "bin": "fake-missing-2-bin",
            "providers": {"npm": {"managed": True, "install": {"windows": "only-windows"}}},
        }
        self._write_config("agents.json", defs)
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["agents"]["fake-missing-2"] = {"enabled": True}
        self._write_config("manager.json", manager)
        payload = self._assert_parity(
            "agents:plan", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json"
        )
        by_element = {a["element"]: a for a in payload["data"]["actions"]}
        self.assertEqual(by_element["agents:fake-missing"]["cmd"], "posix-install")
        self.assertNotIn("agents:fake-missing-2", by_element)
        self.assertIn("AGENT_INSTALL_UNAVAILABLE", [w["code"] for w in payload["warnings"]])

    def test_agents_harness_goes_red_when_rust_diverges(self):
        wrapper = self.temp_dir / "mutating-engine.sh"
        wrapper.write_text(
            "#!/bin/sh\n"
            "%s \"$@\" | python3 -c \"\n"
            "import json, sys\n"
            "p = json.load(sys.stdin)\n"
            "els = p.get('data', {}).get('elements', [])\n"
            "if els:\n"
            "    els[0]['observed']['installed'] = not els[0]['observed']['installed']\n"
            "print(json.dumps(p, indent=2, ensure_ascii=False))\n"
            "\"\n" % DEBUG_BIN,
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        py = self._run_python(
            "agents:status", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json"
        )
        mutated = self._run_rust(
            "agents:status", "--config", "config/manager.json", "--def-file", "config/agents.json", "--json",
            binary=wrapper,
        )
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(mutated.stdout)))
        self.assertTrue(any("installed" in diff for diff in diffs), "比较器必须红: %s" % diffs)


@unittest.skipUnless(CARGO, "cargo 不可用，跳过 rust parity")
class EngineRsSkillsParityTests(unittest.TestCase):
    """skills 域双实现对齐：v2 矩阵、链接拓扑、收件箱、freshness、plan 翻译。"""

    @classmethod
    def setUpClass(cls):
        build = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=ENGINE_RS, capture_output=True, text=True, check=False
        )
        if build.returncode != 0:
            raise unittest.SkipTest("cargo build 失败: %s" % build.stderr[-500:])
        if not DEBUG_BIN.exists():
            raise unittest.SkipTest("rust binary 缺失: %s" % DEBUG_BIN)

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_rs_skills_parity_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        (self.repo / "config" / "manager.local.json").unlink(missing_ok=True)
        (self.repo / "config" / "manager.synced.json").unlink(missing_ok=True)
        self.claude_skills = self.home / ".claude" / "skills"
        self.codex_skills = self.home / ".codex" / "skills"
        self.claude_skills.mkdir(parents=True)
        self.codex_skills.mkdir(parents=True)
        self._write_corpus()

    def _make_skill_repo(self, repo_name, skill_names):
        repo = self.repo / "agent-skill" / "repositories" / repo_name
        for name in skill_names:
            skill_dir = repo / name
            skill_dir.mkdir(parents=True, exist_ok=True)
            (skill_dir / "SKILL.md").write_text(
                "---\nname: %s\ndescription: rs parity\n---\n" % name, encoding="utf-8"
            )
        (repo / ".agent-manager-owned").write_text("{}\n", encoding="utf-8")
        return repo

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _write_corpus(self, coll_policy=None):
        """coll 矩阵集合（skill-a 双 agent、skill-b 仅 claude、skill-c 收件箱）
        + legacy 旧式集合（skill-x，升格等价=范围内启用）。"""
        self.coll = self._make_skill_repo("coll", ("skill-a", "skill-b", "skill-c"))
        self.legacy = self._make_skill_repo("legacy", ("skill-x",))
        self._write_config("skills.json", {
            "repositories": {
                "coll": {"url": "https://example.invalid/coll.git", "agents": ["claude-code", "codex"]},
                "legacy": {"url": "https://example.invalid/legacy.git", "agents": ["claude-code"]},
            },
            "agents": {"claude-code": str(self.claude_skills), "codex": str(self.codex_skills)},
        })
        self._write_config("manager.json", {
            "agents": {}, "tools": {},
            "skills": {
                "coll": coll_policy
                or {
                    "enabled": True,
                    "skills": {
                        "skill-a": {"agents": {"claude-code": True, "codex": True}},
                        "skill-b": {"agents": {"claude-code": True, "codex": False}},
                        "skill-c": {},
                    },
                },
                "legacy": {"enabled": True},
            },
            "mcp": {},
        })

    def _link_corpus(self):
        os.symlink(str(self.coll / "skill-a"), str(self.claude_skills / "skill-a"))  # linked
        os.symlink(str(self.coll / "skill-gone"), str(self.codex_skills / "skill-a"))  # dead-link
        os.symlink(str(self.coll / "skill-a"), str(self.claude_skills / "skill-z"))  # 额外受管链接

    def _env(self, extra=None):
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_RUN_ID"] = "parity-run-skills"
        env["AGENT_MANAGER_LOG_FILE"] = str(self.temp_dir / "parity.log")
        env["AGENT_MANAGER_LOCK_HELD"] = "false"
        env["AGENT_MANAGER_ROOT_DIR"] = str(self.repo)
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "upstream-cache.json")
        if extra:
            env.update(extra)
        return env

    def _run_python(self, *argv):
        return subprocess.run(
            ["python3", str(self.repo / "src" / "engine_v1.py"), *argv],
            cwd=self.repo, env=self._env(), capture_output=True, text=True, check=False, timeout=60,
        )

    def _run_rust(self, *argv, binary=None):
        return subprocess.run(
            [str(binary or DEBUG_BIN), *argv],
            cwd=self.repo, env=self._env(), capture_output=True, text=True, check=False, timeout=60,
        )

    def _assert_parity(self, *argv):
        py = self._run_python(*argv)
        rs = self._run_rust(*argv)
        self.assertEqual(
            py.returncode, rs.returncode,
            "exit 码不一致: python=%d rust=%d\npy stderr: %s\nrs stderr: %s"
            % (py.returncode, rs.returncode, py.stderr[-500:], rs.stderr[-500:]),
        )
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(rs.stdout)))
        self.assertEqual(diffs, [], "envelope 不一致:\n%s" % "\n".join(diffs))
        return json.loads(py.stdout)

    def _status_argv(self):
        return ("skills:status", "--config", "config/manager.json", "--def-file", "config/skills.json", "--json")

    def test_skills_status_parity_matrix_corpus(self):
        self._link_corpus()
        payload = self._assert_parity(*self._status_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["skill:coll"]["state"], "drifted")
        self.assertEqual(elements["skill:coll"]["observed"]["discovery_inbox"], ["skill-c"])
        self.assertEqual(elements["skill:coll"]["observed"]["extra_links"][0]["name"], "skill-z")
        self.assertEqual(elements["skill:coll/skill-a"]["state"], "drifted")
        self.assertEqual(
            elements["skill:coll/skill-a"]["observed"]["agents"]["codex"]["state"], "dead-link"
        )
        self.assertEqual(elements["skill:coll/skill-b"]["state"], "missing")
        self.assertEqual(
            elements["skill:coll/skill-b"]["policy"]["agents"], {"claude-code": True, "codex": False}
        )
        self.assertEqual(elements["skill:coll/skill-c"]["state"], "disabled")
        self.assertTrue(elements["skill:coll/skill-c"]["observed"]["discovery_inbox"])
        self.assertEqual(elements["skill:legacy"]["state"], "managed-ok")
        self.assertFalse(elements["skill:legacy"]["spec"]["matrix_mode"])
        # 旧式升格：skill-x 对 claude-code 默认启用。
        self.assertEqual(
            elements["skill:legacy/skill-x"]["policy"]["agents"], {"claude-code": True}
        )
        self.assertEqual(elements["skill:legacy/skill-x"]["state"], "missing")

    def test_skills_status_parity_new_skill_default_enabled(self):
        self._write_corpus(coll_policy={"enabled": True, "new_skill_default": "enabled"})
        payload = self._assert_parity(*self._status_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        # 新默认 enabled：未列矩阵的 skill 跟随 collection 目标集 → 全部缺链。
        self.assertEqual(elements["skill:coll/skill-c"]["state"], "missing")
        self.assertEqual(elements["skill:coll"]["observed"]["discovery_inbox"], [])
        self.assertEqual(
            elements["skill:coll/skill-c"]["policy"]["agents"], {"claude-code": True, "codex": True}
        )

    def test_skills_status_parity_missing_cache(self):
        self._write_config("skills.json", {
            "repositories": {"gone": {"url": "https://example.invalid/gone.git", "agents": ["claude-code"]}},
            "agents": {"claude-code": str(self.claude_skills)},
        })
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {"gone": {"enabled": True}}, "mcp": {}})
        payload = self._assert_parity(*self._status_argv())
        gone = {e["id"]: e for e in payload["data"]["elements"]}["skill:gone"]
        self.assertEqual(gone["state"], "not-materialized")
        self.assertFalse(gone["observed"]["repo_cached"])
        self.assertIn("SKILL_REPO_CACHE_MISSING", [w["code"] for w in payload["warnings"]])

    def test_skills_status_parity_invalid_matrix_fails_closed(self):
        self._write_corpus(coll_policy={"enabled": True, "skills": {"ghost-skill": {"agents": {"claude-code": True}}}})
        payload = self._assert_parity(*self._status_argv())
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "SKILL_CONFIG_INVALID")
        self.assertEqual(payload["errors"][0]["element"], "skill:coll")

    def test_skills_status_parity_duplicate_name_across_collections(self):
        self._make_skill_repo("coll-b", ("skill-a",))
        defs = json.loads((self.repo / "config" / "skills.json").read_text(encoding="utf-8"))
        defs["repositories"]["coll-b"] = {"url": "https://example.invalid/b.git", "agents": ["claude-code"]}
        self._write_config("skills.json", defs)
        manager = json.loads((self.repo / "config" / "manager.json").read_text(encoding="utf-8"))
        manager["skills"]["coll-b"] = {"enabled": True}
        self._write_config("manager.json", manager)
        payload = self._assert_parity(*self._status_argv())
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "SKILL_DUPLICATE_NAME")
        self.assertEqual(payload["errors"][0]["element"], "skill:coll-b/skill-a")

    def test_skills_status_parity_unmaterialized_when_target_dirs_absent(self):
        self._make_skill_repo("fresh", ("skill-y",))
        self._write_config("skills.json", {
            "repositories": {"fresh": {"url": "https://example.invalid/fresh.git", "agents": ["claude-code"]}},
            "agents": {"claude-code": str(self.home / ".claude" / "skills")},
        })
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {"fresh": {"enabled": True}}, "mcp": {}})
        shutil.rmtree(self.claude_skills.parent)  # .claude 整目录不存在
        payload = self._assert_parity(*self._status_argv())
        skill_y = {e["id"]: e for e in payload["data"]["elements"]}["skill:fresh/skill-y"]
        self.assertEqual(skill_y["state"], "not-materialized")

    def test_skills_status_parity_freshness_cache(self):
        (self.temp_dir / "upstream-cache.json").write_text(
            json.dumps({
                "version": 1,
                "entries": {
                    "skill:coll": {
                        "state": "behind", "current": "aaa111", "upstream": "bbb222",
                        "checked_at": "2099-01-01T00:00:00Z", "probe": "git-ls-remote",
                    }
                },
            }),
            encoding="utf-8",
        )
        payload = self._assert_parity(*self._status_argv())
        freshness = {e["id"]: e["freshness"] for e in payload["data"]["elements"]}
        self.assertEqual(freshness["skill:coll"]["state"], "behind")
        self.assertEqual(freshness["skill:coll"]["upstream"], "bbb222")
        self.assertEqual(freshness["skill:legacy"]["state"], "unknown")
        self.assertEqual(freshness["skill:coll/skill-a"]["state"], "not-applicable")

    def test_skills_plan_parity_ordering_and_types(self):
        self._link_corpus()
        (self.temp_dir / "upstream-cache.json").write_text(
            json.dumps({
                "version": 1,
                "entries": {
                    "skill:coll": {
                        "state": "behind", "current": "aaa111", "upstream": "bbb222",
                        "checked_at": "2099-01-01T00:00:00Z", "probe": "git-ls-remote",
                    }
                },
            }),
            encoding="utf-8",
        )
        payload = self._assert_parity(
            "skills:plan", "--config", "config/manager.json", "--def-file", "config/skills.json", "--json"
        )
        actions = payload["data"]["actions"]
        # 集合动作（git-pull + 额外链接 unlink）排在 skill 动作之前。
        self.assertEqual(actions[0]["op"], "git-pull")
        self.assertEqual(actions[0]["element"], "skill:coll")
        self.assertEqual(actions[0]["risk"], "network")
        self.assertIn("bbb222", actions[0]["note"])
        ops = [(a["element"], a["op"]) for a in actions]
        self.assertIn(("skill:coll", "unlink"), ops)  # 额外链接 skill-z
        self.assertIn(("skill:coll/skill-a", "unlink"), ops)  # codex 死链
        self.assertIn(("skill:coll/skill-b", "symlink"), ops)  # claude 缺链
        self.assertIn(("skill:legacy/skill-x", "symlink"), ops)
        self.assertNotIn(("skill:coll/skill-c", "symlink"), ops)  # 收件箱无动作
        summary = payload["data"]["summary"]
        self.assertEqual(summary["network"], 1)
        self.assertEqual(summary["exec-local"], len(actions) - 1)

    def test_skills_plan_parity_cache_missing_clone(self):
        self._write_config("skills.json", {
            "repositories": {"gone": {"url": "https://example.invalid/gone.git", "agents": ["claude-code"]}},
            "agents": {"claude-code": str(self.claude_skills)},
        })
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {"gone": {"enabled": True}}, "mcp": {}})
        payload = self._assert_parity(
            "skills:plan", "--config", "config/manager.json", "--def-file", "config/skills.json", "--json"
        )
        actions = payload["data"]["actions"]
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["op"], "git-pull")
        self.assertIn("https://example.invalid/gone.git", actions[0]["note"])

    def test_skills_harness_goes_red_when_rust_diverges(self):
        self._link_corpus()
        wrapper = self.temp_dir / "mutating-engine.sh"
        wrapper.write_text(
            "#!/bin/sh\n"
            "%s \"$@\" | python3 -c \"\n"
            "import json, sys\n"
            "p = json.load(sys.stdin)\n"
            "els = p.get('data', {}).get('elements', [])\n"
            "if els:\n"
            "    els[0]['observed']['repo_cached'] = not els[0]['observed']['repo_cached']\n"
            "print(json.dumps(p, indent=2, ensure_ascii=False))\n"
            "\"\n" % DEBUG_BIN,
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        py = self._run_python(*self._status_argv())
        mutated = self._run_rust(*self._status_argv(), binary=wrapper)
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(mutated.stdout)))
        self.assertTrue(any("repo_cached" in diff for diff in diffs), "比较器必须红: %s" % diffs)


@unittest.skipUnless(CARGO, "cargo 不可用，跳过 rust parity")
class EngineRsRenderParityTests(unittest.TestCase):
    """rules/hooks/aliases 三域（模板/渲染类）双实现对齐。"""

    @classmethod
    def setUpClass(cls):
        build = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=ENGINE_RS, capture_output=True, text=True, check=False
        )
        if build.returncode != 0:
            raise unittest.SkipTest("cargo build 失败: %s" % build.stderr[-500:])
        if not DEBUG_BIN.exists():
            raise unittest.SkipTest("rust binary 缺失: %s" % DEBUG_BIN)

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_rs_render_parity_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        (self.repo / "config" / "manager.local.json").unlink(missing_ok=True)
        (self.repo / "config" / "manager.synced.json").unlink(missing_ok=True)

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _env(self):
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_RUN_ID"] = "parity-run-render"
        env["AGENT_MANAGER_LOG_FILE"] = str(self.temp_dir / "parity.log")
        env["AGENT_MANAGER_LOCK_HELD"] = "false"
        env["AGENT_MANAGER_ROOT_DIR"] = str(self.repo)
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "upstream-cache.json")
        return env

    def _run_python(self, *argv):
        return subprocess.run(
            ["python3", str(self.repo / "src" / "engine_v1.py"), *argv],
            cwd=self.repo, env=self._env(), capture_output=True, text=True, check=False, timeout=60,
        )

    def _run_rust(self, *argv, binary=None):
        return subprocess.run(
            [str(binary or DEBUG_BIN), *argv],
            cwd=self.repo, env=self._env(), capture_output=True, text=True, check=False, timeout=60,
        )

    def _assert_parity(self, *argv):
        py = self._run_python(*argv)
        rs = self._run_rust(*argv)
        self.assertEqual(
            py.returncode, rs.returncode,
            "exit 码不一致: python=%d rust=%d\npy stderr: %s\nrs stderr: %s"
            % (py.returncode, rs.returncode, py.stderr[-500:], rs.stderr[-500:]),
        )
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(rs.stdout)))
        self.assertEqual(diffs, [], "envelope 不一致:\n%s" % "\n".join(diffs))
        return json.loads(py.stdout)

    # ------------------------------------------------------------------ rules

    def _rules_fixture(self):
        (self.repo / "config" / "rules-targets.list").write_text(
            "claude_global|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/CLAUDE.md\n"
            "claude_drift|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/DRIFT.md\n"
            "claude_missing|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/MISSING.md\n"
            "codex_global|global|rules/dist/codex/AGENTS.md|{{HOME}}/.codex/AGENTS.md\n",
            encoding="utf-8",
        )
        self._write_config("manager.json", {
            "agents": {"claude-code": {"enabled": True}, "codex": {"enabled": False}},
            "tools": {}, "skills": {}, "mcp": {},
        })
        dist_src = self.repo / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md"
        ok_target = self.home / ".claude" / "CLAUDE.md"
        ok_target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(dist_src, ok_target)
        (self.home / ".claude" / "DRIFT.md").write_text("drifted content\n", encoding="utf-8")

    def _rules_argv(self):
        return ("rules:status", "--config", "config/manager.json", "--def-file", "config/rules-targets.list", "--json")

    def test_rules_status_parity_convergence_and_scope(self):
        self._rules_fixture()
        payload = self._assert_parity(*self._rules_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["rules:claude_global"]["state"], "managed-ok")
        self.assertEqual(elements["rules:claude_drift"]["state"], "drifted")
        self.assertEqual(elements["rules:claude_missing"]["state"], "not-materialized")
        self.assertEqual(elements["rules:codex_global"]["state"], "disabled")
        self.assertEqual(elements["rules:dist"]["state"], "managed-ok")

    def test_rules_status_parity_stale_dist_and_manifest_error(self):
        self._rules_fixture()
        src_core = self.repo / "agent-rule" / "rules" / "src" / "shared" / "core.md"
        # 内容语义：src 改了未重建 → 双侧 drifted（mtime 不驱动判定）。
        src_core.write_text(src_core.read_text(encoding="utf-8") + "\nengine test drift line\n", encoding="utf-8")
        payload = self._assert_parity(*self._rules_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["rules:dist"]["state"], "drifted")
        self.assertIn("RULE_BUILD_STALE", [w["code"] for w in payload["warnings"]])
        # mtime 是浮点观测值，两侧各自读取同一文件系统 → 必须相等（不归一化）。
        self.assertIsNotNone(elements["rules:dist"]["observed"]["src_latest_mtime"])

    def test_rules_status_parity_mtime_inversion_is_not_stale(self):
        # CI 全新 checkout 回归反例：mtime 倒置（src 新于 dist）双侧仍 managed-ok。
        self._rules_fixture()
        src_core = self.repo / "agent-rule" / "rules" / "src" / "shared" / "core.md"
        future = os.path.getmtime(self.repo / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md") + 100
        os.utime(src_core, (future, future))
        payload = self._assert_parity(*self._rules_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["rules:dist"]["state"], "managed-ok")
        self.assertEqual(elements["rules:claude_global"]["state"], "managed-ok")

    def test_rules_plan_parity_build_before_distribute(self):
        self._rules_fixture()
        src_core = self.repo / "agent-rule" / "rules" / "src" / "shared" / "core.md"
        src_core.write_text(src_core.read_text(encoding="utf-8") + "\nengine test drift line\n", encoding="utf-8")
        payload = self._assert_parity(
            "rules:plan", "--config", "config/manager.json", "--def-file", "config/rules-targets.list", "--json"
        )
        actions = payload["data"]["actions"]
        self.assertEqual(actions[0]["op"], "run-cmd")
        self.assertEqual(actions[0]["element"], "rules:dist")
        self.assertIn("build-agent-rules.py", actions[0]["cmd"])
        by_element = {a["element"]: a for a in actions}
        # write-file 的 digest 对两侧必须逐字节相等（同一 sha256 约定）。
        self.assertNotEqual(
            by_element["rules:claude_drift"]["before_digest"],
            by_element["rules:claude_drift"]["after_digest"],
        )
        self.assertIsNone(by_element["rules:claude_missing"]["before_digest"])
        self.assertNotIn("rules:codex_global", by_element)

    def test_rules_manifest_bad_line_parity(self):
        (self.repo / "config" / "rules-targets.list").write_text("broken-line-without-columns\n", encoding="utf-8")
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {}})
        payload = self._assert_parity(*self._rules_argv())
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "RULES_MANIFEST_INVALID")

    # ------------------------------------------------------------------ hooks

    CLAUDE_HOOKS_CFG = {
        "settings_path": "../.claude/settings.json",
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo parity-claude"}]}]},
    }
    CODEX_HOOKS_CFG = {
        "hooks_path": "../.codex/hooks.json",
        "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo parity-codex"}]}]},
    }

    def _hooks_fixture(self):
        (self.repo / ".claude" / "settings.json").unlink(missing_ok=True)
        (self.repo / ".codex" / "hooks.json").unlink(missing_ok=True)
        self._write_config("claude-hooks.json", self.CLAUDE_HOOKS_CFG)
        self._write_config("codex-hooks.json", self.CODEX_HOOKS_CFG)
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {}})

    def _hooks_argv(self):
        return (
            "hooks:status", "--config", "config/manager.json",
            "--def-file", "config/claude-hooks.json", "--def-file", "config/codex-hooks.json", "--json",
        )

    def test_hooks_status_parity_not_materialized_then_ok_then_drift(self):
        self._hooks_fixture()
        payload = self._assert_parity(*self._hooks_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["hooks:claude"]["state"], "not-materialized")
        self.assertEqual(elements["hooks:codex"]["state"], "not-materialized")

        (self.repo / ".claude").mkdir(exist_ok=True)
        (self.repo / ".claude" / "settings.json").write_text(
            json.dumps({"other": True, "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo parity-claude"}]}]}}, indent=2) + "\n",
            encoding="utf-8",
        )
        (self.repo / ".codex").mkdir(exist_ok=True)
        (self.repo / ".codex" / "hooks.json").write_text(
            json.dumps({"hooks": self.CODEX_HOOKS_CFG["hooks"]}, indent=2) + "\n", encoding="utf-8"
        )
        payload = self._assert_parity(*self._hooks_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["hooks:claude"]["state"], "managed-ok")
        self.assertEqual(elements["hooks:codex"]["state"], "managed-ok")

        (self.repo / ".claude" / "settings.json").write_text(json.dumps({"other": True, "hooks": {}}) + "\n", encoding="utf-8")
        (self.repo / ".codex" / "hooks.json").write_text(json.dumps({"hooks": {}}) + "\n", encoding="utf-8")
        payload = self._assert_parity(*self._hooks_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["hooks:claude"]["state"], "drifted")
        self.assertEqual(elements["hooks:claude"]["observed"]["missing_signatures"], 1)
        self.assertEqual(elements["hooks:codex"]["state"], "drifted")

    def test_hooks_status_parity_parse_error(self):
        self._hooks_fixture()
        (self.repo / ".codex").mkdir(exist_ok=True)
        (self.repo / ".codex" / "hooks.json").write_text("{ invalid\n", encoding="utf-8")
        payload = self._assert_parity(*self._hooks_argv())
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "HOOKS_TARGET_PARSE_ERROR")
        self.assertEqual(payload["errors"][0]["element"], "hooks:codex")

    def test_hooks_plan_parity_render_digests_byte_identical(self):
        self._hooks_fixture()
        (self.repo / ".claude").mkdir(exist_ok=True)
        (self.repo / ".claude" / "settings.json").write_text(json.dumps({"other": True, "hooks": {}}, indent=2) + "\n", encoding="utf-8")
        payload = self._assert_parity(
            "hooks:plan", "--config", "config/manager.json",
            "--def-file", "config/claude-hooks.json", "--def-file", "config/codex-hooks.json", "--json",
        )
        actions = payload["data"]["actions"]
        self.assertEqual({a["element"] for a in actions}, {"hooks:claude", "hooks:codex"})
        for action in actions:
            self.assertTrue(action["after_digest"].startswith("sha256:"))
        # after_digest 逐字节相等已由 _assert_parity 证明；这里再确认语义字段。
        claude_action = [a for a in actions if a["element"] == "hooks:claude"][0]
        self.assertIn("签名集合", claude_action["note"])

    # ------------------------------------------------------------------ aliases

    def _aliases_fixture(self, aliases=None):
        self._write_config(
            "shell-aliases.json",
            {"timezone": "UTC", "lang": "C", "aliases": aliases if aliases is not None else {"ll": "ls -l", "gs": "git status"}},
        )
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {}})

    def _alias_block(self):
        lines = ["alias gs='git status'", "alias ll='ls -l'"]
        return "# >>> agent-manager shell aliases >>>\n" + "\n".join(lines) + "\n# <<< agent-manager shell aliases <<<"

    def _aliases_argv(self):
        return ("aliases:status", "--config", "config/manager.json", "--def-file", "config/shell-aliases.json", "--json")

    def test_aliases_status_parity_block_states(self):
        self._aliases_fixture()
        (self.home / ".zshrc").write_text("# user\n\n" + self._alias_block() + "\n", encoding="utf-8")
        payload = self._assert_parity(*self._aliases_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["aliases:zshrc"]["state"], "managed-ok")
        self.assertEqual(elements["aliases:zprofile"]["state"], "not-materialized")

        (self.home / ".zprofile").write_text(
            "# >>> agent-manager shell aliases >>>\nalias stale='old'\n# <<< agent-manager shell aliases <<<\n",
            encoding="utf-8",
        )
        payload = self._assert_parity(*self._aliases_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["aliases:zprofile"]["state"], "drifted")

    def test_aliases_status_parity_missing_block_and_empty_alias_list(self):
        self._aliases_fixture()
        (self.home / ".zshrc").write_text("# user only\n", encoding="utf-8")
        payload = self._assert_parity(*self._aliases_argv())
        self.assertEqual({e["id"]: e for e in payload["data"]["elements"]}["aliases:zshrc"]["state"], "missing")

        self._aliases_fixture(aliases={})
        payload = self._assert_parity(*self._aliases_argv())
        elements = {e["id"]: e for e in payload["data"]["elements"]}
        self.assertEqual(elements["aliases:zshrc"]["state"], "managed-ok")
        self.assertEqual(elements["aliases:zprofile"]["state"], "managed-ok")

    def test_aliases_plan_parity_block_digests(self):
        self._aliases_fixture()
        (self.home / ".zprofile").write_text(
            "# >>> agent-manager shell aliases >>>\nalias stale='old'\n# <<< agent-manager shell aliases <<<\n",
            encoding="utf-8",
        )
        payload = self._assert_parity(
            "aliases:plan", "--config", "config/manager.json", "--def-file", "config/shell-aliases.json", "--json"
        )
        by_element = {a["element"]: a for a in payload["data"]["actions"]}
        self.assertEqual(set(by_element), {"aliases:zshrc", "aliases:zprofile"})
        self.assertNotEqual(
            by_element["aliases:zprofile"]["before_digest"],
            by_element["aliases:zprofile"]["after_digest"],
        )
        self.assertIsNone(by_element["aliases:zshrc"]["before_digest"])

    def test_render_harness_goes_red_when_rust_diverges(self):
        self._aliases_fixture()
        wrapper = self.temp_dir / "mutating-engine.sh"
        wrapper.write_text(
            "#!/bin/sh\n"
            "%s \"$@\" | python3 -c \"\n"
            "import json, sys\n"
            "p = json.load(sys.stdin)\n"
            "els = p.get('data', {}).get('elements', [])\n"
            "if els:\n"
            "    els[0]['state'] = 'drifted'\n"
            "print(json.dumps(p, indent=2, ensure_ascii=False))\n"
            "\"\n" % DEBUG_BIN,
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        py = self._run_python(*self._aliases_argv())
        mutated = self._run_rust(*self._aliases_argv(), binary=wrapper)
        diffs = deep_diff(normalize_envelope(json.loads(py.stdout)), normalize_envelope(json.loads(mutated.stdout)))
        self.assertTrue(any("state" in diff for diff in diffs), "比较器必须红: %s" % diffs)


if __name__ == "__main__":
    unittest.main()
