"""Extracted from sync_browser_google_us.py: automation layer."""
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


def playwright_chrome_launch_options(platform_name: str, user_agent: str) -> dict:
    launch_args = [
        *browser_us_chrome_args(user_agent),
        "--disable-gpu",
        "--disable-background-networking",
    ]
    launch_options = {
        "headless": True,
        "args": launch_args,
    }
    chrome_bin = os.environ.get("AGENT_MANAGER_BROWSER_US_CHROME_BIN")
    if chrome_bin:
        launch_options["executable_path"] = chrome_bin
        return launch_options
    if platform_name == "darwin":
        default_chrome_bin = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        if default_chrome_bin.exists():
            launch_options["executable_path"] = str(default_chrome_bin)
            return launch_options
    launch_options["channel"] = "chrome"
    return launch_options


def verify_automation_browser_context(config_dir: Path, timeout: float, platform_name: str) -> AutomationContextResolution:
    if timeout <= 0:
        return AutomationContextResolution(6, "invalid_timeout", errors=("automation context timeout must be positive",))
    try:
        from playwright.sync_api import sync_playwright
    except ImportError as exc:
        return AutomationContextResolution(6, "playwright_missing_failed", errors=(str(exc),))

    try:
        context_payload = load_json_mapping(config_dir / "browser-us-context.json")
    except ValueError as exc:
        return AutomationContextResolution(6, "context_invalid_failed", errors=(str(exc),))
    browser_context = context_payload.get("browser_context")
    if not isinstance(browser_context, dict):
        return AutomationContextResolution(6, "context_invalid_failed", errors=("browser_context must be an object",))
    user_agent = browser_context.get("user_agent")
    if not isinstance(user_agent, str) or not user_agent:
        return AutomationContextResolution(6, "context_invalid_failed", errors=("browser_context.user_agent must be a string",))

    server, thread, runtime_url = start_runtime_probe_server()
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                timeout=max(1000, int(timeout * 1000)),
                **playwright_chrome_launch_options(platform_name, user_agent),
            )
            try:
                context = new_us_playwright_context(browser, browser_context)
                try:
                    page = context.new_page()
                    page.goto(runtime_url, wait_until="load", timeout=max(1000, int(timeout * 1000)))
                    observed = page.evaluate(
                        """
                        async () => {
                          const position = await new Promise((resolve) => {
                            navigator.geolocation.getCurrentPosition(
                              (pos) => resolve({
                                latitude: pos.coords.latitude,
                                longitude: pos.coords.longitude,
                                accuracy: pos.coords.accuracy,
                            }),
                              (error) => resolve({error: error.code + ':' + error.message}),
                              {maximumAge: 0, timeout: 5000}
                            );
                          });
                          const observed = {
                            userAgent: navigator.userAgent,
                            platform: navigator.platform,
                            userAgentDataMobile: navigator.userAgentData ? navigator.userAgentData.mobile : null,
                            userAgentDataPlatform: navigator.userAgentData ? navigator.userAgentData.platform : null,
                            userAgentDataBrands: navigator.userAgentData ? Array.from(navigator.userAgentData.brands || []) : null,
                            userAgentDataHighEntropy: null,
                            webdriver: navigator.webdriver,
                            language: navigator.language,
                            languages: Array.from(navigator.languages || []),
                            vendor: navigator.vendor,
                            product: navigator.product,
                            productSub: navigator.productSub,
                            appCodeName: navigator.appCodeName,
                            appName: navigator.appName,
                            appVersion: navigator.appVersion,
                            timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                            timezoneOffset: new Date().getTimezoneOffset(),
                            dateToString: new Date(Date.UTC(2026, 4, 22, 12, 0, 0)).toString(),
                            dateToTimeString: new Date(Date.UTC(2026, 4, 22, 12, 0, 0)).toTimeString(),
                            timeZoneLongFormat: null,
                            timeZoneNameLong: null,
                            intlLocale: Intl.DateTimeFormat().resolvedOptions().locale,
                            collatorLocale: Intl.Collator().resolvedOptions().locale,
                            pluralRulesLocale: new Intl.PluralRules().resolvedOptions().locale,
                            relativeTimeFormatLocale: new Intl.RelativeTimeFormat().resolvedOptions().locale,
                            localeWeekInfo: null,
                            localeTextInfo: null,
                            localeHourCycles: null,
                            localeCalendars: null,
                            localeNumberingSystems: null,
                            localeTimeZones: null,
                            dateFormat: Intl.DateTimeFormat(undefined, {dateStyle: 'short', timeZone: 'UTC'}).format(new Date(Date.UTC(2026, 4, 22))),
                            currencyFormat: Intl.NumberFormat(undefined, {style: 'currency', currency: 'USD'}).format(1234.56),
                            regionDisplayName: new Intl.DisplayNames(undefined, {type: 'region'}).of('US'),
                            innerWidth: window.innerWidth,
                            innerHeight: window.innerHeight,
                            screenWidth: window.screen.width,
                            screenHeight: window.screen.height,
                            availWidth: window.screen.availWidth,
                            availHeight: window.screen.availHeight,
                            screenColorDepth: window.screen.colorDepth,
                            screenPixelDepth: window.screen.pixelDepth,
                            screenOrientationType: window.screen.orientation ? window.screen.orientation.type : null,
                            screenOrientationAngle: window.screen.orientation ? window.screen.orientation.angle : null,
                            screenIsExtended: window.screen.isExtended,
                            visualViewportWidth: window.visualViewport ? window.visualViewport.width : null,
                            visualViewportHeight: window.visualViewport ? window.visualViewport.height : null,
                            visualViewportScale: window.visualViewport ? window.visualViewport.scale : null,
                            devicePixelRatio: window.devicePixelRatio,
                            maxTouchPoints: navigator.maxTouchPoints,
                            coarsePointer: window.matchMedia('(pointer: coarse)').matches,
                            finePointer: window.matchMedia('(pointer: fine)').matches,
                            hoverHover: window.matchMedia('(hover: hover)').matches,
                            lightColorScheme: window.matchMedia('(prefers-color-scheme: light)').matches,
                            darkColorScheme: window.matchMedia('(prefers-color-scheme: dark)').matches,
                            hardwareConcurrency: navigator.hardwareConcurrency,
                            deviceMemory: navigator.deviceMemory,
                            webglVendor: null,
                            webglRenderer: null,
                            fontChecks: {blocked: {}, metricMatches: {}},
                            pdfViewerEnabled: navigator.pdfViewerEnabled,
                            pluginsLength: navigator.plugins.length,
                            mimeTypesLength: navigator.mimeTypes.length,
                            plugins: Array.from(navigator.plugins || []).map((plugin) => ({
                              name: plugin.name,
                              filename: plugin.filename,
                              description: plugin.description,
                              length: plugin.length,
                              types: Array.from(plugin).map((mimeType) => ({
                                type: mimeType.type,
                                suffixes: mimeType.suffixes,
                                description: mimeType.description,
                              })),
                            })),
                            mimeTypes: Array.from(navigator.mimeTypes || []).map((mimeType) => ({
                              type: mimeType.type,
                              suffixes: mimeType.suffixes,
                              description: mimeType.description,
                              enabledPlugin: mimeType.enabledPlugin ? mimeType.enabledPlugin.name : null,
                            })),
                            speechSynthesisPresent: 'speechSynthesis' in window,
                            localFontAccessPresent: 'queryLocalFonts' in window,
                            keyboardLayout: null,
                            webGpuPresent: 'gpu' in navigator,
                            webGpuAdapterAvailable: null,
                            hardwareDeviceApis: {
                              bluetooth: 'bluetooth' in navigator,
                              usb: 'usb' in navigator,
                              hid: 'hid' in navigator,
                              serial: 'serial' in navigator,
                            },
                            hardwareDeviceCounts: {
                              bluetooth: 0,
                              usb: 0,
                              hid: 0,
                              serial: 0,
                            },
                            hardwareDeviceIdentityExposed: false,
                            hardwareDeviceProbeErrors: {},
                            xrPresent: 'xr' in navigator,
                            gamepadCount: 0,
                            gamepadIdentityExposed: false,
                            gamepadProbeError: null,
                            midiPermission: null,
                            midiInputCount: 0,
                            midiOutputCount: 0,
                            midiIdentityExposed: false,
                            midiAccessDenied: false,
                            midiProbeError: null,
                            accelerometerPermission: null,
                            gyroscopePermission: null,
                            magnetometerPermission: null,
                            windowManagementPermission: null,
                            permissionStates: {},
                            connectionPresent: !!(navigator.connection || navigator.mozConnection || navigator.webkitConnection),
                            connectionEffectiveType: null,
                            connectionRtt: null,
                            connectionDownlink: null,
                            connectionSaveData: null,
                            connectionType: null,
                            screenDetailsIdentityExposed: false,
                            screenDetailsAccessDenied: false,
                            screenDetailsProbeError: null,
                            mediaDeviceCount: null,
                            mediaDeviceKinds: [],
                            mediaDeviceIdentityExposed: false,
                            cameraPermission: null,
                            microphonePermission: null,
                            notificationsPermission: null,
                            notificationPermission: typeof Notification === 'function' ? Notification.permission : null,
                            geolocation: position,
                          };
                          async function permissionState(name) {
                            try {
                              if (!navigator.permissions || !navigator.permissions.query) {
                                return null;
                              }
                              const permission = navigator.permissions.query({name}).then((result) => result.state);
                              const timedOut = new Promise((resolve) => setTimeout(() => resolve('timeout'), 500));
                              return await Promise.race([permission, timedOut]);
                            } catch (error) {
                              return 'error:' + String(error && error.name || error);
                            }
                          }
                          observed.cameraPermission = await permissionState('camera');
                          observed.microphonePermission = await permissionState('microphone');
                          observed.notificationsPermission = await permissionState('notifications');
                          observed.midiPermission = await permissionState('midi');
                          observed.accelerometerPermission = await permissionState('accelerometer');
                          observed.gyroscopePermission = await permissionState('gyroscope');
                          observed.magnetometerPermission = await permissionState('magnetometer');
                          observed.windowManagementPermission = await permissionState('window-management');
                          for (const permissionName of Array.from(window.__agentManagerPermissionProbeNames || [])) {
                            observed.permissionStates[permissionName] = await permissionState(permissionName);
                          }
                          try {
                            const connection = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
                            if (connection) {
                              observed.connectionEffectiveType = connection.effectiveType || null;
                              observed.connectionRtt = connection.rtt || null;
                              observed.connectionDownlink = connection.downlink || null;
                              observed.connectionSaveData = Boolean(connection.saveData);
                              observed.connectionType = connection.type || null;
                            }
                          } catch (error) {
                            observed.connectionError = String(error && error.name || error);
                          }
                          try {
                            const fixedDate = new Date(Date.UTC(2026, 4, 22, 12, 0, 0));
                            const timeZoneFormatter = new Intl.DateTimeFormat(undefined, {
                              weekday: 'long',
                              year: 'numeric',
                              month: 'long',
                              day: 'numeric',
                              hour: 'numeric',
                              minute: 'numeric',
                              timeZoneName: 'long',
                            });
                            observed.timeZoneLongFormat = timeZoneFormatter.format(fixedDate);
                            const timeZonePart = timeZoneFormatter
                              .formatToParts(fixedDate)
                              .find((part) => part.type === 'timeZoneName');
                            observed.timeZoneNameLong = timeZonePart ? timeZonePart.value : null;
                          } catch (error) {
                            observed.timeZoneNameError = String(error && error.name || error);
                          }
                          try {
                            const canvas = document.createElement('canvas');
                            const gl = canvas.getContext('webgl') || canvas.getContext('experimental-webgl');
                            const debugInfo = gl ? gl.getExtension('WEBGL_debug_renderer_info') : null;
                            if (gl && debugInfo) {
                              observed.webglVendor = gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL);
                              observed.webglRenderer = gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL);
                            }
                          } catch (error) {
                            observed.webglError = String(error && error.name || error);
                          }
                          try {
                            const blockedFontFamilies = Array.from(window.__agentManagerBlockedFontFamilies || []);
                            const fallbackFontFamily = window.__agentManagerFontMetricFallbackFamily || 'Arial';
                            const metricSample = 'mmmmmmmmmmWWWWWWWWWWiiiiiiiiii中文测试';
                            const canvas = document.createElement('canvas');
                            const canvasContext = canvas.getContext('2d');
                            const measureWidth = (fontFamily) => {
                              if (!canvasContext) {
                                return null;
                              }
                              canvasContext.font = '32px "' + fontFamily.replace(/"/g, '') + '", sans-serif';
                              return canvasContext.measureText(metricSample).width;
                            };
                            const fallbackWidth = measureWidth(fallbackFontFamily);
                            for (const fontFamily of blockedFontFamilies) {
                              observed.fontChecks.blocked[fontFamily] = document.fonts && document.fonts.check
                                ? document.fonts.check('12px "' + fontFamily.replace(/"/g, '') + '"')
                                : null;
                              const fontWidth = measureWidth(fontFamily);
                              observed.fontChecks.metricMatches[fontFamily] = fallbackWidth !== null && fontWidth !== null
                                ? Math.abs(fontWidth - fallbackWidth) < 0.001
                                : null;
                            }
                          } catch (error) {
                            observed.fontCheckError = String(error && error.name || error);
                          }
                          try {
                            const intlLocale = new Intl.Locale(observed.intlLocale);
                            observed.localeWeekInfo = intlLocale.getWeekInfo ? intlLocale.getWeekInfo() : null;
                            observed.localeTextInfo = intlLocale.getTextInfo ? intlLocale.getTextInfo() : null;
                            observed.localeHourCycles = intlLocale.getHourCycles ? intlLocale.getHourCycles() : null;
                            observed.localeCalendars = intlLocale.getCalendars ? intlLocale.getCalendars() : null;
                            observed.localeNumberingSystems = intlLocale.getNumberingSystems ? intlLocale.getNumberingSystems() : null;
                            observed.localeTimeZones = intlLocale.getTimeZones ? intlLocale.getTimeZones() : null;
                          } catch (error) {
                            observed.localeInfoError = String(error && error.name || error);
                          }
                          try {
                            if (navigator.userAgentData && navigator.userAgentData.getHighEntropyValues) {
                              observed.userAgentDataHighEntropy = await navigator.userAgentData.getHighEntropyValues([
                                'architecture',
                                'bitness',
                                'formFactor',
                                'fullVersionList',
                                'model',
                                'platform',
                                'platformVersion',
                                'uaFullVersion',
                                'wow64',
                                'mobile',
                              ]);
                            }
                          } catch (error) {
                            observed.userAgentDataHighEntropyError = String(error && error.name || error);
                          }
                          try {
                            if (navigator.keyboard && navigator.keyboard.getLayoutMap) {
                              const layoutMap = await Promise.race([
                                navigator.keyboard.getLayoutMap(),
                                new Promise((resolve) => setTimeout(() => resolve(null), 500)),
                              ]);
                              if (layoutMap) {
                                const keyboardLayoutKeys = Array.from(window.__agentManagerKeyboardLayoutKeys || []);
                                observed.keyboardLayout = {};
                                for (const key of keyboardLayoutKeys) {
                                  observed.keyboardLayout[key] = layoutMap.get(key);
                                }
                              }
                            }
                          } catch (error) {
                            observed.keyboardLayoutError = String(error && error.name || error);
                          }
                          try {
                            if (navigator.gpu && navigator.gpu.requestAdapter) {
                              const adapter = await Promise.race([
                                navigator.gpu.requestAdapter(),
                                new Promise((resolve) => setTimeout(() => resolve(null), 500)),
                              ]);
                              observed.webGpuAdapterAvailable = Boolean(adapter);
                            } else {
                              observed.webGpuAdapterAvailable = false;
                            }
                          } catch (error) {
                            observed.webGpuProbeError = String(error && error.name || error);
                          }
                          async function probeHardwareDeviceApi(apiName, methodName) {
                            try {
                              const api = navigator[apiName];
                              if (!api || typeof api[methodName] !== 'function') {
                                observed.hardwareDeviceCounts[apiName] = 0;
                                return;
                              }
                              const devices = await Promise.race([
                                api[methodName](),
                                new Promise((resolve) => setTimeout(() => resolve('timeout'), 500)),
                              ]);
                              if (devices === 'timeout') {
                                observed.hardwareDeviceProbeErrors[apiName] = 'timeout';
                                return;
                              }
                              observed.hardwareDeviceCounts[apiName] = Array.isArray(devices) ? devices.length : 0;
                              if (observed.hardwareDeviceCounts[apiName] > 0) {
                                observed.hardwareDeviceIdentityExposed = true;
                              }
                            } catch (error) {
                              observed.hardwareDeviceProbeErrors[apiName] = String(error && error.name || error);
                            }
                          }
                          await probeHardwareDeviceApi('bluetooth', 'getDevices');
                          await probeHardwareDeviceApi('usb', 'getDevices');
                          await probeHardwareDeviceApi('hid', 'getDevices');
                          await probeHardwareDeviceApi('serial', 'getPorts');
                          try {
                            if (typeof navigator.getGamepads === 'function') {
                              const gamepads = Array.from(navigator.getGamepads() || []).filter(Boolean);
                              observed.gamepadCount = gamepads.length;
                              observed.gamepadIdentityExposed = gamepads.some((gamepad) => Boolean(gamepad.id));
                            }
                          } catch (error) {
                            observed.gamepadProbeError = String(error && error.name || error);
                          }
                          try {
                            if (navigator.requestMIDIAccess) {
                              const midiAccess = await Promise.race([
                                navigator.requestMIDIAccess({sysex: false}),
                                new Promise((resolve) => setTimeout(() => resolve('timeout'), 500)),
                              ]);
                              if (midiAccess === 'timeout') {
                                observed.midiProbeError = 'timeout';
                              } else {
                                const inputs = midiAccess.inputs ? Array.from(midiAccess.inputs.values()) : [];
                                const outputs = midiAccess.outputs ? Array.from(midiAccess.outputs.values()) : [];
                                observed.midiInputCount = inputs.length;
                                observed.midiOutputCount = outputs.length;
                                observed.midiIdentityExposed = inputs.length > 0 || outputs.length > 0;
                              }
                            }
                          } catch (error) {
                            if (error && error.name === 'NotAllowedError') {
                              observed.midiAccessDenied = true;
                            } else {
                              observed.midiProbeError = String(error && error.name || error);
                            }
                          }
                          try {
                            if (typeof window.getScreenDetails === 'function') {
                              const screenDetails = await Promise.race([
                                window.getScreenDetails(),
                                new Promise((resolve) => setTimeout(() => resolve('timeout'), 500)),
                              ]);
                              if (screenDetails === 'timeout') {
                                observed.screenDetailsProbeError = 'timeout';
                              } else {
                                const screens = screenDetails.screens ? Array.from(screenDetails.screens) : [];
                                observed.screenDetailsIdentityExposed = screens.some((screen) => Boolean(screen.label));
                              }
                            }
                          } catch (error) {
                            if (error && error.name === 'NotAllowedError') {
                              observed.screenDetailsAccessDenied = true;
                            } else {
                              observed.screenDetailsProbeError = String(error && error.name || error);
                            }
                          }
                          try {
                            if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
                              const devices = await Promise.race([
                                navigator.mediaDevices.enumerateDevices(),
                                new Promise((resolve) => setTimeout(() => resolve([]), 500)),
                              ]);
                              observed.mediaDeviceCount = devices.length;
                              observed.mediaDeviceKinds = devices.map((device) => device.kind);
                              observed.mediaDeviceIdentityExposed = devices.some(
                                (device) => Boolean(device.label || device.deviceId || device.groupId)
                              );
                            } else {
                              observed.mediaDeviceCount = 0;
                              observed.mediaDeviceKinds = [];
                              observed.mediaDeviceIdentityExposed = false;
                            }
                          } catch (error) {
                            observed.mediaDeviceProbeError = String(error && error.name || error);
                          }
                          return observed;
                        }
                        """
                    )
                finally:
                    context.close()
            finally:
                browser.close()
    except Exception as exc:
        return AutomationContextResolution(6, "automation_context_failed", process_state="failed", errors=(str(exc),))
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)

    if not isinstance(observed, dict):
        return AutomationContextResolution(6, "automation_context_output_missing", process_state="output_missing")
    observed["acceptLanguageHeader"] = server.observed_headers.get("accept-language")
    observed["userAgentHeader"] = server.observed_headers.get("user-agent")
    observed["secChUa"] = server.observed_headers.get("sec-ch-ua")
    observed["secChUaPlatform"] = server.observed_headers.get("sec-ch-ua-platform")
    observed["secChUaMobile"] = server.observed_headers.get("sec-ch-ua-mobile")
    mismatches = validate_automation_context_observation(observed, browser_context)
    if mismatches:
        return AutomationContextResolution(
            6,
            "automation_context_mismatch_failed",
            observed=observed,
            process_state="verified_with_mismatch",
            errors=mismatches,
        )
    return AutomationContextResolution(0, "verified_automation_context_us", observed=observed, process_state="verified")


