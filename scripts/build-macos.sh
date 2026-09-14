#!/usr/bin/env bash
# scripts/build-macos.sh
# 编译并生成 macOS 安装包（.app 与 .dmg），启用源码与主目录路径脱敏重映射
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# 严格限定平台：仅支持 Darwin（x86_64 与 arm64 宿主机各自构建本机架构）
OS_NAME="$(uname -s)"
ARCH_NAME="$(uname -m)"
if [[ "${OS_NAME}" != "Darwin" ]]; then
  echo "错误：仅支持在 Darwin 环境执行 macOS 打包，当前环境: ${OS_NAME} ${ARCH_NAME}" >&2
  exit 1
fi
case "${ARCH_NAME}" in
  x86_64|arm64) ;;
  *)
    echo "错误：不支持的宿主架构: ${ARCH_NAME}" >&2
    exit 1
    ;;
esac

# 构建身份：完整 HEAD SHA + commit ISO 日期 + app/ 与 engine-rs/ 源脏标记。
# 缺失或形状异常即失败（发布候选必须可溯源）；app/ 或 engine-rs/ 存在
# tracked/staged/untracked 源改动时嵌入 -dirty 后缀，不冒充 clean commit。
HEAD_SHA="$(git -C "${REPO_ROOT}" rev-parse HEAD 2>/dev/null || true)"
if [[ ! "${HEAD_SHA}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "错误：无法取得完整 HEAD SHA（构建入口要求 git 仓库与有效 HEAD）" >&2
  exit 1
fi
COMMIT_DATE="$(git -C "${REPO_ROOT}" show -s --format=%cI HEAD 2>/dev/null || true)"
if [[ -z "${COMMIT_DATE}" ]]; then
  echo "错误：无法取得 HEAD commit 的 ISO 日期" >&2
  exit 1
fi
BUILD_COMMIT="${HEAD_SHA}"
if [[ -n "$(git -C "${REPO_ROOT}" status --porcelain -- app engine-rs 2>/dev/null)" ]]; then
  BUILD_COMMIT="${HEAD_SHA}-dirty"
fi
export AGENT_MANAGER_BUILD_COMMIT="${BUILD_COMMIT}"
export AGENT_MANAGER_BUILD_DATE="${COMMIT_DATE}"

# 启用源码路径与构建主目录重映射，防止本机绝对路径泄露到发布二进制中
# 注意：长路径（REPO_ROOT）在前，短路径（HOME）在后
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=${REPO_ROOT}=/YOUR_SOURCE_ROOT --remap-path-prefix=${HOME}=/YOUR_BUILD_HOME"

echo "[INFO] 开始构建 macOS 安装包（app + dmg）..."
echo "[INFO] 构建平台: ${OS_NAME} ${ARCH_NAME}"
echo "[INFO] 构建身份: commit=${BUILD_COMMIT} date=${COMMIT_DATE}"
echo "[INFO] 路径重映射已启用（源码根目录与主目录均已配置脱敏重映射）"

cd "${REPO_ROOT}/app"
pnpm tauri build --bundles app,dmg

DMG_REL="app/src-tauri/target/release/bundle/dmg"
APP_REL="app/src-tauri/target/release/bundle/macos"

echo "[INFO] 构建完成！"
if [[ -d "${REPO_ROOT}/${APP_REL}" ]]; then
  echo "[INFO] .app 产物目录: ${APP_REL}"
fi
if [[ -d "${REPO_ROOT}/${DMG_REL}" ]]; then
  echo "[INFO] .dmg 产物目录: ${DMG_REL}"
fi
