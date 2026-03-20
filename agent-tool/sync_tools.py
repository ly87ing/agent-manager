#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import shutil
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.json_config import get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import (
    TOOL_BROKEN_MARKER,
    TOOL_READY_MARKER,
    detect_tool_readiness,
    repo_tool_artifacts_ready,
)
from src.runtime_logging import configure_logging

configure_logging()

OWNER_MARKER = ".agent-manager-owned"

def run_cmd(cmd, cwd=None, timeout=None):
    """运行 shell 命令并返回结果"""
    try:
        # Determine if we should use shell
        if isinstance(cmd, list):
            # Use list form without shell
            result = subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True, shell=False, timeout=timeout)
        else:
            # Use string form with shell
            result = subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True, shell=True, timeout=timeout)
        return True, result.stdout
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT after {timeout} seconds: {cmd}"
    except subprocess.CalledProcessError as e:
        return False, f"Command '{cmd}' failed with exit code {e.returncode}: {e.stderr}"
    except FileNotFoundError as e:
        return False, f"Directory or executable not found: {e}"
    except Exception as e:
        return False, f"Unexpected error executing '{cmd}': {e}"


class GitStatusCache:
    """缓存 git 仓库状态，避免重复执行慢速网络请求"""
    _instance = None
    
    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
            cls._instance._cache = {}
        return cls._instance
    
    def is_git_up_to_date(self, cwd, timeout=5):
        """检查 git 是否已是最新（带缓存和超时）"""
        cwd_str = str(Path(cwd).resolve())
        
        # 检查缓存
        if cwd_str in self._cache:
            return self._cache[cwd_str]
        
        # 快速检查：先看本地是否有未提交更改
        success, out = run_cmd(["git", "status", "--porcelain"], cwd=cwd, timeout=timeout)
        if not success:
            self._cache[cwd_str] = True  # 失败时保守假设是最新的
            return True
        
        if out.strip():
            # 有本地修改，需要更新
            self._cache[cwd_str] = False
            return False
        
        # 检查远程更新：使用 ls-remote 代替 fetch（更快，不修改本地状态）
        success, out = run_cmd(["git", "ls-remote", "origin", "HEAD"], cwd=cwd, timeout=timeout)
        if not success or out.strip() == "":
            self._cache[cwd_str] = True  # 无法检查时保守假设是最新的
            return True
        
        remote_hash = out.strip().split()[0]
        
        success, local_out = run_cmd(["git", "rev-parse", "HEAD"], cwd=cwd, timeout=timeout)
        if not success:
            self._cache[cwd_str] = True
            return True
        
        local_hash = local_out.strip()
        
        result = (local_hash == remote_hash)
        self._cache[cwd_str] = result
        return result

# 实例化缓存
git_cache = GitStatusCache()


def ready_marker_path(target_dir: Path) -> Path:
    return target_dir / TOOL_READY_MARKER


def broken_marker_path(target_dir: Path) -> Path:
    return target_dir / TOOL_BROKEN_MARKER


def mark_tool_ready(target_dir: Path):
    ready_marker_path(target_dir).write_text("ready\n", encoding="utf-8")
    broken_marker_path(target_dir).unlink(missing_ok=True)


def mark_tool_broken(target_dir: Path):
    broken_marker_path(target_dir).write_text("broken\n", encoding="utf-8")
    ready_marker_path(target_dir).unlink(missing_ok=True)


def clear_tool_markers(target_dir: Path):
    ready_marker_path(target_dir).unlink(missing_ok=True)
    broken_marker_path(target_dir).unlink(missing_ok=True)


def owner_marker_path(target_dir: Path) -> Path:
    return target_dir / OWNER_MARKER


def write_owner_marker(target_dir: Path, tool_name: str):
    owner_marker_path(target_dir).write_text(f"{tool_name}\n", encoding="utf-8")


def is_manager_owned_tool_dir(target_dir: Path) -> bool:
    return owner_marker_path(target_dir).is_file()


def apply_overrides(name, target_dir, dry_run=False):
    """将本地补丁应用到目标工具目录"""
    override_dir = Path(__file__).parent / "overrides" / name
    if not override_dir.exists(): return
    
    logging.info(f"应用 {name} 本地补丁 (Overrides)...")
    for root, _, files in os.walk(override_dir):
        for f in files:
            src = Path(root) / f
            rel_path = src.relative_to(override_dir)
            dst = target_dir / rel_path
            if dry_run:
                logging.info(f"  (预览: 计划覆盖 {rel_path})")
                continue
            dst.parent.mkdir(parents=True, exist_ok=True)
            if dst.is_symlink() or dst.is_file():
                dst.unlink()
            elif dst.is_dir():
                shutil.rmtree(dst)
            shutil.copy2(src, dst)
            logging.info(f"  已覆盖: {rel_path}")

