#!/usr/bin/env bash

set -euo pipefail

payload="$(cat)"
project_dir="${CLAUDE_PROJECT_DIR:-$(pwd)}"

# Fast-path: skip the expensive rebuild verification when every rules-affecting
# path matches HEAD. Falls through to the heavy check when git is unavailable
# or any input has diverged.
if dirty="$(git -C "$project_dir" status --porcelain -- \
        agent-rule/rules/src \
        agent-rule/rules/dist \
        agent-rule/scripts/build-agent-rules.py \
        config/rules-targets.list 2>/dev/null)"; then
    if [ -z "$dirty" ]; then
        exit 0
    fi
fi

PAYLOAD="$payload" PROJECT_DIR="$project_dir" python3 - <<'PY'
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

payload = os.environ.get("PAYLOAD", "")
project_dir = os.path.realpath(os.environ.get("PROJECT_DIR", os.getcwd()))

try:
    data = json.loads(payload or "{}")
except json.JSONDecodeError:
    data = {}

if data.get("stop_hook_active"):
    sys.exit(0)


def emit_block(reason: str) -> None:
    json.dump(
        {
            "decision": "block",
            "reason": reason,
        },
        sys.stdout,
    )
    sys.stdout.write("\n")
    sys.exit(0)

repo_root = Path(project_dir)
src_root = repo_root / "agent-rule" / "rules" / "src"
dist_root = repo_root / "agent-rule" / "rules" / "dist"
builder = repo_root / "agent-rule" / "scripts" / "build-agent-rules.py"
manifest_path = repo_root / "config" / "rules-targets.list"

if not src_root.exists() or not dist_root.exists() or not builder.exists():
    sys.exit(0)
if not manifest_path.exists():
    emit_block(
        "Managed rules manifest is missing: config/rules-targets.list. "
        "Run make rules-build and restore the manifest before stopping."
    )


def snapshot(root: Path) -> dict[str, str]:
    return {
        str(path.relative_to(root)): path.read_text(encoding="utf-8")
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


try:
    with tempfile.TemporaryDirectory() as temp_dir:
        temp_root = Path(temp_dir) / "repo"
        temp_builder = temp_root / "agent-rule" / "scripts" / "build-agent-rules.py"
        temp_src_root = temp_root / "agent-rule" / "rules" / "src"
        temp_manifest_path = temp_root / "config" / "rules-targets.list"
        temp_builder.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(builder, temp_builder)
        shutil.copytree(src_root, temp_src_root)
        temp_manifest_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(manifest_path, temp_manifest_path)
        build = subprocess.run(
            [sys.executable, str(temp_builder)],
            cwd=temp_root / "agent-rule",
            capture_output=True,
            text=True,
            check=False,
        )
        if build.returncode != 0:
            emit_block(
                "Managed rules could not be rebuilt for verification. "
                "Run make rules-build and fix the build before stopping."
            )
        fresh_dist = snapshot(temp_root / "agent-rule" / "rules" / "dist")
except OSError as exc:
    emit_block(
        "Managed rules verification inputs are unreadable "
        f"({manifest_path.name}: {exc}). Run make rules-build and fix the managed rules files before stopping."
    )

if snapshot(dist_root) != fresh_dist:
    emit_block(
        "Rules dist is out of sync with the current source/build logic. "
        "Run make rules-build before stopping."
    )
PY
