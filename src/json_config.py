from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path


def load_json_file(path, *, strict=False, missing_ok=True):
    if not os.path.exists(path):
        if strict and not missing_ok:
            raise ValueError(f"{os.path.basename(path)} 缺失")
        return {}

    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception as exc:
        if strict:
            raise ValueError(f"{os.path.basename(path)} 无效: {exc}") from exc
        return {}


def load_json_object_file(path, *, strict=False, missing_ok=True):
    data = load_json_file(path, strict=strict, missing_ok=missing_ok)
    if isinstance(data, dict):
        return data
    if strict:
        raise ValueError(f"{os.path.basename(path)} 顶层必须是对象")
    return {}


def get_object_section(data, key, source_name):
    value = data.get(key, {})
    if value is None:
        return {}
    if isinstance(value, dict):
        return value
    raise ValueError(f"{source_name}.{key} 必须是对象")


def ensure_mapping_value_types(mapping, source_name, value_types, type_label):
    if not isinstance(mapping, dict):
        raise ValueError(f"{source_name} 必须是对象")
    for key, value in mapping.items():
        if not isinstance(value, value_types):
            raise ValueError(f"{source_name}.{key} 必须是{type_label}")
    return mapping


def get_typed_mapping_section(data, key, source_name, value_types, type_label):
    section = get_object_section(data, key, source_name)
    return ensure_mapping_value_types(section, f"{source_name}.{key}", value_types, type_label)


def atomic_write_json_file(path, data, *, chmod=None):
    target_path = Path(path)
    target_path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_path = tempfile.mkstemp(dir=str(target_path.parent), prefix=f"{target_path.name}.tmp_")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(data, handle, indent=2, ensure_ascii=False)
            handle.write("\n")
        os.replace(temp_path, target_path)
        if chmod is not None:
            os.chmod(target_path, chmod)
    except Exception:
        if os.path.exists(temp_path):
            os.remove(temp_path)
        raise
