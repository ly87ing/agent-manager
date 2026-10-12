#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUNTIME_DIR="$(mktemp -d)"

cleanup() {
    rm -rf "$RUNTIME_DIR"
}
trap cleanup EXIT

cd "$ROOT_DIR"

run() {
    printf '==> %s\n' "$*"
    "$@"
}

# 先按常规工具链构建（cargo 缓存仍在真实 HOME）；构建完成后，被测二进制只在隔离环境中运行。
run cargo build --manifest-path engine-rs/Cargo.toml
ENGINE_BIN="$ROOT_DIR/engine-rs/target/debug/agent-manager-engine"

# 数据目录与 HOME 都隔离进同一个已解析的 mktemp 目录：被测命令的任何写入都不触及真实 HOME 或用户数据目录。
export AGENT_MANAGER_DATA_DIR="$RUNTIME_DIR/data"
export HOME="$RUNTIME_DIR/home"
mkdir -p "$HOME"

run "$ENGINE_BIN" db:status --json
run "$ENGINE_BIN" policy:get --json
# --dry-run 已退役（请求会被拒绝），这里执行真实写入；不运行任何会下载/安装软件的 apply。
run "$ENGINE_BIN" policy:set --layer local --key skills.enabled --value false --json
printf '==> %s\n' "$ENGINE_BIN policy:get --json（回读校验）"
"$ENGINE_BIN" policy:get --json > "$RUNTIME_DIR/policy-readback.json"
# 回读断言：按 JSON 结构检查 skills.enabled，不依赖字段排列。
node -e 'const result = JSON.parse(require("node:fs").readFileSync(process.argv[1], "utf8")); process.exit(result.ok && result.data.effective.skills.enabled === false ? 0 : 1);' "$RUNTIME_DIR/policy-readback.json"
test -f "$AGENT_MANAGER_DATA_DIR/native-manager.db"
printf '==> ok：policy:set 真实写入并经 policy:get 回读确认；全部写入仅限隔离目录 %s\n' "$RUNTIME_DIR"
