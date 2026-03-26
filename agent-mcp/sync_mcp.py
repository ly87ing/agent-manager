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
import shlex

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_list, canonicalize_agent_mapping
from src.command_runner import run_command
from src.json_config import get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import is_agent_installed
from src.runtime_logging import configure_logging
from src.toml_compat import tomllib

configure_logging()

PLACEHOLDER_RE = re.compile(r"\$\{([^}]+)\}")
TOML_BARE_KEY_RE = re.compile(r"^[A-Za-z0-9_-]+$")
TOML_SERVER_HEADER_RE = re.compile(
    r'^\s*\[\s*mcp_servers\.(?P<key>"(?:\\.|[^"\\])*"|[A-Za-z0-9_-]+)',
    re.MULTILINE,
)

def atomic_write_json(path, data):
    """原子化写入 JSON"""
    path = Path(path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_path = tempfile.mkstemp(dir=str(path.parent), prefix=f"{path.name}.tmp_")
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
        os.chmod(temp_path, 0o600)
        os.replace(temp_path, path)
    except Exception as e:
        if os.path.exists(temp_path): os.remove(temp_path)
        raise e

def run_cmd(cmd, cwd=None, timeout=300):
    """运行 shell 命令并返回结果"""
    try:
        result = run_command(cmd, cwd=cwd, check=True, capture_output=True, text=True, timeout=timeout)
        return True, result.stdout
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT after {timeout} seconds: {cmd}"
    except subprocess.CalledProcessError as e:
        return False, f"Command '{cmd}' failed with exit code {e.returncode}: {e.stderr}\n{e.stdout}"
    except FileNotFoundError as e:
        return False, f"Directory or executable not found: {e}"
    except Exception as e:
        return False, f"Unexpected error executing '{cmd}': {e}"

def clone_or_update_repo(name, url, base_dir, dry_run=False, offline=False):
    """下载或更新 Git 仓库"""
    target_dir = Path(base_dir) / name
    if target_dir.exists():
        if (target_dir / ".git").exists():
            if dry_run: logging.info(f"[{name}] (预览: 计划检查更新)")
            elif offline:
                logging.info(f"[{name}] 离线模式，跳过仓库更新。")
            else:
                logging.info(f"正在检查 {name} 更新...")
                success, output = run_cmd(["git", "pull", "--ff-only"], cwd=target_dir)
                if success: logging.info(f"[{name}] 更新成功。")
                else: logging.error(f"[{name}] 更新失败: {output}")
                return target_dir, success
            return target_dir, True
        else:
            logging.error(f"[{name}] 目录存在但不是 git 仓库，无法继续管理。")
            return target_dir, False
    logging.info(f"[{name}] 仓库不存在，准备下载 (git clone)...")
    if dry_run:
        logging.info(f"[{name}] (预览: 离线/在线均仅展示计划，不实际下载)")
        return target_dir, True
    if offline:
        logging.error(f"[{name}] 离线模式且本地仓库缺失，无法下载。")
        return target_dir, False
    success, output = run_cmd(["git", "clone", url, str(target_dir)])
    if success: logging.info(f"[{name}] 下载成功。")
    else: logging.error(f"[{name}] 下载失败: {output}")
    return target_dir, success

def install_dependencies_and_build(repo_path, config, dry_run=False, offline=False):
    """根据仓库类型自动安装依赖和构建"""
    if dry_run: return True
    if offline:
        logging.info(f"[{repo_path.name}] 离线模式，跳过依赖安装与构建。")
        return True
    success_all = True
    setup_cmd = config.get("setup")
    if setup_cmd:
        logging.info(f"[{repo_path.name}] 执行自定义初始化...")
        ok, out = run_cmd(setup_cmd, cwd=repo_path)
        if not ok: logging.error(f"[{repo_path.name}] 失败: {out}"); success_all = False

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
        # Convert command string to list for safer execution
        if isinstance(cmd, str):
            cmd_list = shlex.split(cmd)
            ok, out = run_cmd(cmd_list, cwd=repo_path)
        else:
            ok, out = run_cmd(cmd, cwd=repo_path)
        if ok:
            with open(repo_path / "package.json", 'r', encoding='utf-8') as f:
                try:
                    pkg_data = json.load(f)
                    if "scripts" in pkg_data and "build" in pkg_data["scripts"]:
                        build_ok, build_out = run_cmd([pkg_mgr, "run", "build"], cwd=repo_path)
                        if not build_ok:
                            logging.error(f"[{repo_path.name}] build 失败: {build_out}")
                            success_all = False
                except Exception:
                    pass
        else: logging.error(f"[{repo_path.name}] 失败: {out}"); success_all = False
    return success_all

def config_requires_repo_path(config):
    values = [config.get("command", "")] + config.get("args", [])
    return any(isinstance(value, str) and "{repo_path}" in value for value in values)

def find_unresolved_placeholders(value, path=""):
    unresolved = []
    if isinstance(value, dict):
        for key, inner in value.items():
            next_path = f"{path}.{key}" if path else str(key)
            unresolved.extend(find_unresolved_placeholders(inner, next_path))
    elif isinstance(value, list):
        for idx, inner in enumerate(value):
            next_path = f"{path}[{idx}]"
            unresolved.extend(find_unresolved_placeholders(inner, next_path))
    elif isinstance(value, str):
        for match in PLACEHOLDER_RE.findall(value):
            unresolved.append((path or "<root>", match))
    return unresolved

def load_existing_json_config(agent_name, config_path):
    is_opencode = agent_name == "opencode"
    mcp_key = "mcp" if is_opencode else "mcpServers"
    current_config = {mcp_key: {}}
    if not config_path.exists():
        return current_config
    try:
        with open(config_path, 'r', encoding='utf-8') as f:
            parsed = json.load(f)
            if not isinstance(parsed, dict):
                raise ValueError("顶层必须是对象")
            return parsed
    except Exception as exc:
        logging.error(f"[AGENT: {agent_name}] 现有 JSON 配置解析失败，已停止覆盖: {exc}")
        return None

def toml_quote(value):
    """返回合法的 TOML basic string。"""
    return json.dumps(value, ensure_ascii=False)


def toml_key(value):
    """返回 TOML bare key 或 quoted key。"""
    if TOML_BARE_KEY_RE.fullmatch(value):
        return value
    return json.dumps(value, ensure_ascii=False)


def extract_toml_mcp_server_name(chunk):
    header_match = TOML_SERVER_HEADER_RE.search(chunk)
    if not header_match:
        return None
    key_token = header_match.group("key")
    if key_token.startswith('"'):
        return json.loads(key_token)
    return key_token


def merge_toml_chunks_by_server(chunks):
    if not chunks:
        return chunks
    merged_chunks = [chunks[0]]
    previous_server = extract_toml_mcp_server_name(chunks[0])
    for chunk in chunks[1:]:
        current_server = extract_toml_mcp_server_name(chunk)
        if current_server and current_server == previous_server:
            merged_chunks[-1] += chunk
            continue
        merged_chunks.append(chunk)
        previous_server = current_server
    return merged_chunks


def update_toml_config(agent_name, config_path, active_servers, all_managed_servers, preserved_servers, dry_run):
    """原子化更新 TOML 配置"""
    path = Path(config_path).expanduser()
    content = ""
    if path.exists():
        with open(path, 'r', encoding='utf-8') as f:
            content = f.read()
        try:
            tomllib.loads(content)
        except tomllib.TOMLDecodeError as exc:
            logging.error(f"[AGENT: {agent_name}] 现有 TOML 配置解析失败，已停止覆盖: {exc}")
            return False

    parts = re.split(r'(\n\s*\[)', content)
    chunks = [parts[0]]
    for i in range(1, len(parts), 2): chunks.append(parts[i] + parts[i+1])
    chunks = merge_toml_chunks_by_server(chunks)
    
    new_chunks = []
    preserved_names = set(preserved_servers)
    blocked_active_names = set()
    for chunk in chunks:
        srv_name = extract_toml_mcp_server_name(chunk)
        if srv_name:
            if srv_name in preserved_names:
                new_chunks.append(chunk)
                continue
            is_managed_chunk = '_MANAGED_BY = "agent-manager"' in chunk
            if srv_name in active_servers and not is_managed_chunk:
                if dry_run:
                    logging.info(f"[AGENT: {agent_name}] 发现同名用户自管 TOML 段，真实执行时将拒绝接管: {srv_name} (预览)")
                    blocked_active_names.add(srv_name)
                else:
                    logging.error(f"[AGENT: {agent_name}] 发现同名用户自管 TOML 段，拒绝接管: {srv_name}")
                    return False
            if is_managed_chunk:
                continue
        new_chunks.append(chunk)
    
    base_content = "".join(new_chunks).strip()
    new_blocks = []
    for name, settings in active_servers.items():
        if name in blocked_active_names:
            continue
        quoted_name = toml_key(name)
        block = f"\n\n[mcp_servers.{quoted_name}]\ncommand = {toml_quote(settings['command'])}\n"
        block += f"args = {json.dumps(settings['args'], ensure_ascii=False)}\n"
        env = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
        block += f"[mcp_servers.{quoted_name}.env]\n"
        for k, v in env.items():
            block += f"{toml_key(k)} = {toml_quote(v)}\n"
        new_blocks.append(block)
    
    full_content = base_content + "".join(new_blocks)
    try:
        tomllib.loads(full_content.strip() + "\n")
    except tomllib.TOMLDecodeError as exc:
        logging.error(f"[AGENT: {agent_name}] 生成的 TOML 非法，已停止写入: {exc}")
        return False
    if not dry_run:
        path.parent.mkdir(parents=True, exist_ok=True)
        fd, temp_path = tempfile.mkstemp(dir=str(path.parent), prefix=f"{path.name}.tmp_")
        try:
            with os.fdopen(fd, 'w', encoding='utf-8') as f: f.write(full_content.strip() + "\n")
            os.chmod(temp_path, 0o600)
            os.replace(temp_path, path)
            logging.info(f"[AGENT: {agent_name}] TOML 配置已原子更新。")
        except Exception as e:
            if os.path.exists(temp_path): os.remove(temp_path)
            logging.error(f"TOML 写入失败: {e}")
            return False
    return True

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
            except OSError: pass

def manage_mcp_config(agents_config, mcp_servers, dry_run=False, agent_defs=None):
    """更新各个 Agent 的 MCP 配置文件"""
    all_ok = True
    declared_managed_servers = {
        name for name, srv_config in mcp_servers.items() if srv_config.get("_declared_in_policy", True)
    }
    for agent_name, agent_meta in agents_config.items():
        agent_installed = True
        if agent_defs:
            defn = agent_defs.get(agent_name, {})
            agent_installed = is_agent_installed(agent_name, defn)
        config_path_str = agent_meta if isinstance(agent_meta, str) else agent_meta.get("path")
        file_format = "json" if isinstance(agent_meta, str) else agent_meta.get("format", "json")
        config_path = Path(config_path_str).expanduser()

        active_mcp_settings = {}
        preserved_failed_servers = set()
        for name, srv_config in mcp_servers.items():
            if not srv_config.get("_declared_in_policy", True):
                continue
            if not srv_config.get("enabled", True): continue
            if srv_config.get("agents") and agent_name not in srv_config["agents"]: continue
            if not srv_config.get("_sync_ready", True):
                preserved_failed_servers.add(name)
                continue
            
            repo_path_str = srv_config.get("_repo_path", "")
            repo_path = Path(repo_path_str).resolve() if repo_path_str else ""
            parsed_args = [arg.replace("{repo_path}", str(repo_path)) for arg in srv_config.get("args", [])]
            
            active_mcp_settings[name] = {
                "command": srv_config.get("command"),
                "args": parsed_args,
                "env": srv_config.get("env", {})
            }

        if not agent_installed:
            if not config_path.exists():
                logging.info(f"[AGENT: {agent_name}] 未安装且无配置文件，跳过 MCP 注入。")
                continue
            logging.info(f"[AGENT: {agent_name}] 未安装，仅清理已托管配置。")
            active_mcp_settings = {}

        if not active_mcp_settings and not config_path.exists():
            continue

        if not config_path.parent.exists():
            logging.info(f"[AGENT: {agent_name}] Agent 目录缺失，自动创建以确保后续配置注入。")
            try:
                config_path.parent.mkdir(parents=True, exist_ok=True)
            except Exception as e:
                logging.error(f"无法创建配置目录 {config_path.parent}: {e}")
                all_ok = False
                continue

        if not dry_run: backup_config(config_path, dry_run)

        if file_format == "toml":
            if not update_toml_config(
                agent_name,
                config_path,
                active_mcp_settings,
                declared_managed_servers,
                preserved_failed_servers,
                dry_run,
            ):
                all_ok = False
            continue

        if dry_run:
            logging.info(f"[AGENT: {agent_name}] (预览: 计划原子更新 JSON)")
        else:
            current_config = load_existing_json_config(agent_name, config_path)
            if current_config is None:
                all_ok = False
                continue
            
            # OpenCode 使用 "mcp" 键，其他 Agent 使用 "mcpServers"
            is_opencode = agent_name == "opencode"
            key_name = "mcp" if is_opencode else "mcpServers"
            if key_name not in current_config: current_config[key_name] = {}
            
            # Orphan Removal
            for k in list(current_config[key_name].keys()):
                srv = current_config[key_name][k]
                if isinstance(srv, dict):
                    # 检查 env 或 environment 中的 _MANAGED_BY
                    env_dict = (srv.get('env') or srv.get('environment') or {})
                    if env_dict.get('_MANAGED_BY') == 'agent-manager':
                        if k in preserved_failed_servers:
                            continue
                        if k not in active_mcp_settings:
                            logging.info(f"[AGENT: {agent_name}] 移除孤儿 MCP: {k}")
                            del current_config[key_name][k]
            
            # Inject
            for name, settings in active_mcp_settings.items():
                if is_opencode:
                    # OpenCode 特殊格式: command 是数组，environment 替代 env
                    cmd_array = [settings["command"]] + settings.get("args", [])
                    env_dict = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
                    current_config[key_name][name] = {
                        "type": "local",
                        "command": cmd_array,
                        "environment": env_dict
                    }
                else:
                    # 标准格式
                    if "env" not in settings: settings["env"] = {}
                    settings["env"]["_MANAGED_BY"] = "agent-manager"
                    current_config[key_name][name] = settings
                logging.info(f"[AGENT: {agent_name}] 更新 MCP: {name}")
            
            atomic_write_json(config_path, current_config)
    return all_ok

def sync_secrets_repo(secrets_path):
    """Phase 0: 如果 secrets 目录是 Git 仓库，执行自动更新"""
    if not secrets_path.exists(): return
    
    # 检查 secrets 目录本身或其子目录
    targets = [secrets_path] + [d for d in secrets_path.iterdir() if d.is_dir()]
    for target in targets:
        if (target / ".git").exists():
            logging.info(f"发现密钥仓库: {target.name}，正在同步最新凭证...")
            success, output = run_cmd(["git", "pull", "--ff-only"], cwd=target)
            if success:
                logging.info(f"密钥仓库 {target.name} 同步成功。")
            else:
                logging.warning(f"密钥仓库 {target.name} 同步失败 (可能是网络或权限问题): {output}")

def main():
    parser = argparse.ArgumentParser(description="MCP Server 管理工具 (策略对齐)")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略路径")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "mcp.json"), help="定义路径")
    parser.add_argument("--base-dir", default=str(Path(__file__).resolve().parent), help="基础目录")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过远程仓库与 secrets 更新)")
    parser.add_argument("--prepare-runtime", action="store_true", help="显式准备 repo/runtime（clone、pull、setup、build）")
    args = parser.parse_args()
    
    try:
        policy_root = load_json_object_file(args.config, strict=True, missing_ok=False)
        full_def = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        policy_cfg = get_typed_mapping_section(policy_root, "mcp", Path(args.config).name, (dict,), "对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)

    # 对齐职责：从 manager.json 读取策略，从 mcp.json 读取定义与路径
    try:
        definition_cfg = get_typed_mapping_section(full_def, "servers", Path(args.def_file).name, (dict,), "对象")
        agent_paths = get_typed_mapping_section(full_def, "agent_paths", Path(args.def_file).name, (str, dict), "字符串或对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)
    AGENT_TARGETS = canonicalize_agent_mapping(agent_paths)

    # 递归合并策略开关
    for name, s_def in definition_cfg.items():
        s_def["_declared_in_policy"] = name in policy_cfg
        if name in policy_cfg:
            s_def.update(policy_cfg[name])
        s_def["agents"] = canonicalize_agent_list(s_def.get("agents"))

    # [凭证保护] Phase 0: 扫描并同步密钥池 (secrets/ 目录)
    env_vars = {}
    root_dir = Path(args.def_file).resolve().parent.parent
    secrets_dir = root_dir / "secrets"
    
    if args.offline:
        logging.info("离线模式，跳过 secrets 仓库同步。")
    elif args.prepare_runtime and not args.dry_run:
        sync_secrets_repo(secrets_dir)
    
    # 加载全局凭证并进行占位符替换
    search_paths = [root_dir / ".env", secrets_dir]
    
    for p in search_paths:
        if not p.exists(): continue
        files = [p] if p.is_file() else sorted(p.glob("*.env"))
        for f in files:
            logging.info(f"正在从凭证源加载: {f.name}")
            with open(f, 'r', encoding='utf-8') as f_in:
                for line in f_in:
                    line = line.strip()
                    if not line or line.startswith('#'): continue
                    if '=' in line:
                        k, v = line.split('=', 1)
                        env_vars[k.strip()] = v.strip().strip('"').strip("'")
    
    # 递归遍历 definition_cfg，进行注入
    for srv_name, srv_def in definition_cfg.items():
        if "env" in srv_def:
            for k, v in srv_def["env"].items():
                if isinstance(v, str) and v.startswith("${") and v.endswith("}"):
                    var_name = v[2:-1]
                    if var_name in env_vars:
                        srv_def["env"][k] = env_vars[var_name]
                        logging.info(f"[{srv_name}] 注入凭证: {k} (来源: secrets 池)")
                    elif var_name in os.environ:
                        srv_def["env"][k] = os.environ[var_name]
                        logging.info(f"[{srv_name}] 从系统环境注入: {k}")

    unresolved = []
    for srv_name, srv_def in definition_cfg.items():
        if not srv_def.get("_declared_in_policy"):
            continue
        if not srv_def.get("enabled", True):
            continue
        unresolved.extend(
            [(f"servers.{srv_name}.{path}", var) for path, var in find_unresolved_placeholders(srv_def)]
        )
    if unresolved:
        for path, var_name in unresolved:
            if args.dry_run:
                logging.info(f"未解析的占位符: {path} -> ${{{var_name}}} (预览)")
            else:
                logging.error(f"未解析的占位符: {path} -> ${{{var_name}}}")
        if not args.dry_run:
            sys.exit(1)

    base_dir = Path(args.base_dir).resolve()
    if not base_dir.exists() and not args.dry_run: base_dir.mkdir(parents=True, exist_ok=True)

    all_build_success = True
    for srv_name, srv_config in definition_cfg.items():
        srv_config["_sync_ready"] = srv_config.get("_declared_in_policy", False) and srv_config.get("enabled", True)
        url = srv_config.get("url")
        if not url:
            srv_config["_repo_path"] = ""
        else:
            target_dir = Path(base_dir) / srv_name
            srv_config["_repo_path"] = str(target_dir)
            if not srv_config.get("_declared_in_policy"):
                logging.info(f"[{srv_name}] 未声明策略，跳过。")
                continue
            if not srv_config.get("enabled", True): continue
            repo_required = config_requires_repo_path(srv_config)
            local_repo_ready = target_dir.exists() and (target_dir / ".git").exists()
            if args.prepare_runtime:
                repo_path, repo_ok = clone_or_update_repo(
                    srv_name,
                    url,
                    base_dir,
                    dry_run=args.dry_run,
                    offline=args.offline,
                )
                if not repo_ok:
                    all_build_success = False
                    srv_config["_sync_ready"] = False
                    if not (repo_path.exists() and (repo_path / ".git").exists()):
                        continue
                if not args.dry_run and repo_path.exists():
                    if not install_dependencies_and_build(
                        repo_path,
                        srv_config,
                        dry_run=args.dry_run,
                        offline=args.offline,
                    ):
                        all_build_success = False
                        srv_config["_sync_ready"] = False
            else:
                if repo_required and not local_repo_ready:
                    message = f"[{srv_name}] 配置依赖本地 repo_path，但当前未发现本地仓库；请显式运行 --prepare-runtime。"
                    if args.dry_run:
                        logging.info(f"{message} (预览)")
                    else:
                        logging.error(message)
                        all_build_success = False
                        srv_config["_sync_ready"] = False

    agent_defs = load_json_object_file(REPO_ROOT / "config" / "agents.json", strict=False, missing_ok=True) or {}
    if not manage_mcp_config(AGENT_TARGETS, definition_cfg, dry_run=args.dry_run, agent_defs=agent_defs):
        all_build_success = False
    if not all_build_success: sys.exit(1)
    logging.info("任务完成！")

if __name__ == "__main__":
    main()
