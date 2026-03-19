import unittest
from unittest import mock

from src import manage_runtime_probe
from tests.provider_path_fixtures import (
    posix_brew_cellar_module_path,
    posix_npm_global_module_path,
    windows_npm_bin_path,
    windows_npm_module_path,
)


class ManageRuntimeProbeTests(unittest.TestCase):
    def test_detect_tool_readiness_requires_git_clone_for_repo_backed_tools(self):
        with mock.patch("src.manage_runtime_probe.os.path.isdir") as isdir:
            isdir.side_effect = lambda path: path == "/repo/agent-tool/playwright-cli"

            ready = manage_runtime_probe.detect_tool_readiness(
                "playwright-cli",
                {
                    "url": "https://github.com/microsoft/playwright-cli.git",
                    "setup": "npm install && npx playwright install",
                    "ready_paths": ["node_modules"],
                },
                "/repo",
            )

        self.assertFalse(ready)

    def test_detect_tool_readiness_requires_ready_paths_for_repo_backed_tools(self):
        with mock.patch("src.manage_runtime_probe.os.path.isdir", return_value=True):
            with mock.patch("src.manage_runtime_probe.os.path.exists", return_value=False):
                ready = manage_runtime_probe.detect_tool_readiness(
                    "playwright-cli",
                    {
                        "url": "https://github.com/microsoft/playwright-cli.git",
                        "setup": "npm install && npx playwright install",
                        "ready_paths": ["node_modules"],
                    },
                    "/repo",
                )

        self.assertFalse(ready)

    def test_detect_tool_readiness_accepts_backfilled_ready_paths(self):
        with mock.patch("src.manage_runtime_probe.os.path.isdir", return_value=True):
            with mock.patch("src.manage_runtime_probe.os.path.exists") as path_exists:
                path_exists.side_effect = lambda path: path.endswith("node_modules")
                ready = manage_runtime_probe.detect_tool_readiness(
                    "playwright-cli",
                    {
                        "url": "https://github.com/microsoft/playwright-cli.git",
                        "setup": "npm install && npx playwright install",
                        "ready_paths": ["node_modules"],
                    },
                    "/repo",
                )

        self.assertTrue(ready)

    def test_detect_tool_readiness_honors_broken_repo_marker(self):
        with mock.patch("src.manage_runtime_probe.os.path.isdir", return_value=True):
            with mock.patch("src.manage_runtime_probe.os.path.exists") as path_exists:
                path_exists.side_effect = lambda path: path.endswith(".agent-manager-broken") or path.endswith("node_modules")
                ready = manage_runtime_probe.detect_tool_readiness(
                    "playwright-cli",
                    {
                        "url": "https://github.com/microsoft/playwright-cli.git",
                        "setup": "npm install && npx playwright install",
                        "ready_paths": ["node_modules"],
                    },
                    "/repo",
                )

        self.assertFalse(ready)

    def test_detect_tool_readiness_does_not_trust_ready_marker_when_ready_paths_are_missing(self):
        with mock.patch("src.manage_runtime_probe.os.path.isdir", return_value=True):
            with mock.patch("src.manage_runtime_probe.os.path.exists") as path_exists:
                path_exists.side_effect = lambda path: path.endswith(".agent-manager-ready")
                ready = manage_runtime_probe.detect_tool_readiness(
                    "playwright-cli",
                    {
                        "url": "https://github.com/microsoft/playwright-cli.git",
                        "setup": "npm install && npx playwright install",
                        "ready_paths": ["node_modules"],
                    },
                    "/repo",
                )

        self.assertFalse(ready)

    def test_detect_tool_readiness_uses_binary_for_install_based_tools(self):
        with mock.patch("src.manage_runtime_probe.shutil.which", return_value="/usr/local/bin/auggie"):
            ready = manage_runtime_probe.detect_tool_readiness(
                "auggie",
                {"install": "npm install -g @augmentcode/auggie"},
                "/repo",
            )

        self.assertTrue(ready)

    def test_collect_required_runtime_commands_skips_node_for_empty_policy(self):
        required = manage_runtime_probe.collect_required_runtime_commands(
            {"agents": {}, "tools": {}, "skills": {}, "mcp": {}},
            {},
            {},
            {"repositories": {}},
            {"servers": {}},
        )

        self.assertEqual(required, [])

    def test_collect_required_runtime_commands_ignores_undeclared_skill_and_mcp_repos(self):
        required = manage_runtime_probe.collect_required_runtime_commands(
            {"agents": {}, "tools": {}, "skills": {}, "mcp": {}},
            {},
            {},
            {"repositories": {"repo1": {"url": "https://example.invalid/repo.git"}}},
            {"servers": {"srv1": {"url": "https://example.invalid/mcp.git"}}},
        )

        self.assertEqual(required, [])

    def test_collect_required_runtime_commands_skips_ready_repo_tool_setup_when_auto_update_disabled(self):
        with mock.patch("src.manage_runtime_probe.detect_tool_readiness", return_value=True):
            required = manage_runtime_probe.collect_required_runtime_commands(
                {
                    "agents": {},
                    "tools": {"playwright-cli": {"enabled": True, "auto_update": False}},
                    "skills": {},
                    "mcp": {},
                },
                {},
                {
                    "playwright-cli": {
                        "url": "https://github.com/microsoft/playwright-cli.git",
                        "setup": "npm install && npx playwright install",
                        "ready_paths": ["node_modules"],
                    }
                },
                {"repositories": {}},
                {"servers": {}},
                repo_root="/repo",
            )

        self.assertEqual(required, [])

    def test_detect_all_providers_supports_windows_npm_bin_path(self):
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: windows_npm_bin_path("claude") if binary == "claude" else None,
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "claude",
            {
                "providers": {
                    "npm": {
                        "install": "npm install -g @anthropic-ai/claude-code",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["npm"])

    def test_detect_all_providers_supports_windows_node_modules_path(self):
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: (
                windows_npm_module_path("@anthropic-ai/claude-code", "bin/claude.js")
                if binary == "claude"
                else None
            ),
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "claude",
            {
                "providers": {
                    "npm": {
                        "install": "npm install -g @anthropic-ai/claude-code",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["npm"])

    def test_detect_all_providers_matches_manage_shell_order(self):
        command_outputs = {
            ("brew", "list", "--formula", "-1"): (True, ""),
            ("brew", "list", "--cask", "-1"): (True, "claude-code\n"),
            ("gh", "extension", "list"): (True, "owner/claude-ext 1.0.0\n"),
        }

        def run_cmd(cmd, timeout=10):
            return command_outputs.get(tuple(cmd), (False, ""))

        def which(binary):
            mapping = {
                "brew": "/opt/homebrew/bin/brew",
                "gh": "/opt/homebrew/bin/gh",
                "claude": posix_npm_global_module_path("@anthropic-ai/claude-code", "bin/claude.js"),
            }
            return mapping.get(binary)

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=run_cmd,
            which=which,
            path_exists=lambda _path: False,
            realpath=lambda _path: _path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "claude",
            {
                "providers": {
                    "brew": {
                        "install": "brew install --cask claude-code",
                    }
                },
                "ext_name": "owner/claude-ext",
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["brew-cask", "npm", "gh"])

    def test_detect_all_providers_infers_npm_from_non_brew_resolved_symlink(self):
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: {
                "claude": "/usr/local/bin/claude",
            }.get(binary),
            path_exists=lambda _path: False,
            realpath=lambda _path: posix_npm_global_module_path("@anthropic-ai/claude-code", "bin/claude.js"),
        )

        providers = manage_runtime_probe.detect_all_providers(
            "claude",
            {
                "providers": {
                    "npm": {
                        "install": "npm install -g @anthropic-ai/claude-code",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["npm"])

    def test_detect_all_providers_does_not_infer_npm_from_brew_managed_resolved_symlink(self):
        command_outputs = {
            ("brew", "list", "--formula", "-1"): (True, "gemini-cli\n"),
            ("gh", "extension", "list"): (True, ""),
        }

        def run_cmd(cmd, timeout=10):
            return command_outputs.get(tuple(cmd), (False, ""))

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=run_cmd,
            which=lambda binary: {
                "brew": "/opt/homebrew/bin/brew",
                "gemini": "/opt/homebrew/bin/gemini",
            }.get(binary),
            path_exists=lambda _path: False,
            realpath=lambda _path: posix_brew_cellar_module_path(
                "gemini-cli",
                "@google/gemini-cli",
                "bin/gemini.js",
            ),
        )

        providers = manage_runtime_probe.detect_all_providers(
            "gemini",
            {
                "providers": {
                    "brew": {
                        "install": "brew install gemini-cli",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["brew"])

    def test_detect_all_providers_falls_back_to_system_when_binary_exists(self):
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: "/Applications/Cursor.app/Contents/MacOS/Cursor" if binary == "cursor" else None,
            path_exists=lambda path: path == "/Applications/Cursor.app",
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "cursor",
            {
                "app_path": "/Applications/Cursor.app",
                "providers": {},
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["system"])

    def test_inventory_caches_provider_queries(self):
        run_cmd = mock.Mock(
            side_effect=[
                (True, "claude-code\n"),
                (True, ""),
                (True, ""),
            ]
        )

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=run_cmd,
            which=lambda binary: "/opt/homebrew/bin/brew" if binary == "brew" else None,
        )

        self.assertEqual(inventory.get_brew_formulae(), {"claude-code"})
        self.assertEqual(inventory.get_brew_formulae(), {"claude-code"})

        run_cmd.assert_called_once_with(["brew", "list", "--formula", "-1"], timeout=10)
