#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import json
import shutil
import sys
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.json_config import ensure_mapping_value_types, get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import (
    ProviderInventory as ProviderCache,
    detect_all_providers as detect_all_providers_runtime,
    get_pkg_name,
)
from src.runtime_logging import configure_logging

configure_logging()

def run_cmd(cmd, timeout=300):
    """运行 shell 命令并返回结果"""
    try:
        # If cmd is a list, we do not use shell
        if isinstance(cmd, list):
            result = subprocess.run(cmd, check=True, capture_output=True, text=True, shell=False, timeout=timeout)
        else:
            # Assume it's a string and we need shell for complex commands
            result = subprocess.run(cmd, check=True, capture_output=True, text=True, shell=True, timeout=timeout)
        return True, result.stdout
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT after {timeout} seconds: {cmd}"
    except subprocess.CalledProcessError as e:
        return False, f"Command '{cmd}' failed with exit code {e.returncode}: {e.stderr}"
    except FileNotFoundError as e:
        return False, f"Directory or executable not found: {e}"
    except Exception as e:
        return False, f"Unexpected error executing '{cmd}': {e}"


def detect_all_providers(bin_name, config=None):
    """委托共享 runtime probe 执行 Provider 探测，避免实现漂移。"""
    return detect_all_providers_runtime(bin_name, config, inventory=ProviderCache())

def canonical_provider_name(provider_name):
    if provider_name.startswith("brew"):
        return "brew"
    return provider_name

def classify_provider_state(actual_providers, definition):
    managed_defs = definition.get("providers", {})
    managed = []
    system_only = []
    unsupported = []
    for provider in actual_providers:
        canonical = canonical_provider_name(provider)
        if provider == "system":
            system_only.append(provider)
        elif canonical in managed_defs:
            managed.append(provider)
        else:
            unsupported.append(provider)
    return managed, system_only, unsupported

def detect_provider_state(bin_name, definition):
    actual = detect_all_providers(bin_name, definition)
    managed, system_only, unsupported = classify_provider_state(actual, definition)
    return {
        "all": actual,
        "managed": managed,
        "system_only": system_only,
        "unsupported": unsupported,
    }

def get_update_probe_cmd(provider_name, provider_cfg):
    if not provider_cfg:
        return None
    explicit = provider_cfg.get("check_update")
    if explicit:
        return explicit
    if provider_name.startswith("brew"):
        pkg_name = get_pkg_name(provider_cfg.get("install", ""))
        if not pkg_name:
            return None
        if provider_name == "brew-cask":
            return f"brew outdated --cask {pkg_name}"
        return f"brew outdated {pkg_name}"
    return None

def resolve_active_provider_variant(managed_actual, provider_key):
    for provider_name in managed_actual:
        if canonical_provider_name(provider_name) == provider_key:
            return provider_name
    return provider_key

