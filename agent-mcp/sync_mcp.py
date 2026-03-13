#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import json
import re
import sys
import datetime
import shutil
import tempfile
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

def atomic_write_json(path, data):
    """原子化写入 JSON"""
    path = Path(path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_path = tempfile.mkstemp(dir=str(path.parent), prefix=f"{path.name}.tmp_")
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
        os.replace(temp_path, path)
        os.chmod(path, 0o600)
    except Exception as e:
        if os.path.exists(temp_path): os.remove(temp_path)
        raise e

def run_cmd(cmd, cwd=None):
    """运行 shell 命令并返回结果"""
    try:
        result = subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True, shell=isinstance(cmd, str))
        return True, result.stdout
    except subprocess.CalledProcessError as e:
        return False, f"{e.stderr}\n{e.stdout}"

def clone_or_update_repo(name, url, base_dir, dry_run=False):
    """下载或更新 Git 仓库"""
    target_dir = Path(base_dir) / name
    if target_dir.exists():
        if (target_dir / ".git").exists():
            if dry_run: logging.info(f"[{name}] (预览: 计划检查更新)")
            else:
                logging.info(f"正在检查 {name} 更新...")
                success, output = run_cmd("git pull", cwd=target_dir)
                if success: logging.info(f"[{name}] 更新成功。")
                else: logging.error(f"[{name}] 更新失败: {output}")
        else:
            logging.warning(f"[{name}] 目录存在但不是 git 仓库，跳过更新。")
    else:
        logging.info(f"[{name}] 仓库不存在，准备下载 (git clone)...")
        if not dry_run:
            success, output = run_cmd(f"git clone {url} \"{target_dir}\"")
            if success: logging.info(f"[{name}] 下载成功。")
            else: logging.error(f"[{name}] 下载失败: {output}")
    return target_dir

def install_dependencies_and_build(repo_path, config, dry_run=False):
    """根据仓库类型自动安装依赖和构建"""
    if dry_run: return True
    success_all = True
    setup_cmd = config.get("setup")
    if setup_cmd:
        logging.info(f"[{repo_path.name}] 执行自定义初始化...")
        ok, out = run_cmd(setup_cmd, cwd=repo_path)
        if not ok: logging.error(f"[{repo_path.name}] 失败: {out}"); success_all = False

    if (repo_path / "package.json").exists():
        logging.info(f"[{repo_path.name}] 处理 Node.js 依赖...")
        if (repo_path / "yarn.lock").exists(): cmd = "yarn install"
        elif (repo_path / "pnpm-lock.yaml").exists(): cmd = "pnpm install"
        else: cmd = "npm install"
        ok, out = run_cmd(cmd, cwd=repo_path)
        if ok:
            with open(repo_path / "package.json", 'r', encoding='utf-8') as f:
                try:
                    pkg_data = json.load(f)
                    if "scripts" in pkg_data and "build" in pkg_data["scripts"]:
                        run_cmd("npm run build", cwd=repo_path)
                except: pass
        else: logging.error(f"[{repo_path.name}] 失败: {out}"); success_all = False
    return success_all

def update_toml_config(agent_name, config_path, active_servers, all_managed_servers, dry_run):
    """原子化更新 TOML 配置"""
    path = Path(config_path).expanduser()
    content = ""
    if path.exists():
        with open(path, 'r', encoding='utf-8') as f: content = f.read()

    parts = re.split(r'(\n\s*\[)', content)
    chunks = [parts[0]]
    for i in range(1, len(parts), 2): chunks.append(parts[i] + parts[i+1])
    
    new_chunks = []
    managed_names = set(all_managed_servers)
    for chunk in chunks:
        header_match = re.search(r'\[\s*mcp_servers\.([^.\]\s]+)', chunk)
        if header_match:
            srv_name = header_match.group(1)
            if srv_name in managed_names or '_MANAGED_BY = "agent-manager"' in chunk: continue
        new_chunks.append(chunk)
    
    base_content = "".join(new_chunks).strip()
    new_blocks = []
    for name, settings in active_servers.items():
        block = f'\n\n[mcp_servers.{name}]\ncommand = "{settings["command"]}"\n'
        block += f'args = [{", ".join([f"\"{a}\"" for a in settings["args"]])}]\n'
        env = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
        block += f'[mcp_servers.{name}.env]\n'
        for k, v in env.items(): block += f'{k} = "{v}"\n'
        new_blocks.append(block)
    
    full_content = base_content + "".join(new_blocks)
    if not dry_run:
        path.parent.mkdir(parents=True, exist_ok=True)
        fd, temp_path = tempfile.mkstemp(dir=str(path.parent), prefix=f"{path.name}.tmp_")
        try:
            with os.fdopen(fd, 'w', encoding='utf-8') as f: f.write(full_content.strip() + "\n")
            os.replace(temp_path, path)
            os.chmod(path, 0o600)
            logging.info(f"INFO: [AGENT: {agent_name}] TOML 配置已原子更新。")
        except Exception as e:
            if os.path.exists(temp_path): os.remove(temp_path)
            logging.error(f"TOML 写入失败: {e}")

