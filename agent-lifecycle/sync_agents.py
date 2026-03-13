#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import json
import shutil
import sys
from pathlib import Path

# 配置日志
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

def run_cmd(cmd):
    """运行 shell 命令并返回结果"""
    try:
        result = subprocess.run(cmd, check=True, capture_output=True, text=True, shell=True)
        return True, result.stdout
    except subprocess.CalledProcessError as e:
        return False, f"{e.stderr}\n{e.stdout}"

def detect_all_providers(bin_name, config=None):
    """全量扫描当前系统中管理该 Agent 的所有 Provider"""
    found = []
    
    def get_pkg_name(cmd):
        import re
        if not cmd: return None
        m = re.search(r'(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)', cmd)
        return m.group(1) if m else None

    if config and "ext_name" in config:
        ext_name = config["ext_name"]
        if shutil.which("gh"):
            success, out = run_cmd("gh extension list")
            if success and any(line.strip().startswith(ext_name) for line in out.splitlines()):
                found.append("gh")

    brew_pkg = get_pkg_name(config.get('providers', {}).get('brew', {}).get('install', '')) if config else None
    if shutil.which("brew") and brew_pkg:
        if run_cmd(f"brew list --formula {brew_pkg} 2>/dev/null")[0]: found.append("brew")
        elif run_cmd(f"brew list --cask {brew_pkg} 2>/dev/null")[0]: found.append("brew-cask")
            
    bin_path = shutil.which(bin_name)
    if bin_path and ("/node/" in bin_path or "/npm/" in bin_path or bin_path.endswith("node") or bin_path.endswith("npm")):
        found.append("npm")
    
    app_path = config.get('app_path') if config else None
    if (app_path and os.path.exists(app_path)) or bin_path:
        if not found: found.append("system")
        
    return found

def reconcile_agent(name, definition, policy, dry_run=False):
    # 合并策略
    enabled = policy.get("enabled", True)
    auto_install = policy.get("auto_install", True)
    auto_update = policy.get("auto_update", True)
    
    preferred = policy.get("preferred_provider") or definition.get("preferred_provider", "npm")
    
    bin_name = definition.get("bin")
    providers = definition.get("providers", {})
    
    all_actual = detect_all_providers(bin_name, definition)
    is_installed = len(all_actual) > 0
    
    if len(all_actual) > 1:
        logging.warning(f"[AGENT: {name}] 发现多源安装冲突! 涉及: {', '.join(all_actual)}。")

    if enabled and not is_installed:
        if not auto_install:
            logging.info(f"[AGENT: {name}] 配置开启但 auto_install 关闭，跳过安装。")
            return True
        logging.info(f"[AGENT: {name}] 状态对齐: 准备通过 {preferred} 安装...")
        p_cfg = providers.get(preferred)
        if p_cfg:
            if dry_run: logging.info(f"  (预览: 计划执行 {p_cfg['install']})")
            else:
                success, out = run_cmd(p_cfg["install"])
                if success: logging.info(f"[AGENT: {name}] 安装成功。")
                else: logging.error(f"[AGENT: {name}] 安装失败: {out}"); return False
        return True

    elif not enabled and is_installed:
        logging.warning(f"[AGENT: {name}] 状态对齐: 配置已禁用，准备卸载...")
        for provider in all_actual:
            p_key = "brew" if "brew" in provider else provider
            p_cfg = providers.get(p_key)
            if p_cfg and "uninstall" in p_cfg:
                if dry_run: logging.info(f"  (预览: 计划执行 {p_cfg['uninstall']})")
                else:
                    success, out = run_cmd(p_cfg["uninstall"])
                    if success: logging.info(f"[AGENT: {name}] 从 {provider} 卸载成功。")
                    else: logging.error(f"[AGENT: {name}] 卸载失败: {out}")
        return True

    elif enabled and is_installed:
        if not auto_update:
            logging.info(f"[AGENT: {name}] 状态就绪，auto_update 关闭，跳过更新。")
            return True
        logging.info(f"[AGENT: {name}] 状态一致，正在尝试检查更新...")
        p_key = "brew" if "brew" in all_actual[0] else all_actual[0]
        p_cfg = providers.get(p_key)
        if p_cfg and "upgrade" in p_cfg:
            if dry_run: logging.info(f"  (预览: 计划执行 {p_cfg['upgrade']})")
            else:
                success, out = run_cmd(p_cfg["upgrade"])
                if success: logging.info(f"[AGENT: {name}] 已是最新 / 升级成功。")
                else: logging.warning(f"[AGENT: {name}] 升级尝试未完全成功: {out}")
    return True

def main():
    parser = argparse.ArgumentParser(description="Agent 生命周期对齐工具")
    parser.add_argument("--config", default="../config/manager.json", help="策略配置路径")
    parser.add_argument("--def-file", default="../config/agents.json", help="定义文件路径")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--yes", action="store_true", help="自动确认")
    args = parser.parse_args()

    policy_cfg = load_json(args.config).get("agents", {})
    def_cfg = load_json(args.def_file)

    logging.info("正在审计 Agent 生命周期意愿清单 (策略驱动)...")
    all_ok = True
    for name, definition in def_cfg.items():
        policy = policy_cfg.get(name, {})
        if not reconcile_agent(name, definition, policy, args.dry_run):
            all_ok = False
            
    if not all_ok: sys.exit(1)
    logging.info("Agent 状态对齐任务完成。")

if __name__ == "__main__":
    main()
