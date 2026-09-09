#!/usr/bin/env python3
from __future__ import annotations

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
import copy
from pathlib import Path
import shlex

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_id, canonicalize_agent_list, canonicalize_agent_mapping
from src.command_runner import run_command
from src.git_remote_probe import probe_git_remote_state
from src.json_config import get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import is_agent_installed
from src.mcp_config import resolve_mcp_server_args
from src.runtime_logging import configure_logging
from src.toml_compat import tomllib
from src.mcp_bindings import (
    MANAGED_BY_KEY,
    MANAGED_BY_VALUE,
    MCP_BINDINGS_POLICY_KEY,
    UNSET,
    McpBindingConflictError,
    McpBindingError,
    McpConflictError,
    McpScopeError,
    McpSharedTargetError,
    atomic_rollback_file,
    check_shared_agent_target,
    clean_definition,
    compute_applied_digest,
    compute_definition_version,
    extract_server_from_config,
    get_binding,
    load_mcp_bindings,
    remove_binding,
    safe_atomic_write_and_verify,
    save_mcp_bindings,
    set_binding,
)
from src.storage.db import connect_ready_db, get_db_path, is_sqlite_active

configure_logging()

PLACEHOLDER_RE = re.compile(r"^\$\{([^}]+)\}$")
TOML_BARE_KEY_RE = re.compile(r"^[A-Za-z0-9_-]+$")
TOML_SERVER_HEADER_RE = re.compile(
    r'^\s*\[\s*mcp_servers\.(?P<key>"(?:\\.|[^"\\])*"|[A-Za-z0-9_-]+)',
    re.MULTILINE,
)
TOML_SERVER_INT_KEYS = ("startup_timeout_sec",)
def atomic_write_json(path, data, expected_old_bytes=UNSET):
    """Atomically write JSON with concurrency check via safe_atomic_write_and_verify"""
    path = Path(path).expanduser()
    content = json.dumps(data, indent=2, ensure_ascii=False) + "\n"

    def _verify_json_parse(p):
        try:
            with open(p, "r", encoding="utf-8") as f:
                doc = json.load(f)
            return isinstance(doc, dict)
        except Exception:
            return False

    safe_atomic_write_and_verify(
        path,
        content_writer=lambda: content,
        verify_cb=_verify_json_parse,
        expected_old_bytes=expected_old_bytes,
    )

def run_cmd(cmd, cwd=None, timeout=300):
    """Run a shell command and return the result"""
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


def configure_serena_project_folder(config, dry_run=False, offline=False):
    """Keep Serena's per-project data outside project worktrees."""
    location = config.get("project_serena_folder_location")
    if not location:
        return True

    command = config.get("command")
    args = config.get("args") or []
    if not command or len(args) < 2 or args[0] != "--from":
        logging.error("[serena] 无法从受管命令解析固定版本，未更新项目数据目录。")
        return False

    if dry_run:
        logging.info(f"[serena] (预览: 将项目数据目录设为 {location})")
        return True

    sync_script = (
        "import sys; "
        "from serena.config.serena_config import SerenaConfig; "
        "config = SerenaConfig.from_config_file(); "
        "desired = sys.argv[1]; "
        "changed = config.project_serena_folder_location != desired; "
        "config.project_serena_folder_location = desired; "
        "config._save() if changed else None"
    )
    sync_command = [command]
    if offline:
        sync_command.append("--offline")
    sync_command.extend(["--from", args[1], "python", "-c", sync_script, location])

    ok, output = run_cmd(sync_command, timeout=120)
    if not ok:
        logging.error(f"[serena] 项目数据目录配置失败: {output}")
        return False

    logging.info(f"[serena] 项目数据目录已对齐: {location}")
    return True


def repo_head_is_detached(repo_path):
    success, _ = run_cmd(["git", "symbolic-ref", "-q", "HEAD"], cwd=repo_path)
    return not success

def clone_or_update_repo(name, url, base_dir, dry_run=False, offline=False, force_refresh=False):
    """Clone or update a Git repository"""
    target_dir = Path(base_dir) / name
    if target_dir.exists():
        if (target_dir / ".git").exists():
            if dry_run: logging.info(f"[{name}] (预览: 计划检查更新)")
            elif offline:
                logging.info(f"[{name}] 离线模式，跳过仓库更新。")
            else:
                probe_state = probe_git_remote_state(run_cmd, target_dir)
                logging.info(f"正在检查 {name} 更新...")
                update_cmd = ["git", "pull", "--ff-only"]
                if repo_head_is_detached(target_dir):
                    if (
                        probe_state["status"] == "latest"
                        and probe_state.get("local_remote_head") == probe_state.get("remote_head")
                    ):
                        logging.info(f"[{name}] 内容已最新。")
                        return target_dir, True
                    update_cmd = ["git", "fetch", "--tags", "origin"]
                elif probe_state["status"] == "latest":
                    logging.info(f"[{name}] 内容已最新。")
                    return target_dir, True
                elif probe_state["status"] == "unknown" and not force_refresh:
                    logging.warning(f"[{name}] 更新探测失败，默认跳过远端刷新；如需强制刷新请使用 --force-refresh。")
                    return target_dir, True
                elif probe_state["status"] == "unknown":
                    logging.warning(f"[{name}] 更新探测失败，但已显式开启强制刷新，继续刷新远端状态。")
                success, output = run_cmd(update_cmd, cwd=target_dir)
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
    """Automatically install dependencies and build based on the repository type"""
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

def placeholder_name(value):
    if not isinstance(value, str):
        return None
    match = PLACEHOLDER_RE.fullmatch(value)
    return match.group(1) if match else None

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
        match = placeholder_name(value)
        if match is not None:
            unresolved.append((path or "<root>", match))
    return unresolved

def parse_json_config_bytes(agent_name: str, raw_bytes: bytes | None, config_path_label: str = "") -> dict:
    is_opencode = (agent_name == "opencode")
    mcp_key = "mcp" if is_opencode else "mcpServers"
    if raw_bytes is None or len(raw_bytes) == 0:
        return {mcp_key: {}}
    try:
        parsed = json.loads(raw_bytes.decode("utf-8"))
    except Exception as exc:
        raise McpConflictError(f"配置文件 JSON 解析失败，拒绝覆盖: {config_path_label} ({exc})") from exc
    if not isinstance(parsed, dict):
        raise McpConflictError(f"配置文件根节点不是对象，拒绝覆盖: {config_path_label}")
    if mcp_key in parsed:
        if not isinstance(parsed[mcp_key], dict):
            raise McpConflictError(f"配置文件中的 '{mcp_key}' 不是对象，拒绝覆盖: {config_path_label}")
    else:
        parsed[mcp_key] = {}
    return parsed


def load_existing_json_config(agent_name, config_path):
    path = Path(config_path).expanduser()
    if not path.exists():
        is_opencode = agent_name == "opencode"
        mcp_key = "mcp" if is_opencode else "mcpServers"
        return {mcp_key: {}}
    try:
        raw_bytes = path.read_bytes()
        return parse_json_config_bytes(agent_name, raw_bytes, str(config_path))
    except Exception as exc:
        logging.error(f"[AGENT: {agent_name}] 现有 JSON 配置解析失败，已停止覆盖: {exc}")
        return None

def toml_quote(value):
    """Return a valid TOML basic string."""
    return json.dumps(value, ensure_ascii=False)


def toml_key(value):
    """Return a TOML bare key or quoted key."""
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


