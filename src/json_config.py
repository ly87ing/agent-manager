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
