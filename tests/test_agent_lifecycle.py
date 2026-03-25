import json
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from tests.agent_lifecycle_import import sync_agents
from tests.provider_path_fixtures import (
    posix_brew_cellar_module_path,
    posix_npm_global_module_path,
    windows_npm_bin_path,
    windows_npm_module_path,
)


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-lifecycle" / "sync_agents.py"


class DetectAllProvidersTests(unittest.TestCase):
    def test_detect_all_providers_delegates_to_runtime_probe_inventory(self):
        with mock.patch.object(sync_agents, "ProviderCache") as provider_cache_cls:
            with mock.patch.object(
                sync_agents,
                "detect_all_providers_runtime",
                return_value=["npm"],
                create=True,
            ) as runtime_detect:
                providers = sync_agents.detect_all_providers("claude", {"providers": {}})

        self.assertEqual(providers, ["npm"])
        runtime_detect.assert_called_once_with(
            "claude",
            {"providers": {}},
            inventory=provider_cache_cls.return_value,
        )

    def test_detects_npm_when_bin_resolves_into_node_modules(self):
        inventory = sync_agents.ProviderCache(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: "/usr/local/bin/claude" if binary == "claude" else None,
            path_exists=lambda _path: False,
            realpath=lambda _path: posix_npm_global_module_path("@anthropic-ai/claude-code", "bin/claude.js"),
        )

        with mock.patch.object(sync_agents, "ProviderCache", return_value=inventory):
            providers = sync_agents.detect_all_providers(
                "claude",
                {
                    "providers": {
                        "brew": {
                            "install": "brew install --cask claude-code",
                        }
                    }
                },
            )

        self.assertIn("npm", providers)

    def test_brew_managed_node_modules_path_is_not_misclassified_as_npm(self):
        command_outputs = {
            ("brew", "list", "--formula", "-1"): (True, "gemini-cli\n"),
            ("gh", "extension", "list"): (True, ""),
        }

        inventory = sync_agents.ProviderCache(
            run_cmd=lambda cmd, timeout=10: command_outputs.get(tuple(cmd), (False, "")),
            which=lambda binary: {
                "brew": "/usr/local/bin/brew",
                "gemini": "/usr/local/bin/gemini",
            }.get(binary),
            path_exists=lambda _path: False,
            realpath=lambda _path: posix_brew_cellar_module_path(
                "gemini-cli",
                "@google/gemini-cli",
                "dist/index.js",
                prefix="/usr/local",
            ),
        )

        with mock.patch.object(sync_agents, "ProviderCache", return_value=inventory):
            providers = sync_agents.detect_all_providers(
                "gemini",
                {
                    "providers": {
                        "brew": {
                            "install": "brew install gemini-cli",
                        }
                    }
                },
            )

        self.assertEqual(providers, ["brew"])

    def test_detects_npm_from_windows_npm_bin_path(self):
        inventory = sync_agents.ProviderCache(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: windows_npm_bin_path("claude") if binary == "claude" else None,
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        with mock.patch.object(sync_agents, "ProviderCache", return_value=inventory):
            providers = sync_agents.detect_all_providers(
                "claude",
                {
                    "providers": {
                        "npm": {
                            "install": "npm install -g @anthropic-ai/claude-code",
                        }
                    }
                },
            )

        self.assertIn("npm", providers)

    def test_detects_npm_when_windows_bin_resolves_into_node_modules(self):
        inventory = sync_agents.ProviderCache(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: (
                r"C:\Users\test\AppData\Local\Programs\Claude\claude.cmd" if binary == "claude" else None
            ),
            path_exists=lambda _path: False,
            realpath=lambda _path: windows_npm_module_path("@anthropic-ai/claude-code", "bin/claude.js"),
        )

        with mock.patch.object(sync_agents, "ProviderCache", return_value=inventory):
            providers = sync_agents.detect_all_providers(
                "claude",
                {
                    "providers": {
                        "npm": {
                            "install": "npm install -g @anthropic-ai/claude-code",
                        }
                    }
                },
            )

        self.assertIn("npm", providers)


class ReconcileAgentTests(unittest.TestCase):
    def test_no_install_mode_skips_missing_enabled_agent(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "npm": {
                    "install": "npm install -g @anthropic-ai/claude-code",
                }
            },
        }
        policy = {"enabled": True, "auto_install": True}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=[]),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="INFO") as logs,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy, allow_install=False)

        self.assertTrue(result)
        run_cmd.assert_not_called()
        self.assertTrue(any("禁止安装缺失 Agent" in line for line in logs.output), logs.output)

    def test_system_only_detection_is_audited_but_not_updated(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "npm": {
                    "upgrade": "npm update -g @anthropic-ai/claude-code",
                }
            },
        }
        policy = {"enabled": True, "auto_update": True}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=["system"]),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="INFO") as logs,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy)

        self.assertTrue(result)
        run_cmd.assert_not_called()
        self.assertTrue(any("仅检测到系统安装" in line for line in logs.output), logs.output)

    def test_invalid_preferred_provider_fails_loudly(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "npm": {
                    "install": "npm install -g @anthropic-ai/claude-code",
                }
            },
        }
        policy = {
            "enabled": True,
            "auto_install": True,
            "preferred_provider": "brew",
        }

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=[]),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="ERROR") as logs,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_not_called()
        self.assertTrue(any("preferred_provider" in line for line in logs.output), logs.output)

    def test_install_rechecks_post_condition(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "npm": {
                    "install": "npm install -g @anthropic-ai/claude-code",
                }
            },
        }
        policy = {"enabled": True, "auto_install": True}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", side_effect=[[], []]),
            mock.patch.object(sync_agents, "run_cmd", return_value=(True, "installed")) as run_cmd,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_called_once()

    def test_uninstall_rechecks_post_condition(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "npm": {
                    "uninstall": "npm uninstall -g @anthropic-ai/claude-code",
                }
            },
        }
        policy = {"enabled": False}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", side_effect=[["npm"], ["npm"]]),
            mock.patch.object(sync_agents, "run_cmd", return_value=(True, "removed")) as run_cmd,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_called_once()

    def test_upgrade_rechecks_post_condition(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "npm": {
                    "upgrade": "npm update -g @anthropic-ai/claude-code",
                }
            },
        }
        policy = {"enabled": True, "auto_update": True}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", side_effect=[["npm"], []]),
            mock.patch.object(sync_agents, "run_cmd", return_value=(True, "updated")) as run_cmd,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_called_once()

    def test_brew_upgrade_skips_when_outdated_probe_is_empty(self):
        definition = {
            "display_name": "OpenCode",
            "bin": "opencode",
            "providers": {
                "brew": {
                    "install": "brew install opencode",
                    "upgrade": "brew upgrade opencode",
                }
            },
        }
        policy = {"enabled": True, "auto_update": True, "preferred_provider": "brew"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=["brew"]),
            mock.patch.object(sync_agents, "run_cmd", return_value=(True, "")) as run_cmd,
            self.assertLogs(level="INFO") as logs,
        ):
            result = sync_agents.reconcile_agent("opencode", definition, policy)

        self.assertTrue(result)
        run_cmd.assert_called_once_with("brew outdated opencode", timeout=60)
        self.assertTrue(any("未发现可用更新" in line for line in logs.output), logs.output)

    def test_brew_cask_upgrade_runs_outdated_probe_before_upgrade(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "providers": {
                "brew": {
                    "install": "brew install --cask claude-code",
                    "upgrade": "brew upgrade --cask claude-code",
                }
            },
        }
        policy = {"enabled": True, "auto_update": True, "preferred_provider": "brew"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", side_effect=[["brew-cask"], ["brew-cask"]]),
            mock.patch.object(sync_agents, "run_cmd", side_effect=[(True, "claude-code"), (True, "updated")]) as run_cmd,
        ):
            result = sync_agents.reconcile_agent("claude-code", definition, policy)

        self.assertTrue(result)
        self.assertEqual(
            run_cmd.call_args_list,
            [
                mock.call("brew outdated --cask claude-code", timeout=60),
                mock.call("brew upgrade --cask claude-code"),
            ],
        )

    def test_brew_probe_failure_skips_direct_upgrade(self):
        definition = {
            "display_name": "OpenCode",
            "bin": "opencode",
            "providers": {
                "brew": {
                    "install": "brew install opencode",
                    "upgrade": "brew upgrade opencode",
                }
            },
        }
        policy = {"enabled": True, "auto_update": True, "preferred_provider": "brew"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=["brew"]),
            mock.patch.object(sync_agents, "run_cmd", return_value=(False, "brew locked")) as run_cmd,
            self.assertLogs(level="WARNING") as logs,
        ):
            result = sync_agents.reconcile_agent("opencode", definition, policy)

        self.assertTrue(result)
        run_cmd.assert_called_once_with("brew outdated opencode", timeout=60)
        self.assertTrue(any("更新探测失败，跳过升级" in line for line in logs.output), logs.output)


