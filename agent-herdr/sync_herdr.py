#!/usr/bin/env python3
from __future__ import annotations

import argparse
import logging
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.command_runner import run_command
from src.json_config import get_object_section, get_typed_mapping_section, load_json_object_file
from src.runtime_logging import configure_logging

configure_logging()


class HerdrSyncError(ValueError):
    pass


def _require_bool(mapping: dict, key: str, source: str, default: bool) -> bool:
    value = mapping.get(key, default)
    if not isinstance(value, bool):
        raise HerdrSyncError(f"{source}.{key} 必须是布尔值")
    return value


def find_herdr_binary(binary_def: dict) -> str | None:
    command = binary_def.get("command")
    if not isinstance(command, str) or not command.strip():
        raise HerdrSyncError("herdr.json.binary.command 必须是非空字符串")

    detected = shutil.which(command)
    if detected:
        return detected

    fallback_paths = binary_def.get("fallback_paths", [])
    if not isinstance(fallback_paths, list) or not all(isinstance(path, str) for path in fallback_paths):
        raise HerdrSyncError("herdr.json.binary.fallback_paths 必须是字符串数组")
    for raw_path in fallback_paths:
        candidate = Path(raw_path).expanduser()
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def install_binary(binary_def: dict) -> None:
    install_def = get_object_section(binary_def, "install", "herdr.json.binary")
    platform_key = "windows" if os.name == "nt" else "posix"
    command = install_def.get(platform_key)
    if not isinstance(command, str) or not command.strip():
        raise HerdrSyncError(f"herdr.json.binary.install.{platform_key} 必须是非空字符串")
    try:
        run_command(command, check=True, capture_output=True, text=True, timeout=300)
    except subprocess.TimeoutExpired as exc:
        raise HerdrSyncError("Herdr 官方安装器执行超时") from exc
    except subprocess.CalledProcessError as exc:
        detail = (exc.stderr or exc.stdout or "").strip()
        raise HerdrSyncError(f"Herdr 官方安装器失败: {detail or f'exit {exc.returncode}'}") from exc
    except OSError as exc:
        raise HerdrSyncError(f"无法执行 Herdr 官方安装器: {exc}") from exc


def run_herdr_command(binary: str, *args: str) -> str:
    try:
        result = subprocess.run(
            [binary, *args],
            check=True,
            capture_output=True,
            text=True,
            timeout=60,
        )
    except subprocess.TimeoutExpired as exc:
        raise HerdrSyncError(f"Herdr 命令超时: {' '.join(args)}") from exc
    except subprocess.CalledProcessError as exc:
        detail = (exc.stderr or exc.stdout or "").strip()
        raise HerdrSyncError(f"Herdr 命令失败 ({' '.join(args)}): {detail or f'exit {exc.returncode}'}") from exc
    except OSError as exc:
        raise HerdrSyncError(f"无法执行 Herdr: {exc}") from exc
    return result.stdout.strip()


def parse_integration_status(output: str) -> dict[str, str]:
    statuses: dict[str, str] = {}
    for raw_line in output.splitlines():
        line = raw_line.strip()
        if not line or ":" not in line:
            continue
        target, status = line.split(":", 1)
        target = target.strip()
        status = status.strip()
        if target and status:
            statuses[target] = status
    return statuses


def integration_is_current(status: str | None) -> bool:
    return bool(status and status.startswith("current"))


def command_exists(command: str) -> bool:
    return bool(shutil.which(command))


def sync_integrations(
    binary: str,
    integration_defs: dict,
    integration_policy: dict,
    *,
    dry_run: bool,
) -> None:
    statuses = parse_integration_status(run_herdr_command(binary, "integration", "status"))
    changed_targets: list[str] = []

    for agent_id, definition in integration_defs.items():
        policy = integration_policy.get(agent_id, {})
        if not isinstance(policy, dict):
            raise HerdrSyncError(f"manager.json.herdr.integrations.{agent_id} 必须是对象")
        if not _require_bool(policy, "enabled", f"manager.json.herdr.integrations.{agent_id}", True):
            continue

        target = definition.get("target")
        agent_command = definition.get("agent_command")
        if not isinstance(target, str) or not target:
            raise HerdrSyncError(f"herdr.json.integrations.{agent_id}.target 必须是非空字符串")
        if not isinstance(agent_command, str) or not agent_command:
            raise HerdrSyncError(f"herdr.json.integrations.{agent_id}.agent_command 必须是非空字符串")

        status = statuses.get(target)
        if integration_is_current(status):
            logging.info(f"[Herdr:{agent_id}] 集成已是当前版本: {status}")
            continue
        if not command_exists(agent_command):
            logging.warning(f"[Herdr:{agent_id}] 未发现 {agent_command}，保留策略并跳过本次集成安装。")
            continue
        if dry_run:
            logging.info(f"[Herdr:{agent_id}] 计划执行: herdr integration install {target}")
            continue

        run_herdr_command(binary, "integration", "install", target)
        changed_targets.append(target)
        logging.info(f"[Herdr:{agent_id}] 集成安装完成。")

    if not changed_targets:
        return

    statuses = parse_integration_status(run_herdr_command(binary, "integration", "status"))
    invalid = [target for target in changed_targets if not integration_is_current(statuses.get(target))]
    if invalid:
        raise HerdrSyncError(f"Herdr 集成安装后未通过回读验证: {', '.join(invalid)}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="安装 Herdr 并收敛 Claude Code / Codex 官方集成")
    parser.add_argument("--config", required=True, help="有效 manager 策略文件")
    parser.add_argument("--def-file", required=True, help="Herdr 定义文件")
    parser.add_argument("--dry-run", action="store_true", help="仅显示计划动作")
    parser.add_argument("--offline", action="store_true", help="禁止缺失时联网安装 Herdr")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        manager = load_json_object_file(args.config, strict=True, missing_ok=False)
        definitions = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        policy = get_object_section(manager, "herdr", Path(args.config).name)
        if not _require_bool(policy, "enabled", f"{Path(args.config).name}.herdr", False):
            logging.info("当前策略未启用 Herdr。")
            return 0

        auto_install = _require_bool(policy, "auto_install", f"{Path(args.config).name}.herdr", True)
        integration_policy = get_object_section(policy, "integrations", f"{Path(args.config).name}.herdr")
        binary_def = get_object_section(definitions, "binary", Path(args.def_file).name)
        integration_defs = get_typed_mapping_section(
            definitions,
            "integrations",
            Path(args.def_file).name,
            (dict,),
            "对象",
        )

        binary = find_herdr_binary(binary_def)
        if binary is None:
            if args.dry_run:
                logging.info("[Herdr] 计划使用官方安装器安装 CLI；随后收敛已安装 Agent 的集成。")
                return 0
            if args.offline:
                raise HerdrSyncError("离线模式下未发现 Herdr，不能调用官方安装器。")
            if not auto_install:
                raise HerdrSyncError("未发现 Herdr，且 manager 策略禁止自动安装。")
            install_binary(binary_def)
            binary = find_herdr_binary(binary_def)
            if binary is None:
                raise HerdrSyncError("Herdr 安装器执行完成，但未找到可执行文件。")
            logging.info(f"[Herdr] CLI 安装完成: {binary}")

        version = run_herdr_command(binary, "--version")
        logging.info(f"[Herdr] {version or 'CLI 可用'}")
        sync_integrations(
            binary,
            integration_defs,
            integration_policy,
            dry_run=args.dry_run,
        )
        return 0
    except (HerdrSyncError, ValueError) as exc:
        logging.error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
