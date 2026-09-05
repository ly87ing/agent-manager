#!/usr/bin/env python3
"""Engine Contract v1 envelope infrastructure and domain kernels.

Contract source of truth: contract/engine-v1/*.schema.json (see contract/README.md)
and docs/GUI-ARCHITECTURE-PROPOSAL.md section 2 (envelope, exit codes) and
section 3 (dual-axis element model: convergence 'state' + upstream 'freshness').

Slices so far: status+plan for agents/mcp/tools/rules/hooks/aliases/skills,
read-only secrets status, system.audit (thin wrapper over doctor's
build_report), and system.refresh-upstream — the only writer of the upstream
probe cache (src/upstream_cache.py); status commands read that cache for the
freshness axis and stay zero-network. profile/policy/runs land in later phases.

Rules enforced here:
- stdout carries exactly one envelope JSON payload (indent=2, ensure_ascii=False);
  human-readable text only goes to stderr.
- Exit codes: 0 = ok (warnings allowed), 1 = errors non-empty,
  2 = contract/usage error (unknown domain, missing --json, ...).
- status is read-only and zero-network; the freshness block is rendered as
  'not-applicable' until the upstream probe cache slice lands.
- secrets never appear in any payload: env placeholders are reported as the
  variable name plus a resolved boolean, never as values (red line).
"""
from __future__ import annotations

import argparse
import copy
import datetime
import hashlib
import importlib.util
import json
import os
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.agent_ids import canonicalize_agent_list, canonicalize_agent_mapping
from src.command_runner import run_command
from src.git_remote_probe import (
    GIT_REMOTE_STATE_LATEST,
    GIT_REMOTE_STATE_STALE,
    probe_git_remote_state,
)
from src.json_config import get_typed_mapping_section, load_json_object_file
from src.manage_doctor import (
    _managed_mcp_matches,
    _discover_repo_skill_targets,
    _paths_match,
    actual_skill_targets,
    build_report,
    build_tree_signature,
    collect_skill_discovery_inbox,
    command_exists,
    command_version,
    expected_managed_mcp_servers,
    expected_managed_skill_targets,
    get_unsupported_actions,
    is_managed_skill_target_path,
    managed_mcp_servers,
    resolve_skill_search_paths,
    rules_match,
)
from src.manage_runtime_probe import (
    ProviderInventory,
    canonical_provider_name,
    classify_provider_state,
    collect_active_rule_agents,
    detect_all_providers,
    detect_tool_readiness,
    is_agent_installed,
    repo_skill_matrix_mode,
    resolve_managed_tool_dir,
    resolve_provider_command,
    resolve_repo_skill_agent_overrides,
    rule_target_agent_id,
    validate_repo_new_skill_default,
    validate_repo_skill_matrix,
)
from src.npm_flags import OFFICIAL_NPM_REGISTRY
from src import upstream_cache

CONTRACT_ID = "agent-manager/engine/v1"

# Domains named by the command surface (proposal section 2.3). Skills are
# deliberately absent from the kernel maps: a parallel slice is evolving the
# skills v2 semantics, so skills commands keep answering DOMAIN_NOT_IMPLEMENTED.
KNOWN_DOMAINS = (
    "agents",
    "mcp",
    "skills",
    "rules",
    "hooks",
    "tools",
    "aliases",
    "secrets",
    "profile",
    "policy",
    "system",
    "runs",
)

# Proposal section 3.3: until the upstream probe cache exists, every element
# reports the upstream axis as not applicable. The schema requires all six
# fields (nullable), so they are emitted explicitly.
FRESHNESS_NOT_APPLICABLE = {
    "state": "not-applicable",
    "current": None,
    "upstream": None,
    "checked_at": None,
    "probe": None,
    "cache_age_sec": None,
}

MCP_APPLY_ACTION = {
    "id": "apply",
    "risk": "write-local",
    "needs_lock": True,
    "offline_ok": True,
}

# Proposal section 5 control matrix: write-local actions for config-file
# convergence; install/upgrade actions are network or install-remove grade.
RULES_APPLY_ACTION = dict(MCP_APPLY_ACTION)
HOOKS_APPLY_ACTION = dict(MCP_APPLY_ACTION)
ALIAS_APPLY_ACTION = dict(MCP_APPLY_ACTION)
SECRETS_PULL_ACTION = {
    "id": "pull",
    "risk": "network",
    "needs_lock": True,
    "offline_ok": False,
}
SKILL_REPO_PULL_ACTION = {
    "id": "pull",
    "risk": "network",
    "needs_lock": True,
    "offline_ok": False,
}
SKILL_APPLY_ACTION = {
    "id": "apply",
    "risk": "exec-local",
    "needs_lock": True,
    "offline_ok": True,
}
AGENT_INSTALL_ACTION = {
    "id": "install",
    "risk": "install-remove",
    "needs_lock": True,
    "offline_ok": False,
}
AGENT_UPGRADE_ACTION = {
    "id": "upgrade",
    "risk": "network",
    "needs_lock": True,
    "offline_ok": False,
}
AGENT_UNINSTALL_ACTION = {
    "id": "uninstall",
    "risk": "install-remove",
    "needs_lock": True,
    "offline_ok": False,
}
TOOL_INSTALL_ACTION = {
    "id": "install",
    "risk": "install-remove",
    "needs_lock": True,
    "offline_ok": False,
}
TOOL_UPGRADE_ACTION = {
    "id": "upgrade",
    "risk": "install-remove",
    "needs_lock": True,
    "offline_ok": False,
}

ALIAS_MARKER_BEGIN = "# >>> agent-manager shell aliases >>>"
ALIAS_MARKER_END = "# <<< agent-manager shell aliases <<<"


def _load_module_from_path(relative_path, module_name):
    """Import a repo script read-only; directories like 'agent-mcp' are not
    valid Python identifiers, so modules are loaded by file path (mirrors
    tests/agent_mcp_import.py)."""
    module_path = REPO_ROOT / relative_path
    spec = importlib.util.spec_from_file_location(module_name, module_path)
    module = importlib.util.module_from_spec(spec)
    assert spec is not None and spec.loader is not None
    spec.loader.exec_module(module)
    return module


_sync_mcp = _load_module_from_path("agent-mcp/sync_mcp.py", "agent_manager_sync_mcp")
placeholder_name = _sync_mcp.placeholder_name
load_env_files = _sync_mcp.load_env_files

_sync_claude_hooks = _load_module_from_path(
    "agent-rule/scripts/sync-claude-hooks.py", "agent_manager_sync_claude_hooks"
)
_sync_codex_hooks = _load_module_from_path(
    "agent-rule/scripts/sync-codex-hooks.py", "agent_manager_sync_codex_hooks"
)
_link_skills = _load_module_from_path("agent-skill/link_skills.py", "agent_manager_link_skills")

# sync_tools.py 在工作区演进中出现过 py3.9 不兼容形态；引擎只在探测编排时惰性
# 加载它，加载失败只降级上游探测能力（freshness 变 not-applicable + refresh 给
# 提示），绝不拖垮 status 主路径。
_SYNC_TOOLS_UNAVAILABLE = None


def _load_sync_tools():
    global _SYNC_TOOLS_UNAVAILABLE
    try:
        return _load_module_from_path("agent-tool/sync_tools.py", "agent_manager_sync_tools")
    except Exception as exc:
        _SYNC_TOOLS_UNAVAILABLE = exc
        return None


def diagnostic(code, element, message):
    """One warning/error entry; 'element' is an element id or None (run-level)."""
    return {"code": code, "element": element, "message": message}


def envelope(command, mode, lock, run_id, log_file, data, warnings, errors):
    """Build the uniform stdout payload defined by envelope.schema.json."""
    return {
        "contract": CONTRACT_ID,
        "run_id": run_id,
        "command": command,
        "mode": {
            "dry_run": bool(mode.get("dry_run", False)),
            "offline": bool(mode.get("offline", False)),
        },
        "lock": {
            "held": bool(lock.get("held", False)),
            "pid": lock.get("pid"),
        },
        "log_file": log_file,
        "ok": not errors,
        "data": data,
        "warnings": warnings,
        "errors": errors,
    }


def emit_envelope(payload):
    json.dump(payload, sys.stdout, indent=2, ensure_ascii=False)
    sys.stdout.write("\n")


def generate_run_id():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return "%s-%x" % (stamp, os.getpid())


def _redact_env_for_spec(env_dict):
    """Spec-layer env: keep placeholder names verbatim, mask literal values."""
    redacted = {}
    for key, value in env_dict.items():
        if placeholder_name(value) is not None:
            redacted[key] = value
        else:
            redacted[key] = "<literal>"
    return redacted


def _env_resolution_status(env_dict, secrets_env):
    """Per-env-key placeholder status; values are never read into the payload."""
    status = {}
    for key, value in env_dict.items():
        var_name = placeholder_name(value)
        if var_name is None:
            status[key] = {"placeholder": None, "resolved": True}
        else:
            status[key] = {
                "placeholder": var_name,
                "resolved": var_name in secrets_env or var_name in os.environ,
            }
    return status


def _probe_mcp_agent_targets(config, mcp_cfg):
    """Read each agent's MCP target file once; parse failures become errors."""
    agents_policy = config.get("agents", {})
    if not isinstance(agents_policy, dict):
        agents_policy = {}
    else:
        # manage.sh 仅在存在覆盖层时才 canonicalize；基线直出时这里兜底（对齐 doctor）。
        agents_policy = canonicalize_agent_mapping(agents_policy)
    probes = {}
    errors = []
    for agent_id, path_config in canonicalize_agent_mapping(mcp_cfg.get("agent_paths", {})).items():
        if isinstance(path_config, str):
            target_path, file_format = path_config, "json"
        else:
            target_path = path_config.get("path", "")
            file_format = path_config.get("format", "json")
        target_path = os.path.expanduser(target_path)
        agent_policy = agents_policy.get(agent_id, {})
        agent_enabled = agent_policy.get("enabled", True) if isinstance(agent_policy, dict) else True
        probe = {
            "target": target_path,
            "format": file_format,
            "agent_enabled": agent_enabled,
            "target_exists": os.path.exists(target_path),
            "servers": {},
            "parse_error": None,
        }
        if probe["target_exists"]:
            try:
                probe["servers"] = managed_mcp_servers(target_path, file_format, agent_id)
            except ValueError as exc:
                probe["parse_error"] = str(exc)
                errors.append(
                    diagnostic(
                        "MCP_TARGET_PARSE_ERROR",
                        None,
                        "agent '%s' 的 MCP 配置解析失败 (%s): %s" % (agent_id, target_path, exc),
                    )
                )
        probes[agent_id] = probe
    return probes, errors


