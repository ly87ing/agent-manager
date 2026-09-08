"""Atomic migration from legacy policy layers and definitions to manager.db."""
from __future__ import annotations

import contextlib
import copy
import datetime
import json
import os
import sqlite3
import sys
import uuid
from pathlib import Path

from src.agent_ids import canonicalize_agent_mapping
from src.json_config import load_json_object_file
from src.policy_layers import (
    apply_definition_overlays,
    deep_merge,
    overlay_definitions,
)
from src.secret_scan import scan_text_for_secrets
from src.storage.db import (
    SCHEMA_VERSION,
    DatabaseNotReadyError,
    connect_db,
    enforce_db_permissions,
    ensure_db_ready,
    get_db_path,
    init_schema,
)


class MigrationError(RuntimeError):
    """Raised when migration from legacy configuration fails or target DB is corrupt."""


def _now_iso() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


@contextlib.contextmanager
def _migration_lock(lock_file_path: Path):
    """Acquire a cross-platform exclusive file lock.

    Uses fcntl on POSIX systems (lazy imported to prevent Windows ImportError)
    and msvcrt on Windows.
    """
    lock_f = open(lock_file_path, "a")
    try:
        if sys.platform == "win32":
            try:
                import msvcrt
                lock_f.seek(0)
                msvcrt.locking(lock_f.fileno(), msvcrt.LK_LOCK, 1)
                try:
                    yield
                finally:
                    try:
                        lock_f.seek(0)
                        msvcrt.locking(lock_f.fileno(), msvcrt.LK_UNLCK, 1)
                    except OSError:
                        pass
            except (ImportError, OSError) as exc:
                raise MigrationError(f"Windows 文件锁获取失败: {exc}") from exc
        else:
            try:
                import fcntl
                fcntl.flock(lock_f.fileno(), fcntl.LOCK_EX)
                try:
                    yield
                finally:
                    try:
                        fcntl.flock(lock_f.fileno(), fcntl.LOCK_UN)
                    except OSError:
                        pass
            except (ImportError, OSError) as exc:
                raise MigrationError(f"文件锁获取失败: {exc}") from exc
    finally:
        lock_f.close()


def _write_domain_definitions(tmp_conn, domain, base_defs, merged_policy, now_str):
    """Write one definition domain into the definitions table.

    Overlays are extracted from the fully merged policy (baseline manager.json
    included, then synced/local) via the shared overlay_definitions helper and
    validated/merged by apply_definition_overlays — the same fail-close rules as
    the legacy file path. Tombstones (_delete: true) are preserved as deleted
    rows so repeated startups never resurrect them.
    """
    overlay = overlay_definitions(merged_policy, domain)
    db_domain = {"agents": "agent", "tools": "tool", "mcp": "mcp", "skills": "skill"}[domain]
    tombstoned = {
        name
        for name, frag in overlay.items()
        if isinstance(frag, dict) and frag.get("_delete") is True
    }
    try:
        merged, _warnings = apply_definition_overlays(base_defs, overlay, domain=domain)
    except ValueError as exc:
        raise MigrationError(f"{domain} 定义覆盖层非法，拒绝迁移: {exc}") from exc

    for name in sorted(set(base_defs) | set(overlay)):
        is_deleted = name in tombstoned
        defn = {} if is_deleted else merged.get(name, {})
        raw_json = json.dumps(defn, ensure_ascii=False)
        if scan_text_for_secrets(raw_json, file_label=f"{domain} definition {name}"):
            raise MigrationError(f"{domain} 定义 {name} 包含疑似敏感信息，拒绝迁移")
        tmp_conn.execute(
            """
            INSERT INTO definitions (domain, id, definition_json, deleted, updated_at)
            VALUES (?, ?, ?, ?, ?)
            """,
            (db_domain, name, raw_json, 1 if is_deleted else 0, now_str),
        )
    return len(set(base_defs) | set(overlay))


def _check_existing_db(path: Path) -> Path | None:
    """Check if path is an existing, valid SQLite manager.db.

    If valid and already migrated, returns path.
    If file exists but is corrupted, empty, or lacks migration completion marker,
    raises MigrationError immediately to protect existing data from being overwritten.
    If file does not exist, returns None.
    """
    if not path.exists():
        return None

    if not path.is_file() or path.stat().st_size == 0:
        raise MigrationError(
            f"已有数据库文件存在但损坏或为空 ({path})，拒绝覆盖以保护原数据"
        )

    try:
        conn = connect_db(path, read_only=True)
        try:
            check = conn.execute("PRAGMA quick_check;").fetchone()
            if not check or check[0] != "ok":
                raise MigrationError(
                    f"已有数据库文件存在但完整性校验失败 ({path})，拒绝覆盖以保护原数据"
                )
            ensure_db_ready(conn, path)
        finally:
            conn.close()
    except MigrationError:
        raise
    except DatabaseNotReadyError as exc:
        raise MigrationError(str(exc)) from exc
    except Exception as exc:
        raise MigrationError(
            f"已有数据库文件存在但损坏或读取失败 ({exc})，拒绝覆盖以保护原数据"
        ) from exc
    return path


