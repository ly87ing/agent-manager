"""Unit tests for src/upstream_cache.py and the system.refresh-upstream orchestration.

The refresh probes are exercised with a fully fake `net` mapping (no network);
the engine main() is invoked in-process for the offline-denial path.
"""

import io
import json
import os
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout
from datetime import timedelta
from pathlib import Path

from src import engine_v1, upstream_cache


class CacheEnvironmentTests(unittest.TestCase):
    def setUp(self):
        self._saved = {name: os.environ.get(name) for name in (upstream_cache.CACHE_ENV, upstream_cache.TTL_ENV)}
        self.addCleanup(self._restore)

    def _restore(self):
        for name, value in self._saved.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value

    def test_cache_path_prefers_env_override(self):
        os.environ[upstream_cache.CACHE_ENV] = "/tmp/engine_test_cache_marker.json"
        self.assertEqual(upstream_cache.cache_path(), Path("/tmp/engine_test_cache_marker.json"))

    def test_cache_path_defaults_to_tmpdir_per_user(self):
        os.environ.pop(upstream_cache.CACHE_ENV, None)
        path = upstream_cache.cache_path()
        self.assertTrue(path.name.startswith("agent_manager_upstream_"), path)
        self.assertTrue(path.name.endswith(".json"), path)

    def test_ttl_defaults_to_24h_and_honors_env(self):
        os.environ.pop(upstream_cache.TTL_ENV, None)
        self.assertEqual(upstream_cache.ttl_sec(), 24 * 3600)
        os.environ[upstream_cache.TTL_ENV] = "60"
        self.assertEqual(upstream_cache.ttl_sec(), 60)
        os.environ[upstream_cache.TTL_ENV] = "not-a-number"
        self.assertEqual(upstream_cache.ttl_sec(), 24 * 3600)


class CacheIoTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_upstream_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.path = self.temp_dir / "cache.json"

    def test_missing_and_corrupt_files_yield_empty_cache(self):
        self.assertEqual(upstream_cache.load_cache(self.path), {"version": 1, "entries": {}})
        self.path.write_text("{ not json", encoding="utf-8")
        self.assertEqual(upstream_cache.load_cache(self.path), {"version": 1, "entries": {}})
        self.path.write_text('{"version": 1, "entries": []}', encoding="utf-8")
        self.assertEqual(upstream_cache.load_cache(self.path), {"version": 1, "entries": {}})

    def test_write_entries_merges_and_preserves_existing(self):
        upstream_cache.write_entries({"agents:a": {"state": "behind"}}, self.path)
        upstream_cache.write_entries({"mcp:b": {"state": "current"}}, self.path)
        cache = upstream_cache.load_cache(self.path)
        self.assertEqual(set(cache["entries"]), {"agents:a", "mcp:b"})
        self.assertEqual(cache["entries"]["agents:a"]["state"], "behind")

    def test_write_is_atomic_and_private(self):
        upstream_cache.write_entries({"agents:a": {"state": "current"}}, self.path)
        self.assertEqual(oct(self.path.stat().st_mode & 0o777), "0o600")
        leftovers = [p.name for p in self.temp_dir.iterdir() if p.name != "cache.json"]
        self.assertEqual(leftovers, [], "原子写不得留下临时文件: %s" % leftovers)

    def test_staleness_and_age(self):
        now = upstream_cache.now_utc()
        fresh = {"checked_at": upstream_cache.to_iso(now - timedelta(seconds=10))}
        old = {"checked_at": upstream_cache.to_iso(now - timedelta(days=2))}
        self.assertFalse(upstream_cache.is_stale(fresh, now))
        self.assertTrue(upstream_cache.is_stale(old, now))
        self.assertTrue(upstream_cache.is_stale({"checked_at": "garbage"}, now))
        self.assertTrue(upstream_cache.is_stale({}, now))
        self.assertEqual(upstream_cache.cache_age_sec(fresh, now), 10)
        self.assertIsNone(upstream_cache.cache_age_sec({}, now))

        os.environ[upstream_cache.TTL_ENV] = "5"
        self.addCleanup(os.environ.pop, upstream_cache.TTL_ENV)
        self.assertTrue(upstream_cache.is_stale(fresh, now), "TTL=5s 时 10s 前的条目必须 stale")


