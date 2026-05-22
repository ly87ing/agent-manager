#!/usr/bin/env python3
from __future__ import annotations

import argparse
import http.cookiejar
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path


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
EXIT_PROFILE_GEOLOCATION_ACCURACY = 50000
VERIFICATION_REPORT_VERSION = 1
CHROME_ARGS = [
    "--lang=en-US",
    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
]
BLOCK_CONTENT_SETTING = 2
GOOGLE_HK_MARKERS = ("google.com.hk", "hong kong")
SYSTEM_PROFILE_NAMES = {"System Profile", "Guest Profile"}


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


def sync_local_state(path: Path) -> bool:
    data = load_json_mapping(path)
    intl = mapping_section(data, "intl")
    changed = set_mapping_value(intl, "app_locale", APP_LOCALE)
    changed |= set_mapping_value(data, "countryid_at_install", COUNTRY_ID_US)
    changed |= set_mapping_value(data, "variations_country", VARIATIONS_COUNTRY)
    changed |= set_mapping_value(data, "variations_safe_seed_locale", APP_LOCALE)
    changed |= set_mapping_value(data, "variations_safe_seed_permanent_consistency_country", VARIATIONS_COUNTRY)
    changed |= set_mapping_value(data, "variations_safe_seed_session_consistency_country", VARIATIONS_COUNTRY)
    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def sync_preferences(path: Path) -> bool:
    data = load_json_mapping(path)
    changed = False
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
    changed |= set_mapping_value(exceptions, "reduced_accept_language", {})

    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


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


def audit_local_state(path: Path) -> dict:
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


def audit_preferences(path: Path) -> dict:
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
        (("countryid_at_install",), COUNTRY_ID_US),
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
    for key_path, expected in expected_values:
        audit_expected_value(data, key_path, expected, record["mismatches"])
    record["ok"] = not record["mismatches"]
    return record


def audit_browser_state(browser_roots: list[BrowserRoot], explicit_profile: str | None) -> dict:
    audit = {"overall_ok": True, "roots": []}
    for browser_root in browser_roots:
        root_record = {
            "name": browser_root.name,
            "user_data_dir": str(browser_root.user_data_dir),
            "exists": browser_root.user_data_dir.exists(),
            "local_state": None,
            "profiles": [],
            "warnings": [],
        }
        if not browser_root.user_data_dir.exists():
            root_record["warnings"].append("browser user data directory not found")
            audit["roots"].append(root_record)
            continue

        root_record["local_state"] = audit_local_state(browser_root.user_data_dir / "Local State")
        if not root_record["local_state"]["ok"]:
            audit["overall_ok"] = False

        preference_paths = discover_preference_paths(browser_root.user_data_dir, explicit_profile)
        if not preference_paths:
            root_record["warnings"].append("no regular profile Preferences found")
            if explicit_profile:
                audit["overall_ok"] = False
        for preferences_path in preference_paths:
            profile_record = audit_preferences(preferences_path)
            if not profile_record["ok"]:
                audit["overall_ok"] = False
            root_record["profiles"].append(profile_record)

        audit["roots"].append(root_record)
    return audit


def automation_context_payload(exit_profile: ExitProfile | None) -> dict:
    timezone_id = resolved_timezone_id(exit_profile)
    geolocation = resolved_geolocation(exit_profile)
    config_dir_label = "$AGENT_MANAGER_BROWSER_US_CONFIG_DIR"
    return {
        "browser_context": {
            "locale": APP_LOCALE,
            "timezone_id": timezone_id,
            "geolocation": geolocation,
            "permissions": ["geolocation"],
            "extra_http_headers": {
                "Accept-Language": "en-US,en;q=0.9",
            },
        },
        "chrome_launch": {
            "args": CHROME_ARGS,
            "launcher": f"{config_dir_label}/browser-us-chrome.sh",
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
            "Regular browser profiles block geolocation by default to avoid non-US OS/location leaks.",
            "Native Chrome timezone needs DevTools, automation context emulation, or launching through browser-us-chrome.sh.",
        ],
    }


def shell_export_line(name: str, value: str) -> str:
    return f"export {name}={shlex.quote(value)}"