def prune_orphan_tools(base_dir, managed_names, dry_run=False, force_yes=False):
    """清理不再受管理的工具目录"""
    base_path = Path(base_dir).resolve()
    excluded = {"overrides", ".gitignore", "sync_tools.py", "__pycache__"}
    if not base_path.exists(): return

    for item in base_path.iterdir():
        if item.is_dir() and item.name not in managed_names and item.name not in excluded:
            if not is_manager_owned_tool_dir(item):
                logging.info(f"跳过非托管工具目录: {item.name}")
                continue
            logging.warning(f"检测到孤儿工具目录: {item.name}")
            if dry_run: continue
            
            should_delete = force_yes
            if not should_delete and sys.stdin.isatty():
                try:
                    confirm = input(f"  是否删除 {item.name} 以释放磁盘空间? (y/n): ")
                    should_delete = (confirm.lower() == 'y')
                except EOFError: should_delete = False
            
            if should_delete:
                shutil.rmtree(item)
                logging.info(f"  已成功清理: {item.name}")

def _check_ready(name: str, ready_check: str | None) -> bool:
    """根据 ready_check 字段判断工具是否已就绪。

    支持两种前缀：
      bin:<name>  — 用 shutil.which 检查可执行文件是否在 PATH 中
      app:<path>  — 用 os.path.exists 检查 .app 包是否存在
    若 ready_check 为空则始终返回 False（视为未安装）。
    """
    if not ready_check:
        return False
    if ready_check.startswith("bin:"):
        bin_name = ready_check[4:].strip()
        found = shutil.which(bin_name) is not None
        logging.debug(f"[TOOL: {name}] ready_check bin:{bin_name} -> {found}")
        return found
    if ready_check.startswith("app:"):
        app_path = os.path.expanduser(ready_check[4:].strip())
        found = os.path.exists(app_path)
        logging.debug(f"[TOOL: {name}] ready_check app:{app_path} -> {found}")
        return found
    logging.warning(f"[TOOL: {name}] 未知 ready_check 格式: {ready_check}，视为未就绪")
    return False


