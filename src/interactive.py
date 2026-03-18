#!/usr/bin/env python3
import json
import os
import sys
import tempfile
import shutil
from pathlib import Path
import copy

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.json_config import atomic_write_json_file, load_json_file

# 颜色定义
RED = "\033[0;31m"
ERROR_RED = "\033[0;31m"
GREEN = "\033[0;32m"
YELLOW = "\033[0;33m"
BLUE = "\033[1;34m"
CYAN = "\033[0;36m"
MAGENTA = "\033[0;35m"
BOLD = "\033[1m"
NC = "\033[0m"
HDR_BLUE = "\033[1;34m"
WHITE = "\033[1;37m"
SUCCESS_GREEN = "\033[0;32m"
PURPLE = "\033[0;35m"
PENDING_GOLD = "\033[0;33m"

# 全局同步标记
SYNC_REQUIRED = False

def mark_sync_required():
    global SYNC_REQUIRED
    SYNC_REQUIRED = True

# --- 基础工具 ---

def ask(prompt, default=None):
    if default is not None:
        res = input(f"{BOLD}{prompt}{NC} [{YELLOW}{default}{NC}]: ").strip()
        return res if res else default
    return input(f"{BOLD}{prompt}{NC}: ").strip()

def ask_yes_no(prompt, default="y"):
    default_value = default if isinstance(default, str) else ("y" if default else "n")
    res = input(f"{BOLD}{prompt}{NC} (y/n) [{YELLOW}{default_value}{NC}]: ").strip().lower()
    if not res:
        res = default_value
    return res == 'y'

def load_json(path, *, strict=False, missing_ok=True):
    return load_json_file(path, strict=strict, missing_ok=missing_ok)

def save_json(data, path):
    atomic_write_json_file(path, data)

def get_config_root():
    cwd_root = Path.cwd()
    if (cwd_root / "config" / "manager.json").exists():
        return cwd_root
    return REPO_ROOT

def deep_update(dst, src):
    for k, v in src.items():
        if isinstance(v, dict) and isinstance(dst.get(k), dict):
            deep_update(dst[k], v)
        else:
            dst[k] = v

def get_base_config():
    base_path = get_config_root() / "config" / "manager.json"
    base = load_json(base_path, strict=True, missing_ok=False)
    if "agents" in base:
        base["agents"] = canonicalize_agent_mapping(base.get("agents", {}))
    return base

def get_local_config():
    local_path = get_config_root() / "config" / "manager.local.json"
    # Ensure file exists
    if not local_path.exists():
        save_json({}, local_path)
    local = load_json(local_path, strict=True)
    if "agents" in local:
        local["agents"] = canonicalize_agent_mapping(local.get("agents", {}))
    return local

def get_effective_config():
    base = get_base_config()
    local = get_local_config()
    return merge_config(base, local)

def merge_config(base, local):
    merged = copy.deepcopy(base)
    deep_update(merged, local)
    if "agents" in merged:
        merged["agents"] = canonicalize_agent_mapping(merged.get("agents", {}))
    return merged

def get_policy(): 
    # 返回合并后的策略用于显示
    return get_effective_config()

def save_policy(data): 
    # 只保存到用户侧覆盖文件
    local_path = get_config_root() / "config" / "manager.local.json"
    save_json(data, local_path)

# --- 交互式界面 ---

