"""自定义（custom）provider 探测与真实生命周期闭环测试。

契约：providers.custom.managed === true 且 bin 确实可执行、且没有任何
npm/brew/gh/uv/detect_paths 来源时才归 custom；其余仍归 system。生命周期
只认用户填写的 install/upgrade/uninstall 命令，缺 uninstall 必须拒绝。
所有真实执行均在临时目录中的合成可执行脚本上进行，不触碰用户真实安装。
"""

import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from src import manage_runtime_probe
from tests.agent_lifecycle_import import sync_agents


CUSTOM_DEFINITION = {
    "bin": "custom-agent",
    "preferred_provider": "custom",
    "providers": {
        "custom": {
            "managed": True,
            "install": "placeholder",
            "upgrade": "placeholder",
            "uninstall": "placeholder",
        }
    },
}


def _inventory(which_map=None, path_exists=lambda _p: False, realpath=lambda p: p):
    which_map = which_map or {}
    return manage_runtime_probe.ProviderInventory(
        run_cmd=lambda cmd, timeout=10: (False, ""),
        which=lambda binary: which_map.get(binary),
        path_exists=path_exists,
        realpath=realpath,
    )


class CustomProviderDetectionTests(unittest.TestCase):
    def detect(self, config, inventory):
        return manage_runtime_probe.detect_all_providers("custom-agent", config, inventory=inventory)

    def test_custom_managed_with_executable_bin_is_classified_custom(self):
        inventory = _inventory({"custom-agent": "/tmp/fake-bin/custom-agent"})
        providers = self.detect(CUSTOM_DEFINITION, inventory)
        self.assertEqual(providers, ["custom"])

    def test_custom_managed_default_falls_back_to_system(self):
        config = {
            "bin": "custom-agent",
            "providers": {"custom": {"install": "placeholder"}},
        }
        inventory = _inventory({"custom-agent": "/tmp/fake-bin/custom-agent"})
        self.assertEqual(self.detect(config, inventory), ["system"])

    def test_custom_managed_false_falls_back_to_system(self):
        config = {
            "providers": {"custom": {"managed": False, "install": "placeholder"}},
        }
        inventory = _inventory({"custom-agent": "/tmp/fake-bin/custom-agent"})
        self.assertEqual(self.detect(config, inventory), ["system"])

    def test_custom_managed_non_boolean_falls_back_to_system(self):
        config = {
            "providers": {"custom": {"managed": "yes", "install": "placeholder"}},
        }
        inventory = _inventory({"custom-agent": "/tmp/fake-bin/custom-agent"})
        self.assertEqual(self.detect(config, inventory), ["system"])

    def test_custom_managed_without_bin_is_not_custom(self):
        config = dict(CUSTOM_DEFINITION)
        config["app_path"] = "/Applications/CustomAgent.app"
        inventory = _inventory({}, path_exists=lambda p: p == "/Applications/CustomAgent.app")
        self.assertEqual(self.detect(config, inventory), ["system"])

    def test_no_bin_no_app_path_detects_nothing(self):
        inventory = _inventory({})
        self.assertEqual(self.detect(CUSTOM_DEFINITION, inventory), [])

    def test_recognized_detect_paths_source_is_not_swallowed_by_custom(self):
        inventory = _inventory({"custom-agent": "/tmp/fake-bin/custom-agent"})
        config = {
            "providers": {
                "native": {"managed": True, "detect_paths": ["/tmp/fake-bin/custom-agent"]},
                "custom": {"managed": True, "install": "placeholder"},
            }
        }
        self.assertEqual(self.detect(config, inventory), ["native"])

    def test_recognized_npm_source_is_not_swallowed_by_custom(self):
        inventory = _inventory(
            {"custom-agent": "/usr/local/lib/node_modules/.bin/custom-agent"},
        )
        self.assertEqual(self.detect(CUSTOM_DEFINITION, inventory), ["npm"])

    def test_custom_classified_as_managed_in_provider_state(self):
        state = manage_runtime_probe.classify_provider_state(["custom"], CUSTOM_DEFINITION)
        self.assertEqual(state["managed"], ["custom"])
        self.assertEqual(state["unsupported"], [])


