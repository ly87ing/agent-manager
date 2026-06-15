from __future__ import annotations

import os
import re
from pathlib import Path


def load_env_file(path: str | Path) -> dict[str, str]:
    """Parse an env file in KEY=VALUE format and return a dict of variables.

    Blank lines and # comment lines are ignored. missing_ok is controlled by the caller (returns an empty dict when the file does not exist).
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
        # Strip optional surrounding quotes
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
            value = value[1:-1]
        if key:
            env[key] = value
    return env


def render_template(tmpl_str: str, env: dict[str, str]) -> str:
    """Replace ${VAR} placeholders in the template string with the corresponding values from env.

    Only variables present in env are substituted; ${VAR} placeholders not declared in env are left as-is (pass-through),
    to support templates such as zshrc that contain shell's own variable syntax.
    A variable declared in env with an empty-string value is substituted normally (empty values are allowed).
    """
    def replacer(match: re.Match) -> str:
        var_name = match.group(1)
        if var_name not in env:
            return match.group(0)  # keep as-is
        return env[var_name]

    result = re.sub(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}", replacer, tmpl_str)
    return result
