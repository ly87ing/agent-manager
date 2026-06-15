"""Extracted from sync_browser_google_us.py: verify_report layer."""
from __future__ import annotations

import argparse
import datetime
import http.cookiejar
import http.server
import json
import os
import re
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from browser_us.constants import *  # noqa: F401,F403
from browser_us.model import *  # noqa: F401,F403
from browser_us.helpers import *  # noqa: F401,F403
from browser_us.sync_audit import *  # noqa: F401,F403
from browser_us.chrome import *  # noqa: F401,F403


def verification_report_payload(
    browser_roots: list[BrowserRoot],
    managed_browser_root: BrowserRoot,
    explicit_profile: str | None,
    exit_profile: ExitProfile | None,
    platform_name: str,
    verification_state: str,
    google_domain_resolution: GoogleDomainResolution,
    browser_google_resolution: BrowserGoogleResolution,
    runtime_resolution: RuntimeResolution,
    automation_context_resolution: AutomationContextResolution,
    browser_state_audit: dict,
) -> dict:
    timezone_id = resolved_timezone_id(exit_profile)
    geolocation = resolved_geolocation(exit_profile)
    user_agent = resolved_us_desktop_user_agent(platform_name)
    expected_client_hints_platform = client_hints_platform(platform_name)
    expected_navigator_platform = navigator_platform(platform_name)
    timezone_offset_minutes = expected_timezone_offset_minutes(timezone_id)
    timezone_probe_offset_label = timezone_offset_label(
        expected_timezone_offset_minutes_at(timezone_id, US_TIMEZONE_PROBE_UTC)
    )
    timezone_display_needles = list(us_timezone_display_needles(timezone_id))
    state_ok = browser_state_audit.get("overall_ok") is True
    google_domain_ok = google_domain_resolution.return_code == 0
    browser_google_ok = browser_google_resolution.return_code == 0
    runtime_ok = runtime_resolution.return_code == 0
    automation_context_ok = automation_context_resolution.return_code == 0
    if (
        verification_state == "verified_us"
        and state_ok
        and google_domain_ok
        and browser_google_ok
        and runtime_ok
        and automation_context_ok
    ):
        overall_status = "complete"
    elif state_ok and google_domain_ok and browser_google_ok and runtime_ok and automation_context_ok:
        overall_status = "limited"
    else:
        overall_status = "failed"
    return {
        "version": VERIFICATION_REPORT_VERSION,
        "overall_status": overall_status,
        "exit": {
            "verification_state": verification_state,
            "profile": exit_profile_payload(exit_profile),
        },
        "google_domain": google_domain_payload(google_domain_resolution),
        "browser_google": browser_google_payload(browser_google_resolution),
        "runtime": runtime_payload(runtime_resolution),
        "automation_context": automation_context_verification_payload(automation_context_resolution),
        "browser_state_audit": browser_state_audit,
        "browser_roots": [
            {
                "name": browser_root.name,
                "user_data_dir": str(browser_root.user_data_dir),
                "profile_scope": explicit_profile or "all regular profiles",
            }
            for browser_root in browser_roots
        ]
        + [
            {
                "name": managed_browser_root.name,
                "user_data_dir": str(managed_browser_root.user_data_dir),
                "profile_scope": "Default",
            }
        ],
        "enforced_features": {
            "locale": {
                "app_locale": APP_LOCALE,
                "accept_languages": ACCEPT_LANGUAGES,
                "selected_languages": ACCEPT_LANGUAGES,
                "account_accept_languages": ACCEPT_LANGUAGES,
                "account_selected_languages": ACCEPT_LANGUAGES,
                "reduced_accept_language_exceptions": {},
                "disabled_chrome_features": list(DISABLED_CHROME_FEATURES),
                "environment_locale": ENV_LOCALE,
            },
            "local_state_country": {
                "managed_profile_countryid_at_install": COUNTRY_ID_US,
                "managed_profile_variations_country": VARIATIONS_COUNTRY,
                "managed_profile_variations_safe_seed_locale": APP_LOCALE,
                "managed_profile_variations_safe_seed_permanent_consistency_country": VARIATIONS_COUNTRY,
                "managed_profile_variations_safe_seed_session_consistency_country": VARIATIONS_COUNTRY,
                "regular_profile_country_fields": "not forced; active Chrome may rewrite Local State country fields",
            },
            "profile_country": {
                "managed_profile_countryid_at_install": COUNTRY_ID_US,
                "regular_profile_countryid_at_install": "not forced; active Chrome may rewrite this volatile install-time field",
            },
            "managed_profile_storage": {
                "volatile_state_policy": "cleared_on_every_sync",
                "cleared_relative_paths": list(MANAGED_PROFILE_VOLATILE_STATE_RELATIVE_PATHS),
            },
            "translate": {
                "recent_target": TRANSLATE_TARGET_LANGUAGE,
                "account_recent_target": TRANSLATE_TARGET_LANGUAGE,
            },
            "google": {
                "ncr_url": NCR_URL,
                "home_url": GOOGLE_HOME_URL,
                "search_url": SEARCH_URL,
                "suggest_url": SUGGEST_URL,
                "search_gl": "US",
                "search_hl": "en-US",
                "hk_markers_blocked_after_ncr": list(GOOGLE_HK_MARKERS),
                "browser_google_after_ncr": GOOGLE_HOME_URL,
            },
            "webrtc": {
                "ip_handling_policy": "disable_non_proxied_udp",
                "multiple_routes_enabled": False,
                "nonproxied_udp_enabled": False,
                "chrome_launch_policy": "disable_non_proxied_udp",
                "runtime_local_ip_leak": False,
                "runtime_private_ip_candidate_count": 0,
            },
            "network_path": {
                "quic_disabled": True,
                "dns_prefetch_disabled": True,
                "purpose": "reduce browser-side UDP and speculative DNS paths outside the selected proxy/VPN route",
            },
            "geolocation": {
                "regular_profile_content_setting": BLOCK_CONTENT_SETTING,
                "regular_profile_policy": "block",
                "regular_profile_exceptions": {},
                "regular_profile_geolocation_with_options_exceptions": {},
                "automation_geolocation": geolocation,
            },
            "device_permissions": {
                "managed_profile_camera_content_setting": BLOCK_CONTENT_SETTING,
                "managed_profile_microphone_content_setting": BLOCK_CONTENT_SETTING,
                "managed_profile_notifications_content_setting": BLOCK_CONTENT_SETTING,
                "managed_profile_camera_exceptions": {},
                "managed_profile_microphone_exceptions": {},
                "managed_profile_notifications_exceptions": {},
                "permission_prompts": "denied_by_chrome_launch_arg",
                "regular_profile_policy": "not forced; active Chrome may recreate device permission defaults or site exceptions",
            },
            "automation": {
                "locale": APP_LOCALE,
                "timezone_id": timezone_id,
                "timezone_offset_minutes": timezone_offset_minutes,
                "timezone_probe_offset_label": timezone_probe_offset_label,
                "timezone_display_needles": timezone_display_needles,
                "context_geolocation": geolocation,
                "context_permissions": ["geolocation"],
                "runtime_language": APP_LOCALE,
                "runtime_languages": ["en-US", "en"],
                "automation_languages": ["en-US", "en"],
                "runtime_intl_locale": APP_LOCALE,
                "runtime_intl_calendar": "gregory",
                "runtime_intl_numbering_system": "latn",
                "runtime_intl_hour_cycle": "h12",
                "runtime_collator_locale": APP_LOCALE,
                "runtime_plural_rules_locale": APP_LOCALE,
                "runtime_relative_time_format_locale": APP_LOCALE,
                "runtime_intl_locale_region": "US",
                "runtime_navigator_language_region": "US",
                "runtime_locale_week_info": {"firstDay": 7, "weekend": [6, 7]},
                "runtime_locale_text_info": {"direction": "ltr"},
                "runtime_locale_hour_cycles": ["h12"],
                "runtime_locale_calendars": ["gregory"],
                "runtime_locale_numbering_systems": ["latn"],
                "runtime_locale_timezones_include_current_timezone": True,
                "runtime_number_format": "1,234,567.89",
                "runtime_currency_format": "$1,234.56",
                "runtime_date_format": "5/22/26",
                "runtime_region_display_name": "United States",
                "runtime_list_format": "A, B, and C",
                "runtime_accept_language_header": "en-US,en;q=0.9",
                "runtime_sec_ch_ua_matches_chrome_major": True,
                "runtime_sec_ch_ua_platform": expected_client_hints_platform,
                "runtime_sec_ch_ua_mobile": "?0",
                "runtime_user_agent_data_brands_match_chrome_major": True,
                "runtime_user_agent_high_entropy_matches_desktop_surface": True,
                "runtime_navigator_vendor": "Google Inc.",
                "runtime_navigator_product": "Gecko",
                "runtime_navigator_product_sub": "20030107",
                "runtime_navigator_app_code_name": "Mozilla",
                "runtime_navigator_app_name": "Netscape",
                "runtime_navigator_app_version": navigator_app_version_from_user_agent(user_agent),
                "runtime_desktop_surface": True,
                "chrome_window_size": dict(US_DESKTOP_CHROME_WINDOW),
                "runtime_screen": dict(US_DESKTOP_SCREEN),
                "runtime_screen_avail": dict(US_DESKTOP_SCREEN),
                "runtime_screen_color_depth": US_DESKTOP_SCREEN_COLOR_DEPTH,
                "runtime_screen_pixel_depth": US_DESKTOP_SCREEN_COLOR_DEPTH,
                "runtime_screen_orientation_type": US_DESKTOP_SCREEN_ORIENTATION_TYPE,
                "runtime_screen_orientation_angle": US_DESKTOP_SCREEN_ORIENTATION_ANGLE,
                "runtime_pdf_viewer_enabled": True,
                "runtime_plugins_present": True,
                "runtime_pdf_plugin_names": list(US_DESKTOP_PDF_PLUGIN_NAMES),
                "runtime_pdf_mime_types": list(US_DESKTOP_PDF_MIME_TYPES),
                "runtime_webdriver": False,
                "runtime_geolocation_permission": "denied",
                "runtime_notification_permission": "denied",
                "runtime_speech_synthesis_present": False,
                "runtime_local_font_access_present": False,
                "runtime_bluetooth_api_present": False,
                "runtime_usb_api_present": False,
                "runtime_hid_api_present": False,
                "runtime_webxr_api_present": False,
                "runtime_gamepad_count": 0,
                "runtime_webrtc_supported": True,
                "runtime_webrtc_local_ip_leak": False,
                "runtime_profile_webrtc_policy": "disable_non_proxied_udp",
                "runtime_profile_geolocation_setting": BLOCK_CONTENT_SETTING,
                "runtime_profile_camera_setting": BLOCK_CONTENT_SETTING,
                "runtime_profile_microphone_setting": BLOCK_CONTENT_SETTING,
                "runtime_profile_notifications_setting": BLOCK_CONTENT_SETTING,
                "extra_http_accept_language": "en-US,en;q=0.9",
                "user_agent": user_agent,
                "client_hints_platform": expected_client_hints_platform,
                "automation_sec_ch_ua_matches_chrome_major": True,
                "automation_sec_ch_ua_platform": expected_client_hints_platform,
                "automation_sec_ch_ua_mobile": "?0",
                "automation_user_agent_data_brands_match_chrome_major": True,
                "automation_user_agent_high_entropy_matches_desktop_surface": True,
                "automation_webdriver": False,
                "navigator_platform": expected_navigator_platform,
                "automation_navigator_vendor": "Google Inc.",
                "automation_navigator_product": "Gecko",
                "automation_navigator_product_sub": "20030107",
                "automation_navigator_app_code_name": "Mozilla",
                "automation_navigator_app_name": "Netscape",
                "automation_navigator_app_version": navigator_app_version_from_user_agent(user_agent),
                "desktop_viewport": dict(US_DESKTOP_VIEWPORT),
                "desktop_screen": dict(US_DESKTOP_SCREEN),
                "desktop_screen_avail": dict(US_DESKTOP_SCREEN),
                "desktop_screen_color_depth": US_DESKTOP_SCREEN_COLOR_DEPTH,
                "desktop_screen_pixel_depth": US_DESKTOP_SCREEN_COLOR_DEPTH,
                "desktop_screen_orientation_type": US_DESKTOP_SCREEN_ORIENTATION_TYPE,
                "desktop_screen_orientation_angle": US_DESKTOP_SCREEN_ORIENTATION_ANGLE,
                "desktop_visual_viewport": dict(US_DESKTOP_VIEWPORT),
                "desktop_visual_viewport_scale": 1,
                "desktop_device_scale_factor": US_DESKTOP_DEVICE_SCALE_FACTOR,
                "desktop_is_mobile": US_DESKTOP_IS_MOBILE,
                "desktop_has_touch": US_DESKTOP_HAS_TOUCH,
                "desktop_color_scheme": US_DESKTOP_COLOR_SCHEME,
                "automation_hardware_concurrency": US_DESKTOP_HARDWARE_CONCURRENCY,
                "automation_device_memory_gb": US_DESKTOP_DEVICE_MEMORY_GB,
                "automation_network_information": dict(US_DESKTOP_NETWORK_INFORMATION),
                "automation_webgl_vendor": US_DESKTOP_WEBGL_VENDOR,
                "automation_webgl_renderer": US_DESKTOP_WEBGL_RENDERER,
                "automation_blocked_font_families": list(BLOCKED_FONT_FAMILIES),
                "automation_font_metric_fallback_family": US_CANVAS_FONT_METRIC_FALLBACK_FAMILY,
                "automation_keyboard_layout_map": dict(US_KEYBOARD_LAYOUT_MAP),
                "automation_locale_week_info": {"firstDay": 7, "weekend": [6, 7]},
                "automation_locale_text_info": {"direction": "ltr"},
                "automation_locale_hour_cycles": ["h12"],
                "automation_locale_calendars": ["gregory"],
                "automation_locale_numbering_systems": ["latn"],
                "automation_locale_timezones_include_current_timezone": True,
                "automation_pdf_viewer_enabled": True,
                "automation_plugins_present": True,
                "automation_pdf_plugin_names": list(US_DESKTOP_PDF_PLUGIN_NAMES),
                "automation_pdf_mime_types": list(US_DESKTOP_PDF_MIME_TYPES),
                "automation_speech_synthesis_present": False,
                "automation_local_font_access_present": False,
                "automation_webgpu_adapter_available": False,
                "automation_hardware_device_apis": {
                    "bluetooth": False,
                    "usb": False,
                    "hid": False,
                    "serial": "present_without_authorized_ports",
                },
                "automation_hardware_device_counts": {
                    "bluetooth": 0,
                    "usb": 0,
                    "hid": 0,
                    "serial": 0,
                },
                "automation_hardware_device_identity_exposed": False,
                "automation_webxr_api_present": False,
                "automation_gamepad_count": 0,
                "automation_gamepad_identity_exposed": False,
                "automation_midi_permission_not_granted": True,
                "automation_midi_identity_exposed": False,
                "automation_sensor_permissions_not_granted": True,
                "automation_window_management_permission_not_granted": True,
                "automation_screen_details_identity_exposed": False,
                "automation_media_device_identity_exposed": False,
                "automation_camera_permission_not_granted": True,
                "automation_microphone_permission_not_granted": True,
                "automation_notifications_permission_not_granted": True,
                "automation_sensitive_permission_grants": {},
                "automation_permission_probe_names": list(DENIED_PERMISSION_PROBES),
                "chrome_launch_args": browser_us_chrome_args(user_agent),
                "chrome_launcher": "$AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER",
                "managed_user_data_dir": "$AGENT_MANAGER_BROWSER_US_USER_DATA_DIR",
                "default_url": NCR_URL,
            },
        },
        "limits": [
            "Remote services can still classify the public VPN IP by their own IP reputation databases.",
            "Regular browser profile Google account history, existing cookies, installed extensions, fonts, GPU, canvas, and hardware fingerprints are outside this local sync script.",
            "Native Chrome timezone changes require launching through browser-us-chrome.sh or an automation context that applies browser-us-context.json.",
        ],
    }


