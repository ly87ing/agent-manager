import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "sync_browser_google_us.py"


class SyncBrowserGoogleUsTests(unittest.TestCase):
    def _run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _write_chrome_profile(self, chrome_root: Path):
        profile_dir = chrome_root / "Profile 1"
        profile_dir.mkdir(parents=True)
        (chrome_root / "Local State").write_text(
            json.dumps({"profile": {"last_used": "Profile 1"}, "keep": "local-state"}),
            encoding="utf-8",
        )
        (profile_dir / "Preferences").write_text(
            json.dumps(
                {
                    "intl": {"accept_languages": "zh-CN,zh"},
                    "default_search_provider": {"name": "Old Google"},
                    "keep": "preferences",
                }
            ),
            encoding="utf-8",
        )
        return profile_dir

    def test_sync_updates_chrome_profile_for_google_us_preflight(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"
            profile_dir = self._write_chrome_profile(chrome_root)

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--no-browser-open",
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)

            local_state = json.loads((chrome_root / "Local State").read_text(encoding="utf-8"))
            preferences = json.loads((profile_dir / "Preferences").read_text(encoding="utf-8"))

            self.assertEqual(local_state["intl"]["app_locale"], "en-US")
            self.assertEqual(local_state["keep"], "local-state")
            self.assertEqual(preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(preferences["default_search_provider"]["name"], "Google US")
            self.assertIn("gl=US", preferences["default_search_provider"]["search_url"])
            self.assertEqual(preferences["keep"], "preferences")
            self.assertTrue((chrome_root / "Local State.agent-manager-before-google-us").exists())
            self.assertTrue((profile_dir / "Preferences.agent-manager-before-google-us").exists())

    def test_dry_run_does_not_write_chrome_profile(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"
            profile_dir = self._write_chrome_profile(chrome_root)
            before_local_state = (chrome_root / "Local State").read_text(encoding="utf-8")
            before_preferences = (profile_dir / "Preferences").read_text(encoding="utf-8")

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--dry-run",
                "--no-browser-open",
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertEqual((chrome_root / "Local State").read_text(encoding="utf-8"), before_local_state)
            self.assertEqual((profile_dir / "Preferences").read_text(encoding="utf-8"), before_preferences)

    def test_missing_chrome_profile_is_warning_not_failure(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--no-browser-open",
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("Chrome user data directory not found", result.stderr)
