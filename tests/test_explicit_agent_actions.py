import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.agent_lifecycle_import import sync_agents


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-lifecycle" / "sync_agents.py"


def _write_executable(path: Path, body: str):
    path.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    path.chmod(0o755)


class ExplicitActionSandbox:
    """合成临时 bin/脚本 + 隔离 HOME/PATH 的自定义 Agent 沙盒（custom provider, managed=true）。"""

    def __init__(self, root: Path):
        self.root = root
        self.bindir = root / "bin"
        self.bindir.mkdir(parents=True)
        self.marker_install = root / "install.ran"
        self.marker_upgrade = root / "upgrade.ran"
        self.marker_uninstall = root / "uninstall.ran"

        bin_path = self.bindir / "myagent"
        _write_executable(
            root / "install.sh",
            "printf '#!/bin/sh\\necho 1.0.0\\n' > " + str(bin_path) + "\n"
            f"chmod +x {bin_path}\n"
            f"touch {self.marker_install}",
        )
        _write_executable(root / "upgrade.sh", f"touch {self.marker_upgrade}")
        _write_executable(
            root / "uninstall.sh",
            f"rm -f {bin_path}\ntouch {self.marker_uninstall}",
        )

        self.definition = {
            "display_name": "My Agent",
            "bin": "myagent",
            "preferred_provider": "custom",
            "providers": {
                "custom": {
                    "managed": True,
                    "install": str(root / "install.sh"),
                    "upgrade": str(root / "upgrade.sh"),
                    "uninstall": str(root / "uninstall.sh"),
                }
            },
        }
        self.def_file = root / "agents.json"
        self.def_file.write_text(json.dumps({"myagent": self.definition}), encoding="utf-8")
        self.policy_file = root / "manager.json"
        self.policy_file.write_text(json.dumps({"agents": {}}), encoding="utf-8")

    def install_fake_bin(self):
        _write_executable(self.bindir / "myagent", "echo 1.0.0")

    def env(self):
        env = os.environ.copy()
        env["HOME"] = str(self.root)
        env["PATH"] = os.pathsep.join([str(self.bindir), "/usr/bin", "/bin"])
        return env

    def run_cli(self, *extra_args):
        return subprocess.run(
            [
                sys.executable,
                str(MODULE_PATH),
                "--config",
                str(self.policy_file),
                "--def-file",
                str(self.def_file),
                *extra_args,
            ],
            env=self.env(),
            capture_output=True,
            text=True,
            timeout=120,
        )


class ExplicitActionCliTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.sandbox = ExplicitActionSandbox(Path(self._tmp.name))

    def test_install_runs_command_and_rechecks_detection(self):
        result = self.sandbox.run_cli("--action", "install", "--only", "myagent")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.sandbox.marker_install.exists())
        self.assertTrue((self.sandbox.bindir / "myagent").exists())

    def test_install_keeps_config_bytes_unchanged(self):
        before_def = self.sandbox.def_file.read_bytes()
        before_policy = self.sandbox.policy_file.read_bytes()
        result = self.sandbox.run_cli("--action", "install", "--only", "myagent")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.sandbox.def_file.read_bytes(), before_def)
        self.assertEqual(self.sandbox.policy_file.read_bytes(), before_policy)

    def test_install_refuses_when_already_installed_and_never_becomes_upgrade(self):
        self.sandbox.install_fake_bin()
        result = self.sandbox.run_cli("--action", "install", "--only", "myagent")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("不会变为升级", result.stderr)
        self.assertFalse(self.sandbox.marker_install.exists())
        self.assertFalse(self.sandbox.marker_upgrade.exists())

    def test_dry_run_parses_but_runs_no_shell_command(self):
        result = self.sandbox.run_cli("--action", "install", "--only", "myagent", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.sandbox.marker_install.exists())
        self.assertFalse((self.sandbox.bindir / "myagent").exists())

    def test_upgrade_uses_actual_managed_source(self):
        self.sandbox.install_fake_bin()
        result = self.sandbox.run_cli("--action", "upgrade", "--only", "myagent")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.sandbox.marker_upgrade.exists())
        self.assertFalse(self.sandbox.marker_install.exists())

    def test_upgrade_refuses_when_not_installed(self):
        result = self.sandbox.run_cli("--action", "upgrade", "--only", "myagent")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.sandbox.marker_upgrade.exists())

    def test_uninstall_requires_allow_flag(self):
        self.sandbox.install_fake_bin()
        result = self.sandbox.run_cli("--action", "uninstall", "--only", "myagent")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--allow-uninstall", result.stderr)
        self.assertTrue((self.sandbox.bindir / "myagent").exists())
        self.assertFalse(self.sandbox.marker_uninstall.exists())

    def test_uninstall_with_allow_flag_removes_bin_and_rechecks(self):
        self.sandbox.install_fake_bin()
        result = self.sandbox.run_cli(
            "--action", "uninstall", "--only", "myagent", "--allow-uninstall"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.sandbox.bindir / "myagent").exists())

    def test_offline_blocks_real_install_but_dry_run_previews(self):
        blocked = self.sandbox.run_cli("--action", "install", "--only", "myagent", "--offline")
        self.assertNotEqual(blocked.returncode, 0)
        self.assertFalse(self.sandbox.marker_install.exists())

        preview = self.sandbox.run_cli(
            "--action", "install", "--only", "myagent", "--offline", "--dry-run"
        )
        self.assertEqual(preview.returncode, 0, preview.stderr)
        self.assertFalse(self.sandbox.marker_install.exists())

    def test_action_requires_exactly_one_defined_agent(self):
        missing_only = self.sandbox.run_cli("--action", "install")
        self.assertEqual(missing_only.returncode, 2, missing_only.stderr)

        unknown = self.sandbox.run_cli("--action", "install", "--only", "ghost-agent")
        self.assertEqual(unknown.returncode, 2, unknown.stderr)

        multiple = self.sandbox.run_cli("--action", "install", "--only", "myagent", "--only", "myagent2")
        self.assertEqual(multiple.returncode, 2, multiple.stderr)

    def test_install_follows_effective_policy_preferred_provider(self):
        # 定义默认 custom，策略层把默认安装方式改为 custom2：CLI 应按 effective 策略选择
        sandbox = self.sandbox
        marker2 = sandbox.root / "install2.ran"
        bin_path = sandbox.bindir / "myagent"
        _write_executable(
            sandbox.root / "install2.sh",
            "printf '#!/bin/sh\\necho 1.0.0\\n' > " + str(bin_path) + "\n"
            f"chmod +x {bin_path}\n"
            f"touch {marker2}",
        )
        definition = json.loads(sandbox.def_file.read_text(encoding="utf-8"))
        definition["myagent"]["providers"]["custom2"] = {
            "managed": True,
            "install": str(sandbox.root / "install2.sh"),
        }
        sandbox.def_file.write_text(json.dumps(definition), encoding="utf-8")
        sandbox.policy_file.write_text(
            json.dumps({"agents": {"myagent": {"preferred_provider": "custom2"}}}),
            encoding="utf-8",
        )

        result = sandbox.run_cli("--action", "install", "--only", "myagent")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(marker2.exists())
        self.assertFalse(sandbox.marker_install.exists())


class ResolveExplicitActionTests(unittest.TestCase):
    """解析规则单测（mock 探测）：升级/卸载沿用唯一已检测受管来源，不按 preferred 迁移。"""

    def test_upgrade_follows_actual_brew_source_not_preferred_native(self):
        definition = {
            "display_name": "Claude Code",
            "bin": "claude",
            "preferred_provider": "native",
            "providers": {
                "native": {"managed": True, "upgrade": "claude update"},
                "brew": {
                    "managed": True,
                    "install": "brew install --cask claude-code",
                    "upgrade": "brew upgrade --cask claude-code",
                },
            },
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=["brew-cask"]):
            provider, command = sync_agents.resolve_explicit_action("claude-code", definition, "upgrade")
        self.assertEqual(provider, "brew-cask")
        self.assertEqual(command, "brew upgrade --cask claude-code")

    def test_upgrade_refuses_multi_source_conflict(self):
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "providers": {
                "native": {"managed": True, "upgrade": "codex update"},
                "npm": {"managed": True, "upgrade": "npm install -g @openai/codex@latest"},
            },
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=["native", "npm"]):
            with self.assertRaisesRegex(ValueError, "多个安装来源"):
                sync_agents.resolve_explicit_action("codex", definition, "upgrade")

    def test_upgrade_refuses_managed_plus_system_mix(self):
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "providers": {"npm": {"managed": True, "upgrade": "npm install -g @openai/codex@latest"}},
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=["npm", "system"]):
            with self.assertRaisesRegex(ValueError, "多个安装来源"):
                sync_agents.resolve_explicit_action("codex", definition, "upgrade")

    def test_uninstall_refuses_managed_plus_unsupported_mix(self):
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "providers": {
                "native": {"managed": True, "uninstall": "uninstall.sh"},
            },
        }
        with mock.patch.object(
            sync_agents, "detect_all_providers", return_value=["native", "ghost-source"]
        ):
            with self.assertRaisesRegex(ValueError, "多个安装来源"):
                sync_agents.resolve_explicit_action("codex", definition, "uninstall")

    def test_upgrade_refuses_unknown_external_source(self):
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "providers": {"npm": {"managed": True, "upgrade": "npm install -g @openai/codex@latest"}},
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=["system"]):
            with self.assertRaisesRegex(ValueError, "无受管 Provider"):
                sync_agents.resolve_explicit_action("codex", definition, "upgrade")

    def test_uninstall_refuses_when_command_missing(self):
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "providers": {"npm": {"managed": True, "upgrade": "npm install -g @openai/codex@latest"}},
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=["npm"]):
            with self.assertRaisesRegex(ValueError, "缺少 uninstall 命令"):
                sync_agents.resolve_explicit_action("codex", definition, "uninstall")

    def test_action_refused_when_officially_unsupported(self):
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "providers": {
                "native": {
                    "managed": True,
                    "install": "install.sh",
                    "unsupported_actions": ["uninstall"],
                }
            },
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=["native"]):
            with self.assertRaisesRegex(ValueError, "官方当前未提供受管 uninstall"):
                sync_agents.resolve_explicit_action("codex", definition, "uninstall")

    def test_explicit_actions_ignore_auto_policy(self):
        # resolve_explicit_action 不接收 policy，天然不受 auto_install/auto_update 影响；
        # 此处钉住执行路径也不读取 policy：auto_install=False 的缺失定义仍可显式安装。
        definition = {
            "display_name": "Codex",
            "bin": "codex",
            "preferred_provider": "native",
            "providers": {"native": {"managed": True, "install": "install.sh"}},
        }
        with mock.patch.object(sync_agents, "detect_all_providers", return_value=[]):
            provider, command = sync_agents.resolve_explicit_action("codex", definition, "install")
        self.assertEqual((provider, command), ("native", "install.sh"))


