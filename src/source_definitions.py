"""Edit the source documents used by rules, hooks and aliases, never generated output."""
from __future__ import annotations

import hashlib
import json
import os
import re
import tempfile
from pathlib import Path

from src.json_config import atomic_write_json_file, load_json_object_file
from src.secret_scan import scan_text_for_secrets
from src import policy_layers

SOURCES = {
    "rules.shared": ("agent-rule/rules/src/shared/core.md", None, "共享规则"),
    "rules.claude": ("agent-rule/rules/src/agents/claude/adapter.md", None, "Claude 规则"),
    "rules.codex": ("agent-rule/rules/src/agents/codex/adapter.md", None, "Codex 规则"),
    "hooks.claude": ("config/claude-hooks.json", "hooks", "Claude Hooks"),
    "hooks.codex": ("config/codex-hooks.json", "hooks", "Codex Hooks"),
    "aliases.shell": ("config/shell-aliases.json", "aliases", "Shell aliases"),
}


def source_path(root, key):
    if key not in SOURCES:
        raise ValueError("未知可编辑来源")
    path = Path(root).resolve() / SOURCES[key][0]
    if path.resolve() != path.absolute() or not path.is_file():
        raise ValueError("来源文件不存在或是符号链接")
    return path


def get_source(root, key):
    path = source_path(root, key)
    content = path.read_text(encoding="utf-8")
    if scan_text_for_secrets(content, file_label=key):
        raise ValueError("来源包含疑似明文敏感信息，拒绝显示")
    field = SOURCES[key][1]
    value = load_json_object_file(path, strict=True)[field] if field else content
    return {"key": key, "label": SOURCES[key][2], "value": value,
            "revision": hashlib.sha256(content.encode()).hexdigest(), "format": "json" if field else "text"}


def set_source(root, key, request, *, snapshot, validate_hooks, dry_run=False):
    current = get_source(root, key)
    if not isinstance(request, dict) or request.get("revision") != current["revision"]:
        raise ValueError("来源已改变，请重新加载后再编辑")
    value = request.get("value")
    field = SOURCES[key][1]
    if field:
        if not isinstance(value, dict):
            raise ValueError("定义必须是 JSON 对象；移除全部条目请使用 {}")
        if field == "aliases" and any(not isinstance(v, str) or not v.strip() for v in value.values()):
            raise ValueError("alias 命令必须是非空字符串")
        if field == "aliases" and any(not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", name) for name in value):
            raise ValueError("alias 名称只能使用字母、数字、下划线和连字符")
        if field == "hooks":
            validate_hooks(value)
    elif not isinstance(value, str):
        raise ValueError("规则内容必须是文本；清空内容可移除该组规则")
    if scan_text_for_secrets(json.dumps(value, ensure_ascii=False), file_label=key):
        raise ValueError("疑似明文敏感信息，拒绝保存")
    path = source_path(root, key)
    if field:
        document = load_json_object_file(path, strict=True)
        if key == "hooks.claude":
            # Preserve exact signatures so the next sync can remove changed/deleted hooks.
            retired = document.setdefault("removed_signatures", [])
            active = {(event, entry.get("matcher"), hook["command"]) for event, entries in value.items() for entry in entries for hook in entry["hooks"]}
            for event, entries in current["value"].items():
                for entry in entries:
                    for hook in entry["hooks"]:
                        signature = {"event": event, "matcher": entry.get("matcher"), "command": hook["command"]}
                        if (event, entry.get("matcher"), hook["command"]) not in active and signature not in retired:
                            retired.append(signature)
        document[field] = value
    if dry_run:
        return current
    local_path = policy_layers.layer_paths(root)["local"]
    local_config = load_json_object_file(local_path, strict=True)
    edited_sources = local_config.get("source_edits", [])
    if not isinstance(edited_sources, list) or any(not isinstance(item, str) for item in edited_sources):
        raise ValueError("source_edits 必须是字符串数组")
    snapshot([str(path), str(local_path)])
    policy_layers.set_policy_value(root, "local", "source_edits", sorted(set(edited_sources) | {SOURCES[key][0]}))
    if field:
        atomic_write_json_file(path, document)
    else:
        fd, temp_name = tempfile.mkstemp(prefix=".rule-", dir=path.parent)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                handle.write(value)
            os.chmod(temp_name, path.stat().st_mode & 0o777)
            os.replace(temp_name, path)
        finally:
            Path(temp_name).unlink(missing_ok=True)
    return get_source(root, key)