class PolicyOwnershipTests(unittest.TestCase):
    def test_direct_invocation_uses_repo_root_default_paths(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            repo_root = temp_root / "repo"
            config_dir = repo_root / "config"
            config_dir.mkdir(parents=True)
            shutil.copytree(ROOT / "src", repo_root / "src")
            config_dir.joinpath("manager.json").write_text(
                json.dumps({"agents": {}}),
                encoding="utf-8",
            )
            config_dir.joinpath("agents.json").write_text(
                json.dumps(
                    {
                        "mystery-agent": {
                            "display_name": "Mystery Agent",
                            "bin": "definitely-not-installed",
                            "providers": {},
                        }
                    }
                ),
                encoding="utf-8",
            )

            script_dir = repo_root / "agent-lifecycle"
            script_dir.mkdir(parents=True)
            script_path = script_dir / "sync_agents.py"
            script_path.write_text(MODULE_PATH.read_text(encoding="utf-8"), encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(script_path),
                    "--dry-run",
                    "--yes",
                ],
                cwd=temp_root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("Agent 状态对齐任务完成", result.stdout + result.stderr)

    def test_missing_policy_entries_are_ignored(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(json.dumps({"agents": {}}), encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "mystery-agent": {
                            "bin": "definitely-not-installed",
                            "providers": {
                                "shell": {
                                    "install": "command-that-should-never-run",
                                }
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
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("未声明策略", result.stdout + result.stderr)

    def test_invalid_policy_config_fails_loudly(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text("{ invalid json\n", encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "claude-code": {
                            "display_name": "Claude Code",
                            "bin": "claude",
                            "providers": {
                                "npm": {
                                    "install": "npm install -g @anthropic-ai/claude-code",
                                }
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

    def test_non_object_policy_section_fails_loudly(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text('{"agents": []}\n', encoding="utf-8")
            def_path.write_text(
                json.dumps(
                    {
                        "claude-code": {
                            "display_name": "Claude Code",
                            "bin": "claude",
                            "providers": {
                                "npm": {
                                    "install": "npm install -g @anthropic-ai/claude-code",
                                }
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
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            combined = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("config.json.agents", combined)
            self.assertIn("必须是对象", combined)

    def test_invalid_definition_file_fails_loudly(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps(
                    {
                        "agents": {
                            "claude-code": {
                                "enabled": True,
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            def_path.write_text("{ invalid json\n", encoding="utf-8")

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
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


class AgentIdConsistencyTests(unittest.TestCase):
    def test_config_uses_stable_agent_ids(self):
        manager_agents = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))["agents"]
        lifecycle_agents = json.loads((ROOT / "config" / "agents.json").read_text(encoding="utf-8"))
        skill_targets = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))["agents"]
        mcp_targets = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))["agent_paths"]

        managed_agent_ids = {
            "claude-code",
            "gemini-cli",
            "cursor",
            "github-copilot",
            "opencode",
        }

        self.assertEqual(set(manager_agents), managed_agent_ids)
        self.assertEqual(set(lifecycle_agents), managed_agent_ids)
        self.assertTrue(managed_agent_ids.issubset(set(skill_targets)))
        self.assertTrue(managed_agent_ids.issubset(set(mcp_targets)))
        self.assertIn("codex", skill_targets)
        self.assertIn("codex", mcp_targets)
