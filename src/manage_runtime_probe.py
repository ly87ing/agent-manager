from __future__ import annotations

import glob
import os
import re
import shlex
import shutil
import subprocess

from src.agent_ids import canonicalize_agent_id, canonicalize_agent_list, canonicalize_agent_mapping
from src.json_config import load_json_object_file


def load_json(path, *, strict=False, missing_ok=True):
    return load_json_object_file(path, strict=strict, missing_ok=missing_ok)


def default_run_cmd(cmd, timeout=10):
    try:
        result = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
            shell=False,
            timeout=timeout,
        )
        return result.returncode == 0, result.stdout
    except Exception:
        return False, ""


def get_pkg_name(cmd):
    if not cmd:
        return None
    match = re.search(r"(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)", cmd)
    return match.group(1) if match else None


def get_uv_tool_name(cmd: str | None) -> str | None:
    if not cmd:
        return None
    try:
        tokens = shlex.split(cmd)
    except ValueError:
        tokens = cmd.split()
    if len(tokens) < 4:
        return None
    if tokens[0] != "uv":
        return None
    if tokens[1:3] != ["tool", "install"]:
        return None
    # 只支持极少量明确的 flags 解析（fail-close），避免误把 flag value 当成 package。
    positional = []
    index = 3
    while index < len(tokens):
        token = tokens[index]
        if not token:
            index += 1
            continue

        if token.startswith("-"):
            if token == "--python":
                index += 2
                continue
            if token.startswith("--python="):
                index += 1
                continue
            return None

        positional.append(token)
        index += 1

    if len(positional) != 1:
        return None
    return positional[0]


def normalize_path_for_match(path):
    if not path:
        return ""
    return path.replace("\\", "/")


def path_looks_brew_managed(path):
    normalized = normalize_path_for_match(path)
    return "/Cellar/" in normalized or "/Caskroom/" in normalized


def path_indicates_npm(path):
    normalized = normalize_path_for_match(path)
    return bool(
        normalized
        and (
            "/node/" in normalized
            or "/npm/" in normalized
            or "/node_modules/" in normalized
            or normalized.endswith("node")
            or normalized.endswith("npm")
            or normalized.endswith("npm.cmd")
        )
    )


def detect_npm_from_paths(bin_path, resolved_bin_path="", brew_detected=False):
    if path_indicates_npm(bin_path):
        return True
    if not resolved_bin_path:
        return False
    if brew_detected and path_looks_brew_managed(resolved_bin_path):
        return False
    return path_indicates_npm(resolved_bin_path)


TOOL_READY_MARKER = ".agent-manager-ready"
TOOL_BROKEN_MARKER = ".agent-manager-broken"
RULE_TARGET_PREFIX_TO_ID = {
    "claude": "claude-code",
    "gemini": "gemini-cli",
    "cursor": "cursor",
    "opencode": "opencode",
    "codex": "codex",
}
ENV_ASSIGNMENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$")
ENV_WRAPPER_FLAGS_WITH_VALUE = {"-C", "--chdir", "-S", "--split-string", "-u", "--unset"}
ENV_WRAPPER_FLAGS = {"-i", "--ignore-environment", "-0", "--null"}
COMMAND_WRAPPER_FLAG_CHARS = {"p", "v", "V"}


def is_command_wrapper_flag(token):
    if token == "--":
        return False
    if not token.startswith("-") or token == "-":
        return False
    return all(char in COMMAND_WRAPPER_FLAG_CHARS for char in token[1:])


def split_command_segments(command):
    segments = []
    in_single = False
    in_double = False
    escaped = False
    start = 0
    index = 0

    while index < len(command):
        char = command[index]
        if escaped:
            escaped = False
            index += 1
            continue
        if char == "\\" and not in_single:
            escaped = True
            index += 1
            continue
        if char == "'" and not in_double:
            in_single = not in_single
            index += 1
            continue
        if char == '"' and not in_single:
            in_double = not in_double
            index += 1
            continue
        if not in_single and not in_double:
            if command.startswith(("&&", "||"), index):
                segments.append(command[start:index])
                index += 2
                start = index
                continue
            if char in ";|":
                segments.append(command[start:index])
                index += 1
                start = index
                continue
        index += 1

    segments.append(command[start:])
    return segments


