#!/usr/bin/env python3
"""Terminal 配置同步引擎。

读取 config/terminal.json，将模板渲染后原子写入用户家目录对应路径。
支持 --dry-run 和 --offline flag。
"""
from __future__ import annotations

import argparse
import logging
import os
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.json_config import load_json_object_file
from src.manage_runtime_probe import check_tool_ready
from src.runtime_logging import configure_logging
from src.template_utils import load_env_file, render_template

configure_logging()


def _check_tool_ready(tool_name: str, tool_defs: dict) -> bool:
    """检查工具是否已安装，复用 tools.json 中的 ready_check 字段。"""
    ready_check = (tool_defs.get(tool_name) or {}).get("ready_check")
    return check_tool_ready(tool_name, ready_check)


def _parse_chmod(chmod_str: str | None) -> int | None:
    """将 '0600' 形式的字符串转换为整数 mode，不合法时返回 None。"""
    if not chmod_str:
        return None
    try:
        return int(chmod_str, 8)
    except ValueError:
        logging.warning(f"无效的 chmod 值: {chmod_str}，跳过权限设置")
        return None


def _atomic_write_text(dest_path: Path, content: str, mode: int | None, dry_run: bool) -> bool:
    """将文本内容原子写入目标路径，可选设置文件权限。"""
    if dry_run:
        logging.info(f"  (预览: 计划写入 {dest_path})")
        return True
    dest_path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_path = tempfile.mkstemp(dir=str(dest_path.parent), prefix=f"{dest_path.name}.tmp_")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(content)
        if mode is not None:
            os.chmod(tmp_path, mode)
        os.replace(tmp_path, dest_path)
        logging.info(f"  已写入: {dest_path}")
        return True
    except Exception as exc:
        logging.error(f"  写入失败 {dest_path}: {exc}")
        if os.path.exists(tmp_path):
            os.remove(tmp_path)
        return False


def sync_terminal(config_path: Path, repo_root: Path, dry_run: bool = False, offline: bool = False) -> bool:
    """执行全部 terminal 配置目标的渲染与写入。"""
    cfg = load_json_object_file(config_path, strict=True, missing_ok=False)

    targets = cfg.get("targets", {})
    if not isinstance(targets, dict):
        logging.error("terminal.json: targets 必须是对象")
        return False

    tools_cfg = load_json_object_file(repo_root / "config" / "tools.json", strict=False, missing_ok=True)
    tool_defs: dict = tools_cfg.get("tools", {})

    secrets_rel = cfg.get("secrets_file", "secrets/terminal.env")
    secrets_path = repo_root / secrets_rel

    env: dict[str, str] = {}
    if secrets_path.exists():
        env = load_env_file(secrets_path)
        logging.debug(f"已加载 secrets: {secrets_path} ({len(env)} 个变量)")
    else:
        logging.warning(f"secrets 文件不存在，变量替换可能失败: {secrets_path}")

    all_ok = True
    for target_name, target_cfg in targets.items():
        if not isinstance(target_cfg, dict):
            logging.error(f"[TERMINAL: {target_name}] 配置必须是对象，跳过")
            all_ok = False
            continue

        enabled = target_cfg.get("enabled", True)
        if not isinstance(enabled, bool):
            logging.error(f"[TERMINAL: {target_name}] enabled 必须是布尔值，跳过")
            all_ok = False
            continue
        if not enabled:
            logging.info(f"[TERMINAL: {target_name}] 已禁用，跳过。")
            continue

        requires = target_cfg.get("requires", [])
        unmet = [t for t in requires if not _check_tool_ready(t, tool_defs)]
        if unmet:
            logging.info(f"[TERMINAL: {target_name}] 前置工具未安装 {unmet}，跳过。")
            continue

        tmpl_rel = target_cfg.get("template")
        dest_str = target_cfg.get("destination")
        chmod_str = target_cfg.get("chmod")

        if not tmpl_rel or not dest_str:
            logging.error(f"[TERMINAL: {target_name}] 缺少 template 或 destination 字段，跳过")
            all_ok = False
            continue

        tmpl_path = repo_root / tmpl_rel
        if not tmpl_path.exists():
            logging.error(f"[TERMINAL: {target_name}] 模板文件不存在: {tmpl_path}")
            all_ok = False
            continue

        tmpl_str = tmpl_path.read_text(encoding="utf-8")

        try:
            rendered = render_template(tmpl_str, env)
        except ValueError as exc:
            logging.error(f"[TERMINAL: {target_name}] 模板渲染失败: {exc}")
            all_ok = False
            continue

        dest_path = Path(os.path.expanduser(dest_str))
        mode = _parse_chmod(chmod_str)

        logging.info(f"[TERMINAL: {target_name}] 同步 {tmpl_rel} -> {dest_path}")
        if not _atomic_write_text(dest_path, rendered, mode, dry_run):
            all_ok = False

    return all_ok


def main():
    parser = argparse.ArgumentParser(description="Terminal 配置同步引擎")
    parser.add_argument(
        "--config",
        default=str(REPO_ROOT / "config" / "terminal.json"),
        help="terminal.json 路径",
    )
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不实际写入")
    parser.add_argument("--offline", action="store_true", help="离线模式（当前引擎无网络操作，保留以统一接口）")
    args = parser.parse_args()

    config_path = Path(args.config)
    if not config_path.exists():
        logging.error(f"配置文件不存在: {config_path}")
        sys.exit(1)

    ok = sync_terminal(config_path, REPO_ROOT, dry_run=args.dry_run, offline=args.offline)
    if not ok:
        sys.exit(1)
    logging.info("Terminal 配置同步完成")


if __name__ == "__main__":
    main()
