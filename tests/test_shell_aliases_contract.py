import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / "config" / "shell-aliases.json"
SYNC = ROOT / "scripts" / "claude" / "sync_shell_aliases.sh"


class ShellAliasesContractTests(unittest.TestCase):
    """时区与 locale 是单一数据源:alias 模板只写占位符,真实值提取到顶层
    `timezone`/`lang` 字段,由 sync_shell_aliases.sh 替换。这两条契约防止有人
    把值硬编码回 alias 模板(退化)或删掉生成器里的替换逻辑(占位符漏出)。"""

    def test_alias_templates_use_placeholders_not_hardcoded_locale(self):
        cfg = json.loads(CONFIG.read_text(encoding="utf-8"))
        self.assertEqual(cfg.get("timezone"), "America/Los_Angeles")
        self.assertEqual(cfg.get("lang"), "en_US.UTF-8")
        for name, tmpl in cfg["aliases"].items():
            # 每条 alias 引用占位符,而非把值写死
            self.assertIn("{timezone}", tmpl, f"{name} 应用 {{timezone}} 占位符")
            self.assertIn("{lang}", tmpl, f"{name} 应用 {{lang}} 占位符")
            # 值不得回退成硬编码字面量
            self.assertNotIn("America/Los_Angeles", tmpl, f"{name} 硬编码了时区")
            self.assertNotIn("en_US.UTF-8", tmpl, f"{name} 硬编码了 locale")

    def test_sync_substitutes_all_placeholders_when_rendering(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / ".zshrc").write_text("", encoding="utf-8")
            result = subprocess.run(
                ["bash", str(SYNC), "--dry-run"],
                capture_output=True,
                text=True,
                check=False,
                env={**os.environ, "HOME": tmp},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        # 占位符必须全部被替换,不能漏进渲染结果
        self.assertNotIn("{timezone}", result.stdout)
        self.assertNotIn("{lang}", result.stdout)
        # 替换后是完整、可用的 alias 行
        self.assertIn(
            'alias claude=\'TZ="America/Los_Angeles" LANG="en_US.UTF-8" '
            'LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions\'',
            result.stdout,
        )

    def test_sync_writes_managed_block_to_both_zshrc_and_zprofile(self):
        """别名必须同步到 .zshrc(interactive)与 .zprofile(login)两个文件——
        后者是让非交互 login shell(如 UU 远程 term)也能加载别名的关键;
        少写任何一个,该 shell 场景的别名就会失效。"""
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(
                ["bash", str(SYNC)],
                capture_output=True,
                text=True,
                check=False,
                env={**os.environ, "HOME": tmp},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            for fn in (".zshrc", ".zprofile"):
                content = (Path(tmp) / fn).read_text(encoding="utf-8")
                self.assertIn("# >>> agent-manager shell aliases >>>", content, f"{fn} 缺受管区块")
                self.assertIn("alias claude=", content, f"{fn} 缺 claude 别名")

    def test_sync_is_idempotent_across_both_targets(self):
        """两个目标文件都要各自幂等:第二次运行不得再改动任一文件。"""
        with tempfile.TemporaryDirectory() as tmp:
            env = {**os.environ, "HOME": tmp}
            subprocess.run(["bash", str(SYNC)], capture_output=True, text=True, env=env, check=True)
            first = {fn: (Path(tmp) / fn).read_text(encoding="utf-8") for fn in (".zshrc", ".zprofile")}
            subprocess.run(["bash", str(SYNC)], capture_output=True, text=True, env=env, check=True)
            for fn, before in first.items():
                self.assertEqual(before, (Path(tmp) / fn).read_text(encoding="utf-8"), f"{fn} 非幂等")


if __name__ == "__main__":
    unittest.main()