def _mcp_cell_state(server_name, probe, expected_servers, server_enabled):
    """Convergence state of one server x agent cell (element-level vocabulary)."""
    if not server_enabled:
        # 期望状态为“不存在”：任何托管残留都是偏离（对齐 doctor 的残留报错语义）。
        return "leftover" if server_name in probe["servers"] else "absent"
    if not probe["agent_enabled"]:
        return "leftover" if server_name in probe["servers"] else "agent-disabled"
    if probe["parse_error"] is not None:
        return "probe-error"
    if not probe["target_exists"]:
        return "not-materialized"
    if server_name not in probe["servers"]:
        return "missing"
    expected = expected_servers.get(server_name)
    if expected is None:
        return "missing"
    if _managed_mcp_matches(expected, probe["servers"][server_name]):
        return "managed-ok"
    return "drifted"


def _aggregate_mcp_state(cell_states):
    """Collapse per-agent cell states into the element-level seven-state axis."""
    if any(state in ("drifted", "leftover", "probe-error") for state in cell_states):
        return "drifted"
    if "missing" in cell_states:
        return "missing"
    if cell_states and all(state == "managed-ok" for state in cell_states):
        return "managed-ok"
    if "not-materialized" in cell_states:
        return "not-materialized"
    # Only 'agent-disabled'/'absent' cells remain: desired state is achieved.
    return "managed-ok"


def build_mcp_status(root_dir, config_file, def_file):
    """'<domain>.status' data payload for the mcp domain plus diagnostics.

    Element universe = servers defined in config/mcp.json (Spec layer); each
    element carries per-agent injection cells in 'observed.agents'.
    """
    warnings = []
    errors = []
    config_name = Path(config_file).name
    def_name = Path(def_file).name
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    policy_cfg = get_typed_mapping_section(config, "mcp", config_name, (dict,), "对象")
    def_data = load_json_object_file(def_file, strict=True, missing_ok=False)
    servers = get_typed_mapping_section(def_data, "servers", def_name, (dict,), "对象")
    agent_paths = get_typed_mapping_section(def_data, "agent_paths", def_name, (str, dict), "字符串或对象")
    mcp_cfg = {"servers": servers, "agent_paths": agent_paths}

    # Same credential sources as agent-mcp/sync_mcp.py, minus the network sync.
    secrets_env = load_env_files([root_dir / ".env", root_dir / "secrets" / "mcp.env"])
    cache = upstream_cache.load_cache()

    probes, probe_errors = _probe_mcp_agent_targets(config, mcp_cfg)
    errors.extend(probe_errors)
    expected_by_agent = {}
    for agent_id, probe in probes.items():
        if probe["agent_enabled"] and probe["parse_error"] is None:
            expected_by_agent[agent_id] = expected_managed_mcp_servers(root_dir, config, mcp_cfg, agent_id)

    elements = []
    for server_name in sorted(servers):
        definition = servers[server_name]
        policy = policy_cfg.get(server_name)
        declared = isinstance(policy, dict)
        enabled = declared and bool(policy.get("enabled", True))
        merged = dict(definition)
        if declared:
            merged.update(policy)
        targeted_agents = canonicalize_agent_list(merged.get("agents"))
        raw_env = definition.get("env", {})
        if not isinstance(raw_env, dict):
            raw_env = {}
        merged_env = merged.get("env", {})
        if not isinstance(merged_env, dict):
            merged_env = {}

        cells = {}
        for agent_id, probe in probes.items():
            if targeted_agents is not None and agent_id not in targeted_agents:
                continue
            cells[agent_id] = {
                "state": _mcp_cell_state(
                    server_name, probe, expected_by_agent.get(agent_id, {}), enabled
                ),
                "target": probe["target"],
                "target_exists": probe["target_exists"],
            }

        if not enabled:
            # 禁用/未声明元素：无残留即收敛于 disabled；有残留则偏离为 drifted。
            if any(cell["state"] == "leftover" for cell in cells.values()):
                state = "drifted"
            else:
                state = "disabled"
        elif not cells:
            state = "disabled"
        else:
            state = _aggregate_mcp_state([cell["state"] for cell in cells.values()])

        env_status = _env_resolution_status(merged_env, secrets_env)
        if enabled:
            for env_key, entry in env_status.items():
                if entry["placeholder"] is not None and not entry["resolved"]:
                    warnings.append(
                        diagnostic(
                            "MCP_ENV_UNRESOLVED",
                            "mcp:%s" % server_name,
                            "env 键 %s 的占位符 %s 未解析（.env / secrets/mcp.env / 环境变量均无）"
                            % (env_key, entry["placeholder"]),
                        )
                    )

        spec = {key: value for key, value in definition.items() if key != "env"}
        spec["env"] = _redact_env_for_spec(raw_env)
        probe_kind = _classify_mcp_probe(merged) if enabled else None
        local_current = None
        if probe_kind is not None and probe_kind[0] == "npm-npx":
            local_current = _npx_cached_version(probe_kind[1])
        freshness = _freshness_from_cache(
            "mcp:%s" % server_name,
            probeable=probe_kind is not None,
            cache=cache,
            local_current=local_current,
            warnings=warnings,
        )
        elements.append(
            {
                "id": "mcp:%s" % server_name,
                "domain": "mcp",
                "spec": spec,
                "policy": {
                    "enabled": enabled,
                    "declared_in_policy": declared,
                    "agents": targeted_agents,
                },
                "observed": {
                    "agents": cells,
                    "env": env_status,
                },
                "state": state,
                "freshness": freshness,
                "actions": [dict(MCP_APPLY_ACTION)],
            }
        )

    return {"domain": "mcp", "elements": elements}, warnings, errors


# ---------------------------------------------------------------------------
# plan helpers (contract/engine-v1/plan.schema.json)
# ---------------------------------------------------------------------------


def plan_action(op, element, target, risk, before_digest=None, after_digest=None, backup_path=None, cmd=None, note=None):
    """One plan-action entry; nullable fields stay explicit per the contract."""
    return {
        "op": op,
        "element": element,
        "target": target,
        "risk": risk,
        "before_digest": before_digest,
        "after_digest": after_digest,
        "backup_path": backup_path,
        "cmd": cmd,
        "note": note,
    }


def plan_data(domain, actions):
    """Wrap actions into the '<domain>.plan' data payload with risk counts."""
    summary = {"read": 0, "write-local": 0, "exec-local": 0, "network": 0, "install-remove": 0}
    for action in actions:
        summary[action["risk"]] += 1
    return {"domain": domain, "actions": actions, "summary": summary}


def _sha256_text(text):
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def _sha256_canonical(obj):
    return _sha256_text(json.dumps(obj, sort_keys=True, ensure_ascii=False))


def _sha256_path(path):
    """Digest of a file, or of a directory tree via its signature map."""
    if os.path.isdir(path):
        return _sha256_canonical(build_tree_signature(path))
    try:
        with open(path, "rb") as handle:
            return "sha256:" + hashlib.sha256(handle.read()).hexdigest()
    except OSError:
        return None


def _rendered_json_digest(payload):
    """Digest of a file as atomic_write_json_file would write it."""
    return _sha256_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")


def _make_element(element_id, domain, spec, policy, observed, state, actions, freshness=None):
    return {
        "id": element_id,
        "domain": domain,
        "spec": spec,
        "policy": policy,
        "observed": observed,
        "state": state,
        "freshness": dict(freshness) if freshness is not None else dict(FRESHNESS_NOT_APPLICABLE),
        "actions": [dict(action) for action in actions],
    }


# ---------------------------------------------------------------------------
# freshness axis: local (zero-network) current probes + cache-backed fill
# ---------------------------------------------------------------------------

_VERSION_RE = re.compile(r"\d+(?:\.\d+)+")


def _extract_version_token(text):
    match = _VERSION_RE.search(text or "")
    return match.group(0) if match else None


def _version_tuple(text):
    token = _extract_version_token(text)
    if token is None:
        return None
    return tuple(int(part) for part in token.split("."))


def _compare_freshness(current, upstream):
    """Map (current, upstream) onto the freshness state enum."""
    if not upstream or not current:
        return "unknown"
    if current == upstream:
        return "current"
    current_key = _version_tuple(current)
    upstream_key = _version_tuple(upstream)
    if current_key and upstream_key:
        width = max(len(current_key), len(upstream_key))
        current_key += (0,) * (width - len(current_key))
        upstream_key += (0,) * (width - len(upstream_key))
        if current_key == upstream_key:
            return "current"
        return "ahead" if current_key > upstream_key else "behind"
    # Unorderable values (e.g. commit hashes): any difference reads as behind;
    # ahead is only asserted when versions can be ordered.
    return "behind"


def _run_probe_cmd(cmd, cwd=None, timeout=30):
    try:
        result = run_command(cmd, cwd=cwd, check=True, capture_output=True, text=True, timeout=timeout)
        return True, result.stdout
    except Exception:
        return False, ""


def _agent_local_version(definition):
    """Local bin --version probe (read-only, no network; doctor precedent)."""
    bin_name = definition.get("bin")
    if not bin_name or not command_exists(bin_name):
        return None
    return _extract_version_token(command_version(bin_name))


def _npx_cached_version(package):
    """Best-effort read of the npx cache (~/.npm/_npx); None when absent."""
    if not package:
        return None
    cache_base = os.path.expanduser(os.path.join("~", ".npm", "_npx"))
    try:
        entries = os.listdir(cache_base)
    except OSError:
        return None
    package_json_rel = os.path.join("node_modules", *package.split("/"), "package.json")
    for entry in sorted(entries):
        package_json = os.path.join(cache_base, entry, package_json_rel)
        try:
            with open(package_json, encoding="utf-8") as handle:
                version = json.load(handle).get("version")
            if isinstance(version, str) and version.strip():
                return version.strip()
        except (OSError, ValueError):
            continue
    return None


def _local_git_head(repo_dir):
    ok, out = _run_probe_cmd(["git", "rev-parse", "HEAD"], cwd=repo_dir)
    if not ok:
        return None
    head = out.strip().splitlines()
    return head[0].strip() if head and head[0].strip() else None


def _freshness_from_cache(element_id, probeable, cache, local_current, warnings):
    """Fill the freshness block for one element (status path, zero network).

    Not probeable (no upstream concept) -> not-applicable, but 'current' still
    carries the locally observed version for visibility (proposal section 3.3,
    detect-only row). Probeable but never refreshed -> unknown with nulls.
    Stale entries render last-known values plus a prompt-level warning.
    """
    if not probeable:
        freshness = dict(FRESHNESS_NOT_APPLICABLE)
        freshness["current"] = local_current
        return freshness
    entry = upstream_cache.get_entry(cache, element_id)
    if entry is None:
        return {
            "state": "unknown",
            "current": local_current,
            "upstream": None,
            "checked_at": None,
            "probe": None,
            "cache_age_sec": None,
        }
    if upstream_cache.is_stale(entry):
        warnings.append(
            diagnostic(
                "UPSTREAM_PROBE_STALE",
                element_id,
                "上游探测缓存超过 TTL，展示 last-known 值；显式 system:refresh-upstream 刷新",
            )
        )
    return {
        "state": entry.get("state", "unknown"),
        "current": entry.get("current") or local_current,
        "upstream": entry.get("upstream"),
        "checked_at": entry.get("checked_at"),
        "probe": entry.get("probe"),
        "cache_age_sec": upstream_cache.cache_age_sec(entry),
    }