def sync_tool(name, definition, policy, base_dir, dry_run=False, force=False):
    # 合并策略
    enabled = policy.get("enabled", True)
    auto_update = policy.get("auto_update", True)
    
    if not enabled:
        uninstall_cmd = definition.get("uninstall")
        ready_check = definition.get("ready_check")
        if uninstall_cmd and _check_ready(name, ready_check):
            if dry_run:
                logging.info(f"[TOOL: {name}] (预览: 策略已禁用，计划卸载)")
            else:
                logging.info(f"[TOOL: {name}] 策略已禁用，执行卸载...")
                ok, out = run_cmd(uninstall_cmd)
                if not ok:
                    logging.error(f"[TOOL: {name}] 卸载失败: {out}")
                    return False
                logging.info(f"[TOOL: {name}] 已卸载")
        else:
            logging.info(f"[TOOL: {name}] (策略已禁用，跳过)")
        return True

    url = definition.get("url")
    install_cmd = definition.get("install")
    setup_cmd = definition.get("setup")
    target_dir = Path(base_dir) / name
    needs_setup = force
    setup_failed = False
    success_build = True
    should_apply_overrides = False
    repo_state_changed = False

    if url:
        if target_dir.exists() and (target_dir / ".git").exists():
            if not auto_update:
                if detect_tool_readiness(name, definition, base_dir):
                    logging.info(f"[TOOL: {name}] 已就绪，auto_update 关闭。")
                    should_apply_overrides = True
                else:
                    logging.warning(f"[TOOL: {name}] 检测到现有仓库未就绪，auto_update 关闭，保留当前状态。")
                    success_build = False
            else:
                if dry_run: logging.info(f"[TOOL: {name}] (预览: 计划检查更新)")
                else:
                    status_ok, status_out = run_cmd(["git", "status", "--porcelain"], cwd=target_dir, timeout=5)
                    if status_ok and status_out.strip():
                        logging.warning(f"[TOOL: {name}] 检测到本地未提交更改，跳过更新以保护工作区。")
                    elif git_cache.is_git_up_to_date(target_dir):
                        logging.info(f"[TOOL: {name}] (内容已最新)")
                        should_apply_overrides = True
                    else:
                        success, out = run_cmd(["git", "pull"], cwd=target_dir)
                        if success:
                            logging.info(f"[TOOL: {name}] (已更新)")
                            needs_setup = True
                            should_apply_overrides = True
                            repo_state_changed = True
                        else: logging.error(f"[TOOL: {name}] (更新失败: {out})"); success_build = False
        else:
            if target_dir.exists():
                logging.warning(f"[TOOL: {name}] 目录异常，正在重装...")
                if not dry_run: shutil.rmtree(target_dir)
            logging.info(f"正在安装 {name}...")
            if not dry_run:
                success, out = run_cmd(["git", "clone", url, str(target_dir)])
                if success:
                    logging.info(f"[TOOL: {name}] (安装成功)")
                    needs_setup = True
                    should_apply_overrides = True
                    repo_state_changed = True
                else: logging.error(f"[TOOL: {name}] (下载失败: {out})"); success_build = False
        
        if not dry_run and should_apply_overrides: apply_overrides(name, target_dir, dry_run)
        if setup_cmd and needs_setup and not dry_run:
            logging.info(f"执行 {name} 初始化...")
            ok, out = run_cmd(setup_cmd, cwd=target_dir)
            if not ok:
                logging.error(f"[TOOL: {name}] 构建失败: {out}")
                setup_failed = True
                success_build = False

        if not dry_run and target_dir.exists() and (target_dir / ".git").exists():
            write_owner_marker(target_dir, name)
            repo_ready = repo_tool_artifacts_ready(definition, target_dir)
            if setup_failed:
                mark_tool_broken(target_dir)
            elif success_build and repo_ready and (
                repo_state_changed or force or not broken_marker_path(target_dir).exists()
            ):
                mark_tool_ready(target_dir)
            elif (repo_state_changed or force) and not repo_ready:
                mark_tool_broken(target_dir)
            if not detect_tool_readiness(name, definition, base_dir):
                success_build = False

    elif install_cmd:
        ready_check = definition.get("ready_check")
        upgrade_cmd = definition.get("upgrade")

        # 检查工具是否已就绪
        already_ready = _check_ready(name, ready_check)

        if already_ready:
            if auto_update and upgrade_cmd:
                if dry_run:
                    logging.info(f"[TOOL: {name}] (预览: 计划执行升级)")
                else:
                    logging.info(f"[TOOL: {name}] 已安装，执行升级...")
                    ok, out = run_cmd(upgrade_cmd)
                    if not ok:
                        logging.warning(f"[TOOL: {name}] 升级失败（保留现有版本）: {out}")
            else:
                logging.info(f"[TOOL: {name}] 已就绪，跳过安装。")
        else:
            if dry_run:
                logging.info(f"[TOOL: {name}] (预览: 计划执行安装)")
            else:
                logging.info(f"执行 {name} 全局安装...")
                ok, out = run_cmd(install_cmd)
                if not ok:
                    logging.error(f"[TOOL: {name}] 安装失败: {out}")
                    success_build = False

    return success_build

def main():
    parser = argparse.ArgumentParser(description="Agent Tool 管理工具 (策略驱动)")
    parser.add_argument("--base-dir", default=str(Path(__file__).resolve().parent), help="基础目录")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略路径")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "tools.json"), help="定义路径")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--force", action="store_true", help="强制初始化")
    parser.add_argument("--yes", action="store_true", help="自动清理")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过网络操作)")
    args = parser.parse_args()

    try:
        policy_root = load_json_object_file(args.config, strict=True, missing_ok=False)
        def_root = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        policy_cfg = get_typed_mapping_section(policy_root, "tools", Path(args.config).name, (dict,), "对象")
        def_cfg = get_typed_mapping_section(def_root, "tools", Path(args.def_file).name, (dict,), "对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)

    base_dir = Path(args.base_dir).resolve()
    if not base_dir.exists() and not args.dry_run:
        base_dir.mkdir(parents=True, exist_ok=True)

    if args.offline:
        logging.info("离线模式：跳过 git 更新检查")
    
    logging.info("同步全局工具 (策略驱动)...")
    all_ok = True
    for name, definition in def_cfg.items():
        if name not in policy_cfg:
            logging.info(f"[TOOL: {name}] 未声明策略，跳过。")
            continue
        # 检查前置依赖是否就绪
        requires = definition.get("requires", [])
        unmet = [
            req for req in requires
            if not _check_ready(req, (def_cfg.get(req) or {}).get("ready_check"))
        ]
        if unmet:
            logging.info(f"[TOOL: {name}] 前置依赖未就绪 {unmet}，跳过。")
            continue
        policy = policy_cfg.get(name, {})
        # 离线模式下禁用 auto_update
        if args.offline:
            policy = {**policy, "auto_update": False}
        if not sync_tool(name, definition, policy, base_dir, args.dry_run, args.force):
            all_ok = False
    
    if not args.dry_run: prune_orphan_tools(base_dir, def_cfg.keys(), args.dry_run, args.yes)
    if not all_ok: sys.exit(1)
    logging.info("工具同步任务完成！")

if __name__ == "__main__":
    main()