def sync_verification_report(
    config_dir: Path,
    browser_roots: list[BrowserRoot],
    managed_browser_root: BrowserRoot,
    explicit_profile: str | None,
    exit_profile: ExitProfile | None,
    platform_name: str,
    verification_state: str,
    google_domain_resolution: GoogleDomainResolution,
    browser_google_resolution: BrowserGoogleResolution,
    runtime_resolution: RuntimeResolution,
    automation_context_resolution: AutomationContextResolution,
    browser_state_audit: dict,
) -> list[str]:
    report_path = config_dir / "browser-us-verification.json"
    payload = verification_report_payload(
        browser_roots,
        managed_browser_root,
        explicit_profile,
        exit_profile,
        platform_name,
        verification_state,
        google_domain_resolution,
        browser_google_resolution,
        runtime_resolution,
        automation_context_resolution,
        browser_state_audit,
    )
    previous_payload = load_json_mapping(report_path) if report_path.exists() else None
    if previous_payload == payload:
        return []
    ensure_backup(report_path)
    write_json_atomic(report_path, payload)
    return [str(report_path)]


def browser_open_command(platform_name: str, config_dir: Path, url: str) -> list[str] | None:
    launcher_path = config_dir / "browser-us-chrome.sh"
    if launcher_path.exists() and os.access(launcher_path, os.X_OK):
        return [str(launcher_path), url]
    if platform_name == "darwin":
        if Path("/Applications/Google Chrome.app").exists():
            return ["open", "-g", "-a", "Google Chrome", url]
        return ["open", "-g", url]
    if platform_name.startswith("win"):
        return ["cmd", "/c", "start", "", url]
    opener = shutil.which("xdg-open")
    if opener:
        return [opener, url]
    return None


