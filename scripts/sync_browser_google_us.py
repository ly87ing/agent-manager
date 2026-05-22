#!/usr/bin/env python3
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


ACCEPT_LANGUAGES = "en-US,en"
APP_LOCALE = "en-US"
COUNTRY_ID_US = 244
ENV_LOCALE = "en_US.UTF-8"
NCR_URL = "https://www.google.com/ncr"
GOOGLE_HOME_URL = "https://www.google.com/?hl=en-US&gl=US&pws=0"
SEARCH_URL = "https://www.google.com/search?q={searchTerms}&hl=en-US&gl=US&pws=0"
SUGGEST_URL = "https://www.google.com/complete/search?client=chrome&q={searchTerms}&hl=en-US&gl=US"
TRANSLATE_TARGET_LANGUAGE = "en"
VARIATIONS_COUNTRY = "us"
DEFAULT_TIMEZONE_ID = "America/Los_Angeles"
DEFAULT_GEOLOCATION = {
    "latitude": 37.7749,
    "longitude": -122.4194,
    "accuracy": 100,
}
US_DESKTOP_VIEWPORT = {"width": 1365, "height": 768}
US_DESKTOP_SCREEN = {"width": 1365, "height": 768}
US_DESKTOP_DEVICE_SCALE_FACTOR = 1
US_DESKTOP_IS_MOBILE = False
US_DESKTOP_HAS_TOUCH = False
US_DESKTOP_COLOR_SCHEME = "light"
US_DESKTOP_HARDWARE_CONCURRENCY = 8
US_DESKTOP_DEVICE_MEMORY_GB = 8
US_DESKTOP_WEBGL_VENDOR = "Intel Inc."
US_DESKTOP_WEBGL_RENDERER = "Intel Iris OpenGL Engine"
EXIT_PROFILE_GEOLOCATION_ACCURACY = 50000
VERIFICATION_REPORT_VERSION = 1
FALLBACK_CHROME_VERSION = "120.0.0.0"
DISABLED_CHROME_FEATURES = ("ReduceAcceptLanguage",)
BASE_CHROME_ARGS = [
    "--lang=en-US",
    "--accept-lang=en-US,en",
    f"--window-size={US_DESKTOP_VIEWPORT['width']},{US_DESKTOP_VIEWPORT['height']}",
    f"--force-device-scale-factor={US_DESKTOP_DEVICE_SCALE_FACTOR}",
    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
    "--disable-quic",
    "--dns-prefetch-disable",
    "--no-first-run",
    "--disable-search-engine-choice-screen",
    "--disable-sync",
    "--disable-extensions",
    "--deny-permission-prompts",
    f"--disable-features={','.join(DISABLED_CHROME_FEATURES)}",
    "--disable-blink-features=AutomationControlled",
]
BLOCK_CONTENT_SETTING = 2
DEVICE_PERMISSION_CONTENT_SETTINGS = ("media_stream_camera", "media_stream_mic", "notifications")
GOOGLE_HK_MARKERS = ("google.com.hk", "hong kong")
SYSTEM_PROFILE_NAMES = {"System Profile", "Guest Profile"}
MANAGED_PROFILE_VOLATILE_STATE_RELATIVE_PATHS = (
    "Default/Account Web Data",
    "Default/Account Web Data-journal",
    "Default/Affiliation Database",
    "Default/Affiliation Database-journal",
    "Default/ClientCertificates",
    "Default/Cookies",
    "Default/Cookies-journal",
    "Default/DawnGraphiteCache",
    "Default/DawnWebGPUCache",
    "Default/Favicons",
    "Default/Favicons-journal",
    "Default/History",
    "Default/History-journal",
    "Default/IndexedDB",
    "Default/Login Data",
    "Default/Login Data-journal",
    "Default/Login Data For Account",
    "Default/Login Data For Account-journal",
    "Default/Local Storage",
    "Default/Network Persistent State",
    "Default/PersistentOriginTrials",
    "Default/Reporting and NEL",
    "Default/Reporting and NEL-journal",
    "Default/Safe Browsing Cookies",
    "Default/Safe Browsing Cookies-journal",
    "Default/ServerCertificate",
    "Default/ServerCertificate-journal",
    "Default/Sessions",
    "Default/Session Storage",
    "Default/Service Worker",
    "Default/Shared Dictionary",
    "Default/Site Characteristics Database",
    "Default/Top Sites",
    "Default/Top Sites-journal",
    "Default/Trust Tokens",
    "Default/Trust Tokens-journal",
    "Default/Cache",
    "Default/Code Cache",
    "Default/GPUCache",
    "Default/Web Data",
    "Default/Web Data-journal",
    "Default/WebStorage",
    "Default/Network/Cookies",
    "Default/Network/Cookies-journal",
)


@dataclass(frozen=True)
class BrowserRoot:
    name: str
    user_data_dir: Path


@dataclass(frozen=True)
class ExitProfile:
    ip: str | None
    country_code: str | None
    timezone_id: str | None
    latitude: float | None
    longitude: float | None
    city: str | None
    region: str | None
    source: str

    def is_us(self) -> bool:
        return self.country_code == "US"


@dataclass(frozen=True)
class ExitResolution:
    return_code: int
    exit_profile: ExitProfile | None
    verification_state: str
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class GoogleDomainResolution:
    return_code: int
    verification_state: str
    final_url: str | None = None
    hk_markers: tuple[str, ...] = ()
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class BrowserGoogleResolution:
    return_code: int
    verification_state: str
    final_url: str | None = None
    title: str | None = None
    hk_markers: tuple[str, ...] = ()
    process_state: str | None = None
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class RuntimeResolution:
    return_code: int
    verification_state: str
    observed: dict | None = None
    process_state: str | None = None
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class AutomationContextResolution:
    return_code: int
    verification_state: str
    observed: dict | None = None
    process_state: str | None = None
    errors: tuple[str, ...] = ()


