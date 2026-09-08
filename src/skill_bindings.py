"""Skill 不可变版本绑定与逐助手分配隔离模块。

在 SQLite 激活后：
1. 使用数据目录 skill-versions 保存不可变内容快照。
2. 复用现有 policy_entries 表，存储一个顶层 key 'skill_bindings' 的 JSON。
   键结构：collection -> skill -> agent -> {version, enabled, assigned}。
3. 通过 connect_ready_db + 事务读写并调用 record_operation 审计。
4. 提供单 Agent 隔离检查、原子链接替换、外部目录防覆盖保护与 DB 失败回滚能力。
"""

from __future__ import annotations

import copy
from datetime import datetime, timezone
import json
import logging
import ntpath
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import uuid

from src.skill_versions import SkillVersionError, _VERSION_ID_RE, snapshot_skill, verify_skill_version
from src.storage.db import connect_ready_db, get_data_dir, get_db_path, is_sqlite_active
from src.storage.operations import record_operation

SKILL_BINDINGS_POLICY_KEY = "skill_bindings"
SKILL_VERSIONS_DIRNAME = "skill-versions"


class SkillBindingError(ValueError):
    """Skill 绑定与分配相关错误。"""


class SkillConflictError(SkillBindingError):
    """目标位置已存在外部非受管内容冲突。"""


class SkillBindingConflictError(SkillBindingError):
    """技能分配存在并发修改冲突。"""


class SkillSharedTargetError(SkillBindingError):
    """目标目录与其他 Agent 共享，无法实现隔离。"""


def validate_path_identifier(name: str, identifier_type: str = "标识符") -> None:
    """校验 ID 或名称用于路径前拒绝 /、反斜线、.、.. 等路径穿越或非法命名。"""
    if not name or not isinstance(name, str):
        raise SkillBindingError(f"{identifier_type} 不能为空")
    if name in (".", "..") or "/" in name or "\\" in name or "\x00" in name:
        raise SkillBindingError(f"{identifier_type} 包含非法路径字符或逃逸命名: {name!r}")


def get_skill_versions_dir(data_dir: Path | str | None = None) -> Path:
    """获取不可变 skill 版本存储目录。"""
    target_dir = Path(data_dir) if data_dir is not None else get_data_dir()
    return target_dir / SKILL_VERSIONS_DIRNAME


def _is_windows_path(path_str: str) -> bool:
    """判断路径字符串是否为实际 Windows 盘符或 UNC 路径。"""
    if len(path_str) >= 2 and path_str[0].isalpha() and path_str[1] == ":":
        return True
    if path_str.startswith(("\\\\", "//")):
        return True
    return False


def normalize_windows_device_path(path) -> str:
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


def normalize_path_for_compare(path) -> tuple[str, str]:
    path_str = normalize_windows_device_path(path)
    if _is_windows_path(path_str):
        return "windows", ntpath.normcase(ntpath.normpath(path_str))
    return "posix", str(Path(path_str).expanduser().resolve(strict=False))


def is_windows_directory_link(path) -> bool:
    path = Path(path)
    checker = getattr(os.path, "isjunction", None)
    if checker is not None:
        try:
            if checker(path):
                return True
        except OSError:
            return False
    try:
        stat_result = os.lstat(path)
    except OSError:
        return False
    file_attrs = getattr(stat_result, "st_file_attributes", 0)
    reparse_flag = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
    directory_flag = getattr(stat, "FILE_ATTRIBUTE_DIRECTORY", 0)
    return bool(reparse_flag and directory_flag and file_attrs & reparse_flag and file_attrs & directory_flag)


def is_link_entry(path) -> bool:
    path = Path(path)
    return path.is_symlink() or is_windows_directory_link(path)


def resolve_link_target(link_path) -> str:
    link_path = Path(link_path)
    if not is_link_entry(link_path):
        raise OSError(f"Not a link entry: {link_path}")
    try:
        raw_target = normalize_windows_device_path(os.readlink(link_path))
        if _is_windows_path(raw_target):
            return ntpath.normpath(raw_target)
        raw_target_path = Path(raw_target).expanduser()
        resolved_target = raw_target_path if raw_target_path.is_absolute() else (link_path.parent / raw_target_path)
        return str(resolved_target.resolve(strict=False))
    except OSError:
        if is_windows_directory_link(link_path):
            return str(link_path.resolve(strict=False))
        raise