def _classify_agent_probe(definition, policy):
    """Return ('npm', package) when the preferred provider is npm, else None."""
    sync_tools = _load_sync_tools()
    if sync_tools is None:
        return None
    preferred = (policy or {}).get("preferred_provider") or definition.get("preferred_provider", "npm")
    provider_cfg = definition.get("providers", {}).get(preferred, {})
    install_cmd = resolve_provider_command(provider_cfg.get("install"))
    package = sync_tools._extract_npm_package_name(install_cmd) if install_cmd else None
    return ("npm", package) if package else None


def _classify_tool_probe(root_dir, name, definition):
    """Proposal section 3.3 matrix for tools: npm / git repo / uv(pypi) / brew."""
    if definition.get("url"):
        return ("git-repo", resolve_managed_tool_dir(str(root_dir), name))
    sync_tools = _load_sync_tools()
    if sync_tools is None:
        return None
    install_cmd = definition.get("install")
    package = sync_tools._extract_npm_package_name(install_cmd)
    if package:
        return ("npm", package)
    uv_name = sync_tools._extract_uv_tool_name(install_cmd)
    if uv_name:
        return ("pypi", uv_name)
    brew_probe = sync_tools._build_brew_outdated_probe(install_cmd)
    if brew_probe:
        return ("brew", brew_probe[-1])
    return None


def _classify_mcp_probe(merged_definition):
    """npx packages probe npm; uvx 'git+…@branch' sources probe git ls-remote."""
    command = merged_definition.get("command")
    args = merged_definition.get("args") or []
    if not isinstance(args, list):
        return None
    if command == "npx":
        for arg in args:
            if isinstance(arg, str) and not arg.startswith("-"):
                return ("npm-npx", arg)
        return None
    if command == "uvx":
        for arg in args:
            if not isinstance(arg, str) or not arg.startswith("git+"):
                continue
            repo_and_branch = arg[len("git+"):]
            if "@" not in repo_and_branch:
                return None
            repo, branch = repo_and_branch.rsplit("@", 1)
            return ("git-ls-remote", repo, branch)
    return None


def _load_policy_section(config_file, section):
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    typed = get_typed_mapping_section(config, section, Path(config_file).name, (dict,), "对象")
    return config, canonicalize_agent_mapping(typed) if section == "agents" else typed


# ---------------------------------------------------------------------------
# agents domain (proposal section 5: upgrade=network, install/uninstall=install-remove)
# ---------------------------------------------------------------------------


def _agent_element_state(agent_id, definition, policy, declared, enabled, providers, classified, installed, diagnostics):
    """Map spec x policy x observed onto the seven-state convergence axis."""
    preferred = (policy or {}).get("preferred_provider") or definition.get("preferred_provider", "npm")
    if enabled:
        if classified["unsupported"]:
            diagnostics["errors"].append(
                diagnostic(
                    "AGENT_PROVIDER_UNSUPPORTED",
                    "agents:%s" % agent_id,
                    "检测到未声明的安装来源: %s" % ", ".join(classified["unsupported"]),
                )
            )
            return "unsupported"
        if classified["deprecated"]:
            return "drifted"
        if classified["system_only"] and definition.get("require_managed_provider", False):
            return "unmanaged-detected"
        if providers and not installed:
            # doctor: provider 已激活但运行入口缺失 = 漂移。
            return "drifted"
        if classified["managed"] and installed:
            return "managed-ok"
        if providers:
            return "unmanaged-detected"
        return "missing"
    if not declared:
        # 定义层存在但策略未声明（如 detect-only 的 kimi-code）：探测到才上报。
        return "unmanaged-detected" if (providers or installed) else "disabled"
    if providers or installed:
        # doctor: 禁用但仍安装 = 漂移；官方无 uninstall 动作时降为提示。
        preferred_cfg = definition.get("providers", {}).get(canonical_provider_name(preferred), {})
        try:
            unsupported_actions = get_unsupported_actions(preferred_cfg)
        except ValueError as exc:
            diagnostics["errors"].append(
                diagnostic("AGENT_PROVIDER_CONFIG_INVALID", "agents:%s" % agent_id, str(exc))
            )
            unsupported_actions = set()
        if "uninstall" in unsupported_actions:
            diagnostics["warnings"].append(
                diagnostic(
                    "AGENT_OFFICIAL_GAP",
                    "agents:%s" % agent_id,
                    "官方未提供受管 uninstall 动作，引擎不会伪造删除命令",
                )
            )
        return "drifted"
    return "disabled"


def _agent_actions_for_state(state):
    if state == "missing":
        return [AGENT_INSTALL_ACTION]
    if state in ("managed-ok", "drifted"):
        return [AGENT_UPGRADE_ACTION, AGENT_UNINSTALL_ACTION]
    if state == "unmanaged-detected":
        return [AGENT_INSTALL_ACTION, AGENT_UNINSTALL_ACTION]
    return []


def build_agents_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    diagnostics = {"warnings": warnings, "errors": errors}
    config, agents_policy = _load_policy_section(config_file, "agents")
    agent_defs = canonicalize_agent_mapping(
        load_json_object_file(def_files[0], strict=True, missing_ok=False)
    )
    inventory = ProviderInventory()
    cache = upstream_cache.load_cache()
    elements = []
    for agent_id in sorted(agent_defs):
        definition = agent_defs[agent_id]
        if not isinstance(definition, dict):
            continue
        policy = agents_policy.get(agent_id)
        declared = isinstance(policy, dict)
        enabled = declared and bool(policy.get("enabled", True))
        preferred = (policy or {}).get("preferred_provider") or definition.get("preferred_provider", "npm")
        providers = detect_all_providers(definition.get("bin", ""), definition, inventory=inventory)
        classified = classify_provider_state(providers, definition)
        installed = is_agent_installed(agent_id, definition)
        preferred_canonical = canonical_provider_name(preferred)
        preferred_active = any(
            canonical_provider_name(name) == preferred_canonical for name in classified["managed"]
        )
        state = _agent_element_state(
            agent_id, definition, policy, declared, enabled, providers, classified, installed, diagnostics
        )
        probe = _classify_agent_probe(definition, policy) if enabled else None
        freshness = _freshness_from_cache(
            "agents:%s" % agent_id,
            probeable=probe is not None,
            cache=cache,
            local_current=_agent_local_version(definition),
            warnings=warnings,
        )
        elements.append(
            _make_element(
                "agents:%s" % agent_id,
                "agents",
                dict(definition),
                {
                    "enabled": enabled,
                    "declared_in_policy": declared,
                    "auto_install": bool((policy or {}).get("auto_install", True)),
                    "auto_update": bool((policy or {}).get("auto_update", True)),
                    "preferred_provider": preferred,
                },
                {
                    "providers": providers,
                    "classified": classified,
                    "installed": installed,
                    "preferred_active": preferred_active,
                },
                state,
                _agent_actions_for_state(state),
                freshness=freshness,
            )
        )
    return {"domain": "agents", "elements": elements}, warnings, errors


def build_agents_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_agents_status(root_dir, config_file, def_files)
    agent_defs = canonicalize_agent_mapping(
        load_json_object_file(def_files[0], strict=True, missing_ok=False)
    )
    actions = []
    for element in status["elements"]:
        if element["state"] != "missing":
            continue
        agent_id = element["id"].split(":", 1)[1]
        if not element["policy"]["auto_install"]:
            warnings.append(
                diagnostic(
                    "AGENT_INSTALL_DISABLED_BY_POLICY",
                    element["id"],
                    "策略 auto_install=false，plan 不生成安装动作",
                )
            )
            continue
        preferred = element["policy"]["preferred_provider"]
        provider_cfg = agent_defs.get(agent_id, {}).get("providers", {}).get(preferred, {})
        install_cmd = resolve_provider_command(provider_cfg.get("install"))
        if not install_cmd:
            warnings.append(
                diagnostic(
                    "AGENT_INSTALL_UNAVAILABLE",
                    element["id"],
                    "provider '%s' 未提供当前平台的 install 命令" % preferred,
                )
            )
            continue
        actions.append(
            plan_action(
                "install",
                element["id"],
                None,
                "install-remove",
                cmd=install_cmd,
                note="安装缺失 Agent（provider: %s）" % preferred,
            )
        )
    return plan_data("agents", actions), warnings, errors


# ---------------------------------------------------------------------------
# tools domain (proposal section 5: install/upgrade = install-remove)
# ---------------------------------------------------------------------------


def build_tools_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    config_name = Path(config_file).name
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    tools_policy = get_typed_mapping_section(config, "tools", config_name, (dict,), "对象")
    tool_defs = get_typed_mapping_section(
        load_json_object_file(def_files[0], strict=True, missing_ok=False),
        "tools",
        Path(def_files[0]).name,
        (dict,),
        "对象",
    )
    inventory = ProviderInventory()
    cache = upstream_cache.load_cache()
    elements = []
    for name in sorted(set(tool_defs) | set(tools_policy)):
        definition = tool_defs.get(name)
        policy = tools_policy.get(name)
        declared = isinstance(policy, dict)
        enabled = declared and bool(policy.get("enabled", True))
        if not isinstance(definition, dict):
            errors.append(
                diagnostic(
                    "TOOL_DEFINITION_MISSING",
                    "tools:%s" % name,
                    "策略启用了 '%s' 但 %s 缺少定义" % (name, Path(def_files[0]).name),
                )
            )
            elements.append(
                _make_element(
                    "tools:%s" % name,
                    "tools",
                    {},
                    {"enabled": enabled, "declared_in_policy": declared},
                    {"ready": False},
                    "unsupported",
                    [],
                )
            )
            continue
        ready = detect_tool_readiness(name, definition, str(root_dir), inventory=inventory)
        if enabled:
            state = "managed-ok" if ready else "missing"
            if not ready and definition.get("doctor_severity") == "warning":
                warnings.append(
                    diagnostic("TOOL_OPTIONAL_NOT_READY", "tools:%s" % name, "可选工具未就绪（提示级）")
                )
        else:
            state = "disabled"
        actions = []
        if state == "missing":
            actions = [TOOL_INSTALL_ACTION]
        elif state == "managed-ok":
            actions = [TOOL_UPGRADE_ACTION]
        probe = _classify_tool_probe(root_dir, name, definition) if enabled else None
        local_current = None
        if probe is not None and probe[0] == "git-repo":
            local_current = _local_git_head(probe[1])
        freshness = _freshness_from_cache(
            "tools:%s" % name,
            probeable=probe is not None,
            cache=cache,
            local_current=local_current,
            warnings=warnings,
        )
        elements.append(
            _make_element(
                "tools:%s" % name,
                "tools",
                dict(definition),
                {
                    "enabled": enabled,
                    "declared_in_policy": declared,
                    "auto_update": bool((policy or {}).get("auto_update", True)),
                },
                {"ready": ready, "doctor_severity": definition.get("doctor_severity", "error")},
                state,
                actions,
                freshness=freshness,
            )
        )
    return {"domain": "tools", "elements": elements}, warnings, errors


