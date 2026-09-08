"""Projection of SQLite manager.db state into non-sensitive runtime JSON."""
from __future__ import annotations

import json
from pathlib import Path

from src.json_config import atomic_write_json_file
from src.storage.db import connect_ready_db


def project_config_from_db(db_path: Path) -> dict:
    """Reconstruct the merged effective configuration from manager.db.

    Transitionally projected to a non-sensitive JSON file so existing Python adapters
    (sync_mcp, link_skills, manage_doctor, etc.) can operate safely without treating
    the projection as a second editable state.
    """
    db_path = Path(db_path)
    conn = connect_ready_db(db_path, read_only=True)
    try:
        # One explicit read transaction: all SELECTs below see a single
        # consistent snapshot even if another process writes mid-projection.
        conn.execute("BEGIN")
        projected = {}
        # 1. Load all policy_entries (loss-free effective policies)
        for row in conn.execute("SELECT key, value_json FROM policy_entries").fetchall():
            try:
                projected[row["key"]] = json.loads(row["value_json"])
            except Exception as exc:
                raise RuntimeError(f"配置项 {row['key']} 的 JSON 数据损坏，拒绝输出半份配置: {exc}") from exc

        # 2. Reconstruct definition overlays with _replace: True to prevent resurrecting deleted baseline fields
        # Agents
        agent_defs = {}
        for row in conn.execute("SELECT id, definition_json, deleted FROM definitions WHERE domain = 'agent'").fetchall():
            aid = row["id"]
            if row["deleted"]:
                agent_defs[aid] = {"_delete": True}
            else:
                try:
                    defn = json.loads(row["definition_json"])
                    defn["_replace"] = True
                    agent_defs[aid] = defn
                except Exception as exc:
                    raise RuntimeError(f"Agent 定义 {aid} 的 JSON 数据损坏，拒绝输出半份配置: {exc}") from exc
        if agent_defs:
            projected["agent_definitions"] = agent_defs

        # Tools
        tool_defs = {}
        for row in conn.execute("SELECT id, definition_json, deleted FROM definitions WHERE domain = 'tool'").fetchall():
            tid = row["id"]
            if row["deleted"]:
                tool_defs[tid] = {"_delete": True}
            else:
                try:
                    defn = json.loads(row["definition_json"])
                    defn["_replace"] = True
                    tool_defs[tid] = defn
                except Exception as exc:
                    raise RuntimeError(f"Tool 定义 {tid} 的 JSON 数据损坏，拒绝输出半份配置: {exc}") from exc
        if tool_defs:
            projected["tool_definitions"] = tool_defs

        # MCP definitions
        mcp_defs = {}
        for row in conn.execute("SELECT id, definition_json, deleted FROM definitions WHERE domain = 'mcp'").fetchall():
            sid = row["id"]
            if row["deleted"]:
                mcp_defs[sid] = {"_delete": True}
            else:
                try:
                    defn = json.loads(row["definition_json"])
                    defn["_replace"] = True
                    mcp_defs[sid] = defn
                except Exception as exc:
                    raise RuntimeError(f"MCP 服务定义 {sid} 的 JSON 数据损坏，拒绝输出半份配置: {exc}") from exc
        if mcp_defs:
            projected.setdefault("mcp", {})["definitions"] = mcp_defs

        # Skill repositories
        skill_repos = {}
        for row in conn.execute("SELECT id, definition_json, deleted FROM definitions WHERE domain = 'skill'").fetchall():
            kid = row["id"]
            if row["deleted"]:
                skill_repos[kid] = {"_delete": True}
            else:
                try:
                    defn = json.loads(row["definition_json"])
                    defn["_replace"] = True
                    skill_repos[kid] = defn
                except Exception as exc:
                    raise RuntimeError(f"Skill 仓库定义 {kid} 的 JSON 数据损坏，拒绝输出半份配置: {exc}") from exc
        if skill_repos:
            projected.setdefault("skills", {})["repositories"] = skill_repos

        return projected
    finally:
        conn.close()


def write_projected_config(db_path: Path, output_path: Path | str) -> Path:
    """Project DB state and write atomically to output_path."""
    projected = project_config_from_db(db_path)
    target = Path(output_path)
    atomic_write_json_file(target, projected, chmod=0o600)
    return target
