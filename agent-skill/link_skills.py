#!/usr/bin/env python3
import argparse
import copy
import json
import logging
import ntpath
import os
import platform
import shutil
import stat
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_id, canonicalize_agent_list, canonicalize_agent_mapping
from src.command_runner import run_command
from src.git_remote_probe import probe_git_remote_state
from src.json_config import get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import (
    SkillPolicyMatrixError,
    is_agent_installed,
    repo_skill_matrix_mode,
    resolve_repo_skill_agent_overrides,
    validate_repo_new_skill_default,
    validate_repo_skill_matrix,
)
from src.runtime_logging import configure_logging
from src.skill_bindings import (
    SKILL_BINDINGS_POLICY_KEY,
    SkillBindingError,
    SkillConflictError,
    SkillBindingConflictError,
    SkillSharedTargetError,
    atomic_create_or_replace_link,
    check_not_external_conflict,
    check_shared_agent_target,
    get_binding,
    get_skill_versions_dir,
    is_managed_skill_link,
    load_skill_bindings,
    remove_binding,
    safe_remove_skill_link,
    safe_update_skill_link,
    save_skill_bindings,
    set_binding,
    validate_path_identifier,
)
from src.skill_versions import SkillVersionError, snapshot_skill, verify_skill_version
from src.storage.db import connect_ready_db, get_data_dir, get_db_path, is_sqlite_active

configure_logging()

MANAGED_REPO_DIRNAME = "repositories"
OWNER_MARKER = ".agent-manager-owned"
SKILL_FILE = "SKILL.md"
EXCLUDED_SKILL_DIRS = {".git", "node_modules", "__pycache__", "venv", ".venv"}


class DuplicateSkillNameError(ValueError):
    def __init__(self, conflicts):
        normalized_conflicts = {}
        for skill_name in sorted(conflicts):
            unique_paths = []
            for path in conflicts[skill_name]:
                path_obj = Path(path)
                if path_obj not in unique_paths:
                    unique_paths.append(path_obj)
            normalized_conflicts[skill_name] = tuple(unique_paths)
        self.conflicts = normalized_conflicts
        self.skill_names = tuple(normalized_conflicts.keys())
        self.skill_name = self.skill_names[0]
        details = []
        for skill_name, paths in self.conflicts.items():
            rendered_paths = " / ".join(str(path) for path in paths)
            details.append(f"{skill_name} ({rendered_paths})")
        super().__init__(f"发现重复 skill name: {'; '.join(details)}")


class SkillPathSelectionError(ValueError):
    pass


class SkillAgentScopeError(ValueError):
    pass


