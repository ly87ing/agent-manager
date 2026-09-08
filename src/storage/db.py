"""Database connectivity and schema definition for manager.db."""
from __future__ import annotations

import json
import os
import sqlite3
import sys
import urllib.parse
from pathlib import Path

SCHEMA_VERSION = 1


def get_app_identifier() -> str:
    """Read application identifier from tauri.conf.json or return official identifier."""
    tauri_conf = Path(__file__).resolve().parents[2] / "app" / "src-tauri" / "tauri.conf.json"
    if tauri_conf.is_file():
        try:
            doc = json.loads(tauri_conf.read_text(encoding="utf-8"))
            identifier = doc.get("identifier")
            if identifier and isinstance(identifier, str):
                return identifier
        except Exception:
            pass
    return "com.agentmanager.dev"


def get_data_dir() -> Path:
    """Resolve the unified application data directory (pure: never mkdir/chmod).

    Honors AGENT_MANAGER_DATA_DIR override; otherwise uses platform standard
    matching Tauri app.path().app_data_dir(). Directory creation and permission
    hardening happen only on write paths (connect_db / migrate_if_needed).
    """
    env_override = os.environ.get("AGENT_MANAGER_DATA_DIR")
    if env_override and env_override.strip():
        return Path(env_override).resolve()

    identifier = get_app_identifier()
    if sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support" / identifier
    elif sys.platform in ("win32", "cygwin"):
        appdata = os.environ.get("APPDATA")
        base = (Path(appdata) if appdata else Path.home() / "AppData" / "Roaming") / identifier
    else:
        xdg = os.environ.get("XDG_DATA_HOME")
        base = (Path(xdg) if xdg else Path.home() / ".local" / "share") / identifier

    return base.resolve()


def get_db_path(data_dir: Path | str | None = None) -> Path:
    """Resolve the path to manager.db (pure: never touches the filesystem)."""
    target_dir = Path(data_dir) if data_dir is not None else get_data_dir()
    return target_dir / "manager.db"


def is_sqlite_active(root_dir: Path | str | None = None) -> bool:
    """Determine whether SQLite storage is the primary state source.

    Active only when manager.db already exists at the resolved data directory.
    Environment variables alone never activate it: an uninitialized GUI run
    (AGENT_MANAGER_DATA_DIR set, no DB yet) must keep reading legacy config.
    """
    return get_db_path().is_file()


def enforce_db_permissions(db_path: Path) -> None:
    """Ensure database and companion files have 0600 permissions."""
    for p in (db_path, Path(f"{db_path}-wal"), Path(f"{db_path}-shm")):
        if p.is_file():
            try:
                os.chmod(p, 0o600)
            except OSError:
                pass


class DatabaseNotReadyError(RuntimeError):
    """Raised when an activated manager.db fails schema_version / completion checks."""


def ensure_db_ready(conn: sqlite3.Connection, db_path: Path) -> None:
    """Fail-close readiness gate for an existing manager.db.

    Every read/write entry point on an activated DB must pass this: the
    schema_meta table must exist, schema_version must be a supported integer,
    and the migration completion marker must be present. Anything else means
    corrupt/foreign/future data and is refused WITHOUT modifying the file.
    """
    table_check = conn.execute(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'"
    ).fetchone()
    if not table_check or table_check[0] == 0:
        raise DatabaseNotReadyError(
            f"数据库缺少 schema_meta 表 ({db_path})，拒绝读写以保护原数据"
        )

    ver_row = conn.execute(
        "SELECT value FROM schema_meta WHERE key = 'schema_version'"
    ).fetchone()
    if not ver_row:
        raise DatabaseNotReadyError(
            f"数据库缺少 schema_version 记录 ({db_path})，拒绝读写以保护原数据"
        )
    try:
        ver = int(ver_row["value"])
    except (TypeError, ValueError):
        raise DatabaseNotReadyError(
            f"数据库 schema_version 非法 ({ver_row['value']})，拒绝读写以保护原数据"
        ) from None
    if ver > SCHEMA_VERSION:
        raise DatabaseNotReadyError(
            f"数据库版本 ({ver}) 高于当前程序支持版本 ({SCHEMA_VERSION})，拒绝访问"
        )

    done_row = conn.execute(
        "SELECT value FROM schema_meta WHERE key = 'migration_completed_at'"
    ).fetchone()
    if not done_row or not done_row["value"]:
        raise DatabaseNotReadyError(
            f"数据库缺少 migration_completed_at 标记 ({db_path})，拒绝读写以保护原数据"
        )


def connect_db(db_path: Path, *, read_only: bool = False, timeout: float = 30.0) -> sqlite3.Connection:
    """Open a robust SQLite connection with WAL mode and foreign key enforcement."""
    db_path = Path(db_path)
    if not read_only:
        db_path.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.chmod(db_path.parent, 0o700)
        except OSError:
            pass

    # Percent-encode the path so characters like '?', '#', '%' or quotes cannot
    # corrupt the URI (never interpolate a raw path into a SQLite URI).
    uri_path = urllib.parse.quote(str(db_path.resolve()), safe="/:")
    uri = f"file:{uri_path}?mode=ro" if read_only else f"file:{uri_path}"
    conn = sqlite3.connect(uri, uri=True, timeout=timeout)
    conn.row_factory = sqlite3.Row
    if not read_only:
        conn.execute("PRAGMA journal_mode = WAL;")
        conn.execute("PRAGMA busy_timeout = 30000;")
        conn.execute("PRAGMA synchronous = NORMAL;")
        enforce_db_permissions(db_path)
    conn.execute("PRAGMA foreign_keys = ON;")
    return conn


def connect_ready_db(db_path: Path, *, read_only: bool = False, timeout: float = 30.0) -> sqlite3.Connection:
    """Connect to an activated manager.db, enforcing ensure_db_ready first.

    Readiness is always verified on a read-only connection, so a rejected
    (corrupt/foreign/future-version) database is never modified — no journal
    or WAL writes happen before the gate passes. Migration internals use plain
    connect_db to build the temporary database explicitly.
    """
    db_path = Path(db_path)
    gate = connect_db(db_path, read_only=True, timeout=timeout)
    try:
        ensure_db_ready(gate, db_path)
    except Exception:
        gate.close()
        raise
    if read_only:
        return gate
    gate.close()
    return connect_db(db_path, read_only=False, timeout=timeout)


def init_schema(conn: sqlite3.Connection) -> None:
    """Initialize tables in manager.db with zero-loss policy and definition representations."""
    with conn:
        conn.executescript("""
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Loss-free policy configurations (effective combined policy per section or entry)
        CREATE TABLE IF NOT EXISTS policy_entries (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Domain definitions and local overlays / tombstones
        CREATE TABLE IF NOT EXISTS definitions (
            domain TEXT NOT NULL,
            id TEXT NOT NULL,
            definition_json TEXT NOT NULL,
            deleted INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (domain, id)
        );

        -- Editable sources (hooks, aliases, rules)
        CREATE TABLE IF NOT EXISTS sources (
            key TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            format TEXT NOT NULL,
            content TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Operation audit logs
        CREATE TABLE IF NOT EXISTS operation_logs (
            id TEXT PRIMARY KEY,
            run_id TEXT,
            operation_type TEXT NOT NULL,
            target TEXT,
            detail_json TEXT,
            created_at TEXT NOT NULL
        );
        """)
