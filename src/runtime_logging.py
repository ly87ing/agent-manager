from __future__ import annotations

import logging
import os


DEFAULT_LOG_FORMAT = "%(asctime)s [%(levelname)s] %(message)s"
DEFAULT_LOG_DATEFMT = "%Y-%m-%d %H:%M:%S"


def resolve_log_level(default="INFO"):
    raw_level = str(os.getenv("LOG_LEVEL", default) or default).strip().upper()
    return getattr(logging, raw_level, getattr(logging, str(default).strip().upper(), logging.INFO))


def configure_logging(*, default="INFO", force=False):
    level = resolve_log_level(default)
    logging.basicConfig(
        level=level,
        format=DEFAULT_LOG_FORMAT,
        datefmt=DEFAULT_LOG_DATEFMT,
        force=force,
    )
    logging.debug("Logging initialized at level %s", logging.getLevelName(level))
    return level
