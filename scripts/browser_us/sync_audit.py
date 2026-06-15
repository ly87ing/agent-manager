"""Extracted from sync_browser_google_us.py: sync_audit layer."""
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

