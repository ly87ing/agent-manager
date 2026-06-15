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
from src.json_config import atomic_write_json_file, ensure_mapping_value_types, get_typed_mapping_section, load_json_object_file

# Color definitions
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

# Global sync flag
SYNC_REQUIRED = False

def mark_sync_required():
    global SYNC_REQUIRED
    SYNC_REQUIRED = True

# --- Basic utilities ---

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
    return load_json_object_file(path, strict=strict, missing_ok=missing_ok)

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
    base["agents"] = canonicalize_agent_mapping(
        get_typed_mapping_section(base, "agents", "manager.json", (dict,), "对象")
    )
    base["tools"] = get_typed_mapping_section(base, "tools", "manager.json", (dict,), "对象")
    base["skills"] = get_typed_mapping_section(base, "skills", "manager.json", (dict,), "对象")
    base["mcp"] = get_typed_mapping_section(base, "mcp", "manager.json", (dict,), "对象")
    return base


def get_agent_definitions():
    defs_path = get_config_root() / "config" / "agents.json"
    if not defs_path.exists():
        return {}
    defs = load_json(defs_path, strict=True, missing_ok=False)
    return canonicalize_agent_mapping(
        ensure_mapping_value_types(defs, "agents.json", (dict,), "对象")
    )

def get_local_config():
    local_path = get_config_root() / "config" / "manager.local.json"
    # Ensure file exists
    if not local_path.exists():
        save_json({}, local_path)
    local = load_json(local_path, strict=True)
    if "agents" in local:
        local["agents"] = canonicalize_agent_mapping(
            get_typed_mapping_section(local, "agents", "manager.local.json", (dict,), "对象")
        )
    if "tools" in local:
        local["tools"] = get_typed_mapping_section(local, "tools", "manager.local.json", (dict,), "对象")
    if "skills" in local:
        local["skills"] = get_typed_mapping_section(local, "skills", "manager.local.json", (dict,), "对象")
    if "mcp" in local:
        local["mcp"] = get_typed_mapping_section(local, "mcp", "manager.local.json", (dict,), "对象")
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
    # Return the merged policy for display
    return get_effective_config()

def save_policy(data):
    # Save only to the user-side override file
    local_path = get_config_root() / "config" / "manager.local.json"
    save_json(data, local_path)

# --- Interactive interface ---

def main():
    # Ensure the local config file exists
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
    print(f"当前编辑文件: {get_config_root() / 'config' / 'manager.local.json'}")
    print("基线策略来自 config/manager.json；此界面只写本地覆盖层。")

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
            if SYNC_REQUIRED:
                confirm = ask_yes_no("有未保存的更改，确认放弃？", "n")
                if not confirm:
                    continue
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
    
    # Simple key-value editing: decide whether build-type config applies based on section_key
    enabled = ask_yes_no(f"启用 {label}?", current_eff_val.get("enabled", True))
    
    new_val = copy.deepcopy(current_loc_val)
    new_val["enabled"] = enabled
    
    if section_key == "agents":
        auto_install = ask_yes_no(f"自动安装 {label}?", current_eff_val.get("auto_install", True))
        auto_update = ask_yes_no(f"自动更新 {label}?", current_eff_val.get("auto_update", True))
        new_val["auto_install"] = auto_install
        new_val["auto_update"] = auto_update
        agent_defs = get_agent_definitions()
        provider_defs = (agent_defs.get(key) or {}).get("providers") or {}
        provider_options = list(provider_defs.keys())
        if provider_options:
            default_provider = current_eff_val.get("preferred_provider") or (agent_defs.get(key) or {}).get("preferred_provider") or provider_options[0]
            while True:
                provider = ask(f"首选安装来源 {provider_options}", default_provider)
                if provider in provider_options:
                    new_val["preferred_provider"] = provider
                    break
                print(f"{ERROR_RED}无效 provider，请输入 {provider_options} 之一。{NC}")
    elif section_key == "tools":
        auto_update = ask_yes_no(f"自动更新 {label}?", current_eff_val.get("auto_update", True))
        new_val["auto_update"] = auto_update
    
    # Update local config only
    if section_key not in local:
        local[section_key] = {}
    local[section_key][key] = new_val
    # Update SYNC_REQUIRED flag
    mark_sync_required()
    print(f"{SUCCESS_GREEN}{section_name} {label} 已更新，等待保存。{NC}")

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n已中断。")
        sys.exit(1)
