#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path


ACCEPT_LANGUAGES = "en-US,en"
APP_LOCALE = "en-US"
ENV_LOCALE = "en_US.UTF-8"
NCR_URL = "https://www.google.com/ncr"
SEARCH_URL = "https://www.google.com/search?q={searchTerms}&hl=en-US&gl=US&pws=0"
SUGGEST_URL = "https://www.google.com/complete/search?client=chrome&q={searchTerms}&hl=en-US&gl=US"
DEFAULT_TIMEZONE_ID = "America/Los_Angeles"
DEFAULT_GEOLOCATION = {
    "latitude": 37.7749,
    "longitude": -122.4194,
    "accuracy": 100,
}
EXIT_PROFILE_GEOLOCATION_ACCURACY = 50000
CHROME_ARGS = [
    "--lang=en-US",
    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
]
BLOCK_CONTENT_SETTING = 2
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
    changed |= set_mapping_value(data, "countryid_at_install", 244)
    changed |= set_mapping_value(data, "variations_country", "us")
    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def sync_preferences(path: Path) -> bool:
    data = load_json_mapping(path)
    changed = False

    intl = mapping_section(data, "intl")
    changed |= set_mapping_value(intl, "accept_languages", ACCEPT_LANGUAGES)
    changed |= set_mapping_value(intl, "selected_languages", ACCEPT_LANGUAGES)

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

    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def automation_context_payload(exit_profile: ExitProfile | None) -> dict:
    timezone_id = resolved_timezone_id(exit_profile)
    geolocation = resolved_geolocation(exit_profile)
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
            "search_url": SEARCH_URL,
            "suggest_url": SUGGEST_URL,
        },
        "limits": [
            "This config derives timezone and automation geolocation from the current US proxy exit IP when providers return those fields.",
            "This config cannot change proxy exit IP geolocation or override how remote sites classify the public IP.",
            "Regular browser profiles block geolocation by default to avoid non-US OS/location leaks.",
            "Native Chrome timezone needs DevTools, automation context emulation, or launching through browser-us.env.",
        ],
    }


def sync_automation_context(config_dir: Path, exit_profile: ExitProfile | None) -> list[str]:
    config_path = config_dir / "browser-us-context.json"
    env_path = config_dir / "browser-us.env"
    payload = automation_context_payload(exit_profile)
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
            f"export TZ={shlex.quote(timezone_id)}",
            f"export LANG={shlex.quote(ENV_LOCALE)}",
            "export LANGUAGE=en_US:en",
            f"export LC_ALL={shlex.quote(ENV_LOCALE)}",
            f"export AGENT_MANAGER_BROWSER_US_CONTEXT={shlex.quote(str(config_path))}",
            f"export AGENT_MANAGER_BROWSER_US_CHROME_ARGS={shlex.quote(' '.join(CHROME_ARGS))}",
            "",
        ]
    )
    previous_env = env_path.read_text(encoding="utf-8") if env_path.exists() else None
    if previous_env != env_content:
        ensure_backup(env_path)
        write_text_atomic(env_path, env_content)
        changed_paths.append(str(env_path))

    return changed_paths


def browser_open_command(platform_name: str, url: str) -> list[str] | None:
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


def open_google_ncr(platform_name: str) -> bool:
    command = browser_open_command(platform_name, NCR_URL)
    if command is None:
        print("WARN: no browser opener found; skipped google.com/ncr bootstrap", file=sys.stderr)
        return False
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
    if not args.offline and not args.skip_exit_check:
        print("Would verify public proxy exit profile and fail if a non-US country is detected")
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


def resolve_exit_profile(args: argparse.Namespace) -> tuple[int, ExitProfile | None]:
    if args.offline or args.skip_exit_check:
        if args.offline:
            print("Offline mode: skipped public exit-profile verification")
        else:
            print("Public exit-profile verification skipped")
        return 0, None

    try:
        exit_profile, errors = detect_exit_profile(args.exit_check_timeout, args.exit_profile_json)
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2, None

    if exit_profile is None:
        print("WARN: could not verify public exit profile; Google may still classify by proxy IP", file=sys.stderr)
        for error in errors[:3]:
            print(f"WARN: {error}", file=sys.stderr)
        return 0, None

    print(f"Detected public exit profile: {exit_profile_summary(exit_profile)} ({exit_profile.source})")
    if exit_profile.is_us():
        return 0, exit_profile

    message = f"Detected public exit country is {exit_profile.country_code}, not US"
    if args.allow_non_us_exit:
        print(f"WARN: {message}", file=sys.stderr)
        return 0, exit_profile
    print(f"ERROR: {message}", file=sys.stderr)
    return 2, exit_profile


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    platform_name = sys.platform
    config_dir = Path(args.config_dir).expanduser() if args.config_dir else args.home.expanduser() / ".config" / "agent-manager"
    browser_roots = resolve_browser_roots(args, platform_name)

    if args.dry_run:
        print_dry_run(args, browser_roots, config_dir)
        return 0

    exit_status, exit_profile = resolve_exit_profile(args)
    if exit_status != 0:
        return exit_status

    changed_paths = []
    changed_paths.extend(sync_browser_roots(browser_roots, args.profile))
    changed_paths.extend(sync_automation_context(config_dir, exit_profile))

    opened_ncr = False
    if args.offline:
        print("Offline mode: skipped google.com/ncr browser bootstrap")
    elif args.no_browser_open:
        print("Browser open disabled: skipped google.com/ncr browser bootstrap")
    else:
        opened_ncr = open_google_ncr(platform_name)

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