def reconcile_agent(agent_id, definition, policy, dry_run=False, allow_install=True):
    label = get_agent_display_name(agent_id, definition)
    # 合并策略
    enabled = policy.get("enabled", True)
    auto_install = policy.get("auto_install", True)
    auto_update = policy.get("auto_update", True)
    
    preferred = policy.get("preferred_provider") or definition.get("preferred_provider", "npm")
    
    bin_name = definition.get("bin")
    providers = definition.get("providers", {})

    if preferred not in providers:
        logging.error(
            f"[AGENT: {label}] invalid preferred_provider='{preferred}'，可选值: {', '.join(sorted(providers)) or '无'}"
        )
        return False
    
    state = detect_provider_state(bin_name, definition)
    all_actual = state["all"]
    managed_actual = state["managed"]
    system_actual = state["system_only"]
    unsupported_actual = state["unsupported"]
    is_installed = len(all_actual) > 0
    
    if len(managed_actual) > 1:
        logging.warning(f"[AGENT: {label}] 发现多源安装冲突! 涉及: {', '.join(managed_actual)}。")

    if unsupported_actual:
        logging.error(f"[AGENT: {label}] 检测到未声明的 Provider: {', '.join(unsupported_actual)}。")
        return False

    if system_actual and not managed_actual:
        if enabled:
            logging.info(f"[AGENT: {label}] 仅检测到系统安装，视为外部存在，不执行托管安装/更新。")
        else:
            logging.info(f"[AGENT: {label}] 仅检测到系统安装，视为外部存在，不执行卸载。")
        return True

    if enabled and not is_installed:
        if not allow_install:
            logging.info(f"[AGENT: {label}] 禁止安装缺失 Agent，跳过安装。")
            return True
        if not auto_install:
            logging.info(f"[AGENT: {label}] 配置开启但 auto_install 关闭，跳过安装。")
            return True
        logging.info(f"[AGENT: {label}] 状态对齐: 准备通过 {preferred} 安装...")
        p_cfg = providers.get(preferred)
        if not p_cfg or "install" not in p_cfg:
            logging.error(f"[AGENT: {label}] Provider '{preferred}' 缺少 install 命令。")
            return False
        if dry_run:
            logging.info(f"  (预览: 计划执行 {p_cfg['install']})")
        else:
            success, out = run_cmd(p_cfg["install"])
            if success:
                post_state = detect_provider_state(bin_name, definition)
                if not post_state["managed"]:
                    logging.error(f"[AGENT: {label}] 安装后复核失败：仍未检测到受管 Provider。")
                    return False
                logging.info(f"[AGENT: {label}] 安装成功。")
            else:
                logging.error(f"[AGENT: {label}] 安装失败: {out}")
                return False
        return True

    elif not enabled and is_installed:
        if not managed_actual:
            logging.info(f"[AGENT: {label}] 检测到外部安装，但当前无受管 Provider，可跳过卸载。")
            return True
        logging.warning(f"[AGENT: {label}] 状态对齐: 配置已禁用，准备卸载...")
        all_ok = True
        for provider in managed_actual:
            p_key = canonical_provider_name(provider)
            p_cfg = providers.get(p_key)
            if not p_cfg or "uninstall" not in p_cfg:
                logging.error(f"[AGENT: {label}] Provider '{provider}' 缺少 uninstall 命令。")
                all_ok = False
                continue
            if dry_run:
                logging.info(f"  (预览: 计划执行 {p_cfg['uninstall']})")
            else:
                success, out = run_cmd(p_cfg["uninstall"])
                if success:
                    logging.info(f"[AGENT: {label}] 从 {provider} 卸载成功。")
                else:
                    logging.error(f"[AGENT: {label}] 卸载失败: {out}")
                    all_ok = False
        if not all_ok or dry_run:
            return all_ok
        post_state = detect_provider_state(bin_name, definition)
        if post_state["managed"]:
            logging.error(f"[AGENT: {label}] 卸载后复核失败：仍检测到受管 Provider: {', '.join(post_state['managed'])}。")
            return False
        return True

    elif enabled and is_installed:
        if not managed_actual:
            logging.info(f"[AGENT: {label}] 检测到外部安装，但当前无受管 Provider，跳过更新。")
            return True
        if not auto_update:
            logging.info(f"[AGENT: {label}] 状态就绪，auto_update 关闭，跳过更新。")
            return True
        logging.info(f"[AGENT: {label}] 状态一致，正在尝试检查更新...")
        actual_provider_keys = [canonical_provider_name(provider) for provider in managed_actual]
        p_key = preferred if preferred in actual_provider_keys else actual_provider_keys[0]
        active_provider = resolve_active_provider_variant(managed_actual, p_key)
        p_cfg = providers.get(p_key)
        if p_cfg and "upgrade" in p_cfg:
            if dry_run:
                logging.info(f"  (预览: 计划执行 {p_cfg['upgrade']})")
            else:
                probe_cmd = get_update_probe_cmd(active_provider, p_cfg)
                if probe_cmd:
                    probe_success, probe_out = run_cmd(probe_cmd, timeout=60)
                    if probe_success:
                        if not probe_out.strip():
                            logging.info(f"[AGENT: {label}] 未发现可用更新，跳过升级。")
                            return True
                    else:
                        logging.warning(f"[AGENT: {label}] 更新探测失败，跳过升级: {probe_out}")
                        return True
                success, out = run_cmd(p_cfg["upgrade"])
                if success:
                    post_state = detect_provider_state(bin_name, definition)
                    if not post_state["managed"]:
                        logging.error(f"[AGENT: {label}] 升级后复核失败：受管 Provider 已丢失。")
                        return False
                    logging.info(f"[AGENT: {label}] 已是最新 / 升级成功。")
                else:
                    logging.error(f"[AGENT: {label}] 升级失败: {out}")
                    return False
    return True

def main():
    parser = argparse.ArgumentParser(description="Agent 生命周期对齐工具")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略配置路径")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "agents.json"), help="定义文件路径")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--yes", action="store_true", help="自动确认")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过网络操作)")
    parser.add_argument("--no-install", action="store_true", help="禁止安装缺失的 Agent，仅做更新/卸载/审计")
    args = parser.parse_args()

    try:
        policy_root = load_json_object_file(args.config, strict=True, missing_ok=False)
        def_root = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        policy_cfg = canonicalize_agent_mapping(
            get_typed_mapping_section(policy_root, "agents", Path(args.config).name, (dict,), "对象")
        )
        def_cfg = canonicalize_agent_mapping(
            ensure_mapping_value_types(def_root, Path(args.def_file).name, (dict,), "对象")
        )
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)

    if args.offline:
        logging.info("离线模式：跳过更新检查")
    
    logging.info("正在审计 Agent 生命周期意愿清单 (策略驱动)...")
    all_ok = True
    for agent_id, definition in def_cfg.items():
        label = get_agent_display_name(agent_id, definition)
        if agent_id not in policy_cfg:
            logging.info(f"[AGENT: {label}] 未声明策略，跳过。")
            continue
        policy = policy_cfg.get(agent_id, {})
        # 离线模式下禁用 auto_update
        if args.offline:
            policy = {**policy, "auto_update": False}
        if not reconcile_agent(agent_id, definition, policy, args.dry_run, allow_install=not args.no_install):
            all_ok = False
            
    if not all_ok: sys.exit(1)
    logging.info("Agent 状态对齐任务完成。")

if __name__ == "__main__":
    main()
