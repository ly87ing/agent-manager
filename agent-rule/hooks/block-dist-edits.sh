#!/usr/bin/env bash

set -euo pipefail

payload="$(cat)"
project_dir="${CLAUDE_PROJECT_DIR:-$(pwd)}"

PAYLOAD="$payload" PROJECT_DIR="$project_dir" python3 - <<'PY'
import json
import os
import sys

payload = os.environ.get("PAYLOAD", "")
project_dir = os.path.realpath(os.environ.get("PROJECT_DIR", os.getcwd()))

try:
    data = json.loads(payload or "{}")
except json.JSONDecodeError:
    sys.exit(0)

tool_input = data.get("tool_input") or {}
file_path = tool_input.get("file_path")
if not file_path:
    sys.exit(0)

candidate = file_path if os.path.isabs(file_path) else os.path.join(project_dir, file_path)
candidate = os.path.realpath(candidate)
blocked_root = os.path.realpath(os.path.join(project_dir, "agent-rule", "rules", "dist"))

if candidate == blocked_root or candidate.startswith(blocked_root + os.sep):
    json.dump(
        {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": (
                    "agent-rule/rules/dist/ is generated output. "
                    "Edit agent-rule/rules/src/ and run make rules-build instead."
                ),
            }
        },
        sys.stdout,
    )
    sys.stdout.write("\n")
PY