def open_google_ncr(platform_name: str, config_dir: Path) -> bool:
    command = browser_open_command(platform_name, config_dir, NCR_URL)
    if command is None:
        print("WARN: no browser opener found; skipped google.com/ncr bootstrap", file=sys.stderr)
        return False
    launcher_path = config_dir / "browser-us-chrome.sh"
    if command[0] == str(launcher_path):
        try:
            subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except OSError as exc:
            print(f"WARN: failed to launch google.com/ncr through browser-us-chrome.sh: {exc}", file=sys.stderr)
            return False
        return True
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        print(f"WARN: failed to open google.com/ncr: {detail}", file=sys.stderr)
        return False
    return True


def read_json_url(url: str, timeout: float) -> dict:
    request = urllib.request.Request(url, headers={"User-Agent": "agent-manager"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, dict):
        raise ValueError("response must contain a JSON object")
    return payload


def read_google_text(opener: urllib.request.OpenerDirector, url: str, timeout: float) -> tuple[str, str]:
    request = urllib.request.Request(
        url,
        headers={
            "Accept-Language": "en-US,en;q=0.9",
            "User-Agent": "agent-manager",
        },
    )
    with opener.open(request, timeout=timeout) as response:
        body = response.read(500_000)
        charset = response.headers.get_content_charset() or "utf-8"
        return response.geturl(), body.decode(charset, errors="ignore")


def verify_google_domain_after_ncr(timeout: float) -> GoogleDomainResolution:
    cookie_jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookie_jar))
    try:
        read_google_text(opener, NCR_URL, timeout)
        final_url, body = read_google_text(opener, GOOGLE_HOME_URL, timeout)
    except (OSError, TimeoutError, urllib.error.URLError, UnicodeError) as exc:
        return GoogleDomainResolution(4, "unverified_failed", errors=(str(exc),))

    parsed_host = urllib.parse.urlparse(final_url).netloc.lower()
    body_lower = body.lower()
    hk_markers = tuple(marker for marker in GOOGLE_HK_MARKERS if marker.lower() in body_lower)
    if parsed_host.endswith("google.com.hk") and "final_url_host:google.com.hk" not in hk_markers:
        hk_markers = (*hk_markers, "final_url_host:google.com.hk")
    if hk_markers:
        return GoogleDomainResolution(4, "hk_detected_failed", final_url=final_url, hk_markers=hk_markers)
    return GoogleDomainResolution(0, "verified_google_com_after_ncr", final_url=final_url)


def google_hk_markers_from_text(*values: str | None) -> tuple[str, ...]:
    combined_text = "\n".join(value for value in values if isinstance(value, str)).lower()
    return tuple(marker for marker in GOOGLE_HK_MARKERS if marker.lower() in combined_text)


RUNTIME_JSON_MARKER = "AGENT_MANAGER_RUNTIME_JSON:"