def render_toml_content(agent_name, config_path, active_servers, all_managed_servers, preserved_servers, dry_run=False, only_servers=None, old_bytes=UNSET, no_binding_servers=None):
    path = Path(config_path).expanduser()
    content = ""
    raw_bytes = old_bytes if old_bytes is not UNSET else (path.read_bytes() if path.exists() else None)
    if raw_bytes is not None and len(raw_bytes) > 0:
        try:
            content = raw_bytes.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise McpConflictError(f"TOML 文件编码错误: {config_path} ({exc})") from exc
        if tomllib is not None:
            try:
                tomllib.loads(content)
            except Exception as exc:
                raise McpConflictError(f"现有 TOML 配置解析失败，拒绝覆盖: {config_path} ({exc})") from exc

    parts = re.split(r'(\n\s*\[)', content)
    chunks = [parts[0]]
    for i in range(1, len(parts), 2): chunks.append(parts[i] + parts[i+1])
    chunks = merge_toml_chunks_by_server(chunks)
    
    new_chunks = []
    preserved_names = set(preserved_servers)
    for chunk in chunks:
        srv_name = extract_toml_mcp_server_name(chunk)
        if srv_name:
            if only_servers is not None and srv_name not in only_servers:
                new_chunks.append(chunk)
                continue
            if srv_name in preserved_names:
                new_chunks.append(chunk)
                continue
            is_managed_chunk = '_MANAGED_BY = "agent-manager"' in chunk
            if srv_name in active_servers:
                if not is_managed_chunk:
                    if is_sqlite_active():
                        raise McpConflictError(f"发现同名外部 TOML 项: {srv_name}，已有同名外部项不得覆盖")
                    logging.info(f"[AGENT: {agent_name}] 接管同名 TOML 段: {srv_name}")
                continue
            if is_managed_chunk:
                if no_binding_servers and srv_name in no_binding_servers:
                    logging.warning(f"[AGENT: {agent_name}] 受管项 '{srv_name}' 无历史绑定记录，仅凭标记不擅自删除，已保留")
                    new_chunks.append(chunk)
                continue
        new_chunks.append(chunk)
    
    base_content = "".join(new_chunks).strip()
    new_blocks = []
    for name, settings in active_servers.items():
        if only_servers is not None and name not in only_servers:
            continue
        quoted_name = toml_key(name)
        block = f"\n\n[mcp_servers.{quoted_name}]\ncommand = {toml_quote(settings['command'])}\n"
        block += f"args = {json.dumps(settings['args'], ensure_ascii=False)}\n"
        for key in TOML_SERVER_INT_KEYS:
            if key not in settings:
                continue
            value = settings[key]
            if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
                raise McpBindingError(f"{name}.{key} 必须是正整数")
            block += f"{toml_key(key)} = {value}\n"
        env = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
        block += f"[mcp_servers.{quoted_name}.env]\n"
        for k, v in env.items():
            block += f"{toml_key(k)} = {toml_quote(v)}\n"
        new_blocks.append(block)
        if not dry_run:
            logging.info(f"[AGENT: {agent_name}] 更新 MCP: {name}")
    
    full_content = base_content + "".join(new_blocks)
    if tomllib is not None:
        try:
            tomllib.loads(full_content.strip() + "\n")
        except Exception as exc:
            raise McpConflictError(f"生成的 TOML 非法，已停止写入: {exc}") from exc
    return full_content.strip() + "\n"


def update_toml_config(agent_name, config_path, active_servers, all_managed_servers, preserved_servers, dry_run, only_servers=None, expected_old_bytes=UNSET):
    """Atomically update TOML configuration with concurrency check via safe_atomic_write_and_verify"""
    try:
        rendered = render_toml_content(
            agent_name,
            config_path,
            active_servers,
            all_managed_servers,
            preserved_servers,
            dry_run=dry_run,
            only_servers=only_servers,
            old_bytes=expected_old_bytes,
        )
    except McpConflictError:
        raise
    except Exception as exc:
        logging.error(f"[AGENT: {agent_name}] TOML 渲染失败: {exc}")
        return False

    if dry_run:
        return True

    expected_toml_servers = {
        name: settings
        for name, settings in active_servers.items()
        if only_servers is None or name in only_servers
    }
    try:
        safe_atomic_write_and_verify(
            Path(config_path),
            content_writer=lambda: rendered,
            verify_cb=make_managed_entries_verify_cb(agent_name, "toml", expected_toml_servers),
            expected_old_bytes=expected_old_bytes,
        )
        logging.info(f"[AGENT: {agent_name}] TOML 配置已原子更新。")
        return True
    except McpConflictError:
        raise
    except Exception as e:
        logging.error(f"TOML 写入失败: {e}")
        return False
    return True

def backup_config(path, dry_run=False, agent_name=None):
    """5-generation rotating backup"""
    path = Path(path).expanduser()
    if not path.exists():
        return True
    timestamp = datetime.datetime.now().strftime("%Y%m%d%H%M%S")
    bak_path = path.with_suffix(f"{path.suffix}.bak.{timestamp}")
    if not dry_run:
        try:
            shutil.copy2(path, bak_path)
        except OSError as exc:
            label = f"[AGENT: {agent_name}] " if agent_name else ""
            logging.error(f"{label}备份现有 MCP 配置失败: {exc}")
            return False
        backups = sorted(list(path.parent.glob(f"{path.name}.bak.*")), key=os.path.getmtime, reverse=True)
        for old_bak in backups[5:]:
            try: os.remove(old_bak)
            except OSError: pass
    return True


def should_sync_mcp_target(agent_name, agent_defs):
    if not agent_defs:
        return True
    agent_def = agent_defs.get(agent_name)
    if agent_def is None:
        # Codex is config-managed even though it has no lifecycle install metadata yet.
        return canonicalize_agent_id(agent_name) == "codex"
    return is_agent_installed(agent_name, agent_def)


def preflight_existing_entry(target_agent, target_server, existing_entry, is_managed, existing_binding, bootstrap_expected_digest=None):
    """五动作共用 preflight（在 dry-run 短路前执行）：目标条目已存在时必须受管且与记录摘要一致。

    - 外部非受管同名项：拒绝覆盖；
    - 受管项与 binding.applied_digest 不一致：外部修改（含保留 marker 但改 env/未知字段），拒绝；
    - 处于禁用状态 (enabled=False)：配置文件中不得出现该受管项，否则属于外部修改，拒绝；
    - 无活跃历史绑定 (无 binding 或 assigned=False 墓碑)：必须与当前有效定义摘要一致才允许 bootstrap，拒绝仅凭 marker 接管。
    返回错误消息字符串；通过返回 None。错误信息不含 definition/env 内容。
    """
    if existing_entry is None:
        return None
    if not is_managed:
        return f"[AGENT: {target_agent}] 已存在同名外部 MCP 项 '{target_server}'，已有同名外部项不得覆盖"
    cur_digest = compute_applied_digest(
        existing_entry["command"], existing_entry["args"], existing_entry["env"], existing_entry["extra"]
    )
    active_binding = existing_binding if (existing_binding and existing_binding.get("assigned")) else None
    if active_binding:
        if active_binding.get("enabled") is False:
            return f"[AGENT: {target_agent}] 外部文件并发变更: 受管项 '{target_server}' 被外部修改，拒绝覆盖"
        recorded_digest = active_binding.get("applied_digest")
        if not recorded_digest or cur_digest != recorded_digest:
            return f"[AGENT: {target_agent}] 外部文件并发变更: 受管项 '{target_server}' 被外部修改，拒绝覆盖"
    else:
        if bootstrap_expected_digest is None or cur_digest != bootstrap_expected_digest:
            return f"[AGENT: {target_agent}] 受管项 '{target_server}' 与当前定义不一致且无历史绑定，拒绝仅凭标记接管"
    return None


def make_managed_entries_verify_cb(agent_name, file_format, expected_servers):
    """构造读回校验回调：逐 server 核对受管标记与完整实际条目摘要（含未知字段）。

    被外部改过但保留 marker 的配置会因摘要不匹配而判失败并触发回滚。
    TOML 渲染器仅写 command/args/整型超时/env，期望摘要按渲染输出取相同子集。
    """
    def _expected_digest(settings):
        if file_format == "toml":
            extra = {
                k: settings[k]
                for k in TOML_SERVER_INT_KEYS
                if k in settings and settings[k] is not None
            }
        else:
            extra = settings
        return compute_applied_digest(
            settings.get("command"), settings.get("args"), settings.get("env"), extra
        )

    def _verify(p):
        for name, settings in expected_servers.items():
            entry, is_managed = extract_server_from_config(p, agent_name, file_format, name)
            if entry is None or not is_managed:
                return False
            actual_digest = compute_applied_digest(
                entry["command"], entry["args"], entry["env"], entry["extra"]
            )
            if actual_digest != _expected_digest(settings):
                return False
        return True
    return _verify

