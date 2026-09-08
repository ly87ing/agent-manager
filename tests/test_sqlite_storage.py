"""Unit and integration tests for SQLite storage (Phase 1A).

Covers:
- Atomic migration with approved 5 tables (schema_meta, policy_entries, definitions, sources, operation_logs)
- Zero-loss preservation of auto_install, auto_update, preferred_provider, and per-skill agent overrides
- Corrupt DB protection (existing corrupt or empty DB is never overwritten, original bytes preserved)
- Future schema protection (higher schema_version rejected without overwrite)
- Concurrent migration file locking
- Strict permissions (0700 directory, 0600 db/wal/shm)
- Isolated fixture root (never migrating real developer checkout)
- CLI entrypoint integration via manage.sh
"""

import json
import os
import shutil
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path

from src.secret_scan import scan_text_for_secrets
from src.storage import (
    MigrationError,
    connect_db,
    get_db_path,
    is_sqlite_active,
    migrate_if_needed,
    policy_delete_db,
    policy_get_db,
    policy_set_db,
    project_config_from_db,
    write_projected_config,
)

ROOT = Path(__file__).resolve().parent.parent


class SqliteStorageTests(unittest.TestCase):
    def setUp(self):
        # Isolate environment so real developer configs are never read
        self._saved_env = {}
        for k in (
            "AGENT_MANAGER_LOCAL_CONFIG",
            "AGENT_MANAGER_SYNCED_CONFIG",
            "AGENT_MANAGER_BASE_CONFIG",
            "AGENT_MANAGER_DATA_DIR",
        ):
            self._saved_env[k] = os.environ.get(k)
            os.environ.pop(k, None)

        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_mgr_db_test_"))
        self.data_dir = self.temp_dir / "data"
        self.data_dir.mkdir()
        self.db_path = self.data_dir / "manager.db"

        # Isolated fixture root: all inputs are strictly synthesized
        self.repo_root = self.temp_dir / "repo"
        self.repo_root.mkdir()
        config_dir = self.repo_root / "config"
        config_dir.mkdir()

        (config_dir / "manager.json").write_text(
            json.dumps({
                "agents": {
                    "claude-code": {
                        "enabled": True,
                        "auto_install": False,
                        "auto_update": True,
                        "preferred_provider": "native",
                    },
                    "codex": {
                        "enabled": False,
                        "preferred_provider": "pip",
                    },
                },
                "mcp": {
                    "test-srv": {"enabled": True},
                },
                "skills": {
                    "sample-repo": {
                        "enabled": True,
                        "skills": {
                            "skill-a": {
                                "enabled": True,
                                "agents": {
                                    "codex": False,
                                    "claude-code": True,
                                },
                            },
                        },
                    },
                },
                "tools": {
                    "test-tool": {"enabled": True},
                },
                "timezone": "Asia/Shanghai",
            }, indent=2),
            encoding="utf-8",
        )
        (config_dir / "agents.json").write_text(
            json.dumps({
                "claude-code": {"bin": "claude", "display_name": "Claude Code"},
                "codex": {"bin": "codex", "display_name": "Codex"},
            }, indent=2),
            encoding="utf-8",
        )
        (config_dir / "mcp.json").write_text(
            json.dumps({
                "servers": {
                    "test-srv": {"command": "srv-bin", "args": ["--test"], "agents": ["claude-code"]},
                },
            }, indent=2),
            encoding="utf-8",
        )
        (config_dir / "skills.json").write_text(
            json.dumps({
                "repositories": {
                    "sample-repo": {"url": "https://example.com/skills.git", "agents": ["claude-code"]},
                },
            }, indent=2),
            encoding="utf-8",
        )
        (config_dir / "tools.json").write_text(
            json.dumps({
                "tools": {
                    "test-tool": {"bin": "tool-bin", "install": "npm i -g tool"},
                },
            }, indent=2),
            encoding="utf-8",
        )

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)
        for k, v in self._saved_env.items():
            if v is not None:
                os.environ[k] = v
            else:
                os.environ.pop(k, None)

    def test_migration_creates_db_and_schema_meta(self):
        """Verify initial migration creates manager.db and populates schema_meta and 5 tables."""
        target = migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        self.assertEqual(target, self.db_path)
        self.assertTrue(self.db_path.is_file())

        conn = connect_db(self.db_path, read_only=True)
        try:
            meta = {row["key"]: row["value"] for row in conn.execute("SELECT key, value FROM schema_meta")}
            self.assertEqual(meta.get("schema_version"), "1")
            self.assertIn("migration_completed_at", meta)

            defs = {(row["domain"], row["id"]): row for row in conn.execute("SELECT * FROM definitions")}
            self.assertIn(("agent", "claude-code"), defs)
            self.assertEqual(defs[("agent", "claude-code")]["deleted"], 0)
            self.assertIn(("mcp", "test-srv"), defs)

            entries = {row["key"]: json.loads(row["value_json"]) for row in conn.execute("SELECT * FROM policy_entries")}
            self.assertIn("agents", entries)
            self.assertEqual(entries["timezone"], "Asia/Shanghai")
        finally:
            conn.close()

    def test_policy_properties_preserved_loss_free(self):
        """Verify auto_install/auto_update/preferred_provider and per-skill agent overrides are 100% preserved."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        projected = project_config_from_db(self.db_path)

        # 1. Agent policy properties
        claude_policy = projected["agents"]["claude-code"]
        self.assertTrue(claude_policy["enabled"])
        self.assertFalse(claude_policy["auto_install"])
        self.assertTrue(claude_policy["auto_update"])
        self.assertEqual(claude_policy["preferred_provider"], "native")

        codex_policy = projected["agents"]["codex"]
        self.assertFalse(codex_policy["enabled"])
        self.assertEqual(codex_policy["preferred_provider"], "pip")

        # 2. Per-skill agent overrides
        skill_cfg = projected["skills"]["sample-repo"]
        self.assertTrue(skill_cfg["enabled"])
        self.assertIn("skills", skill_cfg)
        skill_a_agents = skill_cfg["skills"]["skill-a"]["agents"]
        self.assertFalse(skill_a_agents["codex"])
        self.assertTrue(skill_a_agents["claude-code"])

    def test_corrupt_db_is_never_overwritten(self):
        """Verify an existing corrupted DB causes MigrationError and its bytes are strictly preserved."""
        corrupt_bytes = b"CORRUPTED_SQLITE_GARBAGE_DATA_12345\n\x00\xff"
        self.db_path.write_bytes(corrupt_bytes)

        with self.assertRaises(MigrationError) as ctx:
            migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        self.assertIn("损坏", str(ctx.exception))
        # Ensure target file was NOT overwritten or deleted
        self.assertEqual(self.db_path.read_bytes(), corrupt_bytes)

    def test_empty_db_file_is_rejected_as_corrupted(self):
        """Verify an existing 0-byte DB causes MigrationError and is never overwritten."""
        self.db_path.touch()
        self.assertEqual(self.db_path.stat().st_size, 0)

        with self.assertRaises(MigrationError) as ctx:
            migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        self.assertIn("损坏或为空", str(ctx.exception))
        self.assertTrue(self.db_path.is_file())
        self.assertEqual(self.db_path.stat().st_size, 0)

    def test_future_schema_version_is_rejected(self):
        """Verify an existing DB with future schema_version is rejected without overwriting."""
        conn = connect_db(self.db_path)
        try:
            conn.execute("CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT);")
            conn.execute("INSERT INTO schema_meta VALUES ('schema_version', '999');")
            conn.execute("INSERT INTO schema_meta VALUES ('migration_completed_at', '2099-01-01T00:00:00Z');")
            conn.commit()
        finally:
            conn.close()

        with self.assertRaises(MigrationError) as ctx:
            migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        self.assertIn("高于当前程序支持版本", str(ctx.exception))

    def test_db_and_dir_permissions(self):
        """Verify directory has 0700 permissions and DB file has 0600 permissions."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        dir_mode = self.data_dir.stat().st_mode & 0o777
        db_mode = self.db_path.stat().st_mode & 0o777
        self.assertEqual(dir_mode, 0o700)
        self.assertEqual(db_mode, 0o600)

    def test_migration_imports_local_overlays_and_tombstones(self):
        """Verify synced and local overlay values and tombstones (_delete: True) are imported."""
        local_cfg = self.temp_dir / "manager.local.json"
        local_cfg.write_text(
            json.dumps({
                "agent_definitions": {
                    "custom-agent": {"bin": "custom-bin"},
                    "codex": {"_delete": True},
                },
                "mcp": {
                    "definitions": {
                        "test-srv": {"_delete": True},
                        "local-srv": {"command": "local-bin"},
                    },
                },
            }),
            encoding="utf-8",
        )
        os.environ["AGENT_MANAGER_LOCAL_CONFIG"] = str(local_cfg)
        target = migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        conn = connect_db(target, read_only=True)
        try:
            defs = {(row["domain"], row["id"]): row for row in conn.execute("SELECT * FROM definitions")}
            self.assertEqual(defs[("agent", "codex")]["deleted"], 1, "codex should be tombstoned")
            self.assertIn(("agent", "custom-agent"), defs)
            self.assertEqual(defs[("agent", "custom-agent")]["deleted"], 0)

            self.assertEqual(defs[("mcp", "test-srv")]["deleted"], 1, "test-srv should be tombstoned")
            self.assertIn(("mcp", "local-srv"), defs)
        finally:
            conn.close()

    def test_repeated_startup_does_not_reimport_or_resurrect_deleted_items(self):
        """Verify repeated startup never re-imports or resurrects deleted items."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        # Delete an agent in SQLite
        policy_delete_db(self.db_path, "agent_definitions.claude-code")

        # Verify it is deleted in projection
        cfg1 = project_config_from_db(self.db_path)
        self.assertEqual(cfg1["agent_definitions"]["claude-code"], {"_delete": True})

        # Run migration again (simulating repeated startup)
        target2 = migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        self.assertEqual(target2, self.db_path)

        # It MUST NOT resurrect claude-code!
        cfg2 = project_config_from_db(self.db_path)
        self.assertEqual(cfg2["agent_definitions"]["claude-code"], {"_delete": True})

    def test_migration_failure_protection_leaves_no_partial_db(self):
        """Verify failure during migration cleans up and leaves no partial manager.db."""
        (self.repo_root / "config" / "manager.json").write_text("{ invalid json ", encoding="utf-8")

        with self.assertRaises(MigrationError):
            migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        self.assertFalse(self.db_path.is_file())
        tmp_files = list(self.data_dir.glob("*.tmp.*"))
        self.assertEqual(tmp_files, [])

    def test_policy_set_and_delete_persistence_across_reconnect(self):
        """Verify additions, modifications, and deletions persist across connection restarts."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        # Add custom definitions
        policy_set_db(self.db_path, "agents.brand-new.enabled", True)
        policy_set_db(self.db_path, "agent_definitions.brand-new", {"bin": "bn-bin"})
        policy_set_db(self.db_path, "mcp.definitions.custom-mcp", {"command": "mcp-bin", "args": ["--port", "9999"]})
        policy_set_db(self.db_path, "custom_flag", 42)

        # Verify from projected config
        cfg = project_config_from_db(self.db_path)
        self.assertTrue(cfg["agents"]["brand-new"]["enabled"])
        self.assertEqual(cfg["agent_definitions"]["brand-new"]["bin"], "bn-bin")
        self.assertEqual(cfg["mcp"]["definitions"]["custom-mcp"]["command"], "mcp-bin")
        self.assertEqual(cfg["custom_flag"], 42)

        # Delete custom-mcp definition
        policy_delete_db(self.db_path, "mcp.definitions.custom-mcp")
        cfg_after = project_config_from_db(self.db_path)
        self.assertEqual(cfg_after["mcp"]["definitions"]["custom-mcp"], {"_delete": True})

    def test_no_double_writing_to_legacy_files(self):
        """Verify everyday writes to SQLite do NOT write back to legacy JSON files."""
        local_cfg = self.temp_dir / "manager.local.json"
        local_cfg.write_text(json.dumps({"timezone": "UTC"}, indent=2) + "\n", encoding="utf-8")
        os.environ["AGENT_MANAGER_LOCAL_CONFIG"] = str(local_cfg)

        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        before_content = local_cfg.read_text(encoding="utf-8")

        # Perform writes to SQLite
        policy_set_db(self.db_path, "agents.another.enabled", True)
        policy_set_db(self.db_path, "agent_definitions.another", {"bin": "another-bin"})
        policy_delete_db(self.db_path, "agent_definitions.another")

        # Verify local_cfg was NOT modified
        after_content = local_cfg.read_text(encoding="utf-8")
        self.assertEqual(before_content, after_content, "Legacy local file must NOT be written to!")

    def test_secrets_gate_blocks_writing_sensitive_data_to_sqlite(self):
        """Verify suspected secrets are blocked by secret scan before writing to DB."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        fake_secret = "ghp_" + ("A" * 36)
        with self.assertRaises(ValueError) as ctx:
            policy_set_db(self.db_path, "secret_key", {"token": fake_secret})
        self.assertIn("疑似明文敏感信息", str(ctx.exception))

    def test_nested_definition_field_write_lands_on_definition_row(self):
        """agent_definitions.<id>.<field> must update the definitions row, never policy_entries."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        policy_set_db(self.db_path, "agent_definitions.claude-code.display_name", "Renamed")

        conn = connect_db(self.db_path, read_only=True)
        try:
            row = conn.execute(
                "SELECT definition_json FROM definitions WHERE domain = 'agent' AND id = 'claude-code'"
            ).fetchone()
            self.assertEqual(json.loads(row["definition_json"])["display_name"], "Renamed")
            leaked = conn.execute(
                "SELECT COUNT(*) FROM policy_entries WHERE key = 'agent_definitions'"
            ).fetchone()[0]
            self.assertEqual(leaked, 0, "definition writes must never land in policy_entries")
        finally:
            conn.close()

        cfg = project_config_from_db(self.db_path)
        self.assertEqual(cfg["agent_definitions"]["claude-code"]["display_name"], "Renamed")
        self.assertEqual(cfg["agent_definitions"]["claude-code"]["bin"], "claude")

    def test_whole_collection_definition_write_is_refused(self):
        """Writing an entire definition collection at once must fail loudly (no fake success)."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        with self.assertRaises(ValueError) as ctx:
            policy_set_db(self.db_path, "agent_definitions", {"x": {"bin": "y"}})
        self.assertIn("整体集合写", str(ctx.exception))
        with self.assertRaises(ValueError) as ctx:
            policy_set_db(self.db_path, "mcp.definitions", {})
        self.assertIn("整体集合写", str(ctx.exception))

    def test_invalid_definition_value_leaves_no_changes(self):
        """A rejected write must roll back: no partial changes survive in the transaction."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        with self.assertRaises(ValueError):
            policy_set_db(self.db_path, "agent_definitions.broken", {"display_name": "No Bin"})
        conn = connect_db(self.db_path, read_only=True)
        try:
            row = conn.execute(
                "SELECT COUNT(*) FROM definitions WHERE domain = 'agent' AND id = 'broken'"
            ).fetchone()
            self.assertEqual(row[0], 0)
            ops = conn.execute(
                "SELECT COUNT(*) FROM operation_logs WHERE target = 'agent_definitions.broken'"
            ).fetchone()
            self.assertEqual(ops[0], 0)
        finally:
            conn.close()

    def test_policy_get_db_payload_matches_policy_contract_schema(self):
        """The SQLite policy:get payload must validate against contract/engine-v1/policy.schema.json."""
        from tests.contract_validator import load_schema, validate

        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        payload = policy_get_db(self.db_path)

        schema = load_schema(ROOT / "contract" / "engine-v1" / "policy.schema.json")
        errors = validate(schema, payload)
        self.assertEqual(errors, [], "policy_get_db payload violates policy.schema.json: %s" % errors)

        layers = payload["layers"]
        self.assertIsNone(layers["baseline"])
        self.assertIsNone(layers["synced"])
        self.assertIsNone(layers["local"])
        self.assertIn("sqlite", layers)
        self.assertEqual(layers["sqlite"], payload["effective"])

    def test_db_migrate_dry_run_never_creates_db(self):
        """engine-level --dry-run must skip migration even when called directly (not only via manage.sh)."""
        env = os.environ.copy()
        env["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)
        # engine_v1 imports sibling modules from the real repo; run real code
        # with an isolated DATA_DIR so no real state is touched.
        res = subprocess.run(
            ["python3", str(ROOT / "src" / "engine_v1.py"), "db:migrate",
             "--config", str(self.repo_root / "config" / "manager.json"),
             "--python-cmd", "python3", "--json", "--dry-run"],
            cwd=ROOT, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        payload = json.loads(res.stdout)
        self.assertTrue(payload["ok"])
        self.assertTrue(payload["data"]["dry_run"])
        self.assertTrue(payload["data"]["would_migrate"])
        self.assertFalse(self.db_path.is_file(), "db:migrate --dry-run must never create manager.db")

    def test_concurrent_migration_is_serialized_by_lock(self):
        """Concurrent migrate_if_needed calls must serialize: one migrates, others no-op."""
        import threading

        results = []
        errors = []

        def worker():
            try:
                results.append(migrate_if_needed(self.repo_root, data_dir=self.data_dir))
            except Exception as exc:  # noqa: BLE001 - collected for assertion
                errors.append(exc)

        threads = [threading.Thread(target=worker) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=60)

        self.assertEqual(errors, [], "concurrent migration raised: %s" % errors)
        self.assertEqual(results, [self.db_path] * 4)
        conn = connect_db(self.db_path, read_only=True)
        try:
            check = conn.execute("PRAGMA quick_check;").fetchone()
            self.assertEqual(check[0], "ok")
            ver = conn.execute(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'"
            ).fetchone()
            self.assertEqual(ver["value"], "1")
        finally:
            conn.close()

    def test_special_character_data_dir_paths(self):
        """Data dirs containing quotes, spaces, '?', '#' must work (SQLite URI escaping)."""
        tricky = self.temp_dir / "da'ta dir?#%"
        tricky.mkdir(parents=True)
        db = tricky / "manager.db"

        target = migrate_if_needed(self.repo_root, data_dir=tricky)
        self.assertEqual(target, db)
        self.assertTrue(db.is_file())

        # Read-only path also goes through the URI and must not break.
        cfg = project_config_from_db(db)
        self.assertEqual(cfg["timezone"], "Asia/Shanghai")

        policy_set_db(db, "agents.tricky.enabled", True)
        self.assertTrue(project_config_from_db(db)["agents"]["tricky"]["enabled"])

    def test_pure_path_getters_create_nothing(self):
        """get_data_dir/get_db_path/is_sqlite_active are pure: no mkdir/chmod side effects."""
        from src.storage import get_data_dir

        ghost = self.temp_dir / "never-created"
        os.environ["AGENT_MANAGER_DATA_DIR"] = str(ghost)
        try:
            self.assertEqual(get_data_dir(), ghost.resolve())
            self.assertEqual(get_db_path(), ghost.resolve() / "manager.db")
            self.assertFalse(is_sqlite_active())
            self.assertFalse(ghost.exists(), "pure getters must not create the data dir")
        finally:
            os.environ.pop("AGENT_MANAGER_DATA_DIR", None)

    def test_readonly_policy_get_falls_back_to_legacy_without_db(self):
        """GUI-uninitialized case: DATA_DIR set but no DB -> policy:get reads legacy layers, creates nothing."""
        env = os.environ.copy()
        env["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)
        res = subprocess.run(
            ["python3", str(ROOT / "src" / "engine_v1.py"), "policy:get",
             "--config", str(self.repo_root / "config" / "manager.json"),
             "--python-cmd", "python3", "--json"],
            cwd=ROOT, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        payload = json.loads(res.stdout)
        self.assertTrue(payload["ok"])
        # Legacy effective config still served (from --config), nothing written.
        self.assertIn("claude-code", payload["data"]["effective"]["agents"])
        self.assertNotEqual(payload["data"]["attribution"].get("agents"), "sqlite")
        self.assertFalse(self.db_path.is_file(), "read-only policy:get must not create manager.db")
        wal = Path(f"{self.db_path}-wal")
        self.assertFalse(wal.exists())

    def _copy_code_into_fixture(self):
        """Copy manage.sh + engine code into the synthetic fixture root (never read the real checkout's state)."""
        shutil.copytree(
            ROOT / "src",
            self.repo_root / "src",
            ignore=shutil.ignore_patterns("__pycache__"),
        )
        for module_path in (
            "agent-mcp/sync_mcp.py",
            "agent-skill/link_skills.py",
            "agent-tool/sync_tools.py",
            "agent-rule/scripts/build-agent-rules.py",
        ):
            dest = self.repo_root / module_path
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / module_path, dest)
        for extra_dir in ("agent-mcp", "agent-profile", "agent-lifecycle"):
            shutil.copytree(
                ROOT / extra_dir,
                self.repo_root / extra_dir,
                ignore=shutil.ignore_patterns("__pycache__"),
                dirs_exist_ok=True,
            )
        shutil.copytree(
            ROOT / "agent-skill",
            self.repo_root / "agent-skill",
            ignore=shutil.ignore_patterns("__pycache__", "repositories"),
            dirs_exist_ok=True,
        )
        shutil.copytree(
            ROOT / "agent-tool",
            self.repo_root / "agent-tool",
            ignore=shutil.ignore_patterns("__pycache__", "playwright-cli"),
            dirs_exist_ok=True,
        )
        shutil.copytree(
            ROOT / "agent-rule" / "scripts",
            self.repo_root / "agent-rule" / "scripts",
            dirs_exist_ok=True,
        )
        shutil.copy2(ROOT / "manage.sh", self.repo_root / "manage.sh")
        return self.repo_root / "manage.sh"

    def _isolated_env(self):
        env = os.environ.copy()
        for k in ("AGENT_MANAGER_BASE_CONFIG", "AGENT_MANAGER_SYNCED_CONFIG", "AGENT_MANAGER_LOCAL_CONFIG"):
            env.pop(k, None)
        env["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.temp_dir / "lock")
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.temp_dir / "log")
        return env

    def test_cli_access_via_manage_sh_with_isolated_fixture_root(self):
        """Integration test: manage.sh db:status/db:migrate/policy:set/policy:get with copied code + synthetic fixture.

        The real developer checkout is never read: manage.sh and src/ are copied
        into the synthetic fixture root, and only AGENT_MANAGER_DATA_DIR (the
        pre-existing override) points at the isolated data directory.
        """
        manage_sh = self._copy_code_into_fixture()
        env = self._isolated_env()

        # 0. db:status before initialization: valid envelope, DB is NOT created
        res = subprocess.run(
            [str(manage_sh), "db:status", "--json"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        payload = json.loads(res.stdout)
        self.assertTrue(payload["ok"])
        self.assertFalse(payload["data"]["active"])
        self.assertFalse(payload["data"]["exists"])
        self.assertFalse(self.db_path.is_file(), "db:status must never create manager.db")

        # 1. Explicit db:migrate initializes manager.db
        res = subprocess.run(
            [str(manage_sh), "db:migrate", "--json"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        payload = json.loads(res.stdout)
        self.assertTrue(payload["ok"])
        self.assertTrue(payload["data"]["active"])
        self.assertTrue(self.db_path.is_file())

        # 2. policy:set writes directly into manager.db
        res_set = subprocess.run(
            [str(manage_sh), "policy:set", "--layer", "local", "--key", "agents.cli-agent.enabled", "--value", "true", "--json"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res_set.returncode, 0, res_set.stderr)
        payload_set = json.loads(res_set.stdout)
        self.assertTrue(payload_set["ok"])
        self.assertIn("manager.db", payload_set["data"]["path"])

        # 3. policy:get returns effective state sourced from SQLite
        res_get = subprocess.run(
            [str(manage_sh), "policy:get", "--json"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res_get.returncode, 0, res_get.stderr)
        payload_get = json.loads(res_get.stdout)
        self.assertTrue(payload_get["ok"])
        self.assertIn("cli-agent", payload_get["data"]["effective"]["agents"])
        self.assertEqual(payload_get["data"]["attribution"]["agents"], "sqlite")

    def _bump_schema_version(self, version: str):
        """Bump schema_version with a plain connection and freeze the resulting bytes."""
        conn = sqlite3.connect(str(self.db_path))
        try:
            conn.execute("UPDATE schema_meta SET value = ? WHERE key = 'schema_version'", (version,))
            conn.commit()
        finally:
            conn.close()
        return self.db_path.read_bytes()

    def test_baseline_definition_overlays_are_migrated(self):
        """Baseline manager.json agent_definitions/mcp.definitions must land in definitions, not be dropped."""
        mgr = json.loads((self.repo_root / "config" / "manager.json").read_text(encoding="utf-8"))
        mgr["agent_definitions"] = {
            "claude-code": {"display_name": "Base Renamed"},
            "base-only": {"bin": "base-bin"},
        }
        mgr["mcp"]["definitions"] = {"base-mcp": {"command": "base-mcp-bin"}}
        (self.repo_root / "config" / "manager.json").write_text(json.dumps(mgr), encoding="utf-8")

        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        conn = connect_db(self.db_path, read_only=True)
        try:
            defs = {(row["domain"], row["id"]): row for row in conn.execute("SELECT * FROM definitions")}
            claude = json.loads(defs[("agent", "claude-code")]["definition_json"])
            self.assertEqual(claude["display_name"], "Base Renamed")
            self.assertEqual(claude["bin"], "claude")
            self.assertIn(("agent", "base-only"), defs)
            mcp = json.loads(defs[("mcp", "base-mcp")]["definition_json"])
            self.assertEqual(mcp["command"], "base-mcp-bin")
            leaked = conn.execute(
                "SELECT COUNT(*) FROM policy_entries WHERE key IN ('agent_definitions', 'mcp')"
            ).fetchone()[0]
            # mcp policy entry survives minus its definitions subtree; agent_definitions is gone entirely
            keys = {row["key"] for row in conn.execute("SELECT key FROM policy_entries")}
            self.assertNotIn("agent_definitions", keys)
            mcp_entry = json.loads(
                conn.execute("SELECT value_json FROM policy_entries WHERE key = 'mcp'").fetchone()["value_json"]
            )
            self.assertNotIn("definitions", mcp_entry)
        finally:
            conn.close()

    def test_invalid_baseline_overlay_rejects_migration(self):
        """A baseline overlay missing a required key must abort migration with no DB left behind."""
        mgr = json.loads((self.repo_root / "config" / "manager.json").read_text(encoding="utf-8"))
        mgr["agent_definitions"] = {"broken": {"display_name": "No Bin"}}
        (self.repo_root / "config" / "manager.json").write_text(json.dumps(mgr), encoding="utf-8")

        with self.assertRaises(MigrationError) as ctx:
            migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        self.assertIn("拒绝迁移", str(ctx.exception))
        self.assertFalse(self.db_path.is_file())

    def test_profile_pull_push_refused_when_sqlite_active(self):
        """profile:pull/push must refuse with PROFILE_SQLITE_ACTIVE before touching network or legacy files."""
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        env = os.environ.copy()
        env["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)

        fake_synced = self.temp_dir / "manager.synced.json"
        fake_remote = self.temp_dir / "remote.git"
        fake_local = self.temp_dir / "manager.local.json"
        fake_local.write_text(
            json.dumps({"profile_sync": {"repo_url": str(fake_remote)}}), encoding="utf-8"
        )

        for verb in ("pull", "push"):
            res = subprocess.run(
                ["python3", str(ROOT / "agent-profile" / "sync_profile.py"),
                 "profile:%s" % verb,
                 "--synced-config", str(fake_synced),
                 "--local-config", str(fake_local),
                 "--json"],
                cwd=ROOT, env=env, capture_output=True, text=True, check=False,
            )
            self.assertEqual(res.returncode, 1, "%s stderr: %s" % (verb, res.stderr))
            payload = json.loads(res.stdout)
            self.assertFalse(payload["ok"])
            self.assertEqual(payload["errors"][0]["code"], "PROFILE_SQLITE_ACTIVE")

        # Backend was never invoked: no clone, no legacy file writes.
        self.assertFalse(fake_remote.exists())
        self.assertFalse(fake_synced.exists())

    def test_policy_set_delete_dry_run_via_manage_sh_changes_nothing(self):
        """policy:set/delete --dry-run: no writes whether the DB is uninitialized or initialized."""
        manage_sh = self._copy_code_into_fixture()
        env = self._isolated_env()
        local_cfg = self.repo_root / "config" / "manager.local.json"

        # 1. Uninitialized: dry-run set must not create DB nor legacy local file.
        res = subprocess.run(
            [str(manage_sh), "policy:set", "--layer", "local", "--key", "agents.dry.enabled",
             "--value", "true", "--json", "--dry-run"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertTrue(json.loads(res.stdout)["ok"])
        self.assertFalse(self.db_path.is_file(), "dry-run set must not create manager.db")
        self.assertFalse(local_cfg.exists(), "dry-run set must not write the legacy local layer")

        res_del = subprocess.run(
            [str(manage_sh), "policy:delete", "--layer", "local", "--key", "agents.dry.enabled",
             "--json", "--dry-run"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(res_del.returncode, 0, res_del.stderr)
        self.assertFalse(self.db_path.is_file())
        self.assertFalse(local_cfg.exists())

        # 2. Initialized: dry-run set/delete leave the DB byte-identical.
        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        frozen = self.db_path.read_bytes()

        for argv in (
            ["policy:set", "--layer", "local", "--key", "agents.dry.enabled", "--value", "true"],
            ["policy:delete", "--layer", "local", "--key", "agents.claude-code.enabled"],
        ):
            res = subprocess.run(
                [str(manage_sh), *argv, "--json", "--dry-run"],
                cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
            )
            self.assertEqual(res.returncode, 0, "%s: %s" % (argv[0], res.stderr))
            payload = json.loads(res.stdout)
            self.assertTrue(payload["ok"])
            self.assertIsNone(payload["data"]["path"], "dry-run must not report a written path")

        self.assertEqual(self.db_path.read_bytes(), frozen, "dry-run must not modify manager.db")

    def test_future_db_rejected_by_every_read_write_entry_point(self):
        """A future-version activated DB is refused by get/set/delete/project/migrate, bytes untouched."""
        from src.storage import DatabaseNotReadyError

        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        frozen = self._bump_schema_version("999")

        with self.assertRaises(DatabaseNotReadyError):
            policy_get_db(self.db_path)
        with self.assertRaises(DatabaseNotReadyError):
            policy_set_db(self.db_path, "agents.x.enabled", True)
        with self.assertRaises(DatabaseNotReadyError):
            policy_delete_db(self.db_path, "agents.claude-code")
        with self.assertRaises(DatabaseNotReadyError):
            project_config_from_db(self.db_path)
        with self.assertRaises(MigrationError):
            migrate_if_needed(self.repo_root, data_dir=self.data_dir)

        self.assertEqual(self.db_path.read_bytes(), frozen, "rejected entry points must not modify the DB")

    def test_future_db_rejected_via_manage_sh(self):
        """manage.sh policy:get/policy:set against a future DB fail nonzero and never modify it."""
        manage_sh = self._copy_code_into_fixture()
        env = self._isolated_env()

        migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        frozen = self._bump_schema_version("999")

        res_get = subprocess.run(
            [str(manage_sh), "policy:get", "--json"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertNotEqual(res_get.returncode, 0, "policy:get must refuse a future-version DB")

        res_set = subprocess.run(
            [str(manage_sh), "policy:set", "--layer", "local", "--key", "agents.x.enabled", "--value", "true", "--json"],
            cwd=self.repo_root, env=env, capture_output=True, text=True, check=False,
        )
        self.assertNotEqual(res_set.returncode, 0, "policy:set must refuse a future-version DB")

        self.assertEqual(self.db_path.read_bytes(), frozen, "manage.sh must not modify a rejected DB")

    def test_migration_published_db_is_self_contained_for_read_only_connections(self):
        """Regression: ensure manager.db is published in a self-contained state.

        On SQLite implementations (e.g. macOS system SQLite 3.43.2), closing a
        WAL database can leave companion -wal/-shm files. If the temporary DB is
        renamed while depending on WAL mode, subsequent mode=ro read-only connections
        fail with OperationalError('unable to open database file'). Migration must
        checkpoint and switch journal mode to DELETE prior to atomic swap, guaranteeing
        immediate read_only accessibility without WAL files.
        """
        target = migrate_if_needed(self.repo_root, data_dir=self.data_dir)
        self.assertEqual(target, self.db_path)
        self.assertTrue(self.db_path.is_file())

        # Verify no stray temporary companion files remain in data_dir
        stray_tmp = list(self.data_dir.glob("*.tmp*"))
        self.assertEqual(stray_tmp, [], "temporary migration files must be cleaned up")

        # Explicitly ensure target_db can be opened read-only even when no -wal file exists
        wal_file = Path(f"{self.db_path}-wal")
        self.assertFalse(wal_file.exists(), "freshly migrated DB must not leave orphaned -wal")

        ro_conn = connect_db(self.db_path, read_only=True)
        try:
            row = ro_conn.execute("SELECT count(*) FROM definitions").fetchone()
            self.assertGreater(row[0], 0)
            meta = {r["key"]: r["value"] for r in ro_conn.execute("SELECT key, value FROM schema_meta")}
            self.assertIn("migration_completed_at", meta)
        finally:
            ro_conn.close()

        # project_config_from_db uses read_only=True internally and must succeed
        projected = project_config_from_db(self.db_path)
        self.assertIn("agents", projected)

        # Subsequent write must switch cleanly back to WAL and function normally
        policy_set_db(self.db_path, "agents.claude-code.enabled", False)
        updated = project_config_from_db(self.db_path)
        self.assertFalse(updated["agents"]["claude-code"]["enabled"])

    def test_isolated_runtime_migrate_project_policy_lifecycle(self):
        """Verify full lifecycle (migrate -> project -> policy read/write -> repeated startup)

        Runs under isolated HOME and AGENT_MANAGER_DATA_DIR using synthesized configs.
        Tests under sys.executable, and if /usr/bin/python3 exists and is distinct,
        also verifies against system python.
        """
        import sys
        pythons_to_test = [sys.executable]
        system_python = "/usr/bin/python3"
        if Path(system_python).is_file() and os.access(system_python, os.X_OK) and system_python != sys.executable:
            pythons_to_test.append(system_python)

        for py_cmd in pythons_to_test:
            with self.subTest(python=py_cmd):
                sub_temp = Path(tempfile.mkdtemp(prefix="agent_mgr_lifecycle_"))
                try:
                    sub_home = sub_temp / "home"
                    sub_home.mkdir()
                    sub_data = sub_temp / "data"
                    sub_data.mkdir()
                    sub_repo = sub_temp / "repo"
                    shutil.copytree(self.repo_root, sub_repo)

                    sub_env = os.environ.copy()
                    sub_env["HOME"] = str(sub_home)
                    sub_env["AGENT_MANAGER_DATA_DIR"] = str(sub_data)
                    sub_env["AGENT_MANAGER_LOCAL_CONFIG"] = str(sub_temp / "manager.local.json")
                    sub_env["AGENT_MANAGER_SYNCED_CONFIG"] = str(sub_temp / "manager.synced.json")
                    sub_env["PYTHONPATH"] = str(ROOT)

                    code = """
import os, sys, json
from pathlib import Path
from src.storage import migrate_if_needed, project_config_from_db, policy_set_db, policy_delete_db, policy_get_db

repo = Path(sys.argv[1])
data_dir = Path(os.environ['AGENT_MANAGER_DATA_DIR'])

# 1. migrate_if_needed
target_db = migrate_if_needed(repo, data_dir=data_dir)
assert target_db.is_file(), 'manager.db missing'

# 2. project immediately via read-only mode
cfg = project_config_from_db(target_db)
assert 'agents' in cfg, 'missing agents in projection'

# 3. policy:get
p_get = policy_get_db(target_db)
assert p_get['attribution']['agents'] == 'sqlite'

# 4. policy:set
policy_set_db(target_db, 'agents.claude-code.enabled', False)
cfg_after = project_config_from_db(target_db)
assert cfg_after['agents']['claude-code']['enabled'] is False

# 5. policy:delete
policy_delete_db(target_db, 'agents.claude-code.enabled')

# 6. repeated startup (no-op re-migration)
target_db2 = migrate_if_needed(repo, data_dir=data_dir)
assert target_db2 == target_db
print('LIFECYCLE_OK')
"""
                    res = subprocess.run(
                        [py_cmd, "-c", code, str(sub_repo)],
                        cwd=ROOT,
                        env=sub_env,
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(res.returncode, 0, f"{py_cmd} failed:\nSTDOUT:\n{res.stdout}\nSTDERR:\n{res.stderr}")
                    self.assertIn("LIFECYCLE_OK", res.stdout)
                finally:
                    shutil.rmtree(sub_temp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
