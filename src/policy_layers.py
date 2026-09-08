"""Three-layer policy stack: baseline + synced + local (proposal section 7.2).

Merge order and semantics are byte-compatible with manage.sh's inline merge
(base ⊕ synced ⊕ local, dicts deep-merge, everything else overrides, agent ids
canonicalized at the end). Layer file locations honor the same env overrides as
manage.sh: AGENT_MANAGER_SYNCED_CONFIG / AGENT_MANAGER_LOCAL_CONFIG.

policy.set writes only to synced/local (baseline stays read-only); the value is
scanned for suspected plaintext secrets before writing (hard gate, same pattern
set as scripts/check_no_secrets.py), and key path segments are whitelisted.
"""
from __future__ import annotations

import copy
import json
import os
import re
from pathlib import Path

from src.agent_ids import canonicalize_agent_mapping
from src.json_config import atomic_write_json_file, load_json_object_file
from src.secret_scan import scan_text_for_secrets

SYNCED_CONFIG_ENV = "AGENT_MANAGER_SYNCED_CONFIG"
LOCAL_CONFIG_ENV = "AGENT_MANAGER_LOCAL_CONFIG"
WRITABLE_LAYERS = ("synced", "local")
LAYER_NAMES = ("baseline", "synced", "local")
_KEY_SEGMENT_RE = re.compile(r"^[A-Za-z0-9_-]+$")


class PolicySetError(ValueError):
    """Usage-level policy.set rejection (bad layer / bad key / bad value JSON)."""


class PolicySecretGateError(ValueError):
    """The value failed the suspected-plaintext gate; nothing was written."""


def layer_paths(root_dir):
    root_dir = Path(root_dir)
    return {
        "baseline": Path(os.environ.get("AGENT_MANAGER_BASE_CONFIG", root_dir / "config" / "manager.json")),
        "synced": Path(os.environ.get(SYNCED_CONFIG_ENV, root_dir / "config" / "manager.synced.json")),
        "local": Path(os.environ.get(LOCAL_CONFIG_ENV, root_dir / "config" / "manager.local.json")),
    }


def load_layers(root_dir):
    """Return {layer: object|None}; None means the layer file does not exist."""
    layers = {}
    for name, path in layer_paths(root_dir).items():
        if path.exists():
            layers[name] = load_json_object_file(path, strict=True, missing_ok=False)
        else:
            layers[name] = None
    return layers


def deep_merge(dst, src):
    """manage.sh inline merge 的同语义实现：dict⊕dict 递归，其余覆盖。"""
    for key, value in src.items():
        if isinstance(value, dict) and value.get("_replace") is not True and isinstance(dst.get(key), dict):
            deep_merge(dst[key], value)
        else:
            dst[key] = value
    return dst


def effective_config(layers):
    merged = {}
    for name in LAYER_NAMES:
        if layers.get(name):
            deep_merge(merged, layers[name])
    if "agents" in merged:
        merged["agents"] = canonicalize_agent_mapping(merged.get("agents", {}))
    return merged


def attribute_top_level_keys(layers):
    """顶层键的层归属：最后（最高优先）包含该键的层。"""
    attribution = {}
    for name in LAYER_NAMES:
        layer = layers.get(name)
        if not isinstance(layer, dict):
            continue
        for key in layer:
            attribution[key] = name
    return attribution


def set_policy_value(root_dir, layer, key, value):
    """Write value at dot-path ``key`` into the synced/local layer file.

    Raises PolicySetError for usage problems and PolicySecretGateError when the
    gate trips; returns the layer file path on success.
    """
    if layer not in WRITABLE_LAYERS:
        raise PolicySetError("baseline 层只读；--layer 只能是 %s" % " / ".join(WRITABLE_LAYERS))
    segments = key.split(".")
    if not segments or any(not _KEY_SEGMENT_RE.match(segment) for segment in segments):
        raise PolicySetError(
            "非法 key 路径段（点号分隔，每段只允许字母/数字/_/-）: %s" % key
        )
    rendered = json.dumps(value, ensure_ascii=False)
    findings = scan_text_for_secrets(rendered, file_label="policy.set value")
    if findings:
        raise PolicySecretGateError(
            "疑似明文敏感信息（%s），拒绝写入策略层" % ", ".join(sorted({f.kind for f in findings}))
        )

    path = layer_paths(root_dir)[layer]
    data = load_json_object_file(path, strict=True, missing_ok=True)
    cursor = data
    for segment in segments[:-1]:
        nxt = cursor.get(segment)
        if not isinstance(nxt, dict):
            nxt = {}
            cursor[segment] = nxt
        cursor = nxt
    cursor[segments[-1]] = value
    definition_domains = {"agent_definitions": ("agents", "agents.json", None), "tool_definitions": ("tools", "tools.json", "tools")}
    if segments[0] in definition_domains:
        domain, filename, field = definition_domains[segments[0]]
        layers = load_layers(root_dir)
        layers[layer] = data
        effective = effective_config(layers)
        base = load_json_object_file(Path(root_dir) / "config" / filename, strict=True)
        apply_definition_overlays(base.get(field, {}) if field else base, overlay_definitions(effective, domain), domain=domain)
    atomic_write_json_file(path, data, chmod=0o600)
    return path


class PolicyKeyNotFoundError(ValueError):
    """policy:delete 指定的键在目标层不存在。"""