def manage_mcp_config(agents_config, mcp_servers, dry_run=False, agent_defs=None, only_servers=None, raw_mcp_servers=None, env_vars=None):
    """Update the MCP configuration file for each Agent"""
    all_ok = True
    declared_managed_servers = {
        name for name, srv_config in mcp_servers.items() if srv_config.get("_declared_in_policy", True)
    }
    db_bindings = {}
    if is_sqlite_active():
        try:
            db_bindings = load_mcp_bindings(get_db_path())
        except Exception as exc:
            logging.error(f"加载数据库绑定失败: {exc}")
            return False
        for s_name, a_map in db_bindings.items():
            if isinstance(a_map, dict) and any(b.get("assigned") for b in a_map.values() if isinstance(b, dict)):
                declared_managed_servers.add(s_name)

    for agent_name, agent_meta in agents_config.items():
        agent_installed = should_sync_mcp_target(agent_name, agent_defs)
        config_path_str = agent_meta if isinstance(agent_meta, str) else agent_meta.get("path")
        file_format = "json" if isinstance(agent_meta, str) else agent_meta.get("format", "json")
        config_path = Path(config_path_str).expanduser()
        old_bytes = config_path.read_bytes() if config_path.exists() else None
        scoped_binding_changes = []

        if only_servers is not None:
            agent_relevant = False
            for s_name in only_servers:
                if is_sqlite_active():
                    b = get_binding(db_bindings, s_name, agent_name)
                    if b and b.get("assigned"):
                        agent_relevant = True
                        break
                s_cfg = mcp_servers.get(s_name)
                if s_cfg is not None:
                    s_agents = s_cfg.get("agents")
                    if s_agents is None or agent_name in s_agents:
                        agent_relevant = True
                        break
            if not agent_relevant and config_path.exists():
                if file_format == "json":
                    try:
                        with open(config_path, "r", encoding="utf-8") as f:
                            raw_doc = json.load(f)
                        servers_dict = raw_doc.get("mcp" if agent_name == "opencode" else "mcpServers") or {}
                        if any(s in servers_dict for s in only_servers):
                            agent_relevant = True
                    except Exception:
                        pass
                elif file_format == "toml":
                    try:
                        toml_text = config_path.read_text(encoding="utf-8")
                        existing_names = set()
                        if tomllib is not None:
                            try:
                                parsed = tomllib.loads(toml_text)
                                if isinstance(parsed, dict) and isinstance(parsed.get("mcp_servers"), dict):
                                    existing_names = set(parsed["mcp_servers"].keys())
                            except Exception:
                                pass
                        if not existing_names:
                            for m in TOML_SERVER_HEADER_RE.finditer(toml_text):
                                k = m.group("key")
                                existing_names.add(json.loads(k) if k.startswith('"') else k)
                        if any(s in existing_names for s in only_servers):
                            agent_relevant = True
                    except Exception:
                        pass
            if not agent_relevant:
                continue

        active_mcp_settings = {}
        preserved_failed_servers = set()
        candidate_names = list(mcp_servers.keys())
        if is_sqlite_active():
            for s_name, a_map in db_bindings.items():
                if s_name not in candidate_names and isinstance(a_map, dict):
                    b_entry = a_map.get(agent_name)
                    if b_entry and b_entry.get("assigned"):
                        candidate_names.append(s_name)

        for name in candidate_names:
            srv_config = mcp_servers.get(name)
            if is_sqlite_active():
                binding_entry = get_binding(db_bindings, name, agent_name)
                if binding_entry is not None:
                    if not binding_entry.get("assigned", True):
                        continue
                    # 预检必须先于任何撤下/跳过决策：已绑定目标的现有受管项
                    # 必须与旧 binding 摘要一致，否则外部变化时整 Agent 零写失败。
                    digest_mismatch = False
                    cur_digest = None
                    try:
                        file_entry, is_managed = extract_server_from_config(config_path, agent_name, file_format, name)
                    except McpConflictError as exc:
                        logging.error(f"[AGENT: {agent_name}] 现有配置文件损坏/冲突: {exc}")
                        all_ok = False
                        preserved_failed_servers.add(name)
                        scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue
                    if file_entry is not None and is_managed:
                        recorded_digest = binding_entry.get("applied_digest")
                        if recorded_digest:
                            cur_digest = compute_applied_digest(file_entry["command"], file_entry["args"], file_entry["env"], file_entry["extra"])
                            digest_mismatch = cur_digest != recorded_digest
                    # 撤下/跳过路径只允许处理与旧 binding 一致的项：外部变化即整 Agent 零写失败
                    if not binding_entry.get("enabled", True):
                        if digest_mismatch:
                            logging.error(f"[AGENT: {agent_name}] 外部文件并发变更: 受管项 '{name}' 被外部修改，拒绝覆盖")
                            all_ok = False
                            preserved_failed_servers.add(name)
                            scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue
                    if srv_config is not None and not srv_config.get("enabled", True):
                        if digest_mismatch:
                            logging.error(f"[AGENT: {agent_name}] 外部文件并发变更: 受管项 '{name}' 被外部修改，拒绝覆盖")
                            all_ok = False
                            preserved_failed_servers.add(name)
                            scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue
                    if srv_config is not None and srv_config.get("agents") is not None:
                        allowed = [canonicalize_agent_id(a) for a in srv_config["agents"]]
                        if canonicalize_agent_id(agent_name) not in allowed:
                            if digest_mismatch:
                                logging.error(f"[AGENT: {agent_name}] 外部文件并发变更: 受管项 '{name}' 被外部修改，拒绝覆盖")
                                all_ok = False
                                preserved_failed_servers.add(name)
                                scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                            continue
                    if file_entry is not None and not is_managed:
                        logging.error(f"[AGENT: {agent_name}] 已存在同名外部 MCP 项 '{name}'，已有同名外部项不得覆盖")
                        all_ok = False
                        preserved_failed_servers.add(name)
                        scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue

                    pinned_def = copy.deepcopy(binding_entry.get("definition", {}))
                    if "env" in pinned_def and isinstance(pinned_def["env"], dict):
                        for k, v in pinned_def["env"].items():
                            var_name = placeholder_name(v)
                            if var_name:
                                if env_vars and var_name in env_vars:
                                    pinned_def["env"][k] = env_vars[var_name]
                                elif var_name in os.environ:
                                    pinned_def["env"][k] = os.environ[var_name]
                    try:
                        effective_args = resolve_mcp_server_args(pinned_def, agent_name)
                    except ValueError as exc:
                        logging.error(f"[AGENT: {agent_name}] MCP {name} 参数无效: {exc}")
                        all_ok = False
                        preserved_failed_servers.add(name)
                        scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue
                    repo_path_str = srv_config.get("_repo_path", "") if srv_config else ""
                    repo_path = Path(repo_path_str).resolve() if repo_path_str else ""
                    parsed_args = [arg.replace("{repo_path}", str(repo_path)) for arg in effective_args]
                    pinned_def["args"] = parsed_args

                    unresolved = find_unresolved_placeholders(pinned_def)
                    if unresolved:
                        missing_vars = sorted(set(var for _, var in unresolved))
                        logging.error(f"[AGENT: {agent_name}] MCP 服务 '{name}' 存在未解析的凭证变量，拒绝更新: {missing_vars}")
                        all_ok = False
                        preserved_failed_servers.add(name)
                        scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue

                    active_settings = {
                        "command": pinned_def.get("command"),
                        "args": parsed_args,
                        "env": pinned_def.get("env", {}),
                    }
                    if agent_name == "opencode":
                        active_settings["type"] = "local"
                    if file_format == "toml":
                        for key in TOML_SERVER_INT_KEYS:
                            if key in pinned_def:
                                active_settings[key] = pinned_def[key]
                    active_mcp_settings[name] = active_settings
                    new_digest = compute_applied_digest(active_settings["command"], active_settings["args"], active_settings["env"], active_settings)
                    # 活跃路径：与旧摘要不一致时，仅当文件与绑定定义派生内容一致（DB 摘要过期）
                    # 才允许继续；否则属于外部修改，拒绝覆盖。
                    if digest_mismatch and cur_digest != new_digest:
                        logging.error(f"[AGENT: {agent_name}] 外部文件并发变更: 受管项 '{name}' 被外部修改，拒绝覆盖")
                        all_ok = False
                        preserved_failed_servers.add(name)
                        del active_mcp_settings[name]
                        scoped_binding_changes = [c for c in scoped_binding_changes if c[0] != name]
                        continue
                    if binding_entry.get("applied_digest") != new_digest:
                        old_binding_snapshot = copy.deepcopy(binding_entry)
                        new_entry = set_binding(db_bindings, name, agent_name, version=binding_entry.get("version", ""), definition=binding_entry.get("definition", {}), enabled=True, assigned=True, applied_digest=new_digest)
                        scoped_binding_changes.append((name, agent_name, old_binding_snapshot, copy.deepcopy(new_entry)))
                else:
                    if srv_config is not None and not srv_config.get("_declared_in_policy", True):
                        continue
                    if srv_config is None:
                        continue
                    try:
                        file_entry, is_managed = extract_server_from_config(config_path, agent_name, file_format, name)
                    except McpConflictError as exc:
                        logging.error(f"[AGENT: {agent_name}] 现有配置文件损坏/冲突: {exc}")
                        all_ok = False
                        preserved_failed_servers.add(name)
                        continue
                    if file_entry is not None:
                        if not is_managed:
                            preserved_failed_servers.add(name)
                            continue
                        file_digest = compute_applied_digest(file_entry["command"], file_entry["args"], file_entry["env"], file_entry["extra"])
                        effective_args = resolve_mcp_server_args(srv_config, agent_name)
                        repo_path_str = srv_config.get("_repo_path", "")
                        repo_path = Path(repo_path_str).resolve() if repo_path_str else ""
                        parsed_args = [arg.replace("{repo_path}", str(repo_path)) for arg in effective_args]
                        expected_settings = {
                            "command": srv_config.get("command"),
                            "args": parsed_args,
                            "env": srv_config.get("env", {}),
                        }
                        if agent_name == "opencode":
                            expected_settings["type"] = "local"
                        if file_format == "toml":
                            for key in TOML_SERVER_INT_KEYS:
                                if key in srv_config:
                                    expected_settings[key] = srv_config[key]
                        expected_digest = compute_applied_digest(expected_settings["command"], expected_settings["args"], expected_settings["env"], expected_settings)
                        if file_digest == expected_digest:
                            raw_def = clean_definition(raw_mcp_servers[name]) if raw_mcp_servers and name in raw_mcp_servers else clean_definition(srv_config)
                            version = compute_definition_version(raw_def)
                            new_entry = set_binding(db_bindings, name, agent_name, version=version, definition=raw_def, enabled=True, assigned=True, applied_digest=file_digest)
                            scoped_binding_changes.append((name, agent_name, None, copy.deepcopy(new_entry)))
                            active_settings = {
                                "command": srv_config.get("command"),
                                "args": parsed_args,
                                "env": srv_config.get("env", {}),
                            }
                            if agent_name == "opencode":
                                active_settings["type"] = "local"
                            if file_format == "toml":
                                for key in TOML_SERVER_INT_KEYS:
                                    if key in srv_config:
                                        active_settings[key] = srv_config[key]
                            active_mcp_settings[name] = active_settings
                        else:
                            logging.error(f"[AGENT: {agent_name}] 现有受管项 {name} 内容与当前定义不匹配且无历史绑定，拒绝猜测覆盖")
                            all_ok = False
                            preserved_failed_servers.add(name)
                            continue
                    else:
                        continue
            else:
                if srv_config is not None and not srv_config.get("_declared_in_policy", True): continue
                if srv_config is None or not srv_config.get("enabled", True): continue
                if srv_config.get("agents") and agent_name not in srv_config["agents"]: continue
                if not srv_config.get("_sync_ready", True):
                    preserved_failed_servers.add(name)
                    continue

                try:
                    effective_args = resolve_mcp_server_args(srv_config, agent_name)
                except ValueError as exc:
                    logging.error(f"[AGENT: {agent_name}] MCP {name} 参数无效: {exc}")
                    all_ok = False
                    preserved_failed_servers.add(name)
                    continue

                repo_path_str = srv_config.get("_repo_path", "")
                repo_path = Path(repo_path_str).resolve() if repo_path_str else ""
                parsed_args = [arg.replace("{repo_path}", str(repo_path)) for arg in effective_args]

                active_settings = {
                    "command": srv_config.get("command"),
                    "args": parsed_args,
                    "env": srv_config.get("env", {})
                }
                if file_format == "toml":
                    for key in TOML_SERVER_INT_KEYS:
                        if key in srv_config:
                            active_settings[key] = srv_config[key]
                active_mcp_settings[name] = active_settings

        if is_sqlite_active() and preserved_failed_servers:
            continue

        if not agent_installed:
            if not config_path.exists():
                logging.info(f"[AGENT: {agent_name}] 未安装且无配置文件，跳过 MCP 注入。")
                continue
            if is_sqlite_active() and any(
                isinstance(a_map, dict)
                and isinstance(a_map.get(agent_name), dict)
                and a_map[agent_name].get("assigned")
                for a_map in db_bindings.values()
            ):
                # 程序未安装仅告警，不改变资源绑定意图：已预检的绑定仍按 pinned
                # definition 正常同步（凭据轮换/全局停用等语义保持有效）。
                logging.warning(f"[AGENT: {agent_name}] 未探测到程序安装；按已绑定配置正常同步，跳过未安装清理。")
            else:
                logging.info(f"[AGENT: {agent_name}] 未安装，仅清理已托管配置。")
                active_mcp_settings = {}

        if not active_mcp_settings and not config_path.exists():
            continue

        if not config_path.parent.exists():
            if not dry_run:
                logging.info(f"[AGENT: {agent_name}] Agent 目录缺失，自动创建以确保后续配置注入。")
                try:
                    config_path.parent.mkdir(parents=True, exist_ok=True)
                except Exception as e:
                    logging.error(f"无法创建配置目录 {config_path.parent}: {e}")
                    all_ok = False
                    continue
            else:
                logging.info(f"[AGENT: {agent_name}] Agent 目录缺失，演练模式跳过目录创建。")

        if not dry_run and not is_sqlite_active() and not backup_config(config_path, dry_run, agent_name=agent_name):
            all_ok = False
            continue

        if file_format == "toml":
            if not is_sqlite_active():
                try:
                    ok = update_toml_config(
                        agent_name,
                        config_path,
                        active_mcp_settings,
                        declared_managed_servers,
                        preserved_failed_servers,
                        dry_run,
                        only_servers=only_servers,
                    )
                except Exception as exc:
                    logging.error(f"[AGENT: {agent_name}] {exc}")
                    all_ok = False
                    continue
                if not ok:
                    all_ok = False
                continue

            no_binding_servers = set()
            if old_bytes:
                try:
                    toml_text = old_bytes.decode("utf-8")
                    probe_parts = re.split(r'(\n\s*\[)', toml_text)
                    probe_chunks = [probe_parts[0]]
                    for i in range(1, len(probe_parts), 2):
                        probe_chunks.append(probe_parts[i] + probe_parts[i+1])
                    for chunk in merge_toml_chunks_by_server(probe_chunks):
                        if '_MANAGED_BY = "agent-manager"' not in chunk:
                            continue
                        orphan_name = extract_toml_mcp_server_name(chunk)
                        if not orphan_name or orphan_name in active_mcp_settings:
                            continue
                        orphan_binding = get_binding(db_bindings, orphan_name, agent_name)
                        if not (orphan_binding and orphan_binding.get("assigned")):
                            no_binding_servers.add(orphan_name)
                except Exception:
                    no_binding_servers = set()

            try:
                rendered = render_toml_content(
                    agent_name,
                    config_path,
                    active_mcp_settings,
                    declared_managed_servers,
                    preserved_failed_servers,
                    dry_run=dry_run,
                    only_servers=only_servers,
                    old_bytes=old_bytes,
                    no_binding_servers=no_binding_servers,
                )
            except McpConflictError as c_exc:
                logging.error(f"[AGENT: {agent_name}] {c_exc}")
                all_ok = False
                continue
            except Exception as exc:
                logging.error(f"[AGENT: {agent_name}] TOML 渲染失败: {exc}")
                all_ok = False
                continue

            if dry_run:
                continue

            expected_toml_servers = {
                name: settings
                for name, settings in active_mcp_settings.items()
                if only_servers is None or name in only_servers
            }
            _verify_toml = make_managed_entries_verify_cb(agent_name, "toml", expected_toml_servers)

            db_cb = None
            if scoped_binding_changes:
                db_cb = lambda: save_mcp_bindings(get_db_path(), db_bindings, action="sync", target=agent_name, scoped_changes=scoped_binding_changes)

            try:
                safe_atomic_write_and_verify(
                    config_path,
                    content_writer=lambda: rendered,
                    verify_cb=_verify_toml,
                    db_commit_cb=db_cb,
                    expected_old_bytes=old_bytes,
                )
            except McpConflictError as c_exc:
                logging.error(f"[AGENT: {agent_name}] {c_exc}")
                all_ok = False
                continue
            except Exception as exc:
                logging.error(f"[AGENT: {agent_name}] TOML 写入或验证失败: {exc}")
                all_ok = False
                continue
            continue

        try:
            current_config = parse_json_config_bytes(agent_name, old_bytes, str(config_path))
        except McpConflictError as c_exc:
            logging.error(f"[AGENT: {agent_name}] 现有配置文件损坏/冲突: {c_exc}")
            all_ok = False
            continue

        # OpenCode uses the "mcp" key, while other agents use "mcpServers"
        is_opencode = agent_name == "opencode"
        key_name = "mcp" if is_opencode else "mcpServers"

        if dry_run:
            logging.info(f"[AGENT: {agent_name}] (预览: 计划原子更新 JSON)")
        else:
            if key_name not in current_config: current_config[key_name] = {}
            
            # Orphan Removal
            for k in list(current_config[key_name].keys()):
                if only_servers is not None and k not in only_servers:
                    continue
                srv = current_config[key_name][k]
                if isinstance(srv, dict):
                    # Check _MANAGED_BY in either env or environment
                    env_dict = (srv.get('env') or srv.get('environment') or {})
                    if env_dict.get('_MANAGED_BY') == 'agent-manager':
                        if k in preserved_failed_servers:
                            continue
                        if k not in active_mcp_settings:
                            if is_sqlite_active():
                                k_binding = get_binding(db_bindings, k, agent_name)
                                if not (k_binding and k_binding.get("assigned")):
                                    logging.warning(f"[AGENT: {agent_name}] 受管项 '{k}' 无历史绑定记录，仅凭标记不擅自删除，已保留")
                                    continue
                            logging.info(f"[AGENT: {agent_name}] 移除孤儿 MCP: {k}")
                            del current_config[key_name][k]
            
            # Inject
            for name, settings in active_mcp_settings.items():
                if only_servers is not None and name not in only_servers:
                    continue
                if is_opencode:
                    # OpenCode special format: command is an array, and environment replaces env
                    cmd_array = [settings["command"]] + settings.get("args", [])
                    env_dict = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
                    current_config[key_name][name] = {
                        "type": "local",
                        "command": cmd_array,
                        "environment": env_dict
                    }
                else:
                    # Standard format
                    if "env" not in settings: settings["env"] = {}
                    settings["env"]["_MANAGED_BY"] = "agent-manager"
                    current_config[key_name][name] = settings
                logging.info(f"[AGENT: {agent_name}] 更新 MCP: {name}")

            if is_sqlite_active():
                rendered_json = json.dumps(current_config, indent=2, ensure_ascii=False) + "\n"

                expected_json_servers = {
                    name: settings
                    for name, settings in active_mcp_settings.items()
                    if only_servers is None or name in only_servers
                }
                _verify_json = make_managed_entries_verify_cb(agent_name, "json", expected_json_servers)

                db_cb = None
                if scoped_binding_changes:
                    db_cb = lambda: save_mcp_bindings(get_db_path(), db_bindings, action="sync", target=agent_name, scoped_changes=scoped_binding_changes)

                try:
                    safe_atomic_write_and_verify(
                        config_path,
                        content_writer=lambda: rendered_json,
                        verify_cb=_verify_json,
                        db_commit_cb=db_cb,
                        expected_old_bytes=old_bytes,
                    )
                except McpConflictError as c_exc:
                    logging.error(f"[AGENT: {agent_name}] {c_exc}")
                    all_ok = False
                    continue
                except Exception as exc:
                    logging.error(f"[AGENT: {agent_name}] JSON 写入或验证失败: {exc}")
                    all_ok = False
                    continue
            else:
                try:
                    atomic_write_json(config_path, current_config)
                except Exception as exc:
                    logging.error(f"[AGENT: {agent_name}] JSON 配置写入失败，跳过该端: {exc}")
                    all_ok = False
                    continue
    return all_ok

