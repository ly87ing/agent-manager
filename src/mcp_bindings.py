"""MCP 不可变版本绑定与逐助手分配隔离模块。

在 SQLite 激活后：
1. 复用 policy_entries 表，存储顶层 key 为 'mcp_bindings' 的 JSON。
   键结构：server -> agent -> {version, definition, enabled, assigned, applied_digest}。
   - version: 未注入凭证的定义规范 JSON 摘要 (SHA256)。
   - definition: 保存未注入凭证引用的所选定义 (保留 ${VAR} 变量引用原样，绝不明文落库)。
   - enabled: 该 Agent 是否启用该服务。
   - assigned: 该 Agent 是否分配该服务 (assigned=False 为墓碑)。
   - applied_digest: 配置文件中实际写入内容的规范摘要 (只保存实际字段摘要，不保存凭证明文，用于识别外部变更)。
2. 通过 connect_ready_db + BEGIN IMMEDIATE 事务读写并调用 record_operation 审计。
3. 提供单 Agent 物理路径隔离检查、外部同名项防覆盖保护、原子文件写与回读验证、
   以及 DB 失败时的内存回滚能力。
"""

from __future__ import annotations

import copy
from datetime import datetime, timezone
import hashlib
import json
import logging
import ntpath
import os
from pathlib import Path
import re
import shutil
import stat
import tempfile
from typing import Any, Callable

from src.storage.db import connect_ready_db, get_data_dir, get_db_path, is_sqlite_active
from src.storage.operations import record_operation
from src.toml_compat import tomllib
from src.secret_scan import scan_text_for_secrets

MCP_BINDINGS_POLICY_KEY = "mcp_bindings"
MANAGED_BY_KEY = "_MANAGED_BY"
MANAGED_BY_VALUE = "agent-manager"

TOML_BARE_KEY_RE = re.compile(r"^[A-Za-z0-9_-]+$")
TOML_SERVER_HEADER_RE = re.compile(
    r'^\s*\[\s*mcp_servers\.(?P<key>"(?:\\.|[^"\\])*"|[A-Za-z0-9_-]+)',
    re.MULTILINE,
)
TOML_SERVER_INT_KEYS = ("startup_timeout_sec",)


class _UnsetSentinel:
    def __repr__(self) -> str:
        return "<UNSET>"


UNSET = _UnsetSentinel()


class McpBindingError(ValueError):
    """MCP 绑定与分配相关基础错误。"""


class McpConflictError(McpBindingError):
    """目标配置存在外部非受管内容或外部修改冲突。"""


class McpBindingConflictError(McpBindingError):
    """MCP 分配存在并发事务修改冲突。"""


class McpSharedTargetError(McpBindingError):
    """目标配置文件与其他 Agent 共享，无法实现隔离。"""


class McpScopeError(McpBindingError):
    """作用域、Agent 或操作类型非法。"""


def validate_identifier(name: str, identifier_type: str = "标识符") -> None:
    """校验 ID 或名称，拒绝空值、路径穿越或非法字符。"""
    if not name or not isinstance(name, str):
        raise McpBindingError(f"{identifier_type} 不能为空")
    if name in (".", "..") or "/" in name or "\\" in name or "\x00" in name:
        raise McpBindingError(f"{identifier_type} 包含非法字符: {name!r}")


def _is_windows_path(path_str: str) -> bool:
    if len(path_str) >= 2 and path_str[0].isalpha() and path_str[1] == ":":
        return True
    if path_str.startswith(("\\\\", "//")):
        return True
    return False


def normalize_windows_device_path(path: Any) -> str:
    path_str = os.fspath(path)
    prefix_mappings = (
        ("\\\\?\\UNC\\", "\\\\"),
        ("\\??\\UNC\\", "\\\\"),
        ("\\\\?\\", ""),
        ("\\??\\", ""),
    )
    for prefix, replacement in prefix_mappings:
        if path_str.startswith(prefix):
            return replacement + path_str[len(prefix):]
    return path_str


def normalize_path_for_compare(path: Any) -> tuple[str, str]:
    path_str = normalize_windows_device_path(path)
    if _is_windows_path(path_str):
        return "windows", ntpath.normcase(ntpath.normpath(path_str))
    return "posix", str(Path(path_str).expanduser().resolve(strict=False))