def load_json_mapping(path: Path) -> dict:
    if not path.exists():
        return {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"{path} is not valid JSON") from exc
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def write_json_atomic(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as temp_file:
            json.dump(data, temp_file, ensure_ascii=False, indent=2, sort_keys=True)
            temp_file.write("\n")
        os.replace(temp_name, path)
    except Exception:
        try:
            os.unlink(temp_name)
        except FileNotFoundError:
            pass
        raise


def write_text_atomic(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as temp_file:
            temp_file.write(content)
        os.replace(temp_name, path)
    except Exception:
        try:
            os.unlink(temp_name)
        except FileNotFoundError:
            pass
        raise


def known_browser_roots(home: Path, platform_name: str) -> list[BrowserRoot]:
    if platform_name == "darwin":
        base = home / "Library" / "Application Support"
        return [
            BrowserRoot("Google Chrome", base / "Google" / "Chrome"),
            BrowserRoot("Google Chrome Canary", base / "Google" / "Chrome Canary"),
            BrowserRoot("Chromium", base / "Chromium"),
            BrowserRoot("Brave", base / "BraveSoftware" / "Brave-Browser"),
            BrowserRoot("Microsoft Edge", base / "Microsoft Edge"),
        ]
    if platform_name.startswith("win"):
        local_app_data = Path(os.environ.get("LOCALAPPDATA") or home / "AppData" / "Local")
        return [
            BrowserRoot("Google Chrome", local_app_data / "Google" / "Chrome" / "User Data"),
            BrowserRoot("Google Chrome Canary", local_app_data / "Google" / "Chrome SxS" / "User Data"),
            BrowserRoot("Chromium", local_app_data / "Chromium" / "User Data"),
            BrowserRoot("Brave", local_app_data / "BraveSoftware" / "Brave-Browser" / "User Data"),
            BrowserRoot("Microsoft Edge", local_app_data / "Microsoft" / "Edge" / "User Data"),
        ]
    config_home = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
    return [
        BrowserRoot("Google Chrome", config_home / "google-chrome"),
        BrowserRoot("Google Chrome Beta", config_home / "google-chrome-beta"),
        BrowserRoot("Google Chrome Unstable", config_home / "google-chrome-unstable"),
        BrowserRoot("Chromium", config_home / "chromium"),
        BrowserRoot("Brave", config_home / "BraveSoftware" / "Brave-Browser"),
        BrowserRoot("Microsoft Edge", config_home / "microsoft-edge"),
    ]


def resolve_browser_roots(args: argparse.Namespace, platform_name: str) -> list[BrowserRoot]:
    if args.chrome_user_data_dir:
        return [BrowserRoot("Configured Chromium", Path(args.chrome_user_data_dir).expanduser())]

    home = args.home.expanduser()
    roots = [root for root in known_browser_roots(home, platform_name) if root.user_data_dir.exists()]
    if roots:
        return roots

    return [known_browser_roots(home, platform_name)[0]]


def discover_preference_paths(user_data_dir: Path, explicit_profile: str | None) -> list[Path]:
    if explicit_profile:
        return [user_data_dir / explicit_profile / "Preferences"]

    preference_paths = []
    for candidate in sorted(user_data_dir.iterdir(), key=lambda path: path.name):
        if not candidate.is_dir() or candidate.name in SYSTEM_PROFILE_NAMES:
            continue
        preference_path = candidate / "Preferences"
        if preference_path.exists():
            preference_paths.append(preference_path)
    return preference_paths


def ensure_backup(path: Path) -> None:
    if not path.exists():
        return
    backup_path = path.with_name(f"{path.name}.agent-manager-before-google-us")
    if not backup_path.exists():
        shutil.copy2(path, backup_path)


def set_mapping_value(mapping: dict, key: str, value) -> bool:
    changed = mapping.get(key) != value
    mapping[key] = value
    return changed


def mapping_section(parent: dict, key: str) -> dict:
    section = parent.setdefault(key, {})
    if not isinstance(section, dict):
        section = {}
        parent[key] = section
    return section


def clean_string(value) -> str | None:
    if not isinstance(value, str):
        return None
    stripped = value.strip()
    return stripped or None


def first_clean_string(mapping: dict, keys: tuple[str, ...]) -> str | None:
    for key in keys:
        value = clean_string(mapping.get(key))
        if value:
            return value
    return None


def country_code_from_value(value) -> str | None:
    text = clean_string(value)
    if not text or len(text) != 2:
        return None
    return text.upper()


def first_country_code(mapping: dict, keys: tuple[str, ...]) -> str | None:
    for key in keys:
        country_code = country_code_from_value(mapping.get(key))
        if country_code:
            return country_code
    return None


def parse_float(value) -> float | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        return float(value)
    text = clean_string(value)
    if text is None:
        return None
    try:
        return float(text)
    except ValueError:
        return None


def parse_ipinfo_loc(value) -> tuple[float | None, float | None]:
    text = clean_string(value)
    if not text or "," not in text:
        return None, None
    latitude_text, longitude_text = text.split(",", 1)
    return parse_float(latitude_text), parse_float(longitude_text)


def valid_coordinates(latitude: float | None, longitude: float | None) -> tuple[float | None, float | None]:
    if latitude is None or longitude is None:
        return None, None
    if not -90 <= latitude <= 90 or not -180 <= longitude <= 180:
        return None, None
    return latitude, longitude


def first_coordinates(mapping: dict) -> tuple[float | None, float | None]:
    latitude = parse_float(mapping.get("latitude"))
    longitude = parse_float(mapping.get("longitude"))
    if latitude is None:
        latitude = parse_float(mapping.get("lat"))
    if longitude is None:
        longitude = parse_float(mapping.get("lon"))
    if latitude is None or longitude is None:
        loc_latitude, loc_longitude = parse_ipinfo_loc(mapping.get("loc"))
        if latitude is None:
            latitude = loc_latitude
        if longitude is None:
            longitude = loc_longitude
    return valid_coordinates(latitude, longitude)


def valid_timezone_id(value) -> str | None:
    text = clean_string(value)
    if not text or "/" not in text or any(character.isspace() for character in text):
        return None
    return text


def timezone_id_from_payload(payload: dict) -> str | None:
    timezone_value = payload.get("timezone")
    if isinstance(timezone_value, dict):
        for key in ("id", "name", "timezone"):
            timezone_id = valid_timezone_id(timezone_value.get(key))
            if timezone_id:
                return timezone_id
        return None
    return valid_timezone_id(timezone_value)


def exit_profile_from_payload(payload: dict, source: str) -> ExitProfile:
    latitude, longitude = first_coordinates(payload)
    return ExitProfile(
        ip=first_clean_string(payload, ("ip", "query")),
        country_code=first_country_code(payload, ("country_code", "country")),
        timezone_id=timezone_id_from_payload(payload),
        latitude=latitude,
        longitude=longitude,
        city=first_clean_string(payload, ("city",)),
        region=first_clean_string(payload, ("region", "region_name")),
        source=source,
    )


def exit_profile_payload(exit_profile: ExitProfile | None) -> dict:
    if exit_profile is None:
        return {
            "ip": None,
            "country_code": None,
            "timezone_id": None,
            "latitude": None,
            "longitude": None,
            "city": None,
            "region": None,
            "source": None,
        }
    return {
        "ip": exit_profile.ip,
        "country_code": exit_profile.country_code,
        "timezone_id": exit_profile.timezone_id,
        "latitude": exit_profile.latitude,
        "longitude": exit_profile.longitude,
        "city": exit_profile.city,
        "region": exit_profile.region,
        "source": exit_profile.source,
    }


def google_domain_payload(google_domain_resolution: GoogleDomainResolution) -> dict:
    return {
        "verification_state": google_domain_resolution.verification_state,
        "final_url": google_domain_resolution.final_url,
        "hk_markers": list(google_domain_resolution.hk_markers),
        "errors": list(google_domain_resolution.errors),
    }


def browser_google_payload(browser_google_resolution: BrowserGoogleResolution) -> dict:
    return {
        "verification_state": browser_google_resolution.verification_state,
        "final_url": browser_google_resolution.final_url,
        "title": browser_google_resolution.title,
        "hk_markers": list(browser_google_resolution.hk_markers),
        "process_state": browser_google_resolution.process_state,
        "errors": list(browser_google_resolution.errors),
    }


def runtime_payload(runtime_resolution: RuntimeResolution) -> dict:
    return {
        "verification_state": runtime_resolution.verification_state,
        "observed": runtime_resolution.observed,
        "process_state": runtime_resolution.process_state,
        "errors": list(runtime_resolution.errors),
    }


def automation_context_verification_payload(automation_context_resolution: AutomationContextResolution) -> dict:
    return {
        "verification_state": automation_context_resolution.verification_state,
        "observed": automation_context_resolution.observed,
        "process_state": automation_context_resolution.process_state,
        "errors": list(automation_context_resolution.errors),
    }


def accept_language_is_us(value) -> bool:
    text = clean_string(value)
    if text is None:
        return False
    lowered = text.lower()
    first_range = lowered.split(",", 1)[0].split(";", 1)[0].strip()
    return first_range == "en-us" and "zh" not in lowered and "hk" not in lowered


def resolved_timezone_id(exit_profile: ExitProfile | None) -> str:
    if exit_profile and exit_profile.is_us() and exit_profile.timezone_id:
        return exit_profile.timezone_id
    return DEFAULT_TIMEZONE_ID


def resolved_geolocation(exit_profile: ExitProfile | None) -> dict:
    if exit_profile and exit_profile.is_us() and exit_profile.latitude is not None and exit_profile.longitude is not None:
        return {
            "latitude": exit_profile.latitude,
            "longitude": exit_profile.longitude,
            "accuracy": EXIT_PROFILE_GEOLOCATION_ACCURACY,
        }
    return dict(DEFAULT_GEOLOCATION)


def expected_timezone_offset_minutes(timezone_id: str) -> int | None:
    try:
        timezone = ZoneInfo(timezone_id)
    except ZoneInfoNotFoundError:
        return None
    now_utc = datetime.datetime.now(datetime.timezone.utc)
    offset = now_utc.astimezone(timezone).utcoffset()
    if offset is None:
        return None
    return int(round(-offset.total_seconds() / 60))


def sync_local_state(path: Path, enforce_country: bool = True) -> bool:
    data = load_json_mapping(path)
    intl = mapping_section(data, "intl")
    changed = set_mapping_value(intl, "app_locale", APP_LOCALE)
    if enforce_country:
        changed |= set_mapping_value(data, "countryid_at_install", COUNTRY_ID_US)
        changed |= set_mapping_value(data, "variations_country", VARIATIONS_COUNTRY)
        changed |= set_mapping_value(data, "variations_safe_seed_locale", APP_LOCALE)
        changed |= set_mapping_value(data, "variations_safe_seed_permanent_consistency_country", VARIATIONS_COUNTRY)
        changed |= set_mapping_value(data, "variations_safe_seed_session_consistency_country", VARIATIONS_COUNTRY)
    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def sync_preferences(
    path: Path,
    enforce_install_country: bool = True,
    enforce_device_permissions: bool = True,
) -> bool:
    data = load_json_mapping(path)
    changed = False
    if enforce_install_country:
        changed |= set_mapping_value(data, "countryid_at_install", COUNTRY_ID_US)

    intl = mapping_section(data, "intl")
    changed |= set_mapping_value(intl, "accept_languages", ACCEPT_LANGUAGES)
    changed |= set_mapping_value(intl, "selected_languages", ACCEPT_LANGUAGES)
    changed |= set_mapping_value(data, "translate_recent_target", TRANSLATE_TARGET_LANGUAGE)

    account_values = mapping_section(data, "account_values")
    account_intl = mapping_section(account_values, "intl")
    changed |= set_mapping_value(account_intl, "accept_languages", ACCEPT_LANGUAGES)
    changed |= set_mapping_value(account_intl, "selected_languages", ACCEPT_LANGUAGES)
    changed |= set_mapping_value(account_values, "translate_recent_target", TRANSLATE_TARGET_LANGUAGE)

    spellcheck = mapping_section(data, "spellcheck")
    changed |= set_mapping_value(spellcheck, "dictionaries", ["en-US"])
    changed |= set_mapping_value(spellcheck, "dictionary", "en-US")
    changed |= set_mapping_value(spellcheck, "use_spelling_service", True)

    default_search_provider = mapping_section(data, "default_search_provider")
    desired_provider = {
        "enabled": True,
        "keyword": "google.com",
        "name": "Google US",
        "search_url": SEARCH_URL,
        "suggest_url": SUGGEST_URL,
    }
    for key, value in desired_provider.items():
        changed |= set_mapping_value(default_search_provider, key, value)

    web_rtc = mapping_section(data, "webrtc")
    changed |= set_mapping_value(web_rtc, "ip_handling_policy", "disable_non_proxied_udp")
    changed |= set_mapping_value(web_rtc, "multiple_routes_enabled", False)
    changed |= set_mapping_value(web_rtc, "nonproxied_udp_enabled", False)

    profile = mapping_section(data, "profile")
    default_content_settings = mapping_section(profile, "default_content_setting_values")
    changed |= set_mapping_value(default_content_settings, "geolocation", BLOCK_CONTENT_SETTING)
    content_settings = mapping_section(profile, "content_settings")
    exceptions = mapping_section(content_settings, "exceptions")
    changed |= set_mapping_value(exceptions, "geolocation", {})
    changed |= set_mapping_value(exceptions, "geolocation_with_options", {})
    if enforce_device_permissions:
        for setting_name in DEVICE_PERMISSION_CONTENT_SETTINGS:
            changed |= set_mapping_value(default_content_settings, setting_name, BLOCK_CONTENT_SETTING)
            changed |= set_mapping_value(exceptions, setting_name, {})
    changed |= set_mapping_value(exceptions, "reduced_accept_language", {})

    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def managed_profile_volatile_state_targets(user_data_dir: Path) -> list[tuple[str, Path]]:
    root = user_data_dir.resolve(strict=False)
    targets = []
    for relative_path in MANAGED_PROFILE_VOLATILE_STATE_RELATIVE_PATHS:
        relative = Path(relative_path)
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"invalid managed profile volatile path: {relative_path}")
        target = user_data_dir / relative
        if not target.resolve(strict=False).is_relative_to(root):
            raise ValueError(f"managed profile volatile path escapes profile root: {target}")
        targets.append((relative_path, target))
    return targets


def clear_managed_profile_volatile_state(user_data_dir: Path) -> list[str]:
    changed_paths = []
    for _relative_path, target in managed_profile_volatile_state_targets(user_data_dir):
        if not target.exists() and not target.is_symlink():
            continue
        if target.is_dir() and not target.is_symlink():
            shutil.rmtree(target)
        else:
            target.unlink()
        changed_paths.append(str(target))
    return changed_paths


def sync_managed_browser_us_profile(user_data_dir: Path) -> list[str]:
    changed_paths = []
    local_state_path = user_data_dir / "Local State"
    preferences_path = user_data_dir / "Default" / "Preferences"
    if sync_local_state(local_state_path):
        changed_paths.append(str(local_state_path))
    if sync_preferences(preferences_path):
        changed_paths.append(str(preferences_path))
    changed_paths.extend(clear_managed_profile_volatile_state(user_data_dir))
    return changed_paths


def nested_value(mapping: dict, keys: tuple[str, ...]):
    current = mapping
    for key in keys:
        if not isinstance(current, dict) or key not in current:
            return None
        current = current[key]
    return current


def audit_expected_value(data: dict, key_path: tuple[str, ...], expected, mismatches: list[dict]) -> None:
    actual = nested_value(data, key_path)
    if actual != expected:
        mismatches.append(
            {
                "path": ".".join(key_path),
                "expected": expected,
                "actual": actual,
            }
        )


def audit_local_state(path: Path, enforce_country: bool = True) -> dict:
    record = {"path": str(path), "exists": path.exists(), "ok": False, "mismatches": []}
    if not path.exists():
        record["mismatches"].append({"path": str(path), "expected": "existing JSON object", "actual": "missing"})
        return record
    try:
        data = load_json_mapping(path)
    except ValueError as exc:
        record["mismatches"].append({"path": str(path), "expected": "valid JSON object", "actual": str(exc)})
        return record

    audit_expected_value(data, ("intl", "app_locale"), APP_LOCALE, record["mismatches"])
    if enforce_country:
        audit_expected_value(data, ("countryid_at_install",), COUNTRY_ID_US, record["mismatches"])
        audit_expected_value(data, ("variations_country",), VARIATIONS_COUNTRY, record["mismatches"])
        audit_expected_value(data, ("variations_safe_seed_locale",), APP_LOCALE, record["mismatches"])
        audit_expected_value(
            data,
            ("variations_safe_seed_permanent_consistency_country",),
            VARIATIONS_COUNTRY,
            record["mismatches"],
        )
        audit_expected_value(
            data,
            ("variations_safe_seed_session_consistency_country",),
            VARIATIONS_COUNTRY,
            record["mismatches"],
        )
    record["ok"] = not record["mismatches"]
    return record


def audit_preferences(
    path: Path,
    enforce_install_country: bool = True,
    enforce_device_permissions: bool = True,
) -> dict:
    record = {"path": str(path), "exists": path.exists(), "ok": False, "mismatches": []}
    if not path.exists():
        record["mismatches"].append({"path": str(path), "expected": "existing JSON object", "actual": "missing"})
        return record
    try:
        data = load_json_mapping(path)
    except ValueError as exc:
        record["mismatches"].append({"path": str(path), "expected": "valid JSON object", "actual": str(exc)})
        return record

    expected_values = [
        (("intl", "accept_languages"), ACCEPT_LANGUAGES),
        (("intl", "selected_languages"), ACCEPT_LANGUAGES),
        (("account_values", "intl", "accept_languages"), ACCEPT_LANGUAGES),
        (("account_values", "intl", "selected_languages"), ACCEPT_LANGUAGES),
        (("translate_recent_target",), TRANSLATE_TARGET_LANGUAGE),
        (("account_values", "translate_recent_target"), TRANSLATE_TARGET_LANGUAGE),
        (("spellcheck", "dictionaries"), ["en-US"]),
        (("spellcheck", "dictionary"), "en-US"),
        (("spellcheck", "use_spelling_service"), True),
        (("default_search_provider", "enabled"), True),
        (("default_search_provider", "keyword"), "google.com"),
        (("default_search_provider", "name"), "Google US"),
        (("default_search_provider", "search_url"), SEARCH_URL),
        (("default_search_provider", "suggest_url"), SUGGEST_URL),
        (("webrtc", "ip_handling_policy"), "disable_non_proxied_udp"),
        (("webrtc", "multiple_routes_enabled"), False),
        (("webrtc", "nonproxied_udp_enabled"), False),
        (("profile", "default_content_setting_values", "geolocation"), BLOCK_CONTENT_SETTING),
        (("profile", "content_settings", "exceptions", "geolocation"), {}),
        (("profile", "content_settings", "exceptions", "geolocation_with_options"), {}),
        (("profile", "content_settings", "exceptions", "reduced_accept_language"), {}),
    ]
    if enforce_device_permissions:
        expected_values.extend(
            [
                (("profile", "default_content_setting_values", "media_stream_camera"), BLOCK_CONTENT_SETTING),
                (("profile", "default_content_setting_values", "media_stream_mic"), BLOCK_CONTENT_SETTING),
                (("profile", "default_content_setting_values", "notifications"), BLOCK_CONTENT_SETTING),
                (("profile", "content_settings", "exceptions", "media_stream_camera"), {}),
                (("profile", "content_settings", "exceptions", "media_stream_mic"), {}),
                (("profile", "content_settings", "exceptions", "notifications"), {}),
            ]
        )
    if enforce_install_country:
        expected_values.insert(0, (("countryid_at_install",), COUNTRY_ID_US))
    for key_path, expected in expected_values:
        audit_expected_value(data, key_path, expected, record["mismatches"])
    record["ok"] = not record["mismatches"]
    return record


def audit_managed_profile_volatile_state(user_data_dir: Path) -> dict:
    records = []
    for relative_path, target in managed_profile_volatile_state_targets(user_data_dir):
        exists = target.exists() or target.is_symlink()
        records.append(
            {
                "relative_path": relative_path,
                "path": str(target),
                "exists": exists,
                "ok": not exists,
            }
        )
    return {
        "ok": all(record["ok"] for record in records),
        "policy": "cleared_on_every_sync",
        "paths": records,
    }


def audit_browser_state(
    browser_roots: list[BrowserRoot],
    explicit_profile: str | None,
    managed_browser_root: BrowserRoot | None = None,
) -> dict:
    audit = {"overall_ok": True, "roots": []}
    root_scopes = [(browser_root, explicit_profile, False, False, False) for browser_root in browser_roots]
    if managed_browser_root is not None:
        root_scopes.append((managed_browser_root, None, True, True, True))
    for (
        browser_root,
        profile_scope,
        enforce_local_state_country,
        enforce_install_country,
        enforce_device_permissions,
    ) in root_scopes:
        root_record = {
            "name": browser_root.name,
            "user_data_dir": str(browser_root.user_data_dir),
            "exists": browser_root.user_data_dir.exists(),
            "local_state": None,
            "profiles": [],
            "volatile_state": None,
            "warnings": [],
        }
        if not browser_root.user_data_dir.exists():
            root_record["warnings"].append("browser user data directory not found")
            audit["roots"].append(root_record)
            continue

        root_record["local_state"] = audit_local_state(
            browser_root.user_data_dir / "Local State",
            enforce_country=enforce_local_state_country,
        )
        if not root_record["local_state"]["ok"]:
            audit["overall_ok"] = False

        preference_paths = discover_preference_paths(browser_root.user_data_dir, profile_scope)
        if not preference_paths:
            root_record["warnings"].append("no regular profile Preferences found")
            if profile_scope:
                audit["overall_ok"] = False
        for preferences_path in preference_paths:
            profile_record = audit_preferences(
                preferences_path,
                enforce_install_country=enforce_install_country,
                enforce_device_permissions=enforce_device_permissions,
            )
            if not profile_record["ok"]:
                audit["overall_ok"] = False
            root_record["profiles"].append(profile_record)

        if enforce_local_state_country:
            root_record["volatile_state"] = audit_managed_profile_volatile_state(browser_root.user_data_dir)
            if not root_record["volatile_state"]["ok"]:
                audit["overall_ok"] = False

        audit["roots"].append(root_record)
    return audit


def chrome_executable_candidates(platform_name: str) -> list[Path]:
    candidates = []
    configured_chrome = clean_string(os.environ.get("AGENT_MANAGER_BROWSER_US_CHROME_BIN"))
    if configured_chrome:
        candidates.append(Path(configured_chrome).expanduser())
    if platform_name == "darwin":
        candidates.append(Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"))
    elif platform_name.startswith("win"):
        local_app_data = clean_string(os.environ.get("LOCALAPPDATA"))
        if local_app_data:
            candidates.append(Path(local_app_data) / "Google" / "Chrome" / "Application" / "chrome.exe")
    else:
        for executable_name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
            executable = shutil.which(executable_name)
            if executable:
                candidates.append(Path(executable))
    return candidates


def detected_chrome_version(platform_name: str) -> str | None:
    for executable in chrome_executable_candidates(platform_name):
        if not executable.exists():
            continue
        try:
            result = subprocess.run(
                [str(executable), "--version"],
                check=False,
                capture_output=True,
                text=True,
                timeout=3,
            )
        except (OSError, subprocess.TimeoutExpired):
            continue
        version_text = f"{result.stdout} {result.stderr}"
        match = re.search(r"\b(\d+\.\d+\.\d+\.\d+)\b", version_text)
        if match:
            return match.group(1)
    return None


def desktop_user_agent_platform(platform_name: str) -> str:
    if platform_name == "darwin":
        return "Macintosh; Intel Mac OS X 10_15_7"
    if platform_name.startswith("win"):
        return "Windows NT 10.0; Win64; x64"
    return "X11; Linux x86_64"


def client_hints_platform(platform_name: str) -> str:
    if platform_name == "darwin":
        return "macOS"
    if platform_name.startswith("win"):
        return "Windows"
    return "Linux"


def navigator_platform(platform_name: str) -> str:
    if platform_name == "darwin":
        return "MacIntel"
    if platform_name.startswith("win"):
        return "Win32"
    return "Linux x86_64"


def navigator_platform_from_user_agent(user_agent: str) -> str:
    if "Macintosh;" in user_agent:
        return "MacIntel"
    if "Windows" in user_agent:
        return "Win32"
    return "Linux x86_64"


def resolved_us_desktop_user_agent(platform_name: str) -> str:
    chrome_version = detected_chrome_version(platform_name) or FALLBACK_CHROME_VERSION
    return (
        f"Mozilla/5.0 ({desktop_user_agent_platform(platform_name)}) "
        f"AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{chrome_version} Safari/537.36"
    )


def browser_us_chrome_args(user_agent: str) -> list[str]:
    return [
        *BASE_CHROME_ARGS,
        f"--user-agent={user_agent}",
    ]


def automation_context_payload(exit_profile: ExitProfile | None, managed_user_data_dir: Path, platform_name: str) -> dict:
    timezone_id = resolved_timezone_id(exit_profile)
    geolocation = resolved_geolocation(exit_profile)
    user_agent = resolved_us_desktop_user_agent(platform_name)
    config_dir_label = "$AGENT_MANAGER_BROWSER_US_CONFIG_DIR"
    return {
        "browser_context": {
            "locale": APP_LOCALE,
            "timezone_id": timezone_id,
            "user_agent": user_agent,
            "navigator_languages": ["en-US", "en"],
            "client_hints_platform": client_hints_platform(platform_name),
            "navigator_platform": navigator_platform(platform_name),
            "geolocation": geolocation,
            "permissions": ["geolocation"],
            "extra_http_headers": {
                "Accept-Language": "en-US,en;q=0.9",
            },
            "viewport": dict(US_DESKTOP_VIEWPORT),
            "screen": dict(US_DESKTOP_SCREEN),
            "device_scale_factor": US_DESKTOP_DEVICE_SCALE_FACTOR,
            "is_mobile": US_DESKTOP_IS_MOBILE,
            "has_touch": US_DESKTOP_HAS_TOUCH,
            "color_scheme": US_DESKTOP_COLOR_SCHEME,
            "hardware_concurrency": US_DESKTOP_HARDWARE_CONCURRENCY,
            "device_memory_gb": US_DESKTOP_DEVICE_MEMORY_GB,
            "webgl_vendor": US_DESKTOP_WEBGL_VENDOR,
            "webgl_renderer": US_DESKTOP_WEBGL_RENDERER,
        },
        "chrome_launch": {
            "args": [
                f"--user-data-dir={managed_user_data_dir}",
                "--profile-directory=Default",
                *browser_us_chrome_args(user_agent),
            ],
            "launcher": f"{config_dir_label}/browser-us-chrome.sh",
            "managed_user_data_dir": str(managed_user_data_dir),
        },
        "environment": {
            "TZ": timezone_id,
            "LANG": ENV_LOCALE,
            "LANGUAGE": "en_US:en",
            "LC_ALL": ENV_LOCALE,
        },
        "exit_profile": exit_profile_payload(exit_profile),
        "google": {
            "ncr_url": NCR_URL,
            "home_url": GOOGLE_HOME_URL,
            "search_url": SEARCH_URL,
            "suggest_url": SUGGEST_URL,
        },
        "limits": [
            "This config derives timezone and automation geolocation from the current US proxy exit IP when providers return those fields.",
            "This config cannot change proxy exit IP geolocation or override how remote sites classify the public IP.",
            "Managed Browser US profile cookies, history, storage, trust tokens, reporting/NEL, site characteristics, shared dictionaries, top sites, sessions, and caches are cleared on every sync.",
            "Regular browser profiles block geolocation by default to avoid non-US OS/location leaks.",
            "Native Chrome timezone needs DevTools, automation context emulation, or launching through browser-us-chrome.sh.",
            "Native Chrome screen geometry is launched as a stable US desktop profile when browser-us-chrome.sh is used.",
        ],
    }


def shell_export_line(name: str, value: str) -> str:
    return f"export {name}={shlex.quote(value)}"


def browser_us_launcher_content(
    config_path: Path,
    managed_user_data_dir: Path,
    timezone_id: str,
    user_agent: str,
    platform_name: str,
) -> str:
    chrome_args = " ".join(shlex.quote(arg) for arg in browser_us_chrome_args(user_agent))
    user_data_arg = '--user-data-dir="$BROWSER_US_USER_DATA_DIR"'
    lines = [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "",
        "# Generated by agent-manager scripts/sync_browser_google_us.py",
        shell_export_line("TZ", timezone_id),
        shell_export_line("LANG", ENV_LOCALE),
        "export LANGUAGE=en_US:en",
        shell_export_line("LC_ALL", ENV_LOCALE),
        shell_export_line("AGENT_MANAGER_BROWSER_US_CONTEXT", str(config_path)),
        shell_export_line("AGENT_MANAGER_BROWSER_US_CHROME_ARGS", " ".join(browser_us_chrome_args(user_agent))),
        shell_export_line("AGENT_MANAGER_BROWSER_US_USER_AGENT", user_agent),
        shell_export_line("AGENT_MANAGER_BROWSER_US_DEFAULT_USER_DATA_DIR", str(managed_user_data_dir)),
        shell_export_line("AGENT_MANAGER_BROWSER_US_DEFAULT_URL", NCR_URL),
        'export AGENT_MANAGER_BROWSER_US_USER_DATA_DIR="${AGENT_MANAGER_BROWSER_US_USER_DATA_DIR:-$AGENT_MANAGER_BROWSER_US_DEFAULT_USER_DATA_DIR}"',
        'BROWSER_US_USER_DATA_DIR="${AGENT_MANAGER_BROWSER_US_USER_DATA_DIR}"',
        'mkdir -p "$BROWSER_US_USER_DATA_DIR"',
        'if [ "$#" -eq 0 ]; then',
        '  set -- "$AGENT_MANAGER_BROWSER_US_DEFAULT_URL"',
        "fi",
        "",
    ]
    if platform_name == "darwin":
        lines.extend(
            [
                'CHROME_BIN="${AGENT_MANAGER_BROWSER_US_CHROME_BIN:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"',
                'if [ ! -x "$CHROME_BIN" ]; then',
                '  echo "Google Chrome executable not found: $CHROME_BIN" >&2',
                "  exit 127",
                "fi",
                f'exec "$CHROME_BIN" {user_data_arg} --profile-directory=Default {chrome_args} "$@"',
            ]
        )
    elif platform_name.startswith("win"):
        lines.extend(
            [
                'echo "browser-us-chrome.sh is for POSIX shells; use browser-us.env values with your Windows Chrome launcher." >&2',
                "exit 127",
            ]
        )
    else:
        lines.extend(
            [
                'CHROME_BIN="${AGENT_MANAGER_BROWSER_US_CHROME_BIN:-${CHROME_BIN:-}}"',
                'if [ -z "$CHROME_BIN" ]; then',
                '  CHROME_BIN="$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || true)"',
                "fi",
                'if [ -z "$CHROME_BIN" ] || [ ! -x "$CHROME_BIN" ]; then',
                '  echo "Chrome/Chromium executable not found; set AGENT_MANAGER_BROWSER_US_CHROME_BIN." >&2',
                "  exit 127",
                "fi",
                f'exec "$CHROME_BIN" {user_data_arg} --profile-directory=Default {chrome_args} "$@"',
            ]
        )
    return "\n".join(lines) + "\n"


def write_executable_text_atomic(path: Path, content: str) -> None:
    write_text_atomic(path, content)
    path.chmod(0o755)


def sync_automation_context(config_dir: Path, exit_profile: ExitProfile | None, platform_name: str) -> list[str]:
    config_path = config_dir / "browser-us-context.json"
    env_path = config_dir / "browser-us.env"
    launcher_path = config_dir / "browser-us-chrome.sh"
    managed_user_data_dir = config_dir / "browser-us-profile"
    payload = automation_context_payload(exit_profile, managed_user_data_dir, platform_name)
    payload["chrome_launch"]["launcher"] = str(launcher_path)
    timezone_id = payload["environment"]["TZ"]
    user_agent = payload["browser_context"]["user_agent"]

    changed_paths = []
    previous_payload = load_json_mapping(config_path) if config_path.exists() else None
    if previous_payload != payload:
        ensure_backup(config_path)
        write_json_atomic(config_path, payload)
        changed_paths.append(str(config_path))

    env_content = "\n".join(
        [
            "# Generated by agent-manager scripts/sync_browser_google_us.py",
            shell_export_line("TZ", timezone_id),
            shell_export_line("LANG", ENV_LOCALE),
            "export LANGUAGE=en_US:en",
            shell_export_line("LC_ALL", ENV_LOCALE),
            shell_export_line("AGENT_MANAGER_BROWSER_US_CONTEXT", str(config_path)),
            shell_export_line("AGENT_MANAGER_BROWSER_US_CHROME_ARGS", " ".join(browser_us_chrome_args(user_agent))),
            shell_export_line("AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER", str(launcher_path)),
            shell_export_line("AGENT_MANAGER_BROWSER_US_USER_DATA_DIR", str(managed_user_data_dir)),
            shell_export_line("AGENT_MANAGER_BROWSER_US_USER_AGENT", user_agent),
            shell_export_line("AGENT_MANAGER_BROWSER_US_DEFAULT_URL", NCR_URL),
            "",
        ]
    )
    previous_env = env_path.read_text(encoding="utf-8") if env_path.exists() else None
    if previous_env != env_content:
        ensure_backup(env_path)
        write_text_atomic(env_path, env_content)
        changed_paths.append(str(env_path))

    launcher_content = browser_us_launcher_content(config_path, managed_user_data_dir, timezone_id, user_agent, platform_name)
    previous_launcher = launcher_path.read_text(encoding="utf-8") if launcher_path.exists() else None
    launcher_mode = launcher_path.stat().st_mode & 0o777 if launcher_path.exists() else None
    if previous_launcher != launcher_content or launcher_mode != 0o755:
        ensure_backup(launcher_path)
        write_executable_text_atomic(launcher_path, launcher_content)
        changed_paths.append(str(launcher_path))

    return changed_paths


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
                "runtime_desktop_surface": True,
                "runtime_pdf_viewer_enabled": True,
                "runtime_plugins_present": True,
                "runtime_webdriver": False,
                "runtime_geolocation_permission": "denied",
                "runtime_notification_permission": "denied",
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
                "desktop_viewport": dict(US_DESKTOP_VIEWPORT),
                "desktop_screen": dict(US_DESKTOP_SCREEN),
                "desktop_device_scale_factor": US_DESKTOP_DEVICE_SCALE_FACTOR,
                "desktop_is_mobile": US_DESKTOP_IS_MOBILE,
                "desktop_has_touch": US_DESKTOP_HAS_TOUCH,
                "desktop_color_scheme": US_DESKTOP_COLOR_SCHEME,
                "automation_hardware_concurrency": US_DESKTOP_HARDWARE_CONCURRENCY,
                "automation_device_memory_gb": US_DESKTOP_DEVICE_MEMORY_GB,
                "automation_webgl_vendor": US_DESKTOP_WEBGL_VENDOR,
                "automation_webgl_renderer": US_DESKTOP_WEBGL_RENDERER,
                "automation_locale_week_info": {"firstDay": 7, "weekend": [6, 7]},
                "automation_locale_text_info": {"direction": "ltr"},
                "automation_locale_hour_cycles": ["h12"],
                "automation_locale_calendars": ["gregory"],
                "automation_locale_numbering_systems": ["latn"],
                "automation_locale_timezones_include_current_timezone": True,
                "automation_pdf_viewer_enabled": True,
                "automation_plugins_present": True,
                "automation_camera_permission_not_granted": True,
                "automation_microphone_permission_not_granted": True,
                "automation_notifications_permission_not_granted": True,
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


def runtime_probe_html() -> str:
    marker = json.dumps(RUNTIME_JSON_MARKER)
    return (
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body></body>"
        "<script>"
        "(async()=>{"
        "const observed={"
        "userAgent:navigator.userAgent,"
        "platform:navigator.platform,"
        "userAgentDataMobile:navigator.userAgentData?navigator.userAgentData.mobile:null,"
        "userAgentDataPlatform:navigator.userAgentData?navigator.userAgentData.platform:null,"
        "userAgentDataBrands:navigator.userAgentData?Array.from(navigator.userAgentData.brands||[]):null,"
        "userAgentDataHighEntropy:null,"
        "webdriver:navigator.webdriver,"
        "language:navigator.language,"
        "languages:Array.from(navigator.languages||[]),"
        "innerWidth:window.innerWidth,"
        "innerHeight:window.innerHeight,"
        "outerWidth:window.outerWidth,"
        "outerHeight:window.outerHeight,"
        "screenWidth:window.screen.width,"
        "screenHeight:window.screen.height,"
        "availWidth:window.screen.availWidth,"
        "availHeight:window.screen.availHeight,"
        "devicePixelRatio:window.devicePixelRatio,"
        "maxTouchPoints:navigator.maxTouchPoints,"
        "coarsePointer:window.matchMedia('(pointer: coarse)').matches,"
        "finePointer:window.matchMedia('(pointer: fine)').matches,"
        "hoverHover:window.matchMedia('(hover: hover)').matches,"
        "lightColorScheme:window.matchMedia('(prefers-color-scheme: light)').matches,"
        "darkColorScheme:window.matchMedia('(prefers-color-scheme: dark)').matches,"
        "pdfViewerEnabled:navigator.pdfViewerEnabled,"
        "pluginsLength:navigator.plugins.length,"
        "mimeTypesLength:navigator.mimeTypes.length,"
        "timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,"
        "intlLocale:Intl.DateTimeFormat().resolvedOptions().locale,"
        "intlCalendar:Intl.DateTimeFormat().resolvedOptions().calendar,"
        "intlNumberingSystem:Intl.DateTimeFormat().resolvedOptions().numberingSystem,"
        "intlHourCycle:Intl.DateTimeFormat(undefined,{hour:'numeric'}).resolvedOptions().hourCycle,"
        "collatorLocale:Intl.Collator().resolvedOptions().locale,"
        "pluralRulesLocale:new Intl.PluralRules().resolvedOptions().locale,"
        "relativeTimeFormatLocale:new Intl.RelativeTimeFormat().resolvedOptions().locale,"
        "intlLocaleRegion:null,"
        "navigatorLanguageRegion:null,"
        "localeWeekInfo:null,"
        "localeTextInfo:null,"
        "localeHourCycles:null,"
        "localeCalendars:null,"
        "localeNumberingSystems:null,"
        "localeTimeZones:null,"
        "numberFormat:new Intl.NumberFormat().format(1234567.89),"
        "currencyFormat:new Intl.NumberFormat(undefined,{style:'currency',currency:'USD'}).format(1234.56),"
        "dateFormat:new Intl.DateTimeFormat(undefined,{dateStyle:'short',timeZone:'UTC'}).format(new Date(Date.UTC(2026,4,22))),"
        "regionDisplayName:null,"
        "listFormat:new Intl.ListFormat().format(['A','B','C']),"
        "timezoneOffset:new Date().getTimezoneOffset(),"
        "geolocationPermission:null,"
        "notificationPermission:typeof Notification==='function'?Notification.permission:null,"
        "webrtcSupported:typeof RTCPeerConnection==='function',"
        "webrtcCandidateCount:0,"
        "webrtcHostCandidateCount:0,"
        "webrtcMdnsCandidateCount:0,"
        "webrtcPrivateIpCandidateCount:0,"
        "webrtcLocalIpLeak:false"
        "};"
        "try{observed.intlLocaleRegion=new Intl.Locale(observed.intlLocale).region||null;}catch(error){}"
        "try{observed.navigatorLanguageRegion=new Intl.Locale(observed.language).region||null;}catch(error){}"
        "try{const intlLocale=new Intl.Locale(observed.intlLocale);"
        "observed.localeWeekInfo=intlLocale.getWeekInfo?intlLocale.getWeekInfo():null;"
        "observed.localeTextInfo=intlLocale.getTextInfo?intlLocale.getTextInfo():null;"
        "observed.localeHourCycles=intlLocale.getHourCycles?intlLocale.getHourCycles():null;"
        "observed.localeCalendars=intlLocale.getCalendars?intlLocale.getCalendars():null;"
        "observed.localeNumberingSystems=intlLocale.getNumberingSystems?intlLocale.getNumberingSystems():null;"
        "observed.localeTimeZones=intlLocale.getTimeZones?intlLocale.getTimeZones():null;"
        "}catch(error){observed.localeInfoError=String(error&&error.name||error);}"
        "try{observed.regionDisplayName=new Intl.DisplayNames(undefined,{type:'region'}).of('US');}catch(error){}"
        "try{"
        "if(navigator.userAgentData&&navigator.userAgentData.getHighEntropyValues){"
        "observed.userAgentDataHighEntropy=await navigator.userAgentData.getHighEntropyValues(["
        "'architecture','bitness','formFactor','fullVersionList','model','platform',"
        "'platformVersion','uaFullVersion','wow64','mobile'"
        "]);"
        "}"
        "}catch(error){observed.userAgentDataHighEntropyError=String(error&&error.name||error);}"
        "try{"
        "if(navigator.permissions&&navigator.permissions.query){"
        "observed.geolocationPermission=(await navigator.permissions.query({name:'geolocation'})).state;"
        "}"
        "}catch(error){"
        "observed.geolocationPermissionError=String(error&&error.name||error);"
        "}"
        "try{"
        "if(observed.webrtcSupported){"
        "const candidates=[];"
        "const pc=new RTCPeerConnection({iceServers:[]});"
        "pc.createDataChannel('agent-manager-us-probe');"
        "pc.onicecandidate=(event)=>{if(event.candidate&&event.candidate.candidate){candidates.push(event.candidate.candidate);}};"
        "await pc.setLocalDescription(await pc.createOffer());"
        "await new Promise((resolve)=>{"
        "let resolved=false;"
        "const finish=()=>{if(!resolved){resolved=true;resolve();}};"
        "pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete'){finish();}};"
        "setTimeout(finish,1500);"
        "});"
        "pc.close();"
        "const privatePattern=/\\b(10(?:\\.\\d{1,3}){3}|192\\.168(?:\\.\\d{1,3}){2}|172\\.(?:1[6-9]|2\\d|3[0-1])(?:\\.\\d{1,3}){2}|127(?:\\.\\d{1,3}){3}|169\\.254(?:\\.\\d{1,3}){2}|::1|f[cd][0-9a-f]{2}:|fe80:)\\b/i;"
        "observed.webrtcCandidateCount=candidates.length;"
        "observed.webrtcHostCandidateCount=candidates.filter((candidate)=>/\\btyp host\\b/i.test(candidate)).length;"
        "observed.webrtcMdnsCandidateCount=candidates.filter((candidate)=>/\\.local\\b/i.test(candidate)).length;"
        "observed.webrtcPrivateIpCandidateCount=candidates.filter((candidate)=>privatePattern.test(candidate)).length;"
        "observed.webrtcLocalIpLeak=observed.webrtcHostCandidateCount>0||observed.webrtcMdnsCandidateCount>0||observed.webrtcPrivateIpCandidateCount>0;"
        "}"
        "}catch(error){"
        "observed.webrtcProbeError=String(error&&error.name||error);"
        "}"
        "document.body.textContent="
        + marker
        + "+JSON.stringify(observed);"
        "})();"
        "</script></html>"
    )


class RuntimeProbeHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.server.observed_headers["accept-language"] = self.headers.get("Accept-Language")
        self.server.observed_headers["user-agent"] = self.headers.get("User-Agent")
        self.server.observed_headers["sec-ch-ua"] = self.headers.get("Sec-CH-UA")
        self.server.observed_headers["sec-ch-ua-platform"] = self.headers.get("Sec-CH-UA-Platform")
        self.server.observed_headers["sec-ch-ua-mobile"] = self.headers.get("Sec-CH-UA-Mobile")
        body = runtime_probe_html().encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args) -> None:
        return


def start_runtime_probe_server() -> tuple[http.server.ThreadingHTTPServer, threading.Thread, str]:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), RuntimeProbeHandler)
    server.observed_headers = {}
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f"http://127.0.0.1:{server.server_port}/runtime-us-probe"
    return server, thread, url