def sync_secrets_repo(secrets_path):
    """Phase 0: If the secrets directory is a Git repository, perform an automatic update"""
    if not secrets_path.exists(): return
    
    # Check the secrets directory itself and its subdirectories
    targets = [secrets_path] + [d for d in secrets_path.iterdir() if d.is_dir()]
    for target in targets:
        if (target / ".git").exists():
            logging.info(f"发现密钥仓库: {target.name}，正在同步最新凭证...")
            success, output = run_cmd(["git", "pull", "--ff-only"], cwd=target)
            if success:
                logging.info(f"密钥仓库 {target.name} 同步成功。")
            else:
                logging.warning(f"密钥仓库 {target.name} 同步失败 (可能是网络或权限问题): {output}")

def load_env_files(paths):
    env_vars = {}
    for path in paths:
        if not path.exists():
            continue
        logging.info(f"正在从凭证源加载: {path.name}")
        with open(path, 'r', encoding='utf-8') as f_in:
            for line in f_in:
                line = line.strip()
                if not line or line.startswith('#'): continue
                if '=' in line:
                    k, v = line.split('=', 1)
                    env_vars[k.strip()] = v.strip().strip('"').strip("'")
    return env_vars


def render_single_server_json(config_path, agent_name, server_name, settings, old_bytes=UNSET):
    path = Path(config_path).expanduser()
    raw = old_bytes if old_bytes is not UNSET else (path.read_bytes() if path.exists() else None)
    doc = parse_json_config_bytes(agent_name, raw, str(config_path))
    is_opencode = (agent_name == "opencode")
    key_name = "mcp" if is_opencode else "mcpServers"
    if is_opencode:
        cmd_array = [settings["command"]] + list(settings.get("args", []) or [])
        env_dict = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
        doc[key_name][server_name] = {
            "type": "local",
            "command": cmd_array,
            "environment": env_dict,
        }
    else:
        entry = copy.deepcopy(settings)
        entry["env"] = {**settings.get("env", {}), "_MANAGED_BY": "agent-manager"}
        doc[key_name][server_name] = entry
    return json.dumps(doc, indent=2, ensure_ascii=False) + "\n"