def browser_us_launcher_content(config_path: Path, timezone_id: str, platform_name: str) -> str:
    chrome_args = " ".join(shlex.quote(arg) for arg in CHROME_ARGS)
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
        shell_export_line("AGENT_MANAGER_BROWSER_US_CHROME_ARGS", " ".join(CHROME_ARGS)),
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
                f'exec "$CHROME_BIN" {chrome_args} "$@"',
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
                f'exec "$CHROME_BIN" {chrome_args} "$@"',
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
    payload = automation_context_payload(exit_profile)
    payload["chrome_launch"]["launcher"] = str(launcher_path)
    timezone_id = payload["environment"]["TZ"]

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
            shell_export_line("AGENT_MANAGER_BROWSER_US_CHROME_ARGS", " ".join(CHROME_ARGS)),
            shell_export_line("AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER", str(launcher_path)),
            "",
        ]
    )
    previous_env = env_path.read_text(encoding="utf-8") if env_path.exists() else None
    if previous_env != env_content:
        ensure_backup(env_path)
        write_text_atomic(env_path, env_content)
        changed_paths.append(str(env_path))

    launcher_content = browser_us_launcher_content(config_path, timezone_id, platform_name)
    previous_launcher = launcher_path.read_text(encoding="utf-8") if launcher_path.exists() else None
    launcher_mode = launcher_path.stat().st_mode & 0o777 if launcher_path.exists() else None
    if previous_launcher != launcher_content or launcher_mode != 0o755:
        ensure_backup(launcher_path)
        write_executable_text_atomic(launcher_path, launcher_content)
        changed_paths.append(str(launcher_path))

    return changed_paths


def verification_report_payload(
    browser_roots: list[BrowserRoot],
    explicit_profile: str | None,
    exit_profile: ExitProfile | None,
    verification_state: str,
    google_domain_resolution: GoogleDomainResolution,
    browser_state_audit: dict,
) -> dict:
    timezone_id = resolved_timezone_id(exit_profile)
    geolocation = resolved_geolocation(exit_profile)
    state_ok = browser_state_audit.get("overall_ok") is True
    google_domain_ok = google_domain_resolution.return_code == 0
    if verification_state == "verified_us" and state_ok and google_domain_ok:
        overall_status = "complete"
    elif state_ok and google_domain_ok:
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
        "browser_state_audit": browser_state_audit,
        "browser_roots": [
            {
                "name": browser_root.name,
                "user_data_dir": str(browser_root.user_data_dir),
                "profile_scope": explicit_profile or "all regular profiles",
            }
            for browser_root in browser_roots
        ],
        "enforced_features": {
            "locale": {
                "app_locale": APP_LOCALE,
                "accept_languages": ACCEPT_LANGUAGES,
                "selected_languages": ACCEPT_LANGUAGES,
                "account_accept_languages": ACCEPT_LANGUAGES,
                "account_selected_languages": ACCEPT_LANGUAGES,
                "reduced_accept_language_exceptions": {},
                "environment_locale": ENV_LOCALE,
            },
            "local_state_country": {
                "countryid_at_install": COUNTRY_ID_US,
                "variations_country": VARIATIONS_COUNTRY,
                "variations_safe_seed_locale": APP_LOCALE,
                "variations_safe_seed_permanent_consistency_country": VARIATIONS_COUNTRY,
                "variations_safe_seed_session_consistency_country": VARIATIONS_COUNTRY,
            },
            "profile_country": {
                "countryid_at_install": COUNTRY_ID_US,
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
            },
            "webrtc": {
                "ip_handling_policy": "disable_non_proxied_udp",
                "multiple_routes_enabled": False,
                "nonproxied_udp_enabled": False,
                "chrome_launch_policy": "disable_non_proxied_udp",
            },
            "geolocation": {
                "regular_profile_content_setting": BLOCK_CONTENT_SETTING,
                "regular_profile_policy": "block",
                "regular_profile_exceptions": {},
                "regular_profile_geolocation_with_options_exceptions": {},
                "automation_geolocation": geolocation,
            },
            "automation": {
                "locale": APP_LOCALE,
                "timezone_id": timezone_id,
                "extra_http_accept_language": "en-US,en;q=0.9",
                "chrome_launch_args": CHROME_ARGS,
                "chrome_launcher": "$AGENT_MANAGER_BROWSER_US_CHROME_LAUNCHER",
            },
        },
        "limits": [
            "Remote services can still classify the public VPN IP by their own IP reputation databases.",
            "Google account history, existing cookies, installed extensions, fonts, GPU, canvas, and hardware fingerprints are outside this local sync script.",
            "Native Chrome timezone changes require launching through browser-us-chrome.sh or an automation context that applies browser-us-context.json.",
        ],
    }


