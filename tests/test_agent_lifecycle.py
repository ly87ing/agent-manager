import json
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from agent_lifecycle_import import sync_agents


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-lifecycle" / "sync_agents.py"


class DetectAllProvidersTests(unittest.TestCase):
    @mock.patch.object(sync_agents, "ProviderCache")
    @mock.patch.object(sync_agents.os.path, "exists", return_value=False)
    @mock.patch.object(sync_agents.os.path, "realpath")
    @mock.patch.object(sync_agents.shutil, "which", return_value="/usr/local/bin/claude")
    def test_detects_npm_when_bin_resolves_into_node_modules(
        self,
        _which,
        realpath,
        _exists,
        provider_cache_cls,
    ):
        cache = provider_cache_cls.return_value
        cache.get_gh_extensions.return_value = ""
        cache.get_brew_formulae.return_value = set()
        cache.get_brew_casks.return_value = set()
        realpath.return_value = "/usr/local/lib/node_modules/@anthropic-ai/claude-code/bin/claude.js"

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


class ReconcileAgentTests(unittest.TestCase):
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


class PolicyOwnershipTests(unittest.TestCase):
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
