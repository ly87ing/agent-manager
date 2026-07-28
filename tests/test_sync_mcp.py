import json
import re
import subprocess
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from unittest import mock

from src.toml_compat import tomllib
from tests.agent_mcp_import import sync_mcp


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-mcp" / "sync_mcp.py"


# Python 3.14's argparse colorizes --help output (respecting NO_COLOR/FORCE_COLOR),
# which inserts SGR escapes between flag tokens. Strip them so text assertions test
# the documented content, not the coloring.
_ANSI_SGR_RE = re.compile(r"\x1b\[[0-9;]*m")


def _strip_ansi(text):
    return _ANSI_SGR_RE.sub("", text)


class SyncMcpTests(unittest.TestCase):
    def test_help_lists_force_refresh_short_flag(self):
        result = subprocess.run(
            ["python3", str(MODULE_PATH), "-h"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)
        self.assertIn("-f, --force-refresh", _strip_ansi(combined))

    def test_committed_mcp_definitions_target_only_managed_agents(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        self.assertEqual(
            set(definitions["agent_paths"].keys()),
            {"claude-code", "antigravity-cli", "codex"},
        )

    def test_committed_augment_context_engine_targets_only_managed_agents(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        agents = definitions["servers"]["augment-context-engine"]["agents"]

        self.assertIsInstance(agents, list)
        self.assertEqual(set(agents), {"claude-code", "antigravity-cli", "codex"})

    def test_committed_mcp_policy_keeps_augment_context_engine_disabled_by_default(self):
        policy = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertFalse(policy["mcp"]["augment-context-engine"]["enabled"])

    def test_committed_mcp_policy_enables_chrome_devtools_fixed_version(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        policy = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        chrome_devtools = definitions["servers"]["chrome-devtools"]
        self.assertEqual(chrome_devtools["command"], "npx")
        self.assertRegex(chrome_devtools["args"][1], r"^chrome-devtools-mcp@\d")
        self.assertNotIn("@latest", chrome_devtools["args"][1])
        self.assertEqual(chrome_devtools["startup_timeout_sec"], 120)
        self.assertEqual(
            chrome_devtools["version_lock"],
            {
                "type": "npm",
                "package": "chrome-devtools-mcp",
                "fields": {"args.1": "chrome-devtools-mcp@{version}"},
            },
        )
        self.assertEqual(chrome_devtools.get("env", {}), {})
        self.assertIsNone(chrome_devtools["agents"])
        self.assertTrue(policy["mcp"]["chrome-devtools"]["enabled"])

    def test_committed_grok_search_definition_sets_codex_startup_timeout(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))

        self.assertEqual(definitions["servers"]["grok-search"]["startup_timeout_sec"], 120)

    def test_committed_grok_search_definition_declares_search_provider_credentials(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))

        env = definitions["servers"]["grok-search"]["env"]
        self.assertIn("TAVILY_ENABLED", env)
        self.assertIn("TAVILY_API_KEY", env)
        self.assertIn("TAVILY_API_URL", env)
        self.assertIn("FIRECRAWL_API_KEY", env)
        self.assertIn("FIRECRAWL_API_URL", env)
        self.assertIn("GROK_MODEL", env)
        self.assertEqual(env["TAVILY_ENABLED"], "${TAVILY_ENABLED}")
        self.assertEqual(env["TAVILY_API_KEY"], "${TAVILY_API_KEY}")
        self.assertEqual(env["TAVILY_API_URL"], "${TAVILY_API_URL}")
        self.assertEqual(env["FIRECRAWL_API_KEY"], "${FIRECRAWL_API_KEY}")
        self.assertEqual(env["FIRECRAWL_API_URL"], "${FIRECRAWL_API_URL}")
        self.assertEqual(env["GROK_MODEL"], "${GROK_MODEL}")

    def test_committed_grok_search_definition_uses_fixed_git_commit(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        grok_search = definitions["servers"]["grok-search"]

        self.assertRegex(
            grok_search["args"][1],
            r"^git\+https://github\.com/GuDaStudio/GrokSearch@[0-9a-f]{40}$",
        )
        self.assertEqual(grok_search["version_lock"]["type"], "git-ref")
        self.assertEqual(grok_search["version_lock"]["ref"], "grok-with-tavily")

    def test_committed_serena_definition_uses_fixed_git_commit(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        serena = definitions["servers"]["serena"]

        self.assertEqual(serena["command"], "uvx")
        self.assertRegex(
            serena["args"][1],
            r"^git\+https://github\.com/oraios/serena@[0-9a-f]{40}$",
        )
        self.assertIn("start-mcp-server", serena["args"])
        self.assertIn("ide-assistant", serena["args"])
        self.assertEqual(serena.get("env", {}), {})
        self.assertEqual(
            serena["project_serena_folder_location"],
            "/tmp/serena$projectDir/.serena",
        )
        self.assertIsNone(serena["agents"])
        self.assertEqual(serena["startup_timeout_sec"], 120)
        self.assertEqual(serena["version_lock"]["type"], "git-ref")
        self.assertEqual(serena["version_lock"]["ref"], "main")
        self.assertEqual(
            serena["version_lock"]["fields"],
            {"args.1": "git+https://github.com/oraios/serena@{version}"},
        )

    def test_configure_serena_project_folder_uses_pinned_runtime_and_saves_only_changes(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))
        serena = definitions["servers"]["serena"]

        with mock.patch.object(sync_mcp, "run_cmd", return_value=(True, "")) as run_cmd:
            ok = sync_mcp.configure_serena_project_folder(serena)

        self.assertTrue(ok)
        command = run_cmd.call_args.args[0]
        self.assertEqual(command[:4], ["uvx", "--from", serena["args"][1], "python"])
        self.assertEqual(command[4], "-c")
        self.assertIn(
            "changed = config.project_serena_folder_location != desired",
            command[-2],
        )
        self.assertIn("config._save() if changed else None", command[-2])
        self.assertEqual(command[-1], "/tmp/serena$projectDir/.serena")
        self.assertEqual(run_cmd.call_args.kwargs, {"timeout": 120})

    def test_prepare_runtime_applies_serena_project_folder_policy(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "manager.json"
            def_path = temp_root / "mcp.json"
            config_path.write_text(
                json.dumps({"mcp": {"serena": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "serena": {
                                "command": "uvx",
                                "args": [
                                    "--from",
                                    "git+https://github.com/oraios/serena@" + "a" * 40,
                                    "serena",
                                    "start-mcp-server",
                                ],
                                "env": {},
                                "project_serena_folder_location": (
                                    "/tmp/serena$projectDir/.serena"
                                ),
                                "agents": None,
                            }
                        },
                        "agent_paths": {},
                    }
                ),
                encoding="utf-8",
            )

            with (
                mock.patch.object(
                    sys,
                    "argv",
                    [
                        "sync_mcp.py",
                        "--config",
                        str(config_path),
                        "--def-file",
                        str(def_path),
                        "--base-dir",
                        str(temp_root / "mcp"),
                        "--prepare-runtime",
                    ],
                ),
                mock.patch.object(
                    sync_mcp,
                    "configure_serena_project_folder",
                    return_value=True,
                ) as configure_serena,
                mock.patch.object(sync_mcp, "manage_mcp_config", return_value=True),
            ):
                sync_mcp.main()

        configure_serena.assert_called_once()
        configured_server = configure_serena.call_args.args[0]
        self.assertEqual(
            configured_server["project_serena_folder_location"],
            "/tmp/serena$projectDir/.serena",
        )
        self.assertEqual(
            configure_serena.call_args.kwargs,
            {"dry_run": False, "offline": False},
        )

    def test_committed_mcp_policy_enables_serena(self):
        policy = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertTrue(policy["mcp"]["serena"]["enabled"])

    def test_main_applies_local_version_pins_to_server_args(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "manager.json"
            def_path = temp_root / "mcp.json"
            agent_config_path = temp_root / "agent.json"
            config_path.write_text(
                json.dumps({"mcp": {"demo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "demo": {
                                "command": "npx",
                                "args": ["-y", "demo-mcp@0.1.0"],
                                "env": {},
                                "version_lock": {
                                    "type": "npm",
                                    "package": "demo-mcp",
                                    "fields": {"args.1": "demo-mcp@{version}"},
                                },
                                "agents": ["claude-code"],
                            }
                        },
                        "agent_paths": {"claude-code": str(agent_config_path)},
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "version-pins.local.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "version_locks": {"mcp.json": {"servers.demo": "4.5.6"}},
                        "skill_repositories": {},
                    }
                ),
                encoding="utf-8",
            )

            with mock.patch.object(
                sys,
                "argv",
                ["sync_mcp.py", "--config", str(config_path), "--def-file", str(def_path), "--base-dir", str(temp_root / "mcp")],
            ):
                with mock.patch.object(sync_mcp, "is_agent_installed", return_value=True):
                    sync_mcp.main()

            updated = json.loads(agent_config_path.read_text(encoding="utf-8"))
            self.assertEqual(updated["mcpServers"]["demo"]["args"], ["-y", "demo-mcp@4.5.6"])

    def test_codex_toml_receives_startup_timeout_without_json_leak(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            json_config_path = temp_root / "agent.json"
            toml_config_path = temp_root / "config.toml"

            ok = sync_mcp.manage_mcp_config(
                {
                    "TmpAgent": str(json_config_path),
                    "codex": {"path": str(toml_config_path), "format": "toml"},
                },
                {
                    "grok-search": {
                        "command": "uvx",
                        "args": ["--from", "repo", "grok-search"],
                        "env": {"GROK_API_KEY": "token"},
                        "startup_timeout_sec": 120,
                    }
                },
            )

            self.assertTrue(ok)
            json_updated = json.loads(json_config_path.read_text(encoding="utf-8"))
            self.assertNotIn("startup_timeout_sec", json_updated["mcpServers"]["grok-search"])

            toml_updated = tomllib.loads(toml_config_path.read_text(encoding="utf-8"))
            self.assertEqual(toml_updated["mcp_servers"]["grok-search"]["startup_timeout_sec"], 120)

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
                chrome_devtools["args"],
            )
            self.assertEqual(
                json_updated["mcpServers"]["chrome-devtools"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

            toml_updated = tomllib.loads(toml_config_path.read_text(encoding="utf-8"))
            self.assertEqual(toml_updated["mcp_servers"]["chrome-devtools"]["command"], "npx")
            self.assertEqual(
                toml_updated["mcp_servers"]["chrome-devtools"]["args"],
                chrome_devtools["args"],
            )
            self.assertEqual(
                toml_updated["mcp_servers"]["chrome-devtools"]["startup_timeout_sec"],
                120,
            )
            self.assertEqual(
                toml_updated["mcp_servers"]["chrome-devtools"]["env"]["_MANAGED_BY"],
                "agent-manager",
            )

    def test_antigravity_json_uses_standard_mcp_server_shape(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            antigravity_config_path = temp_root / "mcp_config.json"

            with mock.patch.object(sync_mcp, "is_agent_installed", return_value=True):
                ok = sync_mcp.manage_mcp_config(
                    {"antigravity-cli": {"path": str(antigravity_config_path), "format": "json"}},
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
                    agent_defs={"antigravity-cli": {"bin": "agy"}},
                )

            self.assertTrue(ok)
            updated = json.loads(antigravity_config_path.read_text(encoding="utf-8"))
            self.assertEqual(
                updated["mcpServers"]["augment-context-engine"]["command"],
                "auggie",
            )
            self.assertNotIn("timeout", updated["mcpServers"]["augment-context-engine"])

    def test_clone_or_update_repo_skips_pull_when_probe_reports_latest(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            base_dir = Path(temp_dir)
            repo_path = base_dir / "demo-mcp"
            (repo_path / ".git").mkdir(parents=True)

            with unittest.mock.patch.object(
                sync_mcp,
                "probe_git_remote_state",
                return_value={"status": "latest", "remote_head": "abc", "local_remote_head": "abc"},
            ):
                with unittest.mock.patch.object(sync_mcp, "repo_head_is_detached", return_value=False):
                    with unittest.mock.patch.object(sync_mcp, "run_cmd") as run_cmd:
                        repo, ok = sync_mcp.clone_or_update_repo(
                            "demo-mcp",
                            "https://example.invalid/repo.git",
                            base_dir,
                        )

        self.assertEqual(repo, repo_path)
        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_count, 0)

    def test_clone_or_update_repo_probe_failure_skips_pull_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            base_dir = Path(temp_dir)
            repo_path = base_dir / "demo-mcp"
            (repo_path / ".git").mkdir(parents=True)

            with unittest.mock.patch.object(sync_mcp, "probe_git_remote_state", return_value={"status": "unknown"}):
                with unittest.mock.patch.object(sync_mcp, "repo_head_is_detached", return_value=False):
                    with unittest.mock.patch.object(sync_mcp, "run_cmd") as run_cmd:
                        repo, ok = sync_mcp.clone_or_update_repo(
                            "demo-mcp",
                            "https://example.invalid/repo.git",
                            base_dir,
                        )

        self.assertEqual(repo, repo_path)
        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_count, 0)

    def test_clone_or_update_repo_probe_failure_force_refresh_runs_pull(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            base_dir = Path(temp_dir)
            repo_path = base_dir / "demo-mcp"
            (repo_path / ".git").mkdir(parents=True)

            with unittest.mock.patch.object(sync_mcp, "probe_git_remote_state", return_value={"status": "unknown"}):
                with unittest.mock.patch.object(sync_mcp, "repo_head_is_detached", return_value=False):
                    with unittest.mock.patch.object(sync_mcp, "run_cmd", return_value=(True, "updated")) as run_cmd:
                        repo, ok = sync_mcp.clone_or_update_repo(
                            "demo-mcp",
                            "https://example.invalid/repo.git",
                            base_dir,
                            force_refresh=True,
                        )

        self.assertEqual(repo, repo_path)
        self.assertTrue(ok)
        self.assertEqual(run_cmd.call_args_list, [unittest.mock.call(["git", "pull", "--ff-only"], cwd=repo_path)])

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

    def test_invalid_existing_json_dry_run_also_fails_closed(self):
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
                    "--dry-run",
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

    def test_mcp_sync_uses_mcp_env_without_terminal_env_override(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_dir = temp_root / "config"
            secrets_dir = temp_root / "secrets"
            config_dir.mkdir()
            secrets_dir.mkdir()
            agent_config_path = temp_root / "agent.json"
            config_path = config_dir / "manager.json"
            def_path = config_dir / "mcp.json"

            config_path.write_text(
                json.dumps({"mcp": {"grok-search": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "servers": {
                            "grok-search": {
                                "command": "uvx",
                                "args": ["grok-search"],
                                "env": {
                                    "GROK_API_URL": "${GROK_API_URL}",
                                    "GROK_API_KEY": "${GROK_API_KEY}",
                                },
                            }
                        },
                        "agent_paths": {
                            "claude-code": str(agent_config_path),
                        },
                    }
                ),
                encoding="utf-8",
            )
            (secrets_dir / "mcp.env").write_text(
                'GROK_API_URL="https://mcp.example/v1"\nGROK_API_KEY="mcp-key"\n',
                encoding="utf-8",
            )
            (secrets_dir / "terminal.env").write_text(
                'GROK_API_KEY="terminal-key"\n',
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
            env = updated["mcpServers"]["grok-search"]["env"]
            self.assertEqual(env["GROK_API_URL"], "https://mcp.example/v1")
            self.assertEqual(env["GROK_API_KEY"], "mcp-key")

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

    def test_manage_mcp_config_fails_cleanly_when_backup_is_not_writable(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            original = {
                "mcpServers": {
                    "user-mcp": {
                        "command": "echo",
                        "args": [],
                    }
                }
            }
            agent_config_path.write_text(json.dumps(original), encoding="utf-8")

            with (
                self.assertLogs(level="ERROR") as captured,
                mock.patch.object(
                    sync_mcp.shutil,
                    "copy2",
                    side_effect=PermissionError("Operation not permitted"),
                ),
            ):
                ok = sync_mcp.manage_mcp_config(
                    {"TmpAgent": str(agent_config_path)},
                    {
                        "demo": {
                            "enabled": True,
                            "_declared_in_policy": True,
                            "_sync_ready": True,
                            "command": "echo",
                            "args": [],
                            "env": {},
                        }
                    },
                )

            self.assertFalse(ok)
            self.assertEqual(json.loads(agent_config_path.read_text(encoding="utf-8")), original)
            self.assertIn("备份现有 MCP 配置失败", "\n".join(captured.output))

    def test_invalid_existing_json_mcp_section_aborts_without_overwrite(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            agent_config_path = temp_root / "agent.json"
            original = json.dumps({"mcpServers": []})
            agent_config_path.write_text(original, encoding="utf-8")

            ok = sync_mcp.manage_mcp_config(
                {"TmpAgent": str(agent_config_path)},
                {
                    "demo": {
                        "enabled": True,
                        "_declared_in_policy": True,
                        "_sync_ready": True,
                        "command": "echo",
                        "args": [],
                        "env": {},
                    }
                },
            )

            self.assertFalse(ok)
            self.assertEqual(agent_config_path.read_text(encoding="utf-8"), original)

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


class JsonWriteFailureIsolationTests(unittest.TestCase):
    """atomic_write_json re-raises, and manage_mcp_config loops over every agent in
    turn. An unwrapped failure on the first agent therefore aborted the whole pass,
    so the agents after it in AGENT_TARGETS silently got nothing that run — while the
    TOML branch had always accounted for its own failures per agent."""

    def test_one_agents_write_failure_does_not_skip_the_rest(self):
        with tempfile.TemporaryDirectory() as tmp:
            first = Path(tmp) / "first.json"
            second = Path(tmp) / "second.json"
            first.write_text("{}", encoding="utf-8")
            second.write_text("{}", encoding="utf-8")
            targets = {"claude-code": str(first), "antigravity-cli": str(second)}
            servers = {"demo": {"command": "demo", "args": [], "enabled": True}}
            written = []

            real_write = sync_mcp.atomic_write_json

            def flaky_write(path, data):
                if str(path) == str(first):
                    raise OSError("no space left on device")
                written.append(str(path))
                return real_write(path, data)

            with mock.patch.object(sync_mcp, "atomic_write_json", flaky_write), \
                 mock.patch.object(sync_mcp, "should_sync_mcp_target", lambda *a, **k: True), \
                 mock.patch.object(sync_mcp, "backup_config", lambda *a, **k: True):
                ok = sync_mcp.manage_mcp_config(targets, servers)

            self.assertFalse(ok, "a failed write must be reported, not swallowed")
            self.assertIn(str(second), written, "the next agent must still be written")