def sync_verification_report(
    config_dir: Path,
    browser_roots: list[BrowserRoot],
    explicit_profile: str | None,
    exit_profile: ExitProfile | None,
    verification_state: str,
    google_domain_resolution: GoogleDomainResolution,
    browser_state_audit: dict,
) -> list[str]:
    report_path = config_dir / "browser-us-verification.json"
    payload = verification_report_payload(
        browser_roots,
        explicit_profile,
        exit_profile,
        verification_state,
        google_domain_resolution,
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
        "--post-sync-settle-seconds",
        type=float,
        default=float(os.environ.get("AGENT_MANAGER_BROWSER_US_POST_SYNC_SETTLE_SECONDS", "10")),
        help="seconds to wait before the final browser profile read-back audit",
    )
    return parser


def print_dry_run(args: argparse.Namespace, browser_roots: list[BrowserRoot], config_dir: Path) -> None:
    print("Would sync Google US browser state:")
    for root in browser_roots:
        profile_label = args.profile or "all regular profiles"
        print(f"  {root.name}: {root.user_data_dir} ({profile_label})")
    print(f"Would write automation context under {config_dir}")
    print(f"Would set browser locale to {APP_LOCALE}")
    print("Would derive automation timezone and geolocation from the current US VPN exit IP when available")
    print(f"Would set accepted languages to {ACCEPT_LANGUAGES}")
    print(f"Would set default Google search URL to {SEARCH_URL}")
    print("Would set WebRTC policy to disable_non_proxied_udp")
    if args.post_sync_settle_seconds > 0:
        print(f"Would wait {args.post_sync_settle_seconds:g}s before the final browser profile read-back audit")
    if not args.offline and not args.skip_exit_check:
        print("Would verify public proxy exit profile and fail unless a US country is detected")
    print(f"Would write verification report under {config_dir}")
    if not args.offline and not args.skip_exit_check and not args.skip_google_domain_check:
        print("Would verify google.com/ncr keeps Google on google.com instead of google.com.hk")
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
        if sync_local_state(local_state_path):
            changed_paths.append(str(local_state_path))

        preference_paths = discover_preference_paths(user_data_dir, explicit_profile)
        if not preference_paths:
            print(f"WARN: no regular profile Preferences found under: {user_data_dir}", file=sys.stderr)
            continue

        for preferences_path in preference_paths:
            if preferences_path.exists():
                if sync_preferences(preferences_path):
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


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.post_sync_settle_seconds < 0:
        print("ERROR: --post-sync-settle-seconds must be non-negative", file=sys.stderr)
        return 2
    platform_name = sys.platform
    config_dir = Path(args.config_dir).expanduser() if args.config_dir else args.home.expanduser() / ".config" / "agent-manager"
    browser_roots = resolve_browser_roots(args, platform_name)

    if args.dry_run:
        print_dry_run(args, browser_roots, config_dir)
        return 0

    exit_resolution = resolve_exit_profile(args)
    if exit_resolution.return_code != 0:
        return exit_resolution.return_code

    changed_paths = []
    changed_paths.extend(sync_browser_roots(browser_roots, args.profile))
    changed_paths.extend(sync_automation_context(config_dir, exit_resolution.exit_profile, platform_name))
    if args.post_sync_settle_seconds > 0:
        time.sleep(args.post_sync_settle_seconds)
    browser_state_audit = audit_browser_state(browser_roots, args.profile)
    google_domain_resolution = resolve_google_domain(args, exit_resolution)
    changed_paths.extend(
        sync_verification_report(
            config_dir,
            browser_roots,
            args.profile,
            exit_resolution.exit_profile,
            exit_resolution.verification_state,
            google_domain_resolution,
            browser_state_audit,
        )
    )
    if not browser_state_audit["overall_ok"]:
        print_browser_state_audit_errors(browser_state_audit)
        return 3
    if google_domain_resolution.return_code != 0:
        return google_domain_resolution.return_code

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
