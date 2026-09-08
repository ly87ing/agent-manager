"""Database operations for policy get/set/delete and operation auditing."""
from __future__ import annotations

import datetime
import json
import re
import sqlite3
import uuid
from pathlib import Path
from typing import Any

from src.policy_layers import (
    PolicyKeyNotFoundError,
    PolicySecretGateError,
    PolicySetError,
    _KEY_SEGMENT_RE,
    deep_merge,
)
from src.secret_scan import scan_text_for_secrets
from src.storage.db import connect_ready_db
from src.storage.projector import project_config_from_db


def _now_iso() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


# Dot-path prefixes that address definition rows (domain, required non-empty key).
# Mirrors policy_layers.DEFINITION_OVERLAY_RULES so validation is never weaker
# than the legacy apply_definition_overlays path.
_DEFINITION_PREFIXES = {
    ("agent_definitions",): ("agent", "bin"),
    ("tool_definitions",): ("tool", None),
    ("tools", "tool_definitions"): ("tool", None),
    ("mcp", "definitions"): ("mcp", "command"),
    ("skills", "repositories"): ("skill", "url"),
}


def _match_definition_prefix(segments):
    """Return (prefix, domain, required_key) if segments start with a definition prefix."""
    for prefix in sorted(_DEFINITION_PREFIXES, key=len, reverse=True):
        if tuple(segments[: len(prefix)]) == prefix:
            domain, required_key = _DEFINITION_PREFIXES[prefix]
            return prefix, domain, required_key
    return None


def _load_definition_row(conn, domain, entry_id, *, label):
    """Fetch and strictly parse an existing definition row (corrupt JSON is an error)."""
    row = conn.execute(
        "SELECT definition_json, deleted FROM definitions WHERE domain = ? AND id = ?",
        (domain, entry_id),
    ).fetchone()
    if row is None:
        return None
    try:
        doc = json.loads(row["definition_json"])
    except (ValueError, TypeError) as exc:
        raise PolicySetError(f"{label} 现有定义 JSON 损坏，拒绝继续: {exc}") from exc
    if not isinstance(doc, dict):
        raise PolicySetError(f"{label} 现有定义不是对象，拒绝继续")
    return {"doc": doc, "deleted": bool(row["deleted"])}


def _upsert_tombstone(conn, domain, entry_id, now_str):
    conn.execute(
        """
        INSERT INTO definitions (domain, id, definition_json, deleted, updated_at)
        VALUES (?, ?, '{}', 1, ?)
        ON CONFLICT(domain, id) DO UPDATE SET deleted = 1, updated_at = excluded.updated_at
        """,
        (domain, entry_id, now_str),
    )


def _write_definition_fragment(conn, domain, entry_id, fragment, required_key, now_str, *, label):
    """Merge (or _replace) a definition fragment into its definitions row.

    Whole-entry and nested sub-field writes land on the SAME row, so a sub-field
    write can never be masked by the definitions projection (no fake success).
    Raises before writing on invalid input; callers wrap in a transaction so a
    rejected value leaves no changes behind.
    """
    if fragment.get("_delete") is True:
        _upsert_tombstone(conn, domain, entry_id, now_str)
        return

    replace = fragment.get("_replace") is True
    body = {k: v for k, v in fragment.items() if k not in ("_replace", "_delete")}
    existing = None if replace else _load_definition_row(conn, domain, entry_id, label=label)
    if existing is not None:
        merged = existing["doc"]
        deep_merge(merged, body)
    else:
        merged = body

    if required_key is not None:
        req = merged.get(required_key)
        if not isinstance(req, str) or not req.strip():
            raise PolicySetError(f"{label} 缺少合法必填键 {required_key}（非空字符串）")

    raw_json = json.dumps(merged, ensure_ascii=False)
    conn.execute(
        """
        INSERT INTO definitions (domain, id, definition_json, deleted, updated_at)
        VALUES (?, ?, ?, 0, ?)
        ON CONFLICT(domain, id) DO UPDATE SET definition_json = excluded.definition_json, deleted = 0, updated_at = excluded.updated_at
        """,
        (domain, entry_id, raw_json, now_str),
    )