def runtime_context_from_output(output: str) -> dict | None:
    marker_index = output.rfind(RUNTIME_JSON_MARKER)
    if marker_index == -1:
        return None
    json_start = marker_index + len(RUNTIME_JSON_MARKER)
    json_end = output.find("</", json_start)
    payload_text = output[json_start:] if json_end == -1 else output[json_start:json_end]
    try:
        payload = json.loads(payload_text)
    except json.JSONDecodeError:
        return None
    return payload if isinstance(payload, dict) else None


def chrome_major_version(user_agent: str) -> str | None:
    match = re.search(r"\bChrome/(\d+)\.", user_agent)
    return match.group(1) if match else None


def user_agent_data_brands_match_chrome_major(brands, user_agent: str) -> bool:
    major_version = chrome_major_version(user_agent)
    if major_version is None or not isinstance(brands, list):
        return False
    for brand in brands:
        if not isinstance(brand, dict):
            continue
        if brand.get("brand") in {"Chromium", "Google Chrome"} and str(brand.get("version")) == major_version:
            return True
    return False


def sec_ch_ua_matches_chrome_major(value, user_agent: str) -> bool:
    text = clean_string(value)
    major_version = chrome_major_version(user_agent)
    if text is None or major_version is None:
        return False
    return f'v="{major_version}"' in text and ('"Chromium"' in text or '"Google Chrome"' in text)


