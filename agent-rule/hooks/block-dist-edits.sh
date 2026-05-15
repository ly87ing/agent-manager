#!/usr/bin/env bash

set -euo pipefail

payload="$(cat)"
project_dir="${CLAUDE_PROJECT_DIR:-$(pwd)}"

# Fast-path: 99% of tool calls don't target the dist tree at all. Skip the
# Python interpreter entirely when the payload doesn't even mention the
# guarded path.
case "$payload" in
    *agent-rule/rules/dist*) ;;
    *) exit 0 ;;
esac

PAYLOAD="$payload" PROJECT_DIR="$project_dir" python3 - <<'PY'
import json
import os
import re
import sys

payload = os.environ.get("PAYLOAD", "")
project_dir = os.path.realpath(os.environ.get("PROJECT_DIR", os.getcwd()))

try:
    data = json.loads(payload or "{}")
except json.JSONDecodeError:
    sys.exit(0)

tool_input = data.get("tool_input") or {}
blocked_root = os.path.realpath(os.path.join(project_dir, "agent-rule", "rules", "dist"))


def file_targets_dist(file_path: str | None) -> bool:
    if not file_path:
        return False
    candidate = file_path if os.path.isabs(file_path) else os.path.join(project_dir, file_path)
    candidate = os.path.realpath(candidate)
    return candidate == blocked_root or candidate.startswith(blocked_root + os.sep)


def command_writes_dist(command: str | None) -> bool:
    # Conservative heuristic: deny only when the command both mentions the dist
    # tree AND contains a write/destructive shell operator. Legitimate workflows
    # like `make rules-build` invoke the build script without naming the dist
    # path in the bash command itself, so they fall through unblocked.
    if not command or "agent-rule/rules/dist" not in command:
        return False
    if re.search(r"^\*\*\* (Add|Update|Delete) File: .*/?agent-rule/rules/dist/", command, re.MULTILINE):
        return True
    return bool(re.search(
        r"(\s>{1,2}\s|\btee\b|\bcp\b|\bmv\b|\brm\b|\bdd\b|\bsed\s+-i)",
        command,
    ))


if file_targets_dist(tool_input.get("file_path")) or command_writes_dist(tool_input.get("command")):
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
