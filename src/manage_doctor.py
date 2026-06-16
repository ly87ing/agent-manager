from __future__ import annotations

import argparse
import hashlib
import json
import ntpath
import os
import re
import shutil
import stat
import subprocess
from pathlib import Path

from src.agent_ids import canonicalize_agent_list, canonicalize_agent_mapping, get_agent_display_name
from src.json_config import ensure_mapping_value_types, get_typed_mapping_section, load_json_object_file
from src.version_pins import apply_local_version_pins
from src.manage_runtime_probe import (
    ProviderInventory,
    canonical_provider_name,
    classify_provider_state,
    collect_active_mcp_target_agents,
    collect_active_rule_agents,
    collect_active_skill_target_agents,
    collect_required_runtime_commands,
    detect_all_providers,
    detect_tool_readiness,
    is_agent_installed,
    load_json,
    rule_target_agent_id,
)
from src.secret_scan import scan_repo_for_secrets, scan_text_for_secrets
from src.toml_compat import load_tomllib

PLACEHOLDER_PATTERN = re.compile(r"^\$\{[^}]+\}$")
RULE_TARGET_PREFIX_TO_ID = {
    "claude": "claude-code",
    "antigravity": "antigravity-cli",
    "codex": "codex",
}
REQUIRED_CONFIG_FILES = (
    "config/manager.json",
    "config/agents.json",
    "config/tools.json",
    "config/skills.json",
    "config/mcp.json",
    "config/rules-targets.list",
)
SUPPORTED_UNSUPPORTED_ACTIONS = {"install", "update", "uninstall", "repair"}


def human_size(num_bytes: int) -> str:
    units = ["B", "KB", "MB", "GB", "TB"]
    size = float(max(num_bytes, 0))
    for unit in units:
        if size < 1024 or unit == units[-1]:
            if unit == "B":
                return f"{int(size)} {unit}"
            return f"{size:.1f} {unit}"
        size /= 1024
    return f"{num_bytes} B"


def build_tree_signature(root: str) -> dict[str, str]:
    signature: dict[str, str] = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        filenames.sort()
        rel_dir = os.path.relpath(dirpath, root)
        rel_dir = "" if rel_dir == "." else rel_dir
        for dirname in dirnames:
            rel_path = os.path.join(rel_dir, dirname).replace(os.sep, "/")
            signature[f"{rel_path}/"] = "dir"
        for filename in filenames:
            rel_path = os.path.join(rel_dir, filename).replace(os.sep, "/")
            signature[rel_path] = file_md5(os.path.join(dirpath, filename)) or ""
    return signature


def rules_match(src: str, dst: str) -> bool:
    if os.path.isdir(src) or os.path.isdir(dst):
        if not (os.path.isdir(src) and os.path.isdir(dst)):
            return False
        return build_tree_signature(src) == build_tree_signature(dst)
    return file_md5(src) == file_md5(dst)


def file_md5(path: str) -> str | None:
    try:
        with open(path, "rb") as handle:
            return hashlib.md5(handle.read()).hexdigest()
    except OSError:
        return None


def make_entry(name: str, status: str, message: str, *, category: str | None = None) -> dict[str, str]:
    entry = {"name": name, "status": status, "message": message}
    if category:
        entry["category"] = category
    return entry


def append_entry(report: dict, section: str, entry: dict) -> None:
    report["sections"].setdefault(section, []).append(entry)
    if entry["status"] == "error":
        report["summary"]["errors"] += 1
    elif entry["status"] == "warning":
        report["summary"]["warnings"] += 1
        category = entry.get("category")
        if category == "optional_tool":
            report["summary"]["warning_categories"]["optional_tools"] += 1
        elif category == "unmaterialized":
            report["summary"]["warning_categories"]["unmaterialized"] += 1
        elif category == "official_gap":
            report["summary"]["warning_categories"]["official_gaps"] += 1


def get_unsupported_actions(provider_cfg: dict | None) -> set[str]:
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


def validate_json_shape(data: dict, name: str) -> None:
    def validate_browser_us_sync() -> None:
        settings = data.get("browser_us_sync", {})
        if settings is None:
            return
        if not isinstance(settings, dict):
            raise ValueError(f"{name}.browser_us_sync 必须是对象")
        if not isinstance(settings.get("enabled", True), bool):
            raise ValueError(f"{name}.browser_us_sync.enabled 必须是布尔值")

    if name == "manager.json":
        get_typed_mapping_section(data, "agents", name, (dict,), "对象")
        get_typed_mapping_section(data, "tools", name, (dict,), "对象")
        get_typed_mapping_section(data, "skills", name, (dict,), "对象")
        get_typed_mapping_section(data, "mcp", name, (dict,), "对象")
        validate_browser_us_sync()
    elif name == "tools.json":
        get_typed_mapping_section(data, "tools", name, (dict,), "对象")
    elif name == "skills.json":
        get_typed_mapping_section(data, "repositories", name, (dict,), "对象")
        get_typed_mapping_section(data, "agents", name, (str, dict), "字符串或对象")
    elif name == "mcp.json":
        get_typed_mapping_section(data, "servers", name, (dict,), "对象")
        get_typed_mapping_section(data, "agent_paths", name, (str, dict), "字符串或对象")
    elif name == "agents.json":
        ensure_mapping_value_types(data, name, (dict,), "对象")


def command_exists(command: str) -> bool:
    if os.path.isabs(command):
        return os.path.exists(command) and os.access(command, os.X_OK)
    return shutil.which(command) is not None