def positive_integer(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def user_agent_high_entropy_matches_desktop_surface(
    high_entropy,
    user_agent: str,
    expected_client_hints_platform: str,
) -> bool:
    if not isinstance(high_entropy, dict) or not isinstance(user_agent, str):
        return False
    if high_entropy.get("platform") != expected_client_hints_platform:
        return False
    if high_entropy.get("mobile") is not False:
        return False
    if high_entropy.get("model") not in ("", None):
        return False
    if high_entropy.get("wow64") not in (False, None):
        return False

    full_version_list = high_entropy.get("fullVersionList")
    if isinstance(full_version_list, list) and full_version_list:
        if not user_agent_data_brands_match_chrome_major(full_version_list, user_agent):
            return False

    ua_full_version = clean_string(high_entropy.get("uaFullVersion"))
    major_version = chrome_major_version(user_agent)
    if ua_full_version and (major_version is None or not ua_full_version.startswith(f"{major_version}.")):
        return False

    if "Macintosh; Intel Mac OS X 10_15_7" in user_agent:
        if high_entropy.get("platformVersion") not in ("", "10_15_7", None):
            return False
        if high_entropy.get("architecture") not in ("", "x86", None):
            return False

    return True


def runtime_desktop_surface_mismatches(runtime_context: dict) -> tuple[str, ...]:
    mismatches = []
    if runtime_context.get("innerWidth") != US_DESKTOP_VIEWPORT["width"]:
        mismatches.append("runtime viewport width")
    if runtime_context.get("devicePixelRatio") != US_DESKTOP_DEVICE_SCALE_FACTOR:
        mismatches.append("runtime device scale factor")
    if runtime_context.get("maxTouchPoints") != 0:
        mismatches.append("runtime touch points")
    if runtime_context.get("coarsePointer") is not False:
        mismatches.append("runtime coarse pointer")
    if runtime_context.get("finePointer") is not True:
        mismatches.append("runtime fine pointer")
    if runtime_context.get("hoverHover") is not True:
        mismatches.append("runtime hover")
    if runtime_context.get("lightColorScheme") is not True or runtime_context.get("darkColorScheme") is not False:
        mismatches.append("runtime color scheme")
    if runtime_context.get("pdfViewerEnabled") is not True:
        mismatches.append("runtime pdf viewer")
    if not positive_integer(runtime_context.get("pluginsLength")):
        mismatches.append("runtime plugins")
    if not positive_integer(runtime_context.get("mimeTypesLength")):
        mismatches.append("runtime mime types")
    return tuple(mismatches)


def validate_runtime_context(
    runtime_context: dict,
    timezone_id: str,
    user_agent: str,
    expected_client_hints_platform: str,
) -> tuple[str, ...]:
    mismatches = []
    if runtime_context.get("userAgent") != user_agent:
        mismatches.append("navigator.userAgent")
    if runtime_context.get("userAgentHeader") != user_agent:
        mismatches.append("User-Agent header")
    expected_navigator_platform = navigator_platform_from_user_agent(user_agent)
    if runtime_context.get("platform") != expected_navigator_platform:
        mismatches.append("navigator.platform")
    if runtime_context.get("userAgentDataMobile") not in (False, None):
        mismatches.append("runtime user agent mobile")
    if runtime_context.get("webdriver") not in (False, None):
        mismatches.append("runtime webdriver")
    if not user_agent_data_brands_match_chrome_major(runtime_context.get("userAgentDataBrands"), user_agent):
        mismatches.append("runtime user agent brands")
    user_agent_data_platform = runtime_context.get("userAgentDataPlatform")
    if user_agent_data_platform is not None and user_agent_data_platform != expected_client_hints_platform:
        mismatches.append("runtime user agent platform")
    if runtime_context.get("userAgentDataHighEntropyError") or not user_agent_high_entropy_matches_desktop_surface(
        runtime_context.get("userAgentDataHighEntropy"),
        user_agent,
        expected_client_hints_platform,
    ):
        mismatches.append("runtime user agent high entropy")
    if not sec_ch_ua_matches_chrome_major(runtime_context.get("secChUa"), user_agent):
        mismatches.append("Sec-CH-UA")
    sec_ch_ua_platform = runtime_context.get("secChUaPlatform")
    if sec_ch_ua_platform is not None and sec_ch_ua_platform.strip('"') != expected_client_hints_platform:
        mismatches.append("Sec-CH-UA-Platform")
    sec_ch_ua_mobile = runtime_context.get("secChUaMobile")
    if sec_ch_ua_mobile is not None and sec_ch_ua_mobile != "?0":
        mismatches.append("Sec-CH-UA-Mobile")
    if runtime_context.get("language") != APP_LOCALE:
        mismatches.append("navigator.language")
    if runtime_context.get("languages") != ["en-US", "en"]:
        mismatches.append("navigator.languages")
    mismatches.extend(runtime_desktop_surface_mismatches(runtime_context))
    if runtime_context.get("timezone") != timezone_id:
        mismatches.append("Intl timezone")
    expected_offset = expected_timezone_offset_minutes(timezone_id)
    if expected_offset is None:
        mismatches.append("timezone offset rule")
    elif runtime_context.get("timezoneOffset") != expected_offset:
        mismatches.append("timezone offset")
    if runtime_context.get("intlLocale") != APP_LOCALE:
        mismatches.append("Intl locale")
    if runtime_context.get("intlCalendar") != "gregory":
        mismatches.append("Intl calendar")
    if runtime_context.get("intlNumberingSystem") != "latn":
        mismatches.append("Intl numbering system")
    if runtime_context.get("intlHourCycle") != "h12":
        mismatches.append("Intl hour cycle")
    if runtime_context.get("collatorLocale") != APP_LOCALE:
        mismatches.append("Intl collator locale")
    if runtime_context.get("pluralRulesLocale") != APP_LOCALE:
        mismatches.append("Intl plural rules locale")
    if runtime_context.get("relativeTimeFormatLocale") != APP_LOCALE:
        mismatches.append("Intl relative time format locale")
    if runtime_context.get("intlLocaleRegion") != "US":
        mismatches.append("Intl locale region")
    if runtime_context.get("navigatorLanguageRegion") != "US":
        mismatches.append("navigator language region")
    if runtime_context.get("localeInfoError"):
        mismatches.append("Intl locale info")
    if runtime_context.get("localeWeekInfo") != {"firstDay": 7, "weekend": [6, 7]}:
        mismatches.append("Intl locale week info")
    if runtime_context.get("localeTextInfo") != {"direction": "ltr"}:
        mismatches.append("Intl locale text info")
    locale_hour_cycles = runtime_context.get("localeHourCycles")
    if not isinstance(locale_hour_cycles, list) or "h12" not in locale_hour_cycles:
        mismatches.append("Intl locale hour cycles")
    locale_calendars = runtime_context.get("localeCalendars")
    if not isinstance(locale_calendars, list) or "gregory" not in locale_calendars:
        mismatches.append("Intl locale calendars")
    locale_numbering_systems = runtime_context.get("localeNumberingSystems")
    if not isinstance(locale_numbering_systems, list) or "latn" not in locale_numbering_systems:
        mismatches.append("Intl locale numbering systems")
    locale_time_zones = runtime_context.get("localeTimeZones")
    if not isinstance(locale_time_zones, list) or timezone_id not in locale_time_zones:
        mismatches.append("Intl locale time zones")
    if runtime_context.get("numberFormat") != "1,234,567.89":
        mismatches.append("Intl number format")
    if runtime_context.get("currencyFormat") != "$1,234.56":
        mismatches.append("Intl currency format")
    if runtime_context.get("dateFormat") != "5/22/26":
        mismatches.append("Intl date format")
    if runtime_context.get("regionDisplayName") != "United States":
        mismatches.append("Intl region display name")
    if runtime_context.get("listFormat") != "A, B, and C":
        mismatches.append("Intl list format")
    if not accept_language_is_us(runtime_context.get("acceptLanguageHeader")):
        mismatches.append("Accept-Language")
    if runtime_context.get("geolocationPermission") != "denied":
        mismatches.append("geolocation permission")
    if runtime_context.get("notificationPermission") != "denied":
        mismatches.append("Notification permission")
    if runtime_context.get("profileWebrtcPolicy") != "disable_non_proxied_udp":
        mismatches.append("profile WebRTC policy")
    if runtime_context.get("profileWebrtcNonproxiedUdpEnabled") is not False:
        mismatches.append("profile WebRTC nonproxied UDP")
    if runtime_context.get("webrtcSupported") is not True:
        mismatches.append("WebRTC runtime probe support")
    if runtime_context.get("webrtcProbeError"):
        mismatches.append("WebRTC runtime probe")
    if runtime_context.get("webrtcLocalIpLeak") is not False:
        mismatches.append("WebRTC local IP leak")
    if runtime_context.get("webrtcPrivateIpCandidateCount") != 0:
        mismatches.append("WebRTC private IP candidates")
    if runtime_context.get("profileGeolocationSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile geolocation setting")
    if runtime_context.get("profileCameraSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile camera setting")
    if runtime_context.get("profileMicrophoneSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile microphone setting")
    if runtime_context.get("profileNotificationsSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile notifications setting")
    return tuple(mismatches)


def runtime_profile_policy_snapshot(user_data_dir: Path) -> dict:
    try:
        preferences = load_json_mapping(user_data_dir / "Default" / "Preferences")
    except ValueError:
        preferences = {}
    return {
        "profileWebrtcPolicy": nested_value(preferences, ("webrtc", "ip_handling_policy")),
        "profileWebrtcNonproxiedUdpEnabled": nested_value(preferences, ("webrtc", "nonproxied_udp_enabled")),
        "profileGeolocationSetting": nested_value(preferences, ("profile", "default_content_setting_values", "geolocation")),
        "profileCameraSetting": nested_value(
            preferences,
            ("profile", "default_content_setting_values", "media_stream_camera"),
        ),
        "profileMicrophoneSetting": nested_value(
            preferences,
            ("profile", "default_content_setting_values", "media_stream_mic"),
        ),
        "profileNotificationsSetting": nested_value(
            preferences,
            ("profile", "default_content_setting_values", "notifications"),
        ),
    }


def terminate_process_group(process: subprocess.Popen) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return


def text_from_timeout_output(value) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="ignore")
    return str(value)


