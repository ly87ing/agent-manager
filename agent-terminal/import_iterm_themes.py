#!/usr/bin/env python3
"""Idempotent iTerm2 color-preset importer (no GUI dialogs).

Vendors `agent-terminal/iterm-themes/*.itermcolors` into iTerm2's `Custom
Color Presets` plist key via `defaults export | defaults import`. This path
goes through cfprefsd, bypassing iTerm's import dialog entirely — including
the "duplicate preset" confirmation that fires when content matches an
existing preset under a different name.

Skip rules (in order):
  1. Name already present in the plist → skip.
  2. Content already present under any other name → skip and log the match.

iTerm should be quit before running, otherwise the daemon's in-memory copy
may overwrite new presets on iTerm's next preference flush. The script warns
when iTerm is running and exits successfully so callers can decide.

Usage:
    import_iterm_themes.py [--dry-run]
"""
from __future__ import annotations

import argparse
import logging
import os
import platform
import plistlib
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
THEMES_DIR = SCRIPT_DIR / "iterm-themes"
PLIST_DOMAIN = "com.googlecode.iterm2"
PRESETS_KEY = "Custom Color Presets"


def configure_logging() -> None:
    if not logging.getLogger().handlers:
        logging.basicConfig(level=logging.INFO, format="%(message)s")


def is_macos() -> bool:
    if os.environ.get("AGENT_MANAGER_ITERM_FORCE_NON_MACOS"):
        return False
    if os.environ.get("AGENT_MANAGER_ITERM_FORCE_MACOS"):
        return True
    return platform.system() == "Darwin"


def is_iterm_running() -> bool:
    try:
        proc = subprocess.run(
            ["pgrep", "-x", "iTerm2"], capture_output=True, check=False
        )
    except FileNotFoundError:
        return False
    return proc.returncode == 0


def export_iterm_domain() -> dict:
    """Dump the entire iTerm preference domain via `defaults export`.

    Returns `{}` if the domain is empty or `defaults` is unavailable.
    """
    try:
        proc = subprocess.run(
            ["defaults", "export", PLIST_DOMAIN, "-"],
            capture_output=True, check=False,
        )
    except FileNotFoundError:
        return {}
    if proc.returncode != 0 or not proc.stdout:
        return {}
    try:
        return plistlib.loads(proc.stdout)
    except Exception as exc:
        logging.warning(f"plist 解析失败: {exc}")
        return {}


def import_iterm_domain(domain_data: dict) -> bool:
    payload = plistlib.dumps(domain_data)
    try:
        subprocess.run(
            ["defaults", "import", PLIST_DOMAIN, "-"],
            input=payload, capture_output=True, check=True,
        )
    except subprocess.CalledProcessError as exc:
        stderr = (exc.stderr or b"").decode("utf-8", "replace")
        logging.error(f"defaults import 失败: {stderr or exc}")
        return False
    return True


def compute_missing_presets(theme_files, existing_names: set[str]) -> list[Path]:
    """Return theme files whose stem is not in `existing_names`, in input order."""
    return [f for f in theme_files if f.stem not in existing_names]


def find_content_collision(
    color_map: dict, existing_presets: dict
) -> str | None:
    """Return the existing preset name whose color map equals `color_map`, or None."""
    for name, preset in existing_presets.items():
        if preset == color_map:
            return name
    return None


def run_import(dry_run: bool = False) -> bool:
    if not is_macos():
        logging.info("[ITERM THEMES] 非 macOS 环境，跳过。")
        return True

    if not THEMES_DIR.is_dir():
        logging.warning(f"[ITERM THEMES] 主题目录不存在: {THEMES_DIR}")
        return True

    theme_files = sorted(THEMES_DIR.glob("*.itermcolors"))
    if not theme_files:
        logging.info("[ITERM THEMES] 主题目录为空，跳过。")
        return True

    domain = export_iterm_domain()
    existing_presets: dict = dict(domain.get(PRESETS_KEY) or {})

    missing_by_name = compute_missing_presets(theme_files, set(existing_presets.keys()))
    if not missing_by_name:
        logging.info(f"[ITERM THEMES] 已全部导入（{len(theme_files)} 个主题）。")
        return True

    to_insert: list[tuple[Path, dict]] = []
    for theme_file in missing_by_name:
        try:
            color_map = plistlib.loads(theme_file.read_bytes())
        except Exception as exc:
            logging.error(f"  解析失败 {theme_file.name}: {exc}")
            return False
        collision = find_content_collision(color_map, existing_presets)
        if collision:
            logging.info(
                f"  跳过 {theme_file.stem}: 内容与现有 \"{collision}\" 相同"
            )
            continue
        to_insert.append((theme_file, color_map))

    if not to_insert:
        logging.info(
            f"[ITERM THEMES] 名字缺失但内容均已存在，无需写入（{len(theme_files)} 个主题）。"
        )
        return True

    logging.info(f"[ITERM THEMES] 待导入 {len(to_insert)}/{len(theme_files)} 个主题")

    if dry_run:
        for theme_file, _ in to_insert:
            logging.info(f"  (预览: 计划合并 {theme_file.name})")
        return True

    if is_iterm_running():
        logging.warning(
            "  iTerm 正在运行：preset 可能被 iTerm 在退出时回写覆盖。建议退出 iTerm 后再次运行。"
        )

    presets = dict(existing_presets)
    for theme_file, color_map in to_insert:
        presets[theme_file.stem] = color_map
        logging.info(f"  已合并: {theme_file.name}")
    domain[PRESETS_KEY] = presets

    if not import_iterm_domain(domain):
        return False
    logging.info(f"[ITERM THEMES] 写入完成（新增 {len(to_insert)} 个）。")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="iTerm2 color preset importer")
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不写入 plist")
    args = parser.parse_args()
    configure_logging()
    ok = run_import(dry_run=args.dry_run)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
