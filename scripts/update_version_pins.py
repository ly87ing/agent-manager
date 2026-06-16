#!/usr/bin/env python3
import argparse
import json
import logging
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.command_runner import run_command
from src.runtime_logging import configure_logging
from src.version_pins import (
    VersionPinsError,
    apply_version_lock_fields,
    default_version_pins_path,
    get_version_pin,
    iter_version_lock_targets,
    load_version_pins,
    set_version_pin,
    version_pin_key,
    write_version_pins,
)


configure_logging()

OFFICIAL_NPM_REGISTRY = "https://registry.npmjs.org"
MANAGED_CONFIG_FILES = ("agents.json", "tools.json", "mcp.json")


class VersionPinError(Exception):
    pass


def load_json_file(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise VersionPinError(f"{path} 不存在") from exc
    except json.JSONDecodeError as exc:
        raise VersionPinError(f"{path} 不是合法 JSON: {exc}") from exc


def run_capture(cmd):
    try:
        result = run_command(cmd, check=True, capture_output=True, text=True, timeout=60)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, FileNotFoundError) as exc:
        raise VersionPinError(f"命令失败: {' '.join(cmd)}: {exc}") from exc
    return result.stdout.strip()


def fetch_npm_latest_version(package_name: str) -> str:
    output = run_capture(["npm", "view", package_name, "version", "--registry", OFFICIAL_NPM_REGISTRY])
    if not output:
        raise VersionPinError(f"npm 包 {package_name} 未返回版本")
    return output


def fetch_git_ref_commit(remote: str, ref: str) -> str:
    refspec = ref if ref.startswith("refs/") else f"refs/heads/{ref}"
    output = run_capture(["git", "ls-remote", remote, refspec])
    if not output:
        raise VersionPinError(f"Git ref 未找到: {remote} {refspec}")
    commit = output.split()[0]
    if len(commit) != 40 or not all(ch in "0123456789abcdefABCDEF" for ch in commit):
        raise VersionPinError(f"Git ref 返回了非法 commit: {output}")
    return commit.lower()


def resolve_version(lock):
    if not isinstance(lock, dict):
        raise VersionPinError("version_lock 必须是对象")

    lock_type = lock.get("type")
    if lock_type == "npm":
        package_name = lock.get("package")
        if not isinstance(package_name, str) or not package_name:
            raise VersionPinError("npm version_lock 缺少 package")
        return fetch_npm_latest_version(package_name)

    if lock_type == "git-ref":
        remote = lock.get("remote")
        ref = lock.get("ref")
        if not isinstance(remote, str) or not remote:
            raise VersionPinError("git-ref version_lock 缺少 remote")
        if not isinstance(ref, str) or not ref:
            raise VersionPinError("git-ref version_lock 缺少 ref")
        return fetch_git_ref_commit(remote, ref)

    raise VersionPinError(f"不支持的 version_lock.type: {lock_type!r}")


def update_document_versions(document, file_label: str, pins: dict, dry_run: bool):
    changes = []
    for object_path, target, lock in iter_version_lock_targets(document):
        if dry_run:
            changes.append((object_path, "<dry-run>", False))
            continue

        version = resolve_version(lock)
        current_version = get_version_pin(pins, file_label, object_path)
        changed = current_version != version
        apply_version_lock_fields(target, lock, version, f"{file_label}:{version_pin_key(object_path)}")
        set_version_pin(pins, file_label, object_path, version)
        changes.append((object_path, version, changed))
    return changes


def update_config_file(path: Path, pins: dict, dry_run: bool):
    document = load_json_file(path)
    return update_document_versions(document, path.name, pins, dry_run)


def update_config_dir(config_dir: Path, pins_path: Path | None = None, dry_run: bool = False):
    pins_path = pins_path or default_version_pins_path(config_dir)
    pins = load_version_pins(pins_path)
    all_changes = []
    for filename in MANAGED_CONFIG_FILES:
        path = config_dir / filename
        changes = update_config_file(path, pins, dry_run)
        all_changes.extend((filename, object_path, version, changed) for object_path, version, changed in changes)
    if not dry_run and any(changed for _, _, _, changed in all_changes):
        write_version_pins(pins_path, pins)
    return all_changes


def main_with_args_for_test(config_dir: Path, dry_run: bool = False, offline: bool = False, pins_path: Path | None = None):
    if offline:
        logging.info("离线模式，跳过版本锁刷新。")
        return 0
    try:
        all_changes = update_config_dir(Path(config_dir), pins_path=pins_path, dry_run=dry_run)
    except (VersionPinError, VersionPinsError) as exc:
        logging.error(str(exc))
        return 1

    if not all_changes:
        logging.info("没有发现 version_lock 配置。")
        return 0

    for filename, object_path, version, changed in all_changes:
        label = ".".join(object_path)
        if dry_run:
            logging.info(f"[{filename}] 预览：计划刷新版本锁 {label}")
        elif changed:
            logging.info(f"[{filename}] 已刷新版本锁 {label} -> {version}")
        else:
            logging.info(f"[{filename}] 版本锁已是最新 {label} -> {version}")
    if not dry_run:
        local_pins_path = pins_path or default_version_pins_path(Path(config_dir))
        logging.info(f"本地版本锁已写入 {local_pins_path}")
    return 0


def main():
    parser = argparse.ArgumentParser(description="更新受管 MCP / CLI 的固定版本锁")
    parser.add_argument("--config-dir", default=str(REPO_ROOT / "config"), help="配置目录")
    parser.add_argument("--pins-file", help="本地版本锁文件，默认 config/version-pins.local.json")
    parser.add_argument("--dry-run", action="store_true", help="只预览，不联网查询也不写入")
    parser.add_argument("--offline", action="store_true", help="离线模式下跳过版本锁刷新")
    args = parser.parse_args()

    pins_path = Path(args.pins_file) if args.pins_file else None
    return main_with_args_for_test(Path(args.config_dir), dry_run=args.dry_run, offline=args.offline, pins_path=pins_path)


if __name__ == "__main__":
    sys.exit(main())