class FreshnessCompareTests(unittest.TestCase):
    def test_version_comparison_states(self):
        self.assertEqual(engine_v1._compare_freshness("1.2.3", "1.2.3"), "current")
        self.assertEqual(engine_v1._compare_freshness("1.2.3", "1.3.0"), "behind")
        self.assertEqual(engine_v1._compare_freshness("2.0.0", "1.9.9"), "ahead")
        self.assertEqual(engine_v1._compare_freshness("1.2", "1.2.0"), "current")
        self.assertEqual(engine_v1._compare_freshness(None, "1.0.0"), "unknown")
        self.assertEqual(engine_v1._compare_freshness("1.0.0", None), "unknown")

    def test_unorderable_values_fall_back_to_behind(self):
        # commit 哈希无法排序：一致=current，不一致=behind（不猜 ahead）。
        self.assertEqual(engine_v1._compare_freshness("abc123", "abc123"), "current")
        self.assertEqual(engine_v1._compare_freshness("abc123", "def456"), "behind")


class RefreshOrchestrationTests(unittest.TestCase):
    """build_system_refresh_upstream with a fake net mapping; zero network."""

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_refresh_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.root = self.temp_dir / "repo"
        (self.root / "config").mkdir(parents=True)
        self.cache_file = self.temp_dir / "cache.json"
        os.environ[upstream_cache.CACHE_ENV] = str(self.cache_file)
        self.addCleanup(os.environ.pop, upstream_cache.CACHE_ENV)
        self._write_configs()

    def _write_configs(self):
        def write(name, obj):
            (self.root / "config" / name).write_text(json.dumps(obj) + "\n", encoding="utf-8")

        write(
            "manager.json",
            {
                "agents": {"a1": {"enabled": True}, "a-off": {"enabled": False}},
                "tools": {"t1": {"enabled": True}},
                "mcp": {"s1": {"enabled": True}, "s2": {"enabled": True}},
                "skills": {"r1": {"enabled": True}},
            },
        )
        write(
            "agents.json",
            {
                "a1": {"bin": "a1bin", "providers": {"npm": {"managed": True, "install": "npm install -g @fake/a1"}}},
                "a-off": {"bin": "aoff", "providers": {"npm": {"managed": True, "install": "npm install -g @fake/off"}}},
            },
        )
        write("tools.json", {"tools": {"t1": {"install": "npm install -g @fake/t1", "ready_check": "bin:t1bin"}}})
        write(
            "mcp.json",
            {
                "servers": {
                    "s1": {"command": "npx", "args": ["-y", "@fake/s1", "--flag"]},
                    "s2": {"command": "uvx", "args": ["--from", "git+https://example.invalid/s2.git@main", "s2"]},
                },
                "agent_paths": {},
            },
        )
        write("skills.json", {"repositories": {"r1": {"url": "https://example.invalid/r1.git"}}, "agents": {}})
        self.config_file = str(self.root / "config" / "manager.json")

    def _fake_net(self, **overrides):
        net = {
            "npm_view": lambda package: {"@fake/a1": "2.0.0", "@fake/t1": "1.0.0", "@fake/s1": "3.0.0"}[package],
            "npm_global_version": lambda package: "1.0.0",
            "git_remote_state": lambda repo_dir: {
                "status": "stale",
                "local_head": "aaa111",
                "remote_head": "bbb222",
                "local_remote_head": "aaa111",
            },
            "git_ls_remote": lambda repo, branch: "ccc333",
            "pypi_latest": lambda package: "4.0.0",
            "command_version": lambda bin_name: "1.0.0 (fake build)",
            "brew_outdated": lambda package: "",
        }
        net.update(overrides)
        return net

    def test_refresh_writes_cache_and_summarizes_states(self):
        data, warnings, errors = engine_v1.build_system_refresh_upstream(
            self.root, self.config_file, net=self._fake_net()
        )
        self.assertEqual(errors, [])
        self.assertEqual(data["domain"], "system")
        self.assertEqual(data["verb"], "refresh-upstream")
        # a-off 策略禁用 → 不探测。
        ids = [element["id"] for element in data["elements"]]
        self.assertEqual(sorted(ids), ["agents:a1", "mcp:s1", "mcp:s2", "skill:r1", "tools:t1"])

        cache = upstream_cache.load_cache(self.cache_file)
        entry = cache["entries"]["agents:a1"]
        self.assertEqual(entry["state"], "behind")
        self.assertEqual(entry["current"], "1.0.0")
        self.assertEqual(entry["upstream"], "2.0.0")
        self.assertEqual(entry["probe"], "npm-view")
        self.assertNotIn("error", entry)

        self.assertEqual(cache["entries"]["skill:r1"]["state"], "behind")
        self.assertEqual(cache["entries"]["skill:r1"]["upstream"], "bbb222")
        # uvx git 源：上游入库，本地 commit 未探测 → unknown + error（一等状态）。
        s2 = cache["entries"]["mcp:s2"]
        self.assertEqual(s2["state"], "unknown")
        self.assertEqual(s2["upstream"], "ccc333")
        self.assertIn("error", s2)
        self.assertEqual(data["summary"]["probed"], 5)
        # behind: agents:a1 + skill:r1；current: tools:t1（1.0.0 == 1.0.0）；
        # unknown: mcp:s1（npx 缓存无 @fake/s1）+ mcp:s2（uvx 本地 commit 未探测）。
        self.assertEqual(data["summary"]["behind"], 2)
        self.assertEqual(data["summary"]["current"], 1)
        self.assertEqual(data["summary"]["unknown"], 2)
        unknown_codes = [w["code"] for w in warnings]
        self.assertTrue(all(code == "UPSTREAM_PROBE_UNKNOWN" for code in unknown_codes))

    def test_refresh_records_unknown_with_error_on_probe_failure(self):
        net = self._fake_net(npm_view=lambda package: None)
        data, warnings, errors = engine_v1.build_system_refresh_upstream(self.root, self.config_file, net=net)
        self.assertEqual(errors, [])
        cache = upstream_cache.load_cache(self.cache_file)
        for element_id in ("agents:a1", "mcp:s1", "tools:t1"):
            entry = cache["entries"][element_id]
            self.assertEqual(entry["state"], "unknown", element_id)
            self.assertIn("error", entry, element_id)
        self.assertGreaterEqual(data["summary"]["unknown"], 3)

    def test_refresh_only_filters_and_rejects_unknown_element(self):
        data, _warnings, errors = engine_v1.build_system_refresh_upstream(
            self.root, self.config_file, only="agents:a1", net=self._fake_net()
        )
        self.assertEqual(errors, [])
        self.assertEqual([element["id"] for element in data["elements"]], ["agents:a1"])
        cache = upstream_cache.load_cache(self.cache_file)
        self.assertEqual(sorted(cache["entries"]), ["agents:a1"])

        with self.assertRaises(engine_v1.RefreshUsageError):
            engine_v1.build_system_refresh_upstream(
                self.root, self.config_file, only="agents:nope", net=self._fake_net()
            )

    def test_refresh_merge_preserves_unrelated_entries(self):
        upstream_cache.write_entries({"tools:legacy": {"state": "current"}}, self.cache_file)
        engine_v1.build_system_refresh_upstream(
            self.root, self.config_file, only="agents:a1", net=self._fake_net()
        )
        cache = upstream_cache.load_cache(self.cache_file)
        self.assertEqual(set(cache["entries"]), {"tools:legacy", "agents:a1"})

    def test_status_fills_freshness_from_cache(self):
        engine_v1.build_system_refresh_upstream(self.root, self.config_file, net=self._fake_net())
        data, warnings, errors = engine_v1.build_agents_status(
            self.root, self.config_file, [str(self.root / "config" / "agents.json")]
        )
        self.assertEqual(errors, [])
        element = {e["id"]: e for e in data["elements"]}["agents:a1"]
        freshness = element["freshness"]
        self.assertEqual(freshness["state"], "behind")
        self.assertEqual(freshness["current"], "1.0.0")
        self.assertEqual(freshness["upstream"], "2.0.0")
        self.assertIsNotNone(freshness["checked_at"])
        self.assertIsInstance(freshness["cache_age_sec"], int)
        # a-off 禁用 → 不可探测 → not-applicable。
        off = {e["id"]: e for e in data["elements"]}["agents:a-off"]
        self.assertEqual(off["freshness"]["state"], "not-applicable")

    def test_status_without_cache_entry_is_unknown_with_local_current(self):
        data, warnings, errors = engine_v1.build_agents_status(
            self.root, self.config_file, [str(self.root / "config" / "agents.json")]
        )
        self.assertEqual(errors, [])
        freshness = {e["id"]: e for e in data["elements"]}["agents:a1"]["freshness"]
        self.assertEqual(freshness["state"], "unknown")
        self.assertIsNone(freshness["checked_at"])
        self.assertIsNone(freshness["cache_age_sec"])

    def test_status_stale_entry_warns_but_shows_last_known(self):
        engine_v1.build_system_refresh_upstream(self.root, self.config_file, net=self._fake_net())
        cache = upstream_cache.load_cache(self.cache_file)
        old = upstream_cache.to_iso(upstream_cache.now_utc() - timedelta(days=7))
        for entry in cache["entries"].values():
            entry["checked_at"] = old
        upstream_cache.write_entries(cache["entries"], self.cache_file)

        data, warnings, errors = engine_v1.build_agents_status(
            self.root, self.config_file, [str(self.root / "config" / "agents.json")]
        )
        self.assertEqual(errors, [])
        freshness = {e["id"]: e for e in data["elements"]}["agents:a1"]["freshness"]
        self.assertEqual(freshness["state"], "behind", "stale 也必须展示 last-known 值")
        self.assertGreater(freshness["cache_age_sec"], 6 * 24 * 3600)
        stale_warnings = [w for w in warnings if w["code"] == "UPSTREAM_PROBE_STALE"]
        self.assertEqual([w["element"] for w in stale_warnings], ["agents:a1"])


