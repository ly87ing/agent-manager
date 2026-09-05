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
        if isinstance(value, dict) and isinstance(dst.get(key), dict):
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
    atomic_write_json_file(path, data, chmod=0o600)
    return path
