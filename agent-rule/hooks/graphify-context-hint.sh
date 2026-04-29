#!/usr/bin/env bash

set -euo pipefail

payload="$(cat)"
project_dir="${CLAUDE_PROJECT_DIR:-$(pwd)}"

PAYLOAD="$payload" PROJECT_DIR="$project_dir" python3 - <<'PY'
import json
import os
import re
import subprocess
import sys
from pathlib import Path

payload = os.environ.get("PAYLOAD", "")
project_dir = Path(os.environ.get("PROJECT_DIR", os.getcwd())).resolve()

try:
    data = json.loads(payload or "{}")
except json.JSONDecodeError:
    sys.exit(0)

tool_input = data.get("tool_input") if isinstance(data, dict) else {}
if not isinstance(tool_input, dict):
    tool_input = data if isinstance(data, dict) else {}
command = tool_input.get("command", "")
if not isinstance(command, str):
    sys.exit(0)

if not re.search(r"(^|[\s;&|()])(grep|rg|ripgrep|find|fd|ack|ag)(\s|$)", command):
    sys.exit(0)

try:
    git_root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=project_dir,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    repo_root = Path(git_root).resolve()
except (OSError, subprocess.CalledProcessError):
    repo_root = project_dir

graph_path = repo_root / "graphify-out" / "graph.json"
if not graph_path.is_file():
    sys.exit(0)


def graph_appears_stale() -> bool | None:
    try:
        graph_mtime = graph_path.stat().st_mtime
    except OSError:
        return None
    try:
        tracked = subprocess.run(
            ["git", "ls-files", "-z", "--cached", "--modified", "--others", "--exclude-standard"],
            cwd=repo_root,
            capture_output=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    for raw_path in tracked.split(b"\0"):
        if not raw_path:
            continue
        try:
            relative_path = raw_path.decode()
        except UnicodeDecodeError:
            continue
        if relative_path.startswith("graphify-out/"):
            continue
        candidate = repo_root / relative_path
        try:
            if candidate.is_file() and candidate.stat().st_mtime > graph_mtime:
                return True
        except OSError:
            continue
    return False


stale = graph_appears_stale()
if stale is True:
    message = (
        "graphify: Knowledge graph exists but appears stale. "
        "Treat graphify-out/GRAPH_REPORT.md as orientation only; "
        "verify facts against current files with codebase-retrieval, rg, or direct reads. "
        "Run graphify update only when keeping the graph current is part of the task."
    )
elif stale is False:
    message = (
        "graphify: Knowledge graph exists and no newer tracked/unignored files were detected. "
        "Read graphify-out/GRAPH_REPORT.md before searching raw files; still verify precise claims "
        "against current files."
    )
else:
    message = (
        "graphify: Knowledge graph exists. Check whether graphify-out/graph.json is fresh before relying on it; "
        "if stale, use it only as orientation and verify current files."
    )

json.dump(
    {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": message,
        }
    },
    sys.stdout,
)
sys.stdout.write("\n")
PY
