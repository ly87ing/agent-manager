#!/usr/bin/env python3
import json
import os
import sys
import tempfile
import shutil
from pathlib import Path

# 颜色定义
RED = "\033[0;31m"
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
    res = input(f"{BOLD}{prompt}{NC} (y/n) [{YELLOW}{default}{NC}]: ").strip().lower()
    if not res: res = default
    return res == 'y'

def load_json(path):
    if not os.path.exists(path): return {}
    try:
        with open(path, 'r', encoding='utf-8') as f:
            return json.load(f)
    except: return {}

def save_json(path, data):
    """原子化写入，确保配置文件安全"""
    path_obj = Path(path).resolve()
    os.makedirs(path_obj.parent, exist_ok=True)
    fd, temp_path = tempfile.mkstemp(dir=str(path_obj.parent), prefix=f"{path_obj.name}.tmp_")
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
        os.replace(temp_path, path_obj)
    except Exception as e:
        if os.path.exists(temp_path): os.remove(temp_path)
        print(f"{RED}保存失败: {e}{NC}")

def get_policy(): return load_json("config/manager.json")
def save_policy(data): save_json("config/manager.json", data)

# --- 模块功能 ---

def manage_item_policy(category, name):
    """管理项目的策略意愿 (Enabled/Disabled 等)"""
    policy_cfg = get_policy()
    if category not in policy_cfg: policy_cfg[category] = {}
    if name not in policy_cfg[category]: policy_cfg[category][name] = {"enabled": True}
    
    item_policy = policy_cfg[category][name]
    
    while True:
        print(f"\n策略中心 [{category}] -> {BOLD}{name}{NC}")
        status = f"{GREEN}[启用]{NC}" if item_policy.get("enabled", True) else f"{RED}[禁用]{NC}"
        print(f"1. 切换状态: {status}")
        
        if category == "agents":
            auto_i = f"{GREEN}[开启]{NC}" if item_policy.get("auto_install", True) else f"{RED}[关闭]{NC}"
            auto_u = f"{GREEN}[开启]{NC}" if item_policy.get("auto_update", True) else f"{RED}[关闭]{NC}"
            print(f"2. 自动安装: {auto_i}")
            print(f"3. 自动更新: {auto_u}")
            print(f"4. 设置首选供应源: {YELLOW}{item_policy.get('preferred_provider', '未设置')}{NC}")
        elif category == "tools":
            auto_u = f"{GREEN}[开启]{NC}" if item_policy.get("auto_update", True) else f"{RED}[关闭]{NC}"
            print(f"2. 自动更新: {auto_u}")

        print("q. 返回上级")
        
        op = ask("选择操作")
        if op.lower() == 'q': break
        if op == '1': item_policy["enabled"] = not item_policy.get("enabled", True)
        elif op == '2' and category == "agents": item_policy["auto_install"] = not item_policy.get("auto_install", True)
        elif op == '2' and category == "tools": item_policy["auto_update"] = not item_policy.get("auto_update", True)
        elif op == '3' and category == "agents": item_policy["auto_update"] = not item_policy.get("auto_update", True)
        elif op == '4' and category == "agents":
            item_policy["preferred_provider"] = ask("首选源 (brew/npm/gh)", item_policy.get("preferred_provider", "npm"))
        
        save_policy(policy_cfg)
        mark_sync_required()

def menu_agents():
    def_path = "config/agents.json"
    def_cfg = load_json(def_path)
    
    while True:
        os.system('clear')
        policy_cfg = get_policy().get("agents", {})
        print(f"{HDR_BLUE}=== Agent 全生命周期管理 ==={NC}")
        item_list = sorted(def_cfg.keys())
        for i, name in enumerate(item_list, 1):
            p = policy_cfg.get(name, {})
            status = f"{GREEN}ON{NC}" if p.get("enabled", True) else f"{RED}OFF{NC}"
            print(f"{i}. [{status}] {BOLD}{name:20}{NC}")
        
        print("\nq. 返回 / [序号] 管理策略 / e. 编辑技术定义")
        choice = ask("选择操作")
        if choice.lower() == 'q': break
        if choice.lower() == 'e':
            print(f"{YELLOW}提示: 技术定义包含安装指令等细节，建议直接编辑 config/agents.json{NC}")
            input("按回车继续...")
            continue
        try:
            name = item_list[int(choice)-1]
            manage_item_policy("agents", name)
        except: pass

