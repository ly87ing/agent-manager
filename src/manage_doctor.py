from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.json_config import ensure_mapping_value_types, get_typed_mapping_section, load_json_object_file
from src.manage_runtime_probe import (
    ProviderInventory,
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
from src.toml_compat import load_tomllib


SECRET_PATTERN = re.compile(
    r"(sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]{20,}|AIza[a-zA-Z0-9_\-]{20,})"
)
RULE_TARGET_PREFIX_TO_ID = {
    "claude": "claude-code",
    "gemini": "gemini-cli",
    "cursor": "cursor",
    "copilot": "github-copilot",
    "opencode": "opencode",
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


def validate_json_shape(data: dict, name: str) -> None:
    if name == "manager.json":
        get_typed_mapping_section(data, "agents", name, (dict,), "对象")
        get_typed_mapping_section(data, "tools", name, (dict,), "对象")
        get_typed_mapping_section(data, "skills", name, (dict,), "对象")
        get_typed_mapping_section(data, "mcp", name, (dict,), "对象")
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
    agent_defs = canonicalize_agent_mapping(
        ensure_mapping_value_types(
            load_json(root_dir / "config" / "agents.json", strict=True, missing_ok=False),
            "agents.json",
            (dict,),
            "对象",
        )
    )
    tool_defs = get_typed_mapping_section(
        load_json(root_dir / "config" / "tools.json", strict=True, missing_ok=False),
        "tools",
        "tools.json",
        (dict,),
        "对象",
    )
    skill_cfg_raw = load_json(root_dir / "config" / "skills.json", strict=True, missing_ok=False)
    skill_cfg = {
        "repositories": get_typed_mapping_section(skill_cfg_raw, "repositories", "skills.json", (dict,), "对象"),
        "agents": get_typed_mapping_section(skill_cfg_raw, "agents", "skills.json", (str, dict), "字符串或对象"),
    }
    mcp_cfg_raw = load_json(root_dir / "config" / "mcp.json", strict=True, missing_ok=False)
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


def collect_managed_target_paths(root_dir: Path, *, include_skills: bool) -> list[str]:
    targets: set[str] = set()

    def add_target(value):
        if isinstance(value, dict):
            value = value.get("path")
        if not value:
            return
        expanded = str(value).replace("{{HOME}}", str(Path.home()))
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
            validate_json_shape(data, name)
        except ValueError as exc:
            append_entry(report, "config", make_entry(name, "error", str(exc)))
            continue
        content = abs_path.read_text(encoding="utf-8")
        if SECRET_PATTERN.search(content):
            append_entry(report, "config", make_entry(name, "error", "格式正确，但包含明文密钥风险"))
        else:
            append_entry(report, "config", make_entry(name, "ok", "格式正确"))

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
        is_active = len(providers) > 0
        desired = policy.get("enabled", True)
        preferred = policy.get("preferred_provider") or definition.get("preferred_provider", "npm")
        if is_active == desired:
            detail = providers[0] if is_active else "未安装"
            if is_active and providers[0].split("-")[0] != preferred.split("-")[0]:
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
        target_path = os.path.expanduser(target_path)
        label = get_agent_display_name(agent_id, agent_defs.get(agent_id, {}))
        if os.path.exists(target_path):
            with open(target_path, encoding="utf-8") as handle:
                count = len(re.findall(r"_MANAGED_BY", handle.read()))
            append_entry(report, "mcp", make_entry(label, "ok", f"已注入 {count} 个托管服务"))
        else:
            if generated_target_status(agent_id) == "installed":
                message = "尚未生成 MCP 配置"
            else:
                message = "Agent 未安装，未生成 MCP 配置"
            append_entry(report, "mcp", make_entry(label, "warning", message, category="unmaterialized"))

    for agent_id, agent_path in canonicalize_agent_mapping(skill_cfg.get("agents", {})).items():
        if agent_id not in active_skill_agents:
            continue
        target_path = os.path.expanduser(agent_path)
        label = get_agent_display_name(agent_id, agent_defs.get(agent_id, {}))
        if os.path.isdir(target_path):
            links = [name for name in os.listdir(target_path) if os.path.islink(os.path.join(target_path, name))]
            append_entry(report, "skills", make_entry(label, "ok", f"{len(links)} 个物理链接"))
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
            if agent_id not in active_rule_agents:
                continue
            src = os.path.join(root_dir, "agent-rule", parts[2])
            dst = os.path.expanduser(parts[3].replace("{{HOME}}", os.path.expanduser("~")))
            if os.path.exists(dst):
                if rules_match(src, dst):
                    append_entry(report, "rules", make_entry(parts[0], "ok", "一致"))
                else:
                    append_entry(report, "rules", make_entry(parts[0], "error", "漂移"))
            else:
                if generated_target_status(RULE_TARGET_PREFIX_TO_ID.get(parts[0].split("_", 1)[0], agent_id)) == "installed":
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
    if report["summary"]["warnings"] > 0 and (categories["optional_tools"] > 0 or categories["unmaterialized"] > 0):
        lines.append("")
        lines.append("[ 提示项分类 ]")
        if categories["optional_tools"] > 0:
            lines.append(f"  ⚠ 可选工具未就绪: {categories['optional_tools']} 项")
        if categories["unmaterialized"] > 0:
            lines.append(f"  ⚠ 托管产物尚未生成: {categories['unmaterialized']} 项")

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
