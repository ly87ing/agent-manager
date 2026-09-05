"""Sandbox tests for agent-profile/sync_profile.py (Phase 2b, proposal §7.2).

Two simulated machines (separated config/cache/snapshot dirs + machine ids)
share one bare git remote; every writable path is redirected by env overrides
(AGENT_MANAGER_SYNCED_CONFIG / AGENT_MANAGER_LOCAL_CONFIG /
AGENT_MANAGER_PROFILE_CACHE / AGENT_MANAGER_SNAPSHOT_DIR /
AGENT_MANAGER_PROFILE_MACHINE_ID), so the real repo and HOME stay untouched.
manage.sh is exercised through its real wiring for dispatch/lock/help-facing
behavior; heavy merge scenarios call the engine module directly.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from tests.contract_validator import load_schema, validate

ROOT = Path(__file__).resolve().parent.parent
CONTRACT_DIR = ROOT / "contract" / "engine-v1"
ENVELOPE_SCHEMA = load_schema(CONTRACT_DIR / "envelope.schema.json")
PROFILE_SCHEMA = load_schema(CONTRACT_DIR / "profile.schema.json")
SYNC_PROFILE = ROOT / "agent-profile" / "sync_profile.py"
MANAGE = ROOT / "manage.sh"


class SyncProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_profile_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.remote = self.temp_dir / "remote.git"
        subprocess.run(
            ["git", "init", "--bare", "--initial-branch=main", str(self.remote)],
            check=True,
            capture_output=True,
        )

    # ------------------------------------------------------------------ helpers

    def _machine_dir(self, name):
        machine_dir = self.temp_dir / name
        machine_dir.mkdir(parents=True)
        return machine_dir

    def _env(self, machine_dir, machine_id, backend_via="env"):
        env = os.environ.copy()
        env.pop("PROFILE_SYNC_REPO_URL", None)
        env["HOME"] = str(machine_dir)
        env["GIT_CONFIG_NOSYSTEM"] = "1"
        env["GIT_TERMINAL_PROMPT"] = "0"
        env["AGENT_MANAGER_SYNCED_CONFIG"] = str(machine_dir / "manager.synced.json")
        env["AGENT_MANAGER_LOCAL_CONFIG"] = str(machine_dir / "manager.local.json")
        env["AGENT_MANAGER_PROFILE_CACHE"] = str(machine_dir / "cache")
        env["AGENT_MANAGER_SNAPSHOT_DIR"] = str(machine_dir / "snapshots")
        env["AGENT_MANAGER_PROFILE_MACHINE_ID"] = machine_id
        env["AGENT_MANAGER_LOCK_FILE"] = str(machine_dir / "lock")
        env["AGENT_MANAGER_LOG_PREFIX"] = str(machine_dir / "logs" / "agent_manager_sync")
        if backend_via == "env":
            env["PROFILE_SYNC_REPO_URL"] = str(self.remote)
        elif backend_via == "local-file":
            self._write_json(
                machine_dir / "manager.local.json",
                {"profile_sync": {"repo_url": str(self.remote)}},
            )
        elif backend_via != "none":
            raise ValueError(backend_via)
        return env

    @staticmethod
    def _write_json(path, obj):
        path.write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _read_synced(self, machine_dir):
        path = machine_dir / "manager.synced.json"
        return json.loads(path.read_text(encoding="utf-8")) if path.is_file() else None

    def _read_base(self, machine_dir):
        path = machine_dir / "manager.synced.base.json"
        return json.loads(path.read_text(encoding="utf-8")) if path.is_file() else None

    def _remote_json(self, name):
        result = subprocess.run(
            ["git", "--git-dir", str(self.remote), "show", "main:%s" % name],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            return None
        return json.loads(result.stdout)

    def _run(self, verb, env, *extra, via_manage=False):
        if via_manage:
            argv = [str(MANAGE), "profile:%s" % verb, "--json"] + list(extra)
        else:
            argv = [sys.executable, str(SYNC_PROFILE), "profile:%s" % verb, "--json"] + list(extra)
        return subprocess.run(
            argv, cwd=ROOT, env=env, capture_output=True, text=True, check=False, timeout=120
        )

    def _payload(self, verb, env, *extra, expect_exit=0, via_manage=False):
        result = self._run(verb, env, *extra, via_manage=via_manage)
        self.assertEqual(
            result.returncode, expect_exit, "stdout:\n%s\nstderr:\n%s" % (result.stdout, result.stderr)
        )
        payload = json.loads(result.stdout)
        errors = validate(ENVELOPE_SCHEMA, payload)
        self.assertEqual(errors, [], "envelope violations: %s" % [str(e) for e in errors])
        return payload

    def _assert_profile_schema(self, payload):
        errors = validate(PROFILE_SCHEMA, payload["data"])
        self.assertEqual(errors, [], "profile schema violations: %s" % [str(e) for e in errors])

    # ------------------------------------------------------------------ status / configuration

    def test_unconfigured_backend_is_usage_error_via_manage_sh(self):
        machine_dir = self._machine_dir("a")
        env = self._env(machine_dir, "machine-a", backend_via="none")
        payload = self._payload("status", env, expect_exit=2, via_manage=True)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["errors"][0]["code"], "PROFILE_BACKEND_UNCONFIGURED")
        self.assertIn("PROFILE_SYNC_REPO_URL", payload["errors"][0]["message"])
        self.assertIn("profile_sync.repo_url", payload["errors"][0]["message"])

    def test_status_offline_reports_local_only(self):
        machine_dir = self._machine_dir("a")
        self._write_json(machine_dir / "manager.synced.json", {"ui": {"theme": "dark"}})
        env = self._env(machine_dir, "machine-a")
        payload = self._payload("status", env, "--offline")
        self._assert_profile_schema(payload)
        self.assertEqual(payload["data"]["state"], "local-only")
        self.assertIsNone(payload["data"]["ahead"])
        self.assertIsNone(payload["data"]["behind"])
        self.assertEqual(payload["data"]["local"]["key_count"], 1)
        self.assertEqual([w["code"] for w in payload["warnings"]], ["PROFILE_OFFLINE_LOCAL_ONLY"])
        # offline status 不做任何网络动作：缓存克隆不得创建
        self.assertFalse((machine_dir / "cache").exists())

    def test_status_uninitialized_remote(self):
        machine_dir = self._machine_dir("a")
        env = self._env(machine_dir, "machine-a")
        payload = self._payload("status", env)
        self._assert_profile_schema(payload)
        self.assertEqual(payload["data"]["state"], "uninitialized")
        self.assertEqual(payload["data"]["backend"]["type"], "git")
        self.assertTrue(payload["data"]["backend"]["configured"])

    # ------------------------------------------------------------------ two-machine sync

    def test_two_machine_disjoint_keys_merge_without_conflicts(self):
        dir_a = self._machine_dir("a")
        dir_b = self._machine_dir("b")
        self._write_json(dir_a / "manager.synced.json", {"ui": {"theme": "dark"}, "mcp": {"alpha": {"enabled": True}}})
        self._write_json(dir_b / "manager.synced.json", {"tools": {"ripgrep": {"enabled": True}}})
        env_a = self._env(dir_a, "machine-a")  # 后端地址经 env 提供
        env_b = self._env(dir_b, "machine-b", backend_via="local-file")  # 经 manager.local.json 提供

        push = self._payload("push", env_a)
        self._assert_profile_schema(push)
        self.assertTrue(push["data"]["published"])
        self.assertEqual(push["data"]["state"], "clean")
        self.assertEqual(push["data"]["gate"], {"scanned": True, "findings": 0})
        self.assertIsNotNone(push["data"]["snapshot"])
        self.assertEqual(self._remote_json("profile.json"), {"ui": {"theme": "dark"}, "mcp": {"alpha": {"enabled": True}}})
        meta = self._remote_json("meta.json")
        self.assertEqual(meta["schema_version"], 1)
        self.assertIn("machine-a", meta["machines"])
        self.assertTrue(meta["machines"]["machine-a"]["last_sync"])

        pull = self._payload("pull", env_b)
        self._assert_profile_schema(pull)
        self.assertEqual(pull["data"]["conflicts"], [])
        self.assertEqual(pull["data"]["state"], "ahead")  # B 尚有本地独有键待发布
        self.assertEqual(
            self._read_synced(dir_b),
            {"ui": {"theme": "dark"}, "mcp": {"alpha": {"enabled": True}}, "tools": {"ripgrep": {"enabled": True}}},
        )
        # base 推进到本次拉取的远端内容
        self.assertEqual(self._read_base(dir_b), {"ui": {"theme": "dark"}, "mcp": {"alpha": {"enabled": True}}})

        self._payload("push", env_b)
        meta = self._remote_json("meta.json")
        self.assertEqual(sorted(meta["machines"]), ["machine-a", "machine-b"])
        pull_back = self._payload("pull", env_a)
        self.assertEqual(pull_back["data"]["state"], "clean")
        self.assertIn("tools", self._read_synced(dir_a))
        for env in (env_a, env_b):
            status = self._payload("status", env)
            self.assertEqual(status["data"]["state"], "clean")
            self.assertEqual(status["data"]["ahead"], 0)
            self.assertEqual(status["data"]["behind"], 0)

    def test_conflict_pull_keeps_local_reports_and_base_advances(self):
        dir_a = self._machine_dir("a")
        dir_b = self._machine_dir("b")
        self._write_json(dir_a / "manager.synced.json", {"ui": {"theme": "dark", "font": "mono"}})
        env_a = self._env(dir_a, "machine-a")
        env_b = self._env(dir_b, "machine-b")
        self._payload("push", env_a)
        self._payload("pull", env_b)  # B 与远端对齐，建立 base

        # 两侧同键双改：A 改 theme=dark2 并 push；B 本地改 theme=light
        synced_a = self._read_synced(dir_a)
        synced_a["ui"]["theme"] = "dark2"
        self._write_json(dir_a / "manager.synced.json", synced_a)
        self._payload("push", env_a)
        synced_b = self._read_synced(dir_b)
        synced_b["ui"]["theme"] = "light"
        self._write_json(dir_b / "manager.synced.json", synced_b)

        pull = self._payload("pull", env_b)
        self._assert_profile_schema(pull)
        self.assertEqual([w["code"] for w in pull["warnings"]], ["PROFILE_CONFLICTS"])
        self.assertEqual([c["key"] for c in pull["data"]["conflicts"]], ["ui.theme"])
        conflict = pull["data"]["conflicts"][0]
        for side in ("base", "local", "remote"):
            self.assertEqual(conflict[side]["type"], "string")
            self.assertTrue(conflict[side]["digest"].startswith("sha256:"))
        merged = self._read_synced(dir_b)
        self.assertEqual(merged["ui"]["theme"], "light", "冲突键必须保留本地值")
        self.assertEqual(merged["ui"]["font"], "mono", "未冲突键保持不动")
        # base 推进到远端内容；本地保留的冲突值使 B 相对远端 ahead
        self.assertEqual(self._read_base(dir_b)["ui"]["theme"], "dark2")
        self.assertEqual(pull["data"]["state"], "ahead")

        # 解决：push 发布本地保留值，再 pull 不再报冲突
        self._payload("push", env_b)
        repull = self._payload("pull", env_b)
        self.assertEqual(repull["data"]["state"], "clean")
        self.assertEqual(repull["data"]["conflicts"], [])
        self.assertEqual(repull["warnings"], [])
        self.assertEqual(self._remote_json("profile.json")["ui"]["theme"], "light")

    # ------------------------------------------------------------------ gates and refusals

    def test_push_gate_blocks_suspected_secret_with_zero_remote_change(self):
        machine_dir = self._machine_dir("a")
        fake_key = "sk-" + "x" * 30  # 拼接避免仓库扫描命中本测试文件
        self._write_json(machine_dir / "manager.synced.json", {"mcp": {"evil": {"key": fake_key}}})
        env = self._env(machine_dir, "machine-a")
        payload = self._payload("push", env, expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "SECRET_GATE_BLOCKED")
        self.assertIn("openai_like_key", payload["errors"][0]["message"])
        # 零推送：远端保持未初始化
        self.assertIsNone(self._remote_json("profile.json"))
        status = self._payload("status", env)
        self.assertEqual(status["data"]["state"], "uninitialized")

    def test_offline_denies_pull_and_push(self):
        machine_dir = self._machine_dir("a")
        env = self._env(machine_dir, "machine-a")
        for verb in ("pull", "push"):
            with self.subTest(verb=verb):
                payload = self._payload(verb, env, "--offline", expect_exit=1)
                self.assertEqual(payload["errors"][0]["code"], "OFFLINE_NETWORK_DENIED")
        self.assertFalse((machine_dir / "cache").exists(), "offline 拒绝不得触碰后端克隆缓存")

    def test_diverged_push_requires_pull_first(self):
        dir_a = self._machine_dir("a")
        dir_b = self._machine_dir("b")
        self._write_json(dir_a / "manager.synced.json", {"ui": {"theme": "dark"}})
        self._write_json(dir_b / "manager.synced.json", {"other": {"x": 1}})
        env_a = self._env(dir_a, "machine-a")
        env_b = self._env(dir_b, "machine-b")
        self._payload("push", env_a)
        # B 从未 pull 过（无 base）：直接 push 会覆盖远端键，必须被拒
        payload = self._payload("push", env_b, expect_exit=1)
        self.assertEqual(payload["errors"][0]["code"], "PROFILE_DIVERGED")
        self.assertEqual(self._remote_json("profile.json"), {"ui": {"theme": "dark"}})
        # pull 合并后 push 放行，远端两侧键都在
        self._payload("pull", env_b)
        self._payload("push", env_b)
        self.assertEqual(self._remote_json("profile.json"), {"ui": {"theme": "dark"}, "other": {"x": 1}})

    # ------------------------------------------------------------------ dry-run / snapshots

    def test_dry_run_pull_and_push_write_nothing(self):
        dir_a = self._machine_dir("a")
        dir_b = self._machine_dir("b")
        self._write_json(dir_a / "manager.synced.json", {"ui": {"theme": "dark"}})
        env_a = self._env(dir_a, "machine-a")
        self._payload("push", env_a)

        env_b = self._env(dir_b, "machine-b")
        pull = self._payload("pull", env_b, "--dry-run")
        self._assert_profile_schema(pull)
        self.assertIsNone(self._read_synced(dir_b), "dry-run pull 不得写共享层")
        self.assertIsNone(self._read_base(dir_b), "dry-run pull 不得写 base")
        self.assertIsNone(pull["data"]["snapshot"])
        self.assertFalse((dir_b / "snapshots").exists(), "dry-run 不得建快照目录")

        self._write_json(dir_b / "manager.synced.json", {"other": {"x": 1}})
        self._payload("pull", env_b)  # 建立 base 使 dry-run push 到达比对逻辑
        before = self._remote_json("profile.json")
        snapshots_before = sorted(p.name for p in (dir_b / "snapshots").iterdir())
        push = self._payload("push", env_b, "--dry-run")
        self._assert_profile_schema(push)
        self.assertFalse(push["data"]["published"])
        self.assertIsNone(push["data"]["snapshot"])
        self.assertEqual(self._remote_json("profile.json"), before, "dry-run push 不得改远端")
        snapshots_after = sorted(p.name for p in (dir_b / "snapshots").iterdir())
        self.assertEqual(snapshots_after, snapshots_before, "dry-run push 不得新增快照")

    def test_real_pull_snapshots_synced_layer_and_base(self):
        dir_a = self._machine_dir("a")
        dir_b = self._machine_dir("b")
        self._write_json(dir_a / "manager.synced.json", {"ui": {"theme": "dark"}})
        env_a = self._env(dir_a, "machine-a")
        self._payload("push", env_a)

        self._write_json(dir_b / "manager.synced.json", {"old": {"y": 2}})
        env_b = self._env(dir_b, "machine-b")
        pull = self._payload("pull", env_b)
        snapshot = pull["data"]["snapshot"]
        manifest = json.loads(
            (Path(snapshot["path"]) / "manifest.json").read_text(encoding="utf-8")
        )
        self.assertEqual(manifest["command"], "profile:pull")
        paths = {entry["path"]: entry["existed"] for entry in manifest["files"]}
        self.assertTrue(paths[str(dir_b / "manager.synced.json")])
        self.assertFalse(paths[str(dir_b / "manager.synced.base.json")])
        # 快照保留了 pull 前的本地共享层
        backed_up = Path(snapshot["path"]) / "files" / str(dir_b / "manager.synced.json").lstrip("/")
        self.assertEqual(json.loads(backed_up.read_text(encoding="utf-8")), {"old": {"y": 2}})

    # ------------------------------------------------------------------ manage.sh wiring

    def test_manage_sh_lock_semantics(self):
        machine_dir = self._machine_dir("a")
        env = self._env(machine_dir, "machine-a")
        status = self._payload("status", env, via_manage=True)
        self.assertFalse(status["lock"]["held"], "profile:status 属只读族（*:status 通配），必须豁免锁")
        pull = self._payload("pull", env, "--offline", expect_exit=1, via_manage=True)
        self.assertTrue(pull["lock"]["held"], "profile:pull 是 network 级写动作，必须持锁")
        self.assertEqual(pull["errors"][0]["code"], "OFFLINE_NETWORK_DENIED")

    def test_manage_sh_forwards_synced_config_env(self):
        # manage.sh 把 AGENT_MANAGER_SYNCED_CONFIG 的解析结果透传给引擎：
        # 共享层落在沙盒路径而非仓库 config/ 下。
        machine_dir = self._machine_dir("a")
        env = self._env(machine_dir, "machine-a")
        payload = self._payload("status", env, via_manage=True)
        self.assertEqual(payload["data"]["local"]["path"], str(machine_dir / "manager.synced.json"))
        self.assertFalse((ROOT / "config" / "manager.synced.base.json").exists())


if __name__ == "__main__":
    unittest.main()