def menu_mcp():
    def_path = "config/mcp.json"
    def_cfg = load_json(def_path)
    
    while True:
        os.system('clear')
        policy_cfg = get_policy().get("mcp", {})
        print(f"{BLUE}=== MCP 服务管理 ==={NC}")
        items = def_cfg.get("servers", {})
        item_list = sorted(items.keys())
        for i, name in enumerate(item_list, 1):
            p = policy_cfg.get(name, {})
            status = f"{GREEN}ON{NC}" if p.get("enabled", True) else f"{RED}OFF{NC}"
            print(f"{i}. [{status}] {BOLD}{name}{NC}")
        
        print("\na. 添加定义 / [序号] 切换策略 / q. 返回")
        choice = ask("选择操作")
        if choice.lower() == 'q': break
        if choice.lower() == 'a':
            # 简化添加流程
            name = ask("服务名称")
            if name:
                items[name] = {"command": ask("命令", "node"), "args": [], "env": {}}
                save_json(def_path, def_cfg)
                mark_sync_required()
            continue
        try:
            name = item_list[int(choice)-1]
            manage_item_policy("mcp", name)
        except: pass

def main():
    global SYNC_REQUIRED
    while True:
        os.system('clear')
        print(f"{HDR_BLUE}{BOLD}╔════════════════════════════════════════════╗{NC}")
        print(f"{HDR_BLUE}{BOLD}║      Agent Manager 统一策略控制塔         ║{NC}")
        print(f"{HDR_BLUE}{BOLD}╚════════════════════════════════════════════╝{NC}")
        if SYNC_REQUIRED: print(f"\n  {PENDING_GOLD}{BOLD}⚠ 注意: 策略已变更，请执行同步 (Run Sync) 以应用到系统。{NC}")
        
        print(f"\n  {WHITE}1.{NC} 管理 {BOLD}Agents{NC} 生命周期意愿")
        print(f"  {WHITE}2.{NC} 管理 {BOLD}MCP{NC} 服务启用策略")
        print(f"  {WHITE}3.{NC} 管理 {BOLD}Tools{NC} 安装与更新策略")
        print(f"  {WHITE}4.{NC} 管理 {BOLD}Skills{NC} 启用状态")
        print(f"  {WHITE}5.{NC} {YELLOW}执行全量环境大扫除 (Cleanup){NC}")
        print(f"  {WHITE}d.{NC} {BLUE}执行深度诊断 (Doctor){NC}")
        print(f"  {WHITE}s.{NC} {SUCCESS_GREEN}立即同步并认证 (Run Sync){NC}")
        print(f"  {WHITE}q.{NC} 退出")
        
        choice = ask("\n请选择功能")
        if choice == '1': menu_agents()
        elif choice == '2': menu_mcp()
        elif choice == '3':
            item = ask("工具名称")
            if item: manage_item_policy("tools", item)
        elif choice == '4':
            item = ask("技能名称")
            if item: manage_item_policy("skills", item)
        elif choice == '5':
            if ask_yes_no("确认执行环境大扫除?", "n"): os.system("./manage.sh cleanup")
            input("\n回车返回...")
        elif choice == 'd': os.system("./manage.sh doctor"); input("\n回车返回...")
        elif choice == 's': 
            os.system("./manage.sh all")
            input("\n回车返回...")
            SYNC_REQUIRED = False
        elif choice.lower() == 'q': break

if __name__ == "__main__":
    try: main()
    except KeyboardInterrupt: print("\n已退出。")
