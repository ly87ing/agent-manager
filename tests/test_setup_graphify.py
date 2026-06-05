import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class SetupGraphifyRetirementTests(unittest.TestCase):
    def test_setup_script_is_not_committed(self):
        self.assertFalse((ROOT / "scripts" / "setup-graphify.py").exists())

    def test_graphify_tool_is_not_default_managed(self):
        self.assertNotIn("graphify", (ROOT / "config" / "manager.json").read_text(encoding="utf-8"))
        self.assertNotIn("graphify", (ROOT / "config" / "tools.json").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