def is_path_within_base(path, base_path) -> bool:
    path_kind, resolved_path = normalize_path_for_compare(path)
    base_kind, resolved_base = normalize_path_for_compare(base_path)
    if path_kind != base_kind:
        return False
    commonpath_func = ntpath.commonpath if path_kind == "windows" else os.path.commonpath
    try:
        common_path = commonpath_func([resolved_path, resolved_base])
    except ValueError:
        return False
    return common_path == resolved_base


def link_points_within_any_base(link_path, base_paths) -> bool:
    if not is_link_entry(link_path):
        return False
    try:
        target = resolve_link_target(link_path)
    except OSError:
        return False
    for base in base_paths or ():
        if base and is_path_within_base(target, base):
            return True
    return False


def is_managed_skill_link(link_path, managed_bases) -> bool:
    """判断给定路径是否为本工具管理的软链接/目录联接。"""
    return is_link_entry(link_path) and link_points_within_any_base(link_path, managed_bases)


def check_not_external_conflict(link_path, managed_bases) -> None:
    """检查 link_path 是否已被外部常规目录/文件/未受管软链接占用。若是则拒绝覆盖。"""
    link_path = Path(link_path)
    if not link_path.exists() and not is_link_entry(link_path):
        return
    if not is_managed_skill_link(link_path, managed_bases):
        raise SkillConflictError(
            f"目标路径已存在外部非受管内容 ({link_path})，拒绝静默覆盖或删除以防损坏用户文件"
        )


def check_shared_agent_target(agent_name: str, agent_targets: dict) -> None:
    """检查指定 Agent 的技能目录是否与其他 Agent 物理共享。若是则拒绝单 Agent 操作。"""
    if agent_name not in agent_targets:
        return
    target_path = agent_targets[agent_name]
    target_norm = normalize_path_for_compare(target_path)
    for other_agent, other_path in agent_targets.items():
        if other_agent == agent_name:
            continue
        if normalize_path_for_compare(other_path) == target_norm:
            raise SkillSharedTargetError(
                f"Agent '{agent_name}' 的技能目标目录与 Agent '{other_agent}' 共享 ({target_path})，"
                f"单 Agent 操作明确拒绝以防破坏其他 Agent 的隔离状态"
            )


def validate_skill_bindings_data(data: dict) -> None:
    """深度校验 skill_bindings 字典结构、标识符安全与字段类型。

    校验规则：
    1. 根节点必须是 dict。
    2. collection / skill / agent 标识符均通过 validate_path_identifier 校验，
       拒绝 /、反斜线、.、..、空字符串及特殊空字节，防范路径逃逸。
    3. collection 映射、skill 映射、agent 分配记录必须是 dict。
    4. 分配记录中：
       - version 必须为符合 64 位十六进制 SHA-256 摘要格式的字符串；
       - enabled 必须为布尔值（type(enabled) is bool）；
       - assigned 必须为布尔值（type(assigned) is bool）。
    """
    if not isinstance(data, dict):
        raise SkillBindingError("skill_bindings 根节点必须是对象")
    for collection, skills in data.items():
        validate_path_identifier(collection, "集合名称")
        if not isinstance(skills, dict):
            raise SkillBindingError(f"集合 {collection!r} 的技能映射必须是对象，实际: {type(skills).__name__}")
        for skill, agents in skills.items():
            validate_path_identifier(skill, "Skill 名称")
            if not isinstance(agents, dict):
                raise SkillBindingError(f"Skill {collection}/{skill!r} 的 Agent 映射必须是对象，实际: {type(agents).__name__}")
            for agent, record in agents.items():
                validate_path_identifier(agent, "Agent 名称")
                if not isinstance(record, dict):
                    raise SkillBindingError(f"分配记录 {collection}/{skill}@{agent} 必须是对象，实际: {type(record).__name__}")
                version = record.get("version")
                if not isinstance(version, str) or not _VERSION_ID_RE.match(version):
                    raise SkillBindingError(
                        f"分配记录 {collection}/{skill}@{agent} 的 version 格式非法: {version!r}，"
                        f"必须是 64 位十六进制 SHA-256 摘要"
                    )
                enabled = record.get("enabled")
                if type(enabled) is not bool:
                    raise SkillBindingError(
                        f"分配记录 {collection}/{skill}@{agent} 的 enabled 必须是布尔值，实际: {type(enabled).__name__}"
                    )
                assigned = record.get("assigned")
                if type(assigned) is not bool:
                    raise SkillBindingError(
                        f"分配记录 {collection}/{skill}@{agent} 的 assigned 必须是布尔值，实际: {type(assigned).__name__}"
                    )


