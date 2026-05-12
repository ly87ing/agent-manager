#!/usr/bin/env python3
"""Install the reference Claude statusline helper to the user's ~/.claude/.

Usage:
    install_statusline.py [--dry-run] [--target PATH]

The installer is idempotent: it only writes when the destination is missing
or its content differs. It never touches ~/.claude/settings.json — that file
contains personal tokens and is left to the user to update.
"""
from __future__ import annotations

import argparse
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


def main() -> None:
    parser = argparse.ArgumentParser(description="Install the reference Claude statusline helper")
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不实际写入")
    parser.add_argument(
        "--target",
        default=str(DEFAULT_TARGET),
        help=f"目标路径，默认 {DEFAULT_TARGET}",
    )
    args = parser.parse_args()

    _configure_logging()
    target = Path(os.path.expanduser(args.target)).resolve()
    changed = install_statusline(target, dry_run=args.dry_run)

    if not changed:
        return
    rel_hint = "~/.claude/statusline.sh"
    logging.info(
        "\n下一步：编辑 ~/.claude/settings.json，把 statusLine.command 设为:\n"
        f'    "{rel_hint}"\n'
        "若已是该值则无需改动。"
    )


if __name__ == "__main__":
    main()
