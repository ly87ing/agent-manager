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
from typing import Callable


ACCEPT_LANGUAGES = "en-US,en"
APP_LOCALE = "en-US"
ENV_LOCALE = "en_US.UTF-8"
NCR_URL = "https://www.google.com/ncr"
SEARCH_URL = "https://www.google.com/search?q={searchTerms}&hl=en-US&gl=US&pws=0"
SUGGEST_URL = "https://www.google.com/complete/search?client=chrome&q={searchTerms}&hl=en-US&gl=US"
TIMEZONE_ID = "America/Los_Angeles"
GEOLOCATION = {
    "latitude": 37.7749,
    "longitude": -122.4194,
    "accuracy": 100,
}
CHROME_ARGS = [
    "--lang=en-US",
    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
]
SYSTEM_PROFILE_NAMES = {"System Profile", "Guest Profile"}


@dataclass(frozen=True)
class BrowserRoot:
    name: str
    user_data_dir: Path


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


def sync_local_state(path: Path) -> bool:
    data = load_json_mapping(path)
    intl = mapping_section(data, "intl")
    changed = set_mapping_value(intl, "app_locale", APP_LOCALE)
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

    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def automation_context_payload() -> dict:
    return {
        "browser_context": {
            "locale": APP_LOCALE,
            "timezone_id": TIMEZONE_ID,
            "geolocation": GEOLOCATION,
            "permissions": ["geolocation"],
            "extra_http_headers": {
                "Accept-Language": "en-US,en;q=0.9",
            },
        },
        "chrome_launch": {
            "args": CHROME_ARGS,
        },
        "environment": {
            "TZ": TIMEZONE_ID,
            "LANG": ENV_LOCALE,
            "LANGUAGE": "en_US:en",
            "LC_ALL": ENV_LOCALE,
        },
        "google": {
            "ncr_url": NCR_URL,
            "search_url": SEARCH_URL,
            "suggest_url": SUGGEST_URL,
        },
        "limits": [
            "This config cannot change proxy exit IP geolocation.",
            "Native Chrome timezone and geolocation need DevTools or automation context emulation.",
        ],
    }


def sync_automation_context(config_dir: Path) -> list[str]:
    config_path = config_dir / "browser-us-context.json"
    env_path = config_dir / "browser-us.env"
    payload = automation_context_payload()

    changed_paths = []
    previous_payload = load_json_mapping(config_path) if config_path.exists() else None
    if previous_payload != payload:
        ensure_backup(config_path)
        write_json_atomic(config_path, payload)
        changed_paths.append(str(config_path))

    env_content = "\n".join(
        [
            "# Generated by agent-manager scripts/sync_browser_google_us.py",
            f"export TZ={shlex.quote(TIMEZONE_ID)}",
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
        return json.loads(response.read().decode("utf-8"))


def country_from_key(key: str) -> Callable[[dict], str | None]:
    def parse(payload: dict) -> str | None:
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip().upper()
        return None

    return parse


COUNTRY_PROBES: list[tuple[str, Callable[[dict], str | None]]] = [
    ("https://ipinfo.io/json", country_from_key("country")),
    ("https://ipapi.co/json", country_from_key("country_code")),
    ("https://ipwho.is/", country_from_key("country_code")),
]


def detect_exit_country(timeout: float) -> tuple[str | None, str | None, list[str]]:
    errors = []
    for url, parser in COUNTRY_PROBES:
        try:
            payload = read_json_url(url, timeout)
        except (OSError, TimeoutError, urllib.error.URLError, ValueError, json.JSONDecodeError) as exc:
            errors.append(f"{url}: {exc}")
            continue
        country_code = parser(payload)
        if country_code:
            return country_code, url, errors
        errors.append(f"{url}: country code missing")
    return None, None, errors


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
    return parser


def print_dry_run(args: argparse.Namespace, browser_roots: list[BrowserRoot], config_dir: Path) -> None:
    print("Would sync Google US browser state:")
    for root in browser_roots:
        profile_label = args.profile or "all regular profiles"
        print(f"  {root.name}: {root.user_data_dir} ({profile_label})")
    print(f"Would write automation context under {config_dir}")
    print(f"Would set browser locale to {APP_LOCALE}, timezone to {TIMEZONE_ID}, and geolocation to US coordinates")
    print(f"Would set accepted languages to {ACCEPT_LANGUAGES}")
    print(f"Would set default Google search URL to {SEARCH_URL}")
    print("Would set WebRTC policy to disable_non_proxied_udp")
    if not args.offline and not args.skip_exit_check:
        print("Would verify public proxy exit country and fail if a non-US country is detected")
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


def verify_exit_country(args: argparse.Namespace) -> int:
    if args.offline or args.skip_exit_check:
        if args.offline:
            print("Offline mode: skipped public exit-country verification")
        else:
            print("Public exit-country verification skipped")
        return 0

    country_code, source, errors = detect_exit_country(args.exit_check_timeout)
    if country_code is None:
        print("WARN: could not verify public exit country; Google may still classify by proxy IP", file=sys.stderr)
        for error in errors[:3]:
            print(f"WARN: {error}", file=sys.stderr)
        return 0

    print(f"Detected public exit country: {country_code} ({source})")
    if country_code == "US":
        return 0

    message = f"Detected public exit country is {country_code}, not US"
    if args.allow_non_us_exit:
        print(f"WARN: {message}", file=sys.stderr)
        return 0
    print(f"ERROR: {message}", file=sys.stderr)
    return 2


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    platform_name = sys.platform
    config_dir = Path(args.config_dir).expanduser() if args.config_dir else args.home.expanduser() / ".config" / "agent-manager"
    browser_roots = resolve_browser_roots(args, platform_name)

    if args.dry_run:
        print_dry_run(args, browser_roots, config_dir)
        return 0

    changed_paths = []
    changed_paths.extend(sync_browser_roots(browser_roots, args.profile))
    changed_paths.extend(sync_automation_context(config_dir))

    exit_status = verify_exit_country(args)
    if exit_status != 0:
        return exit_status

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
