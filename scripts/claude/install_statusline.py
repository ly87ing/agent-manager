#!/usr/bin/env python3
"""Install the reference Claude statusline helper to the user's ~/.claude/.

Usage:
    install_statusline.py [--dry-run] [--target PATH]
                          [--apply-settings [--statusline-command CMD]]

The shell-helper installer is idempotent: it only writes when the destination
is missing or its content differs.

`--apply-settings` additionally patches ~/.claude/settings.json's
`statusLine` block atomically. Other top-level keys (e.g. `env`, `model`)
are preserved untouched. This is the only path that mutates settings.json;
omit it to keep the historical hands-off behavior.
"""
from __future__ import annotations

import argparse
import json
import logging
import os
import shutil
import stat
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
SOURCE_PATH = SCRIPT_DIR / "statusline.sh"
DEFAULT_TARGET = Path.home() / ".claude" / "statusline.sh"
MODE = 0o755


def _configure_logging() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")


def _atomic_install(source: Path, target: Path, mode: int) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        "wb",
        delete=False,
        dir=str(target.parent),
        prefix=target.name + ".",
        suffix=".tmp",
    ) as tmp:
        tmp.write(source.read_bytes())
        tmp_path = Path(tmp.name)
    os.chmod(tmp_path, mode)
    os.replace(tmp_path, target)


def install_statusline(target: Path, *, dry_run: bool = False) -> bool:
    if not SOURCE_PATH.is_file():
        raise FileNotFoundError(f"reference statusline missing: {SOURCE_PATH}")
    source_bytes = SOURCE_PATH.read_bytes()

    needs_write = True
    needs_chmod = True
    if target.is_file():
        if target.read_bytes() == source_bytes:
            needs_write = False
        current_mode = stat.S_IMODE(target.stat().st_mode)
        if current_mode == MODE:
            needs_chmod = False

    if not needs_write and not needs_chmod:
        logging.info(f"[Claude StatusLine] 已是最新: {target}")
        return False

    if dry_run:
        action = "写入" if needs_write else "修正权限"
        logging.info(f"[Claude StatusLine] (预览: {action} {target})")
        return True

    if needs_write:
        _atomic_install(SOURCE_PATH, target, MODE)
        logging.info(f"[Claude StatusLine] 已写入: {target}")
    else:
        os.chmod(target, MODE)
        logging.info(f"[Claude StatusLine] 已修正权限: {target}")
    return True


def _atomic_write_json(target: Path, payload: dict, dry_run: bool) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    rendered = json.dumps(payload, indent=2, ensure_ascii=False) + "\n"
    if dry_run:
        return
    with tempfile.NamedTemporaryFile(
        "w",
        delete=False,
        dir=str(target.parent),
        prefix=target.name + ".",
        suffix=".tmp",
        encoding="utf-8",
    ) as tmp:
        tmp.write(rendered)
        tmp_path = Path(tmp.name)
    os.chmod(tmp_path, 0o600)
    os.replace(tmp_path, target)


def apply_settings(
    settings_path: Path,
    statusline_command: str,
    *,
    theme: str | None = None,
    dry_run: bool = False,
) -> bool:
    """Patch `statusLine` (and optionally `theme`) in settings.json.

    Top-level keys other than the managed ones are preserved verbatim.
    Returns True when changes were (or would be) applied, False when the file
    is already in the desired state.
    """
    if settings_path.exists():
        try:
            existing = json.loads(settings_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            raise SystemExit(
                f"[Claude Settings] 解析 {settings_path} 失败: {exc}"
            ) from exc
        if not isinstance(existing, dict):
            raise SystemExit(
                f"[Claude Settings] {settings_path} 顶层不是对象，拒绝写入"
            )
    else:
        existing = {}

    desired_status = {"type": "command", "command": statusline_command, "padding": 0}
    status_matches = existing.get("statusLine") == desired_status
    theme_matches = theme is None or existing.get("theme") == theme
    if status_matches and theme_matches:
        logging.info(f"[Claude Settings] 已是最新: {settings_path}")
        return False

    if dry_run:
        bits = [f"statusLine={statusline_command}"]
        if theme is not None:
            bits.append(f"theme={theme}")
        logging.info(
            f"[Claude Settings] (预览: 设 {', '.join(bits)} → {settings_path})"
        )
        return True

    existing["statusLine"] = desired_status
    if theme is not None:
        existing["theme"] = theme
    _atomic_write_json(settings_path, existing, dry_run=False)
    logging.info(f"[Claude Settings] 已更新 statusLine/theme -> {settings_path}")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Install the reference Claude statusline helper")
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不实际写入")
    parser.add_argument(
        "--target",
        default=str(DEFAULT_TARGET),
        help=f"目标路径，默认 {DEFAULT_TARGET}",
    )
    parser.add_argument(
        "--apply-settings",
        action="store_true",
        help="同时合并 statusLine 块到 ~/.claude/settings.json（保留其它字段）",
    )
    parser.add_argument(
        "--settings-path",
        default=None,
        help="settings.json 路径，默认 $HOME/.claude/settings.json",
    )
    parser.add_argument(
        "--statusline-command",
        default=None,
        help="settings.json.statusLine.command 的值，默认 = --target 解析后的绝对路径",
    )
    parser.add_argument(
        "--theme",
        default=None,
        help="同时设置 settings.json.theme，例如 dark-daltonized / dark-ansi",
    )
    args = parser.parse_args()

    _configure_logging()
    target = Path(os.path.expanduser(args.target)).resolve()
    changed = install_statusline(target, dry_run=args.dry_run)

    if args.apply_settings:
        settings_path = (
            Path(os.path.expanduser(args.settings_path)).resolve()
            if args.settings_path
            else Path(os.path.expanduser("~/.claude/settings.json"))
        )
        command_value = args.statusline_command or str(target)
        apply_settings(
            settings_path,
            command_value,
            theme=args.theme,
            dry_run=args.dry_run,
        )
        return

    if not changed:
        return
    rel_hint = "~/.claude/statusline.sh"
    logging.info(
        "\n下一步：编辑 ~/.claude/settings.json，把 statusLine.command 设为:\n"
        f'    "{rel_hint}"\n'
        "若已是该值则无需改动。\n"
        "也可使用 --apply-settings 由本脚本原子合并（保留其它字段）。"
    )


if __name__ == "__main__":
    main()