def command_version(command: str) -> str:
    try:
        result = subprocess.run(
            [command, "--version"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return "可用"
    output = (result.stdout or result.stderr or "").strip()
    if not output:
        return "可用"
    return output.splitlines()[0]


def git_version_info(root_dir: Path) -> dict[str, str] | None:
    if not (root_dir / ".git").exists():
        return None
    try:
        commit = subprocess.run(
            ["git", "-C", str(root_dir), "rev-parse", "--short", "HEAD"],
            check=False,
            capture_output=True,
            text=True,
        ).stdout.strip()
        commit_date = subprocess.run(
            ["git", "-C", str(root_dir), "log", "-1", "--format=%cd", "--date=short"],
            check=False,
            capture_output=True,
            text=True,
        ).stdout.strip()
    except OSError:
        return None
    if not commit or not commit_date:
        return None
    return {"commit": commit, "date": commit_date}


def load_inputs(root_dir: Path, config_file: Path) -> dict:
    config = load_json(config_file, strict=True, missing_ok=False)
    config["agents"] = canonicalize_agent_mapping(
        get_typed_mapping_section(config, "agents", "manager.json", (dict,), "对象")
    )
    agent_defs_raw = load_json(root_dir / "config" / "agents.json", strict=True, missing_ok=False)
    agent_defs_raw = apply_local_version_pins(agent_defs_raw, root_dir / "config" / "agents.json")
    agent_defs = canonicalize_agent_mapping(
        ensure_mapping_value_types(agent_defs_raw, "agents.json", (dict,), "对象")
    )
    tools_raw = load_json(root_dir / "config" / "tools.json", strict=True, missing_ok=False)
    tools_raw = apply_local_version_pins(tools_raw, root_dir / "config" / "tools.json")
    tool_defs = get_typed_mapping_section(
        tools_raw,
        "tools",
        "tools.json",
        (dict,),
        "对象",
    )
    skill_cfg_raw = load_json(root_dir / "config" / "skills.json", strict=True, missing_ok=False)
    skill_cfg_raw = apply_local_version_pins(skill_cfg_raw, root_dir / "config" / "skills.json")
    skill_cfg = {
        "repositories": get_typed_mapping_section(skill_cfg_raw, "repositories", "skills.json", (dict,), "对象"),
        "agents": get_typed_mapping_section(skill_cfg_raw, "agents", "skills.json", (str, dict), "字符串或对象"),
    }
    mcp_cfg_raw = load_json(root_dir / "config" / "mcp.json", strict=True, missing_ok=False)
    mcp_cfg_raw = apply_local_version_pins(mcp_cfg_raw, root_dir / "config" / "mcp.json")
    mcp_cfg = {
        "servers": get_typed_mapping_section(mcp_cfg_raw, "servers", "mcp.json", (dict,), "对象"),
        "agent_paths": get_typed_mapping_section(mcp_cfg_raw, "agent_paths", "mcp.json", (str, dict), "字符串或对象"),
    }
    return {
        "config": config,
        "agent_defs": agent_defs,
        "tool_defs": tool_defs,
        "skill_cfg": skill_cfg,
        "mcp_cfg": mcp_cfg,
    }


def active_toml_target_agents(config: dict, agent_defs: dict, mcp_cfg: dict) -> list[str]:
    agent_paths = canonicalize_agent_mapping(mcp_cfg.get("agent_paths", {}))
    active_agents = collect_active_mcp_target_agents(config, mcp_cfg)
    names = []
    for agent_id in sorted(active_agents):
        path_config = agent_paths.get(agent_id)
        if isinstance(path_config, dict) and path_config.get("format") == "toml":
            names.append(get_agent_display_name(agent_id, agent_defs.get(agent_id, {})))
    return names


def _normalize_mcp_args(raw_args) -> tuple[str, ...] | None:
    if raw_args is None:
        return ()
    if not isinstance(raw_args, list):
        return None
    if not all(isinstance(value, str) for value in raw_args):
        return None
    return tuple(raw_args)


def _normalize_expected_mcp_env(env_dict: dict) -> dict[str, tuple[str, object]] | None:
    items: dict[str, tuple[str, object]] = {}
    for key, value in env_dict.items():
        if key == "_MANAGED_BY":
            continue
        if not isinstance(key, str):
            return None
        if isinstance(value, str) and PLACEHOLDER_PATTERN.fullmatch(value):
            items[key] = ("placeholder", None)
        else:
            items[key] = ("literal", value)
    return items


def _normalize_actual_mcp_env(env_dict: dict) -> dict[str, tuple[str, object]] | None:
    items: dict[str, tuple[str, object]] = {}
    for key, value in env_dict.items():
        if key == "_MANAGED_BY":
            continue
        if not isinstance(key, str):
            return None
        if isinstance(value, str) and PLACEHOLDER_PATTERN.fullmatch(value):
            items[key] = ("placeholder", None)
        else:
            items[key] = ("literal", value)
    return items


def _managed_mcp_matches(expected: dict[str, object], actual: dict[str, object]) -> bool:
    if expected.get("command") != actual.get("command"):
        return False
    if expected.get("args") != actual.get("args"):
        return False

    expected_env = expected.get("env")
    actual_env = actual.get("env")
    if not isinstance(expected_env, dict) or not isinstance(actual_env, dict):
        return expected_env == actual_env
    if set(expected_env) != set(actual_env):
        return False

    for key, (expected_kind, expected_value) in expected_env.items():
        actual_kind, actual_value = actual_env[key]
        if expected_kind == "placeholder":
            if actual_kind != "literal":
                return False
            continue
        if actual_kind != "literal" or actual_value != expected_value:
            return False
    return True


def _normalize_expected_mcp_definition(
    settings: dict,
    *,
    root_dir: Path | None = None,
    server_name: str | None = None,
) -> dict[str, object]:
    env_dict = settings.get("env", {})
    if not isinstance(env_dict, dict):
        env_dict = {}
    args = _normalize_mcp_args(settings.get("args"))
    if args is not None and root_dir is not None and server_name and settings.get("url"):
        repo_path = str((root_dir / "agent-mcp" / server_name).resolve())
        args = tuple(arg.replace("{repo_path}", repo_path) for arg in args)
    return {
        "command": settings.get("command") if isinstance(settings.get("command"), str) else None,
        "args": args,
        "env": _normalize_expected_mcp_env(env_dict),
    }


def _normalize_actual_mcp_definition(settings: dict, agent_id: str) -> dict[str, object] | None:
    env_dict = settings.get("env")
    if not isinstance(env_dict, dict):
        env_dict = settings.get("environment")
    if not isinstance(env_dict, dict) or env_dict.get("_MANAGED_BY") != "agent-manager":
        return None

    command = settings.get("command") if isinstance(settings.get("command"), str) else None
    args = _normalize_mcp_args(settings.get("args"))

    return {
        "command": command,
        "args": args,
        "env": _normalize_actual_mcp_env(env_dict),
    }


def expected_managed_mcp_servers(
    root_dir: Path,
    config: dict,
    mcp_cfg: dict,
    agent_id: str,
) -> dict[str, dict[str, object]]:
    expected = {}
    servers = mcp_cfg.get("servers", {})
    for server_name, policy in config.get("mcp", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = servers.get(server_name)
        if not definition:
            continue
        merged = dict(definition)
        merged.update(policy)
        targeted_agents = canonicalize_agent_list(merged.get("agents"))
        if targeted_agents is None or agent_id in targeted_agents:
            expected[server_name] = _normalize_expected_mcp_definition(
                merged,
                root_dir=root_dir,
                server_name=server_name,
            )
    return expected


def managed_mcp_servers(target_path: str, file_format: str, agent_id: str) -> dict[str, dict[str, object]]:
    def servers_from_mapping(server_mapping: dict) -> dict[str, dict[str, object]]:
        servers: dict[str, dict[str, object]] = {}
        for server_name, settings in server_mapping.items():
            if not isinstance(server_name, str) or not isinstance(settings, dict):
                continue
            normalized = _normalize_actual_mcp_definition(settings, agent_id)
            if normalized is not None:
                servers[server_name] = normalized
        return servers

    try:
        with open(target_path, encoding="utf-8") as handle:
            raw = handle.read()
    except OSError as exc:
        raise ValueError(str(exc)) from exc

    if file_format == "toml":
        parser_module = load_tomllib()
        try:
            parsed = parser_module.loads(raw)
        except Exception as exc:
            raise ValueError(str(exc)) from exc
        server_mapping = parsed.get("mcp_servers", {})
        if not isinstance(server_mapping, dict):
            return {}
        return servers_from_mapping(server_mapping)

    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(str(exc)) from exc
    if not isinstance(parsed, dict):
        raise ValueError("顶层必须是对象")
    server_mapping = parsed.get("mcpServers", {})
    if server_mapping is None:
        return {}
    if not isinstance(server_mapping, dict):
        return {}
    return servers_from_mapping(server_mapping)


def managed_mcp_server_names(target_path: str, file_format: str, agent_id: str) -> set[str]:
    return set(managed_mcp_servers(target_path, file_format, agent_id))


def count_managed_mcp_servers(target_path: str, file_format: str, agent_id: str) -> int:
    return len(managed_mcp_server_names(target_path, file_format, agent_id))


def read_skill_name(skill_md_path: Path) -> str | None:
    try:
        content = skill_md_path.read_text(encoding="utf-8")
    except OSError:
        return None
    lines = content.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    for line in lines[1:]:
        stripped = line.strip()
        if stripped == "---":
            break
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.strip() == "name":
            skill_name = value.strip().strip("'\"")
            if skill_name:
                return skill_name
            break
    return None


def resolve_skill_search_paths(repo_path: Path, config: dict) -> list[Path]:
    raw_paths = config.get("skill_paths")
    if raw_paths is None:
        return [repo_path]
    if not isinstance(raw_paths, list) or not raw_paths:
        raise ValueError("skill_paths 必须是非空数组")

    search_paths = []
    for raw_path in raw_paths:
        if not isinstance(raw_path, str) or not raw_path.strip():
            raise ValueError("skill_paths 中的每一项都必须是非空字符串")
        search_path = (repo_path / raw_path).resolve(strict=False)
        if search_path != repo_path and repo_path not in search_path.parents:
            raise ValueError(f"skill_paths 包含越界路径: {raw_path}")
        if not search_path.exists():
            raise ValueError(f"skill_paths 指向的目录不存在: {raw_path}")
        if not search_path.is_dir():
            raise ValueError(f"skill_paths 指向的路径不是目录: {raw_path}")
        search_paths.append(search_path)
    return search_paths


def expected_managed_skill_targets(root_dir: Path, config: dict, skill_cfg: dict, agent_id: str) -> tuple[dict[str, str], list[str]]:
    expected: dict[str, str] = {}
    problems: list[str] = []
    repo_base = root_dir / "agent-skill" / "repositories"
    repositories = skill_cfg.get("repositories", {})
    excluded_dirs = {".git", "node_modules", "__pycache__", "venv", ".venv"}

    for repo_name, policy in config.get("skills", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = repositories.get(repo_name)
        if not definition:
            continue
        merged = dict(definition)
        merged.update(policy)
        targeted_agents = canonicalize_agent_list(merged.get("agents"))
        if targeted_agents is not None and agent_id not in targeted_agents:
            continue

        repo_path = (repo_base / repo_name).resolve()
        if not repo_path.exists():
            problems.append(f"{repo_name}: 仓库缓存缺失")
            continue
        try:
            search_paths = resolve_skill_search_paths(repo_path, merged)
        except ValueError as exc:
            problems.append(f"{repo_name}: {exc}")
            continue

        repo_targets: dict[str, str] = {}
        duplicate_names: set[str] = set()
        for search_path in search_paths:
            for walk_root, dirs, files in os.walk(search_path):
                dirs[:] = sorted(d for d in dirs if d not in excluded_dirs)
                if "SKILL.md" not in files:
                    continue
                skill_dir = Path(walk_root).resolve()
                skill_name = read_skill_name(skill_dir / "SKILL.md")
                if not skill_name:
                    continue
                rendered_path = str(skill_dir)
                existing = repo_targets.get(skill_name)
                if existing is not None and existing != rendered_path:
                    duplicate_names.add(skill_name)
                    continue
                repo_targets[skill_name] = rendered_path

        if duplicate_names:
            problems.append(f"{repo_name}: 重复 skill name: {', '.join(sorted(duplicate_names))}")
            continue

        for skill_name, rendered_path in repo_targets.items():
            existing = expected.get(skill_name)
            if existing is not None and existing != rendered_path:
                problems.append(f"重复 skill name: {skill_name}")
                continue
            expected[skill_name] = rendered_path
    return expected, problems


def actual_skill_targets(target_path: str) -> tuple[dict[str, str], set[str]]:
    actual: dict[str, str] = {}
    broken: set[str] = set()
    for name in os.listdir(target_path):
        link_path = Path(target_path) / name
        if not _is_skill_link_path(link_path):
            continue
        try:
            actual[name] = str(_resolve_skill_link_target(link_path))
            if not link_path.exists():
                broken.add(name)
        except OSError:
            actual[name] = "<broken>"
            broken.add(name)
    return actual, broken


def _is_windows_directory_link(path: Path) -> bool:
    checker = getattr(os.path, "isjunction", None)
    if checker is not None:
        try:
            if checker(path):
                return True
        except OSError:
            return False
    try:
        stat_result = os.lstat(path)
    except OSError:
        return False
    file_attrs = getattr(stat_result, "st_file_attributes", 0)
    reparse_flag = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
    directory_flag = getattr(stat, "FILE_ATTRIBUTE_DIRECTORY", 0)
    return bool(reparse_flag and directory_flag and file_attrs & reparse_flag and file_attrs & directory_flag)


def _is_skill_link_path(path: Path) -> bool:
    return path.is_symlink() or _is_windows_directory_link(path)


def _normalize_windows_device_path(path: str | os.PathLike[str]) -> str:
    path_str = os.fspath(path)
    prefix_mappings = (
        ("\\\\?\\UNC\\", "\\\\"),
        ("\\??\\UNC\\", "\\\\"),
        ("\\\\?\\", ""),
        ("\\??\\", ""),
    )
    for prefix, replacement in prefix_mappings:
        if path_str.startswith(prefix):
            return replacement + path_str[len(prefix):]
    return path_str


def _normalize_comparable_path(path: str | os.PathLike[str]) -> tuple[str, str]:
    path_str = _normalize_windows_device_path(path)
    if ntpath.isabs(path_str):
        return "windows", ntpath.normcase(ntpath.normpath(path_str))
    return "posix", str(Path(path_str).expanduser().resolve(strict=False))


def _paths_match(left: str | os.PathLike[str] | None, right: str | os.PathLike[str] | None) -> bool:
    if left is None or right is None:
        return left == right
    left_kind, left_path = _normalize_comparable_path(left)
    right_kind, right_path = _normalize_comparable_path(right)
    if left_kind != right_kind:
        return False
    return left_path == right_path


def _path_within_base(path: str | os.PathLike[str], base_path: str | os.PathLike[str]) -> bool:
    path_kind, normalized_path = _normalize_comparable_path(path)
    base_kind, normalized_base = _normalize_comparable_path(base_path)
    if path_kind != base_kind:
        return False
    commonpath_func = ntpath.commonpath if path_kind == "windows" else os.path.commonpath
    try:
        common_path = commonpath_func([normalized_path, normalized_base])
    except ValueError:
        return False
    return common_path == normalized_base


def _resolve_skill_link_target(path: Path) -> str:
    if not _is_skill_link_path(path):
        raise OSError(f"Not a link entry: {path}")
    try:
        raw_target = _normalize_windows_device_path(os.readlink(path))
        if ntpath.isabs(raw_target):
            return ntpath.normpath(raw_target)
        raw_target_path = Path(raw_target).expanduser()
        resolved_target = raw_target_path if raw_target_path.is_absolute() else (path.parent / raw_target_path)
        return str(resolved_target.resolve(strict=False))
    except OSError:
        if _is_windows_directory_link(path):
            return str(path.resolve(strict=False))
        raise


def is_managed_skill_target_path(target_path: str, managed_repo_base: Path) -> bool:
    if target_path == "<broken>":
        return False
    return _path_within_base(target_path, managed_repo_base)


def format_name_set(names: set[str]) -> str:
    return ", ".join(sorted(names)) if names else "无"


def collect_managed_target_paths(root_dir: Path, *, include_skills: bool) -> list[str]:
    targets: set[str] = set()

    def add_target(value):
        if isinstance(value, dict):
            value = value.get("path")
        if not value:
            return
        expanded = str(value).replace("{{HOME}}", str(Path.home())).replace("{{REPO_ROOT}}", str(root_dir))
        targets.add(str(Path(expanded).expanduser()))

    mcp_cfg = load_json_object_file(root_dir / "config" / "mcp.json", strict=True, missing_ok=False)
    mcp_agent_paths = get_typed_mapping_section(mcp_cfg, "agent_paths", "mcp.json", (str, dict), "字符串或对象")
    for value in mcp_agent_paths.values():
        add_target(value)

    if include_skills:
        skills_cfg = load_json_object_file(root_dir / "config" / "skills.json", strict=True, missing_ok=False)
        skill_agents = get_typed_mapping_section(skills_cfg, "agents", "skills.json", (str, dict), "字符串或对象")
        for value in skill_agents.values():
            add_target(value)

    manifest_path = root_dir / "config" / "rules-targets.list"
    if not manifest_path.exists():
        raise ValueError("rules-targets.list 缺失")
    for raw_line in manifest_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        parts = [part.strip() for part in line.split("|")]
        if len(parts) < 4:
            raise ValueError(f"rules-targets.list 非法条目: {raw_line}")
        add_target(parts[3])

    return sorted(targets)


def list_managed_backup_entries(root_dir: Path) -> list[str]:
    entries: dict[str, float] = {}
    for target in collect_managed_target_paths(root_dir, include_skills=False):
        target_dir = os.path.dirname(target)
        target_name = os.path.basename(target)
        try:
            names = os.listdir(target_dir)
        except OSError:
            continue
        for name in names:
            if name != f"{target_name}.bak" and not name.startswith(f"{target_name}.bak."):
                continue
            path = os.path.join(target_dir, name)
            try:
                mtime = os.path.getmtime(path)
            except OSError:
                continue
            entries[path] = max(entries.get(path, float("-inf")), mtime)
    return [path for path, _mtime in sorted(entries.items(), key=lambda item: (-item[1], item[0]))]


def build_report(*, root_dir: Path | str, config_file: Path | str, python_cmd: str = "python3") -> dict:
    root_dir = Path(root_dir)
    config_file = Path(config_file)
    report = {
        "status": "healthy",
        "version": git_version_info(root_dir),
        "sections": {
            "config": [],
            "security": [],
            "runtime": [],
            "storage": [],
            "agents": [],
            "tools": [],
            "mcp": [],
            "skills": [],
            "rules": [],
            "backups": [],
        },
        "summary": {
            "errors": 0,
            "warnings": 0,
            "warning_categories": {
                "optional_tools": 0,
                "unmaterialized": 0,
                "official_gaps": 0,
            },
        },
    }

    for rel_path in REQUIRED_CONFIG_FILES:
        abs_path = root_dir / rel_path
        name = abs_path.name
        if not abs_path.exists():
            append_entry(report, "config", make_entry(name, "error", "缺失"))
            continue
        if abs_path.suffix != ".json":
            append_entry(report, "config", make_entry(name, "ok", "存在"))
            continue
        try:
            data = load_json_object_file(abs_path, strict=True, missing_ok=False)
        except ValueError as exc:
            append_entry(report, "config", make_entry(name, "error", f"语法错误: {exc}"))
            continue
        try:
            validate_json_shape(data, name)
        except ValueError as exc:
            append_entry(report, "config", make_entry(name, "error", f"格式正确，但结构非法: {exc}"))
            continue
        content = abs_path.read_text(encoding="utf-8")
        if scan_text_for_secrets(content, file_label=name):
            append_entry(report, "config", make_entry(name, "error", "格式正确，但包含明文密钥风险"))
        else:
            append_entry(report, "config", make_entry(name, "ok", "格式正确"))

    repo_secret_findings = scan_repo_for_secrets(root_dir)
    if repo_secret_findings:
        grouped_findings: dict[str, list[tuple[int, str]]] = {}
        for finding in repo_secret_findings:
            grouped_findings.setdefault(finding.path, []).append((finding.line, finding.kind))
        for path, items in sorted(grouped_findings.items()):
            sample = ", ".join(f"{kind}@L{line}" for line, kind in items[:3])
            if len(items) > 3:
                sample = f"{sample} 等 {len(items)} 处"
            append_entry(report, "security", make_entry(path, "error", f"疑似明文敏感信息: {sample}"))
    else:
        append_entry(report, "security", make_entry("tracked repo files", "ok", "未发现疑似明文敏感信息"))

    if report["summary"]["errors"] > 0:
        report["status"] = "error"
        return report

    inputs = load_inputs(root_dir, config_file)
    config = inputs["config"]
    agent_defs = inputs["agent_defs"]
    tool_defs = inputs["tool_defs"]
    skill_cfg = inputs["skill_cfg"]
    mcp_cfg = inputs["mcp_cfg"]

    runtime_commands = [python_cmd]
    runtime_commands.extend(
        collect_required_runtime_commands(config, agent_defs, tool_defs, skill_cfg, mcp_cfg, repo_root=root_dir)
    )
    seen = set()
    for command in runtime_commands:
        if not command or command in seen:
            continue
        seen.add(command)
        if command_exists(command):
            append_entry(report, "runtime", make_entry(command, "ok", command_version(command)))
        else:
            append_entry(report, "runtime", make_entry(command, "error", "缺失"))

    toml_agents = active_toml_target_agents(config, agent_defs, mcp_cfg)
    if toml_agents:
        try:
            parser_module = load_tomllib()
        except ModuleNotFoundError as exc:
            append_entry(
                report,
                "runtime",
                make_entry(
                    "TOML parser",
                    "error",
                    f"缺失；当前策略启用了 {', '.join(toml_agents)} 的 TOML 目标，请安装 tomli 或使用 Python 3.11+ ({exc})",
                ),
            )
        else:
            append_entry(
                report,
                "runtime",
                make_entry("TOML parser", "ok", f"{getattr(parser_module, '__name__', 'tomllib')}"),
            )

    free_space = shutil.disk_usage(root_dir).free
    append_entry(report, "storage", make_entry("磁盘可用", "ok", human_size(free_space)))

    if report["summary"]["errors"] > 0:
        report["status"] = "error"
        return report

    inventory = ProviderInventory()
    active_skill_agents = collect_active_skill_target_agents(config, skill_cfg)
    active_mcp_agents = collect_active_mcp_target_agents(config, mcp_cfg)
    active_rule_agents = collect_active_rule_agents(config, skill_cfg, mcp_cfg)

    def generated_target_status(agent_id: str) -> str:
        if is_agent_installed(agent_id, agent_defs.get(agent_id, {})):
            return "installed"
        return "uninstalled"

    for agent_id, policy in config.get("agents", {}).items():
        definition = agent_defs.get(agent_id, {})
        label = get_agent_display_name(agent_id, definition)
        providers = detect_all_providers(definition.get("bin", ""), definition, inventory=inventory)
        state = classify_provider_state(providers, definition)
        is_active = len(providers) > 0
        runtime_ready = is_agent_installed(agent_id, definition)
        desired = policy.get("enabled", True)
        preferred = policy.get("preferred_provider") or definition.get("preferred_provider", "npm")
        preferred_name = canonical_provider_name(preferred)
        preferred_cfg = definition.get("providers", {}).get(preferred_name, {})
        try:
            preferred_unsupported_actions = get_unsupported_actions(preferred_cfg)
        except ValueError as exc:
            append_entry(
                report,
                "agents",
                make_entry(label, "error", f"Provider '{preferred_name}' unsupported_actions 配置非法: {exc}"),
            )
            continue
        preferred_is_active = any(
            canonical_provider_name(provider_name) == preferred_name for provider_name in state["managed"]
        )
        if state["unsupported"]:
            append_entry(
                report,
                "agents",
                make_entry(label, "error", f"检测到未声明的 Provider: {', '.join(state['unsupported'])}"),
            )
        elif state["deprecated"]:
            append_entry(
                report,
                "agents",
                make_entry(label, "error", f"检测到已弃用安装来源: {', '.join(state['deprecated'])}"),
            )
        elif state["system_only"] and definition.get("require_managed_provider", False):
            append_entry(
                report,
                "agents",
                make_entry(label, "error", "检测到外部安装，未纳入 agent-manager 托管"),
            )
        elif is_active and not runtime_ready:
            detail = state["managed"][0] if state["managed"] else (providers[0] if providers else "unknown")
            append_entry(report, "agents", make_entry(label, "error", f"{detail} 已安装，但运行入口缺失"))
        elif (
            not desired
            and preferred_is_active
            and "uninstall" in preferred_unsupported_actions
        ):
            append_entry(
                report,
                "agents",
                make_entry(
                    label,
                    "warning",
                    "官方当前未提供受管 uninstall 动作，agent-manager 不会伪造删除命令；请先按官方流程自行卸载",
                    category="official_gap",
                ),
            )
        elif is_active == desired:
            detail = state["managed"][0] if state["managed"] else (providers[0] if providers else "未安装")
            if state["managed"] and canonical_provider_name(state["managed"][0]) != canonical_provider_name(preferred):
                detail = f"{detail} (偏离偏好: {preferred})"
            append_entry(report, "agents", make_entry(label, "ok", detail))
        else:
            append_entry(
                report,
                "agents",
                make_entry(label, "error", f"漂移 (实际:{'已装' if is_active else '未装'})"),
            )
    for agent_id, definition in agent_defs.items():
        if agent_id not in config.get("agents", {}):
            append_entry(
                report,
                "agents",
                make_entry(get_agent_display_name(agent_id, definition), "notice", "未声明策略（已忽略）"),
            )

    for tool_name, policy in config.get("tools", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = tool_defs.get(tool_name)
        if not definition:
            append_entry(report, "tools", make_entry(tool_name, "error", "缺少定义"))
            continue
        severity = definition.get("doctor_severity", "error")
        if detect_tool_readiness(tool_name, definition, root_dir, inventory=inventory):
            append_entry(report, "tools", make_entry(tool_name, "ok", "已就绪"))
            continue
        if severity == "warning":
            append_entry(
                report,
                "tools",
                make_entry(tool_name, "warning", "可选项未就绪", category="optional_tool"),
            )
        else:
            append_entry(report, "tools", make_entry(tool_name, "error", "未就绪"))

    agent_paths = canonicalize_agent_mapping(mcp_cfg.get("agent_paths", {}))
    for agent_id, path_config in agent_paths.items():
        if agent_id not in active_mcp_agents:
            continue
        target_path = path_config if isinstance(path_config, str) else path_config.get("path", "")
        file_format = "json" if isinstance(path_config, str) else path_config.get("format", "json")
        target_path = os.path.expanduser(target_path)
        label = get_agent_display_name(agent_id, agent_defs.get(agent_id, {}))
        agent_policy = config.get("agents", {}).get(agent_id, {})
        agent_enabled = agent_policy.get("enabled", True)
        expected_servers = expected_managed_mcp_servers(root_dir, config, mcp_cfg, agent_id)
        expected_names = set(expected_servers)
        expected_count = len(expected_names)
        if os.path.exists(target_path):
            try:
                actual_servers = managed_mcp_servers(target_path, file_format, agent_id)
            except ValueError as exc:
                append_entry(report, "mcp", make_entry(label, "error", f"MCP 配置解析失败: {exc}"))
                continue
            actual_names = set(actual_servers)
            actual_count = len(actual_names)
            if not agent_enabled:
                if actual_names:
                    append_entry(
                        report,
                        "mcp",
                        make_entry(
                            label,
                            "error",
                            "策略已禁用，但仍残留托管服务 "
                            f"({actual_count} 个: {format_name_set(actual_names)})",
                        ),
                    )
                else:
                    append_entry(report, "mcp", make_entry(label, "ok", "策略已禁用"))
                continue
            if actual_names != expected_names:
                append_entry(
                    report,
                    "mcp",
                    make_entry(
                        label,
                        "error",
                        "托管服务集合漂移 "
                        f"(期望 {expected_count} 个托管服务: {format_name_set(expected_names)}，"
                        f"实际 {actual_count} 个托管服务: {format_name_set(actual_names)})",
                    ),
                )
                continue

            drift_names = {
                name
                for name in expected_names
                if not _managed_mcp_matches(expected_servers[name], actual_servers.get(name, {}))
            }
            if not drift_names:
                append_entry(report, "mcp", make_entry(label, "ok", f"已注入 {actual_count} 个托管服务"))
            else:
                append_entry(
                    report,
                    "mcp",
                    make_entry(
                        label,
                        "error",
                        f"托管服务配置漂移 ({format_name_set(drift_names)})",
                    ),
                )
        else:
            if not agent_enabled:
                append_entry(report, "mcp", make_entry(label, "ok", "策略已禁用"))
            elif generated_target_status(agent_id) == "installed":
                message = "尚未生成 MCP 配置"
                append_entry(report, "mcp", make_entry(label, "warning", message, category="unmaterialized"))
            else:
                message = "Agent 未安装，未生成 MCP 配置"
                append_entry(report, "mcp", make_entry(label, "warning", message, category="unmaterialized"))

    for agent_id, agent_path in canonicalize_agent_mapping(skill_cfg.get("agents", {})).items():
        if agent_id not in active_skill_agents:
            continue
        target_path = os.path.expanduser(agent_path)
        label = get_agent_display_name(agent_id, agent_defs.get(agent_id, {}))
        managed_repo_base = (root_dir / "agent-skill" / "repositories").resolve()
        expected_targets, skill_target_problems = expected_managed_skill_targets(root_dir, config, skill_cfg, agent_id)
        if os.path.isdir(target_path):
            if skill_target_problems:
                append_entry(
                    report,
                    "skills",
                    make_entry(label, "error", f"技能托管目标解析失败: {'; '.join(skill_target_problems)}"),
                )
                continue
            actual_targets, broken_names = actual_skill_targets(target_path)
            expected_names = set(expected_targets)
            actual_names = set(actual_targets)
            missing_names = expected_names - actual_names
            drift_names = {
                name for name in expected_names if not _paths_match(actual_targets.get(name), expected_targets.get(name))
            }
            extra_managed_names = {
                name
                for name, actual_target in actual_targets.items()
                if name not in expected_names and is_managed_skill_target_path(actual_target, managed_repo_base)
            }
            if broken_names:
                append_entry(
                    report,
                    "skills",
                    make_entry(
                        label,
                        "error",
                        f"存在死链 skill 链接 ({format_name_set(broken_names)})",
                    ),
                )
            elif not missing_names and not drift_names and not extra_managed_names:
                append_entry(report, "skills", make_entry(label, "ok", f"{len(expected_names)} 个受管链接"))
            elif missing_names:
                append_entry(
                    report,
                    "skills",
                    make_entry(
                        label,
                        "error",
                        f"skill 链接缺失 ({format_name_set(missing_names)})",
                    ),
                )
            elif drift_names:
                append_entry(
                    report,
                    "skills",
                    make_entry(
                        label,
                        "error",
                        f"skill 目标漂移 ({format_name_set(drift_names)})",
                    ),
                )
            else:
                append_entry(
                    report,
                    "skills",
                    make_entry(
                        label,
                        "error",
                        f"存在额外受管 skill 链接 ({format_name_set(extra_managed_names)})",
                    ),
                )
        else:
            if generated_target_status(agent_id) == "installed":
                message = "尚未建立 skills 目录"
            else:
                message = "Agent 未安装，未建立 skills 目录"
            append_entry(report, "skills", make_entry(label, "warning", message, category="unmaterialized"))

    manifest_path = root_dir / "config" / "rules-targets.list"
    if manifest_path.exists():
        for raw_line in manifest_path.read_text(encoding="utf-8").splitlines():
            stripped = raw_line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            parts = [part.strip() for part in stripped.split("|")]
            if len(parts) < 4:
                append_entry(report, "rules", make_entry(stripped, "error", "manifest 非法"))
                continue
            agent_id = rule_target_agent_id(parts[0])
            src = os.path.join(root_dir, "agent-rule", parts[2])
            is_repo_root_target = "{{REPO_ROOT}}" in parts[3]
            if agent_id not in active_rule_agents and not is_repo_root_target:
                continue
            dst = (
                parts[3]
                .replace("{{HOME}}", os.path.expanduser("~"))
                .replace("{{REPO_ROOT}}", str(root_dir))
            )
            dst = os.path.expanduser(dst)
            if os.path.exists(dst):
                if rules_match(src, dst):
                    append_entry(report, "rules", make_entry(parts[0], "ok", "一致"))
                else:
                    append_entry(report, "rules", make_entry(parts[0], "error", "漂移"))
            else:
                if is_repo_root_target or generated_target_status(
                    RULE_TARGET_PREFIX_TO_ID.get(parts[0].split("_", 1)[0], agent_id)
                ) == "installed":
                    message = "尚未分发"
                else:
                    message = "Agent 未安装，未分发"
                append_entry(report, "rules", make_entry(parts[0], "warning", message, category="unmaterialized"))

    try:
        backup_count = len(list_managed_backup_entries(root_dir))
        append_entry(report, "backups", make_entry("有效历史快照", "ok", f"{backup_count} 个"))
    except ValueError as exc:
        append_entry(report, "backups", make_entry("有效历史快照", "error", str(exc)))

    if report["summary"]["errors"] > 0:
        report["status"] = "error"
    elif report["summary"]["warnings"] > 0:
        report["status"] = "warning"
    else:
        report["status"] = "healthy"
    return report


def render_text(report: dict) -> str:
    lines: list[str] = ["Agent Manager 深度诊断 (Doctor Check)", ""]
    version = report.get("version")
    if version:
        lines.append(f"[ 管理系统版本 ]  {version['commit']} ({version['date']})")
        lines.append("")

    lines.append("[ 配置与安全 ]")
    for entry in report["sections"]["config"]:
        symbol = {"ok": "✓", "warning": "⚠", "notice": "⚠", "error": "✗"}[entry["status"]]
        lines.append(f"  {symbol} {entry['name']}: {entry['message']}")

    if report["sections"]["security"]:
        lines.append("")
        lines.append("[ 主仓库敏感信息扫描 ]")
        for entry in report["sections"]["security"]:
            symbol = {"ok": "✓", "warning": "⚠", "notice": "⚠", "error": "✗"}[entry["status"]]
            lines.append(f"  {symbol} {entry['name']}: {entry['message']}")

    if report["summary"]["errors"] == 0 or report["sections"]["runtime"]:
        lines.append("")
        lines.append("[ 运行环境 ]")
        for entry in report["sections"]["runtime"]:
            symbol = {"ok": "✓", "warning": "⚠", "notice": "⚠", "error": "✗"}[entry["status"]]
            lines.append(f"  {symbol} {entry['name']}: {entry['message']}")

        lines.append("")
        lines.append("[ 存储空间 ]")
        for entry in report["sections"]["storage"]:
            lines.append(f"  ✓ {entry['name']}: {entry['message']}")

    if report["sections"]["agents"] or report["sections"]["tools"] or report["sections"]["mcp"] or report["sections"]["skills"] or report["sections"]["rules"]:
        lines.append("")
        lines.append("[ 路径、内容与链接一致性审计 ]")
        audit_sections = (
            ("agents", "--- Agent 状态与策略一致性 ---"),
            ("tools", "--- Tools 就绪审计 ---"),
            ("mcp", "--- MCP 注入审计 ---"),
            ("skills", "--- Skills 链接审计 ---"),
            ("rules", "--- Rules 内容/结构审计 ---"),
        )
        for section_name, title in audit_sections:
            entries = report["sections"][section_name]
            if not entries:
                continue
            lines.append(title)
            width = {
                "agents": 20,
                "tools": 20,
                "mcp": 0,
                "skills": 15,
                "rules": 0,
            }[section_name]
            for entry in entries:
                symbol = {"ok": "✓", "warning": "⚠", "notice": "⚠", "error": "✗"}[entry["status"]]
                label = entry["name"] if width == 0 else f"{entry['name']:<{width}}"
                lines.append(f"  {symbol} {label}: {entry['message']}")
            lines.append("")
        while lines and lines[-1] == "":
            lines.pop()

    if report["sections"]["backups"]:
        lines.append("")
        lines.append("[ 容灾快照统计 ]")
        for entry in report["sections"]["backups"]:
            symbol = {"ok": "✓", "warning": "⚠", "notice": "⚠", "error": "✗"}[entry["status"]]
            lines.append(f"  {symbol} {entry['name']}: {entry['message']}")

    categories = report["summary"]["warning_categories"]
    if report["summary"]["warnings"] > 0 and (
        categories["optional_tools"] > 0
        or categories["unmaterialized"] > 0
        or categories["official_gaps"] > 0
    ):
        lines.append("")
        lines.append("[ 提示项分类 ]")
        if categories["optional_tools"] > 0:
            lines.append(f"  ⚠ 可选工具未就绪: {categories['optional_tools']} 项")
        if categories["unmaterialized"] > 0:
            lines.append(f"  ⚠ 托管产物尚未生成: {categories['unmaterialized']} 项")
        if categories["official_gaps"] > 0:
            lines.append(f"  ⚠ 官方生命周期动作缺口: {categories['official_gaps']} 项")

    lines.append("")
    if report["status"] == "healthy":
        lines.append("✨ 认证通过：环境状态完美。")
    elif report["status"] == "warning":
        lines.append(f"⚠ 认证完成：发现 {report['summary']['warnings']} 个提示项。")
    else:
        lines.append(f"❌ 认证失败：发现 {report['summary']['errors']} 个问题。")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Agent Manager doctor report generator")
    parser.add_argument("--root-dir", required=True)
    parser.add_argument("--config-file", required=True)
    parser.add_argument("--python-cmd", default="python3")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    args = parser.parse_args()

    report = build_report(root_dir=args.root_dir, config_file=args.config_file, python_cmd=args.python_cmd)
    if args.format == "json":
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print(render_text(report), end="")

    return 0 if report["status"] != "error" else 1


if __name__ == "__main__":
    raise SystemExit(main())