def build_tools_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_tools_status(root_dir, config_file, def_files)
    actions = []
    for element in status["elements"]:
        if element["state"] != "missing":
            continue
        definition = element["spec"]
        install_cmd = definition.get("install")
        if not install_cmd and definition.get("url"):
            install_cmd = "git clone %s" % definition["url"]
        setup_cmd = definition.get("setup")
        note_parts = ["安装并初始化工具"]
        if setup_cmd:
            note_parts.append("随后执行 setup: %s" % setup_cmd)
        actions.append(
            plan_action(
                "install",
                element["id"],
                None,
                "install-remove",
                cmd=install_cmd,
                note="；".join(note_parts),
            )
        )
    return plan_data("tools", actions), warnings, errors


# ---------------------------------------------------------------------------
# rules domain (src -> dist build freshness + per-target convergence)
# ---------------------------------------------------------------------------


def _parse_rules_manifest(manifest_path):
    entries = []
    problems = []
    for raw_line in Path(manifest_path).read_text(encoding="utf-8").splitlines():
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        parts = [part.strip() for part in stripped.split("|")]
        if len(parts) < 4:
            problems.append(stripped)
            continue
        entries.append({"name": parts[0], "group": parts[1], "source": parts[2], "target": parts[3]})
    return entries, problems


def _latest_mtime(root):
    latest = None
    for dirpath, _dirnames, filenames in os.walk(root):
        for filename in filenames:
            mtime = os.path.getmtime(os.path.join(dirpath, filename))
            if latest is None or mtime > latest:
                latest = mtime
    return latest


def _rules_active_agents(root_dir, config):
    """Same scope rule as doctor: enabled agents plus skill/mcp target agents."""
    def _load_cfg(rel_name, sections):
        raw = load_json_object_file(root_dir / "config" / rel_name, strict=True, missing_ok=False)
        return {
            key: get_typed_mapping_section(raw, key, rel_name, value_types, label)
            for key, value_types, label in sections
        }

    skill_cfg = _load_cfg("skills.json", (("repositories", (dict,), "对象"), ("agents", (str, dict), "字符串或对象")))
    mcp_cfg = _load_cfg("mcp.json", (("servers", (dict,), "对象"), ("agent_paths", (str, dict), "字符串或对象")))
    return collect_active_rule_agents(config, skill_cfg, mcp_cfg)


def build_rules_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    if "agents" in config:
        config["agents"] = canonicalize_agent_mapping(
            get_typed_mapping_section(config, "agents", Path(config_file).name, (dict,), "对象")
        )
    entries, problems = _parse_rules_manifest(def_files[0])
    for problem in problems:
        errors.append(diagnostic("RULES_MANIFEST_INVALID", None, "manifest 非法行: %s" % problem))
    active_agents = _rules_active_agents(root_dir, config)
    rules_src_dir = root_dir / "agent-rule" / "rules" / "src"
    src_latest_mtime = _latest_mtime(str(rules_src_dir)) if rules_src_dir.is_dir() else None
    home = os.path.expanduser("~")

    elements = []
    stale_products = []
    missing_products = []
    seen_sources = []
    for entry in entries:
        agent_id = rule_target_agent_id(entry["name"])
        is_repo_root_target = "{{REPO_ROOT}}" in entry["target"]
        in_scope = is_repo_root_target or agent_id in active_agents
        src = str(root_dir / "agent-rule" / entry["source"])
        dst = os.path.expanduser(
            entry["target"].replace("{{HOME}}", home).replace("{{REPO_ROOT}}", str(root_dir))
        )
        if entry["source"] not in seen_sources:
            seen_sources.append(entry["source"])
        source_exists = os.path.exists(src)
        target_exists = os.path.exists(dst)
        dist_mtime = os.path.getmtime(src) if source_exists else None
        build_stale = (
            src_latest_mtime is not None and dist_mtime is not None and src_latest_mtime > dist_mtime
        )
        if not source_exists:
            missing_products.append(entry["source"])
        elif build_stale:
            stale_products.append(entry["source"])

        if not in_scope:
            state = "disabled"
        elif not source_exists:
            warnings.append(
                diagnostic(
                    "RULE_SOURCE_NOT_BUILT",
                    "rules:%s" % entry["name"],
                    "dist 产物 %s 尚未生成（需先构建 rules）" % entry["source"],
                )
            )
            state = "not-materialized"
        elif not target_exists:
            state = "not-materialized"
        elif rules_match(src, dst):
            state = "managed-ok"
        else:
            state = "drifted"
        if build_stale and in_scope and state == "managed-ok":
            warnings.append(
                diagnostic(
                    "RULE_BUILD_STALE",
                    "rules:%s" % entry["name"],
                    "rules/src 比 dist 产物新，构建落后（提示级）",
                )
            )
        elements.append(
            _make_element(
                "rules:%s" % entry["name"],
                "rules",
                dict(entry),
                {"enabled": in_scope},
                {
                    "source": src,
                    "target": dst,
                    "source_exists": source_exists,
                    "target_exists": target_exists,
                    "build_stale": build_stale,
                },
                state,
                [RULES_APPLY_ACTION] if in_scope else [],
            )
        )

    # The dist build product itself is an element: staleness is convergence state.
    if missing_products:
        dist_state = "not-materialized"
    elif stale_products:
        dist_state = "drifted"
    else:
        dist_state = "managed-ok"
    elements.append(
        _make_element(
            "rules:dist",
            "rules",
            {"source_dir": "agent-rule/rules/src", "dist_dir": "agent-rule/rules/dist"},
            {"enabled": True},
            {
                "src_latest_mtime": src_latest_mtime,
                "products": seen_sources,
                "missing_products": missing_products,
                "stale_products": stale_products,
            },
            dist_state,
            [RULES_APPLY_ACTION],
        )
    )
    return {"domain": "rules", "elements": elements}, warnings, errors


def build_rules_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_rules_status(root_dir, config_file, def_files)
    # dist 构建必须先于任何分发动作（write-file 依赖构建产物）。
    ordered = [e for e in status["elements"] if e["id"] == "rules:dist"]
    ordered += [e for e in status["elements"] if e["id"] != "rules:dist"]
    actions = []
    for element in ordered:
        if element["id"] == "rules:dist":
            if element["state"] in ("drifted", "not-materialized"):
                actions.append(
                    plan_action(
                        "run-cmd",
                        "rules:dist",
                        None,
                        "exec-local",
                        cmd="python3 agent-rule/scripts/build-agent-rules.py",
                        note="先重建 dist，再分发（构建落后/产物缺失）",
                    )
                )
            continue
        if element["state"] not in ("drifted", "missing", "not-materialized"):
            continue
        if not element["observed"]["source_exists"]:
            continue  # 构建动作会覆盖该情况
        target = element["observed"]["target"]
        is_dir = os.path.isdir(element["observed"]["source"])
        actions.append(
            plan_action(
                "write-file",
                element["id"],
                target,
                "write-local",
                before_digest=_sha256_path(target) if element["observed"]["target_exists"] else None,
                after_digest=_sha256_path(element["observed"]["source"]),
                note="目录收敛同步" if is_dir else "分发 rules 产物",
            )
        )
    return plan_data("rules", actions), warnings, errors


# ---------------------------------------------------------------------------
# hooks domain (claude: signature-set merge into repo .claude/settings.json;
# codex: whole-file render into repo .codex/hooks.json)
# ---------------------------------------------------------------------------


def _collect_present_hook_signatures(existing_hooks):
    """Tolerant variant of _collect_managed_signatures for user-edited files."""
    signatures = set()
    if not isinstance(existing_hooks, dict):
        return signatures
    for event_name, entries in existing_hooks.items():
        if not isinstance(entries, list):
            continue
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            matcher = entry.get("matcher")
            if not isinstance(matcher, str):
                matcher = None
            hooks = entry.get("hooks")
            if not isinstance(hooks, list):
                continue
            for hook in hooks:
                if isinstance(hook, dict) and isinstance(hook.get("command"), str):
                    signatures.add((event_name, matcher, hook["command"]))
    return signatures


def _removed_rule_present(rule, present_signatures):
    for event_name, matcher, command in present_signatures:
        if rule["event"] != event_name:
            continue
        if rule["matcher"] is not None and rule["matcher"] != matcher:
            continue
        if rule["command"] is not None and rule["command"] != command:
            continue
        if rule["command_contains"] is not None and rule["command_contains"] not in command:
            continue
        return True
    return False


def _claude_hooks_expected_render(settings):
    """Replicate sync_claude_hooks' in-memory merge (imported primitives only)."""
    config_path = REPO_ROOT / "config" / "claude-hooks.json"
    _settings_path, managed_hooks, removed = _sync_claude_hooks.build_managed_hooks(config_path)
    managed_signatures = _sync_claude_hooks._collect_managed_signatures(managed_hooks)
    existing_hooks = _sync_claude_hooks._strip_removed_signatures(settings.get("hooks", {}), removed)
    merged_hooks = _sync_claude_hooks._strip_managed_hooks(existing_hooks, managed_signatures)
    for event_name, entries in managed_hooks.items():
        merged_hooks.setdefault(event_name, [])
        merged_hooks[event_name].extend(copy.deepcopy(entries))
    merged = copy.deepcopy(settings)
    merged["hooks"] = merged_hooks
    return merged


def _claude_hooks_status(claude_def):
    settings_path, managed_hooks, removed = _sync_claude_hooks.build_managed_hooks(Path(claude_def))
    managed_signatures = _sync_claude_hooks._collect_managed_signatures(managed_hooks)
    target = str(settings_path)
    target_exists = os.path.exists(target)
    observed = {
        "target": target,
        "target_exists": target_exists,
        "managed_hook_count": len(managed_signatures),
        "missing_signatures": 0,
        "removed_signatures_present": 0,
    }
    if not target_exists:
        return observed, "not-materialized", None
    try:
        settings = load_json_object_file(target, strict=True, missing_ok=False)
    except ValueError as exc:
        return observed, "drifted", str(exc)
    present = _collect_present_hook_signatures(settings.get("hooks", {}))
    missing = len(managed_signatures - present)
    removed_present = sum(1 for rule in removed if _removed_rule_present(rule, present))
    observed["missing_signatures"] = missing
    observed["removed_signatures_present"] = removed_present
    if missing or removed_present:
        return observed, "drifted", None
    return observed, "managed-ok", None