def load_automation_browser_context(config_dir: Path) -> dict | AutomationContextResolution:
    try:
        context_payload = load_json_mapping(config_dir / "browser-us-context.json")
    except ValueError as exc:
        return AutomationContextResolution(6, "context_invalid_failed", errors=(str(exc),))
    browser_context = context_payload.get("browser_context")
    if not isinstance(browser_context, dict):
        return AutomationContextResolution(6, "context_invalid_failed", errors=("browser_context must be an object",))
    return browser_context


def new_us_playwright_context(browser, browser_context: dict):
    context = browser.new_context(
        locale=browser_context.get("locale"),
        timezone_id=browser_context.get("timezone_id"),
        geolocation=browser_context.get("geolocation"),
        permissions=browser_context.get("permissions") or [],
        extra_http_headers=browser_context.get("extra_http_headers") or {},
        user_agent=browser_context.get("user_agent"),
        viewport=browser_context.get("viewport"),
        screen=browser_context.get("screen"),
        device_scale_factor=browser_context.get("device_scale_factor"),
        is_mobile=browser_context.get("is_mobile"),
        has_touch=browser_context.get("has_touch"),
        color_scheme=browser_context.get("color_scheme"),
    )
    navigator_languages = browser_context.get("navigator_languages")
    hardware_concurrency = browser_context.get("hardware_concurrency")
    device_memory_gb = browser_context.get("device_memory_gb")
    network_information = browser_context.get("network_information")
    webgl_vendor = browser_context.get("webgl_vendor")
    webgl_renderer = browser_context.get("webgl_renderer")
    blocked_font_families = browser_context.get("blocked_font_families")
    font_metric_fallback_family = browser_context.get("font_metric_fallback_family")
    keyboard_layout_map = browser_context.get("keyboard_layout_map")
    permission_probe_names_json = json.dumps(list(DENIED_PERMISSION_PROBES))
    context.add_init_script(
        f"""
        (() => {{
          Object.defineProperty(window, '__agentManagerPermissionProbeNames', {{
            configurable: true,
            value: Object.freeze(Array.from({permission_probe_names_json})),
          }});
        }})();
        """
    )
    if isinstance(hardware_concurrency, int) and not isinstance(hardware_concurrency, bool) and hardware_concurrency > 0:
        context.add_init_script(
            f"""
            (() => {{
              Object.defineProperty(Navigator.prototype, 'hardwareConcurrency', {{
                configurable: true,
                get: () => {hardware_concurrency},
              }});
            }})();
            """
        )
    if isinstance(device_memory_gb, int) and not isinstance(device_memory_gb, bool) and device_memory_gb > 0:
        context.add_init_script(
            f"""
            (() => {{
              Object.defineProperty(Navigator.prototype, 'deviceMemory', {{
                configurable: true,
                get: () => {device_memory_gb},
              }});
            }})();
            """
        )
    if isinstance(network_information, dict):
        effective_type = network_information.get("effective_type")
        rtt_ms = network_information.get("rtt_ms")
        downlink_mbps = network_information.get("downlink_mbps")
        save_data = network_information.get("save_data")
        connection_type = network_information.get("type")
        if (
            isinstance(effective_type, str)
            and isinstance(rtt_ms, int)
            and not isinstance(rtt_ms, bool)
            and isinstance(downlink_mbps, (int, float))
            and not isinstance(downlink_mbps, bool)
            and isinstance(save_data, bool)
            and (connection_type is None or isinstance(connection_type, str))
        ):
            network_information_json = json.dumps(
                {
                    "effectiveType": effective_type,
                    "rtt": rtt_ms,
                    "downlink": downlink_mbps,
                    "saveData": save_data,
                    "type": connection_type,
                },
                sort_keys=True,
            )
            context.add_init_script(
                f"""
                (() => {{
                  const networkInformation = Object.freeze(Object.assign({{}}, {network_information_json}));
                  const patchConnection = (connection) => {{
                    if (!connection) {{
                      return;
                    }}
                    const connectionPrototype = Object.getPrototypeOf(connection);
                    const patchTarget = connectionPrototype || connection;
                    for (const [key, value] of Object.entries(networkInformation)) {{
                      Object.defineProperty(patchTarget, key, {{
                        configurable: true,
                        get: () => value,
                      }});
                    }}
                  }};
                  patchConnection(navigator.connection || navigator.mozConnection || navigator.webkitConnection);
                }})();
                """
            )
    if isinstance(webgl_vendor, str) and webgl_vendor and isinstance(webgl_renderer, str) and webgl_renderer:
        webgl_vendor_json = json.dumps(webgl_vendor)
        webgl_renderer_json = json.dumps(webgl_renderer)
        context.add_init_script(
            f"""
            (() => {{
              const webglVendor = {webgl_vendor_json};
              const webglRenderer = {webgl_renderer_json};
              const patchGetParameter = (contextConstructor) => {{
                if (!contextConstructor || !contextConstructor.prototype || !contextConstructor.prototype.getParameter) {{
                  return;
                }}
                const originalGetParameter = contextConstructor.prototype.getParameter;
                Object.defineProperty(contextConstructor.prototype, 'getParameter', {{
                  configurable: true,
                  value(parameter) {{
                    if (parameter === 37445) {{
                      return webglVendor;
                    }}
                    if (parameter === 37446) {{
                      return webglRenderer;
                    }}
                    return originalGetParameter.apply(this, arguments);
                  }},
                }});
              }};
              patchGetParameter(window.WebGLRenderingContext);
              patchGetParameter(window.WebGL2RenderingContext);
            }})();
            """
        )
    if isinstance(blocked_font_families, list) and blocked_font_families:
        blocked_font_names = [font for font in blocked_font_families if isinstance(font, str) and font]
        if blocked_font_names and isinstance(font_metric_fallback_family, str) and font_metric_fallback_family:
            blocked_font_names_json = json.dumps(blocked_font_names)
            font_metric_fallback_family_json = json.dumps(font_metric_fallback_family)
            context.add_init_script(
                f"""
                (() => {{
                  const blockedFontFamilies = Object.freeze(Array.from({blocked_font_names_json}));
                  const fontMetricFallbackFamily = {font_metric_fallback_family_json};
                  const blockedFontNeedles = blockedFontFamilies.map((font) => font.toLowerCase());
                  Object.defineProperty(window, '__agentManagerBlockedFontFamilies', {{
                    configurable: true,
                    value: blockedFontFamilies,
                  }});
                  Object.defineProperty(window, '__agentManagerFontMetricFallbackFamily', {{
                    configurable: true,
                    value: fontMetricFallbackFamily,
                  }});
                  const shouldBlockFont = (font) => {{
                    const fontText = String(font || '').toLowerCase();
                    return blockedFontNeedles.some((blockedFont) => fontText.includes(blockedFont));
                  }};
                  const escapeRegExp = (value) => String(value).replace(/[.*+?^${{}}()|[\\]\\\\]/g, '\\\\$&');
                  const fallbackFontFor = (font) => {{
                    let fallbackFont = String(font || '');
                    for (const blockedFontFamily of blockedFontFamilies) {{
                      const escapedFamily = escapeRegExp(blockedFontFamily);
                      fallbackFont = fallbackFont.replace(
                        new RegExp('"' + escapedFamily + '"', 'ig'),
                        '"' + fontMetricFallbackFamily + '"'
                      );
                      fallbackFont = fallbackFont.replace(
                        new RegExp("'" + escapedFamily + "'", 'ig'),
                        '"' + fontMetricFallbackFamily + '"'
                      );
                      fallbackFont = fallbackFont.replace(new RegExp(escapedFamily, 'ig'), fontMetricFallbackFamily);
                    }}
                    return fallbackFont;
                  }};
                  const patchFontCheck = (target) => {{
                    if (!target || !target.check) {{
                      return;
                    }}
                    const originalCheck = target.check;
                    Object.defineProperty(target, 'check', {{
                      configurable: true,
                      value(font, text) {{
                        if (shouldBlockFont(font)) {{
                          return false;
                        }}
                        return originalCheck.apply(this, arguments);
                      }},
                    }});
                  }};
                  const patchMeasureText = (target) => {{
                    if (!target || !target.measureText) {{
                      return;
                    }}
                    const originalMeasureText = target.measureText;
                    Object.defineProperty(target, 'measureText', {{
                      configurable: true,
                      value(text) {{
                        if (!shouldBlockFont(this && this.font)) {{
                          return originalMeasureText.apply(this, arguments);
                        }}
                        const originalFont = this.font;
                        try {{
                          this.font = fallbackFontFor(originalFont);
                          return originalMeasureText.apply(this, arguments);
                        }} finally {{
                          try {{
                            this.font = originalFont;
                          }} catch (error) {{
                          }}
                        }}
                      }},
                    }});
                  }};
                  if (window.FontFaceSet && FontFaceSet.prototype) {{
                    patchFontCheck(FontFaceSet.prototype);
                  }}
                  if (document.fonts) {{
                    patchFontCheck(document.fonts);
                  }}
                  patchMeasureText(window.CanvasRenderingContext2D && CanvasRenderingContext2D.prototype);
                  patchMeasureText(window.OffscreenCanvasRenderingContext2D && OffscreenCanvasRenderingContext2D.prototype);
                }})();
                """
            )
    if isinstance(keyboard_layout_map, dict) and keyboard_layout_map:
        keyboard_layout_json = json.dumps(keyboard_layout_map, sort_keys=True)
        context.add_init_script(
            f"""
            (() => {{
              const keyboardLayout = Object.freeze(Object.assign({{}}, {keyboard_layout_json}));
              const keyboardLayoutKeys = Object.freeze(Object.keys(keyboardLayout));
              const createKeyboardLayoutMap = () => {{
                const keyboardMap = new Map(keyboardLayoutKeys.map((key) => [key, keyboardLayout[key]]));
                Object.defineProperty(keyboardMap, Symbol.toStringTag, {{
                  configurable: true,
                  value: 'KeyboardLayoutMap',
                }});
                return keyboardMap;
              }};
              Object.defineProperty(window, '__agentManagerKeyboardLayoutKeys', {{
                configurable: true,
                value: keyboardLayoutKeys,
              }});
              const keyboard = navigator.keyboard;
              if (keyboard && keyboard.getLayoutMap) {{
                const keyboardPrototype = Object.getPrototypeOf(keyboard);
                Object.defineProperty(keyboardPrototype, 'getLayoutMap', {{
                  configurable: true,
                  value() {{
                    return Promise.resolve(createKeyboardLayoutMap());
                  }},
                }});
              }} else {{
                Object.defineProperty(Navigator.prototype, 'keyboard', {{
                  configurable: true,
                  get() {{
                    return {{
                      getLayoutMap() {{
                        return Promise.resolve(createKeyboardLayoutMap());
                      }},
                    }};
                  }},
                }});
              }}
            }})();
            """
        )
    if isinstance(navigator_languages, list) and navigator_languages:
        languages_json = json.dumps(navigator_languages)
        context.add_init_script(
            f"""
            (() => {{
              const frozenLanguages = Object.freeze(Array.from({languages_json}));
              Object.defineProperty(Navigator.prototype, 'languages', {{
                configurable: true,
                get: () => frozenLanguages,
              }});
            }})();
            """
        )
    return context