def render_remove_server_json(config_path, agent_name, server_name, old_bytes=UNSET):
    path = Path(config_path).expanduser()
    raw = old_bytes if old_bytes is not UNSET else (path.read_bytes() if path.exists() else None)
    if raw is None:
        return "{}\n"
    doc = parse_json_config_bytes(agent_name, raw, str(config_path))
    is_opencode = (agent_name == "opencode")
    key_name = "mcp" if is_opencode else "mcpServers"
    if server_name in doc[key_name]:
        srv = doc[key_name][server_name]
        env_dict = (srv.get("env") or srv.get("environment") or {})
        if env_dict.get("_MANAGED_BY") != "agent-manager":
            raise McpConflictError(f"目标项不是由 agent-manager 管理，拒绝删除: {server_name}")
        del doc[key_name][server_name]
    return json.dumps(doc, indent=2, ensure_ascii=False) + "\n"


def render_single_server_toml(config_path, agent_name, server_name, settings, action="set", old_bytes=UNSET):
    active_servers = {server_name: settings} if action in ("set", "add", "update", "enable") else {}
    return render_toml_content(
        agent_name,
        config_path,
        active_servers=active_servers,
        all_managed_servers={server_name},
        preserved_servers=set(),
        dry_run=False,
        only_servers={server_name},
        old_bytes=old_bytes,
    )


def write_single_server_config(config_path, agent_name, file_format, server_name, settings, expected_old_bytes=UNSET):
    config_path = Path(config_path).expanduser()
    if file_format == "json":
        rendered = render_single_server_json(config_path, agent_name, server_name, settings, old_bytes=expected_old_bytes)
    elif file_format == "toml":
        rendered = render_single_server_toml(config_path, agent_name, server_name, settings, action="set", old_bytes=expected_old_bytes)
    else:
        raise McpBindingError(f"不支持的格式: {file_format}")

    expected_settings = dict(settings)
    if agent_name == "opencode":
        expected_settings.setdefault("type", "local")
    safe_atomic_write_and_verify(
        config_path,
        content_writer=lambda: rendered,
        verify_cb=make_managed_entries_verify_cb(agent_name, file_format, {server_name: expected_settings}),
        expected_old_bytes=expected_old_bytes,
    )


def remove_single_server_config(config_path, agent_name, file_format, server_name, expected_old_bytes=UNSET):
    config_path = Path(config_path).expanduser()
    if not config_path.exists():
        return
    if file_format == "json":
        rendered = render_remove_server_json(config_path, agent_name, server_name, old_bytes=expected_old_bytes)
    elif file_format == "toml":
        rendered = render_single_server_toml(config_path, agent_name, server_name, {}, action="remove", old_bytes=expected_old_bytes)
    else:
        raise McpBindingError(f"不支持的格式: {file_format}")

    def _verify_removed(p):
        entry, _ = extract_server_from_config(p, agent_name, file_format, server_name)
        return entry is None

    safe_atomic_write_and_verify(
        config_path,
        content_writer=lambda: rendered,
        verify_cb=_verify_removed,
        expected_old_bytes=expected_old_bytes,
    )