def load_skill_bindings(db_path: Path | str) -> dict:
    """从 manager.db 的 policy_entries 表中读取 skill_bindings JSON。"""
    db_path = Path(db_path)
    conn = connect_ready_db(db_path, read_only=True)
    try:
        row = conn.execute(
            "SELECT value_json FROM policy_entries WHERE key = ?",
            (SKILL_BINDINGS_POLICY_KEY,),
        ).fetchone()
        if not row or not row["value_json"]:
            return {}
        try:
            data = json.loads(row["value_json"])
        except (ValueError, TypeError) as exc:
            raise SkillBindingError(f"skill_bindings JSON 损坏: {exc}") from exc
        validate_skill_bindings_data(data)
        # 校验并返回干净数据
        return copy.deepcopy(data)
    finally:
        conn.close()


def parse_target_triple(target: str | None) -> tuple[str, str, str] | None:
    """从形如 collection/skill@agent 的 target 字符串解析 (collection, skill, agent)。"""
    if not target or "@" not in target or "/" not in target:
        return None
    try:
        coll_and_skill, agent = target.split("@", 1)
        collection, skill = coll_and_skill.split("/", 1)
        if collection and skill and agent:
            return collection, skill, agent
    except ValueError:
        pass
    return None


_SENTINEL_UNSPECIFIED = object()


