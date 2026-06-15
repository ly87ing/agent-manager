"""Extracted from sync_browser_google_us.py: helpers layer."""
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
    return expected_timezone_offset_minutes_at(timezone_id, datetime.datetime.now(datetime.timezone.utc))


def expected_timezone_offset_minutes_at(timezone_id: str, utc_datetime: datetime.datetime) -> int | None:
    try:
        timezone = ZoneInfo(timezone_id)
    except ZoneInfoNotFoundError:
        return None
    offset = utc_datetime.astimezone(timezone).utcoffset()
    if offset is None:
        return None
    return int(round(-offset.total_seconds() / 60))


def timezone_offset_label(offset_minutes: int | None) -> str | None:
    if offset_minutes is None:
        return None
    sign = "-" if offset_minutes >= 0 else "+"
    absolute_minutes = abs(offset_minutes)
    return f"GMT{sign}{absolute_minutes // 60:02d}{absolute_minutes % 60:02d}"


def us_timezone_display_needles(timezone_id: str) -> tuple[str, ...]:
    if timezone_id in {"America/Los_Angeles", "America/Vancouver", "America/Tijuana"}:
        return ("Pacific",)
    if timezone_id.startswith("America/Indiana/") or timezone_id.startswith("America/Kentucky/"):
        return ("Eastern",)
    if timezone_id in {"America/New_York", "America/Detroit", "America/Toronto", "America/Nassau"}:
        return ("Eastern",)
    if timezone_id in {"America/Chicago", "America/Indiana/Knox", "America/Menominee"}:
        return ("Central",)
    if timezone_id.startswith("America/North_Dakota/"):
        return ("Central",)
    if timezone_id in {"America/Denver", "America/Boise"}:
        return ("Mountain",)
    if timezone_id == "America/Phoenix":
        return ("Mountain",)
    if timezone_id in {"America/Anchorage", "America/Juneau", "America/Nome", "America/Sitka", "America/Yakutat"}:
        return ("Alaska",)
    if timezone_id == "America/Adak":
        return ("Hawaii", "Aleutian")
    if timezone_id == "Pacific/Honolulu":
        return ("Hawaii",)
    if timezone_id == "America/Puerto_Rico":
        return ("Atlantic",)
    if timezone_id in {"Pacific/Guam", "Pacific/Saipan"}:
        return ("Chamorro",)
    if timezone_id == "Pacific/Pago_Pago":
        return ("Samoa",)
    return ()


def timezone_display_matches(observed: dict, timezone_id: str) -> bool:
    offset_label = timezone_offset_label(expected_timezone_offset_minutes_at(timezone_id, US_TIMEZONE_PROBE_UTC))
    combined = " ".join(
        str(observed.get(field) or "")
        for field in ("dateToString", "dateToTimeString", "timeZoneLongFormat", "timeZoneNameLong")
    )
    lowered = combined.lower()
    if any(marker in lowered for marker in ("hong kong", "china standard", "gmt+0800", "hkt")):
        return False
    if offset_label and offset_label not in combined:
        return False
    needles = us_timezone_display_needles(timezone_id)
    return not needles or any(needle in combined for needle in needles)


def permission_states_allow_no_grants(observed: dict, permission_names: tuple[str, ...] = DENIED_PERMISSION_PROBES) -> bool:
    permission_states = observed.get("permissionStates")
    if not isinstance(permission_states, dict):
        return False
    for permission_name in permission_names:
        if permission_states.get(permission_name) == "granted":
            return False
    return True