def _codex_hooks_status(codex_def):
    hooks_path, managed_hooks = _sync_codex_hooks.build_managed_hooks(Path(codex_def))
    expected = {"hooks": managed_hooks}
    target = str(hooks_path)
    target_exists = os.path.exists(target)
    observed = {
        "target": target,
        "target_exists": target_exists,
        "managed_hook_count": sum(len(entries) for entries in managed_hooks.values()),
    }
    if not target_exists:
        return observed, "not-materialized", None, expected
    try:
        actual = load_json_object_file(target, strict=True, missing_ok=False)
    except ValueError as exc:
        return observed, "drifted", str(exc), expected
    if actual != expected:
        return observed, "drifted", None, expected
    return observed, "managed-ok", None, expected


def build_hooks_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    claude_def = def_files[0]
    codex_def = def_files[1] if len(def_files) > 1 else None
    elements = []

    claude_observed, claude_state, claude_error = _claude_hooks_status(claude_def)
    if claude_error:
        errors.append(
            diagnostic(
                "HOOKS_TARGET_PARSE_ERROR",
                "hooks:claude",
                ".claude/settings.json 解析失败: %s" % claude_error,
            )
        )
    elements.append(
        _make_element(
            "hooks:claude",
            "hooks",
            {"config": "config/claude-hooks.json", "merge": "signature-set"},
            {"enabled": True},
            claude_observed,
            claude_state,
            [HOOKS_APPLY_ACTION],
        )
    )

    if codex_def is not None:
        codex_observed, codex_state, codex_error, _expected = _codex_hooks_status(codex_def)
        if codex_error:
            errors.append(
                diagnostic(
                    "HOOKS_TARGET_PARSE_ERROR",
                    "hooks:codex",
                    ".codex/hooks.json 解析失败: %s" % codex_error,
                )
            )
        elements.append(
            _make_element(
                "hooks:codex",
                "hooks",
                {"config": "config/codex-hooks.json", "merge": "whole-file"},
                {"enabled": True},
                codex_observed,
                codex_state,
                [HOOKS_APPLY_ACTION],
            )
        )
    return {"domain": "hooks", "elements": elements}, warnings, errors


def build_hooks_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_hooks_status(root_dir, config_file, def_files)
    actions = []
    for element in status["elements"]:
        if element["state"] not in ("drifted", "missing", "not-materialized"):
            continue
        target = element["observed"]["target"]
        before = _sha256_path(target) if element["observed"]["target_exists"] else None
        if element["id"] == "hooks:claude":
            settings = {}
            if element["observed"]["target_exists"]:
                settings = load_json_object_file(target, strict=False, missing_ok=True)
            after = _rendered_json_digest(_claude_hooks_expected_render(settings))
            note = "按签名集合合并托管 hooks（保留区块外用户配置）"
        else:
            _observed, _state, _error, expected = _codex_hooks_status(def_files[1])
            after = _rendered_json_digest(expected)
            note = "整体重写 codex hooks.json"
        actions.append(
            plan_action("write-file", element["id"], target, "write-local", before_digest=before, after_digest=after, note=note)
        )
    return plan_data("hooks", actions), warnings, errors


# ---------------------------------------------------------------------------
# aliases domain (managed marker block in ~/.zshrc and ~/.zprofile)
# ---------------------------------------------------------------------------


def _render_alias_lines(cfg):
    """Replica of scripts/claude/sync_shell_aliases.sh's render step."""
    timezone = cfg.get("timezone", "America/Los_Angeles")
    lang = cfg.get("lang", "en_US.UTF-8")
    aliases = cfg.get("aliases", {})
    lines = []
    for name, value in sorted(aliases.items()):
        value = value.replace("{timezone}", timezone).replace("{lang}", lang)
        lines.append("alias %s='%s'" % (name, value))
    return lines


def _extract_alias_block(text):
    """Return (has_block, inner_text); content outside the markers is ignored."""
    inside = False
    found = False
    inner = []
    for line in text.splitlines():
        if line == ALIAS_MARKER_BEGIN:
            inside = True
            found = True
            continue
        if line == ALIAS_MARKER_END:
            inside = False
            continue
        if inside:
            inner.append(line)
    return found, "\n".join(inner)


def build_aliases_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    cfg = load_json_object_file(def_files[0], strict=True, missing_ok=False)
    alias_lines = _render_alias_lines(cfg)
    expected_inner = "\n".join(alias_lines)
    elements = []
    for rc_name, rc_path in (
        ("zshrc", os.path.expanduser("~/.zshrc")),
        ("zprofile", os.path.expanduser("~/.zprofile")),
    ):
        target_exists = os.path.exists(rc_path)
        has_block = False
        matches = False
        if target_exists:
            with open(rc_path, encoding="utf-8") as handle:
                has_block, inner = _extract_alias_block(handle.read())
            matches = has_block and inner == expected_inner
        if not target_exists:
            state = "not-materialized" if alias_lines else "managed-ok"
        elif matches:
            state = "managed-ok"
        elif has_block:
            state = "drifted"
        else:
            state = "missing" if alias_lines else "managed-ok"
        elements.append(
            _make_element(
                "aliases:%s" % rc_name,
                "aliases",
                {"aliases": sorted(cfg.get("aliases", {}).keys()), "marker": "agent-manager shell aliases"},
                {"enabled": True},
                {
                    "target": rc_path,
                    "target_exists": target_exists,
                    "block_present": has_block,
                    "alias_count": len(alias_lines),
                },
                state,
                [ALIAS_APPLY_ACTION],
            )
        )
    return {"domain": "aliases", "elements": elements}, warnings, errors


def build_aliases_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_aliases_status(root_dir, config_file, def_files)
    cfg = load_json_object_file(def_files[0], strict=True, missing_ok=False)
    alias_lines = _render_alias_lines(cfg)
    expected_inner = "\n".join(alias_lines)
    actions = []
    for element in status["elements"]:
        if element["state"] not in ("drifted", "missing", "not-materialized"):
            continue
        target = element["observed"]["target"]
        before = None
        if element["observed"]["target_exists"] and element["observed"]["block_present"]:
            with open(target, encoding="utf-8") as handle:
                _found, inner = _extract_alias_block(handle.read())
            before = _sha256_text(inner)
        actions.append(
            plan_action(
                "write-file",
                element["id"],
                target,
                "write-local",
                before_digest=before,
                after_digest=_sha256_text(expected_inner) if alias_lines else None,
                note="受管区块 digest；marker 区块外内容不动",
            )
        )
    return plan_data("aliases", actions), warnings, errors


# ---------------------------------------------------------------------------
# secrets domain (read-only: repo existence + mcp env placeholder resolution;
# values are never read into any payload — red line)
# ---------------------------------------------------------------------------


def build_secrets_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    secrets_dir = root_dir / "secrets"
    repo_exists = secrets_dir.is_dir()
    is_git = (secrets_dir / ".git").exists()
    env_files = sorted(path.name for path in secrets_dir.glob("*.env")) if repo_exists else []
    if repo_exists and is_git:
        repo_state = "managed-ok"
    elif repo_exists:
        repo_state = "drifted"
    else:
        repo_state = "not-materialized"
    elements = [
        _make_element(
            "secrets:repo",
            "secrets",
            {"path": "secrets/", "kind": "git-subrepo"},
            {"enabled": True},
            {"path": str(secrets_dir), "exists": repo_exists, "is_git": is_git, "env_files": env_files},
            repo_state,
            [SECRETS_PULL_ACTION],
        )
    ]

    # mcp env placeholder resolution summary (same sources as sync_mcp, minus network).
    config_name = Path(config_file).name
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    policy_cfg = get_typed_mapping_section(config, "mcp", config_name, (dict,), "对象")
    def_data = load_json_object_file(def_files[0], strict=True, missing_ok=False)
    servers = get_typed_mapping_section(def_data, "servers", Path(def_files[0]).name, (dict,), "对象")
    secrets_env = load_env_files([root_dir / ".env", secrets_dir / "mcp.env"])
    variables = []
    unresolved = 0
    for server_name in sorted(servers):
        policy = policy_cfg.get(server_name)
        if not isinstance(policy, dict) or not policy.get("enabled", True):
            continue
        merged = dict(servers[server_name])
        merged.update(policy)
        env_dict = merged.get("env", {})
        if not isinstance(env_dict, dict):
            continue
        for env_key, value in env_dict.items():
            var_name = placeholder_name(value)
            if var_name is None:
                continue
            resolved = var_name in secrets_env or var_name in os.environ
            if not resolved:
                unresolved += 1
                warnings.append(
                    diagnostic(
                        "MCP_ENV_UNRESOLVED",
                        "secrets:mcp-env",
                        "server '%s' 的占位符 %s 未解析" % (server_name, var_name),
                    )
                )
            variables.append(
                {
                    "server": server_name,
                    "env_key": env_key,
                    "placeholder": var_name,
                    "resolved": resolved,
                }
            )
    elements.append(
        _make_element(
            "secrets:mcp-env",
            "secrets",
            {"sources": [".env", "secrets/mcp.env", "process env"]},
            {"enabled": True},
            {"variables": variables, "unresolved": unresolved},
            "managed-ok" if unresolved == 0 else "missing",
            [],
        )
    )
    return {"domain": "secrets", "elements": elements}, warnings, errors


# ---------------------------------------------------------------------------
# mcp plan (before digest = current target file; after digest = canonical
# injection section; the full-file render is the apply step's job)
# ---------------------------------------------------------------------------


def build_mcp_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_mcp_status(root_dir, config_file, def_files[0])
    actions = []
    for element in status["elements"]:
        for agent_id, cell in sorted(element["observed"]["agents"].items()):
            cell_state = cell["state"]
            if cell_state in ("managed-ok", "agent-disabled", "absent", "probe-error"):
                continue
            target = cell["target"]
            if cell_state == "leftover":
                actions.append(
                    plan_action(
                        "write-file",
                        element["id"],
                        target,
                        "write-local",
                        before_digest=_sha256_path(target) if cell["target_exists"] else None,
                        after_digest=None,
                        note="移除 agent '%s' 中的托管残留 %s" % (agent_id, element["id"]),
                    )
                )
                continue
            actions.append(
                plan_action(
                    "write-file",
                    element["id"],
                    target,
                    "write-local",
                    before_digest=_sha256_path(target) if cell["target_exists"] else None,
                    after_digest=_sha256_canonical(element["spec"]),
                    note="注入/收敛 %s 到 agent '%s'（before=目标文件整体 digest，after=注入段规范形 digest）"
                    % (element["id"], agent_id),
                )
            )
    return plan_data("mcp", actions), warnings, errors


# ---------------------------------------------------------------------------
# system.audit (thin envelope wrapper over doctor's build_report)
# ---------------------------------------------------------------------------


