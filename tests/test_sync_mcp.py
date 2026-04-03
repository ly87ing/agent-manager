import json
import subprocess
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

from src.toml_compat import tomllib
from tests.agent_mcp_import import sync_mcp


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-mcp" / "sync_mcp.py"


class SyncMcpTests(unittest.TestCase):
    def test_committed_mcp_policy_enables_chrome_devtools_latest(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        policy = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        chrome_devtools = definitions["servers"]["chrome-devtools"]
        self.assertEqual(chrome_devtools["command"], "npx")
        self.assertEqual(chrome_devtools["args"], ["-y", "chrome-devtools-mcp@latest"])
        self.assertEqual(chrome_devtools.get("env", {}), {})
        self.assertIsNone(chrome_devtools["agents"])
        self.assertTrue(policy["mcp"]["chrome-devtools"]["enabled"])

    def test_committed_chrome_devtools_definition_syncs_into_json_and_codex_toml(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        chrome_devtools = deepcopy(definitions["servers"]["chrome-devtools"])
        chrome_devtools.update(
            {
                "enabled": True,
                "_declared_in_policy": True,
                "_sync_ready": True,
            }
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            json_config_path = temp_root / "agent.json"
            toml_config_path = temp_root / "config.toml"
            toml_config_path.write_text('model = "gpt-5.4"\n', encoding="utf-8")

            ok = sync_mcp.manage_mcp_config(
                {
                    "TmpAgent": str(json_config_path),
                    "codex": {"path": str(toml_config_path), "format": "toml"},
                },
                {"chrome-devtools": chrome_devtools},
            )

            self.assertTrue(ok)
            json_updated = json.loads(json_config_path.read_text(encoding="utf-8"))
            self.assertEqual(json_updated["mcpServers"]["chrome-devtools"]["command"], "npx")
            self.assertEqual(
                json_updated["mcpServers"]["chrome-devtools"]["args"],
                ["-y", "chrome-devtools-mcp@latest"],
            )
            self.assertEqual(
                json_updated["mcpServers"]["chrome-devtools"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

            toml_updated = tomllib.loads(toml_config_path.read_text(encoding="utf-8"))
            self.assertEqual(toml_updated["mcp_servers"]["chrome-devtools"]["command"], "npx")
            self.assertEqual(
                toml_updated["mcp_servers"]["chrome-devtools"]["args"],
                ["-y", "chrome-devtools-mcp@latest"],
            )
            self.assertEqual(
                toml_updated["mcp_servers"]["chrome-devtools"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

    def test_invalid_policy_config_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text("{ invalid json\n", encoding="utf-8")
            def_path.write_text('{"servers": {}, "agent_paths": {}}', encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("config.json", combined)
            self.assertIn("无效", combined)

    def test_invalid_definition_config_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"mcp": {}}', encoding="utf-8")
            def_path.write_text("{ invalid json\n", encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("defs.json", combined)
            self.assertIn("无效", combined)

    def test_non_object_definition_section_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"mcp": {}}', encoding="utf-8")
            def_path.write_text('{"servers": [], "agent_paths": {}}', encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("defs.json.servers", combined)
            self.assertIn("必须是对象", combined)

    def test_default_sync_mode_does_not_clone_invalid_repo(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"bad-mcp": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "bad-mcp": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            updated = json.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertIn("bad-mcp", updated["mcpServers"])

    def test_offline_mode_skips_repo_update_and_uses_existing_repo(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_dir = temp_root / "config"
            config_dir.mkdir()
            agent_config_path = temp_root / "agent.json"
            repo_path = temp_root / "mcp" / "offline-mcp"
            (repo_path / ".git").mkdir(parents=True)

            config_path = config_dir / "config.json"
            def_path = config_dir / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"offline-mcp": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "offline-mcp": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "command": "node",
                                "args": ["{repo_path}/server.js"],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                    "--offline",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            updated = json.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertEqual(
                updated["mcpServers"]["offline-mcp"]["args"],
                [f"{repo_path.resolve()}/server.js"],
            )

    def test_missing_policy_server_is_ignored(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(json.dumps({"mcp": {}}), encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "ignored-mcp": {
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                    "--dry-run",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertNotIn("计划原子更新 JSON", combined)
            self.assertNotIn("更新 MCP: ignored-mcp", combined)

    def test_missing_policy_server_removes_existing_managed_json_entry(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(json.dumps({"mcp": {}}), encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "ignored-mcp": {
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )
            agent_config_path.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "ignored-mcp": {
                                "command": "echo",
                                "args": [],
                                "env": {"_MANAGED_BY": "agent-manager"},
                            },
                            "user-mcp": {
                                "command": "echo",
                                "args": [],
                                "env": {"KEEP": "1"},
                            },
                        }
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            updated = json.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertNotIn("ignored-mcp", updated["mcpServers"])
            self.assertIn("user-mcp", updated["mcpServers"])

    def test_missing_policy_server_removes_existing_managed_toml_entry(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(json.dumps({"mcp": {}}), encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "ignored-mcp": {
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "codex": {"path": str(agent_config_path), "format": "toml"},
                        },
                    }
                ),
                encoding="utf-8",
            )
            agent_config_path.write_text(
                "\n".join(
                    [
                        "[mcp_servers.ignored-mcp]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.ignored-mcp.env]",
                        '_MANAGED_BY = "agent-manager"',
                        "",
                        "[mcp_servers.user-mcp]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.user-mcp.env]",
                        'KEEP = "1"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertNotIn("ignored-mcp", updated["mcp_servers"])
            self.assertIn("user-mcp", updated["mcp_servers"])

    def test_prepare_runtime_clone_failure_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"bad-mcp": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "bad-mcp": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {},
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                    "--prepare-runtime",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_prepare_runtime_clone_failure_does_not_write_broken_agent_config(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"bad-mcp": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "bad-mcp": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "command": "node",
                                "args": ["{repo_path}/server.js"],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "TmpAgent": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                    "--prepare-runtime",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(agent_config_path.exists(), result.stdout + result.stderr)

    def test_invalid_existing_json_aborts_without_overwrite(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            original = '{"mcpServers":'
            agent_config_path.write_text(original, encoding="utf-8")
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"demo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "demo": {
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(agent_config_path.read_text(encoding="utf-8"), original)

    def test_invalid_existing_toml_aborts_without_overwrite(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            original = "[mcp_servers.demo\ncommand = \"echo\"\n"
            agent_config_path.write_text(original, encoding="utf-8")
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"demo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "demo": {
                                "command": "echo",
                                "args": [],
                                "env": {},
                            }
                        },
                        "agent_paths": {
                            "codex": {"path": str(agent_config_path), "format": "toml"},
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(agent_config_path.read_text(encoding="utf-8"), original)

    def test_unresolved_placeholder_aborts_sync(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"mcp": {"demo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "demo": {
                                "command": "echo",
                                "args": [],
                                "env": {"TOKEN": "${MISSING_TOKEN}"},
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "mcp"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(agent_config_path.exists(), result.stdout + result.stderr)

    def test_find_unresolved_placeholders_ignores_literal_string_with_placeholder_substring(self):
        unresolved = sync_mcp.find_unresolved_placeholders(
            {"env": {"PROMPT": "prefix-${USER}-suffix"}}
        )

        self.assertEqual(unresolved, [])

    def test_user_owned_toml_section_with_managed_server_name_is_preserved(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            agent_config_path.write_text(
                "\n".join(
                    [
                        "[mcp_servers.bad-mcp]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.bad-mcp.env]",
                        'KEEP = "1"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            sync_mcp.update_toml_config(
                "codex",
                agent_config_path,
                {},
                {"bad-mcp"},
                set(),
                dry_run=False,
            )

            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertIn("bad-mcp", updated["mcp_servers"])
            self.assertEqual(updated["mcp_servers"]["bad-mcp"]["env"]["KEEP"], "1")

    def test_user_owned_toml_section_with_active_managed_server_name_is_replaced(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            agent_config_path.write_text(
                "\n".join(
                    [
                        "[mcp_servers.bad-mcp]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.bad-mcp.env]",
                        'KEEP = "1"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            ok = sync_mcp.update_toml_config(
                "codex",
                agent_config_path,
                {
                    "bad-mcp": {
                        "command": "node",
                        "args": ["server.js"],
                        "env": {"TOKEN": "new-token"},
                    }
                },
                {"bad-mcp"},
                set(),
                dry_run=False,
            )

            self.assertTrue(ok)
            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertEqual(updated["mcp_servers"]["bad-mcp"]["command"], "node")
            self.assertEqual(updated["mcp_servers"]["bad-mcp"]["args"], ["server.js"])
            self.assertNotIn("KEEP", updated["mcp_servers"]["bad-mcp"]["env"])
            self.assertEqual(updated["mcp_servers"]["bad-mcp"]["env"]["TOKEN"], "new-token")
            self.assertEqual(
                updated["mcp_servers"]["bad-mcp"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

    def test_sync_failure_preserves_existing_managed_mcp_entry(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            agent_config_path.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "bad-mcp": {
                                "command": "node",
                                "args": ["/old/server.js"],
                                "env": {"_MANAGED_BY": "agent-manager"},
                            },
                            "user-mcp": {
                                "command": "echo",
                                "args": [],
                                "env": {"KEEP": "1"},
                            },
                        }
                    }
                ),
                encoding="utf-8",
            )

            repo_path = temp_root / "bad-mcp"
            repo_path.mkdir()
            (repo_path / ".git").mkdir()

            sync_mcp.manage_mcp_config(
                {"TmpAgent": str(agent_config_path)},
                {
                    "bad-mcp": {
                        "enabled": True,
                        "_sync_ready": False,
                        "_repo_path": str(repo_path),
                        "command": "node",
                        "args": ["{repo_path}/server.js"],
                        "env": {},
                    }
                },
            )

            updated = json.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertIn("bad-mcp", updated["mcpServers"])
            self.assertIn("user-mcp", updated["mcpServers"])

    def test_sync_failure_preserves_existing_managed_mcp_entry_in_toml(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            agent_config_path.write_text(
                "\n".join(
                    [
                        "[mcp_servers.bad-mcp]",
                        'command = "node"',
                        'args = ["/old/server.js"]',
                        "[mcp_servers.bad-mcp.env]",
                        '_MANAGED_BY = "agent-manager"',
                        "",
                        "[mcp_servers.user-mcp]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.user-mcp.env]",
                        'KEEP = "1"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            repo_path = temp_root / "bad-mcp"
            repo_path.mkdir()
            (repo_path / ".git").mkdir()

            sync_mcp.manage_mcp_config(
                {"codex": {"path": str(agent_config_path), "format": "toml"}},
                {
                    "bad-mcp": {
                        "enabled": True,
                        "_sync_ready": False,
                        "_repo_path": str(repo_path),
                        "command": "node",
                        "args": ["{repo_path}/server.js"],
                        "env": {},
                    }
                },
            )

            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertIn("bad-mcp", updated["mcp_servers"])
            self.assertEqual(updated["mcp_servers"]["bad-mcp"]["args"], ["/old/server.js"])
            self.assertIn("user-mcp", updated["mcp_servers"])

    def test_codex_target_receives_managed_mcp_without_lifecycle_definition(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            agent_config_path.write_text('model = "gpt-5.4"\n', encoding="utf-8")

            sync_mcp.manage_mcp_config(
                {"codex": {"path": str(agent_config_path), "format": "toml"}},
                {
                    "augment-context-engine": {
                        "enabled": True,
                        "_declared_in_policy": True,
                        "_sync_ready": True,
                        "command": "auggie",
                        "args": ["--mcp", "--mcp-auto-workspace"],
                        "env": {"AUGMENT_SESSION_AUTH": "token"},
                    }
                },
                agent_defs={"claude-code": {"bin": "claude"}},
            )

            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertIn("augment-context-engine", updated["mcp_servers"])
            self.assertEqual(
                updated["mcp_servers"]["augment-context-engine"]["command"],
                "auggie",
            )
            self.assertEqual(
                updated["mcp_servers"]["augment-context-engine"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

    def test_unknown_target_without_lifecycle_definition_is_not_treated_as_syncable(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            agent_config_path.write_text('model = "gpt-5.4"\n', encoding="utf-8")

            sync_mcp.manage_mcp_config(
                {"mystery-agent": {"path": str(agent_config_path), "format": "toml"}},
                {
                    "augment-context-engine": {
                        "enabled": True,
                        "_declared_in_policy": True,
                        "_sync_ready": True,
                        "command": "auggie",
                        "args": ["--mcp", "--mcp-auto-workspace"],
                        "env": {"AUGMENT_SESSION_AUTH": "token"},
                    }
                },
                agent_defs={"claude-code": {"bin": "claude"}},
            )

            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertNotIn("mcp_servers", updated)

    def test_managed_toml_server_with_env_subtable_is_not_misclassified_as_user_owned(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"
            agent_config_path.write_text(
                "\n".join(
                    [
                        '[mcp_servers.augment-context-engine]',
                        'command = "auggie"',
                        'args = ["--mcp", "--mcp-auto-workspace"]',
                        '[mcp_servers.augment-context-engine.env]',
                        'AUGMENT_SESSION_AUTH = "old-token"',
                        '_MANAGED_BY = "agent-manager"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            ok = sync_mcp.update_toml_config(
                "codex",
                agent_config_path,
                {
                    "augment-context-engine": {
                        "command": "auggie",
                        "args": ["--mcp", "--mcp-auto-workspace"],
                        "env": {"AUGMENT_SESSION_AUTH": "new-token"},
                    }
                },
                {"augment-context-engine"},
                set(),
                dry_run=False,
            )

            self.assertTrue(ok)
            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertEqual(
                updated["mcp_servers"]["augment-context-engine"]["env"]["AUGMENT_SESSION_AUTH"],
                "new-token",
            )
            self.assertEqual(
                updated["mcp_servers"]["augment-context-engine"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

    def test_update_toml_config_escapes_quoted_args_and_quoted_server_names(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "config.toml"

            ok = sync_mcp.update_toml_config(
                "codex",
                agent_config_path,
                {
                    'augment "beta".engine': {
                        "command": 'node "server"',
                        "args": ['--label=he said "hi"', r"C:\Program Files\Augment\server.js"],
                        "env": {"AUGMENT_SESSION_AUTH": 'token "quoted"'},
                    }
                },
                {'augment "beta".engine'},
                set(),
                dry_run=False,
            )

            self.assertTrue(ok)
            updated = tomllib.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertEqual(
                updated["mcp_servers"]['augment "beta".engine']["command"],
                'node "server"',
            )
            self.assertEqual(
                updated["mcp_servers"]['augment "beta".engine']["args"],
                ['--label=he said "hi"', r"C:\Program Files\Augment\server.js"],
            )
            self.assertEqual(
                updated["mcp_servers"]['augment "beta".engine']["env"]["AUGMENT_SESSION_AUTH"],
                'token "quoted"',
            )
