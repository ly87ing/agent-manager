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
        raise ValueError("codex-hooks.json.hooks 必须是对象")
    validated: dict[str, list[dict]] = {}
    for event_name, entries in raw_hooks.items():
        if not isinstance(entries, list) or not entries:
            raise ValueError(f"codex-hooks.json.hooks.{event_name} 必须是非空数组")
        validated[event_name] = [
            _validate_hook_entry(entry, f"codex-hooks.json.hooks.{event_name}[{index}]")
            for index, entry in enumerate(entries)
        ]
    return validated


def build_managed_hooks(config_path: Path) -> tuple[Path, dict[str, list[dict]]]:
    config = load_json_object_file(config_path, strict=True, missing_ok=False)
    hooks_rel = config.get("hooks_path")
    if not isinstance(hooks_rel, str) or not hooks_rel.strip():
        raise ValueError("codex-hooks.json.hooks_path 必须是非空字符串")
    hooks = _validate_hooks_map(config.get("hooks"))
    hooks_path = Path(hooks_rel)
    if not hooks_path.is_absolute():
        hooks_path = config_path.resolve().parent / hooks_path
    return hooks_path, hooks


def sync_codex_hooks(config_path: Path, *, hooks_override: Path | None = None, dry_run: bool = False) -> Path:
    hooks_path, managed_hooks = build_managed_hooks(config_path)
    if hooks_override is not None:
        hooks_path = hooks_override

    payload = {"hooks": managed_hooks}
    if dry_run:
        logging.info(f"[CODEX Hooks] (预览: 计划写入 {hooks_path})")
        return hooks_path

    atomic_write_json_file(hooks_path, payload)
    logging.info(f"[CODEX Hooks] 已写入: {hooks_path}")
    return hooks_path


def main() -> None:
    parser = argparse.ArgumentParser(description="Sync managed Codex hooks into project .codex/hooks.json")
    parser.add_argument(
        "--config",
        default=str(REPO_ROOT / "config" / "codex-hooks.json"),
        help="codex-hooks.json 路径",
    )
    parser.add_argument("--hooks-path", help="覆盖输出的 Codex hooks.json 路径")
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不实际写入")
    args = parser.parse_args()

    config_path = Path(args.config)
    hooks_override = Path(args.hooks_path) if args.hooks_path else None
    sync_codex_hooks(config_path, hooks_override=hooks_override, dry_run=args.dry_run)


if __name__ == "__main__":
    main()