def build_system_audit(root_dir, config_file, python_cmd="python3"):
    report = build_report(root_dir=root_dir, config_file=config_file, python_cmd=python_cmd)
    warnings = []
    for category, count in sorted(report["summary"]["warning_categories"].items()):
        if count:
            warnings.append(
                diagnostic(category.upper(), None, "doctor 提示项分类 %s: %d 项" % (category, count))
            )
    errors = []
    if report["status"] == "error":
        errors.append(
            diagnostic("AUDIT_ERRORS", None, "doctor 报告 %d 个错误" % report["summary"]["errors"])
        )
    return report, warnings, errors


# ---------------------------------------------------------------------------
# system.refresh-upstream: the only writer of the upstream probe cache
# (proposal section 3.3). Network is allowed here — this is the explicit
# refresh action; status commands never call into this machinery.
# ---------------------------------------------------------------------------


class RefreshUsageError(ValueError):
    """Raised for refresh-time usage errors (e.g. unknown --only element)."""


def _pypi_latest_impl(package):
    sync_tools = _load_sync_tools()
    if sync_tools is None:
        return None
    return sync_tools.fetch_pypi_latest_version(package)


def _default_net():
    """Real probe implementations; tests inject fakes for every key."""
    return {
        "npm_view": lambda package: _npm_view_impl(package),
        "npm_global_version": lambda package: _npm_global_version_impl(package),
        "git_remote_state": lambda repo_dir: probe_git_remote_state(_run_probe_cmd, repo_dir, timeout=15),
        "git_ls_remote": lambda repo, branch: _git_ls_remote_impl(repo, branch),
        "pypi_latest": lambda package: _pypi_latest_impl(package),
        "command_version": lambda bin_name: command_version(bin_name),
        "brew_outdated": lambda package: _brew_outdated_impl(package),
    }


def _npm_view_impl(package):
    ok, out = _run_probe_cmd(
        ["npm", "view", package, "version", "--registry", OFFICIAL_NPM_REGISTRY], timeout=60
    )
    if not ok:
        return None
    line = out.strip().splitlines()
    return line[0].strip() if line and line[0].strip() else None


def _npm_global_version_impl(package):
    ok, out = _run_probe_cmd(["npm", "ls", "-g", package, "--json"], timeout=60)
    if not ok or not out.strip():
        return None
    try:
        data = json.loads(out)
    except ValueError:
        return None
    version = data.get("dependencies", {}).get(package, {}).get("version")
    return version if isinstance(version, str) and version else None


def _git_ls_remote_impl(repo, branch):
    ok, out = _run_probe_cmd(["git", "ls-remote", repo, branch], timeout=30)
    if not ok:
        return None
    lines = out.strip().splitlines()
    if not lines:
        return None
    fields = lines[0].split()
    return fields[0] if fields and fields[0] else None


def _brew_outdated_impl(package):
    ok, out = _run_probe_cmd(["brew", "outdated", package], timeout=30)
    if not ok:
        return None
    # Empty output = not outdated = current; a line means a newer version exists.
    return "" if not out.strip() else out.strip().splitlines()[0]


def _probe_target(target, net):
    """Run one probe-matrix row; failures become unknown+error (first-class)."""
    kind = target["kind"]
    entry = {"state": "unknown", "current": None, "upstream": None, "probe": None, "error": None}
    if kind == "npm":
        current = _extract_version_token(net["command_version"](target["bin"]) or "") if target.get("bin") else None
        upstream = net["npm_view"](target["package"])
        entry.update(current=current, upstream=upstream, probe="npm-view")
        if upstream is None:
            entry["error"] = "npm view %s 失败" % target["package"]
        else:
            entry["state"] = _compare_freshness(current, upstream)
            if current is None:
                entry["error"] = "本地版本未探测到（bin 不可用）"
    elif kind == "npm-tool":
        current = net["npm_global_version"](target["package"])
        upstream = net["npm_view"](target["package"])
        entry.update(current=current, upstream=upstream, probe="npm-view")
        if upstream is None:
            entry["error"] = "npm view %s 失败" % target["package"]
        else:
            entry["state"] = _compare_freshness(current, upstream)
    elif kind == "npm-npx":
        current = _npx_cached_version(target["package"])
        upstream = net["npm_view"](target["package"])
        entry.update(current=current, upstream=upstream, probe="npm-view")
        if upstream is None:
            entry["error"] = "npm view %s 失败" % target["package"]
        elif current is None:
            entry["state"] = "unknown"
            entry["error"] = "npx 缓存中未找到已解析版本"
        else:
            entry["state"] = _compare_freshness(current, upstream)
    elif kind == "git-ls-remote":
        upstream = net["git_ls_remote"](target["repo"], target["branch"])
        entry.update(upstream=upstream, probe="git-ls-remote")
        if upstream is None:
            entry["error"] = "git ls-remote %s %s 失败" % (target["repo"], target["branch"])
        else:
            # uvx 本地缓存 commit 暂不可读（后续 slice）；上游值仍入库供展示。
            entry["state"] = "unknown"
            entry["error"] = "uvx 本地缓存 commit 探测未实现"
    elif kind == "git-repo":
        result = net["git_remote_state"](target["repo_dir"])
        entry.update(current=result.get("local_head"), upstream=result.get("remote_head"), probe="git-ls-remote")
        if result.get("status") == GIT_REMOTE_STATE_LATEST:
            entry["state"] = "current"
        elif result.get("status") == GIT_REMOTE_STATE_STALE:
            entry["state"] = "behind"
        else:
            entry["error"] = "git 远端探测失败（本地仓库缺失或 ls-remote 失败）"
    elif kind == "pypi":
        upstream = net["pypi_latest"](target["package"])
        entry.update(upstream=upstream, probe="pypi-json")
        if upstream is None:
            entry["error"] = "pypi %s 探测失败" % target["package"]
        else:
            entry["state"] = _compare_freshness(entry["current"], upstream)
    elif kind == "brew":
        outdated = net["brew_outdated"](target["package"])
        entry.update(probe="brew-outdated")
        if outdated is None:
            entry["error"] = "brew outdated %s 失败" % target["package"]
        else:
            entry["state"] = "current" if outdated == "" else "behind"
            entry["upstream"] = outdated or None
    return entry


def _collect_refresh_targets(root_dir, config_file):
    """The probeable element universe: enabled agents/tools/MCP servers with a
    recognizable upstream source, plus enabled skill collection repos."""
    root_dir = Path(root_dir)
    config_name = Path(config_file).name
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    agents_policy = canonicalize_agent_mapping(
        get_typed_mapping_section(config, "agents", config_name, (dict,), "对象")
    )
    tools_policy = get_typed_mapping_section(config, "tools", config_name, (dict,), "对象")
    mcp_policy = get_typed_mapping_section(config, "mcp", config_name, (dict,), "对象")
    skills_policy = get_typed_mapping_section(config, "skills", config_name, (dict,), "对象")

    targets = []
    agent_defs = canonicalize_agent_mapping(
        load_json_object_file(root_dir / "config" / "agents.json", strict=True, missing_ok=False)
    )
    for agent_id in sorted(agent_defs):
        policy = agents_policy.get(agent_id)
        if not isinstance(policy, dict) or not policy.get("enabled", True):
            continue
        probe = _classify_agent_probe(agent_defs[agent_id], policy)
        if probe is None:
            continue
        targets.append(
            {"id": "agents:%s" % agent_id, "kind": "npm", "package": probe[1], "bin": agent_defs[agent_id].get("bin")}
        )

    tool_defs = get_typed_mapping_section(
        load_json_object_file(root_dir / "config" / "tools.json", strict=True, missing_ok=False),
        "tools", "tools.json", (dict,), "对象",
    )
    for name in sorted(tool_defs):
        policy = tools_policy.get(name)
        if not isinstance(policy, dict) or not policy.get("enabled", True):
            continue
        probe = _classify_tool_probe(root_dir, name, tool_defs[name])
        if probe is None:
            continue
        kind, value = probe
        if kind == "git-repo":
            targets.append({"id": "tools:%s" % name, "kind": "git-repo", "repo_dir": value})
        elif kind == "npm":
            targets.append({"id": "tools:%s" % name, "kind": "npm-tool", "package": value})
        elif kind == "pypi":
            targets.append({"id": "tools:%s" % name, "kind": "pypi", "package": value})
        elif kind == "brew":
            targets.append({"id": "tools:%s" % name, "kind": "brew", "package": value})

    mcp_defs = get_typed_mapping_section(
        load_json_object_file(root_dir / "config" / "mcp.json", strict=True, missing_ok=False),
        "servers", "mcp.json", (dict,), "对象",
    )
    for server_name in sorted(mcp_defs):
        policy = mcp_policy.get(server_name)
        if not isinstance(policy, dict) or not policy.get("enabled", True):
            continue
        merged = dict(mcp_defs[server_name])
        merged.update(policy)
        probe = _classify_mcp_probe(merged)
        if probe is None:
            continue
        if probe[0] == "npm-npx":
            targets.append({"id": "mcp:%s" % server_name, "kind": "npm-npx", "package": probe[1]})
        else:
            targets.append(
                {"id": "mcp:%s" % server_name, "kind": "git-ls-remote", "repo": probe[1], "branch": probe[2]}
            )

    skill_defs = get_typed_mapping_section(
        load_json_object_file(root_dir / "config" / "skills.json", strict=True, missing_ok=False),
        "repositories", "skills.json", (dict,), "对象",
    )
    for repo_name in sorted(skill_defs):
        policy = skills_policy.get(repo_name)
        if not isinstance(policy, dict) or not policy.get("enabled", True):
            continue
        targets.append(
            {
                "id": "skill:%s" % repo_name,
                "kind": "git-repo",
                "repo_dir": str(root_dir / "agent-skill" / "repositories" / repo_name),
            }
        )
    return targets


