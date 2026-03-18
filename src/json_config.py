from __future__ import annotations

import json
import os


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