def execute_agent_action(
    action: str,
    target_agent: str,
    target_server: str,
    raw_definition_cfg: dict,
    definition_cfg: dict,
    AGENT_TARGETS: dict,
    agent_defs: dict,
    env_vars: dict,
    base_dir: Path,
    dry_run: bool = False,
) -> bool:
    db_path = get_db_path()
    bindings = load_mcp_bindings(db_path)
    target_str = f"{target_server}@{target_agent}"

    agent_meta = AGENT_TARGETS[target_agent]
    config_path_str = agent_meta if isinstance(agent_meta, str) else agent_meta.get("path")
    file_format = "json" if isinstance(agent_meta, str) else agent_meta.get("format", "json")
    if file_format not in ("json", "toml"):
        logging.error(f"Agent '{target_agent}' 不支持的配置格式: {file_format}")
        sys.exit(2)
    config_path = Path(config_path_str).expanduser()
    initial_old_bytes = config_path.read_bytes() if config_path.exists() else None

    agent_installed = should_sync_mcp_target(target_agent, agent_defs)
    if not agent_installed and not config_path.exists():
        logging.error(f"Agent '{target_agent}' 未安装且不存在配置文件")
        return False

    try:
        existing_entry, is_managed = extract_server_from_config(config_path, target_agent, file_format, target_server)
    except McpConflictError as exc:
        logging.error(f"[AGENT: {target_agent}] {exc}")
        return False
    existing_binding = get_binding(bindings, target_server, target_agent)
    existing_binding_snapshot = copy.deepcopy(existing_binding)

    if action == "add":
        if target_server not in definition_cfg:
            logging.error(f"定义中未发现 MCP server: {target_server}")
            sys.exit(2)
        srv_agents = definition_cfg[target_server].get("agents")
        if srv_agents is not None:
            allowed = [canonicalize_agent_id(a) for a in srv_agents]
            if canonicalize_agent_id(target_agent) not in allowed:
                logging.error(f"Agent '{target_agent}' 不在 MCP 服务 '{target_server}' 的允许分配范围 ({allowed})")
                sys.exit(2)
        if existing_binding and existing_binding.get("assigned"):
            logging.error(f"Agent '{target_agent}' 已分配 MCP '{target_server}'，无法重复 add（请使用 update）")
            sys.exit(2)
        if definition_cfg[target_server].get("enabled") is False:
            logging.error(f"MCP 服务 '{target_server}' 全局已禁用，拒绝添加")
            sys.exit(2)
        raw_def = clean_definition(raw_definition_cfg[target_server])
        if not raw_def.get("command"):
            logging.error(f"MCP server '{target_server}' 缺少 command，当前切片仅支持 stdio 模式")
            sys.exit(2)

        version = compute_definition_version(raw_def)
        runtime_def = copy.deepcopy(raw_def)
        if "env" in runtime_def:
            for k, v in runtime_def["env"].items():
                var_name = placeholder_name(v)
                if var_name:
                    if env_vars and var_name in env_vars:
                        runtime_def["env"][k] = env_vars[var_name]
                    elif var_name in os.environ:
                        runtime_def["env"][k] = os.environ[var_name]

        effective_args = resolve_mcp_server_args(runtime_def, target_agent)
        target_dir = Path(base_dir) / target_server if raw_def.get("url") else ""
        runtime_def["args"] = [arg.replace("{repo_path}", str(target_dir)) for arg in effective_args]

        unresolved = find_unresolved_placeholders(runtime_def)
        if unresolved:
            missing_vars = sorted(set(var for _, var in unresolved))
            logging.error(f"[AGENT: {target_agent}] MCP 服务 '{target_server}' 存在未解析的凭证变量，拒绝执行: {missing_vars}")
            return False

        applied_settings = {
            "command": runtime_def.get("command"),
            "args": runtime_def.get("args", []),
            "env": runtime_def.get("env", {}),
        }
        if target_agent == "opencode":
            applied_settings["type"] = "local"
        if file_format == "toml":
            for key in TOML_SERVER_INT_KEYS:
                if key in runtime_def:
                    applied_settings[key] = runtime_def[key]

        applied_digest = compute_applied_digest(applied_settings["command"], applied_settings["args"], applied_settings["env"], applied_settings)
        is_globally_enabled = (definition_cfg[target_server].get("enabled") is not False)

        preflight_error = preflight_existing_entry(
            target_agent,
            target_server,
            existing_entry,
            is_managed,
            existing_binding,
            bootstrap_expected_digest=applied_digest,
        )
        if preflight_error:
            logging.error(preflight_error)
            return False

        if dry_run:
            logging.info(f"[AGENT: {target_agent}] (预览: 添加 MCP {target_server})")
            return True

        def _writer():
            if file_format == "json":
                return render_single_server_json(config_path, target_agent, target_server, applied_settings, old_bytes=initial_old_bytes)
            else:
                return render_single_server_toml(config_path, target_agent, target_server, applied_settings, action="add", old_bytes=initial_old_bytes)

        _verify = make_managed_entries_verify_cb(target_agent, file_format, {target_server: applied_settings})

        def _db_commit():
            set_binding(bindings, target_server, target_agent, version=version, definition=raw_def, enabled=True, assigned=True, applied_digest=applied_digest)
            save_mcp_bindings(db_path, bindings, action="add", target=target_str, detail={"version": version}, server=target_server, agent=target_agent, expected_entry=existing_binding_snapshot)

        try:
            safe_atomic_write_and_verify(
                config_path,
                content_writer=_writer,
                verify_cb=_verify,
                db_commit_cb=_db_commit,
                expected_old_bytes=initial_old_bytes,
            )
        except McpConflictError as c_exc:
            logging.error(f"[AGENT: {target_agent}] {c_exc}")
            return False
        except Exception as exc:
            logging.error(f"[AGENT: {target_agent}] 添加 MCP 失败: {exc}")
            return False
        logging.info(f"[AGENT: {target_agent}] 成功添加 MCP: {target_server}")
        return True

    elif action == "update":
        if not existing_binding or not existing_binding.get("assigned"):
            logging.error(f"Agent '{target_agent}' 不存在 MCP '{target_server}' 的分配绑定，无法 update")
            sys.exit(2)
        if target_server not in definition_cfg:
            logging.error(f"未找到 MCP server 定义: {target_server}")
            sys.exit(2)
        srv_agents = definition_cfg[target_server].get("agents")
        if srv_agents is not None:
            allowed = [canonicalize_agent_id(a) for a in srv_agents]
            if canonicalize_agent_id(target_agent) not in allowed:
                logging.error(f"Agent '{target_agent}' 不在 MCP 服务 '{target_server}' 的允许分配范围 ({allowed})")
                sys.exit(2)
        if definition_cfg[target_server].get("enabled") is False:
            logging.error(f"MCP 服务 '{target_server}' 全局已禁用，拒绝更新")
            sys.exit(2)
        raw_def = clean_definition(raw_definition_cfg[target_server])
        if not raw_def.get("command"):
            logging.error(f"MCP server '{target_server}' 缺少 command，当前切片仅支持 stdio 模式")
            sys.exit(2)

        preflight_error = preflight_existing_entry(target_agent, target_server, existing_entry, is_managed, existing_binding)
        if preflight_error:
            logging.error(preflight_error)
            return False

        new_version = compute_definition_version(raw_def)
        is_enabled = existing_binding.get("enabled", True)

        if not is_enabled:
            if dry_run:
                logging.info(f"[AGENT: {target_agent}] (预览: 更新已禁用 MCP {target_server} 版本为 {new_version})")
                return True
            old_entry = copy.deepcopy(existing_binding)
            set_binding(bindings, target_server, target_agent, version=new_version, definition=raw_def, enabled=False, assigned=True, applied_digest=existing_binding.get("applied_digest", ""))
            save_mcp_bindings(db_path, bindings, action="update", target=target_str, detail={"version": new_version}, server=target_server, agent=target_agent, expected_entry=old_entry)
            logging.info(f"[AGENT: {target_agent}] 成功更新已禁用 MCP 版本: {target_server} -> {new_version}")
            return True

        runtime_def = copy.deepcopy(raw_def)
        if "env" in runtime_def:
            for k, v in runtime_def["env"].items():
                var_name = placeholder_name(v)
                if var_name:
                    if env_vars and var_name in env_vars:
                        runtime_def["env"][k] = env_vars[var_name]
                    elif var_name in os.environ:
                        runtime_def["env"][k] = os.environ[var_name]

        effective_args = resolve_mcp_server_args(runtime_def, target_agent)
        target_dir = Path(base_dir) / target_server if raw_def.get("url") else ""
        runtime_def["args"] = [arg.replace("{repo_path}", str(target_dir)) for arg in effective_args]

        unresolved = find_unresolved_placeholders(runtime_def)
        if unresolved:
            missing_vars = sorted(set(var for _, var in unresolved))
            logging.error(f"[AGENT: {target_agent}] MCP 服务 '{target_server}' 存在未解析的凭证变量，拒绝执行: {missing_vars}")
            return False

        applied_settings = {
            "command": runtime_def.get("command"),
            "args": runtime_def.get("args", []),
            "env": runtime_def.get("env", {}),
        }
        if target_agent == "opencode":
            applied_settings["type"] = "local"
        if file_format == "toml":
            for key in TOML_SERVER_INT_KEYS:
                if key in runtime_def:
                    applied_settings[key] = runtime_def[key]

        applied_digest = compute_applied_digest(applied_settings["command"], applied_settings["args"], applied_settings["env"], applied_settings)

        if dry_run:
            logging.info(f"[AGENT: {target_agent}] (预览: 更新 MCP {target_server} 版本为 {new_version})")
            return True

        def _writer():
            if file_format == "json":
                return render_single_server_json(config_path, target_agent, target_server, applied_settings, old_bytes=initial_old_bytes)
            else:
                return render_single_server_toml(config_path, target_agent, target_server, applied_settings, action="update", old_bytes=initial_old_bytes)

        _verify = make_managed_entries_verify_cb(target_agent, file_format, {target_server: applied_settings})

        old_entry = copy.deepcopy(existing_binding)
        def _db_commit():
            set_binding(bindings, target_server, target_agent, version=new_version, definition=raw_def, enabled=True, assigned=True, applied_digest=applied_digest)
            save_mcp_bindings(db_path, bindings, action="update", target=target_str, detail={"version": new_version}, server=target_server, agent=target_agent, expected_entry=old_entry)

        try:
            safe_atomic_write_and_verify(
                config_path,
                content_writer=_writer,
                verify_cb=_verify,
                db_commit_cb=_db_commit,
                expected_old_bytes=initial_old_bytes,
            )
        except McpConflictError as c_exc:
            logging.error(f"[AGENT: {target_agent}] {c_exc}")
            return False
        except Exception as exc:
            logging.error(f"[AGENT: {target_agent}] 更新 MCP 失败: {exc}")
            return False
        logging.info(f"[AGENT: {target_agent}] 成功更新 MCP: {target_server} -> {new_version}")
        return True

    elif action == "disable":
        if not existing_binding or not existing_binding.get("assigned"):
            logging.error(f"Agent '{target_agent}' 不存在 MCP '{target_server}' 的分配绑定，无法 disable")
            sys.exit(2)

        preflight_error = preflight_existing_entry(target_agent, target_server, existing_entry, is_managed, existing_binding)
        if preflight_error:
            logging.error(preflight_error)
            return False

        if dry_run:
            logging.info(f"[AGENT: {target_agent}] (预览: 禁用 MCP {target_server})")
            return True

        def _writer():
            if file_format == "json":
                return render_remove_server_json(config_path, target_agent, target_server, old_bytes=initial_old_bytes)
            else:
                return render_single_server_toml(config_path, target_agent, target_server, {}, action="remove", old_bytes=initial_old_bytes)

        def _verify(p):
            entry, _ = extract_server_from_config(p, target_agent, file_format, target_server)
            return entry is None

        old_entry = copy.deepcopy(existing_binding)
        def _db_commit():
            set_binding(bindings, target_server, target_agent, version=existing_binding["version"], definition=existing_binding["definition"], enabled=False, assigned=True, applied_digest=existing_binding.get("applied_digest", ""))
            save_mcp_bindings(db_path, bindings, action="disable", target=target_str, detail={"version": existing_binding["version"]}, server=target_server, agent=target_agent, expected_entry=old_entry)

        try:
            safe_atomic_write_and_verify(
                config_path,
                content_writer=_writer,
                verify_cb=_verify,
                db_commit_cb=_db_commit,
                expected_old_bytes=initial_old_bytes,
            )
        except McpConflictError as c_exc:
            logging.error(f"[AGENT: {target_agent}] {c_exc}")
            return False
        except Exception as exc:
            logging.error(f"[AGENT: {target_agent}] 禁用 MCP 失败: {exc}")
            return False
        logging.info(f"[AGENT: {target_agent}] 成功禁用 MCP: {target_server}")
        return True

    elif action == "enable":
        if not existing_binding or not existing_binding.get("assigned"):
            logging.error(f"Agent '{target_agent}' 不存在 MCP '{target_server}' 的分配绑定，无法 enable")
            sys.exit(2)
        srv_agents = definition_cfg.get(target_server, {}).get("agents")
        if srv_agents is not None:
            allowed = [canonicalize_agent_id(a) for a in srv_agents]
            if canonicalize_agent_id(target_agent) not in allowed:
                logging.error(f"Agent '{target_agent}' 不在 MCP 服务 '{target_server}' 的允许分配范围 ({allowed})")
                sys.exit(2)

        preflight_error = preflight_existing_entry(target_agent, target_server, existing_entry, is_managed, existing_binding)
        if preflight_error:
            logging.error(preflight_error)
            return False

        pinned_def = copy.deepcopy(existing_binding["definition"])
        runtime_def = copy.deepcopy(pinned_def)
        if "env" in runtime_def:
            for k, v in runtime_def["env"].items():
                var_name = placeholder_name(v)
                if var_name:
                    if env_vars and var_name in env_vars:
                        runtime_def["env"][k] = env_vars[var_name]
                    elif var_name in os.environ:
                        runtime_def["env"][k] = os.environ[var_name]

        effective_args = resolve_mcp_server_args(runtime_def, target_agent)
        target_dir = Path(base_dir) / target_server if pinned_def.get("url") else ""
        runtime_def["args"] = [arg.replace("{repo_path}", str(target_dir)) for arg in effective_args]

        unresolved = find_unresolved_placeholders(runtime_def)
        if unresolved:
            missing_vars = sorted(set(var for _, var in unresolved))
            logging.error(f"[AGENT: {target_agent}] MCP 服务 '{target_server}' 存在未解析的凭证变量，拒绝执行: {missing_vars}")
            return False

        applied_settings = {
            "command": runtime_def.get("command"),
            "args": runtime_def.get("args", []),
            "env": runtime_def.get("env", {}),
        }
        if target_agent == "opencode":
            applied_settings["type"] = "local"
        if file_format == "toml":
            for key in TOML_SERVER_INT_KEYS:
                if key in runtime_def:
                    applied_settings[key] = runtime_def[key]

        applied_digest = compute_applied_digest(applied_settings["command"], applied_settings["args"], applied_settings["env"], applied_settings)
        is_globally_enabled = (definition_cfg.get(target_server, {}).get("enabled") is not False)
        if not is_globally_enabled:
            logging.error(f"MCP 服务 '{target_server}' 全局已禁用，拒绝恢复启用")
            sys.exit(2)

        if dry_run:
            logging.info(f"[AGENT: {target_agent}] (预览: 恢复启用 MCP {target_server})")
            return True

        def _writer():
            if file_format == "json":
                return render_single_server_json(config_path, target_agent, target_server, applied_settings, old_bytes=initial_old_bytes)
            else:
                return render_single_server_toml(config_path, target_agent, target_server, applied_settings, action="enable", old_bytes=initial_old_bytes)

        _verify = make_managed_entries_verify_cb(target_agent, file_format, {target_server: applied_settings})

        old_entry = copy.deepcopy(existing_binding)
        def _db_commit():
            set_binding(bindings, target_server, target_agent, version=existing_binding["version"], definition=existing_binding["definition"], enabled=True, assigned=True, applied_digest=applied_digest)
            save_mcp_bindings(db_path, bindings, action="enable", target=target_str, detail={"version": existing_binding["version"]}, server=target_server, agent=target_agent, expected_entry=old_entry)

        try:
            safe_atomic_write_and_verify(
                config_path,
                content_writer=_writer,
                verify_cb=_verify,
                db_commit_cb=_db_commit,
                expected_old_bytes=initial_old_bytes,
            )
        except McpConflictError as c_exc:
            logging.error(f"[AGENT: {target_agent}] {c_exc}")
            return False
        except Exception as exc:
            logging.error(f"[AGENT: {target_agent}] 启用 MCP 失败: {exc}")
            return False
        logging.info(f"[AGENT: {target_agent}] 成功恢复启用 MCP: {target_server}")
        return True

    elif action == "remove":
        if not existing_binding:
            logging.error(f"Agent '{target_agent}' 不存在 MCP '{target_server}' 的分配绑定，无法 remove")
            sys.exit(2)

        preflight_error = preflight_existing_entry(target_agent, target_server, existing_entry, is_managed, existing_binding)
        if preflight_error:
            logging.error(preflight_error)
            return False

        if dry_run:
            logging.info(f"[AGENT: {target_agent}] (预览: 移除 MCP {target_server})")
            return True

        if not config_path.exists():
            old_entry = copy.deepcopy(existing_binding)
            remove_binding(bindings, target_server, target_agent)
            save_mcp_bindings(db_path, bindings, action="remove", target=target_str, server=target_server, agent=target_agent, expected_entry=old_entry)
            logging.info(f"[AGENT: {target_agent}] 成功移除 MCP 分配: {target_server}")
            return True

        def _writer():
            if file_format == "json":
                return render_remove_server_json(config_path, target_agent, target_server, old_bytes=initial_old_bytes)
            else:
                return render_single_server_toml(config_path, target_agent, target_server, {}, action="remove", old_bytes=initial_old_bytes)

        def _verify(p):
            entry, _ = extract_server_from_config(p, target_agent, file_format, target_server)
            return entry is None

        old_entry = copy.deepcopy(existing_binding)
        def _db_commit():
            remove_binding(bindings, target_server, target_agent)
            save_mcp_bindings(db_path, bindings, action="remove", target=target_str, server=target_server, agent=target_agent, expected_entry=old_entry)

        try:
            safe_atomic_write_and_verify(
                config_path,
                content_writer=_writer,
                verify_cb=_verify,
                db_commit_cb=_db_commit,
                expected_old_bytes=initial_old_bytes,
            )
        except McpConflictError as c_exc:
            logging.error(f"[AGENT: {target_agent}] {c_exc}")
            return False
        except Exception as exc:
            logging.error(f"[AGENT: {target_agent}] 移除 MCP 失败: {exc}")
            return False
        logging.info(f"[AGENT: {target_agent}] 成功移除 MCP 分配: {target_server}")
        return True

    return False


