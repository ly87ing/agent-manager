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


configure_logging()

OFFICIAL_NPM_REGISTRY = "https://registry.npmjs.org"
VERSION_LOCK_KEY = "version_lock"
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


def write_json_file(path: Path, payload):
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


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


def iter_version_lock_targets(value, path=()):
    if isinstance(value, dict):
        lock = value.get(VERSION_LOCK_KEY)
        if lock is not None:
            yield path, value, lock
        for key, child in value.items():
            if key == VERSION_LOCK_KEY:
                continue
            yield from iter_version_lock_targets(child, path + (str(key),))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from iter_version_lock_targets(child, path + (str(index),))


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


def parse_relative_path(path: str):
    if not isinstance(path, str) or not path:
        raise VersionPinError("version_lock.fields 的路径必须是非空字符串")
    return path.split(".")


def assign_path(root, path, value):
    current = root
    parts = parse_relative_path(path)
    for part in parts[:-1]:
        if isinstance(current, list):
            try:
                current = current[int(part)]
            except (ValueError, IndexError) as exc:
                raise VersionPinError(f"路径不存在: {path}") from exc
            continue
        if not isinstance(current, dict) or part not in current:
            raise VersionPinError(f"路径不存在: {path}")
        current = current[part]

    leaf = parts[-1]
    if isinstance(current, list):
        try:
            index = int(leaf)
            changed = current[index] != value
            current[index] = value
            return changed
        except (ValueError, IndexError) as exc:
            raise VersionPinError(f"路径不存在: {path}") from exc
    elif isinstance(current, dict) and leaf in current:
        changed = current[leaf] != value
        current[leaf] = value
        return changed
    else:
        raise VersionPinError(f"路径不存在: {path}")


def update_document_versions(document, file_label: str, dry_run: bool):
    changes = []
    for object_path, target, lock in iter_version_lock_targets(document):
        fields = lock.get("fields") if isinstance(lock, dict) else None
        if not isinstance(fields, dict) or not fields:
            raise VersionPinError(f"{file_label}:{'.'.join(object_path)} version_lock.fields 必须是非空对象")

        if dry_run:
            changes.append((object_path, "<dry-run>", False))
            continue

        version = resolve_version(lock)
        changed = False
        for relative_path, template in fields.items():
            if not isinstance(template, str) or "{version}" not in template:
                raise VersionPinError(
                    f"{file_label}:{'.'.join(object_path)} field {relative_path} 模板必须包含 {{version}}"
                )
            changed = assign_path(target, relative_path, template.format(version=version)) or changed
        changes.append((object_path, version, changed))
    return changes


def update_config_file(path: Path, dry_run: bool):
    document = load_json_file(path)
    changes = update_document_versions(document, path.name, dry_run)
    if not changes:
        return []
    if not dry_run and any(changed for _, _, changed in changes):
        write_json_file(path, document)
    return changes


def update_config_dir(config_dir: Path, dry_run: bool = False):
    all_changes = []
    for filename in MANAGED_CONFIG_FILES:
        path = config_dir / filename
        changes = update_config_file(path, dry_run)
        all_changes.extend((filename, object_path, version, changed) for object_path, version, changed in changes)
    return all_changes


def main_with_args_for_test(config_dir: Path, dry_run: bool = False, offline: bool = False):
    if offline:
        logging.info("离线模式，跳过版本锁刷新。")
        return 0
    try:
        all_changes = update_config_dir(Path(config_dir), dry_run=dry_run)
    except VersionPinError as exc:
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
    return 0


def main():
    parser = argparse.ArgumentParser(description="更新受管 MCP / CLI 的固定版本锁")
    parser.add_argument("--config-dir", default=str(REPO_ROOT / "config"), help="配置目录")
    parser.add_argument("--dry-run", action="store_true", help="只预览，不联网查询也不写入")
    parser.add_argument("--offline", action="store_true", help="离线模式下跳过版本锁刷新")
    args = parser.parse_args()

    return main_with_args_for_test(Path(args.config_dir), dry_run=args.dry_run, offline=args.offline)


if __name__ == "__main__":
    sys.exit(main())