def build_system_refresh_upstream(root_dir, config_file, only=None, net=None):
    """Probe every upstream source and write the cache (explicit, networked)."""
    net = net or _default_net()
    targets = _collect_refresh_targets(root_dir, config_file)
    warnings = []
    if _SYNC_TOOLS_UNAVAILABLE is not None:
        warnings.append(
            diagnostic(
                "UPSTREAM_PROBE_EXTRACTORS_UNAVAILABLE",
                None,
                "agent-tool/sync_tools.py 不可导入（%s），npm/pypi/brew 探测源降级" % _SYNC_TOOLS_UNAVAILABLE,
            )
        )
    if only is not None:
        targets = [target for target in targets if target["id"] == only]
        if not targets:
            raise RefreshUsageError("--only 指定的元素不可探测或不存在: %s" % only)

    moment = upstream_cache.now_utc()
    checked_at = upstream_cache.to_iso(moment)
    updates = {}
    results = []
    counts = {"current": 0, "behind": 0, "ahead": 0, "unknown": 0}
    for target in targets:
        try:
            entry = _probe_target(target, net)
        except Exception as exc:
            # 单元素探测异常不拖垮整批：unknown 是一等状态（提案 §3.3）。
            entry = {"state": "unknown", "current": None, "upstream": None, "probe": None, "error": str(exc)}
        entry["checked_at"] = checked_at
        updates[target["id"]] = {key: value for key, value in entry.items() if value is not None}
        counts[entry["state"]] = counts.get(entry["state"], 0) + 1
        if entry["state"] == "unknown":
            warnings.append(
                diagnostic(
                    "UPSTREAM_PROBE_UNKNOWN",
                    target["id"],
                    entry["error"] or "上游探测失败",
                )
            )
        results.append(
            {
                "id": target["id"],
                "probe": entry["probe"],
                "state": entry["state"],
                "current": entry["current"],
                "upstream": entry["upstream"],
                "error": entry["error"],
            }
        )
    upstream_cache.write_entries(updates)
    data = {
        "domain": "system",
        "verb": "refresh-upstream",
        "cache_file": str(upstream_cache.cache_path()),
        "elements": results,
        "summary": {
            "probed": len(results),
            "current": counts.get("current", 0),
            "behind": counts.get("behind", 0),
            "ahead": counts.get("ahead", 0),
            "unknown": counts.get("unknown", 0),
        },
    }
    return data, warnings, []


# ---------------------------------------------------------------------------
# skills domain (proposal section 7.1: per-skill x per-agent matrix; freshness
# rides the collection element). Cell vocabulary: linked / missing-link /
# dead-link / drifted / extra-link / disabled; the element axis aggregates to
# the shared seven states.
# ---------------------------------------------------------------------------

_SKILL_DISCOVERY_EXCLUDED_DIRS = {".git", "node_modules", "__pycache__", "venv", ".venv"}


def _skill_cell_state(skill_name, agent_id, expected_by_agent, actual_by_agent, broken_by_agent, repo_base):
    expected = expected_by_agent.get(agent_id, {})
    actual = actual_by_agent.get(agent_id, {})
    broken = broken_by_agent.get(agent_id, set())
    if skill_name in expected:
        if skill_name not in actual:
            return "missing-link"
        if skill_name in broken:
            return "dead-link"
        if _paths_match(actual[skill_name], expected[skill_name]):
            return "linked"
        return "drifted"
    if skill_name not in actual:
        return "disabled"
    if skill_name in broken:
        # 只有指向托管缓存仓库的死链才算我们的清理对象。
        return "dead-link" if is_managed_skill_target_path(actual[skill_name], repo_base) else "disabled"
    if is_managed_skill_target_path(actual[skill_name], repo_base):
        return "extra-link"
    return "disabled"


def _aggregate_skill_state(cell_states):
    if any(state in ("dead-link", "drifted", "extra-link") for state in cell_states):
        return "drifted"
    if "missing-link" in cell_states:
        return "missing"
    if cell_states and all(state == "disabled" for state in cell_states):
        return "disabled"
    if "linked" in cell_states:
        return "managed-ok"
    return "disabled"


def build_skills_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    config = load_json_object_file(config_file, strict=True, missing_ok=False)
    config_name = Path(config_file).name
    skills_policy = get_typed_mapping_section(config, "skills", config_name, (dict,), "对象")
    def_data = load_json_object_file(def_files[0], strict=True, missing_ok=False)
    def_name = Path(def_files[0]).name
    repositories = get_typed_mapping_section(def_data, "repositories", def_name, (dict,), "对象")
    agent_targets = canonicalize_agent_mapping(
        get_typed_mapping_section(def_data, "agents", def_name, (str, dict), "字符串或对象")
    )
    skill_cfg = {"repositories": repositories, "agents": agent_targets}
    repo_base = (Path(root_dir) / "agent-skill" / "repositories").resolve()
    declared_target_ids = set(agent_targets)
    cache = upstream_cache.load_cache()

    # Per-agent expected/actual once — doctor 的矩阵感知语义原样复用。
    expected_by_agent = {}
    actual_by_agent = {}
    broken_by_agent = {}
    target_path_by_agent = {}
    target_exists_by_agent = {}
    for agent_id, target_cfg in agent_targets.items():
        target_path = target_cfg if isinstance(target_cfg, str) else target_cfg.get("path", "")
        target_path = os.path.expanduser(target_path)
        target_path_by_agent[agent_id] = target_path
        target_exists_by_agent[agent_id] = os.path.isdir(target_path)
        expected, _problems = expected_managed_skill_targets(Path(root_dir), config, skill_cfg, agent_id)
        expected_by_agent[agent_id] = expected
        if target_exists_by_agent[agent_id]:
            actual, broken = actual_skill_targets(target_path)
        else:
            actual, broken = {}, set()
        actual_by_agent[agent_id] = actual
        broken_by_agent[agent_id] = broken

    inbox = collect_skill_discovery_inbox(Path(root_dir), config, skill_cfg)

    # 额外受管链接：未被任何启用集合期望、但指向某集合的托管缓存仓库
    # （doctor 的「存在额外受管 skill 链接」语义），挂在属主集合元素上。
    extras_by_repo = {}
    for agent_id in sorted(agent_targets):
        expected_names = set(expected_by_agent.get(agent_id, {}))
        for name, target in sorted(actual_by_agent.get(agent_id, {}).items()):
            if name in expected_names or not is_managed_skill_target_path(target, repo_base):
                continue
            for candidate in repositories:
                if is_managed_skill_target_path(target, (repo_base / candidate).resolve()):
                    extras_by_repo.setdefault(candidate, []).append(
                        {"agent": agent_id, "name": name, "link_path": os.path.join(target_path_by_agent[agent_id], name)}
                    )
                    break

    elements = []
    seen_skill_names = {}
    for repo_name in sorted(repositories):
        definition = repositories[repo_name]
        policy = skills_policy.get(repo_name)
        declared = isinstance(policy, dict)
        enabled = declared and bool(policy.get("enabled", True))
        merged = dict(definition)
        if declared:
            merged.update(policy)
        matrix_mode = repo_skill_matrix_mode(merged)
        targeted_agents = canonicalize_agent_list(merged.get("agents"))
        repo_path = repo_base / repo_name
        repo_cached = repo_path.exists()
        owner_marked = _link_skills.is_manager_owned_repo(repo_path) if repo_cached else False

        # Collection 级 spec/policy 校验与发现（与 doctor 的问题通道同语义，但不依赖 agent 维度）。
        discovered = {}
        config_error = None
        if enabled and repo_cached:
            try:
                validate_repo_new_skill_default(repo_name, merged)
                if matrix_mode:
                    validate_repo_skill_matrix(repo_name, merged, targeted_agents, declared_target_ids)
                search_paths = resolve_skill_search_paths(repo_path.resolve(), merged)
                discovered, duplicate_names = _discover_repo_skill_targets(
                    search_paths, _SKILL_DISCOVERY_EXCLUDED_DIRS
                )
                if duplicate_names:
                    raise ValueError("重复 skill name: %s" % ", ".join(sorted(duplicate_names)))
                if matrix_mode:
                    resolve_repo_skill_agent_overrides(repo_name, merged, set(discovered))
            except ValueError as exc:
                config_error = str(exc)
                errors.append(
                    diagnostic("SKILL_CONFIG_INVALID", "skill:%s" % repo_name, config_error)
                )
            for skill_name in discovered:
                # 跨集合重名 = spec 层冲突（doctor 的「重复 skill name」问题通道）。
                first_repo = seen_skill_names.get(skill_name)
                if first_repo is not None and first_repo != repo_name:
                    errors.append(
                        diagnostic(
                            "SKILL_DUPLICATE_NAME",
                            "skill:%s/%s" % (repo_name, skill_name),
                            "skill 名 '%s' 同时由集合 %s 与 %s 提供" % (skill_name, first_repo, repo_name),
                        )
                    )
                else:
                    seen_skill_names[skill_name] = repo_name

        if not enabled:
            collection_state = "disabled"
        elif not repo_cached:
            collection_state = "not-materialized"
            warnings.append(
                diagnostic(
                    "SKILL_REPO_CACHE_MISSING",
                    "skill:%s" % repo_name,
                    "集合仓库缓存缺失（需 skills:apply / update 拉取）",
                )
            )
        elif config_error is not None:
            collection_state = "unsupported"
        else:
            collection_state = "managed-ok"
        extra_links = extras_by_repo.get(repo_name, [])
        if extra_links and collection_state in ("managed-ok", "disabled"):
            collection_state = "drifted"

        freshness = _freshness_from_cache(
            "skill:%s" % repo_name,
            probeable=enabled,
            cache=cache,
            local_current=_local_git_head(str(repo_path)) if repo_cached else None,
            warnings=warnings,
        )
        elements.append(
            _make_element(
                "skill:%s" % repo_name,
                "skills",
                {
                    "url": definition.get("url"),
                    "skill_paths": merged.get("skill_paths"),
                    "agents": targeted_agents,
                    "matrix_mode": matrix_mode,
                    "new_skill_default": merged.get("new_skill_default", "disabled"),
                },
                {"enabled": enabled, "declared_in_policy": declared},
                {
                    "repo_cache": str(repo_path),
                    "repo_cached": repo_cached,
                    "owner_marked": owner_marked,
                    "discovered": sorted(discovered),
                    "discovery_inbox": inbox.get(repo_name, []),
                    "extra_links": extra_links,
                },
                collection_state,
                [SKILL_REPO_PULL_ACTION] if enabled else [],
                freshness=freshness,
            )
        )

        if not (enabled and repo_cached) or config_error is not None:
            continue
        in_scope_agents = [
            agent_id
            for agent_id in sorted(agent_targets)
            if targeted_agents is None or agent_id in targeted_agents
        ]
        for skill_name in sorted(discovered):
            cells = {}
            for agent_id in in_scope_agents:
                cells[agent_id] = {
                    "state": _skill_cell_state(
                        skill_name, agent_id, expected_by_agent, actual_by_agent, broken_by_agent, repo_base
                    ),
                    "link_path": os.path.join(target_path_by_agent[agent_id], skill_name),
                    "target_dir_exists": target_exists_by_agent[agent_id],
                }
            cell_states = [cell["state"] for cell in cells.values()]
            state = _aggregate_skill_state(cell_states)
            # 全部启用格都缺链且所有目标目录尚未建立 → 提示级 not-materialized。
            if (
                state == "missing"
                and cells
                and all(
                    cell["state"] in ("missing-link", "disabled") for cell in cells.values()
                )
                and not any(cell["target_dir_exists"] for cell in cells.values())
            ):
                state = "not-materialized"
            in_inbox = skill_name in inbox.get(repo_name, [])
            if in_inbox:
                state = "disabled"
            elements.append(
                _make_element(
                    "skill:%s/%s" % (repo_name, skill_name),
                    "skills",
                    {"collection": repo_name, "name": skill_name, "source_path": discovered[skill_name]},
                    {
                        "enabled": any(cell["state"] != "disabled" for cell in cells.values()),
                        "agents": {
                            agent_id: skill_name in expected_by_agent.get(agent_id, {})
                            for agent_id in in_scope_agents
                        },
                    },
                    {
                        "agents": cells,
                        "discovery_inbox": in_inbox,
                    },
                    state,
                    [SKILL_APPLY_ACTION],
                )
            )
    return {"domain": "skills", "elements": elements}, warnings, errors


