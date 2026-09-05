"""Disk cache for upstream probe results (docs/GUI-ARCHITECTURE-PROPOSAL.md section 3.3).

The cache is the only thing '<domain>:status' commands read for the freshness
axis — status stays zero-network; 'system:refresh-upstream' is the only writer.

File layout (single JSON document, atomically rewritten):

    {
      "version": 1,
      "entries": {
        "<element-id>": {
          "state": "current|behind|ahead|unknown",
          "current": "<local version/commit or null>",
          "upstream": "<upstream latest or null>",
          "checked_at": "<ISO-8601 UTC, Z-suffixed>",
          "probe": "npm-view|git-ls-remote|pypi-json|brew-outdated|...",
          "error": "<human-readable probe failure; only present on unknown>"
        }
      }
    }

Path resolution: $AGENT_MANAGER_UPSTREAM_CACHE > $TMPDIR/agent_manager_upstream_<user>.json
(same /tmp state-dir precedent as agent-lifecycle's async state). TTL defaults
to 24h and is tunable via $AGENT_MANAGER_UPSTREAM_TTL_SEC. A stale entry is
still returned — the engine surfaces it as last-known plus cache_age_sec and a
prompt-level warning; it never re-probes implicitly.
"""
from __future__ import annotations

import json
import os
import tempfile
from datetime import datetime, timezone
from getpass import getuser
from pathlib import Path

from src.json_config import atomic_write_json_file

CACHE_ENV = "AGENT_MANAGER_UPSTREAM_CACHE"
TTL_ENV = "AGENT_MANAGER_UPSTREAM_TTL_SEC"
DEFAULT_TTL_SEC = 24 * 3600
CACHE_VERSION = 1

ISO_FORMAT = "%Y-%m-%dT%H:%M:%SZ"


def cache_path():
    configured = os.environ.get(CACHE_ENV)
    if configured:
        return Path(configured).expanduser()
    return Path(tempfile.gettempdir()) / ("agent_manager_upstream_%s.json" % getuser())


def ttl_sec():
    raw = os.environ.get(TTL_ENV, "").strip()
    if raw:
        try:
            return max(0, int(raw))
        except ValueError:
            pass
    return DEFAULT_TTL_SEC


def now_utc():
    return datetime.now(timezone.utc)


def to_iso(moment):
    return moment.strftime(ISO_FORMAT)


def parse_iso(value):
    if not isinstance(value, str) or not value.strip():
        return None
    try:
        return datetime.strptime(value.strip(), ISO_FORMAT).replace(tzinfo=timezone.utc)
    except ValueError:
        return None


def load_cache(path=None):
    """Read the cache; a missing or corrupt file yields an empty cache."""
    path = Path(path) if path is not None else cache_path()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {"version": CACHE_VERSION, "entries": {}}
    if not isinstance(data, dict) or not isinstance(data.get("entries"), dict):
        return {"version": CACHE_VERSION, "entries": {}}
    return {"version": CACHE_VERSION, "entries": data["entries"]}


def write_entries(updates, path=None):
    """Merge ``updates`` into the cache and atomically rewrite the file."""
    path = Path(path) if path is not None else cache_path()
    cache = load_cache(path)
    cache["entries"].update(updates)
    atomic_write_json_file(path, cache, chmod=0o600)
    return cache


def get_entry(cache, element_id):
    entry = cache.get("entries", {}).get(element_id)
    return entry if isinstance(entry, dict) else None


def cache_age_sec(entry, now=None):
    """Age of the cached probe in seconds, or None when there is no timestamp."""
    checked_at = parse_iso(entry.get("checked_at"))
    if checked_at is None:
        return None
    now = now or now_utc()
    return max(0, int((now - checked_at).total_seconds()))


def is_stale(entry, now=None):
    """True when the entry is older than the TTL or its timestamp is unreadable."""
    age = cache_age_sec(entry, now)
    if age is None:
        return True
    return age > ttl_sec()
