#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


ACCEPT_LANGUAGES = "en-US,en"
APP_LOCALE = "en-US"
NCR_URL = "https://www.google.com/ncr"
SEARCH_URL = "https://www.google.com/search?q={searchTerms}&hl=en-US&gl=US&pws=0"
SUGGEST_URL = "https://www.google.com/complete/search?client=chrome&q={searchTerms}&hl=en-US&gl=US"


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


def default_chrome_user_data_dir(home: Path, platform_name: str) -> Path:
    if platform_name == "darwin":
        return home / "Library" / "Application Support" / "Google" / "Chrome"
    if platform_name.startswith("win"):
        local_app_data = os.environ.get("LOCALAPPDATA")
        if local_app_data:
            return Path(local_app_data) / "Google" / "Chrome" / "User Data"
        return home / "AppData" / "Local" / "Google" / "Chrome" / "User Data"
    return home / ".config" / "google-chrome"


def selected_profile_name(user_data_dir: Path, explicit_profile: str | None) -> str:
    if explicit_profile:
        return explicit_profile

    local_state = load_json_mapping(user_data_dir / "Local State")
    profile_state = local_state.get("profile")
    if isinstance(profile_state, dict):
        last_used = profile_state.get("last_used")
        if isinstance(last_used, str) and last_used.strip():
            return last_used

    return "Default"


def ensure_backup(path: Path) -> None:
    if not path.exists():
        return
    backup_path = path.with_name(f"{path.name}.agent-manager-before-google-us")
    if not backup_path.exists():
        shutil.copy2(path, backup_path)


def sync_local_state(path: Path) -> bool:
    data = load_json_mapping(path)
    intl = data.setdefault("intl", {})
    if not isinstance(intl, dict):
        intl = {}
        data["intl"] = intl
    changed = intl.get("app_locale") != APP_LOCALE
    intl["app_locale"] = APP_LOCALE
    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


def sync_preferences(path: Path) -> bool:
    data = load_json_mapping(path)

    intl = data.setdefault("intl", {})
    if not isinstance(intl, dict):
        intl = {}
        data["intl"] = intl

    default_search_provider = data.setdefault("default_search_provider", {})
    if not isinstance(default_search_provider, dict):
        default_search_provider = {}
        data["default_search_provider"] = default_search_provider

    desired_provider = {
        "enabled": True,
        "keyword": "google.com",
        "name": "Google US",
        "search_url": SEARCH_URL,
        "suggest_url": SUGGEST_URL,
    }

    changed = intl.get("accept_languages") != ACCEPT_LANGUAGES
    intl["accept_languages"] = ACCEPT_LANGUAGES

    for key, value in desired_provider.items():
        if default_search_provider.get(key) != value:
            changed = True
        default_search_provider[key] = value

    if changed:
        ensure_backup(path)
        write_json_atomic(path, data)
    return changed


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


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Sync Chrome browser state for Google US regional preflight")
    parser.add_argument("--dry-run", action="store_true", help="print planned changes without writing or opening Chrome")
    parser.add_argument("--offline", action="store_true", help="skip browser navigation that would touch the network")
    parser.add_argument(
        "--home",
        type=Path,
        default=Path.home(),
        help="home directory used to resolve the default Chrome user data path",
    )
    parser.add_argument(
        "--chrome-user-data-dir",
        type=Path,
        default=os.environ.get("AGENT_MANAGER_CHROME_USER_DATA_DIR"),
        help="Chrome user data directory; defaults to the platform Chrome profile root",
    )
    parser.add_argument(
        "--profile",
        default=os.environ.get("AGENT_MANAGER_CHROME_PROFILE"),
        help="Chrome profile directory name, such as Default or Profile 1",
    )
    parser.add_argument(
        "--no-browser-open",
        action="store_true",
        default=os.environ.get("AGENT_MANAGER_BROWSER_US_NO_OPEN") == "1",
        help="do not open google.com/ncr after syncing local profile files",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    platform_name = sys.platform
    user_data_dir = args.chrome_user_data_dir or default_chrome_user_data_dir(args.home.expanduser(), platform_name)
    user_data_dir = Path(user_data_dir).expanduser()

    if args.dry_run:
        print(f"Would sync Chrome Google US browser state under {user_data_dir}")
        if args.profile:
            print(f"Would target Chrome profile {args.profile}")
        print(f"Would set app locale to {APP_LOCALE} and accepted languages to {ACCEPT_LANGUAGES}")
        print(f"Would set default Google search URL to {SEARCH_URL}")
        if not args.offline and not args.no_browser_open:
            print(f"Would open {NCR_URL}")
        return 0

    if not user_data_dir.exists():
        print(f"WARN: Chrome user data directory not found: {user_data_dir}", file=sys.stderr)
        return 0

    profile_name = selected_profile_name(user_data_dir, args.profile)
    local_state_path = user_data_dir / "Local State"
    preferences_path = user_data_dir / profile_name / "Preferences"

    changed_paths = []
    if sync_local_state(local_state_path):
        changed_paths.append(str(local_state_path))

    if preferences_path.exists():
        if sync_preferences(preferences_path):
            changed_paths.append(str(preferences_path))
    else:
        print(f"WARN: Chrome profile Preferences not found: {preferences_path}", file=sys.stderr)

    opened_ncr = False
    if args.offline:
        print("Offline mode: skipped google.com/ncr browser bootstrap")
    elif args.no_browser_open:
        print("Browser open disabled: skipped google.com/ncr browser bootstrap")
    else:
        opened_ncr = open_google_ncr(platform_name)

    if changed_paths:
        print("Synced Google US browser profile files:")
        for path in changed_paths:
            print(f"  {path}")
    else:
        print("Google US browser profile files already matched desired state")

    if opened_ncr:
        print(f"Opened {NCR_URL} to refresh no-country-redirect browser state")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
