import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "sync_browser_google_us.py"


class SyncBrowserGoogleUsTests(unittest.TestCase):
    def _run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), "--post-sync-settle-seconds", "0", *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _tree_digest(self, root: Path) -> dict[str, str]:
        result = {}
        for path in sorted(item for item in root.rglob("*") if item.is_file()):
            relative_path = str(path.relative_to(root))
            file_mode = path.stat().st_mode & 0o777
            result[relative_path] = f"{file_mode:o}:{sha256(path.read_bytes()).hexdigest()}"
        return result

    def _write_chrome_profile(self, chrome_root: Path):
        profile_dir = chrome_root / "Profile 1"
        default_dir = chrome_root / "Default"
        profile_dir.mkdir(parents=True)
        default_dir.mkdir(parents=True)
        (chrome_root / "Local State").write_text(
            json.dumps(
                {
                    "profile": {"last_used": "Profile 1"},
                    "variations_safe_seed_locale": "zh-CN",
                    "variations_safe_seed_permanent_consistency_country": "cn",
                    "variations_safe_seed_session_consistency_country": "cn",
                    "keep": "local-state",
                }
            ),
            encoding="utf-8",
        )
        for target_dir in (default_dir, profile_dir):
            (target_dir / "Preferences").write_text(
                json.dumps(
                    {
                        "countryid_at_install": 21843,
                        "intl": {"accept_languages": "zh-CN,zh"},
                        "account_values": {
                            "intl": {
                                "accept_languages": "zh-CN,zh",
                                "selected_languages": "zh-CN,zh",
                            },
                            "translate_recent_target": "zh-CN",
                        },
                        "default_search_provider": {"name": "Old Google"},
                        "translate_recent_target": "zh-CN",
                        "profile": {
                            "content_settings": {
                                "exceptions": {
                                    "geolocation": {
                                        "https://example.test,*": {"setting": 1},
                                    },
                                    "geolocation_with_options": {
                                        "https://maps.example.test,*": {"last_modified": "1"},
                                    },
                                    "reduced_accept_language": {
                                        "https://search.example.test,*": {"setting": 1},
                                    },
                                },
                            },
                        },
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
            verification_report = json.loads((config_dir / "browser-us-verification.json").read_text(encoding="utf-8"))
            launcher_path = config_dir / "browser-us-chrome.sh"
            launcher_content = launcher_path.read_text(encoding="utf-8")
            managed_profile_root = config_dir / "browser-us-profile"
            managed_local_state = json.loads((managed_profile_root / "Local State").read_text(encoding="utf-8"))
            managed_preferences = json.loads((managed_profile_root / "Default" / "Preferences").read_text(encoding="utf-8"))

            self.assertEqual(local_state["intl"]["app_locale"], "en-US")
            self.assertEqual(local_state["countryid_at_install"], 244)
            self.assertEqual(local_state["variations_country"], "us")
            self.assertEqual(local_state["variations_safe_seed_locale"], "en-US")
            self.assertEqual(local_state["variations_safe_seed_permanent_consistency_country"], "us")
            self.assertEqual(local_state["variations_safe_seed_session_consistency_country"], "us")
            self.assertEqual(local_state["keep"], "local-state")
            self.assertEqual(preferences["countryid_at_install"], 21843)
            self.assertEqual(preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(preferences["intl"]["selected_languages"], "en-US,en")
            self.assertEqual(preferences["account_values"]["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(preferences["account_values"]["intl"]["selected_languages"], "en-US,en")
            self.assertEqual(preferences["translate_recent_target"], "en")
            self.assertEqual(preferences["account_values"]["translate_recent_target"], "en")
            self.assertEqual(preferences["spellcheck"]["dictionaries"], ["en-US"])
            self.assertEqual(preferences["spellcheck"]["dictionary"], "en-US")
            self.assertEqual(preferences["default_search_provider"]["name"], "Google US")
            self.assertIn("gl=US", preferences["default_search_provider"]["search_url"])
            self.assertEqual(preferences["webrtc"]["ip_handling_policy"], "disable_non_proxied_udp")
            self.assertFalse(preferences["webrtc"]["nonproxied_udp_enabled"])
            self.assertEqual(preferences["profile"]["default_content_setting_values"]["geolocation"], 2)
            self.assertEqual(preferences["profile"]["content_settings"]["exceptions"]["geolocation"], {})
            self.assertEqual(preferences["profile"]["content_settings"]["exceptions"]["geolocation_with_options"], {})
            self.assertEqual(preferences["profile"]["content_settings"]["exceptions"]["reduced_accept_language"], {})
            self.assertEqual(preferences["keep"], "Profile 1")
            self.assertEqual(default_preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(default_preferences["account_values"]["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(default_preferences["default_search_provider"]["name"], "Google US")
            self.assertEqual(default_preferences["profile"]["content_settings"]["exceptions"]["geolocation"], {})
            self.assertEqual(managed_local_state["intl"]["app_locale"], "en-US")
            self.assertEqual(managed_preferences["countryid_at_install"], 244)
            self.assertEqual(managed_preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(automation_context["browser_context"]["locale"], "en-US")
            self.assertEqual(automation_context["browser_context"]["timezone_id"], "America/Los_Angeles")
            self.assertEqual(automation_context["google"]["home_url"], "https://www.google.com/?hl=en-US&gl=US&pws=0")
            self.assertEqual(automation_context["chrome_launch"]["launcher"], str(launcher_path))
            self.assertEqual(automation_context["chrome_launch"]["managed_user_data_dir"], str(managed_profile_root))
            self.assertIn(f"--user-data-dir={managed_profile_root}", automation_context["chrome_launch"]["args"])
            self.assertIn("--disable-sync", automation_context["chrome_launch"]["args"])
            self.assertIn("--disable-extensions", automation_context["chrome_launch"]["args"])
            self.assertIn("--force-webrtc-ip-handling-policy=disable_non_proxied_udp", automation_context["chrome_launch"]["args"])
            self.assertEqual(verification_report["overall_status"], "limited")
            self.assertEqual(verification_report["exit"]["verification_state"], "manually_skipped")
            self.assertEqual(verification_report["google_domain"]["verification_state"], "exit_check_skipped")
            self.assertEqual(verification_report["runtime"]["verification_state"], "exit_check_skipped")
            self.assertTrue(verification_report["browser_state_audit"]["overall_ok"])
            self.assertIn(
                "Agent Manager Browser US",
                {root["name"] for root in verification_report["browser_state_audit"]["roots"]},
            )
            self.assertEqual(verification_report["enforced_features"]["local_state_country"]["variations_country"], "us")
            self.assertEqual(verification_report["enforced_features"]["google"]["home_url"], "https://www.google.com/?hl=en-US&gl=US&pws=0")
            self.assertEqual(verification_report["enforced_features"]["google"]["search_gl"], "US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["chrome_launcher"], "$AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER")
            self.assertEqual(verification_report["enforced_features"]["automation"]["default_url"], "https://www.google.com/ncr")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_language"], "en-US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_languages"], ["en-US", "en"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_locale"], "en-US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_calendar"], "gregory")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_numbering_system"], "latn")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_hour_cycle"], "h12")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_locale_region"], "US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_navigator_language_region"], "US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_number_format"], "1,234,567.89")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_list_format"], "A, B, and C")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_accept_language_header"], "en-US,en;q=0.9")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_geolocation_permission"], "denied")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_webrtc_policy"], "disable_non_proxied_udp")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_geolocation_setting"], 2)
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["managed_user_data_dir"],
                "$AGENT_MANAGER_BROWSER_US_USER_DATA_DIR",
            )
            self.assertEqual(launcher_path.stat().st_mode & 0o777, 0o755)
            self.assertIn("export TZ=America/Los_Angeles", launcher_content)
            self.assertIn("AGENT_MANAGER_BROWSER_US_DEFAULT_URL=https://www.google.com/ncr", launcher_content)
            self.assertIn('if [ "$#" -eq 0 ]; then', launcher_content)
            self.assertIn("--lang=en-US", launcher_content)
            self.assertIn('--user-data-dir="$BROWSER_US_USER_DATA_DIR"', launcher_content)
            self.assertIn("--disable-sync", launcher_content)
            self.assertIn("--disable-extensions", launcher_content)
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
            verification_report = json.loads((config_dir / "browser-us-verification.json").read_text(encoding="utf-8"))
            env_content = (config_dir / "browser-us.env").read_text(encoding="utf-8")
            launcher_path = config_dir / "browser-us-chrome.sh"
            launcher_content = launcher_path.read_text(encoding="utf-8")
            self.assertEqual(automation_context["browser_context"]["timezone_id"], "America/New_York")
            self.assertEqual(automation_context["browser_context"]["geolocation"]["latitude"], 40.7128)
            self.assertEqual(automation_context["browser_context"]["geolocation"]["longitude"], -74.006)
            self.assertEqual(automation_context["browser_context"]["geolocation"]["accuracy"], 50000)
            self.assertEqual(automation_context["environment"]["TZ"], "America/New_York")
            self.assertEqual(automation_context["exit_profile"]["ip"], "203.0.113.31")
            self.assertEqual(automation_context["exit_profile"]["country_code"], "US")
            self.assertEqual(automation_context["chrome_launch"]["launcher"], str(launcher_path))
            self.assertEqual(automation_context["chrome_launch"]["managed_user_data_dir"], str(config_dir / "browser-us-profile"))
            self.assertEqual(verification_report["overall_status"], "complete")
            self.assertEqual(verification_report["exit"]["verification_state"], "verified_us")
            self.assertEqual(verification_report["google_domain"]["verification_state"], "exit_profile_json_skipped")
            self.assertEqual(verification_report["runtime"]["verification_state"], "exit_profile_json_skipped")
            self.assertTrue(verification_report["browser_state_audit"]["overall_ok"])
            self.assertEqual(verification_report["exit"]["profile"]["ip"], "203.0.113.31")
            self.assertEqual(verification_report["enforced_features"]["automation"]["timezone_id"], "America/New_York")
            self.assertEqual(launcher_path.stat().st_mode & 0o777, 0o755)
            self.assertIn("export TZ=America/New_York", launcher_content)
            self.assertIn("export TZ=America/New_York", env_content)
            self.assertIn("AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER", env_content)
            self.assertIn("AGENT_MANAGER_BROWSER_US_USER_DATA_DIR", env_content)
            self.assertIn("AGENT_MANAGER_BROWSER_US_DEFAULT_URL", env_content)
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
            config_dir = Path(temp_dir) / "config"
            profile_dir = self._write_chrome_profile(chrome_root)
            before_local_state = (chrome_root / "Local State").read_text(encoding="utf-8")
            before_preferences = (profile_dir / "Preferences").read_text(encoding="utf-8")

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--dry-run",
                "--no-browser-open",
                "--skip-exit-check",
            )

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertEqual((chrome_root / "Local State").read_text(encoding="utf-8"), before_local_state)
            self.assertEqual((profile_dir / "Preferences").read_text(encoding="utf-8"), before_preferences)
            self.assertFalse((config_dir / "browser-us-profile").exists())

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
            self.assertFalse((config_dir / "browser-us-verification.json").exists())

    def test_unverified_exit_profile_fails_closed_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            chrome_root = root / "chrome"
            config_dir = root / "config"
            self._write_chrome_profile(chrome_root)
            sitecustomize = root / "sitecustomize.py"
            sitecustomize.write_text(
                "\n".join(
                    [
                        "import urllib.error",
                        "import urllib.request",
                        "def fail(*args, **kwargs):",
                        "    raise urllib.error.URLError('blocked')",
                        "urllib.request.urlopen = fail",
                        "",
                    ]
                ),
                encoding="utf-8",
            )
            env = os.environ.copy()
            env["PYTHONPATH"] = str(root)

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--chrome-user-data-dir",
                    str(chrome_root),
                    "--config-dir",
                    str(config_dir),
                    "--no-browser-open",
                    "--post-sync-settle-seconds",
                    "0",
                    "--exit-check-timeout",
                    "0.1",
                ],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("could not verify public exit profile", result.stderr)
            self.assertFalse((config_dir / "browser-us-context.json").exists())
            self.assertFalse((config_dir / "browser-us-verification.json").exists())

    def test_post_sync_verification_fails_for_missing_explicit_profile(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"
            config_dir = Path(temp_dir) / "config"
            self._write_chrome_profile(chrome_root)

            result = self._run(
                "--chrome-user-data-dir",
                str(chrome_root),
                "--config-dir",
                str(config_dir),
                "--profile",
                "Missing Profile",
                "--no-browser-open",
                "--skip-exit-check",
            )

            self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
            self.assertIn("Preferences mismatch", result.stderr)
            verification_report = json.loads((config_dir / "browser-us-verification.json").read_text(encoding="utf-8"))
            self.assertEqual(verification_report["overall_status"], "failed")
            self.assertFalse(verification_report["browser_state_audit"]["overall_ok"])

    def test_google_domain_verification_fails_when_ncr_still_returns_hk(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            chrome_root = root / "chrome"
            config_dir = root / "config"
            self._write_chrome_profile(chrome_root)
            sitecustomize = root / "sitecustomize.py"
            sitecustomize.write_text(
                "\n".join(
                    [
                        "import json",
                        "import urllib.request",
                        "class Headers:",
                        "    def get_content_charset(self):",
                        "        return 'utf-8'",
                        "class Response:",
                        "    def __init__(self, url, body):",
                        "        self._url = url",
                        "        self._body = body.encode('utf-8')",
                        "        self.headers = Headers()",
                        "    def __enter__(self):",
                        "        return self",
                        "    def __exit__(self, *args):",
                        "        return False",
                        "    def read(self, *args):",
                        "        return self._body",
                        "    def geturl(self):",
                        "        return self._url",
                        "def fake_urlopen(request, timeout=0):",
                        "    payload = {'ip': '203.0.113.31', 'country': 'US', 'timezone': 'America/Los_Angeles'}",
                        "    return Response(getattr(request, 'full_url', str(request)), json.dumps(payload))",
                        "class Opener:",
                        "    def open(self, request, timeout=0):",
                        "        url = getattr(request, 'full_url', str(request))",
                        "        body = '<html><a href=\"https://www.google.com.hk/imghp\">Images</a></html>'",
                        "        return Response(url, body)",
                        "urllib.request.urlopen = fake_urlopen",
                        "urllib.request.build_opener = lambda *args, **kwargs: Opener()",
                        "",
                    ]
                ),
                encoding="utf-8",
            )
            env = os.environ.copy()
            env["PYTHONPATH"] = str(root)

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--chrome-user-data-dir",
                    str(chrome_root),
                    "--config-dir",
                    str(config_dir),
                    "--no-browser-open",
                    "--post-sync-settle-seconds",
                    "0",
                ],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 4, result.stdout + result.stderr)
            self.assertIn("HK markers", result.stderr)
            verification_report = json.loads((config_dir / "browser-us-verification.json").read_text(encoding="utf-8"))
            self.assertEqual(verification_report["overall_status"], "failed")
            self.assertEqual(verification_report["google_domain"]["verification_state"], "hk_detected_failed")
            self.assertEqual(verification_report["google_domain"]["hk_markers"], ["google.com.hk"])

    def test_runtime_context_parser_extracts_us_runtime_payload(self):
        payload = (
            "<html><head><script>document.write('AGENT_MANAGER_RUNTIME_JSON:'"
            "+JSON.stringify({language:navigator.language}));</script></head>"
            "<body>AGENT_MANAGER_RUNTIME_JSON:"
            '{"language":"en-US","languages":["en-US","en"],'
            '"timezone":"America/Los_Angeles","intlLocale":"en-US",'
            '"intlCalendar":"gregory","intlNumberingSystem":"latn","intlHourCycle":"h12",'
            '"intlLocaleRegion":"US","navigatorLanguageRegion":"US",'
            '"numberFormat":"1,234,567.89","listFormat":"A, B, and C",'
            '"acceptLanguageHeader":"en-US,en;q=0.9",'
            '"geolocationPermission":"denied",'
            '"profileWebrtcPolicy":"disable_non_proxied_udp",'
            '"profileWebrtcNonproxiedUdpEnabled":false,'
            '"profileGeolocationSetting":2}'
            "</body></html>"
        )
        spec = importlib.util.spec_from_file_location("sync_browser_google_us_test", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        try:
            spec.loader.exec_module(module)
        finally:
            sys.modules.pop(spec.name, None)
        runtime_context = module.runtime_context_from_output(payload)
        mismatches = module.validate_runtime_context(runtime_context, "America/Los_Angeles")

        self.assertEqual(runtime_context["language"], "en-US")
        self.assertEqual(runtime_context["languages"], ["en-US", "en"])
        self.assertEqual(runtime_context["timezone"], "America/Los_Angeles")
        self.assertEqual(runtime_context["intlCalendar"], "gregory")
        self.assertEqual(runtime_context["intlNumberingSystem"], "latn")
        self.assertEqual(runtime_context["intlHourCycle"], "h12")
        self.assertEqual(runtime_context["intlLocaleRegion"], "US")
        self.assertEqual(runtime_context["navigatorLanguageRegion"], "US")
        self.assertEqual(runtime_context["numberFormat"], "1,234,567.89")
        self.assertEqual(runtime_context["listFormat"], "A, B, and C")
        self.assertEqual(runtime_context["acceptLanguageHeader"], "en-US,en;q=0.9")
        self.assertEqual(runtime_context["geolocationPermission"], "denied")
        self.assertEqual(runtime_context["profileWebrtcPolicy"], "disable_non_proxied_udp")
        self.assertFalse(runtime_context["profileWebrtcNonproxiedUdpEnabled"])
        self.assertEqual(runtime_context["profileGeolocationSetting"], 2)
        self.assertEqual(mismatches, ())