def check_shared_agent_target(agent_name: str, agent_targets: dict[str, Any]) -> None:
    """检查指定 Agent 的配置文件是否与其他 Agent 共享。若是则拒绝单 Agent 动作。"""
    if agent_name not in agent_targets:
        return
    raw_meta = agent_targets[agent_name]
    target_path = raw_meta if isinstance(raw_meta, str) else raw_meta.get("path")
    if not target_path:
        return
    target_norm = normalize_path_for_compare(target_path)
    for other_agent, other_meta in agent_targets.items():
        if other_agent == agent_name:
            continue
        other_path = other_meta if isinstance(other_meta, str) else other_meta.get("path")
        if not other_path:
            continue
        if normalize_path_for_compare(other_path) == target_norm:
            raise McpSharedTargetError(
                f"Agent '{agent_name}' 的配置文件与 Agent '{other_agent}' 共享 ({target_path})，"
                f"拒绝单 Agent 操作以保证隔离"
            )


def clean_definition(definition: dict) -> dict:
    """清理内部临时下划线属性以及 policy 专属键 (enabled, agents)，得到规范的定义字典。"""
    EXCLUDED = {"enabled", "agents"}
    return {
        k: copy.deepcopy(v)
        for k, v in definition.items()
        if not k.startswith("_") and k not in EXCLUDED
    }


def validate_un_injected_definition(definition: dict, server_name: str = "") -> dict:
    """验证定义不含展开的明文敏感机密，只保存未注入引用，绝不明文落库。"""
    clean = clean_definition(definition)
    serialized = json.dumps(clean, ensure_ascii=False)
    findings = scan_text_for_secrets(serialized, file_label=f"mcp definition {server_name}".strip())
    if findings:
        raise McpBindingError(
            f"MCP server '{server_name}' 定义包含敏感机密凭据，拒绝落库: {findings}"
        )
    return clean


