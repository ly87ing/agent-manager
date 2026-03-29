#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import logging
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.json_config import atomic_write_json_file, load_json_object_file
from src.runtime_logging import configure_logging

configure_logging()


def _validate_hook_entry(entry: object, source_name: str) -> dict:
    if not isinstance(entry, dict):
        raise ValueError(f"{source_name} 条目必须是对象")
    hooks = entry.get("hooks")
    if not isinstance(hooks, list) or not hooks:
        raise ValueError(f"{source_name}.hooks 必须是非空数组")
    if "matcher" in entry and not isinstance(entry["matcher"], str):
        raise ValueError(f"{source_name}.matcher 必须是字符串")
    for index, hook in enumerate(hooks):
        if not isinstance(hook, dict):
            raise ValueError(f"{source_name}.hooks[{index}] 必须是对象")
        if hook.get("type") != "command":
            raise ValueError(f"{source_name}.hooks[{index}].type 必须是 command")
        if not isinstance(hook.get("command"), str) or not hook["command"].strip():
            raise ValueError(f"{source_name}.hooks[{index}].command 必须是非空字符串")
    return copy.deepcopy(entry)


def _validate_hooks_map(raw_hooks: object) -> dict[str, list[dict]]:
    if not isinstance(raw_hooks, dict):
        raise ValueError("claude-hooks.json.hooks 必须是对象")
    validated: dict[str, list[dict]] = {}
    for event_name, entries in raw_hooks.items():
        if not isinstance(entries, list) or not entries:
            raise ValueError(f"claude-hooks.json.hooks.{event_name} 必须是非空数组")
        validated[event_name] = [
            _validate_hook_entry(entry, f"claude-hooks.json.hooks.{event_name}[{index}]")
            for index, entry in enumerate(entries)
        ]
    return validated


def _collect_managed_signatures(hooks: dict[str, list[dict]]) -> set[tuple[str, str | None, str]]:
    signatures: set[tuple[str, str | None, str]] = set()
    for event_name, entries in hooks.items():
        for entry in entries:
            matcher = entry.get("matcher")
            for hook in entry["hooks"]:
                signatures.add((event_name, matcher, hook["command"]))
    return signatures


def _strip_managed_hooks(existing_hooks: object, managed_signatures: set[tuple[str, str | None, str]]) -> dict:
    if not isinstance(existing_hooks, dict):
        return {}
    cleaned: dict = {}
    for event_name, entries in existing_hooks.items():
        if not isinstance(entries, list):
            cleaned[event_name] = copy.deepcopy(entries)
            continue
        kept_entries = []
        for entry in entries:
            if not isinstance(entry, dict):
                kept_entries.append(copy.deepcopy(entry))
                continue
            hooks = entry.get("hooks")
            if not isinstance(hooks, list):
                kept_entries.append(copy.deepcopy(entry))
                continue
            matcher = entry.get("matcher")
            kept_hooks = [
                copy.deepcopy(hook)
                for hook in hooks
                if not (
                    isinstance(hook, dict)
                    and (event_name, matcher, hook.get("command")) in managed_signatures
                )
            ]
            if kept_hooks:
                new_entry = copy.deepcopy(entry)
                new_entry["hooks"] = kept_hooks
                kept_entries.append(new_entry)
        if kept_entries:
            cleaned[event_name] = kept_entries
    return cleaned


def build_managed_hooks(config_path: Path) -> tuple[Path, dict[str, list[dict]]]:
    config = load_json_object_file(config_path, strict=True, missing_ok=False)
    settings_rel = config.get("settings_path")
    if not isinstance(settings_rel, str) or not settings_rel.strip():
        raise ValueError("claude-hooks.json.settings_path 必须是非空字符串")
    hooks = _validate_hooks_map(config.get("hooks"))
    settings_path = Path(settings_rel)
    if not settings_path.is_absolute():
        settings_path = config_path.resolve().parent / settings_path
    return settings_path, hooks


def sync_claude_hooks(config_path: Path, *, settings_override: Path | None = None, dry_run: bool = False) -> Path:
    settings_path, managed_hooks = build_managed_hooks(config_path)
    if settings_override is not None:
        settings_path = settings_override

    existing_settings = load_json_object_file(settings_path, strict=False, missing_ok=True)
    merged_settings = copy.deepcopy(existing_settings)
    managed_signatures = _collect_managed_signatures(managed_hooks)
    merged_hooks = _strip_managed_hooks(existing_settings.get("hooks", {}), managed_signatures)
    for event_name, entries in managed_hooks.items():
        merged_hooks.setdefault(event_name, [])
        merged_hooks[event_name].extend(copy.deepcopy(entries))
    merged_settings["hooks"] = merged_hooks

    if dry_run:
        logging.info(f"[CLAUDE Hooks] (预览: 计划写入 {settings_path})")
        return settings_path

    atomic_write_json_file(settings_path, merged_settings)
    logging.info(f"[CLAUDE Hooks] 已写入: {settings_path}")
    return settings_path


def main() -> None:
    parser = argparse.ArgumentParser(description="Sync managed Claude hooks into project .claude/settings.json")
    parser.add_argument(
        "--config",
        default=str(REPO_ROOT / "config" / "claude-hooks.json"),
        help="claude-hooks.json 路径",
    )
    parser.add_argument("--settings-path", help="覆盖输出的 Claude settings.json 路径")
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不实际写入")
    args = parser.parse_args()

    config_path = Path(args.config)
    settings_override = Path(args.settings_path) if args.settings_path else None
    sync_claude_hooks(config_path, settings_override=settings_override, dry_run=args.dry_run)


if __name__ == "__main__":
    main()