def save_skill_bindings(
    db_path: Path | str,
    bindings: dict,
    *,
    action: str = "update",
    target: str | None = None,
    detail: dict | None = None,
    run_id: str | None = None,
    collection: str | None = None,
    skill: str | None = None,
    agent: str | None = None,
    expected_entry: dict | None | object = _SENTINEL_UNSPECIFIED,
) -> dict:
    """在事务中将 skill_bindings 写入 policy_entries 并记录 operation_logs 审计。

    支持事务内按目标合并与旧值冲突拒绝：
    1. 显式开启 BEGIN IMMEDIATE 事务，在读取前取得写锁，防止并发写冲突。
    2. 事务内重读数据库最新 bindings。坏 JSON 严格 fail-closed 报错并保留数据库原始字节，拒绝覆盖。
    3. 若提供了目标或从 target (形如 collection/skill@agent) 解析出目标：
       - 若 expected_entry 被指定（包含 expected_entry=None，表示期望目标尚不存在）：
         比对当前 DB 中该目标的旧值；若不一致或已存在则抛出 SkillBindingConflictError。
       - 仅将目标 (collection, skill, agent) 的新值合并进最新 DB 数据，避免覆盖其他进程对其他目标的并发更改。
    4. 若为批量操作，则深合并至最新 DB 数据。
    5. 将合并后的最新数据回写数据库、COMMIT 事务、更新传入的 bindings 字典并记录审计日志。
    """
    db_path = Path(db_path)
    now_str = datetime.now(timezone.utc).isoformat()
    op_type = f"skill:binding:{action}" if not action.startswith("skill:") else action
    detail_payload = dict(detail or {})
    detail_payload.setdefault("action", action)

    if collection is None or skill is None or agent is None:
        triple = parse_target_triple(target)
        if triple:
            collection, skill, agent = triple

    conn = connect_ready_db(db_path)
    try:
        conn.isolation_level = None
        conn.execute("BEGIN IMMEDIATE")
        try:
            row = conn.execute(
                "SELECT value_json FROM policy_entries WHERE key = ?",
                (SKILL_BINDINGS_POLICY_KEY,),
            ).fetchone()
            db_bindings = {}
            if row and row["value_json"]:
                try:
                    loaded = json.loads(row["value_json"])
                except (ValueError, TypeError) as exc:
                    raise SkillBindingError(
                        f"policy_entries 中 {SKILL_BINDINGS_POLICY_KEY} 的 JSON 数据损坏，拒绝覆盖以保护原数据: {exc}"
                    ) from exc
                validate_skill_bindings_data(loaded)
                db_bindings = loaded

            if collection and skill and agent:
                if expected_entry is not _SENTINEL_UNSPECIFIED:
                    current_db_entry = get_binding(db_bindings, collection, skill, agent)
                    if expected_entry is None:
                        if current_db_entry is not None:
                            raise SkillBindingConflictError(
                                f"并发冲突: {collection}/{skill}@{agent} 在事务内已存在 "
                                f"(期望不存在，数据库实际: {current_db_entry})"
                            )
                    elif current_db_entry != expected_entry:
                        raise SkillBindingConflictError(
                            f"并发冲突: {collection}/{skill}@{agent} 在事务内已变更 "
                            f"(期望旧值: {expected_entry}, 数据库实际: {current_db_entry})"
                        )
                new_entry = get_binding(bindings, collection, skill, agent)
                if new_entry is not None:
                    set_binding(
                        db_bindings,
                        collection,
                        skill,
                        agent,
                        version=new_entry.get("version", ""),
                        enabled=new_entry.get("enabled", True),
                        assigned=new_entry.get("assigned", True),
                    )
                else:
                    if collection in db_bindings and skill in db_bindings[collection] and agent in db_bindings[collection][skill]:
                        del db_bindings[collection][skill][agent]
            else:
                for c_name, c_skills in bindings.items():
                    if not isinstance(c_skills, dict):
                        continue
                    for s_name, s_agents in c_skills.items():
                        if not isinstance(s_agents, dict):
                            continue
                        for a_name, a_entry in s_agents.items():
                            if isinstance(a_entry, dict):
                                set_binding(
                                    db_bindings,
                                    c_name,
                                    s_name,
                                    a_name,
                                    version=a_entry.get("version", ""),
                                    enabled=a_entry.get("enabled", True),
                                    assigned=a_entry.get("assigned", True),
                                )

            validate_skill_bindings_data(db_bindings)
            value_json = json.dumps(db_bindings, ensure_ascii=False, indent=2, sort_keys=True)
            conn.execute(
                """
                INSERT INTO policy_entries (key, value_json, updated_at)
                VALUES (?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at
                """,
                (SKILL_BINDINGS_POLICY_KEY, value_json, now_str),
            )
            record_operation(
                conn,
                operation_type=op_type,
                target=target,
                detail=detail_payload,
                run_id=run_id,
            )
            conn.execute("COMMIT")
            bindings.clear()
            bindings.update(copy.deepcopy(db_bindings))
            return db_bindings
        except BaseException:
            conn.execute("ROLLBACK")
            raise
    finally:
        conn.close()


def get_binding(bindings: dict, collection: str, skill: str, agent: str) -> dict | None:
    """获取单个 agent 的分配绑定信息。"""
    validate_path_identifier(collection, "集合名称")
    validate_path_identifier(skill, "Skill 名称")
    validate_path_identifier(agent, "Agent 名称")
    return bindings.get(collection, {}).get(skill, {}).get(agent)


def set_binding(
    bindings: dict,
    collection: str,
    skill: str,
    agent: str,
    *,
    version: str,
    enabled: bool = True,
    assigned: bool = True,
) -> dict:
    """设置或更新单个 agent 的分配绑定信息。"""
    validate_path_identifier(collection, "集合名称")
    validate_path_identifier(skill, "Skill 名称")
    validate_path_identifier(agent, "Agent 名称")
    if not isinstance(version, str) or not _VERSION_ID_RE.match(version):
        raise SkillBindingError(
            f"version 格式非法: {version!r}，必须是 64 位十六进制 SHA-256 摘要"
        )
    if type(enabled) is not bool:
        raise SkillBindingError(f"enabled 必须是布尔值，实际: {type(enabled).__name__}")
    if type(assigned) is not bool:
        raise SkillBindingError(f"assigned 必须是布尔值，实际: {type(assigned).__name__}")
    if collection not in bindings or not isinstance(bindings[collection], dict):
        bindings[collection] = {}
    coll_dict = bindings[collection]
    if skill not in coll_dict or not isinstance(coll_dict[skill], dict):
        coll_dict[skill] = {}
    entry = {
        "version": str(version),
        "enabled": bool(enabled),
        "assigned": bool(assigned),
    }
    coll_dict[skill][agent] = entry
    return entry


