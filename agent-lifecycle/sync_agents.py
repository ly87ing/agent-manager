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
from src.command_runner import run_command
from src.json_config import ensure_mapping_value_types, get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import (
    ProviderInventory as ProviderCache,
    canonical_provider_name,
    classify_provider_state,
    detect_all_providers as detect_all_providers_runtime,
    get_pkg_name,
    is_agent_installed,
    resolve_provider_command,
)
from src.runtime_logging import configure_logging

configure_logging()

SUPPORTED_UNSUPPORTED_ACTIONS = {"install", "update", "uninstall", "repair"}
NETWORK_ERROR_MARKERS = (
    "ENOTFOUND",
    "ECONNREFUSED",
    "ECONNRESET",
    "ETIMEDOUT",
    "EAI_AGAIN",
    "Could not resolve host",
    "Failed to fetch",
    "getaddrinfo",
    "network connectivity",
)
OFFICIAL_NPM_REGISTRY = "https://registry.npmjs.org"
OFFICIAL_INSTALLER_URL_MARKERS = (
    "https://claude.ai/install",
    "https://cursor.com/install",
    "https://opencode.ai/install",
    "https://code.kimi.com/install.sh",
)


def _inject_npm_flags(cmd):
    if not isinstance(cmd, str):
        return cmd

    stripped = cmd.strip()
    if not stripped.startswith("npm install -g"):
        return stripped

    hardened = stripped
    npm_flags = (
        ("--registry", OFFICIAL_NPM_REGISTRY),
        ("--fetch-retries", "2"),
        ("--fetch-retry-factor", "2"),
        ("--fetch-retry-mintimeout", "10000"),
        ("--fetch-retry-maxtimeout", "60000"),
        ("--fetch-timeout", "120000"),
    )
    for flag, value in npm_flags:
        if f"{flag} " in hardened or f"{flag}=" in hardened:
            continue
        hardened = f"{hardened} {flag} {value}"
    if "--no-audit" not in hardened:
        hardened = f"{hardened} --no-audit"
    if "--no-fund" not in hardened:
        hardened = f"{hardened} --no-fund"
    return hardened


def _inject_curl_retry_flags(cmd):
    if not isinstance(cmd, str):
        return cmd

    stripped = cmd.strip()
    if not stripped.startswith("curl "):
        return stripped
    if not any(marker in stripped for marker in OFFICIAL_INSTALLER_URL_MARKERS):
        return stripped
    if "--retry " in stripped or "--retry=" in stripped:
        return stripped
    return stripped.replace(
        "curl ",
        "curl --retry 3 --retry-delay 2 --retry-all-errors --connect-timeout 15 --max-time 180 ",
        1,
    )


def _prepare_command(cmd):
    prepared = _inject_npm_flags(cmd)
    return _inject_curl_retry_flags(prepared)


def is_network_error(output):
    return any(marker in (output or "") for marker in NETWORK_ERROR_MARKERS)


def run_cmd(cmd, timeout=300):
    """运行 shell 命令并返回结果"""
    cmd = _prepare_command(cmd)
    try:
        result = run_command(cmd, check=True, capture_output=True, text=True, timeout=timeout)
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

def detect_provider_state(bin_name, definition):
    actual = detect_all_providers(bin_name, definition)
    state = classify_provider_state(actual, definition)
    state["all"] = actual
    return state


def get_unsupported_actions(provider_cfg):
    if not provider_cfg or "unsupported_actions" not in provider_cfg:
        return set()

    raw = provider_cfg.get("unsupported_actions")
    if not isinstance(raw, list):
        raise ValueError("unsupported_actions 必须是字符串列表。")

    unsupported_actions = set()
    invalid_actions = []
    for action in raw:
        if isinstance(action, str) and action in SUPPORTED_UNSUPPORTED_ACTIONS:
            unsupported_actions.add(action)
        else:
            invalid_actions.append(repr(action))

    if invalid_actions:
        raise ValueError(
            "unsupported_actions 仅支持 "
            f"{', '.join(sorted(SUPPORTED_UNSUPPORTED_ACTIONS))}；收到: {', '.join(invalid_actions)}。"
        )

    return unsupported_actions


def get_validated_unsupported_actions(label, provider_name, provider_cfg):
    try:
        return get_unsupported_actions(provider_cfg)
    except ValueError as exc:
        logging.error(f"[AGENT: {label}] Provider '{provider_name}' unsupported_actions 配置非法: {exc}")
        return None


def action_is_officially_unsupported(provider_cfg, action, unsupported_actions=None):
    actions = unsupported_actions if unsupported_actions is not None else get_unsupported_actions(provider_cfg)
    return action in actions


def log_official_gap(label, provider_name, action):
    logging.error(
        f"[AGENT: {label}] Provider '{provider_name}' 官方当前未提供受管 {action} 动作。"
    )