def backup_config(path, dry_run=False):
    """5 代轮转备份"""
    path = Path(path).expanduser()
    if not path.exists(): return
    timestamp = datetime.datetime.now().strftime("%Y%m%d%H%M%S")
    bak_path = path.with_suffix(f"{path.suffix}.bak.{timestamp}")
    if not dry_run:
        shutil.copy2(path, bak_path)
        backups = sorted(list(path.parent.glob(f"{path.name}.bak.*")), key=os.path.getmtime, reverse=True)
        for old_bak in backups[5:]:
            try: os.remove(old_bak)
            except: pass

def manage_mcp_config(agents_config, mcp_servers, dry_run=False):
    """更新各个 Agent 的 MCP 配置文件"""
    for agent_name, agent_meta in agents_config.items():
        config_path_str = agent_meta if isinstance(agent_meta, str) else agent_meta.get("path")
        file_format = "json" if isinstance(agent_meta, str) else agent_meta.get("format", "json")
        config_path = Path(config_path_str).expanduser()
        
        if not config_path.parent.exists():
            logging.info(f"INFO: [AGENT: {agent_name}] Agent 目录缺失，跳过。")
            continue

        if not dry_run: backup_config(config_path, dry_run)

        active_mcp_settings = {}
        for name, srv_config in mcp_servers.items():
            if not srv_config.get("enabled", True): continue
            if srv_config.get("agents") and agent_name not in srv_config["agents"]: continue
            
            repo_path_str = srv_config.get("_repo_path", "")
            repo_path = Path(repo_path_str).resolve() if repo_path_str else ""
            parsed_args = [arg.replace("{repo_path}", str(repo_path)) for arg in srv_config.get("args", [])]
            
            active_mcp_settings[name] = {
                "command": srv_config.get("command"),
                "args": parsed_args,
                "env": srv_config.get("env", {})
            }

        if file_format == "toml":
            update_toml_config(agent_name, config_path, active_mcp_settings, mcp_servers.keys(), dry_run)
            continue

        if dry_run:
            logging.info(f"INFO: [AGENT: {agent_name}] (预览: 计划原子更新 JSON)")
        else:
            current_config = {"mcpServers": {}}
            if config_path.exists():
                try:
                    with open(config_path, 'r', encoding='utf-8') as f: current_config = json.load(f)
                except: pass
            
            key_name = "mcp" if agent_name == "OpenCode" and "mcp" in current_config else "mcpServers"
            if key_name not in current_config: current_config[key_name] = {}
            
            # Orphan Removal
            for k in list(current_config[key_name].keys()):
                srv = current_config[key_name][k]
                if isinstance(srv, dict) and srv.get('env', {}).get('_MANAGED_BY') == 'agent-manager':
                    if k not in active_mcp_settings:
                        logging.info(f"INFO: [AGENT: {agent_name}] 移除孤儿 MCP: {k}")
                        del current_config[key_name][k]
            
            # Inject
            for name, settings in active_mcp_settings.items():
                if "env" not in settings: settings["env"] = {}
                settings["env"]["_MANAGED_BY"] = "agent-manager"
                current_config[key_name][name] = settings
                logging.info(f"INFO: [AGENT: {agent_name}] 更新 MCP: {name}")
            
            atomic_write_json(config_path, current_config)

def main():
    parser = argparse.ArgumentParser(description="MCP Server 管理工具 (策略对齐)")
    parser.add_argument("--config", default="../config/manager.json", help="策略路径")
    parser.add_argument("--def-file", default="../config/mcp.json", help="定义路径")
    parser.add_argument("--base-dir", default=".", help="基础目录")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    args = parser.parse_args()

    # 对齐职责：从 manager.json 读取策略，从 mcp.json 读取定义与路径
    policy_cfg = load_json(args.config).get("mcp", {})
    full_def = load_json(args.def_file)
    definition_cfg = full_def.get("servers", {})
    AGENT_TARGETS = full_def.get("agent_paths", {})

    # 递归合并策略开关
    for name, s_def in definition_cfg.items():
        if name in policy_cfg:
            s_def.update(policy_cfg[name])

    base_dir = Path(args.base_dir).resolve()
    if not base_dir.exists() and not args.dry_run: base_dir.mkdir(parents=True, exist_ok=True)

    all_build_success = True
    for srv_name, srv_config in definition_cfg.items():
        url = srv_config.get("url")
        if not url:
            srv_config["_repo_path"] = ""
        else:
            target_dir = Path(base_dir) / srv_name
            srv_config["_repo_path"] = str(target_dir)
            if not srv_config.get("enabled", True): continue
            repo_path = clone_or_update_repo(srv_name, url, base_dir, dry_run=args.dry_run)
            if not args.dry_run and repo_path.exists():
                if not install_dependencies_and_build(repo_path, srv_config, dry_run=args.dry_run):
                    all_build_success = False
    
    manage_mcp_config(AGENT_TARGETS, definition_cfg, dry_run=args.dry_run)
    if not all_build_success: sys.exit(1)
    logging.info("任务完成！")

if __name__ == "__main__":
    main()
