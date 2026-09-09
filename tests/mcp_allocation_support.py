"""Common fixture and helpers for MCP allocation integration tests."""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

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

REPO_ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = REPO_ROOT / "agent-mcp" / "sync_mcp.py"


def _init_test_db(db_path: Path) -> None:
    """Initialize a compliant manager.db for testing."""
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path))
    init_schema(conn)
    with conn:
        conn.execute("INSERT INTO schema_meta (key, value) VALUES ('schema_version', '1')")
        conn.execute("INSERT INTO schema_meta (key, value) VALUES ('migration_completed_at', '2026-09-08T00:00:00Z')")
    conn.close()


class McpAllocationBaseTestCase(unittest.TestCase):
    """Base fixture providing isolated directories, agent configs, and CLI invocation helper."""

    def setUp(self):
        self._saved_env = {}
        for k in ("HOME", "AGENT_MANAGER_DATA_DIR", "AGENT_MANAGER_DIR"):
            self._saved_env[k] = os.environ.get(k)

        self.temp_dir = Path(tempfile.mkdtemp(prefix="mcp_alloc_test_"))
        self.home_dir = self.temp_dir / "home"
        self.data_dir = self.temp_dir / "data"
        self.config_dir = self.temp_dir / "config"
        self.home_dir.mkdir()
        self.data_dir.mkdir()
        self.config_dir.mkdir()

        os.environ["HOME"] = str(self.home_dir)
        os.environ["AGENT_MANAGER_DATA_DIR"] = str(self.data_dir)

        self.db_path = self.data_dir / "manager.db"
        _init_test_db(self.db_path)

        # Create agent directories
        self.claude_config = self.home_dir / ".claude.json"
        self.codex_config = self.home_dir / ".codex" / "config.toml"
        self.opencode_config = self.home_dir / ".config" / "opencode" / "config.json"
        self.codex_config.parent.mkdir(parents=True)
        self.opencode_config.parent.mkdir(parents=True)

        self.claude_config.write_text("{}", encoding="utf-8")
        self.codex_config.write_text("", encoding="utf-8")
        self.opencode_config.write_text("{}", encoding="utf-8")

        # Create manager.json and mcp.json
        self.manager_json = self.config_dir / "manager.json"
        self.mcp_json = self.config_dir / "mcp.json"
        self.agents_json = self.config_dir / "agents.json"

        self.agents_json.write_text(
            json.dumps(
                {
                    "claude-code": {"display_name": "Claude"},
                    "codex": {"display_name": "Codex"},
                    "opencode": {"display_name": "OpenCode"},
                }
            ),
            encoding="utf-8",
        )

        self.base_definitions = {
            "servers": {
                "demo-srv": {
                    "command": "node",
                    "args": ["demo.js", "--port", "3000"],
                    "env": {"API_TOKEN": "${DEMO_TOKEN}"},
                    "startup_timeout_sec": 45,
                }
            },
            "agent_paths": {
                "claude-code": str(self.claude_config),
                "codex": {"path": str(self.codex_config), "format": "toml"},
                "opencode": str(self.opencode_config),
            },
        }
        self.mcp_json.write_text(json.dumps(self.base_definitions, indent=2), encoding="utf-8")

        self.base_policy = {
            "mcp": {
                "demo-srv": {"enabled": True},
            }
        }
        self.manager_json.write_text(json.dumps(self.base_policy, indent=2), encoding="utf-8")

        # Fake secrets pool
        self.secrets_dir = self.temp_dir / "secrets"
        self.secrets_dir.mkdir()
        (self.secrets_dir / "mcp.env").write_text("DEMO_TOKEN=very_secret_token_val_9999\n", encoding="utf-8")

    def tearDown(self):
        for k, v in self._saved_env.items():
            if v is not None:
                os.environ[k] = v
            else:
                os.environ.pop(k, None)
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def _run_cli(self, args: list[str], env_extra: dict[str, str] | None = None) -> subprocess.CompletedProcess:
        cmd = [
            sys.executable,
            str(MODULE_PATH),
            "--config",
            str(self.manager_json),
            "--def-file",
            str(self.mcp_json),
        ] + args
        run_env = {**os.environ, "HOME": str(self.home_dir), "AGENT_MANAGER_DATA_DIR": str(self.data_dir)}
        if env_extra:
            run_env.update(env_extra)
        return subprocess.run(
            cmd,
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
            env=run_env,
        )
