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
                                    "media_stream_camera": {
                                        "https://camera.example.test,*": {"setting": 1},
                                    },
                                    "media_stream_mic": {
                                        "https://mic.example.test,*": {"setting": 1},
                                    },
                                    "notifications": {
                                        "https://notify.example.test,*": {"setting": 1},
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

    def _write_managed_profile_volatile_state(self, config_dir: Path) -> list[Path]:
        managed_root = config_dir / "browser-us-profile"
        file_relative_paths = [
            "Default/Account Web Data",
            "Default/Account Web Data-journal",
            "Default/Cookies",
            "Default/Cookies-journal",
            "Default/Favicons",
            "Default/Favicons-journal",
            "Default/History",
            "Default/History-journal",
            "Default/Network Persistent State",
            "Default/Network/Cookies",
            "Default/Network/Cookies-journal",
            "Default/Reporting and NEL",
            "Default/Reporting and NEL-journal",
            "Default/Safe Browsing Cookies",
            "Default/Safe Browsing Cookies-journal",
            "Default/Top Sites",
            "Default/Top Sites-journal",
            "Default/Trust Tokens",
            "Default/Trust Tokens-journal",
            "Default/Web Data",
            "Default/Web Data-journal",
        ]
        directory_relative_paths = [
            "Default/ClientCertificates",
            "Default/DawnGraphiteCache",
            "Default/DawnWebGPUCache",
            "Default/IndexedDB",
            "Default/Local Storage",
            "Default/PersistentOriginTrials",
            "Default/Sessions",
            "Default/Session Storage",
            "Default/Service Worker",
            "Default/Shared Dictionary",
            "Default/Site Characteristics Database",
            "Default/Cache",
            "Default/Code Cache",
            "Default/GPUCache",
            "Default/WebStorage",
        ]
        volatile_targets = []
        for relative_path in file_relative_paths:
            target = managed_root / relative_path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("sticky-region-state", encoding="utf-8")
            volatile_targets.append(target)
        for relative_path in directory_relative_paths:
            target = managed_root / relative_path
            target.mkdir(parents=True, exist_ok=True)
            (target / "marker").write_text("sticky-region-state", encoding="utf-8")
            volatile_targets.append(target)
        return volatile_targets

    def _write_exit_profile(self, root: Path, payload: dict) -> Path:
        profile_path = root / "exit-profile.json"
        profile_path.write_text(json.dumps(payload), encoding="utf-8")
        return profile_path

    def test_sync_updates_chrome_profiles_for_google_us_preflight(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            chrome_root = Path(temp_dir) / "chrome"
            config_dir = Path(temp_dir) / "config"
            profile_dir = self._write_chrome_profile(chrome_root)
            volatile_state_paths = self._write_managed_profile_volatile_state(config_dir)

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
            managed_audit = next(
                root_record
                for root_record in verification_report["browser_state_audit"]["roots"]
                if root_record["name"] == "Agent Manager Browser US"
            )

            self.assertEqual(local_state["intl"]["app_locale"], "en-US")
            self.assertNotIn("countryid_at_install", local_state)
            self.assertNotIn("variations_country", local_state)
            self.assertEqual(local_state["variations_safe_seed_locale"], "zh-CN")
            self.assertEqual(local_state["variations_safe_seed_permanent_consistency_country"], "cn")
            self.assertEqual(local_state["variations_safe_seed_session_consistency_country"], "cn")
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
            self.assertNotIn("media_stream_camera", preferences["profile"]["default_content_setting_values"])
            self.assertNotIn("media_stream_mic", preferences["profile"]["default_content_setting_values"])
            self.assertNotIn("notifications", preferences["profile"]["default_content_setting_values"])
            self.assertEqual(preferences["profile"]["content_settings"]["exceptions"]["geolocation"], {})
            self.assertEqual(preferences["profile"]["content_settings"]["exceptions"]["geolocation_with_options"], {})
            self.assertEqual(
                preferences["profile"]["content_settings"]["exceptions"]["media_stream_camera"],
                {"https://camera.example.test,*": {"setting": 1}},
            )
            self.assertEqual(
                preferences["profile"]["content_settings"]["exceptions"]["media_stream_mic"],
                {"https://mic.example.test,*": {"setting": 1}},
            )
            self.assertEqual(
                preferences["profile"]["content_settings"]["exceptions"]["notifications"],
                {"https://notify.example.test,*": {"setting": 1}},
            )
            self.assertEqual(preferences["profile"]["content_settings"]["exceptions"]["reduced_accept_language"], {})
            self.assertEqual(preferences["keep"], "Profile 1")
            self.assertEqual(default_preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(default_preferences["account_values"]["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(default_preferences["default_search_provider"]["name"], "Google US")
            self.assertEqual(default_preferences["profile"]["content_settings"]["exceptions"]["geolocation"], {})
            self.assertEqual(
                default_preferences["profile"]["content_settings"]["exceptions"]["media_stream_camera"],
                {"https://camera.example.test,*": {"setting": 1}},
            )
            self.assertEqual(
                default_preferences["profile"]["content_settings"]["exceptions"]["media_stream_mic"],
                {"https://mic.example.test,*": {"setting": 1}},
            )
            self.assertEqual(
                default_preferences["profile"]["content_settings"]["exceptions"]["notifications"],
                {"https://notify.example.test,*": {"setting": 1}},
            )
            self.assertEqual(managed_local_state["intl"]["app_locale"], "en-US")
            self.assertEqual(managed_preferences["countryid_at_install"], 244)
            self.assertEqual(managed_preferences["intl"]["accept_languages"], "en-US,en")
            self.assertEqual(managed_preferences["profile"]["default_content_setting_values"]["media_stream_camera"], 2)
            self.assertEqual(managed_preferences["profile"]["default_content_setting_values"]["media_stream_mic"], 2)
            self.assertEqual(managed_preferences["profile"]["default_content_setting_values"]["notifications"], 2)
            self.assertEqual(managed_preferences["profile"]["content_settings"]["exceptions"]["media_stream_camera"], {})
            self.assertEqual(managed_preferences["profile"]["content_settings"]["exceptions"]["media_stream_mic"], {})
            self.assertEqual(managed_preferences["profile"]["content_settings"]["exceptions"]["notifications"], {})
            self.assertFalse(any(path.exists() or path.is_symlink() for path in volatile_state_paths))
            self.assertEqual(automation_context["browser_context"]["locale"], "en-US")
            self.assertEqual(automation_context["browser_context"]["timezone_id"], "America/Los_Angeles")
            self.assertIn("Chrome/", automation_context["browser_context"]["user_agent"])
            self.assertIn("Safari/537.36", automation_context["browser_context"]["user_agent"])
            self.assertEqual(automation_context["browser_context"]["navigator_languages"], ["en-US", "en"])
            self.assertEqual(automation_context["browser_context"]["client_hints_platform"], "macOS")
            self.assertEqual(automation_context["browser_context"]["navigator_platform"], "MacIntel")
            self.assertEqual(automation_context["browser_context"]["viewport"], {"width": 1365, "height": 768})
            self.assertEqual(automation_context["browser_context"]["screen"], {"width": 1365, "height": 768})
            self.assertEqual(automation_context["browser_context"]["device_scale_factor"], 1)
            self.assertFalse(automation_context["browser_context"]["is_mobile"])
            self.assertFalse(automation_context["browser_context"]["has_touch"])
            self.assertEqual(automation_context["browser_context"]["color_scheme"], "light")
            self.assertEqual(automation_context["browser_context"]["hardware_concurrency"], 8)
            self.assertEqual(automation_context["browser_context"]["device_memory_gb"], 8)
            self.assertEqual(automation_context["browser_context"]["webgl_vendor"], "Intel Inc.")
            self.assertEqual(automation_context["browser_context"]["webgl_renderer"], "Intel Iris OpenGL Engine")
            self.assertIn("PingFang SC", automation_context["browser_context"]["blocked_font_families"])
            self.assertIn("MingLiU", automation_context["browser_context"]["blocked_font_families"])
            self.assertEqual(automation_context["browser_context"]["font_metric_fallback_family"], "Arial")
            self.assertEqual(automation_context["browser_context"]["keyboard_layout_map"]["KeyQ"], "q")
            self.assertEqual(automation_context["browser_context"]["keyboard_layout_map"]["KeyZ"], "z")
            self.assertEqual(automation_context["google"]["home_url"], "https://www.google.com/?hl=en-US&gl=US&pws=0")
            self.assertEqual(automation_context["chrome_launch"]["launcher"], str(launcher_path))
            self.assertEqual(automation_context["chrome_launch"]["managed_user_data_dir"], str(managed_profile_root))
            self.assertIn(f"--user-data-dir={managed_profile_root}", automation_context["chrome_launch"]["args"])
            self.assertIn("--window-size=1365,768", automation_context["chrome_launch"]["args"])
            self.assertIn("--force-device-scale-factor=1", automation_context["chrome_launch"]["args"])
            self.assertTrue(
                any(arg.startswith("--user-agent=Mozilla/5.0") for arg in automation_context["chrome_launch"]["args"])
            )
            self.assertIn("--disable-sync", automation_context["chrome_launch"]["args"])
            self.assertIn("--disable-extensions", automation_context["chrome_launch"]["args"])
            self.assertIn("--disable-speech-api", automation_context["chrome_launch"]["args"])
            self.assertIn("--deny-permission-prompts", automation_context["chrome_launch"]["args"])
            self.assertIn(
                "--disable-features=ReduceAcceptLanguage,WebGPU,FontAccess,WebBluetooth,WebUSB,WebHID,WebXR,Serial",
                automation_context["chrome_launch"]["args"],
            )
            self.assertIn("--disable-blink-features=AutomationControlled", automation_context["chrome_launch"]["args"])
            self.assertIn("--force-webrtc-ip-handling-policy=disable_non_proxied_udp", automation_context["chrome_launch"]["args"])
            self.assertIn("--disable-quic", automation_context["chrome_launch"]["args"])
            self.assertIn("--dns-prefetch-disable", automation_context["chrome_launch"]["args"])
            self.assertEqual(verification_report["overall_status"], "limited")
            self.assertEqual(verification_report["exit"]["verification_state"], "manually_skipped")
            self.assertEqual(verification_report["google_domain"]["verification_state"], "exit_check_skipped")
            self.assertEqual(verification_report["browser_google"]["verification_state"], "exit_check_skipped")
            self.assertEqual(verification_report["runtime"]["verification_state"], "exit_check_skipped")
            self.assertEqual(verification_report["automation_context"]["verification_state"], "exit_check_skipped")
            self.assertTrue(verification_report["browser_state_audit"]["overall_ok"])
            self.assertIn(
                "Agent Manager Browser US",
                {root["name"] for root in verification_report["browser_state_audit"]["roots"]},
            )
            self.assertTrue(managed_audit["volatile_state"]["ok"])
            self.assertFalse(any(path_record["exists"] for path_record in managed_audit["volatile_state"]["paths"]))
            self.assertEqual(
                verification_report["enforced_features"]["local_state_country"]["managed_profile_variations_country"],
                "us",
            )
            self.assertEqual(
                verification_report["enforced_features"]["managed_profile_storage"]["volatile_state_policy"],
                "cleared_on_every_sync",
            )
            self.assertIn(
                "Default/Cookies",
                verification_report["enforced_features"]["managed_profile_storage"]["cleared_relative_paths"],
            )
            self.assertIn(
                "Default/Local Storage",
                verification_report["enforced_features"]["managed_profile_storage"]["cleared_relative_paths"],
            )
            self.assertIn(
                "Default/Network Persistent State",
                verification_report["enforced_features"]["managed_profile_storage"]["cleared_relative_paths"],
            )
            self.assertIn(
                "Default/Trust Tokens",
                verification_report["enforced_features"]["managed_profile_storage"]["cleared_relative_paths"],
            )
            self.assertIn(
                "Default/Site Characteristics Database",
                verification_report["enforced_features"]["managed_profile_storage"]["cleared_relative_paths"],
            )
            self.assertEqual(verification_report["enforced_features"]["google"]["home_url"], "https://www.google.com/?hl=en-US&gl=US&pws=0")
            self.assertEqual(verification_report["enforced_features"]["google"]["search_gl"], "US")
            self.assertEqual(verification_report["enforced_features"]["google"]["browser_google_after_ncr"], "https://www.google.com/?hl=en-US&gl=US&pws=0")
            self.assertEqual(verification_report["enforced_features"]["automation"]["chrome_launcher"], "$AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER")
            self.assertEqual(verification_report["enforced_features"]["automation"]["default_url"], "https://www.google.com/ncr")
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["context_geolocation"],
                {"latitude": 37.7749, "longitude": -122.4194, "accuracy": 100},
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["context_permissions"], ["geolocation"])
            self.assertEqual(
                verification_report["enforced_features"]["locale"]["disabled_chrome_features"],
                ["ReduceAcceptLanguage", "WebGPU", "FontAccess", "WebBluetooth", "WebUSB", "WebHID", "WebXR", "Serial"],
            )
            self.assertEqual(
                verification_report["enforced_features"]["device_permissions"][
                    "managed_profile_camera_content_setting"
                ],
                2,
            )
            self.assertEqual(
                verification_report["enforced_features"]["device_permissions"][
                    "managed_profile_microphone_content_setting"
                ],
                2,
            )
            self.assertEqual(
                verification_report["enforced_features"]["device_permissions"][
                    "managed_profile_notifications_content_setting"
                ],
                2,
            )
            self.assertEqual(
                verification_report["enforced_features"]["device_permissions"]["permission_prompts"],
                "denied_by_chrome_launch_arg",
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_language"], "en-US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_languages"], ["en-US", "en"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_languages"], ["en-US", "en"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_locale"], "en-US")
            self.assertIsInstance(
                verification_report["enforced_features"]["automation"]["timezone_offset_minutes"],
                int,
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_calendar"], "gregory")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_numbering_system"], "latn")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_hour_cycle"], "h12")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_collator_locale"], "en-US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_plural_rules_locale"], "en-US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_relative_time_format_locale"], "en-US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_intl_locale_region"], "US")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_navigator_language_region"], "US")
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["runtime_locale_week_info"],
                {"firstDay": 7, "weekend": [6, 7]},
            )
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["runtime_locale_text_info"],
                {"direction": "ltr"},
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_locale_hour_cycles"], ["h12"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_locale_calendars"], ["gregory"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_locale_numbering_systems"], ["latn"])
            self.assertTrue(
                verification_report["enforced_features"]["automation"]["runtime_locale_timezones_include_current_timezone"]
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_number_format"], "1,234,567.89")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_currency_format"], "$1,234.56")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_date_format"], "5/22/26")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_region_display_name"], "United States")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_list_format"], "A, B, and C")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_accept_language_header"], "en-US,en;q=0.9")
            self.assertTrue(verification_report["enforced_features"]["automation"]["runtime_sec_ch_ua_matches_chrome_major"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_sec_ch_ua_platform"], "macOS")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_sec_ch_ua_mobile"], "?0")
            self.assertTrue(
                verification_report["enforced_features"]["automation"]["runtime_user_agent_data_brands_match_chrome_major"]
            )
            self.assertTrue(
                verification_report["enforced_features"]["automation"][
                    "runtime_user_agent_high_entropy_matches_desktop_surface"
                ]
            )
            self.assertTrue(verification_report["enforced_features"]["automation"]["runtime_desktop_surface"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["runtime_pdf_viewer_enabled"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["runtime_plugins_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_webdriver"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_geolocation_permission"], "denied")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_notification_permission"], "denied")
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_speech_synthesis_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_local_font_access_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_bluetooth_api_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_usb_api_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_hid_api_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_webxr_api_present"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_gamepad_count"], 0)
            self.assertTrue(verification_report["enforced_features"]["automation"]["runtime_webrtc_supported"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["runtime_webrtc_local_ip_leak"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_webrtc_policy"], "disable_non_proxied_udp")
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_geolocation_setting"], 2)
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_camera_setting"], 2)
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_microphone_setting"], 2)
            self.assertEqual(verification_report["enforced_features"]["automation"]["runtime_profile_notifications_setting"], 2)
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["user_agent"],
                automation_context["browser_context"]["user_agent"],
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["client_hints_platform"], "macOS")
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_sec_ch_ua_matches_chrome_major"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_sec_ch_ua_platform"], "macOS")
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_sec_ch_ua_mobile"], "?0")
            self.assertTrue(
                verification_report["enforced_features"]["automation"][
                    "automation_user_agent_data_brands_match_chrome_major"
                ]
            )
            self.assertTrue(
                verification_report["enforced_features"]["automation"][
                    "automation_user_agent_high_entropy_matches_desktop_surface"
                ]
            )
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_webdriver"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["navigator_platform"], "MacIntel")
            self.assertEqual(verification_report["enforced_features"]["automation"]["desktop_viewport"], {"width": 1365, "height": 768})
            self.assertEqual(verification_report["enforced_features"]["automation"]["desktop_screen"], {"width": 1365, "height": 768})
            self.assertEqual(verification_report["enforced_features"]["automation"]["desktop_device_scale_factor"], 1)
            self.assertFalse(verification_report["enforced_features"]["automation"]["desktop_is_mobile"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["desktop_has_touch"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["desktop_color_scheme"], "light")
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_hardware_concurrency"], 8)
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_device_memory_gb"], 8)
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_webgl_vendor"], "Intel Inc.")
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_webgl_renderer"],
                "Intel Iris OpenGL Engine",
            )
            self.assertIn(
                "PingFang SC",
                verification_report["enforced_features"]["automation"]["automation_blocked_font_families"],
            )
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_font_metric_fallback_family"],
                "Arial",
            )
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_keyboard_layout_map"]["KeyZ"],
                "z",
            )
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_locale_week_info"],
                {"firstDay": 7, "weekend": [6, 7]},
            )
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_locale_text_info"],
                {"direction": "ltr"},
            )
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_locale_hour_cycles"], ["h12"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_locale_calendars"], ["gregory"])
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_locale_numbering_systems"],
                ["latn"],
            )
            self.assertTrue(
                verification_report["enforced_features"]["automation"]["automation_locale_timezones_include_current_timezone"]
            )
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_pdf_viewer_enabled"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_plugins_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_speech_synthesis_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_local_font_access_present"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_webgpu_adapter_available"])
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_hardware_device_apis"],
                {
                    "bluetooth": False,
                    "usb": False,
                    "hid": False,
                    "serial": "present_without_authorized_ports",
                },
            )
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["automation_hardware_device_counts"],
                {"bluetooth": 0, "usb": 0, "hid": 0, "serial": 0},
            )
            self.assertFalse(
                verification_report["enforced_features"]["automation"]["automation_hardware_device_identity_exposed"]
            )
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_webxr_api_present"])
            self.assertEqual(verification_report["enforced_features"]["automation"]["automation_gamepad_count"], 0)
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_gamepad_identity_exposed"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_midi_permission_not_granted"])
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_midi_identity_exposed"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_sensor_permissions_not_granted"])
            self.assertTrue(
                verification_report["enforced_features"]["automation"][
                    "automation_window_management_permission_not_granted"
                ]
            )
            self.assertFalse(
                verification_report["enforced_features"]["automation"]["automation_screen_details_identity_exposed"]
            )
            self.assertFalse(verification_report["enforced_features"]["automation"]["automation_media_device_identity_exposed"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_camera_permission_not_granted"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_microphone_permission_not_granted"])
            self.assertTrue(verification_report["enforced_features"]["automation"]["automation_notifications_permission_not_granted"])
            self.assertFalse(verification_report["enforced_features"]["webrtc"]["runtime_local_ip_leak"])
            self.assertEqual(verification_report["enforced_features"]["webrtc"]["runtime_private_ip_candidate_count"], 0)
            self.assertTrue(verification_report["enforced_features"]["network_path"]["quic_disabled"])
            self.assertTrue(verification_report["enforced_features"]["network_path"]["dns_prefetch_disabled"])
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["managed_user_data_dir"],
                "$AGENT_MANAGER_BROWSER_US_USER_DATA_DIR",
            )
            self.assertEqual(launcher_path.stat().st_mode & 0o777, 0o755)
            self.assertIn("export TZ=America/Los_Angeles", launcher_content)
            self.assertIn("AGENT_MANAGER_BROWSER_US_DEFAULT_URL=https://www.google.com/ncr", launcher_content)
            self.assertIn('if [ "$#" -eq 0 ]; then', launcher_content)
            self.assertIn("--lang=en-US", launcher_content)
            self.assertIn("--window-size=1365,768", launcher_content)
            self.assertIn("--force-device-scale-factor=1", launcher_content)
            self.assertIn("--user-agent=", launcher_content)
            self.assertIn("AGENT_MANAGER_BROWSER_US_USER_AGENT", launcher_content)
            self.assertIn('--user-data-dir="$BROWSER_US_USER_DATA_DIR"', launcher_content)
            self.assertIn("--disable-sync", launcher_content)
            self.assertIn("--disable-extensions", launcher_content)
            self.assertIn("--disable-speech-api", launcher_content)
            self.assertIn("--deny-permission-prompts", launcher_content)
            self.assertIn(
                "--disable-features=ReduceAcceptLanguage,WebGPU,FontAccess,WebBluetooth,WebUSB,WebHID,WebXR,Serial",
                launcher_content,
            )
            self.assertIn("--disable-blink-features=AutomationControlled", launcher_content)
            self.assertIn("--disable-quic", launcher_content)
            self.assertIn("--dns-prefetch-disable", launcher_content)
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
            self.assertEqual(verification_report["browser_google"]["verification_state"], "exit_profile_json_skipped")
            self.assertEqual(verification_report["runtime"]["verification_state"], "exit_profile_json_skipped")
            self.assertEqual(verification_report["automation_context"]["verification_state"], "exit_profile_json_skipped")
            self.assertTrue(verification_report["browser_state_audit"]["overall_ok"])
            self.assertEqual(verification_report["exit"]["profile"]["ip"], "203.0.113.31")
            self.assertEqual(verification_report["enforced_features"]["automation"]["timezone_id"], "America/New_York")
            self.assertEqual(
                verification_report["enforced_features"]["automation"]["context_geolocation"],
                {"latitude": 40.7128, "longitude": -74.006, "accuracy": 50000},
            )
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
        spec = importlib.util.spec_from_file_location("sync_browser_google_us_test", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        try:
            spec.loader.exec_module(module)
        finally:
            sys.modules.pop(spec.name, None)
        timezone_offset = module.expected_timezone_offset_minutes("America/Los_Angeles")
        runtime_user_agent = (
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
            "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
        )
        payload = (
            "<html><head><script>document.write('AGENT_MANAGER_RUNTIME_JSON:'"
            "+JSON.stringify({language:navigator.language}));</script></head>"
            "<body>AGENT_MANAGER_RUNTIME_JSON:"
            '{"userAgent":"'
            + runtime_user_agent
            + '","userAgentHeader":"'
            + runtime_user_agent
            + '","platform":"MacIntel","userAgentDataMobile":false,"userAgentDataPlatform":"macOS",'
            '"userAgentDataBrands":[{"brand":"Chromium","version":"120"},{"brand":"Google Chrome","version":"120"}],'
            '"userAgentDataHighEntropy":{'
            '"architecture":"","bitness":"","fullVersionList":[],"mobile":false,'
            '"model":"","platform":"macOS","platformVersion":"","uaFullVersion":"","wow64":false},'
            '"secChUa":"\\"Chromium\\";v=\\"120\\", \\"Google Chrome\\";v=\\"120\\", \\"Not/A)Brand\\";v=\\"99\\"",'
            '"secChUaPlatform":"\\"macOS\\"","secChUaMobile":"?0",'
            '"webdriver":false,'
            '"language":"en-US","languages":["en-US","en"],'
            '"innerWidth":1365,"innerHeight":629,"outerWidth":0,"outerHeight":0,'
            '"screenWidth":800,"screenHeight":600,"availWidth":800,"availHeight":600,'
            '"devicePixelRatio":1,"maxTouchPoints":0,"coarsePointer":false,'
            '"finePointer":true,"hoverHover":true,"lightColorScheme":true,"darkColorScheme":false,'
            '"pdfViewerEnabled":true,"pluginsLength":5,"mimeTypesLength":2,'
            '"speechSynthesisPresent":false,"localFontAccessPresent":false,'
            '"bluetoothPresent":false,"usbPresent":false,"hidPresent":false,"serialPresent":true,'
            '"xrPresent":false,"gamepadCount":0,'
            '"timezone":"America/Los_Angeles","timezoneOffset":'
            + str(timezone_offset)
            + ',"intlLocale":"en-US",'
            '"intlCalendar":"gregory","intlNumberingSystem":"latn","intlHourCycle":"h12",'
            '"collatorLocale":"en-US","pluralRulesLocale":"en-US","relativeTimeFormatLocale":"en-US",'
            '"intlLocaleRegion":"US","navigatorLanguageRegion":"US",'
            '"localeWeekInfo":{"firstDay":7,"weekend":[6,7]},'
            '"localeTextInfo":{"direction":"ltr"},'
            '"localeHourCycles":["h12"],'
            '"localeCalendars":["gregory"],'
            '"localeNumberingSystems":["latn"],'
            '"localeTimeZones":["America/Los_Angeles","America/New_York"],'
            '"numberFormat":"1,234,567.89","listFormat":"A, B, and C",'
            '"currencyFormat":"$1,234.56","dateFormat":"5/22/26","regionDisplayName":"United States",'
            '"acceptLanguageHeader":"en-US,en;q=0.9",'
            '"geolocationPermission":"denied",'
            '"notificationPermission":"denied",'
            '"webrtcSupported":true,'
            '"webrtcCandidateCount":0,'
            '"webrtcHostCandidateCount":0,'
            '"webrtcMdnsCandidateCount":0,'
            '"webrtcPrivateIpCandidateCount":0,'
            '"webrtcLocalIpLeak":false,'
            '"profileWebrtcPolicy":"disable_non_proxied_udp",'
            '"profileWebrtcNonproxiedUdpEnabled":false,'
            '"profileGeolocationSetting":2,'
            '"profileCameraSetting":2,'
            '"profileMicrophoneSetting":2,'
            '"profileNotificationsSetting":2}'
            "</body></html>"
        )
        runtime_context = module.runtime_context_from_output(payload)
        mismatches = module.validate_runtime_context(runtime_context, "America/Los_Angeles", runtime_user_agent, "macOS")

        self.assertEqual(runtime_context["userAgent"], runtime_user_agent)
        self.assertEqual(runtime_context["userAgentHeader"], runtime_user_agent)
        self.assertEqual(runtime_context["platform"], "MacIntel")
        self.assertFalse(runtime_context["userAgentDataMobile"])
        self.assertEqual(runtime_context["userAgentDataPlatform"], "macOS")
        self.assertEqual(runtime_context["userAgentDataBrands"][0], {"brand": "Chromium", "version": "120"})
        self.assertEqual(runtime_context["userAgentDataHighEntropy"]["platformVersion"], "")
        self.assertIn('"Google Chrome";v="120"', runtime_context["secChUa"])
        self.assertFalse(runtime_context["webdriver"])
        self.assertEqual(runtime_context["language"], "en-US")
        self.assertEqual(runtime_context["languages"], ["en-US", "en"])
        self.assertEqual(runtime_context["innerWidth"], 1365)
        self.assertEqual(runtime_context["devicePixelRatio"], 1)
        self.assertFalse(runtime_context["coarsePointer"])
        self.assertTrue(runtime_context["finePointer"])
        self.assertTrue(runtime_context["hoverHover"])
        self.assertTrue(runtime_context["pdfViewerEnabled"])
        self.assertEqual(runtime_context["pluginsLength"], 5)
        self.assertEqual(runtime_context["mimeTypesLength"], 2)
        self.assertFalse(runtime_context["speechSynthesisPresent"])
        self.assertFalse(runtime_context["localFontAccessPresent"])
        self.assertFalse(runtime_context["bluetoothPresent"])
        self.assertFalse(runtime_context["usbPresent"])
        self.assertFalse(runtime_context["hidPresent"])
        self.assertTrue(runtime_context["serialPresent"])
        self.assertFalse(runtime_context["xrPresent"])
        self.assertEqual(runtime_context["gamepadCount"], 0)
        self.assertEqual(runtime_context["timezone"], "America/Los_Angeles")
        self.assertEqual(runtime_context["timezoneOffset"], timezone_offset)
        self.assertEqual(runtime_context["intlCalendar"], "gregory")
        self.assertEqual(runtime_context["intlNumberingSystem"], "latn")
        self.assertEqual(runtime_context["intlHourCycle"], "h12")
        self.assertEqual(runtime_context["collatorLocale"], "en-US")
        self.assertEqual(runtime_context["pluralRulesLocale"], "en-US")
        self.assertEqual(runtime_context["relativeTimeFormatLocale"], "en-US")
        self.assertEqual(runtime_context["intlLocaleRegion"], "US")
        self.assertEqual(runtime_context["navigatorLanguageRegion"], "US")
        self.assertEqual(runtime_context["localeWeekInfo"], {"firstDay": 7, "weekend": [6, 7]})
        self.assertEqual(runtime_context["localeTextInfo"], {"direction": "ltr"})
        self.assertEqual(runtime_context["localeHourCycles"], ["h12"])
        self.assertEqual(runtime_context["localeCalendars"], ["gregory"])
        self.assertEqual(runtime_context["localeNumberingSystems"], ["latn"])
        self.assertIn("America/Los_Angeles", runtime_context["localeTimeZones"])
        self.assertEqual(runtime_context["numberFormat"], "1,234,567.89")
        self.assertEqual(runtime_context["currencyFormat"], "$1,234.56")
        self.assertEqual(runtime_context["dateFormat"], "5/22/26")
        self.assertEqual(runtime_context["regionDisplayName"], "United States")
        self.assertEqual(runtime_context["listFormat"], "A, B, and C")
        self.assertEqual(runtime_context["acceptLanguageHeader"], "en-US,en;q=0.9")
        self.assertEqual(runtime_context["geolocationPermission"], "denied")
        self.assertEqual(runtime_context["notificationPermission"], "denied")
        self.assertTrue(runtime_context["webrtcSupported"])
        self.assertEqual(runtime_context["webrtcCandidateCount"], 0)
        self.assertEqual(runtime_context["webrtcHostCandidateCount"], 0)
        self.assertEqual(runtime_context["webrtcMdnsCandidateCount"], 0)
        self.assertEqual(runtime_context["webrtcPrivateIpCandidateCount"], 0)
        self.assertFalse(runtime_context["webrtcLocalIpLeak"])
        self.assertEqual(runtime_context["profileWebrtcPolicy"], "disable_non_proxied_udp")
        self.assertFalse(runtime_context["profileWebrtcNonproxiedUdpEnabled"])
        self.assertEqual(runtime_context["profileGeolocationSetting"], 2)
        self.assertEqual(runtime_context["profileCameraSetting"], 2)
        self.assertEqual(runtime_context["profileMicrophoneSetting"], 2)
        self.assertEqual(runtime_context["profileNotificationsSetting"], 2)
        self.assertEqual(mismatches, ())

    def test_automation_context_validator_rejects_non_us_context_drift(self):
        spec = importlib.util.spec_from_file_location("sync_browser_google_us_test", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        try:
            spec.loader.exec_module(module)
        finally:
            sys.modules.pop(spec.name, None)

        browser_context = {
            "locale": "en-US",
            "timezone_id": "America/Los_Angeles",
            "user_agent": (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
            ),
            "navigator_languages": ["en-US", "en"],
            "client_hints_platform": "macOS",
            "navigator_platform": "MacIntel",
            "geolocation": {"latitude": 34.0522, "longitude": -118.2437, "accuracy": 50000},
            "viewport": {"width": 1365, "height": 768},
            "screen": {"width": 1365, "height": 768},
            "device_scale_factor": 1,
            "is_mobile": False,
            "has_touch": False,
            "color_scheme": "light",
            "hardware_concurrency": 8,
            "device_memory_gb": 8,
            "webgl_vendor": "Intel Inc.",
            "webgl_renderer": "Intel Iris OpenGL Engine",
            "blocked_font_families": ["PingFang SC", "MingLiU"],
            "font_metric_fallback_family": "Arial",
            "keyboard_layout_map": {"KeyQ": "q", "KeyZ": "z"},
        }
        observed = {
            "userAgent": browser_context["user_agent"],
            "userAgentHeader": browser_context["user_agent"],
            "platform": "MacIntel",
            "userAgentDataMobile": False,
            "userAgentDataPlatform": "macOS",
            "userAgentDataBrands": [{"brand": "Chromium", "version": "120"}, {"brand": "Google Chrome", "version": "120"}],
            "userAgentDataHighEntropy": {
                "architecture": "x86",
                "bitness": "",
                "fullVersionList": [],
                "mobile": False,
                "model": "",
                "platform": "macOS",
                "platformVersion": "10_15_7",
                "uaFullVersion": "",
                "wow64": False,
            },
            "secChUa": '"Chromium";v="120", "Google Chrome";v="120", "Not/A)Brand";v="99"',
            "secChUaPlatform": '"macOS"',
            "secChUaMobile": "?0",
            "webdriver": False,
            "language": "en-US",
            "languages": ["en-US", "en"],
            "timezone": "America/Los_Angeles",
            "timezoneOffset": module.expected_timezone_offset_minutes("America/Los_Angeles"),
            "intlLocale": "en-US",
            "collatorLocale": "en-US",
            "pluralRulesLocale": "en-US",
            "relativeTimeFormatLocale": "en-US",
            "localeWeekInfo": {"firstDay": 7, "weekend": [6, 7]},
            "localeTextInfo": {"direction": "ltr"},
            "localeHourCycles": ["h12"],
            "localeCalendars": ["gregory"],
            "localeNumberingSystems": ["latn"],
            "localeTimeZones": ["America/Los_Angeles", "America/New_York"],
            "dateFormat": "5/22/26",
            "currencyFormat": "$1,234.56",
            "regionDisplayName": "United States",
            "acceptLanguageHeader": "en-US,en;q=0.9",
            "innerWidth": 1365,
            "innerHeight": 768,
            "screenWidth": 1365,
            "screenHeight": 768,
            "devicePixelRatio": 1,
            "maxTouchPoints": 0,
            "coarsePointer": False,
            "finePointer": True,
            "hoverHover": True,
            "lightColorScheme": True,
            "darkColorScheme": False,
            "hardwareConcurrency": 8,
            "deviceMemory": 8,
            "webglVendor": "Intel Inc.",
            "webglRenderer": "Intel Iris OpenGL Engine",
            "fontChecks": {
                "blocked": {"PingFang SC": False, "MingLiU": False},
                "metricMatches": {"PingFang SC": True, "MingLiU": True},
            },
            "keyboardLayout": {"KeyQ": "q", "KeyZ": "z"},
            "pdfViewerEnabled": True,
            "pluginsLength": 5,
            "mimeTypesLength": 2,
            "speechSynthesisPresent": False,
            "localFontAccessPresent": False,
            "webGpuAdapterAvailable": False,
            "hardwareDeviceApis": {"bluetooth": False, "usb": False, "hid": False, "serial": True},
            "hardwareDeviceCounts": {"bluetooth": 0, "usb": 0, "hid": 0, "serial": 0},
            "hardwareDeviceIdentityExposed": False,
            "hardwareDeviceProbeErrors": {},
            "xrPresent": False,
            "gamepadCount": 0,
            "gamepadIdentityExposed": False,
            "midiPermission": "denied",
            "midiInputCount": 0,
            "midiOutputCount": 0,
            "midiIdentityExposed": False,
            "midiProbeError": None,
            "accelerometerPermission": "denied",
            "gyroscopePermission": "denied",
            "magnetometerPermission": "denied",
            "windowManagementPermission": "denied",
            "screenDetailsIdentityExposed": False,
            "screenDetailsProbeError": None,
            "mediaDeviceIdentityExposed": False,
            "cameraPermission": "prompt",
            "microphonePermission": "prompt",
            "notificationsPermission": "prompt",
            "notificationPermission": "default",
            "geolocation": {"latitude": 34.0522, "longitude": -118.2437, "accuracy": 50000},
        }

        self.assertEqual(module.validate_automation_context_observation(observed, browser_context), ())

        observed["timezone"] = "Asia/Hong_Kong"
        observed["timezoneOffset"] = module.expected_timezone_offset_minutes("Asia/Hong_Kong")
        observed["geolocation"] = {"latitude": 22.3193, "longitude": 114.1694, "accuracy": 50000}
        mismatches = module.validate_automation_context_observation(observed, browser_context)

        self.assertIn("automation timezone", mismatches)
        self.assertIn("automation timezone offset", mismatches)
        self.assertIn("automation geolocation latitude", mismatches)
        self.assertIn("automation geolocation longitude", mismatches)

        observed["timezone"] = "America/Los_Angeles"
        observed["timezoneOffset"] = module.expected_timezone_offset_minutes("America/Los_Angeles")
        observed["geolocation"] = {"latitude": 34.0522, "longitude": -118.2437, "accuracy": 50000}
        observed["platform"] = "Win32"
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation navigator.platform", mismatches)

        observed["platform"] = "MacIntel"
        observed["userAgentDataBrands"] = [{"brand": "Chromium", "version": "99"}]
        observed["secChUa"] = '"Chromium";v="99"'
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation user agent brands", mismatches)
        self.assertIn("automation Sec-CH-UA", mismatches)

        observed["userAgentDataBrands"] = [
            {"brand": "Chromium", "version": "120"},
            {"brand": "Google Chrome", "version": "120"},
        ]
        observed["secChUa"] = '"Chromium";v="120", "Google Chrome";v="120", "Not/A)Brand";v="99"'
        observed["secChUaPlatform"] = '"Windows"'
        observed["secChUaMobile"] = "?1"
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation Sec-CH-UA-Platform", mismatches)
        self.assertIn("automation Sec-CH-UA-Mobile", mismatches)

        observed["secChUaPlatform"] = '"macOS"'
        observed["secChUaMobile"] = "?0"
        observed["userAgentDataHighEntropy"] = {
            "architecture": "x86",
            "bitness": "",
            "fullVersionList": [],
            "mobile": False,
            "model": "",
            "platform": "macOS",
            "platformVersion": "15_0_0",
            "uaFullVersion": "",
            "wow64": False,
        }
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation user agent high entropy", mismatches)

        observed["userAgentDataHighEntropy"] = {
            "architecture": "x86",
            "bitness": "",
            "fullVersionList": [],
            "mobile": False,
            "model": "",
            "platform": "macOS",
            "platformVersion": "10_15_7",
            "uaFullVersion": "",
            "wow64": False,
        }
        observed["webdriver"] = True
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation webdriver", mismatches)

        observed["webdriver"] = False
        observed["localeWeekInfo"] = {"firstDay": 1, "weekend": [6, 7]}
        observed["localeTimeZones"] = ["Asia/Hong_Kong"]
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation Intl locale week info", mismatches)
        self.assertIn("automation Intl locale time zones", mismatches)

        observed["localeWeekInfo"] = {"firstDay": 7, "weekend": [6, 7]}
        observed["localeTimeZones"] = ["America/Los_Angeles", "America/New_York"]
        observed["innerWidth"] = 390
        observed["maxTouchPoints"] = 5
        observed["coarsePointer"] = True
        observed["finePointer"] = False
        observed["hoverHover"] = False
        observed["hardwareConcurrency"] = 2
        observed["deviceMemory"] = 4
        observed["webglVendor"] = "Google Inc. (Apple)"
        observed["webglRenderer"] = "ANGLE Metal Renderer"
        observed["fontChecks"] = {
            "blocked": {"PingFang SC": True, "MingLiU": False},
            "metricMatches": {"PingFang SC": False, "MingLiU": True},
        }
        observed["pluginsLength"] = 0
        observed["speechSynthesisPresent"] = True
        observed["localFontAccessPresent"] = True
        observed["keyboardLayout"] = {"KeyQ": "a", "KeyZ": "w"}
        observed["webGpuAdapterAvailable"] = True
        observed["hardwareDeviceApis"] = {"bluetooth": True, "usb": True, "hid": True, "serial": True}
        observed["hardwareDeviceCounts"] = {"bluetooth": 1, "usb": 1, "hid": 1, "serial": 1}
        observed["hardwareDeviceIdentityExposed"] = True
        observed["hardwareDeviceProbeErrors"] = {}
        observed["xrPresent"] = True
        observed["gamepadCount"] = 1
        observed["gamepadIdentityExposed"] = True
        observed["midiPermission"] = "granted"
        observed["midiInputCount"] = 1
        observed["midiOutputCount"] = 1
        observed["midiIdentityExposed"] = True
        observed["windowManagementPermission"] = "granted"
        observed["screenDetailsIdentityExposed"] = True
        observed["accelerometerPermission"] = "granted"
        observed["gyroscopePermission"] = "granted"
        observed["magnetometerPermission"] = "granted"
        observed["mediaDeviceIdentityExposed"] = True
        mismatches = module.validate_automation_context_observation(observed, browser_context)

        self.assertIn("automation viewport width", mismatches)
        self.assertIn("automation touch points", mismatches)
        self.assertIn("automation coarse pointer", mismatches)
        self.assertIn("automation fine pointer", mismatches)
        self.assertIn("automation hover", mismatches)
        self.assertIn("automation hardware concurrency", mismatches)
        self.assertIn("automation device memory", mismatches)
        self.assertIn("automation WebGL vendor", mismatches)
        self.assertIn("automation WebGL renderer", mismatches)
        self.assertIn("automation blocked font: PingFang SC", mismatches)
        self.assertIn("automation blocked font metric: PingFang SC", mismatches)
        self.assertIn("automation plugins", mismatches)
        self.assertIn("automation speech synthesis", mismatches)
        self.assertIn("automation local font access", mismatches)
        self.assertIn("automation keyboard layout", mismatches)
        self.assertIn("automation WebGPU adapter", mismatches)
        self.assertIn("automation bluetooth API", mismatches)
        self.assertIn("automation usb API", mismatches)
        self.assertIn("automation hid API", mismatches)
        self.assertIn("automation bluetooth device count", mismatches)
        self.assertIn("automation usb device count", mismatches)
        self.assertIn("automation hid device count", mismatches)
        self.assertIn("automation serial device count", mismatches)
        self.assertIn("automation hardware device identity", mismatches)
        self.assertIn("automation WebXR API", mismatches)
        self.assertIn("automation gamepad count", mismatches)
        self.assertIn("automation gamepad identity", mismatches)
        self.assertIn("automation MIDI permission", mismatches)
        self.assertIn("automation MIDI input count", mismatches)
        self.assertIn("automation MIDI output count", mismatches)
        self.assertIn("automation MIDI identity", mismatches)
        self.assertIn("automation window management permission", mismatches)
        self.assertIn("automation screen details identity", mismatches)
        self.assertIn("automation accelerometerPermission", mismatches)
        self.assertIn("automation gyroscopePermission", mismatches)
        self.assertIn("automation magnetometerPermission", mismatches)
        self.assertIn("automation media device identity", mismatches)

        observed["innerWidth"] = 1365
        observed["maxTouchPoints"] = 0
        observed["coarsePointer"] = False
        observed["finePointer"] = True
        observed["hoverHover"] = True
        observed["hardwareConcurrency"] = 8
        observed["deviceMemory"] = 8
        observed["webglVendor"] = "Intel Inc."
        observed["webglRenderer"] = "Intel Iris OpenGL Engine"
        observed["fontChecks"] = {
            "blocked": {"PingFang SC": False, "MingLiU": False},
            "metricMatches": {"PingFang SC": True, "MingLiU": True},
        }
        observed["pluginsLength"] = 5
        observed["speechSynthesisPresent"] = False
        observed["localFontAccessPresent"] = False
        observed["keyboardLayout"] = {"KeyQ": "q", "KeyZ": "z"}
        observed["webGpuAdapterAvailable"] = False
        observed["hardwareDeviceApis"] = {"bluetooth": False, "usb": False, "hid": False, "serial": True}
        observed["hardwareDeviceCounts"] = {"bluetooth": 0, "usb": 0, "hid": 0, "serial": 0}
        observed["hardwareDeviceIdentityExposed"] = False
        observed["xrPresent"] = False
        observed["gamepadCount"] = 0
        observed["gamepadIdentityExposed"] = False
        observed["midiPermission"] = "denied"
        observed["midiInputCount"] = 0
        observed["midiOutputCount"] = 0
        observed["midiIdentityExposed"] = False
        observed["windowManagementPermission"] = "denied"
        observed["screenDetailsIdentityExposed"] = False
        observed["accelerometerPermission"] = "denied"
        observed["gyroscopePermission"] = "denied"
        observed["magnetometerPermission"] = "denied"
        observed["mediaDeviceIdentityExposed"] = False
        observed["cameraPermission"] = "granted"
        observed["languages"] = ["en-US"]
        mismatches = module.validate_automation_context_observation(observed, browser_context)
        self.assertIn("automation cameraPermission", mismatches)
        self.assertIn("automation navigator.languages", mismatches)

    def test_browser_google_marker_detection_catches_hk_signals(self):
        spec = importlib.util.spec_from_file_location("sync_browser_google_us_test", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        try:
            spec.loader.exec_module(module)
        finally:
            sys.modules.pop(spec.name, None)

        self.assertEqual(
            module.google_hk_markers_from_text(
                "https://www.google.com/?hl=en-US&gl=US&pws=0",
                "<html><a href='https://www.google.com.hk/imghp'>Images</a> Hong Kong</html>",
            ),
            ("google.com.hk", "hong kong"),
        )
        self.assertEqual(module.google_hk_markers_from_text("https://www.google.com/", "Google"), ())
