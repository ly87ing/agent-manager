import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "scripts" / "fetch_models.py"
SPEC = importlib.util.spec_from_file_location("fetch_models_module", MODULE_PATH)
fetch_models = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(fetch_models)


class FetchModelsTests(unittest.TestCase):
    def test_render_configuration_guide_masks_api_key(self):
        guide = fetch_models.render_configuration_guide(
            "https://api.example.com",
            "sk-secret-1234567890",
            "OpenAI",
            ["gpt-4o"],
        )

        self.assertIn("sk-s...7890", guide)
        self.assertNotIn("sk-secret-1234567890", guide)