def build_skills_plan(root_dir, config_file, def_files):
    status, warnings, errors = build_skills_status(root_dir, config_file, def_files)
    actions = []
    # 集合仓库动作先行：链接目标都在缓存仓库里。
    for element in status["elements"]:
        if "/" in element["id"]:
            continue
        if element["state"] == "not-materialized":
            actions.append(
                plan_action(
                    "git-pull",
                    element["id"],
                    element["observed"]["repo_cache"],
                    "network",
                    note="拉取/克隆集合仓库缓存（url: %s）" % (element["spec"].get("url") or "?"),
                )
            )
        elif element["freshness"]["state"] == "behind":
            actions.append(
                plan_action(
                    "git-pull",
                    element["id"],
                    element["observed"]["repo_cache"],
                    "network",
                    note="集合仓库落后上游（%s → %s）"
                    % ((element["freshness"]["current"] or "?")[:12], (element["freshness"]["upstream"] or "?")[:12]),
                )
            )
        for extra in element["observed"].get("extra_links", []):
            actions.append(
                plan_action(
                    "unlink",
                    element["id"],
                    extra["link_path"],
                    "exec-local",
                    note="移除额外受管链接 %s@%s（策略未启用）" % (extra["name"], extra["agent"]),
                )
            )
    for element in status["elements"]:
        if "/" not in element["id"]:
            continue
        source_path = element["spec"]["source_path"]
        for agent_id, cell in sorted(element["observed"]["agents"].items()):
            cell_state = cell["state"]
            link_path = cell["link_path"]
            if cell_state == "missing-link":
                actions.append(
                    plan_action(
                        "symlink",
                        element["id"],
                        link_path,
                        "exec-local",
                        note="启用并链接到 %s" % source_path,
                    )
                )
            elif cell_state == "dead-link":
                actions.append(
                    plan_action(
                        "unlink",
                        element["id"],
                        link_path,
                        "exec-local",
                        note="死链清理（链接目标已不存在）",
                    )
                )
            elif cell_state == "extra-link":
                actions.append(
                    plan_action(
                        "unlink",
                        element["id"],
                        link_path,
                        "exec-local",
                        note="移除额外受管链接（策略未启用 %s@%s）" % (element["spec"]["name"], agent_id),
                    )
                )
            elif cell_state == "drifted":
                actions.append(
                    plan_action(
                        "symlink",
                        element["id"],
                        link_path,
                        "exec-local",
                        note="链接重定向到 %s" % source_path,
                    )
                )
    return plan_data("skills", actions), warnings, errors


# Kernel registry. Every kernel takes (root_dir, config_file, def_files) and
# returns (data, warnings, errors). profile/policy/runs land in later phases.
STATUS_KERNELS = {
    "agents": build_agents_status,
    "aliases": build_aliases_status,
    "hooks": build_hooks_status,
    "mcp": lambda root_dir, config_file, def_files: build_mcp_status(root_dir, config_file, def_files[0]),
    "rules": build_rules_status,
    "secrets": build_secrets_status,
    "skills": build_skills_status,
    "tools": build_tools_status,
}
PLAN_KERNELS = {
    "agents": build_agents_plan,
    "aliases": build_aliases_plan,
    "hooks": build_hooks_plan,
    "mcp": build_mcp_plan,
    "rules": build_rules_plan,
    "skills": build_skills_plan,
    "tools": build_tools_plan,
}


def _build_parser():
    parser = argparse.ArgumentParser(
        prog="engine_v1.py",
        description="Agent Manager engine (contract v1): '<domain>:<verb>' with envelope JSON on stdout.",
    )
    parser.add_argument("command", nargs="?", help="e.g. 'mcp:status' (dot separator accepted)")
    parser.add_argument("--config", help="merged policy config (manager.json + overlays)")
    parser.add_argument(
        "--def-file",
        action="append",
        help="domain definition file; repeatable (hooks takes claude-hooks.json then codex-hooks.json)",
    )
    parser.add_argument("--python-cmd", default="python3", help="python command for system.audit probes")
    parser.add_argument("--only", help="system:refresh-upstream 只刷新指定元素 id")
    parser.add_argument("--json", action="store_true", help="required: selects the envelope machine interface")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--offline", action="store_true")
    return parser


def main(argv=None):
    run_id = os.environ.get("AGENT_MANAGER_RUN_ID") or generate_run_id()
    log_file = os.environ.get("AGENT_MANAGER_LOG_FILE") or None
    lock_held = os.environ.get("AGENT_MANAGER_LOCK_HELD") == "true"
    lock_pid_raw = os.environ.get("AGENT_MANAGER_LOCK_PID") or ""
    if lock_held:
        lock = {"held": True, "pid": int(lock_pid_raw) if lock_pid_raw.isdigit() else os.getpid()}
    else:
        lock = {"held": False, "pid": None}
    mode = {"dry_run": False, "offline": False}

    parser = _build_parser()
    try:
        args, leftover = parser.parse_known_args(argv)
    except SystemExit:
        # argparse already printed a human hint to stderr; still honor the contract.
        payload = envelope(
            command="unknown",
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic("USAGE_ERROR", None, "参数解析失败；期望 <domain>:<verb> --config <path> --json")],
        )
        emit_envelope(payload)
        return 2

    mode["dry_run"] = bool(args.dry_run)
    mode["offline"] = bool(args.offline)
    raw_command = (args.command or "").strip()
    dotted = raw_command.replace(":", ".", 1)

    def usage_error(code, message, hint):
        payload = envelope(
            command=dotted or "unknown",
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic(code, None, message)],
        )
        emit_envelope(payload)
        print(hint, file=sys.stderr)
        return 2

    if not raw_command:
        return usage_error(
            "USAGE_ERROR",
            "缺少命令；期望 <domain>:status 形式（如 mcp:status）",
            "用法: engine_v1.py <domain>:<verb> --config <merged-config> [--def-file <path>] --json [--dry-run] [--offline]",
        )
    if not args.json:
        return usage_error(
            "JSON_REQUIRED",
            "命令 '%s' 是机器接口，必须显式追加 --json" % dotted,
            "错误: 引擎命令只输出 envelope JSON；请追加 --json（人类可读渲染由 GUI/渲染层负责）",
        )
    if leftover:
        return usage_error(
            "USAGE_ERROR",
            "无法识别的参数: %s" % " ".join(leftover),
            "错误: 无法识别的参数 '%s'" % " ".join(leftover),
        )
    if not args.config:
        return usage_error(
            "USAGE_ERROR",
            "缺少 --config <merged-config>（三层合并后的策略文件）",
            "错误: 缺少 --config 参数",
        )

    domain, _, verb = dotted.partition(".")
    if not domain or not verb:
        return usage_error(
            "USAGE_ERROR",
            "命令 '%s' 不是 <domain>:<verb> 形式" % raw_command,
            "错误: 命令须为 <domain>:<verb> 形式（如 mcp:status）",
        )
    if domain not in KNOWN_DOMAINS:
        return usage_error(
            "UNKNOWN_DOMAIN",
            "未知域 '%s'（已知域: %s）" % (domain, ", ".join(KNOWN_DOMAINS)),
            "错误: 未知域 '%s'；当前已接线域: %s" % (domain, ", ".join(sorted(STATUS_KERNELS))),
        )

    def not_implemented():
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[
                diagnostic(
                    "DOMAIN_NOT_IMPLEMENTED",
                    None,
                    "命令 '%s' 属于契约命令面，但在本切片尚未实现" % dotted,
                )
            ],
        )
        emit_envelope(payload)
        print(
            "提示: '%s' 尚未实现（后续切片落地）；当前可用域: %s（外加 system:audit / system:refresh-upstream）"
            % (dotted, ", ".join(sorted(STATUS_KERNELS))),
            file=sys.stderr,
        )
        return 2

    kernel = None
    kernel_needs_def_file = True
    if verb == "status" and domain in STATUS_KERNELS:
        kernel = STATUS_KERNELS[domain]
    elif verb == "plan" and domain in PLAN_KERNELS:
        kernel = PLAN_KERNELS[domain]
    elif domain == "system" and verb == "audit":
        kernel = "audit"
        kernel_needs_def_file = False
    elif domain == "system" and verb == "refresh-upstream":
        kernel = "refresh"
        kernel_needs_def_file = False
    else:
        return not_implemented()

    if kernel_needs_def_file and not args.def_file:
        return usage_error(
            "USAGE_ERROR",
            "%s 需要 --def-file（域定义文件）" % dotted,
            "错误: 缺少 --def-file 参数",
        )
    if domain == "hooks" and kernel_needs_def_file and len(args.def_file) < 2:
        return usage_error(
            "USAGE_ERROR",
            "hooks 域需要两个 --def-file（claude-hooks.json 与 codex-hooks.json）",
            "错误: hooks 域需要重复两次 --def-file",
        )

    if kernel == "refresh" and mode["offline"]:
        # network 级动作在 offline 下显式拒绝（提案 §5 控制原则 3）。
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[
                diagnostic(
                    "OFFLINE_NETWORK_DENIED",
                    None,
                    "system:refresh-upstream 需要网络探测，offline 模式下拒绝执行",
                )
            ],
        )
        emit_envelope(payload)
        print("错误: offline 模式下禁止上游探测（network 动作）", file=sys.stderr)
        return 1

    try:
        if kernel == "audit":
            data, warnings, errors = build_system_audit(REPO_ROOT, args.config, python_cmd=args.python_cmd)
        elif kernel == "refresh":
            data, warnings, errors = build_system_refresh_upstream(REPO_ROOT, args.config, only=args.only)
        else:
            data, warnings, errors = kernel(REPO_ROOT, args.config, args.def_file)
    except RefreshUsageError as exc:
        return usage_error("USAGE_ERROR", str(exc), "错误: %s" % exc)
    except ValueError as exc:
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic("CONFIG_INVALID", None, str(exc))],
        )
        emit_envelope(payload)
        print("错误: %s" % exc, file=sys.stderr)
        return 1

    payload = envelope(
        command=dotted,
        mode=mode,
        lock=lock,
        run_id=run_id,
        log_file=log_file,
        data=data,
        warnings=warnings,
        errors=errors,
    )
    emit_envelope(payload)
    if errors:
        print("错误: %s 完成但含 %d 个错误" % (dotted, len(errors)), file=sys.stderr)
        return 1
    for warning in warnings:
        print("警告: [%s] %s" % (warning["code"], warning["message"]), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