def collect_runtime_stdout(process: subprocess.Popen, timeout: float) -> tuple[str, str]:
    if process.stdout is None:
        return "", "stdout_unavailable"

    if os.name == "nt":
        try:
            stdout, _stderr = process.communicate(timeout=timeout)
            return text_from_timeout_output(stdout), f"exited_{process.returncode}"
        except subprocess.TimeoutExpired as exc:
            partial_stdout = text_from_timeout_output(exc.output)
            return partial_stdout, "timed_out"

    deadline = time.monotonic() + timeout
    chunks: list[bytes] = []
    stdout_fd = process.stdout.fileno()

    def decoded_output() -> str:
        return b"".join(chunks).decode("utf-8", errors="ignore")

    while True:
        output = decoded_output()
        if runtime_context_from_output(output) is not None:
            return output, "output_observed"

        if process.poll() is not None:
            while True:
                try:
                    chunk = os.read(stdout_fd, 4096)
                except BlockingIOError:
                    break
                if not chunk:
                    break
                chunks.append(chunk)
            return decoded_output(), f"exited_{process.returncode}"

        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return output, "timed_out"

        readable, _writable, _exceptional = select.select([stdout_fd], [], [], min(0.25, remaining))
        if not readable:
            continue
        chunk = os.read(stdout_fd, 4096)
        if chunk:
            chunks.append(chunk)


