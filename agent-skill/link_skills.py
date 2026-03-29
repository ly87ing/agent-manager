#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import shutil
import json
import platform
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_id, canonicalize_agent_list, canonicalize_agent_mapping
from src.command_runner import run_command
from src.json_config import get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import is_agent_installed
from src.runtime_logging import configure_logging

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
    """运行 shell 命令并返回结果"""
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
    marker_path = Path(repo_path) / OWNER_MARKER
    marker_path.write_text(
        json.dumps({"managed_by": "agent-manager", "repository": repo_name}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

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

def ensure_repo_version(repo_path, config, dry_run=False, offline=False):
    revision_kind, revision = resolve_requested_revision(config)
    if not revision:
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
    """根据仓库内容执行依赖安装与初始化"""
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

def clone_or_update_repo(name, url, base_dir, dry_run=False, offline=False, auto_update=True):
    """下载或更新 Git 仓库"""
    target_dir = Path(base_dir) / name
    if target_dir.exists() and (target_dir / ".git").exists():
        if dry_run:
            logging.info(f"[{name}] (预览: 计划检查托管仓库状态)")
        elif offline:
            logging.info(f"[{name}] 离线模式，跳过仓库更新。")
        elif not auto_update:
            logging.info(f"[{name}] auto_update 关闭，跳过仓库更新。")
        else:
            update_cmd = ["git", "pull", "--ff-only"]
            if repo_head_is_detached(target_dir):
                update_cmd = ["git", "fetch", "--tags", "origin"]
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

def create_link(source, link, dry_run=False):
    if dry_run: return
    source, link = Path(source).resolve(), Path(link)
    link_parent = link.parent.resolve()
    if link.exists() or link.is_symlink():
        try:
            if link.is_symlink() or not link.is_dir(): link.unlink()
            else: shutil.rmtree(link)
        except OSError: return
    try:
        if platform.system() == "Windows":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(source)], check=True, capture_output=True)
        else:
            rel_source = os.path.relpath(source, link_parent)
            os.symlink(rel_source, link, target_is_directory=True)
            logging.info(f"链接: {rel_source} -> {link.name}")
    except Exception as e: logging.error(f"链接失败: {e}")

def scrub_dead_links(agents_config, base_dir, dry_run=False):
    base_dir = Path(base_dir).resolve()
    for agent_name, target_path in agents_config.items():
        target_base = Path(target_path).expanduser()
        if not target_base.exists() or not target_base.is_dir(): continue
        for item in target_base.iterdir():
            if not item.is_symlink() or item.exists():
                continue
            try:
                raw_target = Path(os.readlink(item)).expanduser()
                resolved_target = raw_target if raw_target.is_absolute() else (item.parent / raw_target)
                resolved_target = resolved_target.resolve(strict=False)
                if str(resolved_target).startswith(str(base_dir)):
                    logging.info(f"[AGENT: {agent_name}] 移除死链: {item.name}")
                    if not dry_run:
                        item.unlink()
            except OSError:
                continue

def is_active_skill_target(agent_name, target_path, agent_defs):
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
    if not base_path.exists(): return
    for item in base_path.iterdir():
        if not item.is_dir() or item.name in managed_names or item.name in excluded:
            continue
        if not is_manager_owned_repo(item):
            logging.info(f"跳过非托管目录: {item.name}")
            continue
        logging.warning(f"发现孤儿仓库: {item.name}")
        if dry_run: continue
        if force_yes or (sys.stdin.isatty() and input(f"删除 {item.name}? (y/n): ").lower() == 'y'):
            shutil.rmtree(item); logging.info(f"已清理: {item.name}")


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


def remove_repo_skill_links(repo_path, targets, allowed_agents=None, dry_run=False, skill_names=None):
    repo_path = Path(repo_path).resolve()
    managed_skill_names = set(skill_names or [])
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    for agent_name, target_path in active_targets.items():
        target_base = Path(target_path).expanduser()
        if not target_base.exists() or not target_base.is_dir():
            continue
        for item in target_base.iterdir():
            if not item.is_symlink():
                continue
            if item.name not in managed_skill_names:
                continue
            logging.error(f"[AGENT: {agent_name}] 移除来自失效仓库的 skill 链接: {item.name}")
            if not dry_run:
                item.unlink()


def remove_skill_links(skill_name, targets, allowed_agents=None, dry_run=False):
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    for agent_name, target_path in active_targets.items():
        link_path = Path(target_path).expanduser() / skill_name
        if not (link_path.exists() or link_path.is_symlink()):
            continue
        logging.error(f"[AGENT: {agent_name}] 移除冲突 skill 链接: {skill_name}")
        if dry_run:
            continue
        if link_path.is_dir() and not link_path.is_symlink():
            shutil.rmtree(link_path)
        else:
            link_path.unlink()

def manage_links(skill_name, source_path, targets, allowed_agents, is_enabled=True, dry_run=False):
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    for agent_name, target_path in active_targets.items():
        target_base = Path(target_path).expanduser()
        link_path = target_base / skill_name
        if agent_name != "codex" and not target_base.parent.exists() and is_enabled: continue
        if not is_enabled:
            if (link_path.exists() or link_path.is_symlink()) and not dry_run:
                if link_path.is_dir() and not link_path.is_symlink(): shutil.rmtree(link_path)
                else: link_path.unlink()
            continue
        if not target_base.exists() and not dry_run: target_base.mkdir(parents=True, exist_ok=True)
        if dry_run: logging.info(f"[AGENT: {agent_name}] (预览: 计划加载 {skill_name})"); continue
        create_link(source_path, link_path, dry_run)

