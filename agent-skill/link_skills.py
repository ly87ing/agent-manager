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

from src.agent_ids import canonicalize_agent_list, canonicalize_agent_mapping

# 配置日志：统一审计时间戳
logging.basicConfig(
    level=logging.INFO, 
    format='%(asctime)s [%(levelname)s] %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)

MANAGED_REPO_DIRNAME = "repositories"
OWNER_MARKER = ".agent-manager-owned"
SKILL_FILE = "SKILL.md"
EXCLUDED_SKILL_DIRS = {".git", "node_modules", "__pycache__", "venv", ".venv"}

def load_json(path):
    if not os.path.exists(path): return {}
    try:
        with open(path, 'r', encoding='utf-8') as f:
            return json.load(f)
    except: return {}

def run_cmd(cmd, cwd=None, timeout=300):
    """运行 shell 命令并返回结果"""
    try:
        result = subprocess.run(cmd, cwd=cwd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, shell=isinstance(cmd, str), timeout=timeout)
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
                        run_cmd(f"{pkg_mgr} run build", cwd=repo_path)
                except: pass
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
            success, output = run_cmd(["git", "pull", "--ff-only"], cwd=target_dir)
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
    return target_dir, True

def find_skills_recursively(search_path):
    found_skills = {}
    search_path = Path(search_path).resolve()
    if not search_path.exists(): return found_skills
    for root, dirs, files in os.walk(search_path):
        dirs[:] = [d for d in dirs if d not in EXCLUDED_SKILL_DIRS]
        if SKILL_FILE in files:
            skill_dir = Path(root)
            skill_name = read_skill_name(skill_dir / SKILL_FILE)
            if not skill_name:
                continue
            found_skills[skill_name] = skill_dir
    return found_skills

def create_link(source, link, dry_run=False):
    if dry_run: return
    source, link = Path(source).resolve(), Path(link)
    if link.exists() or link.is_symlink():
        try:
            if link.is_symlink() or not link.is_dir(): link.unlink()
            else: shutil.rmtree(link)
        except: return
    try:
        if platform.system() == "Windows":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(source)], check=True, capture_output=True)
        else:
            rel_source = os.path.relpath(source, link.parent)
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

def manage_links(skill_name, source_path, targets, allowed_agents, is_enabled=True, dry_run=False):
    active_targets = targets
    if allowed_agents is not None:
        active_targets = {k: v for k, v in targets.items() if k in allowed_agents}
    for agent_name, target_path in active_targets.items():
        target_base = Path(target_path).expanduser()
        link_path = target_base / skill_name
        if not target_base.parent.exists() and is_enabled: continue
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
    parser.add_argument("--config", default="../config/manager.json", help="策略配置")
    parser.add_argument("--def-file", default="../config/skills.json", help="定义文件")
    parser.add_argument("--base-dir", default=".", help="目录")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过远程仓库更新与依赖安装)")
    parser.add_argument("--yes", action="store_true", help="自动清理")
    args = parser.parse_args()
    
    # 对齐职责：合并策略
    policy_cfg = load_json(args.config).get("skills", {})
    full_def = load_json(args.def_file)
    REPOS = {name: dict(config) for name, config in full_def.get("repositories", {}).items()}
    AGENT_TARGETS = canonicalize_agent_mapping(
        {k: (v if isinstance(v, str) else v.get("path")) for k, v in full_def.get("agents", {}).items()}
    )
    
    for name, r_def in REPOS.items():
        if name in policy_cfg: r_def.update(policy_cfg[name])
        r_def["agents"] = canonicalize_agent_list(r_def.get("agents"))

    base_dir = Path(args.base_dir).resolve()
    repo_base_dir = get_managed_repo_base(base_dir)
    if not base_dir.exists() and not args.dry_run: base_dir.mkdir(parents=True, exist_ok=True)
    if not repo_base_dir.exists() and not args.dry_run: repo_base_dir.mkdir(parents=True, exist_ok=True)
    logging.info("执行环境洗刷...")
    scrub_dead_links(AGENT_TARGETS, repo_base_dir, args.dry_run)
    
    all_success = True
    for repo_name, config in REPOS.items():
        if not config.get("enabled", True):
            logging.info(f"[{repo_name}] (策略已禁用，跳过)")
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
        if not args.dry_run:
            write_owner_marker(repo_path, repo_name)
        if not ensure_repo_version(repo_path, config, args.dry_run, args.offline):
            all_success = False
            continue
        if not install_dependencies_and_build(repo_path, config, args.dry_run, args.offline): all_success = False
        skills = find_skills_recursively(repo_path)
        for s_name, s_path in skills.items():
            manage_links(s_name, s_path, AGENT_TARGETS, config.get("agents"), True, args.dry_run)
    
    if not args.dry_run: prune_orphan_repos(repo_base_dir, REPOS.keys(), args.dry_run, args.yes)
    if not all_success: sys.exit(1)
    logging.info("技能同步完成！")

if __name__ == "__main__":
    main()
