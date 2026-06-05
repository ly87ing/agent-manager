import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ClaudeStatuslineRetirementTests(unittest.TestCase):
    def test_makefile_does_not_expose_statusline_targets(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")

        self.assertNotIn("claude-statusline", makefile)
        self.assertNotIn("claude\\:statusline", makefile)
        self.assertNotIn("claude:statusline", makefile)

    def test_manage_sh_does_not_wire_or_install_statusline(self):
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")

        self.assertNotIn("claude:statusline", manage)
        self.assertNotIn("install_statusline.py", manage)
        self.assertNotIn("~/.claude/statusline.sh", manage)

    def test_statusline_helper_artifacts_are_not_committed(self):
        self.assertFalse((ROOT / "scripts" / "claude" / "install_statusline.py").exists())
        self.assertFalse((ROOT / "scripts" / "claude" / "statusline.sh").exists())


if __name__ == "__main__":
    unittest.main()