def main():
    # 确保本地配置文件存在
    local_path = get_config_root() / "config" / "manager.local.json"
    if not local_path.exists():
        save_json({}, local_path)
    
    try:
        local_policy = get_local_config()
        get_base_config()
    except ValueError as exc:
        print(f"{ERROR_RED}{exc}{NC}")
        raise SystemExit(1)

    print(f"{HDR_BLUE}{BOLD}Agent Manager 交互式配置中心{NC}")
    print("=" * 50)

    while True:
        print("\n请选择操作:")
        print("1. 查看当前策略（合并后）")
        print("2. 编辑 Agent 策略")
        print("3. 编辑 Tool 策略")
        print("4. 编辑 Skill 策略")
        print("5. 编辑 MCP 策略")
        print("6. 保存更改")
        print("7. 退出而不保存")
        
        choice = ask("请输入选项编号", "1")
        
        if choice == "1":
            policy = merge_config(get_base_config(), local_policy)
            print(json.dumps(policy, indent=2, ensure_ascii=False))
        elif choice == "2":
            edit_section("agents", "Agent", local_policy)
        elif choice == "3":
            edit_section("tools", "Tool", local_policy)
        elif choice == "4":
            edit_section("skills", "Skill", local_policy)
        elif choice == "5":
            edit_section("mcp", "MCP", local_policy)
        elif choice == "6":
            save_policy(local_policy)
            print(
                f"{SUCCESS_GREEN}配置已保存到用户侧覆盖文件，请运行 ./manage.sh update 以应用更改；"
                f"若需安装缺失 Agent，请改用 ./manage.sh bootstrap。{NC}"
            )
            break
        elif choice == "7":
            print("已放弃更改。")
            break
        else:
            print(f"{ERROR_RED}无效选项，请重新选择。{NC}")

def edit_section(section_key, section_name, local_policy):
    # For editing we work on the local config directly, but we need to show current effective values
    effective = merge_config(get_base_config(), local_policy)
    local = local_policy
    section_eff = effective.get(section_key, {})
    section_loc = local.get(section_key, {})
    if not section_eff:
        print(f"{YELLOW}暂无{section_name}策略。{NC}")
        return
    
    print(f"\n{section_name}策略列表:")
    keys = list(section_eff.keys())
    for i, k in enumerate(keys, 1):
        label = get_agent_display_name(k) if section_key == "agents" else k
        print(f"  {i}. {label}")
    print("  0. 返回")
    
    idx = ask("请选择要编辑的条目编号 (0 返回)", "0")
    try:
        idx_int = int(idx)
        if idx_int == 0:
            return
        if not (1 <= idx_int <= len(keys)):
            print(f"{ERROR_RED}无效编号。{NC}")
            return
    except ValueError:
        print(f"{ERROR_RED}请输入数字。{NC}")
        return
    
    key = keys[idx_int - 1]
    label = get_agent_display_name(key) if section_key == "agents" else key
    # Current effective value
    current_eff_val = section_eff.get(key, {})
    # Current local value (may be missing)
    current_loc_val = section_loc.get(key, {})
    print(f"\n正在编辑 {section_name}: {label}")
    print(f"当前有效值: {json.dumps(current_eff_val, indent=2, ensure_ascii=False)}")
    print(f"当前本地值: {json.dumps(current_loc_val, indent=2, ensure_ascii=False)}")
    
    # 简单的键值编辑：根据 section_key 决定是否有构建类配置
    enabled = ask_yes_no(f"启用 {label}?", current_eff_val.get("enabled", True))
    
    new_val = copy.deepcopy(current_loc_val)
    new_val["enabled"] = enabled
    
    if section_key == "agents":
        auto_install = ask_yes_no(f"自动安装 {label}?", current_eff_val.get("auto_install", True))
        auto_update = ask_yes_no(f"自动更新 {label}?", current_eff_val.get("auto_update", True))
        new_val["auto_install"] = auto_install
        new_val["auto_update"] = auto_update
    elif section_key == "tools":
        auto_update = ask_yes_no(f"自动更新 {label}?", current_eff_val.get("auto_update", True))
        new_val["auto_update"] = auto_update
    
    # Update local config only
    if section_key not in local:
        local[section_key] = {}
    local[section_key][key] = new_val
    # Update SYNC_REQUIRED flag
    global SYNC_REQUIRED
    SYNC_REQUIRED = True
    print(f"{SUCCESS_GREEN}{section_name} {label} 已更新，等待保存。{NC}")

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n已中断。")
        sys.exit(1)
