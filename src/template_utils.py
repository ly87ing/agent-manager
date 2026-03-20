from __future__ import annotations

import os
import re
from pathlib import Path


def load_env_file(path: str | Path) -> dict[str, str]:
    """解析 KEY=VALUE 格式的 env 文件，返回变量字典。

    忽略空行和 # 注释行。missing_ok 由调用方控制（文件不存在时返回空字典）。
    """
    env: dict[str, str] = {}
    env_path = Path(path)
    if not env_path.exists():
        return env
    for line in env_path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip()
        # 去除可选引号
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
            value = value[1:-1]
        if key:
            env[key] = value
    return env


def render_template(tmpl_str: str, env: dict[str, str]) -> str:
    """将模板字符串中的 ${VAR} 占位符替换为 env 中对应值。

    若存在未被替换的 ${VAR} 占位符则抛出 ValueError。
    """
    def replacer(match: re.Match) -> str:
        var_name = match.group(1)
        if var_name not in env:
            raise ValueError(f"模板变量未定义: ${{{var_name}}}")
        return env[var_name]

    result = re.sub(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}", replacer, tmpl_str)
    return result
