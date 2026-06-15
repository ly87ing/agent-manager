"""Extracted from sync_browser_google_us.py: chrome layer."""
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


def navigator_app_version_from_user_agent(user_agent: str) -> str:
    prefix = "Mozilla/"
    if user_agent.startswith(prefix):
        return user_agent[len(prefix) :]
    return user_agent


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
            "network_information": dict(US_DESKTOP_NETWORK_INFORMATION),
            "webgl_vendor": US_DESKTOP_WEBGL_VENDOR,
            "webgl_renderer": US_DESKTOP_WEBGL_RENDERER,
            "blocked_font_families": list(BLOCKED_FONT_FAMILIES),
            "font_metric_fallback_family": US_CANVAS_FONT_METRIC_FALLBACK_FAMILY,
            "keyboard_layout_map": dict(US_KEYBOARD_LAYOUT_MAP),
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

