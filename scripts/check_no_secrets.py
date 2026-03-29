#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path


def main() -> int:
    root_dir = Path(__file__).resolve().parent.parent
    if str(root_dir) not in sys.path:
        sys.path.insert(0, str(root_dir))

    from src.secret_scan import scan_repo_for_secrets

    findings = scan_repo_for_secrets(root_dir)
    if not findings:
        print("OK: no suspected plaintext secrets found outside the secrets repository.")
        return 0

    print("ERROR: suspected plaintext secrets found in agent-manager repository files:")
    for finding in findings:
        print(f" - {finding.path}:{finding.line} [{finding.kind}]")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