class CustomProviderLifecycleTests(unittest.TestCase):
    """真实执行 reconcile_agent，bin 为临时目录中的合成可执行脚本。"""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.bin_path = self.bin_dir / "custom-agent"
        self._path_patch = mock.patch.dict(
            os.environ,
            {"PATH": str(self.bin_dir) + os.pathsep + os.environ.get("PATH", "")},
        )
        self._path_patch.start()
        self.addCleanup(self._path_patch.stop)

    def write_bin(self, version):
        self.bin_path.write_text(
            "#!/bin/sh\n"
            'if [ "$1" = "--version" ]; then\n'
            f'    echo "custom-agent {version}"\n'
            "    exit 0\n"
            "fi\n"
            "exit 0\n",
            encoding="utf-8",
        )
        self.bin_path.chmod(self.bin_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    def install_cmd(self, version):
        return (
            f"printf '%s\\n' '#!/bin/sh' "
            f"'if [ \"$1\" = \"--version\" ]; then echo \"custom-agent {version}\"; exit 0; fi' "
            f"'exit 0' > {self.bin_path} && chmod +x {self.bin_path}"
        )

    def definition(self, **custom_overrides):
        custom = {
            "managed": True,
            "install": self.install_cmd("1.0.0"),
            "upgrade": self.install_cmd("2.0.0"),
            "uninstall": f"rm {self.bin_path}",
        }
        custom.update(custom_overrides)
        return {
            "bin": "custom-agent",
            "preferred_provider": "custom",
            "providers": {"custom": custom},
        }

    def read_version(self):
        result = subprocess.run(
            [str(self.bin_path), "--version"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        return result.stdout.strip()

    def install_first(self):
        self.assertTrue(
            sync_agents.reconcile_agent("custom-agent", self.definition(), {"enabled": True}),
            "首次安装必须成功",
        )
        self.assertTrue(self.bin_path.exists())

    def test_install_creates_bin_and_version_reads_back(self):
        self.assertFalse(self.bin_path.exists())
        self.install_first()
        self.assertEqual(self.read_version(), "custom-agent 1.0.0")
        state = sync_agents.detect_provider_state("custom-agent", self.definition())
        self.assertEqual(state["managed"], ["custom"])

    def test_install_fails_when_command_succeeds_but_bin_missing(self):
        definition = self.definition(install="true")
        self.assertFalse(sync_agents.reconcile_agent("custom-agent", definition, {"enabled": True}))
        self.assertFalse(self.bin_path.exists())

    def test_install_fails_on_nonzero_exit(self):
        definition = self.definition(install="false")
        self.assertFalse(sync_agents.reconcile_agent("custom-agent", definition, {"enabled": True}))
        self.assertFalse(self.bin_path.exists())

    def test_update_runs_user_upgrade_command_and_rechecks(self):
        self.install_first()
        self.assertTrue(
            sync_agents.reconcile_agent("custom-agent", self.definition(), {"enabled": True}),
            "重复升级必须遵循用户命令成功",
        )
        self.assertEqual(self.read_version(), "custom-agent 2.0.0")

    def test_uninstall_uses_user_command_and_rechecks(self):
        self.install_first()
        self.assertTrue(
            sync_agents.reconcile_agent("custom-agent", self.definition(), {"enabled": False}),
            "提供卸载命令时必须卸载成功",
        )
        self.assertFalse(self.bin_path.exists())
        state = sync_agents.detect_provider_state("custom-agent", self.definition())
        self.assertEqual(state["managed"], [])

    def test_missing_uninstall_command_refuses_without_deleting(self):
        self.install_first()
        definition = self.definition()
        del definition["providers"]["custom"]["uninstall"]
        self.assertFalse(
            sync_agents.reconcile_agent("custom-agent", definition, {"enabled": False}),
            "缺卸载命令必须拒绝，不能造 rm",
        )
        self.assertTrue(self.bin_path.exists(), "拒绝卸载时不得删除文件")

    def test_disabled_external_install_is_not_uninstalled(self):
        # managed 缺省（非显式 true）→ 归 system，视为外部安装，禁用时不卸载。
        self.write_bin("1.0.0")
        definition = {
            "bin": "custom-agent",
            "preferred_provider": "custom",
            "providers": {"custom": {"install": self.install_cmd("1.0.0")}},
        }
        self.assertTrue(sync_agents.reconcile_agent("custom-agent", definition, {"enabled": False}))
        self.assertTrue(self.bin_path.exists())


if __name__ == "__main__":
    unittest.main()
