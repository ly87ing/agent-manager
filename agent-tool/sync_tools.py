#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import shutil
import json
import sys
import time
from pathlib import Path

# 配置日志：统一审计时间戳
logging.basicConfig(
    level=logging.INFO, 
    format='%(asctime)s [%(levelname)s] %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)

def load_json(path):
    if not os.path.exists(path): return {}
    try:
        with open(path, 'r', encoding='utf-8') as f:
            return json.load(f)
    except: return {}

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
            shutil.copy2(src, dst)
            logging.info(f"  已覆盖: {rel_path}")

def prune_orphan_tools(base_dir, managed_names, dry_run=False, force_yes=False):
    """清理不再受管理的工具目录"""
    base_path = Path(base_dir).resolve()
    excluded = {"overrides", ".gitignore", "sync_tools.py", "__pycache__"}
    if not base_path.exists(): return

    for item in base_path.iterdir():
        if item.is_dir() and item.name not in managed_names and item.name not in excluded:
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

def sync_tool(name, definition, policy, base_dir, dry_run=False, force=False):
    # 合并策略
    enabled = policy.get("enabled", True)
    auto_update = policy.get("auto_update", True)
    
    if not enabled:
        logging.info(f"[TOOL: {name}] (策略已禁用，跳过)")
        return True

    url = definition.get("url")
    install_cmd = definition.get("install")
    setup_cmd = definition.get("setup")
    target_dir = Path(base_dir) / name
    needs_setup = force
    success_build = True

    if url:
        if target_dir.exists() and (target_dir / ".git").exists():
            if not auto_update:
                logging.info(f"[TOOL: {name}] 已就绪，auto_update 关闭。")
            else:
                if dry_run: logging.info(f"[TOOL: {name}] (预览: 计划检查更新)")
                else:
                    if git_cache.is_git_up_to_date(target_dir): logging.info(f"[TOOL: {name}] (内容已最新)")
                    else:
                        success, out = run_cmd(["git", "pull"], cwd=target_dir)
                        if success: logging.info(f"[TOOL: {name}] (已更新)"); needs_setup = True
                        else: logging.error(f"[TOOL: {name}] (更新失败: {out})"); success_build = False
        else:
            if target_dir.exists():
                logging.warning(f"[TOOL: {name}] 目录异常，正在重装...")
                if not dry_run: shutil.rmtree(target_dir)
            logging.info(f"正在安装 {name}...")
            if not dry_run:
                success, out = run_cmd(["git", "clone", url, str(target_dir)])
                if success: logging.info(f"[TOOL: {name}] (安装成功)"); needs_setup = True
                else: logging.error(f"[TOOL: {name}] (下载失败: {out})"); success_build = False
        
        if not dry_run: apply_overrides(name, target_dir, dry_run)
        if setup_cmd and needs_setup and not dry_run:
            logging.info(f"执行 {name} 初始化...")
            ok, out = run_cmd(setup_cmd, cwd=target_dir)
            if not ok: logging.error(f"[TOOL: {name}] 构建失败: {out}"); success_build = False

    elif install_cmd:
        if dry_run: logging.info(f"[TOOL: {name}] (预览: 计划执行安装)")
        else:
            logging.info(f"执行 {name} 全局安装...")
            ok, out = run_cmd(install_cmd)
            if not ok: logging.error(f"[TOOL: {name}] 安装失败: {out}"); success_build = False
                
    return success_build

def main():
    parser = argparse.ArgumentParser(description="Agent Tool 管理工具 (策略驱动)")
    parser.add_argument("--base-dir", default=".", help="基础目录")
    parser.add_argument("--config", default="../config/manager.json", help="策略路径")
    parser.add_argument("--def-file", default="../config/tools.json", help="定义路径")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--force", action="store_true", help="强制初始化")
    parser.add_argument("--yes", action="store_true", help="自动清理")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过网络操作)")
    args = parser.parse_args()

    policy_cfg = load_json(args.config).get("tools", {})
    def_cfg = load_json(args.def_file).get("tools", {})
    base_dir = Path(args.base_dir).resolve()

    if args.offline:
        logging.info("离线模式：跳过 git 更新检查")
    
    logging.info("同步全局工具 (策略驱动)...")
    all_ok = True
    for name, definition in def_cfg.items():
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
