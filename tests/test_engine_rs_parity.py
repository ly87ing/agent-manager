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
MESSAGE_MASKED_CODES = {"MCP_TARGET_PARSE_ERROR"}


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


if __name__ == "__main__":
    unittest.main()
