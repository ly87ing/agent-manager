#!/usr/bin/env bash
# scripts/build-macos.sh
# 编译并生成 macOS 安装包（.app 与 .dmg），启用源码与主目录路径脱敏重映射
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# 严格限定平台：仅支持 Darwin x86_64
OS_NAME="$(uname -s)"
ARCH_NAME="$(uname -m)"
if [[ "${OS_NAME}" != "Darwin" ]] || [[ "${ARCH_NAME}" != "x86_64" ]]; then
  echo "错误：仅支持在 Darwin x86_64 环境执行 macOS 打包，当前环境: ${OS_NAME} ${ARCH_NAME}" >&2
  exit 1
fi

# 启用源码路径与构建主目录重映射，防止本机绝对路径泄露到发布二进制中
# 注意：长路径（REPO_ROOT）在前，短路径（HOME）在后
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=${REPO_ROOT}=/YOUR_SOURCE_ROOT --remap-path-prefix=${HOME}=/YOUR_BUILD_HOME"

echo "[INFO] 开始构建 macOS 安装包（app + dmg）..."
echo "[INFO] 构建平台: ${OS_NAME} ${ARCH_NAME}"
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