def remove_binding(
    bindings: dict,
    collection: str,
    skill: str,
    agent: str,
) -> dict:
    """标记单个 agent 的分配为 assigned=False，防止后续 sync 自动复活。"""
    validate_path_identifier(collection, "集合名称")
    validate_path_identifier(skill, "Skill 名称")
    validate_path_identifier(agent, "Agent 名称")
    entry = get_binding(bindings, collection, skill, agent)
    old_version = (entry.get("version") if entry else None) or ("0" * 64)
    if not isinstance(old_version, str) or not _VERSION_ID_RE.match(old_version):
        old_version = "0" * 64
    return set_binding(
        bindings,
        collection,
        skill,
        agent,
        version=old_version,
        enabled=False,
        assigned=False,
    )


def atomic_create_or_replace_link(source_path: Path, link_path: Path) -> None:
    """原子或安全地创建/更新链接，若平台不支持符号链接则明确报错。"""
    source_path = Path(source_path).resolve()
    link_path = Path(link_path)
    link_parent = link_path.parent.resolve()
    link_parent.mkdir(parents=True, exist_ok=True)

    if link_path.exists() and not is_link_entry(link_path) and link_path.is_dir():
        raise SkillConflictError(f"目标入口已存在外部非受管目录，拒绝覆盖或递归删除: {link_path}")

    if platform.system() == "Windows":
        if link_path.exists() or is_link_entry(link_path):
            if is_windows_directory_link(link_path) and not link_path.is_symlink():
                os.rmdir(link_path)
            elif link_path.is_symlink() or not link_path.is_dir():
                link_path.unlink()
            else:
                raise SkillConflictError(
                    f"目标入口是普通目录，Windows atomic helper 拒绝递归删除: {link_path}"
                )
        res = subprocess.run(
            ["cmd", "/c", "mklink", "/J", str(link_path), str(source_path)],
            capture_output=True,
            text=True,
        )
        if res.returncode != 0:
            raise OSError(f"Windows mklink failed: {res.stderr}")
        return

    # POSIX: 使用同目录临时 symlink + os.replace 实现原子替换
    rel_source = os.path.relpath(source_path, link_parent)
    temp_link = link_parent / f".{link_path.name}.tmp-{uuid.uuid4().hex}"
    try:
        os.symlink(rel_source, temp_link, target_is_directory=True)
    except OSError as exc:
        raise OSError(f"创建符号链接失败（平台不支持或权限不足）: {exc}") from exc

    try:
        os.replace(temp_link, link_path)
    except OSError:
        try:
            if temp_link.is_symlink() or temp_link.exists():
                temp_link.unlink()
        except OSError:
            pass
        raise