def record_operation(
    db_or_conn: Path | sqlite3.Connection,
    operation_type: str,
    target: str | None = None,
    detail: dict | None = None,
    run_id: str | None = None,
) -> str:
    """Record an audit trace of an operation in operation_logs."""
    op_id = str(uuid.uuid4())
    now_str = _now_iso()
    detail_str = json.dumps(detail, ensure_ascii=False) if detail else None

    if isinstance(db_or_conn, sqlite3.Connection):
        db_or_conn.execute(
            """
            INSERT INTO operation_logs (id, run_id, operation_type, target, detail_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (op_id, run_id, operation_type, target, detail_str, now_str),
        )
    else:
        conn = connect_ready_db(db_or_conn)
        try:
            with conn:
                conn.execute(
                    """
                    INSERT INTO operation_logs (id, run_id, operation_type, target, detail_json, created_at)
                    VALUES (?, ?, ?, ?, ?, ?)
                    """,
                    (op_id, run_id, operation_type, target, detail_str, now_str),
                )
        finally:
            conn.close()
    return op_id


def policy_get_db(db_path: Path) -> dict:
    """Return policy get payload sourced directly from manager.db.

    Keeps the three legacy layer keys (all null — the legacy files are no longer
    the source of truth) and adds the optional ``sqlite`` layer per
    contract/engine-v1/policy.schema.json.
    """
    effective = project_config_from_db(db_path)
    return {
        "layers": {
            "baseline": None,
            "synced": None,
            "local": None,
            "sqlite": effective,
        },
        "effective": effective,
        "attribution": {k: "sqlite" for k in effective},
        "layer": None,
        "key": None,
        "value": None,
        "path": str(db_path),
    }


def policy_set_db(
    db_path: Path,
    key: str,
    value: Any,
    *,
    run_id: str | None = None,
) -> Path:
    """Write a policy key/value directly into manager.db without modifying legacy files."""
    segments = key.split(".")
    if not segments or any(not _KEY_SEGMENT_RE.match(segment) for segment in segments):
        raise PolicySetError(f"非法 key 路径段（点号分隔，每段只允许字母/数字/_/-）: {key}")

    rendered = json.dumps(value, ensure_ascii=False)
    findings = scan_text_for_secrets(rendered, file_label="policy.set value")
    if findings:
        raise PolicySecretGateError(
            "疑似明文敏感信息（%s），拒绝写入数据库" % ", ".join(sorted({f.kind for f in findings}))
        )

    now_str = _now_iso()
    conn = connect_ready_db(db_path)
    try:
        with conn:
            matched = _match_definition_prefix(segments)
            if matched is not None:
                prefix, domain, required_key = matched
                rest = segments[len(prefix):]
                prefix_label = ".".join(prefix)
                if not rest:
                    raise PolicySetError(
                        f"暂不支持 {prefix_label} 整体集合写；请按 {prefix_label}.<id>[.<field>] 逐条写入"
                    )
                entry_id = rest[0]
                if len(rest) == 1:
                    if not isinstance(value, dict):
                        raise PolicySetError(f"{prefix_label}.{entry_id} 必须是对象")
                    fragment = value
                else:
                    fragment = {}
                    cursor = fragment
                    for seg in rest[1:-1]:
                        cursor[seg] = {}
                        cursor = cursor[seg]
                    cursor[rest[-1]] = value
                _write_definition_fragment(
                    conn, domain, entry_id, fragment, required_key, now_str,
                    label=f"{prefix_label}.{entry_id}",
                )
            else:
                # General policy entries (agents.*, tools.*, mcp.*, skills.*, custom keys)
                top = segments[0]
                sub = segments[1:]
                if not sub:
                    conn.execute(
                        """
                        INSERT INTO policy_entries (key, value_json, updated_at)
                        VALUES (?, ?, ?)
                        ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
                        """,
                        (top, json.dumps(value, ensure_ascii=False), now_str),
                    )
                else:
                    cur_row = conn.execute(
                        "SELECT value_json FROM policy_entries WHERE key = ?",
                        (top,),
                    ).fetchone()
                    doc = {}
                    if cur_row and cur_row["value_json"]:
                        try:
                            doc = json.loads(cur_row["value_json"])
                        except (ValueError, TypeError) as exc:
                            raise PolicySetError(
                                f"配置项 {top} 的 JSON 数据损坏，拒绝覆盖: {exc}"
                            ) from exc
                    if not isinstance(doc, dict):
                        raise PolicySetError(f"配置项 {top} 不是对象，拒绝写入子字段 {key}")

                    cursor = doc
                    for seg in sub[:-1]:
                        if seg not in cursor or not isinstance(cursor[seg], dict):
                            cursor[seg] = {}
                        cursor = cursor[seg]
                    cursor[sub[-1]] = value

                    conn.execute(
                        """
                        INSERT INTO policy_entries (key, value_json, updated_at)
                        VALUES (?, ?, ?)
                        ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
                        """,
                        (top, json.dumps(doc, ensure_ascii=False), now_str),
                    )

            record_operation(conn, "policy:set", target=key, detail={"value": value}, run_id=run_id)
    finally:
        conn.close()

    return db_path


def policy_delete_db(
    db_path: Path,
    key: str,
    *,
    run_id: str | None = None,
) -> Path:
    """Delete or tombstone a key in manager.db."""
    segments = key.split(".")
    if not segments or any(not _KEY_SEGMENT_RE.match(segment) for segment in segments):
        raise PolicySetError(f"非法 key 路径段: {key}")

    now_str = _now_iso()
    conn = connect_ready_db(db_path)

    try:
        with conn:
            matched = _match_definition_prefix(segments)
            if matched is not None:
                prefix, domain, required_key = matched
                rest = segments[len(prefix):]
                prefix_label = ".".join(prefix)
                if not rest:
                    raise PolicySetError(
                        f"暂不支持 {prefix_label} 整体集合删除；请按 {prefix_label}.<id>[.<field>] 逐条删除"
                    )
                entry_id = rest[0]
                if len(rest) == 1:
                    _upsert_tombstone(conn, domain, entry_id, now_str)
                else:
                    label = f"{prefix_label}.{entry_id}"
                    existing = _load_definition_row(conn, domain, entry_id, label=label)
                    if existing is None or existing["deleted"]:
                        raise PolicyKeyNotFoundError(f"键不存在: {key}")
                    doc = existing["doc"]
                    cursor = doc
                    for seg in rest[1:-1]:
                        if not isinstance(cursor, dict) or seg not in cursor:
                            raise PolicyKeyNotFoundError(f"键不存在: {key}")
                        cursor = cursor[seg]
                    if not isinstance(cursor, dict) or rest[-1] not in cursor:
                        raise PolicyKeyNotFoundError(f"键不存在: {key}")
                    del cursor[rest[-1]]
                    if required_key is not None:
                        req = doc.get(required_key)
                        if not isinstance(req, str) or not req.strip():
                            raise PolicySetError(
                                f"{label} 删除后缺少合法必填键 {required_key}（非空字符串），拒绝删除"
                            )
                    conn.execute(
                        """
                        UPDATE definitions SET definition_json = ?, updated_at = ?
                        WHERE domain = ? AND id = ?
                        """,
                        (json.dumps(doc, ensure_ascii=False), now_str, domain, entry_id),
                    )
            else:
                # General policy entries
                top = segments[0]
                sub = segments[1:]
                row = conn.execute(
                    "SELECT value_json FROM policy_entries WHERE key = ?",
                    (top,),
                ).fetchone()
                if not row:
                    raise PolicyKeyNotFoundError(f"键不存在: {key}")

                if not sub:
                    conn.execute("DELETE FROM policy_entries WHERE key = ?", (top,))
                else:
                    try:
                        doc = json.loads(row["value_json"])
                    except (ValueError, TypeError) as exc:
                        raise PolicySetError(
                            f"配置项 {top} 的 JSON 数据损坏，拒绝继续: {exc}"
                        ) from exc
                    if not isinstance(doc, dict):
                        raise PolicySetError(f"配置项 {top} 不是对象，无法删除子字段 {key}")
                    cursor = doc
                    for seg in sub[:-1]:
                        if not isinstance(cursor, dict) or seg not in cursor:
                            raise PolicyKeyNotFoundError(f"键不存在: {key}")
                        cursor = cursor[seg]
                    if not isinstance(cursor, dict) or sub[-1] not in cursor:
                        raise PolicyKeyNotFoundError(f"键不存在: {key}")
                    del cursor[sub[-1]]
                    conn.execute(
                        "UPDATE policy_entries SET value_json = ?, updated_at = ? WHERE key = ?",
                        (json.dumps(doc, ensure_ascii=False), now_str, top),
                    )

            record_operation(conn, "policy:delete", target=key, run_id=run_id)
    finally:
        conn.close()

    return db_path
