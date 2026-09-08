#!/usr/bin/env python3
"""Engine Contract v1 envelope infrastructure and domain kernels.

Contract source of truth: contract/engine-v1/*.schema.json (see contract/README.md)
and docs/GUI-ARCHITECTURE-PROPOSAL.md section 2 (envelope, exit codes) and
section 3 (dual-axis element model: convergence 'state' + upstream 'freshness').

Slices so far: status+plan for agents/mcp/tools/rules/hooks/aliases/skills,
read-only secrets status, system.audit (thin wrapper over doctor's
build_report), system.refresh-upstream (only writer of src/upstream_cache.py),
and the Phase 2a write path: '<domain>:apply' (delegating to the existing
syncer scripts, pre-write snapshots via src/engine_apply.py),
system.converge (update-pipeline equivalent), snapshots:list/restore,
runs:list/get, and policy.get/set (three-layer stack via src/policy_layers.py).
profile lands in a later phase.

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
import ntpath
import os
import re
import stat
import sys
from getpass import getuser
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
    build_report,
    build_tree_signature,
    collect_skill_discovery_inbox,
    command_exists,
    command_version,
    expected_managed_mcp_servers,
    expected_managed_skill_targets,
    get_unsupported_actions,
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
from src import engine_apply, policy_layers, source_definitions, storage, upstream_cache

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
    "snapshots",
    "sources",
    "db",
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
# 只读复用构建器的纯渲染函数（TARGET_SPECS/compact_lines/render_target_content）；
# 其模块级 ROOT 常量不参与引擎调用（引擎按自身 root 读 src）。
_build_rules = _load_module_from_path("agent-rule/scripts/build-agent-rules.py", "agent_manager_build_rules")

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
    # 定义覆盖层：merged 策略里的 mcp.definitions 片段深合并到 mcp.json 之上。
    servers, overlay_warnings = policy_layers.apply_definition_overlays(
        servers, policy_layers.overlay_definitions(config, "mcp"), domain="mcp"
    )
    for overlay_warning in overlay_warnings:
        warnings.append(diagnostic("DEFINITION_OVERLAY_UNKNOWN_KEYS", None, overlay_warning))
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

    all_server_names = set(servers)
    for probe in probes.values():
        all_server_names.update(probe["servers"])

    elements = []
    for server_name in sorted(all_server_names):
        is_defined = server_name in servers
        definition = servers[server_name] if is_defined else {}
        policy = policy_cfg.get(server_name) if is_defined else None
        declared = isinstance(policy, dict) if is_defined else False
        enabled = declared and bool(policy.get("enabled", True)) if is_defined else False
        merged = dict(definition)
        if declared:
            merged.update(policy)
        targeted_agents = canonicalize_agent_list(merged.get("agents")) if is_defined else None
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

        if not is_defined:
            has_leftover = any(cell["state"] == "leftover" for cell in cells.values())
            if not has_leftover:
                continue
            state = "drifted"
        elif not enabled:
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
        if is_defined:
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
        else:
            freshness = dict(FRESHNESS_NOT_APPLICABLE)
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


def _agent_available_actions(agent_id, definition, policy, state):
    """按真实安装状态/唯一受管来源/当前平台有效命令产生显式动作。

    可用性判定复用 sync_agents.resolve_explicit_action 同一解析规则（可执行才展示）：
    新定义未声明 policy 但有有效 install 命令时仍提供安装；已停用/未安装可安装；
    升级/卸载仅当唯一已检测来源受管且该来源有当前平台有效命令；缺卸载命令不显示卸载。
    state 复用 status 本次探测结果，不二次探测。
    """
    effective = dict(definition)
    preferred = (policy or {}).get("preferred_provider")
    if preferred:
        effective["preferred_provider"] = preferred
    sync_agents = _load_sync_agents()
    actions = []
    for action_name, template in (
        ("install", AGENT_INSTALL_ACTION),
        ("upgrade", AGENT_UPGRADE_ACTION),
        ("uninstall", AGENT_UNINSTALL_ACTION),
    ):
        try:
            sync_agents.resolve_explicit_action(agent_id, effective, action_name, state=state)
        except ValueError:
            continue
        actions.append(dict(template))
    return actions


def build_agents_status(root_dir, config_file, def_files):
    warnings = []
    errors = []
    diagnostics = {"warnings": warnings, "errors": errors}
    config, agents_policy = _load_policy_section(config_file, "agents")
    agent_defs = canonicalize_agent_mapping(
        load_json_object_file(def_files[0], strict=True, missing_ok=False)
    )
    agent_defs, overlay_warnings = policy_layers.apply_definition_overlays(agent_defs, policy_layers.overlay_definitions(config, "agents"), domain="agents")
    warnings.extend(diagnostic("DEFINITION_OVERLAY_UNKNOWN_KEYS", None, item) for item in overlay_warnings)
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
                _agent_available_actions(agent_id, definition, policy, {"all": providers, **classified}),
                freshness=freshness,
            )
        )
    return {"domain": "agents", "elements": elements}, warnings, errors


_SYNC_AGENTS_MODULE = None


def _load_sync_agents():
    """惰性加载 agent-lifecycle/sync_agents.py（目录名含连字符，不能常规 import）。

    显式生命周期动作的解析规则（resolve_explicit_action）由 sync_agents 持有，
    引擎 plan 与 apply 复用同一规则，避免两处语义漂移。
    """
    global _SYNC_AGENTS_MODULE
    if _SYNC_AGENTS_MODULE is None:
        spec = importlib.util.spec_from_file_location(
            "sync_agents", REPO_ROOT / "agent-lifecycle" / "sync_agents.py"
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _SYNC_AGENTS_MODULE = module
    return _SYNC_AGENTS_MODULE


_EXPLICIT_ACTION_PLAN_OPS = {"install": "install", "upgrade": "run-cmd", "uninstall": "uninstall"}


def _build_agents_explicit_plan(root_dir, config_file, def_files, only, action):
    """agents:plan --action install|upgrade|uninstall：解析真实命令，不写策略/DB。"""
    status, warnings, errors = build_agents_status(root_dir, config_file, def_files)
    if errors:
        return plan_data("agents", []), warnings, errors
    element_id = only[0]
    element = next((item for item in status["elements"] if item["id"] == element_id), None)
    if element is None:
        return plan_data("agents", []), warnings, [
            diagnostic("USAGE_ERROR", None, "未知元素 '%s'" % element_id)
        ]
    agent_id = element_id.split(":", 1)[1]
    definition = dict(element["spec"])
    # effective 策略的默认安装方式优先于定义默认值
    policy_preferred = element["policy"].get("preferred_provider")
    if policy_preferred:
        definition["preferred_provider"] = policy_preferred
    sync_agents = _load_sync_agents()
    plan_state = {"all": element["observed"]["providers"], **element["observed"]["classified"]}
    try:
        provider, command = sync_agents.resolve_explicit_action(agent_id, definition, action, state=plan_state)
    except ValueError as exc:
        return plan_data("agents", []), warnings, [
            diagnostic("AGENT_ACTION_UNAVAILABLE", element_id, str(exc))
        ]
    risk = "network" if action == "upgrade" else "install-remove"
    note = {
        "install": "显式安装（provider: %s）" % provider,
        "upgrade": "显式升级（沿用实际来源: %s）" % provider,
        "uninstall": "显式卸载（provider: %s）" % provider,
    }[action]
    actions = [plan_action(_EXPLICIT_ACTION_PLAN_OPS[action], element_id, None, risk, cmd=command, note=note)]
    return plan_data("agents", actions), warnings, errors


def build_agents_plan(root_dir, config_file, def_files, only=None, agent=None, action=None):
    if action is not None:
        return _build_agents_explicit_plan(root_dir, config_file, def_files, only, action)
    status, warnings, errors = build_agents_status(root_dir, config_file, def_files)
    agent_defs = {element["id"].split(":", 1)[1]: element["spec"] for element in status["elements"]}
    actions = []
    for element in status["elements"]:
        if only is not None and element["id"] not in only:
            continue
        agent_id = element["id"].split(":", 1)[1]
        if element["state"] == "missing":
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
            continue
        # 升级动作：同步器语义（enabled+installed+受管 provider 激活+auto_update
        # → 无条件跑激活 provider 的 upgrade 命令）。freshness=current 时不产出
        # （同步器空跑无副作用，GUI 也不需要噪音按钮）。
        if element["state"] != "managed-ok":
            continue
        policy = element["policy"]
        observed = element["observed"]
        if not observed["installed"] or not observed["classified"]["managed"]:
            continue
        if not policy["auto_update"]:
            warnings.append(
                diagnostic(
                    "AGENT_UPDATE_DISABLED_BY_POLICY",
                    element["id"],
                    "策略 auto_update=false，plan 不生成升级动作",
                )
            )
            continue
        preferred = policy["preferred_provider"]
        managed_active = [canonical_provider_name(name) for name in observed["classified"]["managed"]]
        preferred_canonical = canonical_provider_name(preferred)
        active_key = preferred_canonical if preferred_canonical in managed_active else managed_active[0]
        provider_cfg = agent_defs.get(agent_id, {}).get("providers", {}).get(active_key, {})
        upgrade_cmd = resolve_provider_command(provider_cfg.get("upgrade"))
        if provider_cfg and "upgrade" in provider_cfg and not upgrade_cmd:
            warnings.append(
                diagnostic(
                    "AGENT_UPGRADE_UNAVAILABLE",
                    element["id"],
                    "provider '%s' 未提供当前平台的 upgrade 命令" % active_key,
                )
            )
            continue
        if not upgrade_cmd:
            continue
        freshness = element["freshness"]
        if freshness["state"] == "current":
            continue
        note = _upgrade_note("Agent", active_key, freshness)
        actions.append(
            plan_action(
                "run-cmd",
                element["id"],
                None,
                "network",
                cmd=upgrade_cmd,
                note=note,
            )
        )
    return plan_data("agents", actions), warnings, errors


def _upgrade_note(kind_label, provider_key, freshness):
    """升级动作的 freshness 说明：behind 带版本箭头；unknown 说明未检查；
    not-applicable 说明无上游探测源（升级语义跟随同步器）。"""
    provider_part = "（provider: %s）" % provider_key if provider_key else ""
    state = freshness.get("state")
    if state == "behind":
        return "升级%s%s：当前 %s → 最新 %s" % (
            kind_label,
            provider_part,
            freshness.get("current") or "?",
            freshness.get("upstream") or "?",
        )
    if state == "unknown":
        return "升级%s%s：上游未检查或探测失败（unknown）" % (kind_label, provider_part)
    return "升级%s%s：无上游探测源，跟随同步器升级语义" % (kind_label, provider_part)


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
    tool_defs, overlay_warnings = policy_layers.apply_definition_overlays(tool_defs, policy_layers.overlay_definitions(config, "tools"), domain="tools")
    warnings.extend(diagnostic("DEFINITION_OVERLAY_UNKNOWN_KEYS", None, item) for item in overlay_warnings)
    removed = {name for name, fragment in policy_layers.overlay_definitions(config, "tools").items() if fragment.get("_delete") is True}
    for name in sorted((set(tool_defs) | set(tools_policy)) - removed):
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


def build_tools_plan(root_dir, config_file, def_files, only=None, agent=None):
    status, warnings, errors = build_tools_status(root_dir, config_file, def_files)
    actions = []
    for element in status["elements"]:
        if only is not None and element["id"] not in only:
            continue
        if element["state"] == "missing":
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
            continue
        # 升级动作（对齐同步器：就绪+auto_update → 跟随 upgrade 命令）；
        # freshness=current 时不产出（噪音控制），behind/unknown/not-applicable 产出。
        if element["state"] != "managed-ok":
            continue
        if not element["policy"]["auto_update"]:
            warnings.append(
                diagnostic(
                    "TOOL_UPDATE_DISABLED_BY_POLICY",
                    element["id"],
                    "策略 auto_update=false，plan 不生成升级动作",
                )
            )
            continue
        definition = element["spec"]
        upgrade_cmd = definition.get("upgrade")
        if not upgrade_cmd:
            continue
        freshness = element["freshness"]
        if freshness["state"] == "current":
            continue
        note_parts = [_upgrade_note("工具", None, freshness)]
        if definition.get("setup"):
            note_parts.append("升级后按需重跑 setup: %s" % definition["setup"])
        actions.append(
            plan_action(
                "run-cmd",
                element["id"],
                None,
                "install-remove",
                cmd=upgrade_cmd,
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


def _read_text_or_none(path):
    try:
        return Path(path).read_text(encoding="utf-8")
    except OSError:
        return None


def _rules_dist_expected_content(root_dir, source_rel):
    """dist 新鲜度的内容语义：用当前 src 渲染出的期望内容与 dist 现有内容比对。

    构建器（build-agent-rules.py，只读复用其渲染原语）的拼装规则：
    dist/<target>/<filename> = compact_lines(shared/core.md) + compact_lines(
    agents/<adapter>/adapter.md)，join("\n") + 尾部换行。无法渲染（产物不在
    TARGET_SPECS 映射内 / src 文件缺失）→ None（不参与 stale 判定）。
    """
    parts = source_rel.split("/")
    if len(parts) != 4 or parts[0] != "rules" or parts[1] != "dist":
        return None
    spec = _build_rules.TARGET_SPECS.get(parts[2])
    if not spec or spec["filename"] != parts[3]:
        return None
    src_root = Path(root_dir) / "agent-rule" / "rules" / "src"
    try:
        core_lines = _build_rules.compact_lines((src_root / "shared" / "core.md").read_text(encoding="utf-8"))
        adapter_lines = _build_rules.compact_lines(
            (src_root / "agents" / spec["adapter"] / "adapter.md").read_text(encoding="utf-8")
        )
    except OSError:
        return None
    return _build_rules.render_target_content(core_lines, adapter_lines)


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
        # dist 新鲜度 = 内容语义：当前 src 渲染期望 ≠ dist 现有内容 → 落后。
        # mtime 只作观测（全新 checkout 的文件时间戳全是 checkout 时刻，不能驱动状态）。
        expected_content = _rules_dist_expected_content(root_dir, entry["source"])
        build_stale = (
            expected_content is not None
            and source_exists
            and _read_text_or_none(src) != expected_content
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


def build_rules_plan(root_dir, config_file, def_files, only=None, agent=None):
    status, warnings, errors = build_rules_status(root_dir, config_file, def_files)
    # dist 构建必须先于任何分发动作（write-file 依赖构建产物）。
    ordered = [e for e in status["elements"] if e["id"] == "rules:dist"]
    ordered += [e for e in status["elements"] if e["id"] != "rules:dist"]
    if only is not None:
        dist_is_target = "rules:dist" in only or "dist" in only
        dist = next((e for e in status["elements"] if e["id"] == "rules:dist"), None)
        dist_drifted = dist is not None and dist["state"] in ("drifted", "not-materialized")
        target_elements = [e for e in status["elements"] if e["id"] != "rules:dist" and (e["id"] in only or e["id"].split(":", 1)[1] in only)]
        dist_needed_as_prereq = len(target_elements) > 0 and dist_drifted
        keep_ids = set(only) | {("rules:%s" % x if not x.startswith("rules:") else x) for x in only}
        if dist_is_target or dist_needed_as_prereq:
            keep_ids.add("rules:dist")
        else:
            keep_ids.discard("rules:dist")
        ordered = [e for e in ordered if e["id"] in keep_ids]
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


def build_hooks_plan(root_dir, config_file, def_files, only=None, agent=None):
    status, warnings, errors = build_hooks_status(root_dir, config_file, def_files)
    actions = []
    for element in status["elements"]:
        if only is not None and element["id"] not in only and element["id"].split(":", 1)[1] not in only:
            continue
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


def build_aliases_plan(root_dir, config_file, def_files, only=None, agent=None):
    status, warnings, errors = build_aliases_status(root_dir, config_file, def_files)
    cfg = load_json_object_file(def_files[0], strict=True, missing_ok=False)
    alias_lines = _render_alias_lines(cfg)
    expected_inner = "\n".join(alias_lines)
    actions = []
    for element in status["elements"]:
        if only is not None and element["id"] not in only and element["id"].split(":", 1)[1] not in only:
            continue
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


def build_mcp_plan(root_dir, config_file, def_files, only=None, agent=None):
    status, warnings, errors = build_mcp_status(root_dir, config_file, def_files[0])
    actions = []
    root_path = Path(root_dir)

    if only is None:
        secrets_dir = root_path / "secrets"
        if secrets_dir.is_dir() and (secrets_dir / ".git").exists():
            actions.append(
                plan_action(
                    "git-pull",
                    "mcp:secrets",
                    str(secrets_dir),
                    "network",
                    note="拉取并同步共享凭证仓库 (secrets)",
                )
            )

    for element in status["elements"]:
        if only is not None and element["id"] not in only and element["id"].split(":", 1)[1] not in only:
            continue
        server_name = element["id"].split(":", 1)[1]
        enabled = element.get("policy", {}).get("enabled", True)
        spec = element.get("spec", {})
        if enabled:
            url = spec.get("url")
            if url:
                repo_dir = root_path / "agent-mcp" / server_name
                actions.append(
                    plan_action(
                        "git-pull",
                        element["id"],
                        str(repo_dir),
                        "network",
                        note="准备/更新本地源码仓库及依赖构建（url: %s）" % url,
                    )
                )
                setup_cmd = spec.get("setup")
                if setup_cmd:
                    actions.append(
                        plan_action(
                            "run-cmd",
                            element["id"],
                            str(repo_dir),
                            "exec-local",
                            cmd=setup_cmd,
                            note="执行自定义初始化: %s" % setup_cmd,
                        )
                    )
                if (repo_dir / "package.json").exists():
                    actions.append(
                        plan_action(
                            "run-cmd",
                            element["id"],
                            str(repo_dir),
                            "network",
                            note="安装 Node.js 依赖并构建",
                        )
                    )
            elif server_name == "serena":
                serena_folder = spec.get("project_serena_folder_location")
                if serena_folder:
                    actions.append(
                        plan_action(
                            "run-cmd",
                            element["id"],
                            None,
                            "exec-local",
                            note="配置 Serena 项目数据目录 (%s)" % serena_folder,
                        )
                    )

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
    agent_defs, _ = policy_layers.apply_definition_overlays(agent_defs, policy_layers.overlay_definitions(config, "agents"), domain="agents")
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
    tool_defs, _ = policy_layers.apply_definition_overlays(tool_defs, policy_layers.overlay_definitions(config, "tools"), domain="tools")
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
        if isinstance(only, str):
            only_set = {only}
        else:
            only_set = set(only)
        targets = [target for target in targets if target["id"] in only_set]
        found_ids = {t["id"] for t in targets}
        missing = only_set - found_ids
        if missing:
            raise RefreshUsageError("--only 指定的元素不可探测或不存在: %s" % ", ".join(sorted(missing)))

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


def _engine_normalize_path(path):
    """版本稳定的路径归一：resolve 软链 + normcase（Windows 折叠大小写）。

    doctor 的 _normalize_comparable_path 在 py<=3.12 上因 ntpath.isabs 把
    POSIX 绝对路径劫持进 windows 分支（只 normpath 不 resolve），导致混合前缀
    布局（/var 软链 vs 已解析前缀）下比较失真；引擎的收敛语义必须与解释器
    版本无关，故引擎自带实现（同物理路径判定）。
    """
    return os.path.normcase(str(Path(path).expanduser().resolve(strict=False)))


def _engine_paths_match(left, right):
    if left is None or right is None:
        return left == right
    return _engine_normalize_path(left) == _engine_normalize_path(right)


def _engine_is_windows_directory_link(path):
    """doctor._is_windows_directory_link port：isjunction 优先，旧解释器退回 lstat 属性位。"""
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


def _engine_resolve_skill_link_target(link_path):
    """版本稳定的链接目标解析（= doctor 在 py3.13+ 的行为）。

    doctor 的 _resolve_skill_link_target 在 py<=3.12 上被 ntpath.isabs 劫持：
    POSIX 绝对目标走 ntpath.normpath 被折叠成反斜杠串（'\\var\\folders\\...'），
    比较链路随之失真。引擎自管解析，三平台解释器行为一致：
    - Windows 设备前缀（\\\\?\\、\\??\\）剥离；
    - Windows 绝对路径（盘符或 \\\\ 开头）→ ntpath.normpath；
    - 其余（POSIX）→ expanduser 后绝对化（相对路径拼到链接所在目录）再 resolve。
    """
    raw = os.fspath(os.readlink(link_path))
    for prefix, replacement in (
        ("\\\\?\\UNC\\", "\\\\"),
        ("\\??\\UNC\\", "\\\\"),
        ("\\\\?\\", ""),
        ("\\??\\", ""),
    ):
        if raw.startswith(prefix):
            raw = replacement + raw[len(prefix):]
            break
    is_windows_abs = raw.startswith("\\\\") or (len(raw) > 2 and raw[1] == ":" and raw[0].isalpha())
    if is_windows_abs:
        return ntpath.normpath(raw)
    resolved = Path(raw).expanduser()
    if not resolved.is_absolute():
        resolved = Path(link_path).parent / resolved
    return str(resolved.resolve(strict=False))


def _engine_actual_skill_targets(target_path):
    """actual_skill_targets 的版本稳定替代（doctor 的返回值在 py<=3.12 会被
    ntpath.normpath 折叠反斜杠，引擎需要解释器无关的链接拓扑）。"""
    actual = {}
    broken = set()
    for name in os.listdir(target_path):
        link_path = Path(target_path) / name
        if not (link_path.is_symlink() or _engine_is_windows_directory_link(link_path)):
            continue
        try:
            actual[name] = _engine_resolve_skill_link_target(link_path)
            if not link_path.exists():
                broken.add(name)
        except OSError:
            actual[name] = "<broken>"
            broken.add(name)
    return actual, broken


def _engine_is_managed_skill_path(target_path, repo_base):
    """链接指向托管缓存仓库（物理路径包含，版本稳定）。"""
    if target_path == "<broken>":
        return False
    normalized = _engine_normalize_path(target_path)
    base = _engine_normalize_path(str(repo_base))
    return normalized == base or normalized.startswith(base + os.sep)


def _skill_cell_state(skill_name, agent_id, expected_by_agent, actual_by_agent, broken_by_agent, repo_base):
    expected = expected_by_agent.get(agent_id, {})
    actual = actual_by_agent.get(agent_id, {})
    broken = broken_by_agent.get(agent_id, set())
    if skill_name in expected:
        if skill_name not in actual:
            return "missing-link"
        if skill_name in broken:
            return "dead-link"
        if _engine_paths_match(actual[skill_name], expected[skill_name]):
            return "linked"
        return "drifted"
    if skill_name not in actual:
        return "disabled"
    if skill_name in broken:
        # 只有指向托管缓存仓库的死链才算我们的清理对象。
        return "dead-link" if _engine_is_managed_skill_path(actual[skill_name], repo_base) else "disabled"
    if _engine_is_managed_skill_path(actual[skill_name], repo_base):
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
    # 定义覆盖层：merged 策略里的 skills.repositories 片段深合并到 skills.json 之上。
    repositories, overlay_warnings = policy_layers.apply_definition_overlays(
        repositories, policy_layers.overlay_definitions(config, "skills"), domain="skills"
    )
    for overlay_warning in overlay_warnings:
        warnings.append(diagnostic("DEFINITION_OVERLAY_UNKNOWN_KEYS", None, overlay_warning))
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
            actual, broken = _engine_actual_skill_targets(target_path)
        else:
            actual, broken = {}, set()
        actual_by_agent[agent_id] = actual
        broken_by_agent[agent_id] = broken

    inbox = collect_skill_discovery_inbox(Path(root_dir), config, skill_cfg)

    # 额外受管链接：未被任何启用集合期望、但指向某集合的托管缓存仓库
    # （doctor 的「存在额外受管 skill 链接」语义），挂在属主集合元素上。
    candidates = set(repositories)
    if repo_base.exists():
        for d in repo_base.iterdir():
            if d.is_dir():
                candidates.add(d.name)
    extras_by_repo = {}
    for agent_id in sorted(agent_targets):
        expected_names = set(expected_by_agent.get(agent_id, {}))
        for name, target in sorted(actual_by_agent.get(agent_id, {}).items()):
            if name in expected_names or not _engine_is_managed_skill_path(target, repo_base):
                continue
            for candidate in sorted(candidates):
                if _engine_is_managed_skill_path(target, (repo_base / candidate).resolve()):
                    extras_by_repo.setdefault(candidate, []).append(
                        {"agent": agent_id, "name": name, "link_path": os.path.join(target_path_by_agent[agent_id], name)}
                    )
                    break

    elements = []
    seen_skill_names = {}
    all_repo_names = sorted(set(repositories) | set(extras_by_repo))
    for repo_name in all_repo_names:
        if repo_name not in repositories:
            repo_path = repo_base / repo_name
            repo_cached = repo_path.exists()
            owner_marked = _link_skills.is_manager_owned_repo(repo_path) if repo_cached else False
            extra_links = extras_by_repo.get(repo_name, [])
            elements.append(
                _make_element(
                    "skill:%s" % repo_name,
                    "skills",
                    {
                        "url": None,
                        "skill_paths": None,
                        "agents": None,
                        "matrix_mode": False,
                        "new_skill_default": "disabled",
                    },
                    {"enabled": False, "declared_in_policy": False},
                    {
                        "repo_cache": str(repo_path),
                        "repo_cached": repo_cached,
                        "owner_marked": owner_marked,
                        "discovered": [],
                        "discovery_inbox": [],
                        "extra_links": extra_links,
                    },
                    "drifted" if extra_links else "disabled",
                    [],
                    freshness=dict(FRESHNESS_NOT_APPLICABLE),
                )
            )
            continue
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


def build_skills_plan(root_dir, config_file, def_files, only=None, agent=None):
    status, warnings, errors = build_skills_status(root_dir, config_file, def_files)
    actions = []
    # 集合仓库动作先行：链接目标都在缓存仓库里。
    for element in status["elements"]:
        if "/" in element["id"]:
            continue
        collection_in_only = False
        if only is not None:
            collection_id = element["id"]
            repo_name = element["id"].split(":", 1)[1]
            collection_in_only = collection_id in only or repo_name in only
            skill_in_only = any(
                o.startswith(f"{collection_id}/") or o.startswith(f"{repo_name}/")
                for o in only
            )
            state = element["state"]
            if not collection_in_only and not (skill_in_only and state == "not-materialized"):
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
        elif collection_in_only and element["spec"].get("auto_update", True) and element["spec"].get("url") and element["policy"].get("enabled", True):
            actions.append(
                plan_action(
                    "git-pull",
                    element["id"],
                    element["observed"]["repo_cache"],
                    "network",
                    note="探测远端上游更新集合仓库缓存（url: %s）" % (element["spec"].get("url") or "?"),
                )
            )
        for extra in element["observed"].get("extra_links", []):
            if agent is not None and extra["agent"] != agent:
                continue
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
        if only is not None:
            collection_id = "skill:%s" % element["spec"]["collection"]
            collection_name = element["spec"]["collection"]
            item_name = element["id"].split(":", 1)[1]
            if (
                element["id"] not in only
                and item_name not in only
                and collection_id not in only
                and collection_name not in only
            ):
                continue
        source_path = element["spec"]["source_path"]
        for agent_id, cell in sorted(element["observed"]["agents"].items()):
            if agent is not None and agent_id != agent:
                continue
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


# ---------------------------------------------------------------------------
# Write path (Phase 2a): '<domain>:apply' delegates to the existing syncer
# scripts with manage.sh's exact argument assembly; 'system:converge' replays
# the update pipeline step order. A snapshot of the domain's target files is
# taken before any non-dry-run write (proposal section 4.10).
# ---------------------------------------------------------------------------


class ApplyGateError(ValueError):
    """Missing explicit confirmation (e.g. --allow-uninstall); usage error."""


def _snapshot_targets_for(root_dir, domains, config_file, def_files, only=None, agent=None):
    """(files, links) covered by a snapshot for the given domain set."""
    root_dir = Path(root_dir)
    files = []
    links = []
    if "mcp" in domains:
        def_path = def_files[0] if def_files else root_dir / "config" / "mcp.json"
        mcp_data = load_json_object_file(def_path, strict=True, missing_ok=False)
        servers = get_typed_mapping_section(mcp_data, "servers", Path(def_path).name, (dict,), "对象")
        config = load_json_object_file(config_file, strict=True, missing_ok=True)
        servers, _ = policy_layers.apply_definition_overlays(
            servers, policy_layers.overlay_definitions(config, "mcp"), domain="mcp"
        )
        agent_paths = get_typed_mapping_section(
            mcp_data, "agent_paths", Path(def_path).name, (str, dict), "字符串或对象"
        )
        only_servers = set()
        if only:
            for item in only:
                s = item.split(":", 1)[1] if item.startswith("mcp:") else item
                only_servers.add(s)

        for agent_id, path_config in canonicalize_agent_mapping(agent_paths).items():
            target_str = path_config if isinstance(path_config, str) else path_config.get("path", "")
            target_path = Path(os.path.expanduser(target_str))
            file_format = "json" if isinstance(path_config, str) else path_config.get("format", "json")
            if only:
                relevant = False
                for s_name in only_servers:
                    s_cfg = servers.get(s_name)
                    if s_cfg is not None:
                        s_agents = s_cfg.get("agents")
                        if s_agents is None or agent_id in s_agents:
                            relevant = True
                            break
                if not relevant and target_path.exists():
                    if file_format == "json":
                        try:
                            with open(target_path, "r", encoding="utf-8") as f:
                                doc = json.load(f)
                            sec = doc.get("mcp" if agent_id == "opencode" else "mcpServers") or {}
                            if any(s in sec for s in only_servers):
                                relevant = True
                        except Exception:
                            pass
                    elif file_format == "toml":
                        try:
                            txt = target_path.read_text(encoding="utf-8")
                            names = set()
                            try:
                                from src.toml_compat import tomllib
                            except ImportError:
                                tomllib = None
                            if tomllib is not None:
                                try:
                                    doc = tomllib.loads(txt)
                                    if isinstance(doc, dict):
                                        sec = doc.get("mcp_servers")
                                        if isinstance(sec, dict):
                                            names = set(sec.keys())
                                except Exception:
                                    pass
                            if not names:
                                for m in re.finditer(r'^\s*\[\s*mcp_servers\.(?P<key>"(?:\\.|[^"\\])*"|[A-Za-z0-9_-]+)', txt, re.MULTILINE):
                                    k = m.group("key")
                                    names.add(json.loads(k) if k.startswith('"') else k)
                            if any(s in names for s in only_servers):
                                relevant = True
                        except Exception:
                            pass
                if not relevant:
                    continue
            files.append(str(target_path))
    if "hooks" in domains:
        claude_hook = str(_sync_claude_hooks.build_managed_hooks(root_dir / "config" / "claude-hooks.json")[0])
        codex_hook = str(_sync_codex_hooks.build_managed_hooks(root_dir / "config" / "codex-hooks.json")[0])
        if only:
            for item in only:
                norm = item.split(":", 1)[1] if item.startswith("hooks:") else item
                if "claude" in norm:
                    files.append(claude_hook)
                elif "codex" in norm:
                    files.append(codex_hook)
        else:
            files.append(claude_hook)
            files.append(codex_hook)
    if "aliases" in domains:
        targets = []
        if only:
            for item in only:
                norm = item.split(":", 1)[1] if item.startswith("aliases:") else item
                if norm == "zshrc":
                    targets.append(os.path.expanduser("~/.zshrc"))
                elif norm == "zprofile":
                    targets.append(os.path.expanduser("~/.zprofile"))
        else:
            targets = [os.path.expanduser("~/.zshrc"), os.path.expanduser("~/.zprofile")]
        files.extend(targets)
    if "rules" in domains:
        manifest_path = def_files[0] if def_files else root_dir / "config" / "rules-targets.list"
        entries, _problems = _parse_rules_manifest(manifest_path)
        home = os.path.expanduser("~")
        for entry in entries:
            target_name = entry.get("name", "")
            if only and not any(o == target_name or o == f"rules:{target_name}" for o in only):
                continue
            files.append(
                os.path.expanduser(
                    entry["target"].replace("{{HOME}}", home).replace("{{REPO_ROOT}}", str(root_dir))
                )
            )
    if "skills" in domains:
        def_path = def_files[0] if def_files else root_dir / "config" / "skills.json"
        skills_data = load_json_object_file(def_path, strict=True, missing_ok=False)
        raw_repos = get_typed_mapping_section(skills_data, "repositories", Path(def_path).name, (dict,), "对象")
        config = load_json_object_file(config_file, strict=True, missing_ok=True)
        repositories, _ = policy_layers.apply_definition_overlays(
            raw_repos, policy_layers.overlay_definitions(config, "skills"), domain="skills"
        )
        agent_targets = canonicalize_agent_mapping(
            get_typed_mapping_section(skills_data, "agents", Path(def_path).name, (str, dict), "字符串或对象")
        )
        skill_cfg = {"repositories": repositories, "agents": agent_targets}
        repo_base = (root_dir / "agent-skill" / "repositories").resolve()

        target_collections = set()
        target_skills = set()
        if only:
            for item in only:
                norm = item.split(":", 1)[1] if item.startswith("skill:") else item
                if "/" in norm:
                    c, s = norm.split("/", 1)
                    target_skills.add((c, s))
                else:
                    target_collections.add(norm)

        for agent_id, target_cfg in agent_targets.items():
            if agent is not None and agent_id != agent:
                continue
            target_dir = target_cfg if isinstance(target_cfg, str) else target_cfg.get("path", "")
            target_dir = os.path.expanduser(target_dir)

            expected, _ = expected_managed_skill_targets(root_dir, config, skill_cfg, agent_id)
            actual, _ = _engine_actual_skill_targets(target_dir) if os.path.isdir(target_dir) else ({}, set())

            seen_paths = set()
            for s_name, src_path in expected.items():
                try:
                    rel = Path(src_path).resolve().relative_to(repo_base)
                    coll = rel.parts[0]
                except (ValueError, IndexError):
                    coll = ""
                if only:
                    if coll not in target_collections and (coll, s_name) not in target_skills:
                        continue
                link_path = os.path.join(target_dir, s_name)
                seen_paths.add(link_path)
                if os.path.islink(link_path):
                    links.append({"path": link_path, "target": os.readlink(link_path), "existed": True})
                else:
                    links.append({"path": link_path, "target": "", "existed": False})

            for s_name, tgt in actual.items():
                link_path = os.path.join(target_dir, s_name)
                if link_path in seen_paths:
                    continue
                if not _engine_is_managed_skill_path(tgt, repo_base):
                    continue
                try:
                    rel = Path(tgt).resolve().relative_to(repo_base)
                    coll = rel.parts[0]
                except (ValueError, IndexError):
                    coll = ""
                if only:
                    if coll not in target_collections and (coll, s_name) not in target_skills:
                        continue
                links.append({"path": link_path, "target": tgt, "existed": True})
    return files, links


def _apply_step_specs(domain, root_dir, config_file, def_files, mode, python_cmd, build_stale_rules=True, only=None, agent=None, action=None):
    """Step specs [(name, cmd, cwd, timeout, fatal)] mirroring manage.sh args.

    build_stale_rules=False（system:converge 用）：update 管线从不构建 dist，
    converge 必须逐字节等价，故禁用构建增强；rules:apply（域命令，提案 §2.3
    「构建+分发」）保留 build-if-stale。"""
    root_dir = Path(root_dir)
    dry = mode["dry_run"]
    offline = mode["offline"]
    steps = []
    if domain == "mcp":
        cmd = [python_cmd, "sync_mcp.py", "--config", str(config_file), "--def-file", str(def_files[0]), "--prepare-runtime"]
        if only:
            for s in only:
                srv = s.split(":", 1)[1] if s.startswith("mcp:") else s
                cmd.extend(["--only", srv])
        if offline:
            cmd.append("--offline")
        if dry:
            cmd.append("--dry-run")
        steps.append(("mcp-sync", cmd, root_dir / "agent-mcp", 600, True))
    elif domain == "skills":
        cmd = [python_cmd, "link_skills.py", "--config", str(config_file), "--def-file", str(def_files[0]), "--yes"]
        if only:
            for s in only:
                cmd.extend(["--only", s])
        if agent:
            cmd.extend(["--agent", agent])
        if offline:
            cmd.append("--offline")
        if dry:
            cmd.append("--dry-run")
        steps.append(("skills-link", cmd, root_dir / "agent-skill", 600, True))
    elif domain == "tools":
        cmd = [python_cmd, "sync_tools.py", "--config", str(config_file), "--def-file", str(def_files[0]), "--yes"]
        if only:
            for s in only:
                tool_name = s.split(":", 1)[1] if s.startswith("tools:") else s
                cmd.extend(["--only", tool_name])
        if offline:
            cmd.append("--offline")
        if dry:
            cmd.append("--dry-run")
        steps.append(("tools-sync", cmd, root_dir / "agent-tool", 600, True))
    elif domain == "rules":
        status, _warnings, _errors = build_rules_status(root_dir, config_file, def_files)
        dist = next((e for e in status["elements"] if e["id"] == "rules:dist"), None)
        targets = [
            e["spec"]["name"]
            for e in status["elements"]
            if e["id"] != "rules:dist" and e["policy"]["enabled"]
        ]
        if only is not None:
            targets = [
                name for name in targets
                if ("rules:%s" % name) in only or name in only
            ]
        dist_is_target = only is not None and ("rules:dist" in only or "dist" in only)
        dist_drifted = dist is not None and dist["state"] in ("drifted", "not-materialized")
        dist_needed_as_prereq = len(targets) > 0 and dist_drifted
        should_build_dist = build_stale_rules and (
            (only is None and dist_drifted) or dist_is_target or dist_needed_as_prereq
        ) and not dry
        if should_build_dist:
            steps.append(
                ("rules-build", [python_cmd, "scripts/build-agent-rules.py"], root_dir / "agent-rule", 600, True)
            )
        if targets:
            cmd = ["bash", "./scripts/sync-agent-rules.sh"]
            for name in targets:
                cmd += ["--target", name]
            if dry:
                cmd.append("--dry-run")
            steps.append(("rules-sync", cmd, root_dir / "agent-rule", 600, True))
    elif domain == "hooks":
        for script, name, element_id in (
            ("./scripts/sync-claude-hooks.py", "claude-hooks", "hooks:claude"),
            ("./scripts/sync-codex-hooks.py", "codex-hooks", "hooks:codex"),
        ):
            if only is not None and element_id not in only and element_id.split(":", 1)[1] not in only:
                continue
            cmd = [python_cmd, script]
            if dry:
                cmd.append("--dry-run")
            steps.append((name, cmd, root_dir / "agent-rule", 600, True))
    elif domain == "aliases":
        cmd = ["bash", str(root_dir / "scripts" / "claude" / "sync_shell_aliases.sh")]
        if only:
            for s in only:
                norm = s.split(":", 1)[1] if s.startswith("aliases:") else s
                cmd.extend(["--target", norm])
        if dry:
            cmd.append("--dry-run")
        steps.append(("shell-aliases", cmd, root_dir, 60, True))
    elif domain == "agents":
        cmd = [python_cmd, "sync_agents.py", "--config", str(config_file), "--def-file", str(def_files[0]), "--yes"]
        if only:
            for s in only:
                agent_name = s.split(":", 1)[1] if s.startswith("agents:") else s
                cmd.extend(["--only", agent_name])
        if action:
            # 显式生命周期动作：同一解析规则已在 agents:plan --action 预览过
            cmd.extend(["--action", action])
            if action == "uninstall":
                # 引擎侧已用 --allow-uninstall 二次确认，透传给 sync_agents 的同名门槛
                cmd.append("--allow-uninstall")
        if offline:
            cmd.append("--offline")
        if dry:
            cmd.append("--dry-run")
        # 引擎 apply 前台执行（envelope 等待结果），不投递 manage.sh 的后台异步任务。
        steps.append(("agents-lifecycle", cmd, root_dir / "agent-lifecycle", 600, True))
    return steps


def _agents_apply_gate(root_dir, config_file, def_files, allow_uninstall, only=None):
    """卸载是 install-remove 级二次确认：禁用但仍有受管安装时须显式放行。"""
    if allow_uninstall:
        return
    status, _warnings, _errors = build_agents_status(root_dir, config_file, def_files)
    pending = [
        element["id"]
        for element in status["elements"]
        if element["state"] == "drifted"
        and not element["policy"]["enabled"]
        and element["observed"]["classified"]["managed"]
    ]
    if only is not None:
        pending = [p for p in pending if p in only or p.split(":", 1)[1] in only]
    if pending:
        raise ApplyGateError(
            "以下 Agent 的禁用将触发卸载（install-remove 级），须显式追加 --allow-uninstall: %s"
            % ", ".join(pending)
        )


def build_domain_apply(domain, root_dir, config_file, def_files, mode, run_id, log_file, python_cmd, allow_uninstall=False, only=None, agent=None, action=None):
    """'<domain>:apply': snapshot -> run syncer steps -> step summary."""
    warnings = []
    errors = []
    if domain == "agents" and action is None:
        _agents_apply_gate(root_dir, config_file, def_files, allow_uninstall, only=only)
    if domain == "agents" and action == "uninstall" and not allow_uninstall:
        raise ApplyGateError("显式卸载（install-remove 级）须显式追加 --allow-uninstall")
    snapshot = None
    if not mode["dry_run"]:
        files, links = _snapshot_targets_for(root_dir, {domain}, config_file, def_files, only=only, agent=agent)
        manifest = engine_apply.create_snapshot(run_id, "%s.apply" % domain, files, links)
        snapshot = {
            "run_id": manifest["run_id"],
            "path": str(engine_apply.snapshot_root() / run_id),
            "file_count": len(manifest["files"]),
            "link_count": len(manifest["links"]),
        }
    steps = _apply_step_specs(domain, root_dir, config_file, def_files, mode, python_cmd, only=only, agent=agent, action=action)
    results = []
    for name, cmd, cwd, timeout, fatal in steps:
        result = engine_apply.run_step(name, cmd, cwd, log_file if not mode["dry_run"] else None, timeout)
        results.append({"name": result["name"], "ok": result["ok"], "exit_code": result["exit_code"]})
        if not result["ok"] and fatal:
            errors.append(
                diagnostic("APPLY_STEP_FAILED", None, "步骤 %s 失败（exit %d，详见日志）" % (name, result["exit_code"]))
            )
    # 显式 install/uninstall 成功（sync_agents 已做装后/卸后复核）后才保存 enabled，
    # 防止下一次自动收敛把卸载重新安装/把新安装卸载；失败不写任何策略。
    if (
        domain == "agents"
        and action in ("install", "uninstall")
        and not mode["dry_run"]
        and not errors
        and results
        and all(step["ok"] for step in results)
    ):
        agent_id = only[0].split(":", 1)[1] if only and ":" in only[0] else (only[0] if only else None)
        enabled_value = "true" if action == "install" else "false"
        try:
            build_policy_set(
                root_dir, config_file, "local", "agents.%s.enabled" % agent_id, enabled_value, run_id=run_id
            )
        except Exception as exc:
            errors.append(
                diagnostic("POLICY_SAVE_FAILED", only[0] if only else None, "动作已执行但 enabled 策略保存失败: %s" % exc)
            )
        else:
            warnings.append(
                diagnostic("POLICY_SAVED", only[0] if only else None, "已保存 agents.%s.enabled=%s" % (agent_id, enabled_value))
            )
    data = {
        "domain": domain,
        "dry_run": mode["dry_run"],
        "steps": results,
        "snapshot": snapshot,
        "skipped": [],
    }
    return data, warnings, errors


def _browser_us_enabled(config_file):
    """manage.sh browser_us_sync_enabled 的同语义判定（env 覆盖优先）。"""
    override = os.environ.get("AGENT_MANAGER_BROWSER_US_SYNC_ENABLED", "").strip().lower()
    if override in ("1", "true", "yes", "on", "enabled"):
        return True
    if override in ("0", "false", "no", "off", "disabled"):
        return False
    data = load_json_object_file(config_file, strict=True, missing_ok=False)
    settings = data.get("browser_us_sync")
    if not isinstance(settings, dict):
        settings = {}
    return bool(settings.get("enabled", True))


def _converge_step_specs(root_dir, config_file, mode, python_cmd):
    """run_apply_steps 的同序重放（update 管线语义）；secrets 拉取仅在已有
    仓库时进行且非致命（对齐 manage.sh maybe_init_secrets 的失败继续语义），
    交互式初始化永远不做；browser_us 步骤遵循其独立开关。
    返回 (steps, skipped)。"""
    root_dir = Path(root_dir)
    dry = mode["dry_run"]
    steps = []
    skipped = []
    if (root_dir / "secrets" / ".git").is_dir() and not mode["offline"]:
        cmd = ["git", "-C", str(root_dir / "secrets"), "pull", "--ff-only"]
        if not dry:
            steps.append(("secrets-pull", cmd, root_dir, 600, False))
    if not _browser_us_enabled(config_file):
        skipped.append("browser-us")
    for domain in ("aliases", "tools", "rules", "skills", "mcp", "hooks"):
        def_files = None
        if domain == "mcp":
            def_files = [root_dir / "config" / "mcp.json"]
        elif domain == "skills":
            def_files = [root_dir / "config" / "skills.json"]
        elif domain == "tools":
            def_files = [root_dir / "config" / "tools.json"]
        elif domain == "rules":
            def_files = [root_dir / "config" / "rules-targets.list"]
        before = len(steps)
        steps.extend(_apply_step_specs(domain, root_dir, config_file, def_files, mode, python_cmd, build_stale_rules=False))
        if domain == "rules" and len(steps) == before:
            skipped.append("rules-sync")
        if domain == "tools" and _browser_us_enabled(config_file):
            cmd = [python_cmd, "scripts/sync_browser_google_us.py"]
            if dry:
                cmd.append("--dry-run")
            if mode["offline"]:
                cmd.append("--offline")
            steps.append(("browser-us", cmd, root_dir, 120, True))
    return steps, skipped


def build_system_converge(root_dir, config_file, mode, run_id, log_file, python_cmd):
    """'system:converge': update 管线等价物（提案 §2.3）。浏览器 US 同步步骤与
    skill 引用审计按 manage.sh 语义处理（前者有独立开关，后者仅警告）。"""
    warnings = []
    errors = []
    snapshot = None
    if not mode["dry_run"]:
        files, links = _snapshot_targets_for(
            root_dir, {"mcp", "hooks", "aliases", "rules", "skills"}, config_file, None
        )
        manifest = engine_apply.create_snapshot(run_id, "system.converge", files, links)
        snapshot = {
            "run_id": manifest["run_id"],
            "path": str(engine_apply.snapshot_root() / run_id),
            "file_count": len(manifest["files"]),
            "link_count": len(manifest["links"]),
        }
    steps, skipped = _converge_step_specs(root_dir, config_file, mode, python_cmd)
    results = []
    for name, cmd, cwd, timeout, fatal in steps:
        result = engine_apply.run_step(name, cmd, cwd, log_file if not mode["dry_run"] else None, timeout)
        results.append({"name": result["name"], "ok": result["ok"], "exit_code": result["exit_code"]})
        if not result["ok"]:
            if fatal:
                errors.append(
                    diagnostic(
                        "CONVERGE_STEP_FAILED", None, "步骤 %s 失败（exit %d，详见日志）" % (name, result["exit_code"])
                    )
                )
            else:
                warnings.append(
                    diagnostic(
                        "CONVERGE_STEP_SOFT_FAILED", None, "步骤 %s 失败但不致命（exit %d）" % (name, result["exit_code"])
                    )
                )
    data = {
        "pipeline": "update",
        "dry_run": mode["dry_run"],
        "steps": results,
        "snapshot": snapshot,
        "skipped": skipped,
    }
    return data, warnings, errors


# ---------------------------------------------------------------------------
# runs / snapshots / policy domains
# ---------------------------------------------------------------------------


def _log_prefix():
    return os.environ.get("AGENT_MANAGER_LOG_PREFIX") or "/tmp/agent_manager_sync_%s" % getuser()


def _run_entry(path, latest_target):
    stat = path.stat()
    prefix_name = Path(_log_prefix()).name
    run_id = path.name[len(prefix_name) + 1 : -len(".log")]
    return {
        "run_id": run_id,
        "log_file": str(path),
        "size_bytes": stat.st_size,
        "mtime": datetime.datetime.fromtimestamp(stat.st_mtime, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "is_latest": str(path) == latest_target,
    }


class RunNotFoundError(ValueError):
    """runs:get 指定的 run_id 无对应日志文件。"""


def build_runs_list(root_dir, config_file):
    prefix = Path(_log_prefix())
    latest_target = None
    try:
        latest_target = (prefix.parent / (prefix.name + ".latest")).read_text(encoding="utf-8").strip()
    except OSError:
        pass
    runs = []
    for path in sorted(prefix.parent.glob(prefix.name + "_*.log"), key=lambda p: p.name, reverse=True):
        runs.append(_run_entry(path, latest_target))
    return {"runs": runs, "tail": None, "lines": None}, [], []


def build_runs_get(root_dir, config_file, run_id, lines):
    prefix = Path(_log_prefix())
    path = prefix.parent / ("%s_%s.log" % (prefix.name, run_id))
    if not path.is_file():
        raise RunNotFoundError("运行日志不存在: %s" % run_id)
    with open(path, encoding="utf-8", errors="replace") as handle:
        all_lines = handle.read().splitlines()
    tail = all_lines[-lines:] if lines > 0 else all_lines
    latest_target = None
    try:
        latest_target = (prefix.parent / (prefix.name + ".latest")).read_text(encoding="utf-8").strip()
    except OSError:
        pass
    data = {
        "runs": [_run_entry(path, latest_target)],
        "tail": "\n".join(tail) + ("\n" if tail else ""),
        "lines": len(tail),
    }
    return data, [], []


def build_snapshots_list(root_dir, config_file):
    return {"snapshots": engine_apply.list_snapshots(), "restore": None}, [], []


def build_snapshots_restore(root_dir, config_file, run_id, resnapshot_run_id):
    restore = engine_apply.restore_snapshot(run_id, resnapshot_run_id=resnapshot_run_id)
    return {"snapshots": None, "restore": restore}, [], []


def build_policy_get(root_dir, config_file):
    if storage.is_sqlite_active(root_dir):
        db_path = storage.get_db_path()
        return storage.policy_get_db(db_path), [], []
    layers = policy_layers.load_layers(root_dir)
    data = {
        "layers": layers,
        "effective": policy_layers.effective_config(layers),
        "attribution": policy_layers.attribute_top_level_keys(layers),
        "layer": None,
        "key": None,
        "value": None,
        "path": None,
    }
    return data, [], []


def build_policy_set(root_dir, config_file, layer, key, raw_value, run_id=None, dry_run=False):
    if layer not in policy_layers.WRITABLE_LAYERS:
        raise policy_layers.PolicySetError(
            "baseline 层只读；--layer 只能是 %s" % " / ".join(policy_layers.WRITABLE_LAYERS)
        )
    try:
        value = json.loads(raw_value)
    except ValueError as exc:
        raise policy_layers.PolicySetError("--value 不是合法 JSON: %s" % exc)
    if dry_run:
        # 显式 --dry-run：只校验，不写入任何后端（SQLite 或旧层文件）。
        data = {
            "layers": None,
            "effective": None,
            "attribution": None,
            "layer": layer,
            "key": key,
            "value": value,
            "path": None,
        }
        warnings = [diagnostic("DRY_RUN", None, "dry-run：未写入 %s" % key)]
        return data, warnings, []
    if storage.is_sqlite_active(root_dir):
        db_path = storage.get_db_path()
        path = storage.policy_set_db(db_path, key, value, run_id=run_id)
        data = {
            "layers": None,
            "effective": None,
            "attribution": None,
            "layer": layer,
            "key": key,
            "value": value,
            "path": str(path),
        }
        return data, [], []
    path = policy_layers.set_policy_value(root_dir, layer, key, value)
    data = {
        "layers": None,
        "effective": None,
        "attribution": None,
        "layer": layer,
        "key": key,
        "value": value,
        "path": str(path),
    }
    return data, [], []


def build_policy_delete(root_dir, config_file, layer, key, run_id, dry_run=False):
    """policy:delete：从 synced/local 层删除键；写前快照（复用运行快照机制）。"""
    if layer not in policy_layers.WRITABLE_LAYERS:
        raise policy_layers.PolicySetError(
            "baseline 层只读；--layer 只能是 %s" % " / ".join(policy_layers.WRITABLE_LAYERS)
        )
    if dry_run:
        # 显式 --dry-run：不删除、不写任何后端（SQLite 或旧层文件）。
        data = {
            "layers": None,
            "effective": None,
            "attribution": None,
            "layer": layer,
            "key": key,
            "value": None,
            "path": None,
        }
        warnings = [diagnostic("DRY_RUN", None, "dry-run：未删除 %s" % key)]
        return data, warnings, []
    if storage.is_sqlite_active(root_dir):
        db_path = storage.get_db_path()
        path = storage.policy_delete_db(db_path, key, run_id=run_id)
        data = {
            "layers": None,
            "effective": None,
            "attribution": None,
            "layer": layer,
            "key": key,
            "value": None,
            "path": str(path),
        }
        return data, [], []
    path = policy_layers.layer_paths(root_dir)[layer]
    # 先只读探测键存在性（让 PolicyKeyNotFoundError 先于快照触发，空跑不留快照）。
    probe = load_json_object_file(path, strict=False, missing_ok=True)
    cursor = probe
    for segment in key.split("."):
        if not isinstance(cursor, dict) or segment not in cursor:
            raise policy_layers.PolicyKeyNotFoundError("键不存在: %s" % key)
        cursor = cursor[segment]
    if path.is_file():
        engine_apply.create_snapshot(run_id, "policy.delete", [str(path)], [])
    written = policy_layers.delete_policy_value(root_dir, layer, key)
    data = {
        "layers": None,
        "effective": None,
        "attribution": None,
        "layer": layer,
        "key": key,
        "value": None,
        "path": str(written),
    }
    return data, [], []


def build_db_status(root_dir, config_file):
    db_path = storage.get_db_path()
    exists = db_path.is_file()
    summary = {}
    ready = False
    if exists:
        conn = storage.connect_db(db_path, read_only=True)
        try:
            storage.ensure_db_ready(conn, db_path)
            ready = True
            for row in conn.execute("SELECT key, value FROM schema_meta").fetchall():
                summary[row["key"]] = row["value"]
            summary["counts"] = {
                "agents": conn.execute("SELECT COUNT(*) FROM definitions WHERE domain = 'agent' AND deleted = 0").fetchone()[0],
                "tools": conn.execute("SELECT COUNT(*) FROM definitions WHERE domain = 'tool' AND deleted = 0").fetchone()[0],
                "mcp": conn.execute("SELECT COUNT(*) FROM definitions WHERE domain = 'mcp' AND deleted = 0").fetchone()[0],
                "skills": conn.execute("SELECT COUNT(*) FROM definitions WHERE domain = 'skill' AND deleted = 0").fetchone()[0],
                "policies": conn.execute("SELECT COUNT(*) FROM policy_entries").fetchone()[0],
            }
        except Exception as exc:
            # 损坏/外来/未来版本的库：status 如实报告不就绪，绝不尝试修复或写入。
            summary["error"] = str(exc)
        finally:
            conn.close()
    data = {
        "active": storage.is_sqlite_active(root_dir),
        "ready": ready,
        "data_dir": str(storage.get_data_dir()),
        "db_path": str(db_path),
        "exists": exists,
        "meta": summary,
    }
    return data, [], []


def build_db_migrate(root_dir, config_file, dry_run=False):
    if dry_run:
        # 显式 --dry-run：绝不创建/迁移 DB，仅报告当前状态。
        data, warnings, errors = build_db_status(root_dir, config_file)
        data["dry_run"] = True
        data["would_migrate"] = not data["exists"]
        warnings.append({"code": "DRY_RUN", "element": None, "message": "dry-run：未执行迁移，manager.db 状态未改变"})
        return data, warnings, errors
    storage.migrate_if_needed(root_dir)
    return build_db_status(root_dir, config_file)


# Kernel registry. Every kernel takes (root_dir, config_file, def_files) and
# returns (data, warnings, errors). profile lands in a later phase.
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
# apply 复用既有同步器（引擎不重实现写逻辑）；secrets 只读，profile 后续阶段。
APPLY_DOMAINS = ("agents", "aliases", "hooks", "mcp", "rules", "skills", "tools")


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
    parser.add_argument(
        "--only",
        action="append",
        default=[],
        help="范围控制：只针对指定元素 id（如 mcp:fetch；可重复）；system:refresh-upstream 亦可用",
    )
    parser.add_argument(
        "--agent",
        help="skills 域专属范围控制：仅针对指定 agent id（如 codex）",
    )
    parser.add_argument("--allow-uninstall", action="store_true", help="agents:apply 显式放行卸载（install-remove 级二次确认）")
    parser.add_argument("--action", choices=("install", "upgrade", "uninstall"), help="agents:plan/apply 显式生命周期动作；必须配合 --only 恰好一个 Agent")
    parser.add_argument("--run", help="runs:get / snapshots:restore 的 run_id")
    parser.add_argument("--lines", type=int, default=2000, help="runs:get 日志尾部行数（默认 2000；0=全部）")
    parser.add_argument("--layer", help="policy:set 的落点层（synced|local）")
    parser.add_argument("--key", help="policy:set 的点分键路径")
    parser.add_argument("--value", help="policy:set 的 JSON 值")
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
    # 同步器步骤在子目录里执行（cwd=agent-*/），配置路径一律绝对化。
    if args.config:
        args.config = str(Path(args.config).resolve())
    if args.def_file:
        args.def_file = [str(Path(p).resolve()) for p in args.def_file]
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
    elif verb == "apply" and domain in APPLY_DOMAINS:
        kernel = "apply"
    elif domain == "system" and verb == "audit":
        kernel = "audit"
        kernel_needs_def_file = False
    elif domain == "system" and verb == "refresh-upstream":
        kernel = "refresh"
        kernel_needs_def_file = False
    elif domain == "system" and verb == "converge":
        kernel = "converge"
        kernel_needs_def_file = False
    elif domain == "runs" and verb in ("list", "get"):
        kernel = "runs:%s" % verb
        kernel_needs_def_file = False
    elif domain == "snapshots" and verb in ("list", "restore"):
        kernel = "snapshots:%s" % verb
        kernel_needs_def_file = False
    elif domain == "policy" and verb in ("get", "set", "delete"):
        kernel = "policy:%s" % verb
        kernel_needs_def_file = False
    elif domain == "sources" and verb in ("get", "set"):
        kernel = "sources:%s" % verb
        kernel_needs_def_file = False
    elif domain == "db" and verb in ("status", "migrate"):
        kernel = "db:%s" % verb
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
    if kernel in ("runs:get", "snapshots:restore") and not args.run:
        return usage_error(
            "USAGE_ERROR",
            "%s 需要 --run <run_id>" % dotted,
            "错误: 缺少 --run 参数",
        )
    if kernel == "policy:set" and (not args.layer or not args.key or args.value is None):
        return usage_error(
            "USAGE_ERROR",
            "policy:set 需要 --layer synced|local --key <dot.path> --value <json>",
            "错误: policy:set 参数不完整（--layer/--key/--value 缺一不可）",
        )
    if kernel == "policy:delete" and (not args.layer or not args.key):
        return usage_error(
            "USAGE_ERROR",
            "policy:delete 需要 --layer synced|local --key <dot.path>",
            "错误: policy:delete 参数不完整（--layer/--key 缺一不可）",
        )

    if args.agent:
        if domain != "skills":
            return usage_error("USAGE_ERROR", "--agent 仅对 skills 域有效", "错误: --agent 参数只允许用于 skills 域")
        if verb in ("plan", "apply"):
            skills_def = (
                args.def_file[0]
                if (args.def_file and len(args.def_file) > 0)
                else str(Path(REPO_ROOT) / "config" / "skills.json")
            )
            if os.path.exists(skills_def):
                skills_data = load_json_object_file(skills_def, strict=False, missing_ok=True) or {}
                agent_targets = canonicalize_agent_mapping(
                    get_typed_mapping_section(skills_data, "agents", Path(skills_def).name, (str, dict), "字符串或对象")
                )
                if args.agent not in agent_targets:
                    return usage_error(
                        "USAGE_ERROR",
                        f"未知 agent '{args.agent}'",
                        f"错误: agent '{args.agent}' 未在 skills 定义中声明",
                    )

    DOMAIN_PREFIXES = {
        "agents": "agents:",
        "aliases": "aliases:",
        "hooks": "hooks:",
        "mcp": "mcp:",
        "rules": "rules:",
        "skills": "skill:",
        "tools": "tools:",
    }
    if args.only and verb in ("plan", "apply"):
        expected_prefix = DOMAIN_PREFIXES.get(domain)
        for item in args.only:
            if not expected_prefix or not item.startswith(expected_prefix):
                return usage_error(
                    "CROSS_DOMAIN_SCOPE",
                    f"元素 '{item}' 不属于域 '{domain}'（期望前缀 '{expected_prefix}'）",
                    f"错误: 跨域范围控制 '{item}' 拒绝",
                )
        status_kernel = STATUS_KERNELS.get(domain)
        if status_kernel:
            try:
                status_data, _sw, _se = status_kernel(REPO_ROOT, args.config, args.def_file)
                if _se:
                    return usage_error(
                        "CONFIG_INVALID",
                        f"配置验证失败: {'; '.join(e.get('message', '') for e in _se)}",
                        f"错误: 域 '{domain}' 配置验证失败",
                    )
                existing_ids = {e["id"] for e in status_data.get("elements", [])}
                for item in args.only:
                    if item not in existing_ids:
                        return usage_error(
                            "USAGE_ERROR",
                            f"未知元素 '{item}'",
                            f"错误: 元素 '{item}' 在域 '{domain}' 中不存在",
                        )
            except Exception as exc:
                if isinstance(exc, (RefreshUsageError, ApplyGateError)):
                    raise
                return usage_error(
                    "CONFIG_INVALID",
                    f"无法验证范围 '{', '.join(args.only)}': {exc}",
                    f"错误: 域 '{domain}' 状态读取失败: {exc}",
                )

    if args.action:
        if domain != "agents" or verb not in ("plan", "apply"):
            return usage_error(
                "USAGE_ERROR",
                "--action 仅允许 agents:plan / agents:apply",
                "错误: --action 只允许用于 agents:plan 或 agents:apply",
            )
        if len(args.only or []) != 1:
            return usage_error(
                "USAGE_ERROR",
                "--action 必须用 --only 指定恰好一个 Agent",
                "错误: --action 需要 --only agents:<id> 且恰好一个",
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
            data, warnings, errors = build_system_refresh_upstream(REPO_ROOT, args.config, only=args.only if args.only else None)
        elif kernel == "apply":
            data, warnings, errors = build_domain_apply(
                domain, REPO_ROOT, args.config, args.def_file, mode, run_id, log_file,
                args.python_cmd, allow_uninstall=args.allow_uninstall,
                only=args.only if args.only else None, agent=args.agent,
                action=args.action,
            )
        elif kernel == "converge":
            data, warnings, errors = build_system_converge(
                REPO_ROOT, args.config, mode, run_id, log_file, args.python_cmd
            )
        elif kernel == "runs:list":
            data, warnings, errors = build_runs_list(REPO_ROOT, args.config)
        elif kernel == "runs:get":
            data, warnings, errors = build_runs_get(REPO_ROOT, args.config, args.run, args.lines)
        elif kernel == "snapshots:list":
            data, warnings, errors = build_snapshots_list(REPO_ROOT, args.config)
        elif kernel == "snapshots:restore":
            data, warnings, errors = build_snapshots_restore(
                REPO_ROOT, args.config, args.run, resnapshot_run_id="%s-re-%s" % (args.run, run_id)
            )
        elif kernel == "sources:get":
            if args.key and args.key not in ("rules", "hooks", "aliases"):
                raise ValueError("--key 只支持 rules、hooks 或 aliases")
            data = {"sources": [source_definitions.get_source(REPO_ROOT, key) for key in source_definitions.SOURCES if not args.key or key.startswith(args.key + ".")]}
            warnings, errors = [], []
        elif kernel == "sources:set":
            source = source_definitions.set_source(REPO_ROOT, args.key, json.loads(args.value or "null"),
                snapshot=lambda paths: engine_apply.create_snapshot(run_id, "sources.set", paths, []),
                validate_hooks=_sync_claude_hooks._validate_hooks_map, dry_run=mode["dry_run"])
            data = {"sources": [source]}
            warnings, errors = [], []
        elif kernel == "policy:get":
            data, warnings, errors = build_policy_get(REPO_ROOT, args.config)
        elif kernel == "policy:set":
            data, warnings, errors = build_policy_set(REPO_ROOT, args.config, args.layer, args.key, args.value, run_id=run_id, dry_run=mode["dry_run"])
        elif kernel == "policy:delete":
            data, warnings, errors = build_policy_delete(REPO_ROOT, args.config, args.layer, args.key, run_id, dry_run=mode["dry_run"])
        elif kernel == "db:status":
            data, warnings, errors = build_db_status(REPO_ROOT, args.config)
        elif kernel == "db:migrate":
            data, warnings, errors = build_db_migrate(REPO_ROOT, args.config, dry_run=mode["dry_run"])
        else:
            if verb == "plan":
                data, warnings, errors = kernel(
                    REPO_ROOT, args.config, args.def_file,
                    only=args.only if args.only else None, agent=args.agent,
                    **({"action": args.action} if domain == "agents" else {}),
                )
            else:
                data, warnings, errors = kernel(REPO_ROOT, args.config, args.def_file)
    except RefreshUsageError as exc:
        return usage_error("USAGE_ERROR", str(exc), "错误: %s" % exc)
    except ApplyGateError as exc:
        return usage_error("UNINSTALL_REQUIRES_FLAG", str(exc), "错误: %s" % exc)
    except policy_layers.PolicySecretGateError as exc:
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic("POLICY_SECRET_DENIED", None, str(exc))],
        )
        emit_envelope(payload)
        print("错误: %s" % exc, file=sys.stderr)
        return 1
    except policy_layers.PolicyKeyNotFoundError as exc:
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic("POLICY_KEY_NOT_FOUND", None, str(exc))],
        )
        emit_envelope(payload)
        print("错误: %s" % exc, file=sys.stderr)
        return 1
    except policy_layers.PolicySetError as exc:
        return usage_error("USAGE_ERROR", str(exc), "错误: %s" % exc)
    except RunNotFoundError as exc:
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic("RUN_NOT_FOUND", None, str(exc))],
        )
        emit_envelope(payload)
        print("错误: %s" % exc, file=sys.stderr)
        return 1
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