def safe_update_skill_link(
    source_path: Path,
    link_path: Path,
    db_commit_cb,
    managed_bases: list[Path],
) -> None:
    """安全更新受管 skill 链接并在发生任何错误或 DB 失败时恢复旧链接。

    保证：
    1. 目标若存在外部同名目录/文件，抛出 SkillConflictError 拒绝覆盖。
    2. 原子替换、回读验证以及 db_commit_cb 发生任何异常均触发回滚。
    3. 异常恢复严格限制：仅在当前入口仍为本次写入的 source_path 链接时才允许恢复/清理；
       若入口在执行期间被外部并发替换（如被替换为外部目录、普通文件或其他目标链接），
       必须完整保留并报冲突，绝不能删除新外部目录或金丝雀内容。
    """
    source_path = Path(source_path).resolve()
    link_path = Path(link_path)

    check_not_external_conflict(link_path, managed_bases)

    old_target = None
    if is_link_entry(link_path):
        try:
            old_target = resolve_link_target(link_path)
        except OSError:
            old_target = None

    try:
        atomic_create_or_replace_link(source_path, link_path)
        actual_target = resolve_link_target(link_path)
        if Path(actual_target).resolve() != source_path:
            raise SkillBindingError(
                f"链接回读目标不匹配: 期望 {source_path}, 实际 {actual_target}"
            )
        if db_commit_cb is not None:
            db_commit_cb()
    except BaseException:
        # 异常恢复严格限制：
        # 1. 若当前入口存在且仍为本次写入的 source_path 软链接：允许回滚至 old_target 或清理新建链接（unlink/os.rmdir，绝不 rmtree）；
        # 2. 若当前入口完全不存在且存在 old_target：允许恢复原链接至 old_target；
        # 3. 若当前入口已被外部并发修改（如被替换为外部目录、普通文件或其他目标链接）：
        #    必须完整保留外部内容并报冲突，绝不能删除或篡改新外部目录及金丝雀文件！
        should_rollback_old = False
        should_cleanup_new = False

        if not link_path.exists() and not is_link_entry(link_path):
            if old_target is not None:
                should_rollback_old = True
        elif is_link_entry(link_path):
            try:
                target_str = resolve_link_target(link_path)
                if Path(target_str).resolve() == source_path:
                    if old_target is not None:
                        should_rollback_old = True
                    else:
                        should_cleanup_new = True
            except OSError:
                pass

        if should_rollback_old:
            try:
                atomic_create_or_replace_link(Path(old_target), link_path)
            except Exception as rollback_exc:
                logging.error(f"操作失败后恢复原链接失败: {rollback_exc}")
        elif should_cleanup_new:
            try:
                if is_windows_directory_link(link_path) and not link_path.is_symlink():
                    os.rmdir(link_path)
                else:
                    link_path.unlink()
            except Exception as rollback_exc:
                logging.error(f"操作失败后清理新建链接失败: {rollback_exc}")
        else:
            logging.error(
                f"操作失败后回滚跳过: 入口 {link_path} 当前已被外部或并发修改，"
                f"保留外部内容并报冲突，绝不删除或覆盖"
            )
        raise


def safe_remove_skill_link(
    link_path: Path | str,
    db_commit_cb,
    managed_bases: list[Path] | None = None,
) -> None:
    """安全移除受管 skill 链接并在 DB 失败时恢复旧链接。

    保证：
    1. 目标若存在外部非受管内容，抛出 SkillConflictError 拒绝删除以防损坏文件。
    2. 记录原链接目标，移除链接并在 db_commit_cb 失败时恢复。
    3. 若 db_commit_cb 失败，仅在当前入口未被外部内容占用时恢复原链接到原目标。
    """
    link_path = Path(link_path)
    if managed_bases is not None and (link_path.exists() or is_link_entry(link_path)):
        check_not_external_conflict(link_path, managed_bases)

    if not link_path.exists() and not is_link_entry(link_path):
        if db_commit_cb is not None:
            db_commit_cb()
        return

    old_target = None
    if is_link_entry(link_path):
        try:
            old_target = resolve_link_target(link_path)
        except OSError:
            old_target = None
    elif link_path.is_dir():
        raise SkillConflictError(f"目标入口是普通目录，拒绝移除: {link_path}")

    try:
        if is_windows_directory_link(link_path) and not link_path.is_symlink():
            os.rmdir(link_path)
        elif not link_path.is_dir() or link_path.is_symlink():
            link_path.unlink()
        else:
            raise SkillConflictError(f"目标入口是普通目录，拒绝删除: {link_path}")

        if db_commit_cb is not None:
            db_commit_cb()
    except BaseException:
        if old_target is not None:
            if not link_path.exists() and not is_link_entry(link_path):
                try:
                    atomic_create_or_replace_link(Path(old_target), link_path)
                except Exception as rollback_exc:
                    logging.error(f"移除操作失败后恢复原链接失败: {rollback_exc}")
            else:
                logging.error(
                    f"移除操作失败后回滚跳过: 入口 {link_path} 已存在外部或并发内容，拒绝覆盖"
                )
        raise