def command_head(tokens):
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if ENV_ASSIGNMENT_RE.match(token):
            index += 1
            continue

        base = os.path.basename(token)
        if base == "env":
            index += 1
            while index < len(tokens):
                token = tokens[index]
                if token == "--":
                    index += 1
                    break
                if ENV_ASSIGNMENT_RE.match(token) or token in ENV_WRAPPER_FLAGS:
                    index += 1
                    continue
                if token in ENV_WRAPPER_FLAGS_WITH_VALUE:
                    index += 2
                    continue
                if token.startswith("--unset=") or token.startswith("--chdir="):
                    index += 1
                    continue
                break
            continue
        if base == "command":
            index += 1
            while index < len(tokens):
                token = tokens[index]
                if token == "--":
                    index += 1
                    break
                if is_command_wrapper_flag(token):
                    index += 1
                    continue
                break
            continue
        return base
    return None


def iter_command_heads(command):
    if not command:
        return []
    heads = []
    for segment in split_command_segments(command):
        segment = segment.strip()
        if not segment:
            continue
        try:
            tokens = shlex.split(segment)
        except ValueError:
            tokens = segment.split()
        head = command_head(tokens)
        if head:
            heads.append(head)
    return heads


def collect_required_runtime_commands(config, agent_defs, tool_defs, skill_cfg, mcp_cfg, repo_root=None):
    required = []

    def add(cmd):
        if cmd and cmd not in required:
            required.append(cmd)

    for agent_name, policy in config.get("agents", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = agent_defs.get(agent_name, {})
        providers = definition.get("providers", {})
        preferred = policy.get("preferred_provider") or definition.get("preferred_provider")
        candidate_providers = []
        if preferred and preferred in providers:
            candidate_providers.append(providers[preferred])
        else:
            candidate_providers.extend(providers.values())
        if policy.get("auto_install", True):
            for provider in candidate_providers:
                for head in iter_command_heads(provider.get("install")):
                    add(head)
        if policy.get("auto_update", True):
            for provider in candidate_providers:
                for head in iter_command_heads(provider.get("upgrade")):
                    add(head)

    for tool_name, policy in config.get("tools", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = tool_defs.get(tool_name, {})
        tool_ready = False
        if repo_root is not None:
            tool_ready = detect_tool_readiness(tool_name, definition, repo_root)
        needs_tool_runtime = not (tool_ready and not policy.get("auto_update", True))
        if definition.get("url") and needs_tool_runtime:
            add("git")
        if needs_tool_runtime:
            for head in iter_command_heads(definition.get("install")):
                add(head)
            ready_check = definition.get("ready_check") or ""
            tool_binary = ready_check[4:].strip() if ready_check.startswith("bin:") else None
            for head in iter_command_heads(definition.get("setup")):
                if tool_binary and head == tool_binary:
                    continue
                add(head)

    for repo_name, repo_def in skill_cfg.get("repositories", {}).items():
        policy = config.get("skills", {}).get(repo_name)
        if policy is None:
            continue
        if policy.get("enabled", True) and repo_def.get("url"):
            add("git")

    for server_name, server_def in mcp_cfg.get("servers", {}).items():
        policy = config.get("mcp", {}).get(server_name)
        if policy is None or not policy.get("enabled", True):
            continue
        if server_def.get("command") == "npx":
            add("npx")
        if server_def.get("url"):
            add("git")

    return required


def expand_targeted_agents(agent_ids, available_agent_ids):
    available = set(available_agent_ids)
    normalized_agents = canonicalize_agent_list(agent_ids)
    if normalized_agents is None:
        return available
    return {agent_id for agent_id in normalized_agents if agent_id in available}


def collect_active_skill_target_agents(config, skill_cfg):
    agent_targets = canonicalize_agent_mapping(skill_cfg.get("agents", {}))
    repositories = skill_cfg.get("repositories", {})
    active = set()
    for repo_name, policy in config.get("skills", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = repositories.get(repo_name)
        if not definition:
            continue
        merged = dict(definition)
        merged.update(policy)
        active.update(expand_targeted_agents(merged.get("agents"), agent_targets.keys()))
    return active


def collect_active_mcp_target_agents(config, mcp_cfg):
    agent_targets = canonicalize_agent_mapping(mcp_cfg.get("agent_paths", {}))
    servers = mcp_cfg.get("servers", {})
    active = set()
    for server_name, policy in config.get("mcp", {}).items():
        if not policy.get("enabled", True):
            continue
        definition = servers.get(server_name)
        if not definition:
            continue
        merged = dict(definition)
        merged.update(policy)
        active.update(expand_targeted_agents(merged.get("agents"), agent_targets.keys()))
    return active


def collect_active_rule_agents(config, skill_cfg, mcp_cfg):
    active = {
        agent_id
        for agent_id, policy in canonicalize_agent_mapping(config.get("agents", {})).items()
        if policy.get("enabled", True)
    }
    active.update(collect_active_skill_target_agents(config, skill_cfg))
    active.update(collect_active_mcp_target_agents(config, mcp_cfg))
    return active


def rule_target_agent_id(target_name):
    prefix = target_name.split("_", 1)[0]
    return RULE_TARGET_PREFIX_TO_ID.get(prefix, canonicalize_agent_id(prefix))


def repo_tool_artifacts_ready(definition, managed_repo_dir):
    ready_paths = definition.get("ready_paths") or []
    if ready_paths:
        return all(os.path.exists(os.path.join(managed_repo_dir, rel_path)) for rel_path in ready_paths)
    return not definition.get("setup")


def any_ready_checks_pass(name, ready_checks, inventory=None):
    return any(check_tool_ready(name, ready_check, inventory=inventory) for ready_check in ready_checks or [])


def any_globs_match(patterns):
    for pattern in patterns or []:
        expanded = os.path.expanduser(pattern)
        if glob.glob(expanded):
            return True
    return False


def install_tool_artifacts_ready(definition, managed_tool_dir):
    ready_paths = definition.get("ready_paths") or []
    if ready_paths:
        return all(os.path.exists(os.path.join(managed_tool_dir, rel_path)) for rel_path in ready_paths)
    runtime_ready_globs = definition.get("runtime_ready_globs_any") or []
    runtime_ready_checks = definition.get("runtime_ready_checks_any") or []
    if runtime_ready_globs or runtime_ready_checks:
        return False
    if definition.get("setup"):
        return os.path.exists(os.path.join(managed_tool_dir, TOOL_READY_MARKER))
    return True


def resolve_managed_tool_dir(root_or_base, name):
    base = os.fspath(root_or_base)
    candidates = [
        os.path.join(base, "agent-tool", name),
        os.path.join(base, name),
    ]
    for candidate in candidates:
        if os.path.exists(candidate) or os.path.islink(candidate):
            return candidate
    return candidates[0]


def detect_tool_readiness(name, definition, repo_root, inventory=None):
    managed_repo_dir = resolve_managed_tool_dir(repo_root, name)
    if definition.get("url"):
        git_dir = os.path.join(managed_repo_dir, ".git")
        if not os.path.isdir(managed_repo_dir) or not os.path.isdir(git_dir):
            return False
        if os.path.exists(os.path.join(managed_repo_dir, TOOL_BROKEN_MARKER)):
            return False
        ready_paths = definition.get("ready_paths") or []
        if repo_tool_artifacts_ready(definition, managed_repo_dir):
            return True
        if ready_paths:
            return False
        if os.path.exists(os.path.join(managed_repo_dir, TOOL_READY_MARKER)):
            return True
        return False
    ready_check = definition.get("ready_check")
    if ready_check and not check_tool_ready(name, ready_check, inventory=inventory):
        return False
    if any_ready_checks_pass(name, definition.get("runtime_ready_checks_any"), inventory=inventory):
        return True
    if any_globs_match(definition.get("runtime_ready_globs_any")):
        return True
    if definition.get("setup") or definition.get("ready_paths"):
        if os.path.exists(os.path.join(managed_repo_dir, TOOL_BROKEN_MARKER)):
            return False
        return install_tool_artifacts_ready(definition, managed_repo_dir)
    if ready_check:
        return True
    return shutil.which(name) is not None


class ProviderInventory:
    def __init__(
        self,
        run_cmd=None,
        which=None,
        path_exists=None,
        realpath=None,
    ):
        self._run_cmd = run_cmd or default_run_cmd
        self._which = which or shutil.which
        self._path_exists = path_exists or os.path.exists
        self._realpath = realpath or os.path.realpath
        self._brew_formulae = None
        self._brew_casks = None
        self._gh_extensions = None
        self._uv_tools = None

    def get_brew_formulae(self):
        if self._brew_formulae is None:
            self._brew_formulae = set()
            if self._which("brew"):
                success, out = self._run_cmd(["brew", "list", "--formula", "-1"], timeout=10)
                if success and out.strip():
                    self._brew_formulae = set(out.strip().split("\n"))
        return self._brew_formulae

    def get_brew_casks(self):
        if self._brew_casks is None:
            self._brew_casks = set()
            if self._which("brew"):
                success, out = self._run_cmd(["brew", "list", "--cask", "-1"], timeout=10)
                if success and out.strip():
                    self._brew_casks = set(out.strip().split("\n"))
        return self._brew_casks

    def get_gh_extensions(self):
        if self._gh_extensions is None:
            self._gh_extensions = ""
            if self._which("gh"):
                success, out = self._run_cmd(["gh", "extension", "list"], timeout=10)
                if success:
                    self._gh_extensions = out
        return self._gh_extensions

    def get_uv_tools(self):
        if self._uv_tools is None:
            self._uv_tools = set()
            if self._which("uv"):
                success, out = self._run_cmd(["uv", "tool", "list"], timeout=10)
                if success and out.strip():
                    tools = set()
                    for line in out.splitlines():
                        line = line.strip()
                        if not line:
                            continue
                        name = line.split()[0].strip()
                        if name:
                            tools.add(name)
                    self._uv_tools = tools
        return self._uv_tools

    def detect_all_providers(self, bin_name, config=None):
        found = []
        config = config or {}

        bin_path = self._which(bin_name)
        resolved_bin_path = self._realpath(bin_path) if bin_path else ""

        brew_pkg = get_pkg_name(config.get("providers", {}).get("brew", {}).get("install", ""))
        if brew_pkg:
            if brew_pkg in self.get_brew_formulae():
                found.append("brew")
            elif brew_pkg in self.get_brew_casks():
                found.append("brew-cask")

        if detect_npm_from_paths(
            bin_path,
            resolved_bin_path,
            brew_detected=any(provider.startswith("brew") for provider in found),
        ):
            found.append("npm")

        ext_name = config.get("ext_name")
        if ext_name and any(line.strip().startswith(ext_name) for line in self.get_gh_extensions().splitlines()):
            found.append("gh")

        uv_pkg = get_uv_tool_name(config.get("providers", {}).get("uv", {}).get("install"))
        if uv_pkg == "kimi-cli" and uv_pkg in self.get_uv_tools():
            found.append("uv")

        app_path = config.get("app_path")
        if (app_path and self._path_exists(app_path)) or bin_path:
            if not found:
                found.append("system")

        return found

    def check_tool_ready(self, name: str, ready_check: str | None) -> bool:
        import logging

        if not ready_check:
            return False
        if ready_check.startswith("bin:"):
            bin_name = ready_check[4:].strip()
            found = self._which(bin_name) is not None
            logging.debug(f"[TOOL: {name}] ready_check bin:{bin_name} -> {found}")
            return found
        if ready_check.startswith("app:"):
            app_path = os.path.expanduser(ready_check[4:].strip())
            found = self._path_exists(app_path)
            logging.debug(f"[TOOL: {name}] ready_check app:{app_path} -> {found}")
            return found
        if ready_check.startswith("brew:"):
            pkg_name = ready_check[5:].strip()
            if not self._which("brew"):
                logging.debug(f"[TOOL: {name}] ready_check brew:{pkg_name} -> False (brew not found)")
                return False
            found = pkg_name in self.get_brew_formulae() or pkg_name in self.get_brew_casks()
            logging.debug(f"[TOOL: {name}] ready_check brew:{pkg_name} -> {found}")
            return found
        logging.warning(f"[TOOL: {name}] 未知 ready_check 格式: {ready_check}，视为未就绪")
        return False


def detect_all_providers(bin_name, config=None, inventory=None):
    active_inventory = inventory or ProviderInventory()
    return active_inventory.detect_all_providers(bin_name, config)


def check_tool_ready(name: str, ready_check: str | None, inventory=None) -> bool:
    """根据 ready_check 字段判断工具是否已就绪。

    支持三种前缀：
      bin:<name>   — shutil.which 检查可执行文件
      app:<path>   — os.path.exists 检查路径（支持 ~）
      brew:<pkg>   — brew list 检查已安装的 formula/cask（跨架构通用）
    若 ready_check 为空则始终返回 False。
    """
    active_inventory = inventory or ProviderInventory()
    return active_inventory.check_tool_ready(name, ready_check)


def is_agent_installed(agent_name: str, agent_def: dict) -> bool:
    """快速检查 agent 是否已安装。

    优先检查 bin（shutil.which），其次检查 app_path（os.path.exists）。
    两者均未配置时返回 False。
    """
    bin_name = agent_def.get("bin")
    if bin_name and shutil.which(bin_name):
        return True
    app_path = agent_def.get("app_path")
    if app_path and os.path.exists(os.path.expanduser(app_path)):
        return True
    return False