class RefreshCliTests(unittest.TestCase):
    """In-process main() coverage for the refresh-specific CLI paths."""

    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_refresh_cli_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.cache_file = self.temp_dir / "cache.json"
        os.environ[upstream_cache.CACHE_ENV] = str(self.cache_file)
        self.addCleanup(os.environ.pop, upstream_cache.CACHE_ENV)
        config = self.temp_dir / "manager.json"
        config.write_text(
            json.dumps({"agents": {}, "tools": {}, "skills": {}, "mcp": {}}) + "\n", encoding="utf-8"
        )
        self.config_file = str(config)
        # engine 的 root_dir 由自身 __file__ 推导（真实仓库）；空策略 → 无探测目标。

    def _run_main(self, argv):
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            exit_code = engine_v1.main(argv)
        return exit_code, json.loads(buffer.getvalue())

    def test_offline_denies_refresh_without_writing_cache(self):
        exit_code, payload = self._run_main(
            ["system:refresh-upstream", "--config", self.config_file, "--json", "--offline"]
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(payload["errors"][0]["code"], "OFFLINE_NETWORK_DENIED")
        self.assertFalse(self.cache_file.exists(), "offline 拒绝时不得创建缓存文件")

    def test_only_unknown_element_is_usage_error(self):
        exit_code, payload = self._run_main(
            ["system:refresh-upstream", "--config", self.config_file, "--json", "--only", "agents:nope"]
        )
        self.assertEqual(exit_code, 2)
        self.assertEqual(payload["errors"][0]["code"], "USAGE_ERROR")


if __name__ == "__main__":
    unittest.main()