def main():
    parser = argparse.ArgumentParser(description="MCP Server 管理工具 (策略对齐)")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略路径")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "mcp.json"), help="定义路径")
    parser.add_argument("--base-dir", default=str(Path(__file__).resolve().parent), help="基础目录")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过远程仓库与 secrets 更新)")
    parser.add_argument("--prepare-runtime", action="store_true", help="显式准备 repo/runtime（clone、pull、setup、build）")
    parser.add_argument("-f", "--force-refresh", action="store_true", help="更新探测失败时仍强制执行远端刷新")
    parser.add_argument("--only", action="append", default=[], help="仅针对指定 MCP server（可重复；支持 mcp:<name> 或 <name>）")
    parser.add_argument("--agent", help="指定单个已知 Agent（逐 Agent 动作必填）")
    parser.add_argument(
        "--action",
        choices=["add", "update", "enable", "disable", "remove"],
        help="逐 Agent 分配动作 (add|update|enable|disable|remove)",
    )
    args = parser.parse_args()
    
    try:
        policy_root = load_json_object_file(args.config, strict=True, missing_ok=False)
        full_def = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        policy_cfg = get_typed_mapping_section(policy_root, "mcp", Path(args.config).name, (dict,), "对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)

    # Separation of concerns: read policy from manager.json, read definitions and paths from mcp.json
    try:
        definition_cfg = get_typed_mapping_section(full_def, "servers", Path(args.def_file).name, (dict,), "对象")
        agent_paths = get_typed_mapping_section(full_def, "agent_paths", Path(args.def_file).name, (str, dict), "字符串或对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)
    AGENT_TARGETS = canonicalize_agent_mapping(agent_paths)

    # 定义覆盖层：merged 策略里的 mcp.definitions 片段深合并到 mcp.json 之上
    #（tombstone `_delete: true` 移除条目；形状 fail-close；未知键告警不拒绝）。
    from src.policy_layers import apply_definition_overlays, overlay_definitions
    try:
        definition_cfg, overlay_warnings = apply_definition_overlays(
            definition_cfg, overlay_definitions(policy_root, "mcp"), domain="mcp"
        )
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)
    for overlay_warning in overlay_warnings:
        logging.warning(overlay_warning)

    # Recursively merge policy toggles（"definitions" 是保留键，不是 server 名）
    for name, s_def in definition_cfg.items():
        s_def["_declared_in_policy"] = name in policy_cfg
        if name in policy_cfg and name != "definitions":
            s_def.update(policy_cfg[name])
        s_def["agents"] = canonicalize_agent_list(s_def.get("agents"))

    raw_definition_cfg = {
        name: clean_definition(s_def)
        for name, s_def in definition_cfg.items()
    }

    only_servers = None
    if args.only:
        only_servers = set()
        for item in args.only:
            if ":" in item and not item.startswith("mcp:"):
                logging.error(f"跨域元素 scope 拒绝: {item}")
                sys.exit(2)
            norm = item.split(":", 1)[1] if item.startswith("mcp:") else item
            only_servers.add(norm)

    db_path = get_db_path()
    db_bindings = {}
    if is_sqlite_active():
        try:
            gate_conn = connect_ready_db(db_path, read_only=True)
            gate_conn.close()
            db_bindings = load_mcp_bindings(db_path)
        except Exception as exc:
            if args.action:
                logging.error(f"数据库未就绪或损坏，拒绝执行动作: {exc}")
                sys.exit(2)
            else:
                logging.error(f"加载数据库失败: {exc}")
                sys.exit(1)

    if args.action and not args.agent:
        logging.error("逐 Agent 动作必须指定 --agent")
        sys.exit(2)

    target_agent = None
    if args.agent:
        target_agent = canonicalize_agent_id(args.agent)
        matched_agent = None
        for a_key in AGENT_TARGETS:
            if canonicalize_agent_id(a_key) == target_agent or a_key.lower() == args.agent.lower():
                matched_agent = a_key
                break
        if not matched_agent:
            logging.error(f"未知 Agent: {args.agent}")
            sys.exit(2)
        target_agent = matched_agent

        try:
            check_shared_agent_target(target_agent, AGENT_TARGETS)
        except McpSharedTargetError as exc:
            logging.error(str(exc))
            sys.exit(2)
        AGENT_TARGETS = {target_agent: AGENT_TARGETS[target_agent]}

    if args.action:
        if not args.only or len(args.only) != 1 or len(only_servers) != 1:
            logging.error("逐 Agent 动作必须通过 --only 指定且仅指定单个 server")
            sys.exit(2)
        if not is_sqlite_active():
            logging.error("逐 Agent 动作要求已初始化 SQLite 数据库 (manager.db)，不能静默降级。")
            sys.exit(2)

        target_server = list(only_servers)[0]
        known_servers = set(definition_cfg.keys()) | set(full_def.get("servers", {}).keys()) | set(db_bindings.keys())
        if target_server not in known_servers:
            logging.error(f"未知 MCP server: {target_server}")
            sys.exit(2)

    if only_servers:
        # Evidence of known servers: active definitions, base definitions, or observed managed in agent configs
        known_servers = set(definition_cfg.keys()) | set(full_def.get("servers", {}).keys()) | set(db_bindings.keys())
        for agent_name, agent_meta in AGENT_TARGETS.items():
            config_path_str = agent_meta if isinstance(agent_meta, str) else agent_meta.get("path")
            file_format = "json" if isinstance(agent_meta, str) else agent_meta.get("format", "json")
            config_path = Path(config_path_str).expanduser()
            if not config_path.exists():
                continue
            if file_format == "json":
                try:
                    with open(config_path, "r", encoding="utf-8") as f:
                        data = json.load(f)
                    for key in ("mcpServers", "mcp"):
                        for srv_k, srv_v in data.get(key, {}).items():
                            if isinstance(srv_v, dict):
                                env_d = srv_v.get("env") or srv_v.get("environment") or {}
                                if env_d.get("_MANAGED_BY") == "agent-manager":
                                    known_servers.add(srv_k)
                except Exception:
                    pass
            elif file_format == "toml":
                try:
                    with open(config_path, "r", encoding="utf-8") as f:
                        content = f.read()
                    for chunk in re.split(r'(?=\n\[mcp_servers\.)', content):
                        if '_MANAGED_BY = "agent-manager"' in chunk:
                            name = extract_toml_mcp_server_name(chunk)
                            if name:
                                known_servers.add(name)
                except Exception:
                    pass
        unknown = sorted(only_servers - known_servers)
        if unknown:
            logging.error(f"未知 MCP server: {', '.join(unknown)}")
            sys.exit(2)

    # [Credential protection] Phase 0: scan and sync the secret pool (secrets/ directory)
    env_vars = {}
    root_dir = Path(args.def_file).resolve().parent.parent
    secrets_dir = root_dir / "secrets"
    
    if args.offline:
        logging.info("离线模式，跳过 secrets 仓库同步。")
    elif only_servers is not None:
        logging.info("指定 scope 模式，仅使用已有凭证，跳过共享 secrets 仓库拉取。")
    elif args.prepare_runtime and not args.dry_run:
        sync_secrets_repo(secrets_dir)
    
    # Load MCP-specific credentials and perform placeholder substitution. Do not scan secrets/*.env,
    # to prevent same-named variables from other domains (e.g. terminal.env) overriding MCP-specific keys.
    env_vars.update(load_env_files([root_dir / ".env", secrets_dir / "mcp.env"]))

    agent_defs = load_json_object_file(REPO_ROOT / "config" / "agents.json", strict=False, missing_ok=True) or {}
    agent_defs, _ = apply_definition_overlays(agent_defs, overlay_definitions(policy_root, "agents"), domain="agents")

    if args.action:
        ok = execute_agent_action(
            action=args.action,
            target_agent=target_agent,
            target_server=target_server,
            raw_definition_cfg=raw_definition_cfg,
            definition_cfg=definition_cfg,
            AGENT_TARGETS=AGENT_TARGETS,
            agent_defs=agent_defs,
            env_vars=env_vars,
            base_dir=Path(args.base_dir).resolve(),
            dry_run=args.dry_run,
        )
        sys.exit(0 if ok else 1)
    
    # Recursively walk definition_cfg and inject
    for srv_name, srv_def in definition_cfg.items():
        if "env" in srv_def:
            for k, v in srv_def["env"].items():
                var_name = placeholder_name(v)
                if var_name is not None:
                    if env_vars and var_name in env_vars:
                        srv_def["env"][k] = env_vars[var_name]
                        logging.info(f"[{srv_name}] 注入凭证: {k} (来源: secrets 池)")
                    elif var_name in os.environ:
                        srv_def["env"][k] = os.environ[var_name]
                        logging.info(f"[{srv_name}] 从系统环境注入: {k}")

    unresolved = []
    for srv_name, srv_def in definition_cfg.items():
        if only_servers is not None and srv_name not in only_servers:
            continue
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
        if only_servers is not None and srv_name not in only_servers:
            continue
        srv_config["_sync_ready"] = srv_config.get("_declared_in_policy", False) and srv_config.get("enabled", True)
        url = srv_config.get("url")
        if not url:
            srv_config["_repo_path"] = ""
            if (
                args.prepare_runtime
                and srv_name == "serena"
                and srv_config["_sync_ready"]
                and not configure_serena_project_folder(
                    srv_config,
                    dry_run=args.dry_run,
                    offline=args.offline,
                )
            ):
                all_build_success = False
                srv_config["_sync_ready"] = False
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
                    force_refresh=args.force_refresh,
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
    agent_defs, _ = apply_definition_overlays(agent_defs, overlay_definitions(policy_root, "agents"), domain="agents")
    if not manage_mcp_config(
        AGENT_TARGETS,
        definition_cfg,
        dry_run=args.dry_run,
        agent_defs=agent_defs,
        only_servers=only_servers,
        raw_mcp_servers=raw_definition_cfg,
        env_vars=env_vars,
    ):
        all_build_success = False
    if not all_build_success: sys.exit(1)
    logging.info("任务完成！")

if __name__ == "__main__":
    main()