class EngineAvailableActionsTests(unittest.TestCase):
    """engine_v1._agent_available_actions：状态动作按可执行性产生（复用 resolver 规则，不重复探测）。"""

    @staticmethod
    def _definition(**provider_extra):
        cfg = {"managed": True, "install": "install.sh", "upgrade": "upgrade.sh", "uninstall": "uninstall.sh"}
        cfg.update(provider_extra)
        return {
            "display_name": "My Agent",
            "bin": "myagent",
            "preferred_provider": "custom",
            "providers": {"custom": cfg},
        }

    def _ids(self, definition, policy, state):
        from src import engine_v1

        return [a["id"] for a in engine_v1._agent_available_actions("myagent", definition, policy, state)]

    def test_undeclared_policy_with_install_command_offers_install(self):
        state = {"all": [], "managed": [], "deprecated": [], "system_only": [], "unsupported": []}
        self.assertEqual(self._ids(self._definition(), None, state), ["install"])

    def test_disabled_uninstalled_agent_still_offers_install(self):
        state = {"all": [], "managed": [], "deprecated": [], "system_only": [], "unsupported": []}
        self.assertEqual(self._ids(self._definition(), {"enabled": False}, state), ["install"])

    def test_unique_managed_source_offers_upgrade_and_uninstall(self):
        state = {"all": ["custom"], "managed": ["custom"], "deprecated": [], "system_only": [], "unsupported": []}
        self.assertEqual(self._ids(self._definition(), {"enabled": True}, state), ["upgrade", "uninstall"])

    def test_missing_uninstall_command_hides_uninstall(self):
        state = {"all": ["custom"], "managed": ["custom"], "deprecated": [], "system_only": [], "unsupported": []}
        definition = self._definition()
        self.assertEqual(self._ids(definition, {"enabled": True}, state), ["upgrade", "uninstall"])
        del definition["providers"]["custom"]["uninstall"]
        self.assertEqual(self._ids(definition, {"enabled": True}, state), ["upgrade"])

    def test_multi_source_mix_offers_nothing(self):
        state = {"all": ["custom", "system"], "managed": ["custom"], "deprecated": [], "system_only": ["system"], "unsupported": []}
        self.assertEqual(self._ids(self._definition(), {"enabled": True}, state), [])

    def test_external_unknown_source_offers_nothing(self):
        state = {"all": ["system"], "managed": [], "deprecated": [], "system_only": ["system"], "unsupported": []}
        self.assertEqual(self._ids(self._definition(), {"enabled": True}, state), [])

    def test_preferred_not_declared_hides_install(self):
        definition = self._definition()
        definition["preferred_provider"] = "ghost"
        state = {"all": [], "managed": [], "deprecated": [], "system_only": [], "unsupported": []}
        self.assertEqual(self._ids(definition, None, state), [])


if __name__ == "__main__":
    unittest.main()
