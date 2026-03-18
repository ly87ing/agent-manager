import unittest
from unittest import mock

from src import manage_runtime_probe


class ManageRuntimeProbeTests(unittest.TestCase):
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
                "claude": "/usr/local/lib/node_modules/@anthropic-ai/claude-code/bin/claude.js",
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

    def test_detect_all_providers_does_not_infer_npm_from_resolved_symlink_only(self):
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
            realpath=lambda _path: "/opt/homebrew/lib/node_modules/@google/gemini-cli/bin/gemini.js",
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
