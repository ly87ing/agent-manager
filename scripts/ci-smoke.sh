#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUNTIME_DIR="$(mktemp -d)"

cleanup() {
    rm -rf "$RUNTIME_DIR"
}
trap cleanup EXIT

export HOME="$RUNTIME_DIR/home"
mkdir -p "$HOME"
export AGENT_MANAGER_LOCK_FILE="$RUNTIME_DIR/agent-manager.lock"
export AGENT_MANAGER_LOG_PREFIX="$RUNTIME_DIR/agent-manager-sync"
export AGENT_MANAGER_BASE_CONFIG="$ROOT_DIR/config/manager.smoke.json"
export AGENT_MANAGER_LOCAL_CONFIG="$RUNTIME_DIR/manager.local.json"

run() {
    printf '==> %s\n' "$*"
    "$@"
}

cd "$ROOT_DIR"

run make secrets-check
run ./manage.sh help
run ./manage.sh sync --dry-run
run ./manage.sh update --dry-run
run ./manage.sh plan
run ./manage.sh self-update --dry-run
run make -n sync update bootstrap all plan self-update offline dry-run
run make -n install
