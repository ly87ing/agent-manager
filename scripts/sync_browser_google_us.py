#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path


# The script is loaded two ways that do not put its own directory on
# sys.path: as `python3 scripts/sync_browser_google_us.py` (script dir is
# sys.path[0], fine) and via importlib.spec_from_file_location in tests
# (it is not). Add it explicitly so the browser_us package always imports.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_us.constants import *  # noqa: E402,F401,F403
from browser_us.model import *  # noqa: E402,F401,F403
from browser_us.helpers import *  # noqa: E402,F401,F403
from browser_us.sync_audit import *  # noqa: E402,F401,F403
from browser_us.chrome import *  # noqa: E402,F401,F403
from browser_us.verify_report import *  # noqa: E402,F401,F403
from browser_us.runtime_probe import *  # noqa: E402,F401,F403
from browser_us.automation import *  # noqa: E402,F401,F403
from browser_us.cli import *  # noqa: E402,F401,F403



if __name__ == "__main__":
    raise SystemExit(main())