def main():
    parser = argparse.ArgumentParser(description="Agent Skill 管理工具 (策略对齐)")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略配置")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "skills.json"), help="定义文件")
    parser.add_argument("--base-dir", default=str(Path(__file__).resolve().parent), help="目录")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过远程仓库更新与依赖安装)")
    parser.add_argument("--yes", action="store_true", help="自动清理")
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

    # 对齐职责：合并策略
    REPOS = {name: dict(config) for name, config in repositories.items()}
    AGENT_TARGETS = canonicalize_agent_mapping(
        {k: (v if isinstance(v, str) else v.get("path")) for k, v in agent_targets.items()}
    )

    # 区分全量 targets（用于清理旧链接）和活跃 targets（用于新建链接）
    agent_defs = load_json_object_file(REPO_ROOT / "config" / "agents.json", strict=False, missing_ok=True) or {}
    installed_agents = {
        name for name, target_path in AGENT_TARGETS.items() if is_active_skill_target(name, target_path, agent_defs)
    }
    skipped = [name for name in AGENT_TARGETS if name not in installed_agents]
    for name in skipped:
        if name == "codex":
            logging.info(f"[AGENT: {name}] 不参与全局 skill 新建；仅在仓库显式声明 agents 时接收 skill 链接。")
            continue
        logging.info(f"[AGENT: {name}] 未安装，跳过新建 skill 链接（保留清理能力）。")
    ALL_AGENT_TARGETS = AGENT_TARGETS  # 全量，用于清理旧链接
    AGENT_TARGETS = {k: v for k, v in AGENT_TARGETS.items() if k in installed_agents}  # 活跃，用于新建链接

    for name, r_def in REPOS.items():
        if name in policy_cfg: r_def.update(policy_cfg[name])
        r_def["agents"] = canonicalize_agent_list(r_def.get("agents"))

    base_dir = Path(args.base_dir).resolve()
    repo_base_dir = get_managed_repo_base(base_dir)
    if not base_dir.exists() and not args.dry_run: base_dir.mkdir(parents=True, exist_ok=True)
    if not repo_base_dir.exists() and not args.dry_run: repo_base_dir.mkdir(parents=True, exist_ok=True)
    logging.info("执行环境洗刷...")
    scrub_dead_links(ALL_AGENT_TARGETS, repo_base_dir, args.dry_run)

    all_success = True
    duplicate_skill_names = set()
    duplicate_link_targets = {}
    pending_links = []
    global_skill_registry = {}
    for repo_name, config in REPOS.items():
        repo_path = repo_base_dir / repo_name
        if repo_name not in policy_cfg:
            logging.info(f"[{repo_name}] 未声明策略，跳过。")
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                config.get("agents"),
                args.dry_run,
                collect_repo_skill_names_for_cleanup(repo_path, config),
            )
            continue
        if not config.get("enabled", True):
            logging.info(f"[{repo_name}] (策略已禁用，跳过)")
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                config.get("agents"),
                args.dry_run,
                collect_repo_skill_names_for_cleanup(repo_path, config),
            )
            continue
        repo_path, repo_ok = clone_or_update_repo(
            repo_name,
            config.get("url"),
            repo_base_dir,
            args.dry_run,
            args.offline,
            config.get("auto_update", True),
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
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                config.get("agents"),
                args.dry_run,
                collect_repo_skill_names_for_cleanup(repo_path, config),
            )
            all_success = False
            continue
        except (SkillPathSelectionError, SkillAgentScopeError) as exc:
            logging.error(f"[{repo_name}] {exc}")
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                config.get("agents"),
                args.dry_run,
                collect_repo_skill_names_for_cleanup(repo_path, config),
            )
            all_success = False
            continue
        allowed_agents = config.get("agents")
        repo_targets = resolve_repo_link_targets(AGENT_TARGETS, ALL_AGENT_TARGETS, allowed_agents)
        inactive_agents = sorted(set(ALL_AGENT_TARGETS) - set(repo_targets))
        if inactive_agents:
            remove_repo_skill_links(
                repo_path,
                ALL_AGENT_TARGETS,
                inactive_agents,
                args.dry_run,
                set(skills),
            )
        pending_links.append((repo_name, repo_targets, skills))
        for s_name, s_path in skills.items():
            existing = global_skill_registry.get(s_name)
            if existing is None:
                global_skill_registry[s_name] = (repo_name, s_path, allowed_agents)
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
                allowed_agents,
            )
            logging.error(
                f"发现跨仓库重复 skill name: {s_name} ({existing_repo}:{existing_path} / {repo_name}:{s_path})"
            )

    for s_name, allowed_agents in duplicate_link_targets.items():
        remove_skill_links(s_name, ALL_AGENT_TARGETS, allowed_agents, args.dry_run)

    for repo_name, repo_targets, skills in pending_links:
        for s_name, s_path in skills.items():
            if s_name in duplicate_skill_names:
                logging.error(f"[{repo_name}] 跳过重复 skill name: {s_name}")
                continue
            manage_links(s_name, s_path, repo_targets, None, True, args.dry_run)

    if not args.dry_run:
        prune_orphan_repos(repo_base_dir, REPOS.keys(), args.dry_run, args.yes)
        scrub_dead_links(ALL_AGENT_TARGETS, repo_base_dir, args.dry_run)
    if not all_success: sys.exit(1)
    logging.info("技能同步完成！")

if __name__ == "__main__":
    main()