def verify_google_domain_with_browser_context(
    config_dir: Path,
    timeout: float,
    platform_name: str,
) -> BrowserGoogleResolution:
    if timeout <= 0:
        return BrowserGoogleResolution(7, "invalid_timeout", errors=("browser Google timeout must be positive",))
    try:
        from playwright.sync_api import sync_playwright
    except ImportError as exc:
        return BrowserGoogleResolution(7, "playwright_missing_failed", errors=(str(exc),))

    browser_context = load_automation_browser_context(config_dir)
    if isinstance(browser_context, AutomationContextResolution):
        return BrowserGoogleResolution(
            7,
            browser_context.verification_state,
            errors=browser_context.errors,
            process_state=browser_context.process_state,
        )

    browser = None
    context = None
    user_agent = browser_context.get("user_agent")
    if not isinstance(user_agent, str) or not user_agent:
        return BrowserGoogleResolution(7, "context_invalid_failed", errors=("browser_context.user_agent must be a string",))
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                timeout=max(1000, int(timeout * 1000)),
                **playwright_chrome_launch_options(platform_name, user_agent),
            )
            try:
                context = new_us_playwright_context(browser, browser_context)
                page = context.new_page()
                page.goto(NCR_URL, wait_until="domcontentloaded", timeout=max(1000, int(timeout * 1000)))
                page.goto(GOOGLE_HOME_URL, wait_until="domcontentloaded", timeout=max(1000, int(timeout * 1000)))
                final_url = page.url
                title = page.title()
                try:
                    body_text = page.locator("body").inner_text(timeout=max(1000, int(min(timeout, 5) * 1000)))
                except Exception:
                    body_text = ""
                html = page.content()
            finally:
                if context is not None:
                    context.close()
                if browser is not None:
                    browser.close()
    except Exception as exc:
        return BrowserGoogleResolution(7, "browser_google_failed", process_state="failed", errors=(str(exc),))

    parsed_host = urllib.parse.urlparse(final_url).netloc.lower()
    hk_markers = google_hk_markers_from_text(final_url, title, body_text, html)
    if parsed_host.endswith("google.com.hk") and "final_url_host:google.com.hk" not in hk_markers:
        hk_markers = (*hk_markers, "final_url_host:google.com.hk")
    if hk_markers:
        return BrowserGoogleResolution(
            7,
            "browser_google_hk_detected_failed",
            final_url=final_url,
            title=title,
            hk_markers=hk_markers,
            process_state="verified_with_hk_markers",
        )
    return BrowserGoogleResolution(
        0,
        "verified_browser_google_com_after_ncr",
        final_url=final_url,
        title=title,
        process_state="verified",
    )


EXIT_PROFILE_PROBES = [
    "https://ipinfo.io/json",
    "https://ipapi.co/json",
    "https://ipwho.is/",
]

