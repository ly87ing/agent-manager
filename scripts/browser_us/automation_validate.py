"""Extracted from sync_browser_google_us.py: automation observation validation."""
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
from browser_us.verify_report import *  # noqa: F401,F403
from browser_us.runtime_probe import *  # noqa: F401,F403


def validate_automation_context_observation(observed: dict, browser_context: dict) -> tuple[str, ...]:
    mismatches = []
    expected_geolocation = browser_context.get("geolocation") if isinstance(browser_context, dict) else None
    observed_geolocation = observed.get("geolocation") if isinstance(observed.get("geolocation"), dict) else None
    expected_viewport = browser_context.get("viewport") if isinstance(browser_context.get("viewport"), dict) else None
    expected_screen = browser_context.get("screen") if isinstance(browser_context.get("screen"), dict) else None
    expected_keyboard_layout = (
        browser_context.get("keyboard_layout_map") if isinstance(browser_context.get("keyboard_layout_map"), dict) else None
    )
    expected_network_information = (
        browser_context.get("network_information") if isinstance(browser_context.get("network_information"), dict) else None
    )
    expected_user_agent = browser_context.get("user_agent")
    expected_client_hints_platform = browser_context.get("client_hints_platform")
    expected_navigator_platform = browser_context.get("navigator_platform")
    if observed.get("userAgent") != expected_user_agent:
        mismatches.append("automation navigator.userAgent")
    if observed.get("userAgentHeader") != expected_user_agent:
        mismatches.append("automation User-Agent header")
    if observed.get("platform") != expected_navigator_platform:
        mismatches.append("automation navigator.platform")
    if observed.get("vendor") != "Google Inc.":
        mismatches.append("automation navigator.vendor")
    if observed.get("product") != "Gecko":
        mismatches.append("automation navigator.product")
    if observed.get("productSub") != "20030107":
        mismatches.append("automation navigator.productSub")
    if observed.get("appCodeName") != "Mozilla":
        mismatches.append("automation navigator.appCodeName")
    if observed.get("appName") != "Netscape":
        mismatches.append("automation navigator.appName")
    if observed.get("appVersion") != navigator_app_version_from_user_agent(str(expected_user_agent or "")):
        mismatches.append("automation navigator.appVersion")
    user_agent_data_platform = observed.get("userAgentDataPlatform")
    if user_agent_data_platform is not None and user_agent_data_platform != expected_client_hints_platform:
        mismatches.append("automation user agent platform")
    if not user_agent_data_brands_match_chrome_major(observed.get("userAgentDataBrands"), expected_user_agent):
        mismatches.append("automation user agent brands")
    if observed.get("userAgentDataHighEntropyError") or not user_agent_high_entropy_matches_desktop_surface(
        observed.get("userAgentDataHighEntropy"),
        expected_user_agent,
        expected_client_hints_platform,
    ):
        mismatches.append("automation user agent high entropy")
    if not sec_ch_ua_matches_chrome_major(observed.get("secChUa"), expected_user_agent):
        mismatches.append("automation Sec-CH-UA")
    sec_ch_ua_platform = observed.get("secChUaPlatform")
    if sec_ch_ua_platform is not None and sec_ch_ua_platform.strip('"') != expected_client_hints_platform:
        mismatches.append("automation Sec-CH-UA-Platform")
    sec_ch_ua_mobile = observed.get("secChUaMobile")
    if sec_ch_ua_mobile is not None and sec_ch_ua_mobile != "?0":
        mismatches.append("automation Sec-CH-UA-Mobile")
    if observed.get("language") != APP_LOCALE:
        mismatches.append("automation navigator.language")
    languages = observed.get("languages")
    if languages != browser_context.get("navigator_languages"):
        mismatches.append("automation navigator.languages")
    if observed.get("timezone") != browser_context.get("timezone_id"):
        mismatches.append("automation timezone")
    expected_offset = expected_timezone_offset_minutes(browser_context.get("timezone_id"))
    if expected_offset is None:
        mismatches.append("automation timezone offset rule")
    elif observed.get("timezoneOffset") != expected_offset:
        mismatches.append("automation timezone offset")
    if observed.get("timeZoneNameError") or not timezone_display_matches(observed, str(browser_context.get("timezone_id") or "")):
        mismatches.append("automation Date timezone display")
    if observed.get("intlLocale") != APP_LOCALE:
        mismatches.append("automation Intl locale")
    if observed.get("collatorLocale") != APP_LOCALE:
        mismatches.append("automation Intl collator locale")
    if observed.get("pluralRulesLocale") != APP_LOCALE:
        mismatches.append("automation Intl plural rules locale")
    if observed.get("relativeTimeFormatLocale") != APP_LOCALE:
        mismatches.append("automation Intl relative time format locale")
    if observed.get("localeInfoError"):
        mismatches.append("automation Intl locale info")
    if observed.get("localeWeekInfo") != {"firstDay": 7, "weekend": [6, 7]}:
        mismatches.append("automation Intl locale week info")
    if observed.get("localeTextInfo") != {"direction": "ltr"}:
        mismatches.append("automation Intl locale text info")
    locale_hour_cycles = observed.get("localeHourCycles")
    if not isinstance(locale_hour_cycles, list) or "h12" not in locale_hour_cycles:
        mismatches.append("automation Intl locale hour cycles")
    locale_calendars = observed.get("localeCalendars")
    if not isinstance(locale_calendars, list) or "gregory" not in locale_calendars:
        mismatches.append("automation Intl locale calendars")
    locale_numbering_systems = observed.get("localeNumberingSystems")
    if not isinstance(locale_numbering_systems, list) or "latn" not in locale_numbering_systems:
        mismatches.append("automation Intl locale numbering systems")
    locale_time_zones = observed.get("localeTimeZones")
    if not isinstance(locale_time_zones, list) or browser_context.get("timezone_id") not in locale_time_zones:
        mismatches.append("automation Intl locale time zones")
    if observed.get("dateFormat") != "5/22/26":
        mismatches.append("automation Intl date format")
    if observed.get("currencyFormat") != "$1,234.56":
        mismatches.append("automation Intl currency format")
    if observed.get("regionDisplayName") != "United States":
        mismatches.append("automation Intl region display name")
    if not accept_language_is_us(observed.get("acceptLanguageHeader")):
        mismatches.append("automation Accept-Language")
    if not isinstance(expected_viewport, dict):
        mismatches.append("automation viewport config")
    else:
        if observed.get("innerWidth") != expected_viewport.get("width"):
            mismatches.append("automation viewport width")
        if observed.get("innerHeight") != expected_viewport.get("height"):
            mismatches.append("automation viewport height")
    if not isinstance(expected_screen, dict):
        mismatches.append("automation screen config")
    else:
        if observed.get("screenWidth") != expected_screen.get("width"):
            mismatches.append("automation screen width")
        if observed.get("screenHeight") != expected_screen.get("height"):
            mismatches.append("automation screen height")
        if observed.get("availWidth") != expected_screen.get("width"):
            mismatches.append("automation screen available width")
        if observed.get("availHeight") != expected_screen.get("height"):
            mismatches.append("automation screen available height")
    if observed.get("screenColorDepth") != US_DESKTOP_SCREEN_COLOR_DEPTH:
        mismatches.append("automation screen color depth")
    if observed.get("screenPixelDepth") != US_DESKTOP_SCREEN_COLOR_DEPTH:
        mismatches.append("automation screen pixel depth")
    if observed.get("screenOrientationType") != US_DESKTOP_SCREEN_ORIENTATION_TYPE:
        mismatches.append("automation screen orientation type")
    if observed.get("screenOrientationAngle") != US_DESKTOP_SCREEN_ORIENTATION_ANGLE:
        mismatches.append("automation screen orientation angle")
    if observed.get("screenIsExtended") is not False:
        mismatches.append("automation extended screen")
    if isinstance(expected_viewport, dict):
        if observed.get("visualViewportWidth") != expected_viewport.get("width"):
            mismatches.append("automation visual viewport width")
        if observed.get("visualViewportHeight") != expected_viewport.get("height"):
            mismatches.append("automation visual viewport height")
    if observed.get("visualViewportScale") != 1:
        mismatches.append("automation visual viewport scale")
    if observed.get("devicePixelRatio") != browser_context.get("device_scale_factor"):
        mismatches.append("automation device scale factor")
    if observed.get("maxTouchPoints") != 0:
        mismatches.append("automation touch points")
    if observed.get("coarsePointer") is not False:
        mismatches.append("automation coarse pointer")
    if observed.get("finePointer") is not True:
        mismatches.append("automation fine pointer")
    if observed.get("hoverHover") is not True:
        mismatches.append("automation hover")
    if observed.get("userAgentDataMobile") not in (False, None):
        mismatches.append("automation user agent mobile")
    if observed.get("webdriver") not in (False, None):
        mismatches.append("automation webdriver")
    if observed.get("lightColorScheme") is not True or observed.get("darkColorScheme") is not False:
        mismatches.append("automation color scheme")
    if observed.get("hardwareConcurrency") != browser_context.get("hardware_concurrency"):
        mismatches.append("automation hardware concurrency")
    if observed.get("deviceMemory") != browser_context.get("device_memory_gb"):
        mismatches.append("automation device memory")
    if isinstance(expected_network_information, dict):
        if observed.get("connectionError"):
            mismatches.append("automation network information probe")
        if observed.get("connectionPresent") is not True:
            mismatches.append("automation network information")
        if observed.get("connectionEffectiveType") != expected_network_information.get("effective_type"):
            mismatches.append("automation network effective type")
        if observed.get("connectionRtt") != expected_network_information.get("rtt_ms"):
            mismatches.append("automation network rtt")
        if observed.get("connectionDownlink") != expected_network_information.get("downlink_mbps"):
            mismatches.append("automation network downlink")
        if observed.get("connectionSaveData") is not expected_network_information.get("save_data"):
            mismatches.append("automation network save data")
        if observed.get("connectionType") != expected_network_information.get("type"):
            mismatches.append("automation network type")
    if not permission_states_allow_no_grants(observed):
        mismatches.append("automation sensitive permission grants")
    if observed.get("webglVendor") != browser_context.get("webgl_vendor"):
        mismatches.append("automation WebGL vendor")
    if observed.get("webglRenderer") != browser_context.get("webgl_renderer"):
        mismatches.append("automation WebGL renderer")
    expected_blocked_fonts = browser_context.get("blocked_font_families")
    observed_blocked_fonts = (observed.get("fontChecks") or {}).get("blocked")
    observed_metric_matches = (observed.get("fontChecks") or {}).get("metricMatches")
    if isinstance(expected_blocked_fonts, list) and expected_blocked_fonts:
        if not isinstance(observed_blocked_fonts, dict):
            mismatches.append("automation blocked font checks")
        else:
            for font_family in expected_blocked_fonts:
                if observed_blocked_fonts.get(font_family) is not False:
                    mismatches.append(f"automation blocked font: {font_family}")
        if not isinstance(observed_metric_matches, dict):
            mismatches.append("automation blocked font metric checks")
        else:
            for font_family in expected_blocked_fonts:
                if observed_metric_matches.get(font_family) is not True:
                    mismatches.append(f"automation blocked font metric: {font_family}")
    if observed.get("pdfViewerEnabled") is not True:
        mismatches.append("automation pdf viewer")
    if not positive_integer(observed.get("pluginsLength")):
        mismatches.append("automation plugins")
    if not positive_integer(observed.get("mimeTypesLength")):
        mismatches.append("automation mime types")
    if not pdf_plugin_surface_matches(observed):
        mismatches.append("automation PDF plugin surface")
    if observed.get("speechSynthesisPresent") is not False:
        mismatches.append("automation speech synthesis")
    if observed.get("localFontAccessPresent") is not False:
        mismatches.append("automation local font access")
    if observed.get("keyboardLayoutError"):
        mismatches.append("automation keyboard layout probe")
    if isinstance(expected_keyboard_layout, dict) and observed.get("keyboardLayout") != expected_keyboard_layout:
        mismatches.append("automation keyboard layout")
    if observed.get("webGpuProbeError"):
        mismatches.append("automation WebGPU probe")
    if observed.get("webGpuAdapterAvailable") is not False:
        mismatches.append("automation WebGPU adapter")
    hardware_device_apis = observed.get("hardwareDeviceApis")
    hardware_device_counts = observed.get("hardwareDeviceCounts")
    hardware_device_errors = observed.get("hardwareDeviceProbeErrors")
    if not isinstance(hardware_device_apis, dict):
        mismatches.append("automation hardware device API checks")
    else:
        for api_name in ("bluetooth", "usb", "hid"):
            if hardware_device_apis.get(api_name) is not False:
                mismatches.append(f"automation {api_name} API")
    if not isinstance(hardware_device_counts, dict):
        mismatches.append("automation hardware device count checks")
    else:
        for api_name in ("bluetooth", "usb", "hid", "serial"):
            if hardware_device_counts.get(api_name) != 0:
                mismatches.append(f"automation {api_name} device count")
    if isinstance(hardware_device_errors, dict) and hardware_device_errors:
        mismatches.append("automation hardware device probe")
    elif not isinstance(hardware_device_errors, dict):
        mismatches.append("automation hardware device probe")
    if observed.get("hardwareDeviceIdentityExposed") is not False:
        mismatches.append("automation hardware device identity")
    if observed.get("xrPresent") is not False:
        mismatches.append("automation WebXR API")
    if observed.get("gamepadProbeError"):
        mismatches.append("automation gamepad probe")
    if observed.get("gamepadCount") != 0:
        mismatches.append("automation gamepad count")
    if observed.get("gamepadIdentityExposed") is not False:
        mismatches.append("automation gamepad identity")
    if observed.get("midiProbeError"):
        mismatches.append("automation MIDI probe")
    if observed.get("midiPermission") == "granted":
        mismatches.append("automation MIDI permission")
    if observed.get("midiInputCount") != 0:
        mismatches.append("automation MIDI input count")
    if observed.get("midiOutputCount") != 0:
        mismatches.append("automation MIDI output count")
    if observed.get("midiIdentityExposed") is not False:
        mismatches.append("automation MIDI identity")
    if observed.get("windowManagementPermission") == "granted":
        mismatches.append("automation window management permission")
    if observed.get("screenDetailsProbeError"):
        mismatches.append("automation screen details probe")
    if observed.get("screenDetailsIdentityExposed") is not False:
        mismatches.append("automation screen details identity")
    for permission_name in ("accelerometerPermission", "gyroscopePermission", "magnetometerPermission"):
        if observed.get(permission_name) == "granted":
            mismatches.append(f"automation {permission_name}")
    if observed.get("mediaDeviceProbeError"):
        mismatches.append("automation media device probe")
    if observed.get("mediaDeviceIdentityExposed") is not False:
        mismatches.append("automation media device identity")
    for permission_name in ("cameraPermission", "microphonePermission", "notificationsPermission"):
        if observed.get(permission_name) == "granted":
            mismatches.append(f"automation {permission_name}")
    if not isinstance(expected_geolocation, dict) or not isinstance(observed_geolocation, dict):
        mismatches.append("automation geolocation")
        return tuple(mismatches)
    if not coordinates_match(observed_geolocation.get("latitude"), expected_geolocation.get("latitude")):
        mismatches.append("automation geolocation latitude")
    if not coordinates_match(observed_geolocation.get("longitude"), expected_geolocation.get("longitude")):
        mismatches.append("automation geolocation longitude")
    if not coordinates_match(observed_geolocation.get("accuracy"), expected_geolocation.get("accuracy"), tolerance=0.001):
        mismatches.append("automation geolocation accuracy")
    return tuple(mismatches)