def verify_runtime_browser_context(
    launcher_path: Path,
    timezone_id: str,
    user_agent: str,
    expected_client_hints_platform: str,
    timeout: float,
) -> RuntimeResolution:
    if timeout <= 0:
        return RuntimeResolution(5, "invalid_timeout", errors=("runtime timeout must be positive",))
    if not launcher_path.exists() or not os.access(launcher_path, os.X_OK):
        return RuntimeResolution(5, "launcher_missing", errors=(str(launcher_path),))

    with tempfile.TemporaryDirectory(
        prefix="agent-manager-browser-us-runtime-",
        ignore_cleanup_errors=True,
    ) as temp_user_data_dir:
        temp_user_data_path = Path(temp_user_data_dir)
        sync_managed_browser_us_profile(temp_user_data_path)
        profile_policy = runtime_profile_policy_snapshot(temp_user_data_path)
        server, thread, runtime_url = start_runtime_probe_server()
        command = [
            str(launcher_path),
            "--headless",
            "--disable-gpu",
            "--disable-background-networking",
            "--virtual-time-budget=5000",
            "--dump-dom",
            runtime_url,
        ]
        env = os.environ.copy()
        env["AGENT_MANAGER_BROWSER_US_USER_DATA_DIR"] = temp_user_data_dir
        try:
            process = subprocess.Popen(
                command,
                cwd=str(launcher_path.parent),
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            stdout, process_state = collect_runtime_stdout(process, timeout)
            if process.poll() is None:
                terminate_process_group(process)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    runtime_context = runtime_context_from_output(stdout or "")
    if runtime_context is None:
        if process_state == "timed_out":
            process_state = "timed_out_without_output"
        return RuntimeResolution(
            5,
            "runtime_output_missing",
            process_state=process_state,
            errors=(f"runtime output marker missing; process_state={process_state}",),
        )
    runtime_context["acceptLanguageHeader"] = server.observed_headers.get("accept-language")
    runtime_context["userAgentHeader"] = server.observed_headers.get("user-agent")
    runtime_context["secChUa"] = server.observed_headers.get("sec-ch-ua")
    runtime_context["secChUaPlatform"] = server.observed_headers.get("sec-ch-ua-platform")
    runtime_context["secChUaMobile"] = server.observed_headers.get("sec-ch-ua-mobile")
    runtime_context.update(profile_policy)

    mismatches = validate_runtime_context(runtime_context, timezone_id, user_agent, expected_client_hints_platform)
    if mismatches:
        return RuntimeResolution(
            5,
            "runtime_mismatch_failed",
            observed=runtime_context,
            process_state=process_state,
            errors=mismatches,
        )
    return RuntimeResolution(0, "verified_runtime_us", observed=runtime_context, process_state="verified")


def coordinates_match(actual, expected, tolerance: float = 0.000001) -> bool:
    actual_number = parse_float(actual)
    expected_number = parse_float(expected)
    if actual_number is None or expected_number is None:
        return False
    return abs(actual_number - expected_number) <= tolerance


def validate_automation_context_observation(observed: dict, browser_context: dict) -> tuple[str, ...]:
    mismatches = []
    expected_geolocation = browser_context.get("geolocation") if isinstance(browser_context, dict) else None
    observed_geolocation = observed.get("geolocation") if isinstance(observed.get("geolocation"), dict) else None
    expected_viewport = browser_context.get("viewport") if isinstance(browser_context.get("viewport"), dict) else None
    expected_screen = browser_context.get("screen") if isinstance(browser_context.get("screen"), dict) else None
    expected_user_agent = browser_context.get("user_agent")
    expected_client_hints_platform = browser_context.get("client_hints_platform")
    expected_navigator_platform = browser_context.get("navigator_platform")
    if observed.get("userAgent") != expected_user_agent:
        mismatches.append("automation navigator.userAgent")
    if observed.get("userAgentHeader") != expected_user_agent:
        mismatches.append("automation User-Agent header")
    if observed.get("platform") != expected_navigator_platform:
        mismatches.append("automation navigator.platform")
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
    if observed.get("webglVendor") != browser_context.get("webgl_vendor"):
        mismatches.append("automation WebGL vendor")
    if observed.get("webglRenderer") != browser_context.get("webgl_renderer"):
        mismatches.append("automation WebGL renderer")
    if observed.get("pdfViewerEnabled") is not True:
        mismatches.append("automation pdf viewer")
    if not positive_integer(observed.get("pluginsLength")):
        mismatches.append("automation plugins")
    if not positive_integer(observed.get("mimeTypesLength")):
        mismatches.append("automation mime types")
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
                            timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                            timezoneOffset: new Date().getTimezoneOffset(),
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
                            pdfViewerEnabled: navigator.pdfViewerEnabled,
                            pluginsLength: navigator.plugins.length,
                            mimeTypesLength: navigator.mimeTypes.length,
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
    webgl_vendor = browser_context.get("webgl_vendor")
    webgl_renderer = browser_context.get("webgl_renderer")
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


def load_exit_profile(path: Path) -> ExitProfile:
    expanded_path = path.expanduser()
    payload = load_json_mapping(expanded_path)
    exit_profile = exit_profile_from_payload(payload, str(expanded_path))
    if exit_profile.country_code is None:
        raise ValueError(f"{expanded_path} must contain country or country_code")
    return exit_profile


def detect_exit_profile(timeout: float, profile_json: Path | None) -> tuple[ExitProfile | None, list[str]]:
    if profile_json is not None:
        return load_exit_profile(Path(profile_json)), []

    errors = []
    for url in EXIT_PROFILE_PROBES:
        try:
            payload = read_json_url(url, timeout)
        except (OSError, TimeoutError, urllib.error.URLError, ValueError, json.JSONDecodeError) as exc:
            errors.append(f"{url}: {exc}")
            continue
        exit_profile = exit_profile_from_payload(payload, url)
        if exit_profile.country_code:
            return exit_profile, errors
        errors.append(f"{url}: country code missing")
    return None, errors


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Sync Chromium browser state for Google US regional preflight")
    parser.add_argument("--dry-run", action="store_true", help="print planned changes without writing or opening Chrome")
    parser.add_argument("--offline", action="store_true", help="skip browser navigation and exit-country checks")
    parser.add_argument(
        "--home",
        type=Path,
        default=Path.home(),
        help="home directory used to resolve browser and agent-manager config paths",
    )
    parser.add_argument(
        "--chrome-user-data-dir",
        type=Path,
        default=os.environ.get("AGENT_MANAGER_CHROME_USER_DATA_DIR"),
        help="single Chromium user data directory; when omitted, known Chromium-family browser roots are scanned",
    )
    parser.add_argument(
        "--profile",
        default=os.environ.get("AGENT_MANAGER_CHROME_PROFILE"),
        help="limit sync to one profile directory name, such as Default or Profile 1",
    )
    parser.add_argument(
        "--config-dir",
        type=Path,
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_CONFIG_DIR"),
        help="directory for generated automation context files; defaults to ~/.config/agent-manager",
    )
    parser.add_argument(
        "--no-browser-open",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_NO_OPEN") == "1",
        help="do not open google.com/ncr after syncing local profile files",
    )
    parser.add_argument(
        "--skip-exit-check",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_SKIP_EXIT_CHECK") == "1",
        help="skip public exit-country verification",
    )
    parser.add_argument(
        "--allow-non-us-exit",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_ALLOW_NON_US_EXIT") == "1",
        help="warn instead of failing when the detected public exit country is not US",
    )
    parser.add_argument(
        "--allow-unverified-exit",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_ALLOW_UNVERIFIED_EXIT") == "1",
        help="warn instead of failing when the public exit profile cannot be verified",
    )
    parser.add_argument(
        "--exit-check-timeout",
        type=float,
        default=float(os.environ.get("AGENT_MANAGER_BROWSER_US_EXIT_CHECK_TIMEOUT", "5")),
        help="timeout in seconds per public exit-country probe",
    )
    parser.add_argument(
        "--exit-profile-json",
        type=Path,
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_EXIT_PROFILE_JSON"),
        help="read public exit IP profile from a JSON file instead of network probes",
    )
    parser.add_argument(
        "--skip-google-domain-check",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_SKIP_GOOGLE_DOMAIN_CHECK") == "1",
        help="skip the live google.com/ncr domain verification",
    )
    parser.add_argument(
        "--allow-unverified-google-domain",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_ALLOW_UNVERIFIED_GOOGLE_DOMAIN") == "1",
        help="warn instead of failing when google.com/ncr domain verification cannot be completed",
    )
    parser.add_argument(
        "--skip-browser-google-check",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_SKIP_BROWSER_GOOGLE_CHECK") == "1",
        help="skip the live Chrome/Playwright google.com/ncr verification",
    )
    parser.add_argument(
        "--allow-unverified-browser-google",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_ALLOW_UNVERIFIED_BROWSER_GOOGLE") == "1",
        help="warn instead of failing when live Chrome/Playwright google.com/ncr verification cannot be completed",
    )
    parser.add_argument(
        "--browser-google-check-timeout",
        type=float,
        default=float(os.environ.get("AGENT_MANAGER_BROWSER_US_BROWSER_GOOGLE_CHECK_TIMEOUT", "20")),
        help="timeout in seconds for live Chrome/Playwright google.com/ncr verification",
    )
    parser.add_argument(
        "--skip-runtime-check",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_SKIP_RUNTIME_CHECK") == "1",
        help="skip the headless Chrome runtime locale/timezone verification",
    )
    parser.add_argument(
        "--allow-unverified-runtime",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_ALLOW_UNVERIFIED_RUNTIME") == "1",
        help="warn instead of failing when headless Chrome runtime verification cannot be completed",
    )
    parser.add_argument(
        "--runtime-check-timeout",
        type=float,
        default=float(os.environ.get("AGENT_MANAGER_BROWSER_US_RUNTIME_CHECK_TIMEOUT", "20")),
        help="timeout in seconds for headless Chrome runtime verification",
    )
    parser.add_argument(
        "--skip-automation-context-check",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_SKIP_AUTOMATION_CONTEXT_CHECK") == "1",
        help="skip the Playwright automation geolocation/context verification",
    )
    parser.add_argument(
        "--allow-unverified-automation-context",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_ALLOW_UNVERIFIED_AUTOMATION_CONTEXT") == "1",
        help="warn instead of failing when Playwright automation context verification cannot be completed",
    )
    parser.add_argument(
        "--automation-context-check-timeout",
        type=float,
        default=float(os.environ.get("AGENT_MANAGER_BROWSER_US_AUTOMATION_CONTEXT_CHECK_TIMEOUT", "20")),
        help="timeout in seconds for Playwright automation geolocation/context verification",
    )
    parser.add_argument(
        "--post-sync-settle-seconds",
        type=float,
        default=float(os.environ.get("AGENT_MANAGER_BROWSER_US_POST_SYNC_SETTLE_SECONDS", "10")),
        help="seconds to wait before the final browser profile read-back audit",
    )
    return parser


def print_dry_run(args: argparse.Namespace, browser_roots: list[BrowserRoot], config_dir: Path) -> None:
    print("Would sync Google US browser state:")
    print(f"  Agent Manager Browser US: {config_dir / 'browser-us-profile'} (Default)")
    for root in browser_roots:
        profile_label = args.profile or "all regular profiles"
        print(f"  {root.name}: {root.user_data_dir} ({profile_label})")
    print(f"Would write automation context under {config_dir}")
    print(f"Would set browser locale to {APP_LOCALE}")
    print("Would derive automation timezone and geolocation from the current US VPN exit IP when available")
    print(f"Would set accepted languages to {ACCEPT_LANGUAGES}")
    print(f"Would set default Google search URL to {SEARCH_URL}")
    print("Would set WebRTC policy to disable_non_proxied_udp")
    print("Would clear managed Browser US cookies, history, storage, service workers, and caches")
    if args.post_sync_settle_seconds > 0:
        print(f"Would wait {args.post_sync_settle_seconds:g}s before the final browser profile read-back audit")
    if not args.offline and not args.skip_exit_check:
        print("Would verify public proxy exit profile and fail unless a US country is detected")
    print(f"Would write verification report under {config_dir}")
    if not args.offline and not args.skip_exit_check and not args.skip_google_domain_check:
        print("Would verify google.com/ncr keeps Google on google.com instead of google.com.hk")
    if not args.offline and not args.skip_exit_check and not args.skip_browser_google_check:
        print("Would verify Chrome/Playwright keeps Google on google.com instead of google.com.hk")
    if not args.offline and not args.skip_exit_check and not args.skip_runtime_check:
        print("Would verify headless Chrome runtime language, languages, Intl locale, and timezone")
    if not args.offline and not args.skip_exit_check and not args.skip_automation_context_check:
        print("Would verify Playwright automation geolocation and browser context")
    if not args.offline and not args.no_browser_open:
        print(f"Would open {NCR_URL}")


def sync_browser_roots(browser_roots: list[BrowserRoot], explicit_profile: str | None) -> list[str]:
    changed_paths = []
    for browser_root in browser_roots:
        user_data_dir = browser_root.user_data_dir
        if not user_data_dir.exists():
            print(f"WARN: browser user data directory not found: {user_data_dir}", file=sys.stderr)
            continue

        local_state_path = user_data_dir / "Local State"
        if sync_local_state(local_state_path, enforce_country=False):
            changed_paths.append(str(local_state_path))

        preference_paths = discover_preference_paths(user_data_dir, explicit_profile)
        if not preference_paths:
            print(f"WARN: no regular profile Preferences found under: {user_data_dir}", file=sys.stderr)
            continue

        for preferences_path in preference_paths:
            if preferences_path.exists():
                if sync_preferences(
                    preferences_path,
                    enforce_install_country=False,
                    enforce_device_permissions=False,
                ):
                    changed_paths.append(str(preferences_path))
            else:
                print(f"WARN: browser profile Preferences not found: {preferences_path}", file=sys.stderr)
    return changed_paths


def print_browser_state_audit_errors(browser_state_audit: dict) -> None:
    for root_record in browser_state_audit.get("roots", []):
        local_state = root_record.get("local_state")
        if local_state and not local_state.get("ok"):
            for mismatch in local_state.get("mismatches", []):
                print(
                    f"ERROR: {root_record['name']} Local State mismatch: {mismatch}",
                    file=sys.stderr,
                )
        for profile_record in root_record.get("profiles", []):
            if profile_record.get("ok"):
                continue
            for mismatch in profile_record.get("mismatches", []):
                print(
                    f"ERROR: {root_record['name']} Preferences mismatch: {mismatch}",
                    file=sys.stderr,
                )
        volatile_state = root_record.get("volatile_state")
        if volatile_state and not volatile_state.get("ok"):
            for path_record in volatile_state.get("paths", []):
                if path_record.get("exists"):
                    print(
                        f"ERROR: {root_record['name']} volatile browser state still exists: {path_record['path']}",
                        file=sys.stderr,
                    )


def exit_profile_summary(exit_profile: ExitProfile) -> str:
    parts = [f"country: {exit_profile.country_code}"]
    if exit_profile.ip:
        parts.append(f"ip: {exit_profile.ip}")
    if exit_profile.region:
        parts.append(f"region: {exit_profile.region}")
    if exit_profile.city:
        parts.append(f"city: {exit_profile.city}")
    if exit_profile.timezone_id:
        parts.append(f"timezone: {exit_profile.timezone_id}")
    if exit_profile.latitude is not None and exit_profile.longitude is not None:
        parts.append(f"location: {exit_profile.latitude},{exit_profile.longitude}")
    return ", ".join(parts)


def resolve_exit_profile(args: argparse.Namespace) -> ExitResolution:
    if args.offline or args.skip_exit_check:
        if args.offline:
            print("Offline mode: skipped public exit-profile verification")
            verification_state = "offline_skipped"
        else:
            print("Public exit-profile verification skipped")
            verification_state = "manually_skipped"
        return ExitResolution(0, None, verification_state)

    try:
        exit_profile, errors = detect_exit_profile(args.exit_check_timeout, args.exit_profile_json)
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return ExitResolution(2, None, "invalid_exit_profile", (str(exc),))

    if exit_profile is None:
        message = "could not verify public exit profile; cannot prove Google will see a US proxy IP"
        if args.allow_unverified_exit:
            print(f"WARN: {message}", file=sys.stderr)
            for error in errors[:3]:
                print(f"WARN: {error}", file=sys.stderr)
            return ExitResolution(0, None, "unverified_allowed", tuple(errors))
        print(f"ERROR: {message}", file=sys.stderr)
        for error in errors[:3]:
            print(f"ERROR: {error}", file=sys.stderr)
        return ExitResolution(2, None, "unverified_failed", tuple(errors))

    print(f"Detected public exit profile: {exit_profile_summary(exit_profile)} ({exit_profile.source})")
    if exit_profile.is_us():
        return ExitResolution(0, exit_profile, "verified_us")

    message = f"Detected public exit country is {exit_profile.country_code}, not US"
    if args.allow_non_us_exit:
        print(f"WARN: {message}", file=sys.stderr)
        return ExitResolution(0, exit_profile, "non_us_allowed")
    print(f"ERROR: {message}", file=sys.stderr)
    return ExitResolution(2, exit_profile, "non_us_failed")


def resolve_google_domain(args: argparse.Namespace, exit_resolution: ExitResolution) -> GoogleDomainResolution:
    if args.offline:
        return GoogleDomainResolution(0, "offline_skipped")
    if args.skip_exit_check:
        return GoogleDomainResolution(0, "exit_check_skipped")
    if args.skip_google_domain_check:
        return GoogleDomainResolution(0, "manually_skipped")
    if args.exit_profile_json:
        return GoogleDomainResolution(0, "exit_profile_json_skipped")
    if exit_resolution.verification_state != "verified_us":
        return GoogleDomainResolution(0, "exit_not_verified_us_skipped")

    google_domain_resolution = verify_google_domain_after_ncr(args.exit_check_timeout)
    if google_domain_resolution.return_code == 0:
        print(f"Verified Google domain after NCR: {google_domain_resolution.final_url}")
        return google_domain_resolution

    if google_domain_resolution.verification_state == "unverified_failed" and args.allow_unverified_google_domain:
        print("WARN: could not verify google.com/ncr domain state", file=sys.stderr)
        for error in google_domain_resolution.errors[:3]:
            print(f"WARN: {error}", file=sys.stderr)
        return GoogleDomainResolution(
            0,
            "unverified_allowed",
            final_url=google_domain_resolution.final_url,
            hk_markers=google_domain_resolution.hk_markers,
            errors=google_domain_resolution.errors,
        )

    if google_domain_resolution.hk_markers:
        print(
            f"ERROR: google.com/ncr verification still found HK markers: {', '.join(google_domain_resolution.hk_markers)}",
            file=sys.stderr,
        )
    else:
        print("ERROR: could not verify google.com/ncr domain state", file=sys.stderr)
        for error in google_domain_resolution.errors[:3]:
            print(f"ERROR: {error}", file=sys.stderr)
    return google_domain_resolution


def resolve_browser_google(
    args: argparse.Namespace,
    config_dir: Path,
    exit_resolution: ExitResolution,
    platform_name: str,
) -> BrowserGoogleResolution:
    if args.offline:
        return BrowserGoogleResolution(0, "offline_skipped")
    if args.skip_exit_check:
        return BrowserGoogleResolution(0, "exit_check_skipped")
    if args.skip_browser_google_check:
        return BrowserGoogleResolution(0, "manually_skipped")
    if args.exit_profile_json:
        return BrowserGoogleResolution(0, "exit_profile_json_skipped")
    if exit_resolution.verification_state != "verified_us":
        return BrowserGoogleResolution(0, "exit_not_verified_us_skipped")

    browser_google_resolution = verify_google_domain_with_browser_context(
        config_dir,
        args.browser_google_check_timeout,
        platform_name,
    )
    if browser_google_resolution.return_code == 0:
        print(f"Verified browser Google domain after NCR: {browser_google_resolution.final_url}")
        return browser_google_resolution

    if browser_google_resolution.verification_state == "browser_google_failed" and args.allow_unverified_browser_google:
        print("WARN: could not verify browser Google domain state", file=sys.stderr)
        for error in browser_google_resolution.errors[:3]:
            print(f"WARN: {error}", file=sys.stderr)
        return BrowserGoogleResolution(
            0,
            "unverified_allowed",
            final_url=browser_google_resolution.final_url,
            title=browser_google_resolution.title,
            hk_markers=browser_google_resolution.hk_markers,
            process_state=browser_google_resolution.process_state,
            errors=browser_google_resolution.errors,
        )

    if browser_google_resolution.hk_markers:
        print(
            "ERROR: browser Google NCR verification still found HK markers: "
            f"{', '.join(browser_google_resolution.hk_markers)}",
            file=sys.stderr,
        )
    else:
        print("ERROR: could not verify browser Google domain state", file=sys.stderr)
        for error in browser_google_resolution.errors[:3]:
            print(f"ERROR: {error}", file=sys.stderr)
    return browser_google_resolution


def resolve_runtime_context(
    args: argparse.Namespace,
    config_dir: Path,
    exit_resolution: ExitResolution,
    platform_name: str,
) -> RuntimeResolution:
    if args.offline:
        return RuntimeResolution(0, "offline_skipped")
    if args.skip_exit_check:
        return RuntimeResolution(0, "exit_check_skipped")
    if args.skip_runtime_check:
        return RuntimeResolution(0, "manually_skipped")
    if args.exit_profile_json:
        return RuntimeResolution(0, "exit_profile_json_skipped")
    if exit_resolution.verification_state != "verified_us":
        return RuntimeResolution(0, "exit_not_verified_us_skipped")

    user_agent = resolved_us_desktop_user_agent(platform_name)
    expected_client_hints_platform = client_hints_platform(platform_name)
    runtime_resolution = verify_runtime_browser_context(
        config_dir / "browser-us-chrome.sh",
        resolved_timezone_id(exit_resolution.exit_profile),
        user_agent,
        expected_client_hints_platform,
        args.runtime_check_timeout,
    )
    if runtime_resolution.verification_state == "runtime_output_missing":
        runtime_resolution = verify_runtime_browser_context(
            config_dir / "browser-us-chrome.sh",
            resolved_timezone_id(exit_resolution.exit_profile),
            user_agent,
            expected_client_hints_platform,
            args.runtime_check_timeout,
        )
    if runtime_resolution.return_code == 0:
        observed = runtime_resolution.observed or {}
        print(
            "Verified browser runtime: "
            f"language: {observed.get('language')}, "
            f"languages: {observed.get('languages')}, "
            f"timezone: {observed.get('timezone')}, "
            f"User-Agent: {observed.get('userAgent')}, "
            f"Accept-Language: {observed.get('acceptLanguageHeader')}, "
            f"geolocation permission: {observed.get('geolocationPermission')}, "
            f"WebRTC local IP leak: {observed.get('webrtcLocalIpLeak')}"
        )
        return runtime_resolution

    if args.allow_unverified_runtime:
        print("WARN: could not verify browser runtime US state", file=sys.stderr)
        for error in runtime_resolution.errors[:3]:
            print(f"WARN: {error}", file=sys.stderr)
        return RuntimeResolution(
            0,
            "unverified_allowed",
            observed=runtime_resolution.observed,
            process_state=runtime_resolution.process_state,
            errors=runtime_resolution.errors,
        )

    print("ERROR: could not verify browser runtime US state", file=sys.stderr)
    for error in runtime_resolution.errors[:3]:
        print(f"ERROR: {error}", file=sys.stderr)
    return runtime_resolution


def resolve_automation_context(
    args: argparse.Namespace,
    config_dir: Path,
    exit_resolution: ExitResolution,
    platform_name: str,
) -> AutomationContextResolution:
    if args.offline:
        return AutomationContextResolution(0, "offline_skipped")
    if args.skip_exit_check:
        return AutomationContextResolution(0, "exit_check_skipped")
    if args.skip_automation_context_check:
        return AutomationContextResolution(0, "manually_skipped")
    if args.exit_profile_json:
        return AutomationContextResolution(0, "exit_profile_json_skipped")
    if exit_resolution.verification_state != "verified_us":
        return AutomationContextResolution(0, "exit_not_verified_us_skipped")

    automation_context_resolution = verify_automation_browser_context(
        config_dir,
        args.automation_context_check_timeout,
        platform_name,
    )
    if automation_context_resolution.return_code == 0:
        observed = automation_context_resolution.observed or {}
        geolocation = observed.get("geolocation") if isinstance(observed.get("geolocation"), dict) else {}
        print(
            "Verified automation browser context: "
            f"language: {observed.get('language')}, "
            f"languages: {observed.get('languages')}, "
            f"timezone: {observed.get('timezone')}, "
            f"Accept-Language: {observed.get('acceptLanguageHeader')}, "
            f"geolocation: {geolocation.get('latitude')},{geolocation.get('longitude')} "
            f"accuracy: {geolocation.get('accuracy')}"
        )
        return automation_context_resolution

    if args.allow_unverified_automation_context:
        print("WARN: could not verify Playwright automation browser context", file=sys.stderr)
        for error in automation_context_resolution.errors[:3]:
            print(f"WARN: {error}", file=sys.stderr)
        return AutomationContextResolution(
            0,
            "unverified_allowed",
            observed=automation_context_resolution.observed,
            process_state=automation_context_resolution.process_state,
            errors=automation_context_resolution.errors,
        )

    print("ERROR: could not verify Playwright automation browser context", file=sys.stderr)
    for error in automation_context_resolution.errors[:3]:
        print(f"ERROR: {error}", file=sys.stderr)
    return automation_context_resolution


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.post_sync_settle_seconds < 0:
        print("ERROR: --post-sync-settle-seconds must be non-negative", file=sys.stderr)
        return 2
    if args.runtime_check_timeout <= 0:
        print("ERROR: --runtime-check-timeout must be positive", file=sys.stderr)
        return 2
    if args.browser_google_check_timeout <= 0:
        print("ERROR: --browser-google-check-timeout must be positive", file=sys.stderr)
        return 2
    if args.automation_context_check_timeout <= 0:
        print("ERROR: --automation-context-check-timeout must be positive", file=sys.stderr)
        return 2
    platform_name = sys.platform
    config_dir = Path(args.config_dir).expanduser() if args.config_dir else args.home.expanduser() / ".config" / "agent-manager"
    browser_roots = resolve_browser_roots(args, platform_name)
    managed_browser_root = BrowserRoot("Agent Manager Browser US", config_dir / "browser-us-profile")

    if args.dry_run:
        print_dry_run(args, browser_roots, config_dir)
        return 0

    exit_resolution = resolve_exit_profile(args)
    if exit_resolution.return_code != 0:
        return exit_resolution.return_code

    changed_paths = []
    changed_paths.extend(sync_managed_browser_us_profile(managed_browser_root.user_data_dir))
    changed_paths.extend(sync_browser_roots(browser_roots, args.profile))
    changed_paths.extend(sync_automation_context(config_dir, exit_resolution.exit_profile, platform_name))
    if args.post_sync_settle_seconds > 0:
        time.sleep(args.post_sync_settle_seconds)
    browser_state_audit = audit_browser_state(browser_roots, args.profile, managed_browser_root)
    google_domain_resolution = resolve_google_domain(args, exit_resolution)
    if google_domain_resolution.return_code == 0:
        browser_google_resolution = resolve_browser_google(args, config_dir, exit_resolution, platform_name)
        if browser_google_resolution.return_code == 0:
            runtime_resolution = resolve_runtime_context(args, config_dir, exit_resolution, platform_name)
        else:
            runtime_resolution = RuntimeResolution(0, "browser_google_failed_skipped")
        if browser_google_resolution.return_code == 0 and runtime_resolution.return_code == 0:
            automation_context_resolution = resolve_automation_context(args, config_dir, exit_resolution, platform_name)
        elif browser_google_resolution.return_code != 0:
            automation_context_resolution = AutomationContextResolution(0, "browser_google_failed_skipped")
        else:
            automation_context_resolution = AutomationContextResolution(0, "runtime_failed_skipped")
    else:
        browser_google_resolution = BrowserGoogleResolution(0, "google_domain_failed_skipped")
        runtime_resolution = RuntimeResolution(0, "google_domain_failed_skipped")
        automation_context_resolution = AutomationContextResolution(0, "google_domain_failed_skipped")
    changed_paths.extend(
        sync_verification_report(
            config_dir,
            browser_roots,
            managed_browser_root,
            args.profile,
            exit_resolution.exit_profile,
            platform_name,
            exit_resolution.verification_state,
            google_domain_resolution,
            browser_google_resolution,
            runtime_resolution,
            automation_context_resolution,
            browser_state_audit,
        )
    )
    if not browser_state_audit["overall_ok"]:
        print_browser_state_audit_errors(browser_state_audit)
        return 3
    if google_domain_resolution.return_code != 0:
        return google_domain_resolution.return_code
    if browser_google_resolution.return_code != 0:
        return browser_google_resolution.return_code
    if runtime_resolution.return_code != 0:
        return runtime_resolution.return_code
    if automation_context_resolution.return_code != 0:
        return automation_context_resolution.return_code

    opened_ncr = False
    if args.offline:
        print("Offline mode: skipped google.com/ncr browser bootstrap")
    elif args.no_browser_open:
        print("Browser open disabled: skipped google.com/ncr browser bootstrap")
    else:
        opened_ncr = open_google_ncr(platform_name, config_dir)

    if changed_paths:
        print("Synced Google US browser state:")
        for path in changed_paths:
            print(f"  {path}")
    else:
        print("Google US browser state already matched desired settings")

    if opened_ncr:
        print(f"Opened {NCR_URL} to refresh no-country-redirect browser state")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
