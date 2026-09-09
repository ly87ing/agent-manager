#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUNTIME_DIR="$(mktemp -d)"

cleanup() {
    rm -rf "$RUNTIME_DIR"
}
trap cleanup EXIT

export AGENT_MANAGER_DATA_DIR="$RUNTIME_DIR/data"

run() {
    printf '==> %s\n' "$*"
    "$@"
}

cd "$ROOT_DIR"

run cargo run --manifest-path engine-rs/Cargo.toml -- db:status --json
run cargo run --manifest-path engine-rs/Cargo.toml -- policy:get --json
run cargo run --manifest-path engine-rs/Cargo.toml -- policy:set --layer local --key agents.codex.enabled --value false --dry-run --json
