#!/usr/bin/env python3
"""Merge Claude Code credentials from secrets/claude.env into ~/.claude/settings.json.

The settings.json `env` block has the keys declared in config/claude-env.json
replaced with values resolved from secrets/claude.env (or the process env, as
a fallback). All other top-level fields and any env keys outside the declared
set are preserved verbatim.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.json_config import atomic_write_json_file, load_json_object_file  # noqa: E402
from src.runtime_logging import configure_logging  # noqa: E402

configure_logging()

DEFAULT_CONFIG = REPO_ROOT / "config" / "claude-env.json"
DEFAULT_SECRETS = REPO_ROOT / "secrets" / "claude.env"
PLACEHOLDER_PATTERN = re.compile(r"^\$\{([A-Z0-9_]+)\}$")


def load_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    vars_: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        vars_[key.strip()] = value.strip().strip('"').strip("'")
    return vars_


def resolve_placeholder(template: str, env_vars: dict[str, str]) -> tuple[str | None, str | None]:
    """Return (resolved_value, missing_var). missing_var is set only on failure."""
    match = PLACEHOLDER_PATTERN.match(template)
    if not match:
        return template, None
    var_name = match.group(1)
    if var_name in env_vars:
        return env_vars[var_name], None
    if var_name in os.environ and os.environ[var_name]:
        return os.environ[var_name], None
    return None, var_name


def merge_env_block(existing_settings: dict, declared_env: dict[str, str], resolved: dict[str, str]) -> dict:
    new_settings = dict(existing_settings)
    current_env = new_settings.get("env")
    if not isinstance(current_env, dict):
        current_env = {}
    merged_env = dict(current_env)
    merged_env.update(resolved)
    new_settings["env"] = merged_env
    return new_settings


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Sync Claude Code credentials from secrets/claude.env into ~/.claude/settings.json"
    )
    parser.add_argument("--config", default=str(DEFAULT_CONFIG))
    parser.add_argument("--secrets", default=str(DEFAULT_SECRETS))
    parser.add_argument("--settings-path", default=None, help="覆盖目标 settings.json 路径")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    config_path = Path(args.config)
    try:
        config = load_json_object_file(config_path, strict=True, missing_ok=False)
    except ValueError as exc:
        logging.error(f"[CLAUDE Env] 配置无效: {exc}")
        return 1

    declared_env = config.get("env")
    if not isinstance(declared_env, dict) or not declared_env:
        logging.error(f"[CLAUDE Env] config.env 必须是非空对象: {config_path}")
        return 1

    settings_path_raw = args.settings_path or config.get("settings_path") or "~/.claude/settings.json"
    settings_path = Path(os.path.expanduser(str(settings_path_raw)))

    secrets_path = Path(args.secrets)
    env_vars = load_env_file(secrets_path)

    resolved: dict[str, str] = {}
    missing: list[str] = []
    for key, template in declared_env.items():
        if not isinstance(template, str):
            logging.error(f"[CLAUDE Env] config.env.{key} 必须是字符串占位符")
            return 1
        value, missing_var = resolve_placeholder(template, env_vars)
        if value is None:
            missing.append(missing_var or template)
        else:
            resolved[key] = value

    if missing:
        logging.error(
            "[CLAUDE Env] 缺少凭证: "
            + ", ".join(missing)
            + f"。请在 {secrets_path} 或系统环境补齐。"
        )
        return 1

    existing_settings: dict = {}
    if settings_path.exists():
        try:
            existing_settings = load_json_object_file(settings_path, strict=True, missing_ok=False)
        except ValueError as exc:
            logging.error(f"[CLAUDE Env] {settings_path} 解析失败: {exc}")
            return 1

    new_settings = merge_env_block(existing_settings, declared_env, resolved)
    if new_settings == existing_settings:
        logging.info(f"[CLAUDE Env] 已是最新: {settings_path}")
        return 0

    if args.dry_run:
        logging.info(
            f"[CLAUDE Env] (预览: 注入 {', '.join(resolved.keys())} -> {settings_path})"
        )
        return 0

    atomic_write_json_file(settings_path, new_settings, chmod=0o600)
    logging.info(f"[CLAUDE Env] 已写入: {settings_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
