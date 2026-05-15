from __future__ import annotations

import copy


AGENT_METADATA = {
    "claude-code": {
        "display_name": "Claude Code",
        "aliases": ["Claude Code"],
    },
    "gemini-cli": {
        "display_name": "Gemini CLI",
        "aliases": ["Gemini CLI"],
    },
    "codex": {
        "display_name": "Codex",
        "aliases": ["Codex"],
    },
}


ALIAS_TO_ID = {}
for agent_id, metadata in AGENT_METADATA.items():
    ALIAS_TO_ID[agent_id] = agent_id
    for alias in metadata.get("aliases", []):
        ALIAS_TO_ID[alias] = agent_id


def deep_merge(dst, src):
    for key, value in src.items():
        if isinstance(value, dict) and isinstance(dst.get(key), dict):
            deep_merge(dst[key], value)
        else:
            dst[key] = value
    return dst


def canonicalize_agent_id(agent_id):
    return ALIAS_TO_ID.get(agent_id, agent_id)


def canonicalize_agent_list(agent_ids):
    if agent_ids is None:
        return None
    return [canonicalize_agent_id(agent_id) for agent_id in agent_ids]


def canonicalize_agent_mapping(mapping):
    normalized = {}
    for raw_key, raw_value in (mapping or {}).items():
        agent_id = canonicalize_agent_id(raw_key)
        value = copy.deepcopy(raw_value)
        if agent_id in normalized:
            existing = normalized[agent_id]
            if isinstance(existing, dict) and isinstance(value, dict):
                normalized[agent_id] = deep_merge(existing, value)
                continue
            if existing != value:
                raise ValueError(f"Conflicting values for agent id '{agent_id}'")
        else:
            normalized[agent_id] = value
    return normalized


def get_agent_display_name(agent_id, definition=None):
    if definition and definition.get("display_name"):
        return definition["display_name"]
    canonical_id = canonicalize_agent_id(agent_id)
    return AGENT_METADATA.get(canonical_id, {}).get("display_name", canonical_id)
