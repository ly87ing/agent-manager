from __future__ import annotations

import json
from pathlib import Path

from src.json_config import atomic_write_json_file


LOCAL_VERSION_PINS_FILENAME = "version-pins.local.json"
SCHEMA_VERSION = 1
VERSION_LOCK_KEY = "version_lock"
VERSION_LOCKS_SECTION = "version_locks"
SKILL_REPOSITORIES_SECTION = "skill_repositories"


class VersionPinsError(ValueError):
    pass


def default_version_pins_path(config_dir: Path) -> Path:
    return Path(config_dir) / LOCAL_VERSION_PINS_FILENAME


def version_pin_key(object_path) -> str:
    parts = [str(part) for part in object_path]
    if not parts or any(not part for part in parts):
        raise VersionPinsError("version_lock 路径必须是非空路径")
    return ".".join(parts)


def empty_version_pins() -> dict:
    return {
        "schema_version": SCHEMA_VERSION,
        VERSION_LOCKS_SECTION: {},
        SKILL_REPOSITORIES_SECTION: {},
    }


def _ensure_mapping(value, label: str) -> dict:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise VersionPinsError(f"{label} 必须是对象")
    return value


def load_version_pins(path: Path) -> dict:
    path = Path(path)
    if not path.exists():
        return empty_version_pins()
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise VersionPinsError(f"{path.name} 不是合法 JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise VersionPinsError(f"{path.name} 顶层必须是对象")

    schema_version = payload.get("schema_version", SCHEMA_VERSION)
    if schema_version != SCHEMA_VERSION:
        raise VersionPinsError(f"{path.name}.schema_version 不支持: {schema_version!r}")

    version_locks = _ensure_mapping(payload.get(VERSION_LOCKS_SECTION), f"{path.name}.{VERSION_LOCKS_SECTION}")
    skill_repositories = _ensure_mapping(
        payload.get(SKILL_REPOSITORIES_SECTION),
        f"{path.name}.{SKILL_REPOSITORIES_SECTION}",
    )
    normalized = empty_version_pins()
    normalized[VERSION_LOCKS_SECTION] = version_locks
    normalized[SKILL_REPOSITORIES_SECTION] = skill_repositories
    return normalized


def write_version_pins(path: Path, payload: dict) -> None:
    normalized = empty_version_pins()
    normalized[VERSION_LOCKS_SECTION] = _ensure_mapping(
        payload.get(VERSION_LOCKS_SECTION),
        f"{LOCAL_VERSION_PINS_FILENAME}.{VERSION_LOCKS_SECTION}",
    )
    normalized[SKILL_REPOSITORIES_SECTION] = _ensure_mapping(
        payload.get(SKILL_REPOSITORIES_SECTION),
        f"{LOCAL_VERSION_PINS_FILENAME}.{SKILL_REPOSITORIES_SECTION}",
    )
    atomic_write_json_file(path, normalized)


def get_version_pin(payload: dict, file_label: str, object_path) -> str | None:
    file_locks = _ensure_mapping(
        _ensure_mapping(payload.get(VERSION_LOCKS_SECTION), VERSION_LOCKS_SECTION).get(file_label),
        f"{VERSION_LOCKS_SECTION}.{file_label}",
    )
    value = file_locks.get(version_pin_key(object_path))
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise VersionPinsError(f"{VERSION_LOCKS_SECTION}.{file_label}.{version_pin_key(object_path)} 必须是非空字符串")
    return value


def set_version_pin(payload: dict, file_label: str, object_path, version: str) -> bool:
    if not isinstance(version, str) or not version:
        raise VersionPinsError("版本值必须是非空字符串")
    version_locks = payload.setdefault(VERSION_LOCKS_SECTION, {})
    file_locks = version_locks.setdefault(file_label, {})
    key = version_pin_key(object_path)
    changed = file_locks.get(key) != version
    file_locks[key] = version
    return changed


def validate_git_commit(value: str, label: str) -> str:
    if not isinstance(value, str) or len(value) != 40:
        raise VersionPinsError(f"{label} 必须是 40 位 git commit")
    if not all(ch in "0123456789abcdefABCDEF" for ch in value):
        raise VersionPinsError(f"{label} 必须是 40 位 git commit")
    return value.lower()


def get_skill_repository_pin(payload: dict, repo_name: str) -> str | None:
    repositories = _ensure_mapping(payload.get(SKILL_REPOSITORIES_SECTION), SKILL_REPOSITORIES_SECTION)
    value = repositories.get(repo_name)
    if value is None:
        return None
    return validate_git_commit(value, f"{SKILL_REPOSITORIES_SECTION}.{repo_name}")


def set_skill_repository_pin(payload: dict, repo_name: str, commit: str) -> bool:
    commit = validate_git_commit(commit, f"{SKILL_REPOSITORIES_SECTION}.{repo_name}")
    repositories = payload.setdefault(SKILL_REPOSITORIES_SECTION, {})
    changed = repositories.get(repo_name) != commit
    repositories[repo_name] = commit
    return changed


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


def parse_relative_path(path: str):
    if not isinstance(path, str) or not path:
        raise VersionPinsError("version_lock.fields 的路径必须是非空字符串")
    return path.split(".")


def assign_path(root, path, value) -> bool:
    current = root
    parts = parse_relative_path(path)
    for part in parts[:-1]:
        if isinstance(current, list):
            try:
                current = current[int(part)]
            except (ValueError, IndexError) as exc:
                raise VersionPinsError(f"路径不存在: {path}") from exc
            continue
        if not isinstance(current, dict) or part not in current:
            raise VersionPinsError(f"路径不存在: {path}")
        current = current[part]

    leaf = parts[-1]
    if isinstance(current, list):
        try:
            index = int(leaf)
            changed = current[index] != value
            current[index] = value
            return changed
        except (ValueError, IndexError) as exc:
            raise VersionPinsError(f"路径不存在: {path}") from exc
    if isinstance(current, dict) and leaf in current:
        changed = current[leaf] != value
        current[leaf] = value
        return changed
    raise VersionPinsError(f"路径不存在: {path}")


def apply_version_lock_fields(target: dict, lock: dict, version: str, context_label: str) -> bool:
    fields = lock.get("fields") if isinstance(lock, dict) else None
    if not isinstance(fields, dict) or not fields:
        raise VersionPinsError(f"{context_label} version_lock.fields 必须是非空对象")

    changed = False
    for relative_path, template in fields.items():
        if not isinstance(template, str) or "{version}" not in template:
            raise VersionPinsError(f"{context_label} field {relative_path} 模板必须包含 {{version}}")
        changed = assign_path(target, relative_path, template.format(version=version)) or changed
    return changed


def apply_version_pins_to_document(document: dict, file_label: str, pins: dict) -> list[tuple[tuple[str, ...], str, bool]]:
    changes = []
    for object_path, target, lock in iter_version_lock_targets(document):
        version = get_version_pin(pins, file_label, object_path)
        if version is None:
            continue
        label = f"{file_label}:{version_pin_key(object_path)}"
        changed = apply_version_lock_fields(target, lock, version, label)
        changes.append((object_path, version, changed))
    return changes


def apply_skill_repository_pins_to_document(document: dict, pins: dict) -> list[tuple[str, str, bool]]:
    repositories = document.get("repositories", {})
    if not isinstance(repositories, dict):
        raise VersionPinsError("skills.json.repositories 必须是对象")

    changes = []
    for repo_name, repo_config in repositories.items():
        if not isinstance(repo_config, dict):
            continue
        commit = get_skill_repository_pin(pins, str(repo_name))
        if commit is None:
            continue
        changed = repo_config.get("commit") != commit or "ref" in repo_config
        repo_config["commit"] = commit
        repo_config.pop("ref", None)
        changes.append((str(repo_name), commit, changed))
    return changes


def apply_local_version_pins(document: dict, def_path: Path) -> dict:
    def_path = Path(def_path)
    pins = load_version_pins(default_version_pins_path(def_path.parent))
    apply_version_pins_to_document(document, def_path.name, pins)
    if def_path.name == "skills.json":
        apply_skill_repository_pins_to_document(document, pins)
    return document
