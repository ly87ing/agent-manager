from __future__ import annotations

from src.agent_ids import canonicalize_agent_id, canonicalize_agent_mapping


def resolve_mcp_server_args(settings: dict, agent_id: str) -> list[str]:
    base_args = settings.get("args", [])
    if not isinstance(base_args, list) or not all(
        isinstance(arg, str) for arg in base_args
    ):
        raise ValueError("args 必须是字符串数组")

    raw_extra_args = settings.get("extra_args_by_agent", {})
    if not isinstance(raw_extra_args, dict):
        raise ValueError("extra_args_by_agent 必须是对象")

    extra_args_by_agent = canonicalize_agent_mapping(raw_extra_args)
    for target_agent, extra_args in extra_args_by_agent.items():
        if not isinstance(extra_args, list) or not all(
            isinstance(arg, str) for arg in extra_args
        ):
            raise ValueError(f"extra_args_by_agent.{target_agent} 必须是字符串数组")

    return [
        *base_args,
        *extra_args_by_agent.get(canonicalize_agent_id(agent_id), []),
    ]