def delete_policy_value(root_dir, layer, key):
    """Delete dot-path ``key`` from the synced/local layer file.

    Raises PolicySetError for usage problems and PolicyKeyNotFoundError when
    the key is absent; returns the layer file path on success. 写前快照由
    调用方（引擎内核）负责（复用 src/engine_apply.py 的运行快照机制）。
    """
    if layer not in WRITABLE_LAYERS:
        raise PolicySetError("baseline 层只读；--layer 只能是 %s" % " / ".join(WRITABLE_LAYERS))
    segments = key.split(".")
    if not segments or any(not _KEY_SEGMENT_RE.match(segment) for segment in segments):
        raise PolicySetError(
            "非法 key 路径段（点号分隔，每段只允许字母/数字/_/-）: %s" % key
        )

    path = layer_paths(root_dir)[layer]
    data = load_json_object_file(path, strict=True, missing_ok=True)
    cursor = data
    for segment in segments[:-1]:
        nxt = cursor.get(segment)
        if not isinstance(nxt, dict):
            raise PolicyKeyNotFoundError("键不存在: %s" % key)
        cursor = nxt
    if segments[-1] not in cursor:
        raise PolicyKeyNotFoundError("键不存在: %s" % key)
    del cursor[segments[-1]]
    atomic_write_json_file(path, data, chmod=0o600)
    return path


# ---------------------------------------------------------------------------
# 定义覆盖层（definition overlay）：策略层携带定义片段，深合并到定义文件之上
# ---------------------------------------------------------------------------

# 各定义域的合法键与必填键（形状校验 fail-close 用；未知键告警不拒绝）。
DEFINITION_OVERLAY_RULES = {
    "agents": {
        "overlay_key": "agent_definitions",
        "required_key": "bin",
        "known_keys": {"display_name", "bin", "app_path", "preferred_provider", "require_managed_provider", "providers", "_delete"},
    },
    "tools": {
        "overlay_key": "tool_definitions",
        "required_key": None,
        "known_keys": {"bin", "url", "install", "upgrade", "setup", "doctor_severity", "detect", "_delete"},
    },
    "mcp": {
        "overlay_key": "definitions",
        "required_key": "command",
        "known_keys": {
            "command",
            "args",
            "env",
            "agents",
            "extra_args_by_agent",
            "startup_timeout_sec",
            "project_serena_folder_location",
            "url",
            "_delete",
        },
    },
    "skills": {
        "overlay_key": "repositories",
        "required_key": "url",
        "known_keys": {
            "url",
            "skill_paths",
            "skill_count",
            "agents",
            "new_skill_default",
            "_delete",
        },
    },
}


def apply_definition_overlays(base_defs, overlay_defs, *, domain):
    """base 定义 ⊕ 覆盖层片段（per-key 深合并，``_delete: true`` 为 tombstone）。

    - 片段非对象 / 合并后缺必填键（mcp: command 非空字符串；skills: url 非空
      字符串）→ fail-close ValueError；
    - 未知键 → 收集进返回值 warnings（告警不拒绝）；
    - tombstone 对基线条目同样生效（从有效集合排除）。
    返回 (merged, warnings)。
    """
    rules = DEFINITION_OVERLAY_RULES[domain]
    required_key = rules["required_key"]
    known_keys = rules["known_keys"]
    warnings = []
    merged = copy.deepcopy(base_defs)
    for name, fragment in (overlay_defs or {}).items():
        if not isinstance(fragment, dict):
            raise ValueError("%s 覆盖层片段 %s 必须是对象" % (domain, name))
        if fragment.get("_delete") is True:
            merged.pop(name, None)
            continue
        unknown = sorted(set(fragment) - known_keys - {"_replace"})
        if unknown:
            warnings.append(
                "%s 覆盖层片段 %s 含未知键（忽略生效语义之外的影响）: %s"
                % (domain, name, ", ".join(unknown))
            )
        if fragment.get("_replace") is True:
            merged.pop(name, None)
        fragment = {key: value for key, value in fragment.items() if key not in ("_delete", "_replace")}
        if name in merged and isinstance(merged[name], dict):
            deep_merge(merged[name], fragment)
        else:
            merged[name] = copy.deepcopy(fragment)
    for name, definition in merged.items():
        if not isinstance(definition, dict):
            raise ValueError("%s 定义 %s 必须是对象" % (domain, name))
        if domain == "agents" and name not in (overlay_defs or {}):
            continue
        if required_key is None:
            continue
        value = definition.get(required_key)
        if not isinstance(value, str) or not value.strip():
            raise ValueError(
                "%s 定义 %s 缺少合法必填键 %s（非空字符串；新增/覆盖后生效集合必须完整）"
                % (domain, name, required_key)
            )
    return merged, warnings


def overlay_definitions(config, domain):
    """从合并后策略配置取覆盖层片段（config.<domain>.<overlay_key>，缺省 {}）。"""
    rules = DEFINITION_OVERLAY_RULES[domain]
    if domain in ("agents", "tools"):
        overlay = config.get(rules["overlay_key"], {})
        if not isinstance(overlay, dict):
            raise ValueError("%s 必须是对象" % rules["overlay_key"])
        return overlay
    section = config.get(domain, {})
    if not isinstance(section, dict):
        return {}
    overlay = section.get(rules["overlay_key"], {})
    return overlay if isinstance(overlay, dict) else {}