def get_update_probe_cmd(provider_name, provider_cfg):
    if not provider_cfg:
        return None
    explicit = resolve_provider_command(provider_cfg.get("check_update"))
    if explicit:
        return explicit
    if provider_name.startswith("brew"):
        install_cmd = resolve_provider_command(provider_cfg.get("install"))
        pkg_name = get_pkg_name(install_cmd or "")
        if not pkg_name:
            return None
        if provider_name == "brew-cask":
            return f"brew outdated --cask {pkg_name}"
        return f"brew outdated {pkg_name}"
    return None


def get_repair_cmd(provider_name, provider_cfg, unsupported_actions=None):
    if not provider_cfg:
        return None
    active_unsupported_actions = (
        get_unsupported_actions(provider_cfg) if unsupported_actions is None else unsupported_actions
    )
    if not action_is_officially_unsupported(provider_cfg, "repair", active_unsupported_actions):
        explicit = resolve_provider_command(provider_cfg.get("repair"))
        if explicit:
            return explicit

    install_cmd = None
    if not action_is_officially_unsupported(provider_cfg, "install", active_unsupported_actions):
        install_cmd = resolve_provider_command(provider_cfg.get("install"))
        if provider_name.startswith("brew"):
            pkg_name = get_pkg_name(install_cmd or "")
            if not pkg_name:
                return None
            if provider_name == "brew-cask":
                return f"brew reinstall --cask {pkg_name}"
            return f"brew reinstall {pkg_name}"

    if not action_is_officially_unsupported(provider_cfg, "update", active_unsupported_actions):
        upgrade_cmd = resolve_provider_command(provider_cfg.get("upgrade"))
        if upgrade_cmd:
            return upgrade_cmd
    return install_cmd

def resolve_active_provider_variant(managed_actual, provider_key):
    for provider_name in managed_actual:
        if canonical_provider_name(provider_name) == provider_key:
            return provider_name
    return provider_key

