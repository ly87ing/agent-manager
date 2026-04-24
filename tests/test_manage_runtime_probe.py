import os
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
    def test_resolve_provider_command_prefers_darwin_over_posix_and_default(self):
        command = {
            "default": "installer default",
            "posix": "installer posix",
            "darwin": "installer darwin",
        }

        self.assertEqual(
            manage_runtime_probe.resolve_provider_command(command, platform_name="darwin"),
            "installer darwin",
        )

    def test_resolve_provider_command_uses_windows_entry_on_win32(self):
        command = {
            "default": "installer default",
            "windows": "installer windows",
        }

        self.assertEqual(
            manage_runtime_probe.resolve_provider_command(command, platform_name="win32"),
            "installer windows",
        )

    def test_iter_command_heads_ignores_shell_operators_inside_quotes(self):
        self.assertEqual(
            manage_runtime_probe.iter_command_heads('python3 -c \'print("a&&b")\''),
            ["python3"],
        )

    def test_iter_command_heads_skips_leading_env_assignments(self):
        self.assertEqual(
            manage_runtime_probe.iter_command_heads("FOO=bar git status"),
            ["git"],
        )

    def test_iter_command_heads_skips_env_wrapper(self):
        self.assertEqual(
            manage_runtime_probe.iter_command_heads("env FOO=bar git status"),
            ["git"],
        )

    def test_iter_command_heads_skips_command_wrapper(self):
        self.assertEqual(
            manage_runtime_probe.iter_command_heads("command git status"),
            ["git"],
        )

    def test_iter_command_heads_skips_command_wrapper_flags(self):
        self.assertEqual(
            manage_runtime_probe.iter_command_heads("command -v git >/dev/null || brew install git"),
            ["git", "brew"],
        )

    def test_rule_target_agent_id_maps_kimi_prefix_to_kimi_cli(self):
        self.assertEqual(manage_runtime_probe.rule_target_agent_id("kimi_global"), "kimi-cli")
        self.assertEqual(manage_runtime_probe.rule_target_agent_id("kimi_modules"), "kimi-cli")

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

    def test_detect_tool_readiness_requires_setup_marker_for_install_based_tools(self):
        def path_exists(path):
            return False

        with mock.patch("src.manage_runtime_probe.os.path.exists", side_effect=path_exists):
            with mock.patch("src.manage_runtime_probe.shutil.which", return_value="/usr/local/bin/agent-browser"):
                ready = manage_runtime_probe.detect_tool_readiness(
                    "agent-browser",
                    {
                        "install": "npm install -g agent-browser@latest",
                        "setup": "agent-browser install",
                        "ready_check": "bin:agent-browser",
                    },
                    "/repo",
                )

        self.assertFalse(ready)

    def test_detect_tool_readiness_accepts_setup_marker_for_install_based_tools(self):
        def path_exists(path):
            return path.endswith("agent-tool/agent-browser/.agent-manager-ready")

        with mock.patch("src.manage_runtime_probe.os.path.exists", side_effect=path_exists):
            with mock.patch("src.manage_runtime_probe.shutil.which", return_value="/usr/local/bin/agent-browser"):
                ready = manage_runtime_probe.detect_tool_readiness(
                    "agent-browser",
                    {
                        "install": "npm install -g agent-browser@latest",
                        "setup": "agent-browser install",
                        "ready_check": "bin:agent-browser",
                    },
                    "/repo",
                )

        self.assertTrue(ready)

    def test_detect_tool_readiness_accepts_runtime_ready_checks_without_setup_marker(self):
        with mock.patch("src.manage_runtime_probe.os.path.exists", return_value=False):
            with mock.patch(
                "src.manage_runtime_probe.check_tool_ready",
                side_effect=[True, True],
            ):
                ready = manage_runtime_probe.detect_tool_readiness(
                    "agent-browser",
                    {
                        "install": "npm install -g agent-browser@latest",
                        "setup": "agent-browser install",
                        "ready_check": "bin:agent-browser",
                        "runtime_ready_checks_any": ["app:/Applications/Google Chrome.app"],
                    },
                    "/repo",
                )

        self.assertTrue(ready)

    def test_detect_tool_readiness_requires_runtime_glob_match_when_probe_is_configured(self):
        def path_exists(path):
            return path.endswith("playwright-cli/.agent-manager-ready")

        with mock.patch("src.manage_runtime_probe.os.path.exists", side_effect=path_exists):
            with mock.patch("src.manage_runtime_probe.check_tool_ready", return_value=True):
                with mock.patch("src.manage_runtime_probe.glob.glob", return_value=[]):
                    ready = manage_runtime_probe.detect_tool_readiness(
                        "playwright-cli",
                        {
                            "install": "npm install -g @playwright/cli@latest",
                            "setup": "playwright-cli install-browser",
                            "ready_check": "bin:playwright-cli",
                            "runtime_ready_globs_any": ["~/Library/Caches/ms-playwright/chromium-*"],
                        },
                        "/repo",
                    )

        self.assertFalse(ready)

    def test_detect_tool_readiness_accepts_runtime_glob_match_for_install_based_tools(self):
        with mock.patch("src.manage_runtime_probe.os.path.exists", return_value=False):
            with mock.patch("src.manage_runtime_probe.check_tool_ready", return_value=True):
                with mock.patch(
                    "src.manage_runtime_probe.glob.glob",
                    return_value=["/Users/example/Library/Caches/ms-playwright/chromium-1219"],
                ):
                    ready = manage_runtime_probe.detect_tool_readiness(
                        "playwright-cli",
                        {
                            "install": "npm install -g @playwright/cli@latest",
                            "setup": "playwright-cli install-browser",
                            "ready_check": "bin:playwright-cli",
                            "runtime_ready_globs_any": ["~/Library/Caches/ms-playwright/chromium-*"],
                        },
                        "/repo",
                    )

        self.assertTrue(ready)

    def test_detect_tool_readiness_uses_ready_check_for_app_type_tools(self):
        with mock.patch("src.manage_runtime_probe.os.path.exists", return_value=True):
            ready = manage_runtime_probe.detect_tool_readiness(
                "iterm2",
                {
                    "install": "brew install --cask iterm2",
                    "ready_check": "app:/Applications/iTerm.app",
                },
                "/repo",
            )

        self.assertTrue(ready)

    def test_detect_tool_readiness_ready_check_takes_precedence_over_which(self):
        with mock.patch("src.manage_runtime_probe.os.path.exists", return_value=False):
            with mock.patch("src.manage_runtime_probe.shutil.which", return_value="/usr/bin/iterm2"):
                ready = manage_runtime_probe.detect_tool_readiness(
                    "iterm2",
                    {
                        "install": "brew install --cask iterm2",
                        "ready_check": "app:/Applications/iTerm.app",
                    },
                    "/repo",
                )

        self.assertFalse(ready)

    def test_detect_tool_readiness_reuses_brew_inventory_across_multiple_tools(self):
        calls = []

        def run_cmd(cmd, timeout=10):
            calls.append(tuple(cmd))
            if tuple(cmd) == ("brew", "list", "--formula", "-1"):
                return True, "powerlevel10k\nzsh-autosuggestions\nzsh-syntax-highlighting\n"
            if tuple(cmd) == ("brew", "list", "--cask", "-1"):
                return True, ""
            raise AssertionError(f"unexpected command: {cmd}")

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=run_cmd,
            which=lambda binary: "/opt/homebrew/bin/brew" if binary == "brew" else None,
        )

        self.assertTrue(
            manage_runtime_probe.detect_tool_readiness(
                "powerlevel10k",
                {"ready_check": "brew:powerlevel10k"},
                "/repo",
                inventory=inventory,
            )
        )
        self.assertTrue(
            manage_runtime_probe.detect_tool_readiness(
                "zsh-autosuggestions",
                {"ready_check": "brew:zsh-autosuggestions"},
                "/repo",
                inventory=inventory,
            )
        )
        self.assertTrue(
            manage_runtime_probe.detect_tool_readiness(
                "zsh-syntax-highlighting",
                {"ready_check": "brew:zsh-syntax-highlighting"},
                "/repo",
                inventory=inventory,
            )
        )

        self.assertEqual(calls.count(("brew", "list", "--formula", "-1")), 1)
        self.assertLessEqual(calls.count(("brew", "list", "--cask", "-1")), 1)
        self.assertNotIn(("brew", "list", "--versions", "powerlevel10k"), calls)
        self.assertNotIn(("brew", "list", "--versions", "zsh-autosuggestions"), calls)
        self.assertNotIn(("brew", "list", "--versions", "zsh-syntax-highlighting"), calls)

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

    def test_collect_required_runtime_commands_includes_npx_for_active_chrome_devtools_mcp(self):
        required = manage_runtime_probe.collect_required_runtime_commands(
            {
                "agents": {},
                "tools": {},
                "skills": {},
                "mcp": {"chrome-devtools": {"enabled": True}},
            },
            {},
            {},
            {"repositories": {}},
            {
                "servers": {
                    "chrome-devtools": {
                        "command": "npx",
                        "args": ["-y", "chrome-devtools-mcp@latest", "--isolated"],
                    }
                }
            },
        )

        self.assertEqual(required, ["npx"])

    def test_collect_required_runtime_commands_skips_unmanaged_agent_providers_and_resolves_commands(self):
        required = manage_runtime_probe.collect_required_runtime_commands(
            {
                "agents": {"claude-code": {"enabled": True}},
                "tools": {},
                "skills": {},
                "mcp": {},
            },
            {
                "claude-code": {
                    "providers": {
                        "native": {
                            "managed": True,
                            "install": {
                                "default": "curl -fsSL https://example.invalid/install.sh | bash",
                                "darwin": "curl -fsSL https://example.invalid/install.sh | bash",
                            },
                            "upgrade": {
                                "default": "claude update",
                                "darwin": "claude update",
                            },
                        },
                        "system": {
                            "managed": False,
                            "install": "external-installer bootstrap",
                            "upgrade": "external-installer update",
                        },
                    }
                }
            },
            {},
            {"repositories": {}},
            {"servers": {}},
        )

        self.assertEqual(required, ["curl", "bash", "claude"])

    def test_collect_required_runtime_commands_ignores_windows_transient_installer_heads(self):
        with mock.patch("src.manage_runtime_probe.sys.platform", "win32"):
            required = manage_runtime_probe.collect_required_runtime_commands(
                {
                    "agents": {"claude-code": {"enabled": True}},
                    "tools": {},
                    "skills": {},
                    "mcp": {},
                },
                {
                    "claude-code": {
                        "providers": {
                            "native": {
                                "managed": True,
                                "install": {
                                    "windows": (
                                        "curl -fsSL https://example.invalid/install.cmd -o install.cmd "
                                        "&& install.cmd && del install.cmd"
                                    ),
                                },
                                "upgrade": {
                                    "windows": "claude update",
                                },
                            }
                        }
                    }
                },
                {},
                {"repositories": {}},
                {"servers": {}},
            )

        self.assertEqual(required, ["curl", "claude"])

    def test_collect_required_runtime_commands_skips_tool_owned_setup_heads_for_unready_install_tool(self):
        with mock.patch("src.manage_runtime_probe.detect_tool_readiness", return_value=False):
            required = manage_runtime_probe.collect_required_runtime_commands(
                {
                    "agents": {},
                    "tools": {"agent-browser": {"enabled": True, "auto_update": False}},
                    "skills": {},
                    "mcp": {},
                },
                {},
                {
                    "agent-browser": {
                        "install": "npm install -g agent-browser@latest",
                        "setup": "agent-browser install",
                        "ready_check": "bin:agent-browser",
                    }
                },
                {"repositories": {}},
                {"servers": {}},
                repo_root="/repo",
            )

        self.assertEqual(required, ["npm"])

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

    def test_detect_all_providers_matches_declared_native_paths_before_system_fallback(self):
        native_path = os.path.expanduser("~/.local/bin/claude")
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: native_path if binary == "claude" else None,
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "claude",
            {
                "providers": {
                    "native": {
                        "managed": True,
                        "detect_paths": ["~/.local/bin/claude"],
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["native"])

    def test_detect_all_providers_prefers_declared_native_path_over_npm_realpath(self):
        native_path = os.path.expanduser("~/.local/bin/claude")
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: native_path if binary == "claude" else None,
            path_exists=lambda _path: False,
            realpath=lambda _path: posix_npm_global_module_path("@anthropic-ai/claude-code", "bin/claude.js"),
        )

        providers = manage_runtime_probe.detect_all_providers(
            "claude",
            {
                "providers": {
                    "native": {
                        "managed": True,
                        "detect_paths": ["~/.local/bin/claude"],
                    },
                    "npm": {
                        "install": "npm install -g @anthropic-ai/claude-code",
                    },
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["native", "npm"])

    def test_detect_all_providers_detects_uv_tool_installation(self):
        command_outputs = {
            ("uv", "tool", "list"): (True, "kimi-cli 0.2.0\n"),
        }

        def run_cmd(cmd, timeout=10):
            return command_outputs.get(tuple(cmd), (False, ""))

        def which(binary):
            mapping = {
                "uv": "/opt/homebrew/bin/uv",
                "kimi": "/opt/homebrew/bin/kimi",
            }
            return mapping.get(binary)

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=run_cmd,
            which=which,
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "kimi",
            {
                "providers": {
                    "native": {
                        "detect_uv_tool": "kimi-cli",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["native"])

    def test_detect_all_providers_detects_uv_tool_installation_with_trailing_python_flag(self):
        command_outputs = {
            ("uv", "tool", "list"): (True, "kimi-cli 0.2.0\n"),
        }

        def run_cmd(cmd, timeout=10):
            return command_outputs.get(tuple(cmd), (False, ""))

        def which(binary):
            mapping = {
                "uv": "/opt/homebrew/bin/uv",
                "kimi": "/opt/homebrew/bin/kimi",
            }
            return mapping.get(binary)

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=run_cmd,
            which=which,
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "kimi",
            {
                "providers": {
                    "native": {
                        "install": "curl -LsSf https://code.kimi.com/install.sh | bash",
                        "detect_uv_tool": "kimi-cli",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["native"])

    def test_detect_all_providers_matches_opencode_native_detect_paths(self):
        native_path = os.path.expanduser("~/.local/bin/opencode")
        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: (False, ""),
            which=lambda binary: native_path if binary == "opencode" else None,
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "opencode",
            {
                "providers": {
                    "native": {
                        "managed": True,
                        "detect_paths": [
                            "~/.local/bin/opencode",
                            "~/bin/opencode",
                            "~/.opencode/bin/opencode",
                        ],
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["native"])

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
