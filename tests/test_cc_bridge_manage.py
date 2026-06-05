import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class CcBridgeRetirementTests(unittest.TestCase):
    def test_runtime_manager_script_is_not_committed(self):
        self.assertFalse((ROOT / "scripts" / "cc-bridge" / "manage.py").exists())

    def test_managed_image_dockerfile_is_not_committed(self):
        self.assertFalse((ROOT / "scripts" / "cc-bridge" / "image" / "Dockerfile").exists())

    def test_management_doc_is_not_committed(self):
        self.assertFalse((ROOT / "docs" / "CC-BRIDGE-MANAGEMENT.md").exists())


if __name__ == "__main__":
    unittest.main()
