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


if __name__ == "__main__":
    unittest.main()