def reconcile_agent(agent_id, definition, policy, dry_run=False, allow_install=True, offline=False):
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
    deprecated_actual = state["deprecated"]
    system_actual = state["system_only"]
    unsupported_actual = state["unsupported"]
    is_installed = len(all_actual) > 0
    runtime_ready = is_agent_installed(agent_id, definition)
    
    if len(managed_actual) > 1:
        logging.warning(f"[AGENT: {label}] 发现多源安装冲突! 涉及: {', '.join(managed_actual)}。")

    if unsupported_actual:
        logging.error(f"[AGENT: {label}] 检测到未声明的 Provider: {', '.join(unsupported_actual)}。")
        return False

    if deprecated_actual:
        logging.error(f"[AGENT: {label}] 检测到已弃用安装来源: {', '.join(deprecated_actual)}；请先迁移到受管 provider。")
        return False

    if system_actual and not managed_actual and definition.get("require_managed_provider", False):
        logging.error(f"[AGENT: {label}] 检测到未受管外部安装，当前策略要求受管 provider。")
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
        if offline:
            if dry_run:
                logging.info(f"[AGENT: {label}] (预览: 离线模式会阻止安装缺失 Agent)")
                return True
            logging.error(f"[AGENT: {label}] 离线模式且 Agent 缺失，禁止安装。")
            return False
        logging.info(f"[AGENT: {label}] 状态对齐: 准备通过 {preferred} 安装...")
        p_cfg = providers.get(preferred)
        unsupported_actions = get_validated_unsupported_actions(label, preferred, p_cfg)
        if unsupported_actions is None:
            return False
        if action_is_officially_unsupported(p_cfg, "install", unsupported_actions):
            log_official_gap(label, preferred, "install")
            return False
        install_cmd = resolve_provider_command(p_cfg.get("install")) if p_cfg else None
        if not install_cmd:
            logging.error(f"[AGENT: {label}] Provider '{preferred}' 缺少 install 命令。")
            return False
        if dry_run:
            logging.info(f"  (预览: 计划执行 {install_cmd})")
        else:
            success, out = run_cmd(install_cmd)
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
        managed_provider_entries = []
        for provider in managed_actual:
            p_key = canonical_provider_name(provider)
            p_cfg = providers.get(p_key)
            unsupported_actions = get_validated_unsupported_actions(label, provider, p_cfg)
            if unsupported_actions is None:
                return False
            managed_provider_entries.append((provider, p_cfg, unsupported_actions))
        all_ok = True
        for provider, p_cfg, unsupported_actions in managed_provider_entries:
            if action_is_officially_unsupported(p_cfg, "uninstall", unsupported_actions):
                log_official_gap(label, provider, "uninstall")
                all_ok = False
                continue
            uninstall_cmd = resolve_provider_command(p_cfg.get("uninstall")) if p_cfg else None
            if not uninstall_cmd:
                logging.error(f"[AGENT: {label}] Provider '{provider}' 缺少 uninstall 命令。")
                all_ok = False
                continue
            if dry_run:
                logging.info(f"  (预览: 计划执行 {uninstall_cmd})")
            else:
                success, out = run_cmd(uninstall_cmd)
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
        if not runtime_ready:
            if offline:
                if dry_run:
                    logging.info(f"[AGENT: {label}] (预览: 离线模式会阻止修复缺失的运行入口)")
                    return True
                logging.error(f"[AGENT: {label}] 离线模式下禁止修复缺失的运行入口。")
                return False
            logging.warning(f"[AGENT: {label}] 检测到受管 Provider，但运行入口缺失，准备尝试修复...")
            actual_provider_keys = [canonical_provider_name(provider) for provider in managed_actual]
            p_key = preferred if preferred in actual_provider_keys else actual_provider_keys[0]
            active_provider = resolve_active_provider_variant(managed_actual, p_key)
            p_cfg = providers.get(p_key)
            unsupported_actions = get_validated_unsupported_actions(label, active_provider, p_cfg)
            if unsupported_actions is None:
                return False
            repair_cmd = get_repair_cmd(active_provider, p_cfg, unsupported_actions)
            if not repair_cmd:
                if {"repair", "update", "install"}.issubset(unsupported_actions):
                    logging.error(
                        f"[AGENT: {label}] Provider '{active_provider}' 官方当前未提供受管 repair / update / install 动作，无法修复。"
                    )
                else:
                    logging.error(f"[AGENT: {label}] Provider '{active_provider}' 缺少 repair/install/upgrade 命令，无法修复。")
                return False
            if dry_run:
                logging.info(f"  (预览: 计划执行 {repair_cmd})")
                return True
            success, out = run_cmd(repair_cmd)
            if not success:
                logging.error(f"[AGENT: {label}] 修复失败: {out}")
                return False
            post_state = detect_provider_state(bin_name, definition)
            if not post_state["managed"]:
                logging.error(f"[AGENT: {label}] 修复后复核失败：受管 Provider 已丢失。")
                return False
            if not is_agent_installed(agent_id, definition):
                logging.error(f"[AGENT: {label}] 修复后复核失败：运行入口仍缺失。")
                return False
            logging.info(f"[AGENT: {label}] 修复成功。")
            return True
        if not auto_update:
            logging.info(f"[AGENT: {label}] 状态就绪，auto_update 关闭，跳过更新。")
            return True
        actual_provider_keys = [canonical_provider_name(provider) for provider in managed_actual]
        p_key = preferred if preferred in actual_provider_keys else actual_provider_keys[0]
        active_provider = resolve_active_provider_variant(managed_actual, p_key)
        p_cfg = providers.get(p_key)
        unsupported_actions = get_validated_unsupported_actions(label, active_provider, p_cfg)
        if unsupported_actions is None:
            return False
        if action_is_officially_unsupported(p_cfg, "update", unsupported_actions):
            log_official_gap(label, active_provider, "update")
            return False
        upgrade_cmd = resolve_provider_command(p_cfg.get("upgrade")) if p_cfg else None
        if p_cfg and "upgrade" in p_cfg and not upgrade_cmd:
            logging.error(f"[AGENT: {label}] Provider '{active_provider}' 缺少 upgrade 命令。")
            return False
        if upgrade_cmd:
            if dry_run:
                logging.info(f"  (预览: 计划执行 {upgrade_cmd})")
            else:
                probe_cmd = get_update_probe_cmd(active_provider, p_cfg)
                if probe_cmd:
                    logging.info(f"[AGENT: {label}] 状态一致，正在检查是否有可用更新...")
                    probe_success, probe_out = run_cmd(probe_cmd, timeout=60)
                    if probe_success:
                        if not probe_out.strip():
                            logging.info(f"[AGENT: {label}] 未发现可用更新，跳过升级。")
                            return True
                    else:
                        logging.warning(f"[AGENT: {label}] 更新探测失败，跳过升级: {probe_out}")
                        return True
                else:
                    logging.info(f"[AGENT: {label}] 状态一致，未配置轻量探测，正在执行升级命令...")
                success, out = run_cmd(upgrade_cmd)
                if success:
                    post_state = detect_provider_state(bin_name, definition)
                    if not post_state["managed"]:
                        logging.error(f"[AGENT: {label}] 升级后复核失败：受管 Provider 已丢失。")
                        return False
                    logging.info(f"[AGENT: {label}] 已是最新 / 升级成功。")
                else:
                    if is_network_error(out):
                        logging.warning(f"[AGENT: {label}] 升级遇到网络错误，跳过升级: {out}")
                        return True
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
        if not reconcile_agent(
            agent_id,
            definition,
            policy,
            args.dry_run,
            allow_install=not args.no_install,
            offline=args.offline,
        ):
            all_ok = False
            
    if not all_ok: sys.exit(1)
    logging.info("Agent 状态对齐任务完成。")

if __name__ == "__main__":
    main()
