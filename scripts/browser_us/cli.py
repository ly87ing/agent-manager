"""Extracted from sync_browser_google_us.py: cli layer."""
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
from browser_us.automation import *  # noqa: F401,F403


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