def run_cmd(cmd, cwd=None, timeout=300):
    """Run a shell command and return the result."""
    try:
        result = run_command(
            cmd,
            cwd=cwd,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        return True, result.stdout
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT after {timeout} seconds: {cmd}"
    except subprocess.CalledProcessError as e:
        return False, e.stderr
    except FileNotFoundError as e:
        return False, f"Directory or executable not found: {e}"
    except Exception as e:
        return False, f"Unexpected error executing cmd: {e}"

def get_managed_repo_base(base_dir):
    return Path(base_dir).resolve() / MANAGED_REPO_DIRNAME

def write_owner_marker(repo_path, repo_name):
    repo_path = Path(repo_path)
    marker_path = repo_path / OWNER_MARKER
    marker_path.write_text(
        json.dumps({"managed_by": "agent-manager", "repository": repo_name}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    ensure_local_git_exclude(repo_path, OWNER_MARKER)


def ensure_local_git_exclude(repo_path, pattern):
    exclude_path = Path(repo_path) / ".git" / "info" / "exclude"
    if not exclude_path.parent.is_dir():
        return
    try:
        existing = exclude_path.read_text(encoding="utf-8") if exclude_path.exists() else ""
    except OSError as exc:
        logging.warning(f"[{repo_path.name}] 读取本地 git ignore 失败: {exc}")
        return
    if pattern in {line.strip() for line in existing.splitlines()}:
        return
    try:
        with exclude_path.open("a", encoding="utf-8") as handle:
            if existing and not existing.endswith("\n"):
                handle.write("\n")
            handle.write(f"{pattern}\n")
    except OSError as exc:
        logging.warning(f"[{repo_path.name}] 写入本地 git ignore 失败: {exc}")

def is_manager_owned_repo(repo_path):
    return (Path(repo_path) / OWNER_MARKER).is_file()

def resolve_git_commit(repo_path, revision):
    ok, out = run_cmd(["git", "rev-parse", "--verify", f"{revision}^{{commit}}"], cwd=repo_path)
    return out.strip() if ok else None

def resolve_requested_revision(config):
    if config.get("commit"):
        return "commit", str(config["commit"])
    if config.get("ref"):
        return "ref", str(config["ref"])
    return None, None

def resolve_remote_default_branch(repo_path):
    ok, out = run_cmd(["git", "symbolic-ref", "--short", "refs/remotes/origin/HEAD"], cwd=repo_path)
    if ok:
        ref_name = out.strip()
        if ref_name.startswith("origin/"):
            return ref_name.removeprefix("origin/")
    for candidate in ("main", "master"):
        if resolve_git_commit(repo_path, f"origin/{candidate}") is not None:
            return candidate
    return None

def ensure_repo_version(repo_path, config, dry_run=False, offline=False):
    revision_kind, revision = resolve_requested_revision(config)
    if not revision:
        if not dry_run and config.get("auto_update", True) and repo_head_is_detached(repo_path):
            default_branch = resolve_remote_default_branch(repo_path)
            if not default_branch:
                logging.info(f"[{repo_path.name}] 未固定版本且仓库处于 detached HEAD，但无法解析远端默认分支；保留当前状态。")
                return True
            success, output = run_cmd(["git", "checkout", "-B", default_branch, f"origin/{default_branch}"], cwd=repo_path)
            if not success:
                logging.error(f"[{repo_path.name}] 切回默认分支 {default_branch} 失败: {output}")
                return False
            logging.info(f"[{repo_path.name}] 已切回默认分支 {default_branch} 并对齐最新远端状态。")
        return True
    if dry_run:
        logging.info(f"[{repo_path.name}] (预览: 计划检出 {revision_kind} {revision})")
        return True

    desired_commit = resolve_git_commit(repo_path, revision)
    if desired_commit is None:
        if offline:
            logging.error(f"[{repo_path.name}] 离线模式且本地缺少 {revision_kind} {revision}。")
            return False
        success, output = run_cmd(["git", "fetch", "--tags", "origin"], cwd=repo_path)
        if not success:
            logging.error(f"[{repo_path.name}] 拉取 {revision_kind} {revision} 失败: {output}")
            return False
        desired_commit = resolve_git_commit(repo_path, revision)
        if desired_commit is None:
            logging.error(f"[{repo_path.name}] 无法解析 {revision_kind} {revision}。")
            return False

    current_commit = resolve_git_commit(repo_path, "HEAD")
    if current_commit == desired_commit:
        logging.info(f"[{repo_path.name}] 已对齐到 {revision_kind} {revision}。")
        return True

    success, output = run_cmd(["git", "checkout", "--detach", desired_commit], cwd=repo_path)
    if not success:
        logging.error(f"[{repo_path.name}] 检出 {revision_kind} {revision} 失败: {output}")
        return False
    logging.info(f"[{repo_path.name}] 已检出 {revision_kind} {revision}。")
    return True

def repo_head_is_detached(repo_path):
    success, _ = run_cmd(["git", "symbolic-ref", "-q", "HEAD"], cwd=repo_path)
    return not success


def read_skill_name(skill_md_path):
    try:
        content = Path(skill_md_path).read_text(encoding="utf-8")
    except OSError as exc:
        logging.error(f"[{skill_md_path}] 读取失败: {exc}")
        return None

    lines = content.splitlines()
    if not lines or lines[0].strip() != "---":
        logging.error(f"[{skill_md_path}] 缺少 frontmatter name，跳过。")
        return None

    for line in lines[1:]:
        stripped = line.strip()
        if stripped == "---":
            break
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.strip() == "name":
            skill_name = value.strip().strip("'\"")
            if skill_name:
                return skill_name
            break

    logging.error(f"[{skill_md_path}] 缺少 frontmatter name，跳过。")
    return None

def install_dependencies_and_build(repo_path, config, dry_run=False, offline=False):
    """Install dependencies and run setup based on the repository contents."""
    if dry_run: return True
    if offline:
        logging.info(f"[{repo_path.name}] 离线模式，跳过依赖安装与初始化。")
        return True
    success_all = True
    setup_cmd = config.get("setup")
    if setup_cmd:
        logging.info(f"[{repo_path.name}] 执行自定义初始化...")
        ok, out = run_cmd(setup_cmd, cwd=repo_path)
        if not ok: logging.error(f"[{repo_path.name}] 初始化失败: {out}"); success_all = False

    if (repo_path / "package.json").exists():
        logging.info(f"[{repo_path.name}] 处理 Node.js 依赖...")
        if (repo_path / "yarn.lock").exists():
            pkg_mgr = "yarn"
            cmd = "yarn install"
        elif (repo_path / "pnpm-lock.yaml").exists():
            pkg_mgr = "pnpm"
            cmd = "pnpm install"
        else:
            pkg_mgr = "npm"
            cmd = "npm install"
        ok, out = run_cmd(cmd, cwd=repo_path)
        if ok:
            with open(repo_path / "package.json", 'r', encoding='utf-8') as f:
                try:
                    pkg_data = json.load(f)
                    if "scripts" in pkg_data and "build" in pkg_data["scripts"]:
                        build_ok, build_out = run_cmd([pkg_mgr, "run", "build"], cwd=repo_path)
                        if not build_ok:
                            logging.error(f"[{repo_path.name}] build 失败: {build_out}")
                            success_all = False
                except Exception:
                    pass
        else: logging.error(f"[{repo_path.name}] 依赖安装失败: {out}"); success_all = False
            
    elif (repo_path / "requirements.txt").exists() or (repo_path / "pyproject.toml").exists():
        logging.info(f"[{repo_path.name}] 处理 Python 依赖...")
        pip_cmd = f"{sys.executable} -m pip install"
        cmd = pip_cmd + " -e ." if (repo_path / "setup.py").exists() or (repo_path / "pyproject.toml").exists() else pip_cmd + " -r requirements.txt"
        ok, out = run_cmd(cmd, cwd=repo_path)
        if not ok: logging.error(f"[{repo_path.name}] 依赖安装失败: {out}"); success_all = False
    return success_all

def clone_or_update_repo(name, url, base_dir, dry_run=False, offline=False, auto_update=True, force_refresh=False):
    """Clone or update a Git repository."""
    target_dir = Path(base_dir) / name
    if target_dir.exists() and (target_dir / ".git").exists():
        if dry_run:
            logging.info(f"[{name}] (预览: 计划检查托管仓库状态)")
        elif offline:
            logging.info(f"[{name}] 离线模式，跳过仓库更新。")
        elif not auto_update:
            logging.info(f"[{name}] auto_update 关闭，跳过仓库更新。")
        else:
            probe_state = probe_git_remote_state(run_cmd, target_dir)
            update_cmd = ["git", "pull", "--ff-only"]
            if repo_head_is_detached(target_dir):
                if (
                    probe_state["status"] == "latest"
                    and probe_state.get("local_remote_head") == probe_state.get("remote_head")
                ):
                    logging.info(f"[{name}] 内容已最新。")
                    return target_dir, True
                update_cmd = ["git", "fetch", "--tags", "origin"]
            elif probe_state["status"] == "latest":
                logging.info(f"[{name}] 内容已最新。")
                return target_dir, True
            elif probe_state["status"] == "unknown" and not force_refresh:
                logging.warning(f"[{name}] 更新探测失败，默认跳过远端刷新；如需强制刷新请使用 --force-refresh。")
                return target_dir, True
            elif probe_state["status"] == "unknown":
                logging.warning(f"[{name}] 更新探测失败，但已显式开启强制刷新，继续刷新远端状态。")
            success, output = run_cmd(update_cmd, cwd=target_dir)
            if success: logging.info(f"[{name}] 更新成功。")
            else: logging.error(f"[{name}] 更新失败: {output}")
            return target_dir, success
        return target_dir, True
    if target_dir.exists():
        logging.error(f"[{name}] 目录存在但不是 git 仓库，无法继续管理。")
        return target_dir, False
    logging.info(f"[{name}] 仓库不存在，准备下载到托管缓存...")
    if dry_run:
        logging.info(f"[{name}] (预览: 离线/在线均仅展示计划，不实际下载)")
        return target_dir, True
    if offline:
        logging.error(f"[{name}] 离线模式且本地仓库缺失，无法下载。")
        return target_dir, False
    success, output = run_cmd(["git", "clone", url, str(target_dir)])
    if success:
        logging.info(f"[{name}] 下载成功。")
        write_owner_marker(target_dir, name)
    else:
        logging.error(f"[{name}] 下载失败: {output}")
    return target_dir, success

def find_skills_recursively(search_path):
    found_skills = {}
    duplicate_conflicts = {}
    search_path = Path(search_path).resolve()
    if not search_path.exists():
        return found_skills
    for root, dirs, files in os.walk(search_path):
        dirs[:] = sorted(d for d in dirs if d not in EXCLUDED_SKILL_DIRS)
        if SKILL_FILE in files:
            skill_dir = Path(root)
            skill_name = read_skill_name(skill_dir / SKILL_FILE)
            if not skill_name:
                continue
            existing_skill_dir = found_skills.get(skill_name)
            if existing_skill_dir is not None:
                duplicate_paths = duplicate_conflicts.setdefault(skill_name, [existing_skill_dir])
                duplicate_paths.append(skill_dir)
                continue
            found_skills[skill_name] = skill_dir
    if duplicate_conflicts:
        raise DuplicateSkillNameError(duplicate_conflicts)
    return found_skills


def resolve_skill_search_paths(repo_path, config):
    repo_path = Path(repo_path).resolve()
    raw_paths = config.get("skill_paths")
    if raw_paths is None:
        return [repo_path]
    if not isinstance(raw_paths, list) or not raw_paths:
        raise SkillPathSelectionError("skill_paths 必须是非空数组。")

    search_paths = []
    for raw_path in raw_paths:
        if not isinstance(raw_path, str) or not raw_path.strip():
            raise SkillPathSelectionError("skill_paths 中的每一项都必须是非空字符串。")
        search_path = (repo_path / raw_path).resolve(strict=False)
        if search_path != repo_path and repo_path not in search_path.parents:
            raise SkillPathSelectionError(f"skill_paths 包含越界路径: {raw_path}")
        if not search_path.exists():
            raise SkillPathSelectionError(f"skill_paths 指向的目录不存在: {raw_path}")
        if not search_path.is_dir():
            raise SkillPathSelectionError(f"skill_paths 指向的路径不是目录: {raw_path}")
        search_paths.append(search_path)
    return search_paths


def find_configured_skills(repo_path, config):
    found_skills = {}
    duplicate_conflicts = {}
    for search_path in resolve_skill_search_paths(repo_path, config):
        try:
            skills = find_skills_recursively(search_path)
        except DuplicateSkillNameError as exc:
            for skill_name, paths in exc.conflicts.items():
                duplicate_paths = duplicate_conflicts.setdefault(skill_name, [])
                duplicate_paths.extend(paths)
            continue
        for skill_name, skill_dir in skills.items():
            existing_skill_dir = found_skills.get(skill_name)
            if existing_skill_dir is not None:
                duplicate_paths = duplicate_conflicts.setdefault(skill_name, [existing_skill_dir])
                duplicate_paths.append(skill_dir)
                continue
            found_skills[skill_name] = skill_dir
    if duplicate_conflicts:
        raise DuplicateSkillNameError(duplicate_conflicts)
    return found_skills


def validate_repo_agent_scope(repo_name, config, skills):
    if config.get("agents") is None and len(skills) > 1:
        raise SkillAgentScopeError(f"{repo_name}: 多 skill 仓库必须显式声明 agents，禁止使用 agents: null")


def validate_repo_skill_count(repo_name, config):
    skill_count = config.get("skill_count")
    if skill_count is None:
        return
    if not isinstance(skill_count, int) or isinstance(skill_count, bool) or skill_count <= 0:
        raise ValueError(f"{repo_name}: skill_count 必须是正整数")


def validate_repo_disabled(repo_name, config):
    # 本机 per-skill 禁用名单，来自 policy（manager.json / manager.local.json 的 skills.<repo>.disabled）。
    # 形状 fail-close：缺省表示不禁用；给了就必须是非空字符串数组。
    raw = config.get("disabled")
    if raw is None:
        return
    if not isinstance(raw, list) or any(
        not isinstance(item, str) or not item.strip() for item in raw
    ):
        raise ValueError(f"{repo_name}: skills.{repo_name}.disabled 必须是非空字符串数组")


def partition_disabled_skills(repo_name, config, skills):
    """按本机策略 config.disabled 过滤 skills，返回 (保留的 skills, 实际被禁用的名字集)。
    形状由 validate_repo_disabled 保证；含未知/已移除的名字仅告警忽略（不阻断同步）。"""
    requested = {name.strip() for name in (config.get("disabled") or [])}
    if not requested:
        return skills, set()
    unknown = requested - set(skills)
    if unknown:
        logging.warning(
            f"[{repo_name}] disabled 含未知或已移除的 skill（忽略）: {', '.join(sorted(unknown))}"
        )
    active = requested & set(skills)
    kept = {name: path for name, path in skills.items() if name not in active}
    return kept, active

def create_link(source, link, dry_run=False, managed_bases=None):
    if dry_run: return
    source, link = Path(source).resolve(), Path(link)
    link_parent = link.parent.resolve()
    if managed_bases is not None:
        if link.exists() or is_link_entry(link):
            check_not_external_conflict(link, managed_bases)
    else:
        if link.exists() or is_link_entry(link):
            try:
                remove_path(link)
            except OSError: return
    try:
        atomic_create_or_replace_link(source, link)
        logging.info(f"链接: {os.path.relpath(source, link_parent)} -> {link.name}")
    except Exception as e:
        logging.error(f"链接失败: {e}")
        raise

def is_windows_directory_link(path):
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


def is_link_entry(path):
    path = Path(path)
    return path.is_symlink() or is_windows_directory_link(path)


def _is_windows_path(path_str: str) -> bool:
    if len(path_str) >= 2 and path_str[0].isalpha() and path_str[1] == ":":
        return True
    if path_str.startswith(("\\\\", "//")):
        return True
    return False


def normalize_windows_device_path(path):
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


def normalize_path_for_compare(path):
    path_str = normalize_windows_device_path(path)
    if _is_windows_path(path_str):
        return "windows", ntpath.normcase(ntpath.normpath(path_str))
    return "posix", str(Path(path_str).expanduser().resolve(strict=False))


def remove_path(path):
    path = Path(path)
    if is_windows_directory_link(path) and not path.is_symlink():
        os.rmdir(path)
        return
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
        return
    path.unlink()


def resolve_link_target(link_path) -> str:
    link_path = Path(link_path)
    if not is_link_entry(link_path):
        raise OSError(f"Not a link entry: {link_path}")
    try:
        raw_target = normalize_windows_device_path(os.readlink(link_path))
        if ntpath.isabs(raw_target):
            return ntpath.normpath(raw_target)
        raw_target_path = Path(raw_target).expanduser()
        resolved_target = raw_target_path if raw_target_path.is_absolute() else (link_path.parent / raw_target_path)
        return str(resolved_target.resolve(strict=False))
    except OSError:
        if is_windows_directory_link(link_path):
            return str(link_path.resolve(strict=False))
        raise


def is_path_within_base(path, base_path):
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


def link_points_within_base(link_path, base_path):
    try:
        return is_path_within_base(resolve_link_target(link_path), base_path)
    except OSError:
        return False


def link_points_within_any_base(link_path, base_paths):
    for base_path in base_paths or ():
        if link_points_within_base(link_path, base_path):
            return True
    return False


def scrub_dead_links(
    agents_config,
    base_dir,
    dry_run=False,
    managed_skill_names_by_agent=None,
    planned_removed_base_paths=None,
    store_dir=None,
):
    base_dir = Path(base_dir).resolve()
    managed_skill_names_by_agent = managed_skill_names_by_agent or {}
    planned_removed_base_paths = tuple(
        Path(base_path).resolve(strict=False) for base_path in (planned_removed_base_paths or ())
    )
    for agent_name, target_path in agents_config.items():
        agent_managed_skill_names = set(managed_skill_names_by_agent.get(agent_name, ()))
        target_base = Path(target_path).expanduser()
        if not target_base.exists() or not target_base.is_dir(): continue
        for item in target_base.iterdir():
            if not is_link_entry(item):
                continue
            points_to_planned_removed_base = link_points_within_any_base(item, planned_removed_base_paths)
            if item.exists() and not points_to_planned_removed_base:
                continue
            if item.name not in agent_managed_skill_names and not points_to_planned_removed_base:
                continue
            logging.info(f"[AGENT: {agent_name}] 移除死链: {item.name}")
            if not dry_run:
                remove_path(item)

def is_active_skill_target(agent_name, target_path, agent_defs):
    target_base = Path(target_path).expanduser()
    if target_base.exists() or target_base.parent.exists():
        return True
    agent_def = agent_defs.get(agent_name)
    if agent_def is not None:
        return is_agent_installed(agent_name, agent_def)
    return canonicalize_agent_id(agent_name) == "codex"


def resolve_repo_link_targets(active_targets, all_targets, allowed_agents):
    if allowed_agents is None:
        return dict(active_targets)

    repo_targets = {k: v for k, v in active_targets.items() if k in allowed_agents}
    if "codex" in allowed_agents and "codex" in all_targets:
        repo_targets.setdefault("codex", all_targets["codex"])
    return repo_targets


def prune_orphan_repos(base_dir, managed_names, dry_run=False, force_yes=False):
    base_path = Path(base_dir).resolve()
    excluded = {".gitignore", "__pycache__"}
    planned_removed_paths = []
    if not base_path.exists():
        return planned_removed_paths
    for item in base_path.iterdir():
        if not item.is_dir() or item.name in managed_names or item.name in excluded:
            continue
        if not is_manager_owned_repo(item):
            logging.info(f"跳过非托管目录: {item.name}")
            continue
        logging.warning(f"发现孤儿仓库: {item.name}")
        item_path = item.resolve(strict=False)
        if dry_run:
            if force_yes:
                planned_removed_paths.append(item_path)
            continue
        if force_yes or (sys.stdin.isatty() and input(f"删除 {item.name}? (y/n): ").lower() == 'y'):
            planned_removed_paths.append(item_path)
            shutil.rmtree(item); logging.info(f"已清理: {item.name}")
    return planned_removed_paths


def merge_allowed_agents(existing_agents, new_agents):
    if existing_agents is None or new_agents is None:
        return None
    return sorted(set(existing_agents) | set(new_agents))


def record_duplicate_link_target(duplicate_link_targets, skill_name, allowed_agents):
    if skill_name not in duplicate_link_targets:
        duplicate_link_targets[skill_name] = None if allowed_agents is None else list(allowed_agents)
        return
    duplicate_link_targets[skill_name] = merge_allowed_agents(
        duplicate_link_targets[skill_name],
        allowed_agents,
    )


def collect_repo_skill_names_for_cleanup(repo_path, config=None):
    repo_path = Path(repo_path).resolve()
    search_paths = [repo_path]
    if config is not None:
        try:
            search_paths = resolve_skill_search_paths(repo_path, config)
        except SkillPathSelectionError:
            search_paths = [repo_path]
    skill_names = set()
    for search_path in search_paths:
        if not search_path.exists():
            continue
        for root, dirs, files in os.walk(search_path):
            dirs[:] = sorted(d for d in dirs if d not in EXCLUDED_SKILL_DIRS)
            if SKILL_FILE not in files:
                continue
            skill_name = read_skill_name(Path(root) / SKILL_FILE)
            if skill_name:
                skill_names.add(skill_name)
    return skill_names


def remove_repo_skill_links(
    repo_path,
    targets,
    allowed_agents=None,
    dry_run=False,
    skill_names=None,
    store_dir=None,
    bindings=None,
    repo_name=None,
    db_path=None,
):
    repo_path = Path(repo_path).resolve()
    managed_skill_names = set(skill_names) if skill_names is not None else None
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    for agent_name, target_path in active_targets.items():
        target_base = Path(target_path).expanduser()
        if not target_base.exists() or not target_base.is_dir():
            continue
        for item in target_base.iterdir():
            if not is_link_entry(item):
                continue
            if managed_skill_names is not None and item.name not in managed_skill_names:
                continue
            is_match = False
            if link_points_within_base(item, repo_path):
                is_match = True
            if not is_match:
                continue
            logging.error(f"[AGENT: {agent_name}] 移除来自失效仓库的 skill 链接: {item.name}")
            if not dry_run:
                managed_bases = [b for b in (repo_path, store_dir) if b is not None]
                safe_remove_skill_link(item, None, managed_bases=managed_bases)


def remove_skill_links(skill_name, targets, allowed_agents=None, dry_run=False, managed_bases=None):
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    for agent_name, target_path in active_targets.items():
        link_path = Path(target_path).expanduser() / skill_name
        if not (link_path.exists() or is_link_entry(link_path)):
            continue
        if managed_bases and not is_managed_skill_link(link_path, managed_bases):
            continue
        logging.error(f"[AGENT: {agent_name}] 移除冲突 skill 链接: {skill_name}")
        if dry_run:
            continue
        remove_path(link_path)


def remove_matrix_disabled_skill_links(
    repo_path,
    skill_name,
    targets,
    agent_names,
    dry_run=False,
    store_dir=None,
    bindings=None,
    repo_name=None,
    db_path=None,
):
    """矩阵停用收敛：只解除指向本仓库缓存或受管版本的托管链接，不碰用户手工放置的内容。"""
    repo_path = Path(repo_path).resolve()
    for agent_name in agent_names:
        target_path = targets.get(agent_name)
        if target_path is None:
            continue
        link_path = Path(target_path).expanduser() / skill_name
        if not is_link_entry(link_path):
            continue
        is_match = False
        if link_points_within_base(link_path, repo_path):
            is_match = True
        elif store_dir is not None and bindings is not None and repo_name is not None:
            assignment = get_binding(bindings, repo_name, skill_name, agent_name)
            if assignment and assignment.get("version"):
                expected_content = (Path(store_dir) / assignment["version"] / "content").resolve()
                try:
                    resolved = Path(resolve_link_target(link_path)).resolve()
                    if resolved == expected_content:
                        is_match = True
                except OSError:
                    pass
        if not is_match:
            continue
        logging.info(f"[AGENT: {agent_name}] 策略矩阵停用 skill（移除托管链接）: {skill_name}")
        if not dry_run:
            managed_bases = [b for b in (repo_path, store_dir) if b is not None]
            if db_path and bindings is not None and repo_name:
                old_assignment = copy.deepcopy(get_binding(bindings, repo_name, skill_name, agent_name))
                def make_matrix_disable_cb(ag, old_b):
                    def _commit():
                        bindings_backup = copy.deepcopy(bindings)
                        try:
                            assignment = get_binding(bindings, repo_name, skill_name, ag)
                            if assignment and assignment.get("enabled"):
                                assignment["enabled"] = False
                                save_skill_bindings(
                                    db_path,
                                    bindings,
                                    action="disable",
                                    target=f"{repo_name}/{skill_name}@{ag}",
                                    collection=repo_name,
                                    skill=skill_name,
                                    agent=ag,
                                    expected_entry=old_b,
                                )
                        except BaseException:
                            bindings.clear()
                            bindings.update(bindings_backup)
                            raise
                    return _commit
                safe_remove_skill_link(link_path, make_matrix_disable_cb(agent_name, old_assignment), managed_bases=managed_bases)
            else:
                remove_path(link_path)

def manage_links(
    skill_name,
    source_path,
    targets,
    allowed_agents,
    is_enabled=True,
    dry_run=False,
    store_dir=None,
    bindings=None,
    repo_name=None,
    db_path=None,
    repo_base_dir=None,
):
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    managed_bases = [b for b in (repo_base_dir, store_dir) if b is not None]

    for agent_name, target_path in active_targets.items():
        target_base = Path(target_path).expanduser()
        link_path = target_base / skill_name
        if agent_name != "codex" and not target_base.parent.exists() and is_enabled:
            continue

        if store_dir is not None and bindings is not None and repo_name is not None:
            # SQLite / 不可变版本分支
            assignment = get_binding(bindings, repo_name, skill_name, agent_name)
            if assignment is not None:
                if assignment.get("assigned") is False:
                    # 显式移除过，普通 sync 不复活
                    if (link_path.exists() or is_link_entry(link_path)) and is_managed_skill_link(link_path, managed_bases):
                        if not dry_run:
                            safe_remove_skill_link(link_path, None, managed_bases=managed_bases)
                    continue

                if assignment.get("enabled") is False:
                    # 单 Agent 策略矩阵停用（保留 A开/B关 选择）：撤入口，不复活
                    if (link_path.exists() or is_link_entry(link_path)) and is_managed_skill_link(link_path, managed_bases):
                        if not dry_run:
                            safe_remove_skill_link(link_path, None, managed_bases=managed_bases)
                    continue

                if not is_enabled:
                    # 集合全局或全局 skill 禁用：撤入口，不得更改各 Agent 的 enabled 状态以防丢失选择
                    if (link_path.exists() or is_link_entry(link_path)) and is_managed_skill_link(link_path, managed_bases):
                        if not dry_run:
                            safe_remove_skill_link(link_path, None, managed_bases=managed_bases)
                    continue

                # 启用的已有分配：保持已固定分配版本并链接
                version_id = assignment["version"]
                content_path = verify_skill_version(store_dir, version_id)
                if dry_run:
                    logging.info(f"[AGENT: {agent_name}] (预览: 计划保持版本 {skill_name} -> {version_id[:8]})")
                    continue

                safe_update_skill_link(content_path, link_path, None, managed_bases)
            else:
                # SQLite 模式下新资源不自动分配，必须显式使用 --add-assignment
                continue
        else:
            # 旧式非 SQLite 分发行为保持
            if not is_enabled:
                if (link_path.exists() or is_link_entry(link_path)) and not dry_run:
                    remove_path(link_path)
                continue
            if not target_base.exists() and not dry_run:
                target_base.mkdir(parents=True, exist_ok=True)
            if dry_run:
                logging.info(f"[AGENT: {agent_name}] (预览: 计划加载 {skill_name})")
                continue
            create_link(source_path, link_path, dry_run)


def collect_current_managed_skill_names_by_agent(pending_links, duplicate_skill_names):
    # pending_links 的第二元素是 per-skill 目标映射 {skill_name: {agent: path}}；
    # 旧式定义下每个 skill 的映射都是同一份 repo_targets（升格等价）。
    managed_skill_names_by_agent = {}
    for _repo_name, skill_targets_by_name, skills in pending_links:
        for skill_name in skills:
            if skill_name in duplicate_skill_names:
                continue
            for agent_name in skill_targets_by_name[skill_name]:
                managed_skill_names_by_agent.setdefault(agent_name, set()).add(skill_name)
    return managed_skill_names_by_agent

def main():
    parser = argparse.ArgumentParser(description="Agent Skill 管理工具 (策略对齐)")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略配置")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "skills.json"), help="定义文件")
    parser.add_argument("--base-dir", default=str(Path(__file__).resolve().parent), help="目录")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过远程仓库更新与依赖安装)")
    parser.add_argument("-f", "--force-refresh", action="store_true", help="更新探测失败时仍强制执行远端刷新")
    parser.add_argument("--yes", action="store_true", help="自动清理")
    parser.add_argument("--only", action="append", default=[], help="仅针对指定 skill 或 collection（可重复；支持 skill:<id> 或 <id>）")
    parser.add_argument("--agent", help="仅针对指定 agent（如 codex）")
    action_group = parser.add_mutually_exclusive_group()
    action_group.add_argument("--update-version", action="store_true", help="仅更新指定 Agent 已有 assignment 的 skill 版本")
    action_group.add_argument("--add-assignment", action="store_true", help="显式为指定 Agent 新建/重新添加 skill 分配")
    action_group.add_argument("--remove-assignment", action="store_true", help="删除指定 Agent 的 skill 分配入口并记录 assigned=false")
    action_group.add_argument("--enable-assignment", action="store_true", help="显式为指定 Agent 重新启用已存在的 skill 分配入口")
    action_group.add_argument("--disable-assignment", action="store_true", help="显式为指定 Agent 停用已存在的 skill 分配入口并撤下链接")
    args = parser.parse_args()
    
    try:
        policy_root = load_json_object_file(args.config, strict=True, missing_ok=False)
        full_def = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        policy_cfg = get_typed_mapping_section(policy_root, "skills", Path(args.config).name, (dict,), "对象")
        repositories = get_typed_mapping_section(full_def, "repositories", Path(args.def_file).name, (dict,), "对象")
        agent_targets = get_typed_mapping_section(full_def, "agents", Path(args.def_file).name, (str, dict), "字符串或对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)

    # Separation of concerns: merge policy
    # 定义覆盖层：merged 策略里的 skills.repositories 片段深合并到 skills.json
    # 之上（tombstone `_delete: true` 移除条目；形状 fail-close；未知键告警不拒绝）。
    from src.policy_layers import apply_definition_overlays, overlay_definitions
    try:
        repositories, overlay_warnings = apply_definition_overlays(
            repositories, overlay_definitions(policy_root, "skills"), domain="skills"
        )
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)
    for overlay_warning in overlay_warnings:
        logging.warning(overlay_warning)
    REPOS = {name: dict(config) for name, config in repositories.items()}
    AGENT_TARGETS = canonicalize_agent_mapping(
        {k: (v if isinstance(v, str) else v.get("path")) for k, v in agent_targets.items()}
    )

    # Distinguish the full target set (for cleaning up old links) from the active targets (for creating new links)
    agent_defs = load_json_object_file(REPO_ROOT / "config" / "agents.json", strict=False, missing_ok=True) or {}
    agent_defs, _ = apply_definition_overlays(agent_defs, overlay_definitions(policy_root, "agents"), domain="agents")
    installed_agents = {
        name for name, target_path in AGENT_TARGETS.items() if is_active_skill_target(name, target_path, agent_defs)
    }
    skipped = [name for name in AGENT_TARGETS if name not in installed_agents]
    for name in skipped:
        if name == "codex":
            logging.info(f"[AGENT: {name}] 不参与全局 skill 新建；仅在仓库显式声明 agents 时接收 skill 链接。")
            continue
        logging.info(f"[AGENT: {name}] 未安装，跳过新建 skill 链接（保留清理能力）。")
    ALL_AGENT_TARGETS = AGENT_TARGETS  # full set, used for cleaning up old links
    AGENT_TARGETS = {k: v for k, v in AGENT_TARGETS.items() if k in installed_agents}  # active set, used for creating new links
    DECLARED_AGENT_TARGETS = dict(ALL_AGENT_TARGETS)

    if args.agent:
        if args.agent not in ALL_AGENT_TARGETS:
            logging.error(f"未知 agent: {args.agent}")
            sys.exit(2)
        try:
            check_shared_agent_target(args.agent, DECLARED_AGENT_TARGETS)
        except SkillSharedTargetError as exc:
            logging.error(str(exc))
            sys.exit(2)
        ALL_AGENT_TARGETS = {args.agent: ALL_AGENT_TARGETS[args.agent]}
        AGENT_TARGETS = {k: v for k, v in AGENT_TARGETS.items() if k == args.agent}

    data_dir = get_data_dir()
    db_path = get_db_path(data_dir)
    sqlite_active = db_path.is_file()
    store_dir = get_skill_versions_dir(data_dir) if sqlite_active else None

    base_dir = Path(args.base_dir).resolve()
    repo_base_dir = get_managed_repo_base(base_dir)

    is_explicit_action = bool(
        args.update_version
        or args.add_assignment
        or args.remove_assignment
        or args.enable_assignment
        or args.disable_assignment
    )
    if is_explicit_action:
        if not sqlite_active:
            logging.error(f"新分配动作要求已初始化库 (manager.db)，未检测到数据库 ({db_path})，不能静默降级")
            sys.exit(2)
        try:
            conn = connect_ready_db(db_path, read_only=True)
            conn.close()
        except Exception as exc:
            logging.error(f"数据库未就绪: {exc}")
            sys.exit(2)

        if not args.agent:
            logging.error("显式动作必须指定 --agent <agent>")
            sys.exit(2)
        if len(args.only) != 1:
            logging.error("显式动作需 --only 恰好一个 skill:<collection>/<skill>")
            sys.exit(2)
        item = args.only[0]
        if ":" in item and not item.startswith("skill:"):
            logging.error(f"跨域元素 scope 拒绝: {item}")
            sys.exit(2)
        norm = item[6:] if item.startswith("skill:") else item
        if "/" not in norm:
            logging.error(f"需 --only 恰好一个 skill:<collection>/<skill>，拒绝仅指定集合: {item}")
            sys.exit(2)
        action_collection, action_skill = norm.split("/", 1)
        if not action_collection or not action_skill:
            logging.error(f"非法 skill 路径: {item}")
            sys.exit(2)

        # 路径与标识符安全校验：拒绝 /、\、.、.. 等逃逸
        try:
            validate_path_identifier(action_collection, "集合名称")
            validate_path_identifier(action_skill, "Skill 名称")
            validate_path_identifier(args.agent, "Agent 名称")
        except SkillBindingError as exc:
            logging.error(str(exc))
            sys.exit(2)

        target_agent = args.agent
        target_base = Path(ALL_AGENT_TARGETS[target_agent]).expanduser()
        link_path = target_base / action_skill
        try:
            bindings = load_skill_bindings(db_path)
        except SkillBindingError as exc:
            logging.error(f"加载技能绑定失败: {exc}")
            sys.exit(1)
        managed_bases = [b for b in (repo_base_dir, store_dir) if b is not None]

        if args.remove_assignment:
            # 显式 remove 必须能删除已从上游消失/集合已删除的已有 binding，不可先要求 repo 目录和当前 skill 存在
            assignment = get_binding(bindings, action_collection, action_skill, target_agent)
            check_not_external_conflict(link_path, managed_bases)
            if args.dry_run:
                logging.info(f"[AGENT: {target_agent}] (预览: 计划移除分配 {action_collection}/{action_skill})")
                sys.exit(0)

            old_assignment = copy.deepcopy(assignment)
            def commit_cb():
                bindings_backup = copy.deepcopy(bindings)
                try:
                    remove_binding(bindings, action_collection, action_skill, target_agent)
                    save_skill_bindings(
                        db_path,
                        bindings,
                        action="remove",
                        target=f"{action_collection}/{action_skill}@{target_agent}",
                        detail={"assigned": False},
                        collection=action_collection,
                        skill=action_skill,
                        agent=target_agent,
                        expected_entry=old_assignment,
                    )
                except BaseException:
                    bindings.clear()
                    bindings.update(bindings_backup)
                    raise

            # remove 也只能删当前 binding 自己的目标，不能删另一集合同名目标
            should_remove_link = False
            if is_link_entry(link_path) and assignment and assignment.get("version"):
                expected_content = (store_dir / assignment["version"] / "content").resolve()
                try:
                    if Path(resolve_link_target(link_path)).resolve() == expected_content:
                        should_remove_link = True
                except OSError:
                    pass

            if should_remove_link:
                safe_remove_skill_link(link_path, commit_cb, managed_bases=managed_bases)
            else:
                commit_cb()
            logging.info(f"[AGENT: {target_agent}] 已移除分配 {action_collection}/{action_skill}")
            sys.exit(0)

        if args.disable_assignment:
            assignment = get_binding(bindings, action_collection, action_skill, target_agent)
            if assignment is None or not assignment.get("assigned", False):
                logging.error(f"[{action_collection}/{action_skill}@{target_agent}] 不存在已有 assignment，拒绝 disable-assignment")
                sys.exit(2)
            check_not_external_conflict(link_path, managed_bases)
            if args.dry_run:
                logging.info(f"[AGENT: {target_agent}] (预览: 计划停用分配 {action_collection}/{action_skill})")
                sys.exit(0)

            old_assignment = copy.deepcopy(assignment)
            def commit_cb():
                bindings_backup = copy.deepcopy(bindings)
                try:
                    set_binding(bindings, action_collection, action_skill, target_agent, version=assignment["version"], enabled=False, assigned=True)
                    save_skill_bindings(
                        db_path,
                        bindings,
                        action="disable",
                        target=f"{action_collection}/{action_skill}@{target_agent}",
                        detail={"enabled": False},
                        collection=action_collection,
                        skill=action_skill,
                        agent=target_agent,
                        expected_entry=old_assignment,
                    )
                except BaseException:
                    bindings.clear()
                    bindings.update(bindings_backup)
                    raise

            should_remove_link = False
            if is_link_entry(link_path) and assignment.get("version"):
                expected_content = (store_dir / assignment["version"] / "content").resolve()
                try:
                    if Path(resolve_link_target(link_path)).resolve() == expected_content:
                        should_remove_link = True
                except OSError:
                    pass

            if should_remove_link:
                safe_remove_skill_link(link_path, commit_cb, managed_bases=managed_bases)
            else:
                commit_cb()
            logging.info(f"[AGENT: {target_agent}] 已停用分配 {action_collection}/{action_skill}")
            sys.exit(0)

        if args.enable_assignment:
            assignment = get_binding(bindings, action_collection, action_skill, target_agent)
            if assignment is None or not assignment.get("assigned", False):
                logging.error(f"[{action_collection}/{action_skill}@{target_agent}] 不存在已有 assignment，拒绝 enable-assignment")
                sys.exit(2)

            # 校验集合与 skill 是否被全局禁用
            config = REPOS.get(action_collection) or {}
            repo_policy = policy_cfg.get(action_collection) or {}
            if repo_policy.get("enabled") is False or config.get("enabled") is False:
                logging.error(f"[{action_collection}] 集合已被全局禁用，拒绝启用分配")
                sys.exit(2)

            disabled_skills = set(repo_policy.get("disabled") or config.get("disabled") or [])
            if action_skill in disabled_skills:
                logging.error(f"[{action_collection}/{action_skill}] 该 Skill 已被集合全局禁用，拒绝启用分配")
                sys.exit(2)

            # 校验目标路径是否已被其他集合的同名已分配 binding 占用
            for other_coll in bindings:
                if other_coll != action_collection:
                    other_b = get_binding(bindings, other_coll, action_skill, target_agent)
                    if other_b and other_b.get("assigned", True) and other_b.get("enabled", True):
                        logging.error(
                            f"目标 {target_agent}/{action_skill} 已被集合 {other_coll} 启用分配，拒绝跨集合覆盖"
                        )
                        sys.exit(1)

            # enable 恢复旧 version 而非追新源，源已消失也能恢复；dry-run 之前必须校验已选版本，坏快照不能预演成功
            version_id = assignment["version"]
            content_path = verify_skill_version(store_dir, version_id)

            check_not_external_conflict(link_path, managed_bases)
            if link_path.exists() or is_link_entry(link_path):
                try:
                    current_target = Path(resolve_link_target(link_path)).resolve()
                    if current_target != content_path:
                        logging.error(f"目标入口 {link_path} 实际指向不匹配路径 ({current_target})，拒绝启用覆盖并保留原有状态")
                        sys.exit(1)
                except OSError as exc:
                    logging.error(f"读取现有目标入口失败: {exc}")
                    sys.exit(1)

            if args.dry_run:
                logging.info(f"[AGENT: {target_agent}] (预览: 计划启用分配 {action_collection}/{action_skill})")
                sys.exit(0)

            old_assignment = copy.deepcopy(assignment)
            def commit_cb():
                bindings_backup = copy.deepcopy(bindings)
                try:
                    set_binding(bindings, action_collection, action_skill, target_agent, version=version_id, enabled=True, assigned=True)
                    save_skill_bindings(
                        db_path,
                        bindings,
                        action="enable",
                        target=f"{action_collection}/{action_skill}@{target_agent}",
                        detail={"enabled": True},
                        collection=action_collection,
                        skill=action_skill,
                        agent=target_agent,
                        expected_entry=old_assignment,
                    )
                except BaseException:
                    bindings.clear()
                    bindings.update(bindings_backup)
                    raise
            safe_update_skill_link(content_path, link_path, commit_cb, managed_bases)
            logging.info(f"[AGENT: {target_agent}] 已启用分配 {action_collection}/{action_skill} (恢复版本 {version_id[:8]})")
            sys.exit(0)

        # add/update 必须存在对应集合定义与源目录，并尊重集合/资源全局禁用
        if action_collection not in REPOS:
            logging.error(f"未知 skill 集合: {action_collection}")
            sys.exit(2)

        config = REPOS[action_collection]
        repo_policy = policy_cfg.get(action_collection) or {}
        if repo_policy.get("enabled") is False or config.get("enabled") is False:
            logging.error(f"[{action_collection}] 集合已被全局禁用，拒绝添加或更新分配")
            sys.exit(2)

        disabled_skills = set(repo_policy.get("disabled") or config.get("disabled") or [])
        if action_skill in disabled_skills:
            logging.error(f"[{action_collection}/{action_skill}] 该 Skill 已被集合全局禁用，拒绝添加或更新分配")
            sys.exit(2)

        repo_path = repo_base_dir / action_collection
        if not repo_path.exists():
            logging.error(f"集合目录不存在: {action_collection}")
            sys.exit(2)

        skills = find_configured_skills(repo_path, config)
        if action_skill not in skills:
            logging.error(f"[{action_collection}] 未知 skill: {action_skill}")
            sys.exit(2)

        skill_source_path = skills[action_skill]

        if args.update_version:
            assignment = get_binding(bindings, action_collection, action_skill, target_agent)
            if assignment is None or not assignment.get("assigned", False):
                logging.error(f"[{action_collection}/{action_skill}@{target_agent}] 不存在已有 assignment，拒绝 update-version")
                sys.exit(2)

            is_enabled = bool(assignment.get("enabled", True))
            if is_enabled:
                for other_coll in bindings:
                    if other_coll != action_collection:
                        other_b = get_binding(bindings, other_coll, action_skill, target_agent)
                        if other_b and other_b.get("assigned", True) and other_b.get("enabled", True):
                            logging.error(
                                f"目标 {target_agent}/{action_skill} 已被集合 {other_coll} 启用分配，拒绝跨集合覆盖"
                            )
                            sys.exit(1)

                check_not_external_conflict(link_path, managed_bases)
                old_version = assignment.get("version")
                if (link_path.exists() or is_link_entry(link_path)) and old_version:
                    expected_old_target = (store_dir / old_version / "content").resolve()
                    try:
                        current_target = Path(resolve_link_target(link_path)).resolve()
                        if current_target != expected_old_target:
                            logging.error(
                                f"目标入口 {link_path} 实际指向非本分配期望旧版本 ({current_target})，拒绝更新并保留原有状态"
                            )
                            sys.exit(1)
                    except OSError as exc:
                        logging.error(f"读取现有目标入口失败: {exc}")
                        sys.exit(1)
            else:
                # 绑定A禁用、B启用同名时：更新A只改A版本，不得碰B的入口与路径检查
                pass

            if args.dry_run:
                logging.info(f"[AGENT: {target_agent}] (预览: 计划更新分配 {action_collection}/{action_skill} 版本)")
                sys.exit(0)

            snap = snapshot_skill(skill_source_path, store_dir)
            new_version = snap["version_id"]
            content_path = snap["content_path"]
            old_assignment = copy.deepcopy(assignment)
            if is_enabled:
                def commit_cb():
                    bindings_backup = copy.deepcopy(bindings)
                    try:
                        set_binding(bindings, action_collection, action_skill, target_agent, version=new_version, enabled=True, assigned=True)
                        save_skill_bindings(
                            db_path,
                            bindings,
                            action="update",
                            target=f"{action_collection}/{action_skill}@{target_agent}",
                            detail={"version": new_version},
                            collection=action_collection,
                            skill=action_skill,
                            agent=target_agent,
                            expected_entry=old_assignment,
                        )
                    except BaseException:
                        bindings.clear()
                        bindings.update(bindings_backup)
                        raise
                safe_update_skill_link(content_path, link_path, commit_cb, managed_bases)
            else:
                bindings_backup = copy.deepcopy(bindings)
                try:
                    set_binding(bindings, action_collection, action_skill, target_agent, version=new_version, enabled=False, assigned=True)
                    save_skill_bindings(
                        db_path,
                        bindings,
                        action="update",
                        target=f"{action_collection}/{action_skill}@{target_agent}",
                        detail={"version": new_version},
                        collection=action_collection,
                        skill=action_skill,
                        agent=target_agent,
                        expected_entry=old_assignment,
                    )
                except BaseException:
                    bindings.clear()
                    bindings.update(bindings_backup)
                    raise
            logging.info(f"[AGENT: {target_agent}] 已更新分配 {action_collection}/{action_skill} 版本至 {new_version[:8]}")
            sys.exit(0)

        if args.add_assignment:
            assignment = get_binding(bindings, action_collection, action_skill, target_agent)
            if assignment is not None and assignment.get("assigned", True) is True:
                logging.error(
                    f"[{action_collection}/{action_skill}@{target_agent}] 已存在 assignment，拒绝重复添加。请使用 --update-version 或 --enable-assignment"
                )
                sys.exit(2)

            # 不同 collection 同名 skill 分配实际目标已有另一 binding 时拒绝
            for other_coll in bindings:
                if other_coll != action_collection:
                    other_b = get_binding(bindings, other_coll, action_skill, target_agent)
                    if other_b and other_b.get("assigned", True) and other_b.get("enabled", True):
                        logging.error(
                            f"目标 {target_agent}/{action_skill} 已被集合 {other_coll} 分配，拒绝跨集合覆盖"
                        )
                        sys.exit(1)

            check_not_external_conflict(link_path, managed_bases)
            if link_path.exists() or is_link_entry(link_path):
                logging.error(f"目标入口 {link_path} 已存在其他受管或残留链接，拒绝静默覆盖并保留原有状态")
                sys.exit(1)

            if args.dry_run:
                logging.info(f"[AGENT: {target_agent}] (预览: 计划新建分配 {action_collection}/{action_skill})")
                sys.exit(0)

            snap = snapshot_skill(skill_source_path, store_dir)
            version_id = snap["version_id"]
            content_path = snap["content_path"]
            old_assignment = copy.deepcopy(assignment)
            def commit_cb():
                bindings_backup = copy.deepcopy(bindings)
                try:
                    set_binding(bindings, action_collection, action_skill, target_agent, version=version_id, enabled=True, assigned=True)
                    save_skill_bindings(
                        db_path,
                        bindings,
                        action="add",
                        target=f"{action_collection}/{action_skill}@{target_agent}",
                        detail={"version": version_id},
                        collection=action_collection,
                        skill=action_skill,
                        agent=target_agent,
                        expected_entry=old_assignment,
                    )
                except BaseException:
                    bindings.clear()
                    bindings.update(bindings_backup)
                    raise
            safe_update_skill_link(content_path, link_path, commit_cb, managed_bases)
            logging.info(f"[AGENT: {target_agent}] 已添加分配 {action_collection}/{action_skill} (版本 {version_id[:8]})")
            sys.exit(0)

    only_repos = set()
    only_skills = set()
    if args.only:
        known_repos = set(REPOS.keys()) | set(repositories.keys())
        if repo_base_dir.exists():
            for d in repo_base_dir.iterdir():
                if d.is_dir():
                    known_repos.add(d.name)
        for item in args.only:
            if ":" in item and not item.startswith("skill:"):
                logging.error(f"跨域元素 scope 拒绝: {item}")
                sys.exit(2)
            norm = item[6:] if item.startswith("skill:") else item
            if "/" in norm:
                r_name, s_name = norm.split("/", 1)
                if r_name not in known_repos:
                    logging.error(f"未知 skill 集合: {r_name}")
                    sys.exit(2)
                only_skills.add((r_name, s_name))
            else:
                if norm not in known_repos:
                    logging.error(f"未知 skill 或集合: {norm}")
                    sys.exit(2)
                only_repos.add(norm)

    for name, r_def in REPOS.items():
        if name in policy_cfg: r_def.update(policy_cfg[name])
        r_def["agents"] = canonicalize_agent_list(r_def.get("agents"))
        validate_repo_skill_count(name, r_def)
        validate_repo_disabled(name, r_def)
        validate_repo_new_skill_default(name, r_def)
        validate_repo_skill_matrix(name, r_def, r_def["agents"], set(DECLARED_AGENT_TARGETS))
        unknown_agents = [agent_name for agent_name in (r_def["agents"] or []) if agent_name not in DECLARED_AGENT_TARGETS]
        if unknown_agents:
            raise ValueError(f"{name}: agents 包含未声明的 target: {', '.join(sorted(unknown_agents))}")

    if not base_dir.exists() and not args.dry_run: base_dir.mkdir(parents=True, exist_ok=True)
    if not repo_base_dir.exists() and not args.dry_run: repo_base_dir.mkdir(parents=True, exist_ok=True)
    if not (args.only or args.agent):
        logging.info("执行环境洗刷...")
        if not args.dry_run:
            scrub_dead_links(ALL_AGENT_TARGETS, repo_base_dir, False)

    all_success = True
    duplicate_skill_names = set()
    duplicate_link_targets = {}
    pending_links = []
    global_skill_registry = {}

    bindings = None
    if sqlite_active:
        try:
            bindings = load_skill_bindings(db_path)
        except SkillBindingError as exc:
            logging.error(f"加载技能绑定失败: {exc}")
            sys.exit(1)
        if not args.dry_run:
            for ag_name, ag_target in ALL_AGENT_TARGETS.items():
                if args.agent and ag_name != args.agent:
                    continue
                target_base = Path(ag_target).expanduser()
                if not target_base.is_dir():
                    continue
                for item in target_base.iterdir():
                    if not is_link_entry(item):
                        continue
                    try:
                        target = resolve_link_target(item)
                    except OSError:
                        continue
                    if is_path_within_base(target, repo_base_dir):
                        try:
                            rel = Path(target).relative_to(repo_base_dir)
                            if rel.parts:
                                r_name = rel.parts[0]
                                s_name = item.name
                                if only_repos and r_name not in only_repos:
                                    continue
                                if only_skills and (r_name, s_name) not in only_skills:
                                    continue
                                existing_b = get_binding(bindings, r_name, s_name, ag_name)
                                if not existing_b or not existing_b.get("version"):
                                    source_skill_dir = Path(target)
                                    if (source_skill_dir / SKILL_FILE).is_file():
                                        snap = snapshot_skill(source_skill_dir, store_dir)
                                        v_id = snap["version_id"]
                                        c_path = snap["content_path"]
                                        def make_bootstrap_cb(r, s, a, v, old_b):
                                            def _cb():
                                                bindings_backup = copy.deepcopy(bindings)
                                                try:
                                                    set_binding(bindings, r, s, a, version=v, enabled=True, assigned=True)
                                                    save_skill_bindings(
                                                        db_path,
                                                        bindings,
                                                        action="bootstrap",
                                                        target=f"{r}/{s}@{a}",
                                                        detail={"version": v},
                                                        collection=r,
                                                        skill=s,
                                                        agent=a,
                                                        expected_entry=old_b,
                                                    )
                                                except BaseException:
                                                    bindings.clear()
                                                    bindings.update(bindings_backup)
                                                    raise
                                            return _cb
                                        safe_update_skill_link(
                                            c_path,
                                            item,
                                            make_bootstrap_cb(r_name, s_name, ag_name, v_id, existing_b),
                                            managed_bases=[repo_base_dir, store_dir],
                                        )
                        except Exception as exc:
                            logging.warning(f"快照已有链接失败: {item}: {exc}")

    targeted_repos = set(only_repos) | {r for r, _ in only_skills}
    deleted_targeted_repos = targeted_repos - set(REPOS.keys())
    if not sqlite_active:
        for repo_name in sorted(deleted_targeted_repos):
            repo_path = repo_base_dir / repo_name
            repo_skills_filter = None
            if only_skills:
                matched = {s for r, s in only_skills if r == repo_name}
                if matched and repo_name not in only_repos:
                    repo_skills_filter = matched
            cleanup_skills = collect_repo_skill_names_for_cleanup(repo_path) if repo_path.exists() else set()
            if repo_skills_filter is not None:
                cleanup_skills = cleanup_skills & repo_skills_filter
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                None,
                args.dry_run,
                cleanup_skills if repo_skills_filter is not None else (cleanup_skills or None),
                store_dir=store_dir,
                bindings=bindings,
                repo_name=repo_name,
                db_path=db_path if sqlite_active else None,
            )

    for repo_name, config in REPOS.items():
        if only_repos or only_skills:
            repo_matches = repo_name in only_repos or any(r == repo_name for r, _ in only_skills)
            if not repo_matches:
                continue

        repo_skills_filter = None
        if only_skills:
            matched = {s for r, s in only_skills if r == repo_name}
            if matched and repo_name not in only_repos:
                repo_skills_filter = matched

        repo_path = repo_base_dir / repo_name
        if not sqlite_active and repo_name not in policy_cfg:
            logging.info(f"[{repo_name}] 未声明策略，跳过。")
            cleanup_skills = collect_repo_skill_names_for_cleanup(repo_path, config)
            if repo_skills_filter is not None:
                cleanup_skills = cleanup_skills & repo_skills_filter
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                config.get("agents"),
                args.dry_run,
                cleanup_skills,
                store_dir=store_dir,
                bindings=bindings,
                repo_name=repo_name,
                db_path=None,
            )
            continue
        if not config.get("enabled", True):
            logging.info(f"[{repo_name}] (策略已禁用，跳过)")
            if not sqlite_active:
                cleanup_skills = collect_repo_skill_names_for_cleanup(repo_path, config)
                if repo_skills_filter is not None:
                    cleanup_skills = cleanup_skills & repo_skills_filter
                remove_repo_skill_links(
                    repo_path,
                    ALL_AGENT_TARGETS,
                    config.get("agents"),
                    args.dry_run,
                    cleanup_skills,
                    store_dir=store_dir,
                    bindings=bindings,
                    repo_name=repo_name,
                    db_path=db_path if sqlite_active else None,
                )
            continue
        should_update_repo = config.get("auto_update", True)
        if repo_skills_filter is not None and repo_path.exists():
            should_update_repo = False
        repo_path, repo_ok = clone_or_update_repo(
            repo_name,
            config.get("url"),
            repo_base_dir,
            args.dry_run,
            args.offline,
            should_update_repo,
            args.force_refresh,
        )
        if not repo_ok:
            all_success = False
            if not (repo_path.exists() and (repo_path / ".git").exists()):
                continue
        if args.dry_run and not repo_path.exists():
            logging.info(f"[{repo_name}] (预览: 本地仓库缺失，跳过 skill 枚举)")
            continue
        if not args.dry_run:
            write_owner_marker(repo_path, repo_name)
        if not ensure_repo_version(repo_path, config, args.dry_run, args.offline):
            all_success = False
            continue
        if not install_dependencies_and_build(repo_path, config, args.dry_run, args.offline):
            all_success = False
        try:
            skills = find_configured_skills(repo_path, config)
            skill_agent_overrides = None
            if repo_skill_matrix_mode(config):
                # 矩阵未知名 fail-close；disabled 名单里的 skill 仍是已发现 skill，
                # 矩阵可以引用它（最终 disabled 名单优先，见下方 partition）。
                skill_agent_overrides = resolve_repo_skill_agent_overrides(repo_name, config, set(skills))
            skills, disabled_active = partition_disabled_skills(repo_name, config, skills)
            if repo_skills_filter is not None:
                missing_skills = repo_skills_filter - (set(skills.keys()) | set(disabled_active) | set(collect_repo_skill_names_for_cleanup(repo_path)))
                if missing_skills:
                    logging.error(f"[{repo_name}] 未知 skill: {', '.join(sorted(missing_skills))}")
                    sys.exit(2)
                skills = {k: v for k, v in skills.items() if k in repo_skills_filter}
                disabled_active = disabled_active & repo_skills_filter
            if disabled_active:
                logging.info(
                    f"[{repo_name}] 本机禁用 skill（跳过新建并清理旧链）: {', '.join(sorted(disabled_active))}"
                )
            validate_repo_agent_scope(repo_name, config, skills)
        except DuplicateSkillNameError as exc:
            logging.error(f"[{repo_name}] {exc}")
            for skill_name in exc.skill_names:
                duplicate_skill_names.add(skill_name)
                record_duplicate_link_target(
                    duplicate_link_targets,
                    skill_name,
                    config.get("agents"),
                )
            if not sqlite_active:
                cleanup_skills = collect_repo_skill_names_for_cleanup(repo_path, config)
                if repo_skills_filter is not None:
                    cleanup_skills = cleanup_skills & repo_skills_filter
                remove_repo_skill_links(
                    repo_path,
                    ALL_AGENT_TARGETS,
                    config.get("agents"),
                    args.dry_run,
                    cleanup_skills,
                    store_dir=store_dir,
                    bindings=bindings,
                    repo_name=repo_name,
                    db_path=db_path if sqlite_active else None,
                )
            all_success = False
            continue
        except (SkillPathSelectionError, SkillAgentScopeError, SkillPolicyMatrixError) as exc:
            logging.error(f"[{repo_name}] {exc}")
            if not sqlite_active:
                cleanup_skills = collect_repo_skill_names_for_cleanup(repo_path, config)
                if repo_skills_filter is not None:
                    cleanup_skills = cleanup_skills & repo_skills_filter
                remove_repo_skill_links(
                    repo_path,
                    ALL_AGENT_TARGETS,
                    config.get("agents"),
                    args.dry_run,
                    cleanup_skills,
                    store_dir=store_dir,
                    bindings=bindings,
                    repo_name=repo_name,
                    db_path=db_path if sqlite_active else None,
                )
            all_success = False
            continue
        allowed_agents = config.get("agents")
        repo_targets = resolve_repo_link_targets(AGENT_TARGETS, ALL_AGENT_TARGETS, allowed_agents)
        inactive_agents = sorted(set(ALL_AGENT_TARGETS) - set(repo_targets))
        if not sqlite_active and inactive_agents:
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                inactive_agents,
                args.dry_run,
                set(skills),
                store_dir=store_dir,
                bindings=bindings,
                repo_name=repo_name,
                db_path=db_path if sqlite_active else None,
            )
        all_repo_skill_names = collect_repo_skill_names_for_cleanup(repo_path)
        unconfigured_skill_names = all_repo_skill_names - set(skills.keys())
        if repo_skills_filter is not None:
            unconfigured_skill_names = unconfigured_skill_names & repo_skills_filter
        if not sqlite_active and unconfigured_skill_names:
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                allowed_agents,
                args.dry_run,
                unconfigured_skill_names,
                store_dir=store_dir,
                bindings=bindings,
                repo_name=repo_name,
                db_path=db_path if sqlite_active else None,
            )
        if skill_agent_overrides is None:
            # 旧式定义的升格等价路径：发现范围内全部 skill 跟随 repo 目标集，
            # 每个 skill 的目标映射就是同一份 repo_targets，行为与 v1 逐比特一致。
            skill_targets_by_name = {s_name: repo_targets for s_name in skills}
        else:
            skill_targets_by_name = {}
            inbox_names = []
            for s_name in skills:
                override = skill_agent_overrides[s_name]
                if override is None:
                    skill_targets_by_name[s_name] = repo_targets
                    continue
                per_skill_targets = resolve_repo_link_targets(AGENT_TARGETS, ALL_AGENT_TARGETS, sorted(override))
                deactivated_agents = sorted(set(repo_targets) - set(per_skill_targets))
                if not sqlite_active and deactivated_agents:
                    remove_matrix_disabled_skill_links(
                        repo_path,
                        s_name,
                        ALL_AGENT_TARGETS,
                        deactivated_agents,
                        args.dry_run,
                        store_dir=store_dir,
                        bindings=bindings,
                        repo_name=repo_name,
                        db_path=db_path if sqlite_active else None,
                    )
                if not override:
                    inbox_names.append(s_name)
                skill_targets_by_name[s_name] = per_skill_targets
            if inbox_names:
                logging.info(
                    f"[{repo_name}] 新发现 skill 进入收件箱（未显式启用，不分发）: {', '.join(sorted(inbox_names))}"
                )
        pending_links.append((repo_name, skill_targets_by_name, skills))
        for s_name, s_path in skills.items():
            record_agents = allowed_agents
            if skill_agent_overrides is not None:
                override = skill_agent_overrides[s_name]
                record_agents = allowed_agents if override is None else sorted(override)
            existing = global_skill_registry.get(s_name)
            if existing is None:
                global_skill_registry[s_name] = (repo_name, s_path, record_agents)
                continue
            duplicate_skill_names.add(s_name)
            all_success = False
            existing_repo, existing_path, existing_allowed_agents = existing
            record_duplicate_link_target(
                duplicate_link_targets,
                s_name,
                existing_allowed_agents,
            )
            record_duplicate_link_target(
                duplicate_link_targets,
                s_name,
                record_agents,
            )
            logging.error(
                f"发现跨仓库重复 skill name: {s_name} ({existing_repo}:{existing_path} / {repo_name}:{s_path})"
            )

    managed_bases = [b for b in (repo_base_dir, store_dir) if b is not None]
    if sqlite_active:
        # 第一阶段：预扫描所有活跃分配，在发生任何链接变更前收集同名冲突并锁定跳过
        active_owners_by_target = {}
        conflicted_targets = set()

        for r_name, r_skills in bindings.items():
            if only_repos and r_name not in only_repos:
                continue
            r_cfg = REPOS.get(r_name)
            if r_cfg is None:
                logging.warning(
                    f"[{r_name}] 集合在本地配置中已缺失（集合删除动作尚未接入，暂保留已有不可变绑定与入口）"
                )
            r_disabled = (r_cfg is not None) and (not r_cfg.get("enabled", True))
            r_disabled_skills = set(r_cfg.get("disabled") or []) if r_cfg is not None else set()

            for s_name, s_agents in r_skills.items():
                if only_skills and (r_name, s_name) not in only_skills:
                    continue
                s_disabled = r_disabled or (s_name in r_disabled_skills)

                for ag_name, assignment in s_agents.items():
                    if args.agent and ag_name != args.agent:
                        continue
                    if ag_name not in ALL_AGENT_TARGETS:
                        continue

                    ag_allowed = True
                    if r_cfg is not None and r_cfg.get("agents") is not None:
                        if ag_name not in r_cfg.get("agents", []):
                            ag_allowed = False

                    is_active = (
                        assignment.get("assigned", True)
                        and assignment.get("enabled", True)
                        and not s_disabled
                        and ag_allowed
                    )
                    if is_active:
                        target_key = (ag_name, s_name)
                        if target_key in active_owners_by_target:
                            conflicted_targets.add(target_key)
                            active_owners_by_target[target_key].append(r_name)
                        else:
                            active_owners_by_target[target_key] = [r_name]

        for target_key in sorted(conflicted_targets):
            ag_name, s_name = target_key
            owners = active_owners_by_target[target_key]
            logging.error(f"跨集合同名冲突: {s_name}@{ag_name} 在多个集合 {owners} 同时存在活跃分配，跳过全部变更以保护旧入口")
            all_success = False

        # 第二阶段：遍历 bindings 执行安全收敛
        for r_name, r_skills in bindings.items():
            if only_repos and r_name not in only_repos:
                continue
            r_cfg = REPOS.get(r_name)
            r_disabled = (r_cfg is not None) and (not r_cfg.get("enabled", True))
            r_disabled_skills = set(r_cfg.get("disabled") or []) if r_cfg is not None else set()

            for s_name, s_agents in r_skills.items():
                if only_skills and (r_name, s_name) not in only_skills:
                    continue
                s_disabled = r_disabled or (s_name in r_disabled_skills)

                for ag_name, assignment in s_agents.items():
                    if args.agent and ag_name != args.agent:
                        continue
                    if ag_name not in ALL_AGENT_TARGETS:
                        continue

                    target_key = (ag_name, s_name)
                    if target_key in conflicted_targets:
                        # 冲突双方全部跳过，保持旧入口
                        continue

                    target_base = Path(ALL_AGENT_TARGETS[ag_name]).expanduser()
                    if not target_base.exists() and not args.dry_run:
                        target_base.mkdir(parents=True, exist_ok=True)
                    link_path = target_base / s_name

                    ag_allowed = True
                    if r_cfg is not None and r_cfg.get("agents") is not None:
                        if ag_name not in r_cfg.get("agents", []):
                            ag_allowed = False

                    should_withdraw = (
                        not assignment.get("assigned", True)
                        or not assignment.get("enabled", True)
                        or s_disabled
                        or not ag_allowed
                    )

                    if should_withdraw:
                        # 撤回入口（若链接属于本集合/版本）
                        if (link_path.exists() or is_link_entry(link_path)) and is_managed_skill_link(link_path, managed_bases):
                            should_remove = False
                            if assignment.get("version"):
                                expected = (store_dir / assignment["version"] / "content").resolve()
                                try:
                                    if Path(resolve_link_target(link_path)).resolve() == expected:
                                        should_remove = True
                                except OSError:
                                    pass
                            if not should_remove and link_points_within_base(link_path, repo_base_dir / r_name):
                                should_remove = True
                            if should_remove and not args.dry_run:
                                safe_remove_skill_link(link_path, None, managed_bases=managed_bases)
                        continue

                    # 应该保持/建立链接
                    version_id = assignment.get("version")
                    if not version_id:
                        continue
                    try:
                        content_path = verify_skill_version(store_dir, version_id)
                    except SkillVersionError as exc:
                        logging.error(f"[{r_name}/{s_name}@{ag_name}] 存储版本校验失败: {exc}")
                        all_success = False
                        continue

                    # 无论是否 dry-run，必须校验外部冲突
                    try:
                        check_not_external_conflict(link_path, managed_bases)
                    except SkillConflictError as exc:
                        logging.error(f"[{r_name}/{s_name}@{ag_name}] 冲突: {exc}")
                        all_success = False
                        continue

                    # 核对旧 binding 期望入口：已有链接若指向另一个受管 version 也属于外部/未完成修改，拒绝静默覆盖
                    if link_path.exists() or is_link_entry(link_path):
                        try:
                            current_target = Path(resolve_link_target(link_path)).resolve()
                            if current_target != content_path:
                                logging.error(
                                    f"[{r_name}/{s_name}@{ag_name}] 现有链接指向非本分配期望版本 ({current_target})，拒绝覆盖并保留原有状态"
                                )
                                all_success = False
                                continue
                        except OSError as exc:
                            logging.error(f"[{r_name}/{s_name}@{ag_name}] 读取现有链接失败: {exc}")
                            all_success = False
                            continue

                    if args.dry_run:
                        logging.info(f"[AGENT: {ag_name}] (预览: 计划保持版本 {s_name} -> {version_id[:8]})")
                        continue

                    safe_update_skill_link(content_path, link_path, None, managed_bases)

        managed_skill_names_by_agent = {}
        for ag_name, ag_target in ALL_AGENT_TARGETS.items():
            names = set()
            for r_name, r_skills in bindings.items():
                for s_name, s_agents in r_skills.items():
                    b = s_agents.get(ag_name)
                    if b and b.get("assigned", True) and b.get("enabled", True):
                        names.add(s_name)
            managed_skill_names_by_agent[ag_name] = names

        if not (args.only or args.agent):
            planned_removed_repo_paths = prune_orphan_repos(repo_base_dir, REPOS.keys(), args.dry_run, args.yes)
            scrub_dead_links(
                ALL_AGENT_TARGETS,
                repo_base_dir,
                args.dry_run,
                managed_skill_names_by_agent,
                planned_removed_repo_paths,
                store_dir=store_dir,
            )
        if not all_success:
            sys.exit(1)
        logging.info("技能同步完成！")
        return

    # 旧非 SQLite 路径保持清除
    for s_name, allowed_agents in duplicate_link_targets.items():
        remove_skill_links(s_name, ALL_AGENT_TARGETS, allowed_agents, args.dry_run, managed_bases=managed_bases)

    for repo_name, skill_targets_by_name, skills in pending_links:
        for s_name, s_path in skills.items():
            if s_name in duplicate_skill_names:
                logging.error(f"[{repo_name}] 跳过重复 skill name: {s_name}")
                continue
            try:
                manage_links(
                    s_name,
                    s_path,
                    skill_targets_by_name[s_name],
                    None,
                    True,
                    args.dry_run,
                    store_dir=store_dir,
                    bindings=bindings,
                    repo_name=repo_name,
                    db_path=db_path if sqlite_active else None,
                    repo_base_dir=repo_base_dir,
                )
            except SkillConflictError as exc:
                logging.error(f"[{repo_name}] 冲突: {exc}")
                all_success = False
            except SkillVersionError as exc:
                logging.error(f"[{repo_name}] 技能版本校验失败: {exc}")
                all_success = False

    managed_skill_names_by_agent = collect_current_managed_skill_names_by_agent(
        pending_links,
        duplicate_skill_names,
    )
    if not (args.only or args.agent):
        planned_removed_repo_paths = prune_orphan_repos(repo_base_dir, REPOS.keys(), args.dry_run, args.yes)
        scrub_dead_links(
            ALL_AGENT_TARGETS,
            repo_base_dir,
            args.dry_run,
            managed_skill_names_by_agent,
            planned_removed_repo_paths,
            store_dir=store_dir,
        )
    if not all_success: sys.exit(1)
    logging.info("技能同步完成！")

if __name__ == "__main__":
    main()
