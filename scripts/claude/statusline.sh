#!/usr/bin/env bash
# Minimal Claude Code status line. Reads session JSON on stdin.
# Output: "<model> | <basename(cwd)> (<git branch>)"

set -u

input=$(cat)
model=$(printf '%s' "$input" | jq -r '.model.display_name // .model.id // "claude"')
cwd=$(printf '%s' "$input" | jq -r '.workspace.current_dir // .cwd // "."')
dir=$(basename "$cwd")

branch=""
if [ -d "$cwd/.git" ] || git -C "$cwd" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    branch=$(git -C "$cwd" branch --show-current 2>/dev/null)
fi

if [ -n "$branch" ]; then
    printf '%s | %s (%s)' "$model" "$dir" "$branch"
else
    printf '%s | %s' "$model" "$dir"
fi
