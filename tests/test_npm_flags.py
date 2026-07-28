import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from src.npm_flags import OFFICIAL_NPM_REGISTRY, inject_npm_flags


class NpmFlagsTests(unittest.TestCase):
    def test_global_install_is_forced_onto_the_official_registry(self):
        hardened = inject_npm_flags("npm install -g demo@1.0.0")

        self.assertIn(f"--registry {OFFICIAL_NPM_REGISTRY}", hardened)

    def test_global_install_gets_fetch_retry_and_timeout_hardening(self):
        hardened = inject_npm_flags("npm install -g demo@1.0.0")

        self.assertIn("--fetch-retries 2", hardened)
        self.assertIn("--fetch-retry-factor 2", hardened)
        self.assertIn("--fetch-retry-mintimeout 10000", hardened)
        self.assertIn("--fetch-retry-maxtimeout 60000", hardened)
        self.assertIn("--fetch-timeout 120000", hardened)

    def test_global_install_suppresses_audit_and_fund_output(self):
        hardened = inject_npm_flags("npm install -g demo@1.0.0")

        self.assertIn("--no-audit", hardened)
        self.assertIn("--no-fund", hardened)

    def test_explicitly_requested_registry_is_not_overridden(self):
        hardened = inject_npm_flags("npm install -g demo@1.0.0 --registry https://registry.example.com")

        self.assertNotIn(OFFICIAL_NPM_REGISTRY, hardened)

    def test_explicitly_requested_flag_using_equals_form_is_not_duplicated(self):
        hardened = inject_npm_flags("npm install -g demo@1.0.0 --fetch-retries=7")

        self.assertNotIn("--fetch-retries 2", hardened)

    def test_non_install_npm_command_is_left_alone(self):
        self.assertEqual(inject_npm_flags("npm ls -g"), "npm ls -g")

    def test_list_form_command_is_returned_unchanged(self):
        cmd = ["npm", "view", "demo", "version"]

        self.assertIs(inject_npm_flags(cmd), cmd)


if __name__ == "__main__":
    unittest.main()
