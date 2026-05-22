import json
import subprocess
import tempfile
import unittest
from hashlib import sha256
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

    def _tree_digest(self, root: Path) -> dict[str, str]:
        result = {}
        for path in sorted(item for item in root.rglob("*") if item.is_file()):
            result[str(path.relative_to(root))] = sha256(path.read_bytes()).hexdigest()
        return result

    def _write_chrome_profile(self, chrome_root: Path):
        profile_dir = chrome_root / "Profile 1"
        default_dir = chrome_root / "Default"
        profile_dir.mkdir(parents=True)
        default_dir.mkdir(parents=True)
        (chrome_root / "Local State").write_text(
            json.dumps({"profile": {"last_used": "Profile 1"}, "keep": "local-state"}),
            encoding="utf-8",
        )
        for target_dir in (default_dir, profile_dir):
            (target_dir / "Preferences").write_text(
                json.dumps(
                    {
                        "intl": {"accept_languages": "zh-CN,zh"},
                        "default_search_provider": {"name": "Old Google"},
                        "keep": target_dir.name,
                    }
                ),
                encoding="utf-8",
            )
        return profile_dir

    def _write_exit_profile(self, root: Path, payload: dict) -> Path:
        profile_path = root / "exit-profile.json"
        profile_path.write_text(json.dumps(payload), encoding="utf-8")
        return profile_path

    def test_sync_updates_chrome_profiles_for_google_us_preflight(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"
            config_dir = Path(temp_dir) / "config"
            profile_dir = self._write_chrome_profile(chrome_root)

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--skip-exit-check",
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)

            local_state = json.loads((chrome_root / "Local State").read_text(encoding="utf-8"))
            preferences = json.loads((profile_dir / "Preferences").read_text(encoding="utf-8"))
            default_preferences = json.loads((chrome_root / "Default" / "Preferences").read_text(encoding="utf-8"))
            automation_context = json.loads((config_dir / "browser-us-context.json").read_text(encoding="utf-8"))

            self.assertEqual(local_state["intl"]["app_locale"], "en-US")
            self.assertEqual(local_state["countryid_at_install"], 244)
            self.assertEqual(local_state["variations_country"], "us")
            self.assertEqual(local_state["keep"], "local-state")
            self.assertEqual(preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(preferences["intl"]["selected_languages"], "en-US,en")
            self.assertEqual(preferences["spellcheck"]["dictionaries"], ["en-US"])
            self.assertEqual(preferences["spellcheck"]["dictionary"], "en-US")
            self.assertEqual(preferences["default_search_provider"]["name"], "Google US")
            self.assertIn("gl=US", preferences["default_search_provider"]["search_url"])
            self.assertEqual(preferences["webrtc"]["ip_handling_policy"], "disable_non_proxied_udp")
            self.assertFalse(preferences["webrtc"]["nonproxied_udp_enabled"])
            self.assertEqual(preferences["profile"]["default_content_setting_values"]["geolocation"], 2)
            self.assertEqual(preferences["keep"], "Profile 1")
            self.assertEqual(default_preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(default_preferences["default_search_provider"]["name"], "Google US")
            self.assertEqual(automation_context["browser_context"]["locale"], "en-US")
            self.assertEqual(automation_context["browser_context"]["timezone_id"], "America/Los_Angeles")
            self.assertIn("--force-webrtc-ip-handling-policy=disable_non_proxied_udp", automation_context["chrome_launch"]["args"])
            self.assertTrue((config_dir / "browser-us.env").exists())
            self.assertTrue((chrome_root / "Local State.agent-manager-before-google-us").exists())
            self.assertTrue((profile_dir / "Preferences.agent-manager-before-google-us").exists())

    def test_sync_uses_detected_us_exit_profile_for_automation_context(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            chrome_root = root / "chrome"
            config_dir = root / "config"
            self._write_chrome_profile(chrome_root)
            exit_profile = self._write_exit_profile(
                root,
                {
                    "ip": "203.0.113.31",
                    "country": "US",
                    "timezone": "America/New_York",
                    "loc": "40.7128,-74.0060",
                    "region": "New York",
                    "city": "New York",
                },
            )

            first_result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--exit-profile-json",
                str(exit_profile),
            )

            self.assertEqual(first_result.returncode, 0, first_result.stdout + first_result.stderr)
            self.assertIn("ip: 203.0.113.31", first_result.stdout)
            self.assertIn("region: New York", first_result.stdout)
            self.assertIn("city: New York", first_result.stdout)
            automation_context = json.loads((config_dir / "browser-us-context.json").read_text(encoding="utf-8"))
            env_content = (config_dir / "browser-us.env").read_text(encoding="utf-8")
            self.assertEqual(automation_context["browser_context"]["timezone_id"], "America/New_York")
            self.assertEqual(automation_context["browser_context"]["geolocation"]["latitude"], 40.7128)
            self.assertEqual(automation_context["browser_context"]["geolocation"]["longitude"], -74.006)
            self.assertEqual(automation_context["browser_context"]["geolocation"]["accuracy"], 50000)
            self.assertEqual(automation_context["environment"]["TZ"], "America/New_York")
            self.assertEqual(automation_context["exit_profile"]["ip"], "203.0.113.31")
            self.assertEqual(automation_context["exit_profile"]["country_code"], "US")
            self.assertIn("export TZ=America/New_York", env_content)
            first_digest = self._tree_digest(root)

            second_result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--exit-profile-json",
                str(exit_profile),
            )

            self.assertEqual(second_result.returncode, 0, second_result.stdout + second_result.stderr)
            self.assertEqual(self._tree_digest(root), first_digest)
            self.assertIn("already matched desired settings", second_result.stdout)

    def test_sync_is_idempotent_after_first_convergence(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            chrome_root = root / "chrome"
            config_dir = root / "config"
            self._write_chrome_profile(chrome_root)

            first_result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--skip-exit-check",
            )
            self.assertEqual(first_result.returncode, 0, first_result.stdout + first_result.stderr)
            first_digest = self._tree_digest(root)

            second_result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--skip-exit-check",
            )

            self.assertEqual(second_result.returncode, 0, second_result.stdout + second_result.stderr)
            self.assertEqual(self._tree_digest(root), first_digest)
            self.assertIn("already matched desired settings", second_result.stdout)

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
                "--skip-exit-check",
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertEqual((chrome_root / "Local State").read_text(encoding="utf-8"), before_local_state)
            self.assertEqual((profile_dir / "Preferences").read_text(encoding="utf-8"), before_preferences)

    def test_missing_chrome_profile_is_warning_not_failure(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"
            config_dir = Path(temp_dir) / "config"

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--skip-exit-check",
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("browser user data directory not found", result.stderr)

    def test_non_us_exit_country_fails_closed_when_detected(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            chrome_root = root / "chrome"
            config_dir = root / "config"
            self._write_chrome_profile(chrome_root)
            exit_profile = self._write_exit_profile(root, {"ip": "203.0.113.10", "country": "HK"})

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--no-browser-open",
                "--exit-profile-json",
                str(exit_profile),
            )

            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("Detected public exit country is HK, not US", result.stderr)
            self.assertFalse((config_dir / "browser-us-context.json").exists())