def migrate_if_needed(
    root_dir: Path | str,
    data_dir: Path | None = None,
    db_path: Path | None = None,
) -> Path:
    """Ensure manager.db is initialized and migrated from existing configuration.

    - Uses file locking with double-checked locking inside the lock to avoid race conditions.
    - If manager.db already exists with a valid migration completion record, this is
      a no-op (never re-imports and never resurrects tombstones).
    - If manager.db exists but is corrupted, empty, or missing migration markers,
      raises MigrationError and leaves the original file completely untouched.
    - If manager.db does not exist, an atomic migration is performed via a temporary
      file, ensuring a failed migration never leaves a partial database.
    - All policies (auto_install, auto_update, preferred_provider, per-skill agents)
      are preserved 100% loss-free in policy_entries.
    """
    root_dir = Path(root_dir).resolve()
    target_db = Path(db_path) if db_path else get_db_path(data_dir)

    target_db.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(target_db.parent, 0o700)
    except OSError:
        pass

    lock_file_path = target_db.parent / f"{target_db.name}.migration.lock"
    with _migration_lock(lock_file_path):
        existing = _check_existing_db(target_db)
        if existing is not None:
            return existing

        tmp_db = target_db.parent / f"{target_db.name}.tmp.{os.getpid()}_{uuid.uuid4().hex[:8]}"
        tmp_conn = None

        try:
            tmp_conn = connect_db(tmp_db)
            init_schema(tmp_conn)

            # 1. Load baseline configs
            base_mgr_path = Path(os.environ.get("AGENT_MANAGER_BASE_CONFIG", root_dir / "config" / "manager.json"))
            base_mgr = load_json_object_file(base_mgr_path, strict=True, missing_ok=True) if base_mgr_path.is_file() else {}

            base_agents_path = root_dir / "config" / "agents.json"
            base_agents = load_json_object_file(base_agents_path, strict=True, missing_ok=True) if base_agents_path.is_file() else {}

            base_tools_path = root_dir / "config" / "tools.json"
            base_tools_doc = load_json_object_file(base_tools_path, strict=True, missing_ok=True) if base_tools_path.is_file() else {}
            base_tools = base_tools_doc.get("tools", {}) if isinstance(base_tools_doc, dict) else {}

            base_mcp_path = root_dir / "config" / "mcp.json"
            base_mcp_doc = load_json_object_file(base_mcp_path, strict=True, missing_ok=True) if base_mcp_path.is_file() else {}
            base_mcp_servers = base_mcp_doc.get("servers", {}) if isinstance(base_mcp_doc, dict) else {}

            base_skills_path = root_dir / "config" / "skills.json"
            base_skills_doc = load_json_object_file(base_skills_path, strict=True, missing_ok=True) if base_skills_path.is_file() else {}
            base_skills_repos = base_skills_doc.get("repositories", {}) if isinstance(base_skills_doc, dict) else {}

            # 2. Load optional synced and local layers
            # 诊断文案与旧三层合并分支保持一致（test_manage 契约），不吞掉具体层错误。
            synced_path = Path(os.environ.get("AGENT_MANAGER_SYNCED_CONFIG", root_dir / "config" / "manager.synced.json"))
            try:
                synced_layer = load_json_object_file(synced_path, strict=True, missing_ok=True) if synced_path.is_file() else {}
            except ValueError as exc:
                raise MigrationError(f"共享层配置无效: {exc}") from exc

            local_path = Path(os.environ.get("AGENT_MANAGER_LOCAL_CONFIG", root_dir / "config" / "manager.local.json"))
            try:
                local_layer = load_json_object_file(local_path, strict=True, missing_ok=True) if local_path.is_file() else {}
            except ValueError as exc:
                raise MigrationError(f"本地覆盖配置无效: {exc}") from exc

            # Combined effective policy merge
            merged_policy = copy.deepcopy(base_mgr)
            if synced_layer:
                deep_merge(merged_policy, synced_layer)
            if local_layer:
                deep_merge(merged_policy, local_layer)
            if "agents" in merged_policy and isinstance(merged_policy["agents"], dict):
                merged_policy["agents"] = canonicalize_agent_mapping(merged_policy["agents"])

            now_str = _now_iso()

            # 3-6. Definitions: extract overlays from the fully merged policy
            # (baseline manager.json included) via the shared overlay helpers —
            # same merge/validation semantics as the legacy file path,
            # tombstones preserved as deleted rows.
            agents_count = _write_domain_definitions(tmp_conn, "agents", base_agents, merged_policy, now_str)
            tools_count = _write_domain_definitions(tmp_conn, "tools", base_tools, merged_policy, now_str)
            mcp_count = _write_domain_definitions(tmp_conn, "mcp", base_mcp_servers, merged_policy, now_str)
            skills_count = _write_domain_definitions(tmp_conn, "skills", base_skills_repos, merged_policy, now_str)

            # 7. Policy entries: 100% loss-free effective policies
            # Note: source_edits remains in policy_dict as relative paths!
            policy_dict = copy.deepcopy(merged_policy)
            policy_dict.pop("agent_definitions", None)
            policy_dict.pop("tool_definitions", None)
            if "mcp" in policy_dict and isinstance(policy_dict["mcp"], dict):
                policy_dict["mcp"].pop("definitions", None)
            if "skills" in policy_dict and isinstance(policy_dict["skills"], dict):
                policy_dict["skills"].pop("repositories", None)

            for k, v in policy_dict.items():
                val_json = json.dumps(v, ensure_ascii=False)
                if scan_text_for_secrets(val_json, file_label=f"policy key {k}"):
                    raise MigrationError(f"配置键 {k} 包含疑似敏感信息，拒绝迁移")
                tmp_conn.execute(
                    """
                    INSERT INTO policy_entries (key, value_json, updated_at)
                    VALUES (?, ?, ?)
                    """,
                    (k, val_json, now_str),
                )

            # 8. Write schema_meta
            summary = {
                "schema_version": SCHEMA_VERSION,
                "timestamp": now_str,
                "imported_from": {
                    "baseline": str(base_mgr_path),
                    "synced": str(synced_path) if synced_layer else None,
                    "local": str(local_path) if local_layer else None,
                },
                "agents_count": agents_count,
                "tools_count": tools_count,
                "mcp_count": mcp_count,
                "skills_count": skills_count,
            }
            tmp_conn.execute("INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?)", (str(SCHEMA_VERSION),))
            tmp_conn.execute("INSERT INTO schema_meta (key, value) VALUES ('migration_completed_at', ?)", (now_str,))
            tmp_conn.execute("INSERT INTO schema_meta (key, value) VALUES ('migration_summary', ?)", (json.dumps(summary),))

            # 9. Commit, checkpoint, and switch journal mode to DELETE so the temporary DB
            # file is completely self-contained before atomic swap.
            # In macOS system SQLite (3.43.2) and others, a DB created in WAL mode leaves
            # companion -wal/-shm files after close; renaming only the main file leaves
            # it dependent on a missing WAL file when subsequent read-only connections
            # (mode=ro) open it. Switching to DELETE journal mode flushes and truncates
            # WAL back into the main database file.
            tmp_conn.commit()

            # Ensure checkpoint succeeds with zero busy status
            cp = tmp_conn.execute("PRAGMA wal_checkpoint(TRUNCATE);").fetchone()
            if not cp or cp[0] != 0:
                raise MigrationError(f"WAL checkpoint 未能完成 (busy={cp[0] if cp else 'unknown'})")

            # Ensure journal_mode switch actually transitions to delete
            jm = tmp_conn.execute("PRAGMA journal_mode = DELETE;").fetchone()
            if not jm or str(jm[0]).lower() != "delete":
                raise MigrationError(f"未能将数据库切回 DELETE 日志模式 (当前为 {jm[0] if jm else 'unknown'})")

            check = tmp_conn.execute("PRAGMA integrity_check;").fetchall()
            if not check or check[0][0] != "ok":
                raise MigrationError("数据库完整性校验失败")

            tmp_conn.close()
            tmp_conn = None

            # Self-contained verification: ensure the published file can be opened
            # read-only without companion WAL files before renaming.
            verify_conn = connect_db(tmp_db, read_only=True)
            try:
                ensure_db_ready(verify_conn, tmp_db)
            finally:
                verify_conn.close()

            # Clean up any leftover temporary companion files (-wal, -shm)
            for stray in (Path(f"{tmp_db}-wal"), Path(f"{tmp_db}-shm")):
                if stray.is_file():
                    try:
                        stray.unlink()
                    except OSError:
                        pass

            enforce_db_permissions(tmp_db)

            # 10. Atomic swap
            os.replace(tmp_db, target_db)
            enforce_db_permissions(target_db)
            return target_db

        except Exception as exc:
            if tmp_conn:
                try:
                    tmp_conn.close()
                except Exception:
                    pass
            for stray in (tmp_db, Path(f"{tmp_db}-wal"), Path(f"{tmp_db}-shm")):
                if stray.is_file():
                    try:
                        stray.unlink()
                    except Exception:
                        pass
            if isinstance(exc, MigrationError):
                raise
            raise MigrationError(f"迁移初始化失败: {exc}") from exc