def compute_definition_version(definition: dict) -> str:
    """计算未注入凭证的定义规范 JSON 的 SHA-256 摘要。"""
    clean = clean_definition(definition)
    serialized = json.dumps(clean, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(serialized).hexdigest()


def compute_applied_digest(
    command: Any,
    args: list[str] | None,
    env: dict | None = None,
    extra: dict | None = None,
) -> str:
    """计算写入配置的完整规范条目 SHA-256 摘要（覆盖实际全部字段，用于识别外部变更）。

    extra 覆盖条目内除 command/args/env/environment 之外的全部实际字段（含未知字段），
    仅剥离本工具 marker；调用方可以传入整条 settings 字典，重复的核心键会被排除以免双重计数。
    注意：此处仅输出并保存 SHA-256 摘要，绝不明文存储或在日志中打印 env 敏感值。
    """
    cmd = command if isinstance(command, str) else list(command or [])
    arg_list = list(args or [])
    clean_env = {}
    if isinstance(env, dict):
        for k in sorted(env.keys()):
            if k != MANAGED_BY_KEY:
                clean_env[str(k)] = env[k]

    extra_dict = {}
    if isinstance(extra, dict):
        for k in sorted(extra.keys()):
            if k in ("command", "args", "env", "environment", MANAGED_BY_KEY):
                continue
            extra_dict[str(k)] = extra[k]

    canonical = {
        "command": cmd,
        "args": arg_list,
        "env": clean_env,
        "extra": extra_dict,
    }
    raw = json.dumps(canonical, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


BINDING_ENTRY_KEYS = ("version", "definition", "enabled", "assigned", "applied_digest")
SHA256_HEX_RE = re.compile(r"^[0-9a-f]{64}$")


def validate_binding_entry(server: str, agent: str, entry: Any) -> dict:
    """深层校验单条绑定记录：精确布尔、摘要格式、定义有效且版本与定义摘要匹配。

    墓碑 (assigned=False) 允许显式空 definition + 空 version；其余情况版本必须等于
    未注入凭证定义的 SHA-256。校验失败 fail-closed 抛 McpBindingError，
    错误信息只含标识符与字段名，绝不打印 definition/env 内容。
    """
    validate_identifier(server, "MCP Server 名称")
    validate_identifier(agent, "Agent 名称")
    if not isinstance(entry, dict):
        raise McpBindingError(f"绑定条目 {server}@{agent} 不是对象，fail-closed 拒绝")
    unknown_keys = sorted(set(entry.keys()) - set(BINDING_ENTRY_KEYS))
    if unknown_keys:
        raise McpBindingError(f"绑定条目 {server}@{agent} 含未知字段: {unknown_keys}")
    missing_keys = sorted(set(BINDING_ENTRY_KEYS) - set(entry.keys()))
    if missing_keys:
        raise McpBindingError(f"绑定条目 {server}@{agent} 缺少字段: {missing_keys}")
    for flag in ("enabled", "assigned"):
        if type(entry[flag]) is not bool:
            raise McpBindingError(
                f"绑定条目 {server}@{agent} 字段 {flag} 必须是精确布尔类型，拒绝类型强转"
            )
    definition = entry["definition"]
    if not isinstance(definition, dict):
        raise McpBindingError(f"绑定条目 {server}@{agent} 字段 definition 必须是对象")
    version = entry["version"]
    if not isinstance(version, str):
        raise McpBindingError(f"绑定条目 {server}@{agent} 字段 version 必须是字符串")
    expected_version = compute_definition_version(definition) if definition else ""
    if version != expected_version:
        raise McpBindingError(
            f"绑定条目 {server}@{agent} 版本与未注入定义摘要不匹配，fail-closed 拒绝"
        )
    if entry["assigned"] and (not definition or not version):
        raise McpBindingError(
            f"绑定条目 {server}@{agent} assigned=true 但 definition/version 为空，仅墓碑 (assigned=false) 允许空定义/版本"
        )
    if definition:
        validate_un_injected_definition(definition, server)
    applied_digest = entry["applied_digest"]
    if not isinstance(applied_digest, str) or (
        applied_digest and not SHA256_HEX_RE.fullmatch(applied_digest)
    ):
        raise McpBindingError(f"绑定条目 {server}@{agent} 字段 applied_digest 摘要格式非法")
    return entry


def validate_bindings_tree(bindings: Any) -> dict:
    """深层校验整棵 mcp_bindings 树；任何嵌套坏数据 fail-closed 抛错。"""
    if not isinstance(bindings, dict):
        raise McpBindingError("mcp_bindings 根节点必须是对象")
    for server, agent_map in bindings.items():
        validate_identifier(server, "MCP Server 名称")
        if not isinstance(agent_map, dict):
            raise McpBindingError(f"mcp_bindings[{server}] 不是对象，fail-closed 拒绝")
        for agent, entry in agent_map.items():
            validate_binding_entry(server, agent, entry)
    return bindings


def load_mcp_bindings(db_path: Path | str) -> dict:
    """从 manager.db 的 policy_entries 表中读取 mcp_bindings JSON（深层校验，坏数据 fail-closed）。"""
    db_path = Path(db_path)
    conn = connect_ready_db(db_path, read_only=True)
    try:
        row = conn.execute(
            "SELECT value_json FROM policy_entries WHERE key = ?",
            (MCP_BINDINGS_POLICY_KEY,),
        ).fetchone()
        if not row or not row["value_json"]:
            return {}
        try:
            data = json.loads(row["value_json"])
        except (ValueError, TypeError) as exc:
            raise McpBindingError(f"mcp_bindings JSON 损坏: {exc}") from exc
        validate_bindings_tree(data)
        return copy.deepcopy(data)
    finally:
        conn.close()


def get_binding(bindings: dict, server: str, agent: str) -> dict | None:
    """获取单个 agent 的分配绑定信息。"""
    validate_identifier(server, "MCP Server 名称")
    validate_identifier(agent, "Agent 名称")
    return bindings.get(server, {}).get(agent)


def set_binding(
    bindings: dict,
    server: str,
    agent: str,
    *,
    version: str,
    definition: dict,
    enabled: bool = True,
    assigned: bool = True,
    applied_digest: str = "",
) -> dict:
    """设置或更新单个 agent 的分配绑定信息。"""
    validate_identifier(server, "MCP Server 名称")
    validate_identifier(agent, "Agent 名称")
    if type(enabled) is not bool or type(assigned) is not bool:
        raise McpBindingError(
            f"绑定条目 {server}@{agent} 的 enabled/assigned 必须是精确布尔类型，拒绝类型强转"
        )
    if server not in bindings or not isinstance(bindings[server], dict):
        bindings[server] = {}
    valid_def = validate_un_injected_definition(definition, server) if definition else {}
    expected_version = compute_definition_version(valid_def) if valid_def else ""
    if not isinstance(version, str) or version != expected_version:
        raise McpBindingError(
            f"绑定条目 {server}@{agent} 版本与未注入定义摘要不匹配，fail-closed 拒绝"
        )
    if not isinstance(applied_digest, str) or (
        applied_digest and not SHA256_HEX_RE.fullmatch(applied_digest)
    ):
        raise McpBindingError(f"绑定条目 {server}@{agent} applied_digest 摘要格式非法")
    if assigned and (not valid_def or not version):
        raise McpBindingError(
            f"绑定条目 {server}@{agent} assigned=true 但 definition/version 为空，仅墓碑 (assigned=false) 允许空定义/版本"
        )
    entry = {
        "version": version,
        "definition": valid_def,
        "enabled": enabled,
        "assigned": assigned,
        "applied_digest": applied_digest,
    }
    bindings[server][agent] = entry
    return entry


def remove_binding(bindings: dict, server: str, agent: str) -> dict:
    """标记单个 agent 的分配为 assigned=False，防止后续 sync 自动复活。"""
    validate_identifier(server, "MCP Server 名称")
    validate_identifier(agent, "Agent 名称")
    entry = get_binding(bindings, server, agent)
    if entry is None:
        entry = {
            "version": "",
            "definition": {},
            "enabled": False,
            "assigned": False,
            "applied_digest": "",
        }
    else:
        entry["assigned"] = False
        entry["enabled"] = False
    return set_binding(
        bindings,
        server,
        agent,
        version=entry.get("version", ""),
        definition=entry.get("definition", {}),
        enabled=False,
        assigned=False,
        applied_digest=entry.get("applied_digest", ""),
    )


def save_mcp_bindings(
    db_path: Path | str,
    bindings: dict,
    *,
    action: str = "update",
    target: str | None = None,
    detail: dict | None = None,
    run_id: str | None = None,
    server: str | None = None,
    agent: str | None = None,
    expected_entry: Any = UNSET,
    scoped_changes: list | None = None,
) -> dict:
    """在事务中将 mcp_bindings 写入 policy_entries 并记录 operation_logs 审计。

    scoped_changes 为 (server, agent, expected_old_entry|None, new_entry|None) 列表：
    仅应用本次显式变更并在事务内对比 expected 旧值，绝不整树 stale 合并覆盖其它并发修改。
    new_entry 为 None 表示删除该绑定。数据库中已有坏数据 fail-closed 拒绝覆盖。
    """
    db_path = Path(db_path)
    now_str = datetime.now(timezone.utc).isoformat()
    op_type = f"mcp:binding:{action}" if not action.startswith("mcp:") else action
    detail_payload = dict(detail or {})
    detail_payload.setdefault("action", action)

    if server is None or agent is None:
        if target and "@" in target:
            server, agent = target.split("@", 1)

    conn = connect_ready_db(db_path)
    try:
        conn.execute("BEGIN IMMEDIATE")
        try:
            row = conn.execute(
                "SELECT value_json FROM policy_entries WHERE key = ?",
                (MCP_BINDINGS_POLICY_KEY,),
            ).fetchone()
            db_bindings = {}
            if row and row["value_json"]:
                try:
                    loaded = json.loads(row["value_json"])
                except (ValueError, TypeError) as exc:
                    raise McpBindingError(f"数据库中 mcp_bindings JSON 损坏，拒绝覆盖: {exc}") from exc
                if not isinstance(loaded, dict):
                    raise McpBindingError("数据库中 mcp_bindings 不是对象，拒绝覆盖")
                # 深层校验已有数据，坏数据 fail-closed 绝不覆盖
                validate_bindings_tree(loaded)
                db_bindings = loaded

            def _safe_summary(e: Any) -> str:
                if e is None:
                    return "None"
                if not isinstance(e, dict):
                    return "<invalid>"
                v_short = (e.get("version") or "")[:12]
                return f"version={v_short}..., enabled={e.get('enabled')}, assigned={e.get('assigned')}"

            if scoped_changes is not None:
                for change in scoped_changes:
                    s_name, a_name, expected_old, new_entry = change
                    validate_identifier(s_name, "MCP Server 名称")
                    validate_identifier(a_name, "Agent 名称")
                    current_db_entry = get_binding(db_bindings, s_name, a_name)
                    if current_db_entry != expected_old:
                        raise McpBindingConflictError(
                            f"并发冲突: {s_name}@{a_name} 在事务内已变更 "
                            f"(期望旧状态: {_safe_summary(expected_old)}, 数据库实际状态: {_safe_summary(current_db_entry)})"
                        )
                    if new_entry is None:
                        if s_name in db_bindings and a_name in db_bindings[s_name]:
                            del db_bindings[s_name][a_name]
                    else:
                        validate_binding_entry(s_name, a_name, new_entry)
                        if s_name not in db_bindings or not isinstance(db_bindings[s_name], dict):
                            db_bindings[s_name] = {}
                        db_bindings[s_name][a_name] = copy.deepcopy(new_entry)
            elif server and agent:
                if expected_entry is not UNSET:
                    current_db_entry = get_binding(db_bindings, server, agent)
                    if current_db_entry != expected_entry:
                        raise McpBindingConflictError(
                            f"并发冲突: {server}@{agent} 在事务内已变更 "
                            f"(期望旧状态: {_safe_summary(expected_entry)}, 数据库实际状态: {_safe_summary(current_db_entry)})"
                        )
                new_entry = get_binding(bindings, server, agent)
                if new_entry is not None:
                    set_binding(
                        db_bindings,
                        server,
                        agent,
                        version=new_entry.get("version", ""),
                        definition=new_entry.get("definition", {}),
                        enabled=new_entry.get("enabled", True),
                        assigned=new_entry.get("assigned", True),
                        applied_digest=new_entry.get("applied_digest", ""),
                    )
                else:
                    if server in db_bindings and agent in db_bindings[server]:
                        del db_bindings[server][agent]
            else:
                for s_name, s_agents in bindings.items():
                    if not isinstance(s_agents, dict):
                        continue
                    for a_name, a_entry in s_agents.items():
                        if isinstance(a_entry, dict):
                            set_binding(
                                db_bindings,
                                s_name,
                                a_name,
                                version=a_entry.get("version", ""),
                                definition=a_entry.get("definition", {}),
                                enabled=a_entry.get("enabled", True),
                                assigned=a_entry.get("assigned", True),
                                applied_digest=a_entry.get("applied_digest", ""),
                            )

            value_json = json.dumps(db_bindings, ensure_ascii=False, indent=2, sort_keys=True)
            conn.execute(
                """
                INSERT INTO policy_entries (key, value_json, updated_at)
                VALUES (?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at
                """,
                (MCP_BINDINGS_POLICY_KEY, value_json, now_str),
            )
            record_operation(
                conn,
                operation_type=op_type,
                target=target or (f"{server}@{agent}" if server and agent else None),
                detail=detail_payload,
                run_id=run_id,
            )
            conn.commit()
            bindings.clear()
            bindings.update(copy.deepcopy(db_bindings))
            return db_bindings
        except Exception:
            conn.rollback()
            raise
    finally:
        conn.close()


def extract_server_from_config(
    config_path: Path,
    agent_name: str,
    file_format: str,
    server_name: str,
) -> tuple[dict | None, bool]:
    """从现有配置文件中提取指定 server 的规范信息和托管标记。

    返回: (normalized_dict_or_None, is_managed: bool)
    """
    path = Path(config_path).expanduser()
    if not path.exists():
        return None, False

    if file_format == "json":
        try:
            with open(path, "r", encoding="utf-8") as f:
                doc = json.load(f)
        except Exception as exc:
            raise McpConflictError(f"配置文件 JSON 解析失败，拒绝覆盖: {config_path} ({exc})") from exc

        if not isinstance(doc, dict):
            raise McpConflictError(f"配置文件根节点不是对象，拒绝覆盖: {config_path}")

        is_opencode = (agent_name == "opencode")
        container_key = "mcp" if is_opencode else "mcpServers"
        if container_key not in doc:
            return None, False

        servers = doc[container_key]
        if not isinstance(servers, dict):
            raise McpConflictError(f"配置文件 {container_key} 不是对象，拒绝覆盖: {config_path}")

        if server_name not in servers:
            return None, False

        srv = servers[server_name]
        if not isinstance(srv, dict):
            raise McpConflictError(f"配置文件中 server '{server_name}' 不是对象，拒绝覆盖: {config_path}")

        if is_opencode:
            env = srv.get("environment") or {}
            if not isinstance(env, dict):
                raise McpConflictError(f"配置文件中 server '{server_name}' 的 environment 不是对象，拒绝覆盖: {config_path}")
            is_managed = (env.get(MANAGED_BY_KEY) == MANAGED_BY_VALUE)
            cmd_arr = srv.get("command") or []
            cmd = cmd_arr[0] if cmd_arr else ""
            args = cmd_arr[1:] if len(cmd_arr) > 1 else []
            clean_env = {}
            for k, v in env.items():
                if k == MANAGED_BY_KEY:
                    continue
                if not isinstance(v, str):
                    raise McpConflictError(
                        f"配置文件中 server '{server_name}' 环境变量 '{k}' 包含非法类型 ({type(v).__name__})，拒绝覆盖: {config_path}"
                    )
                clean_env[str(k)] = v
            extra = {k: v for k, v in srv.items() if k not in ("command", "environment")}
            return {
                "command": cmd,
                "args": args,
                "env": clean_env,
                "extra": extra,
            }, is_managed
        else:
            env = srv.get("env") or {}
            if not isinstance(env, dict):
                raise McpConflictError(f"配置文件中 server '{server_name}' 的 env 不是对象，拒绝覆盖: {config_path}")
            is_managed = (env.get(MANAGED_BY_KEY) == MANAGED_BY_VALUE)
            extra = {k: v for k, v in srv.items() if k not in ("command", "args", "env")}
            clean_env = {}
            for k, v in env.items():
                if k == MANAGED_BY_KEY:
                    continue
                if not isinstance(v, str):
                    raise McpConflictError(
                        f"配置文件中 server '{server_name}' 环境变量 '{k}' 包含非法类型 ({type(v).__name__})，拒绝覆盖: {config_path}"
                    )
                clean_env[str(k)] = v
            return {
                "command": srv.get("command", ""),
                "args": srv.get("args", []) or [],
                "env": clean_env,
                "extra": extra,
            }, is_managed

    elif file_format == "toml":
        try:
            content = path.read_text(encoding="utf-8")
        except Exception as exc:
            raise McpConflictError(f"读取 TOML 文件失败: {config_path} ({exc})") from exc

        if not content.strip():
            return None, False

        if tomllib is not None:
            try:
                doc = tomllib.loads(content)
            except Exception as exc:
                raise McpConflictError(f"配置文件 TOML 解析失败，拒绝覆盖: {config_path} ({exc})") from exc

            if not isinstance(doc, dict):
                raise McpConflictError(f"配置文件根节点不是表，拒绝覆盖: {config_path}")

            servers = doc.get("mcp_servers")
            if servers is None:
                return None, False
            if not isinstance(servers, dict):
                raise McpConflictError(f"配置文件 mcp_servers 不是表，拒绝覆盖: {config_path}")

            if server_name not in servers:
                return None, False

            srv = servers[server_name]
            if not isinstance(srv, dict):
                raise McpConflictError(f"配置文件中 mcp_servers.{server_name} 不是表，拒绝覆盖: {config_path}")

            env = srv.get("env") or {}
            if not isinstance(env, dict):
                raise McpConflictError(f"配置文件中 mcp_servers.{server_name} 的 env 不是表，拒绝覆盖: {config_path}")
            is_managed = (env.get(MANAGED_BY_KEY) == MANAGED_BY_VALUE)
            extra = {k: v for k, v in srv.items() if k not in ("command", "args", "env")}
            clean_env = {}
            for k, v in env.items():
                if k == MANAGED_BY_KEY:
                    continue
                if not isinstance(v, str):
                    raise McpConflictError(
                        f"配置文件中 mcp_servers.{server_name} 环境变量 '{k}' 包含非法类型 ({type(v).__name__})，拒绝覆盖: {config_path}"
                    )
                clean_env[str(k)] = v
            return {
                "command": srv.get("command", ""),
                "args": srv.get("args", []) or [],
                "env": clean_env,
                "extra": extra,
            }, is_managed

    return None, False


def toml_quote(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False)


def toml_key(value: str) -> str:
    if TOML_BARE_KEY_RE.fullmatch(value):
        return value
    return json.dumps(value, ensure_ascii=False)


def extract_toml_mcp_server_name(chunk: str) -> str | None:
    header_match = TOML_SERVER_HEADER_RE.search(chunk)
    if not header_match:
        return None
    key_token = header_match.group("key")
    if key_token.startswith('"'):
        return json.loads(key_token)
    return key_token


def merge_toml_chunks_by_server(chunks: list[str]) -> list[str]:
    if not chunks:
        return chunks
    merged_chunks = [chunks[0]]
    previous_server = extract_toml_mcp_server_name(chunks[0])
    for chunk in chunks[1:]:
        current_server = extract_toml_mcp_server_name(chunk)
        if current_server and current_server == previous_server:
            merged_chunks[-1] += chunk
            continue
        merged_chunks.append(chunk)
        previous_server = current_server
    return merged_chunks


def atomic_rollback_file(
    config_path: Path,
    old_bytes: bytes | None,
    expected_current_bytes: bytes | None = None,
) -> None:
    """原子恢复原配置文件（同目录原子替换，不截断覆盖外部并发修改）。"""
    config_path = Path(config_path).expanduser()
    if expected_current_bytes is not None:
        cur_on_disk = config_path.read_bytes() if config_path.exists() else None
        if cur_on_disk != expected_current_bytes:
            logging.error(f"回滚中止: 文件在写入后已被外部并发修改，不进行覆盖恢复 ({config_path})")
            return
    if old_bytes is not None:
        config_path.parent.mkdir(parents=True, exist_ok=True)
        fd_rb, tmp_rb = tempfile.mkstemp(dir=str(config_path.parent), prefix=f"{config_path.name}.rb_")
        try:
            with os.fdopen(fd_rb, "wb") as f_rb:
                f_rb.write(old_bytes)
            os.chmod(tmp_rb, 0o600)
            os.replace(tmp_rb, config_path)
        except Exception as rb_exc:
            if os.path.exists(tmp_rb):
                try:
                    os.remove(tmp_rb)
                except OSError:
                    pass
            logging.error(f"回滚旧配置失败: {rb_exc}")
    else:
        config_path.unlink(missing_ok=True)


def safe_atomic_write_and_verify(
    config_path: Path,
    content_writer: Callable[[], str | bytes],
    verify_cb: Callable[[Path], bool],
    db_commit_cb: Callable[[], None] | None = None,
    dry_run: bool = False,
    expected_old_bytes: bytes | None = UNSET,
) -> None:
    """原子更新配置文件并读回验证，若数据库提交失败则由内存快照回滚。"""
    config_path = Path(config_path).expanduser()
    if dry_run:
        return

    old_bytes = expected_old_bytes if expected_old_bytes is not UNSET else (config_path.read_bytes() if config_path.exists() else None)

    # 先渲染再创建临时文件：render 失败时没有任何临时文件/FD 泄漏，目标文件与 DB 均不变
    new_content = content_writer()
    new_bytes = new_content if isinstance(new_content, bytes) else new_content.encode("utf-8")

    config_path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_path = tempfile.mkstemp(dir=str(config_path.parent), prefix=f"{config_path.name}.tmp_")
    try:
        with os.fdopen(fd, "wb" if isinstance(new_content, bytes) else "w", encoding=None if isinstance(new_content, bytes) else "utf-8") as f:
            f.write(new_content)
        os.chmod(temp_path, 0o600)

        # 检查读旧值到replace前目标文件未被外部改动
        current_disk_bytes = config_path.read_bytes() if config_path.exists() else None
        if current_disk_bytes != old_bytes:
            if os.path.exists(temp_path):
                try:
                    os.remove(temp_path)
                except OSError:
                    pass
            raise McpConflictError(f"外部文件并发变更: 目标文件在写入前已被外部修改 ({config_path})")

        os.replace(temp_path, config_path)
    except Exception as exc:
        if os.path.exists(temp_path):
            try:
                os.remove(temp_path)
            except OSError:
                pass
        if isinstance(exc, McpConflictError):
            raise
        raise McpBindingError(f"原子写入文件失败: {exc}") from exc

    def _restore_old():
        atomic_rollback_file(config_path, old_bytes, expected_current_bytes=new_bytes)

    # 回读验证，若抛异常或返回 False 也要回滚
    verify_ok = False
    try:
        verify_ok = bool(verify_cb(config_path))
    except BaseException as v_exc:
        _restore_old()
        raise McpBindingError(f"配置回读验证异常 ({config_path}): {v_exc}") from v_exc

    if not verify_ok:
        _restore_old()
        raise McpBindingError(f"配置回读验证失败 ({config_path})")

    # 提交数据库，失败则回滚配置文件
    if db_commit_cb is not None:
        try:
            db_commit_cb()
        except BaseException:
            _restore_old()
            raise
