#!/usr/bin/env python3
"""Install the repo-tracked ccstatusline config to the user's ~/.config/.

Usage:
    install_ccstatusline_config.py [--dry-run] [--target PATH] [--source PATH]

The repo carries the canonical ccstatusline `settings.json` next to this
script. This installer writes it atomically into `$XDG_CONFIG_HOME` (or
`~/.config/ccstatusline/settings.json`), preserving byte-for-byte fidelity
so the rendered statusline matches the snapshot under version control.

Idempotent: re-runs are no-ops when the target already matches the source.
"""
from __future__ import annotations

import argparse
import logging
import os
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
SOURCE_PATH = SCRIPT_DIR / "ccstatusline.settings.json"


def _configure_logging() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")


def _default_target() -> Path:
    xdg = os.environ.get("XDG_CONFIG_HOME")
    base = Path(xdg) if xdg else Path.home() / ".config"
    return base / "ccstatusline" / "settings.json"


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


def install_ccstatusline_config(
    source: Path, target: Path, *, dry_run: bool = False, mode: int = 0o600
) -> bool:
    if not source.is_file():
        raise FileNotFoundError(f"reference ccstatusline config missing: {source}")
    source_bytes = source.read_bytes()

    if target.is_file() and target.read_bytes() == source_bytes:
        logging.info(f"[ccstatusline Config] 已是最新: {target}")
        return False

    if dry_run:
        logging.info(f"[ccstatusline Config] (预览: 写入 {target})")
        return True

    _atomic_install(source, target, mode)
    logging.info(f"[ccstatusline Config] 已写入: {target}")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Install the repo-tracked ccstatusline settings.json"
    )
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不实际写入")
    parser.add_argument(
        "--target",
        default=None,
        help="目标路径，默认 $XDG_CONFIG_HOME/ccstatusline/settings.json",
    )
    parser.add_argument(
        "--source",
        default=str(SOURCE_PATH),
        help=f"快照路径，默认 {SOURCE_PATH}",
    )
    args = parser.parse_args()

    _configure_logging()
    source = Path(os.path.expanduser(args.source)).resolve()
    target = (
        Path(os.path.expanduser(args.target)).resolve()
        if args.target
        else _default_target()
    )
    try:
        install_ccstatusline_config(source, target, dry_run=args.dry_run)
    except FileNotFoundError as exc:
        logging.error(str(exc))
        sys.exit(1)


if __name__ == "__main__":
    main()
