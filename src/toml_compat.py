from __future__ import annotations

import importlib
from functools import lru_cache


@lru_cache(maxsize=1)
def load_tomllib():
    for module_name in ("tomllib", "tomli"):
        try:
            return importlib.import_module(module_name)
        except ModuleNotFoundError:
            continue
    raise ModuleNotFoundError(
        "TOML parsing requires Python 3.11+ or the 'tomli' package installed for Python 3.10 and earlier."
    )


class _TomlCompat:
    def __getattr__(self, name):
        return getattr(load_tomllib(), name)


tomllib = _TomlCompat()

