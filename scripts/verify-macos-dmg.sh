#!/usr/bin/env bash
# scripts/verify-macos-dmg.sh
# 独立验收 macOS DMG 安装包与已安装 app 的运行健康度
set -euo pipefail

log_info() {
  echo "[INFO] $*"
}

log_error() {
  echo "[ERROR] $*" >&2
}

fail() {
  log_error "$*"
  exit 1
}

# 1. 参数与文件检查
if [[ $# -lt 1 ]] || [[ -z "${1:-}" ]]; then
  echo "用法: $0 <path-to-dmg>" >&2
  exit 1
fi

DMG_PATH="$1"
if [[ ! -f "$DMG_PATH" ]]; then
  fail "DMG 文件不存在: ${DMG_PATH}"
fi
DMG_ABS="$(cd "$(dirname "$DMG_PATH")" && pwd)/$(basename "$DMG_PATH")"

# 2. 平台与架构限制（严格限定 Darwin x86_64）
OS_NAME="$(uname -s)"
ARCH_NAME="$(uname -m)"
if [[ "$OS_NAME" != "Darwin" ]] || [[ "$ARCH_NAME" != "x86_64" ]]; then
  fail "平台拒绝：本验收脚本仅支持 Darwin x86_64，当前系统为 ${OS_NAME} ${ARCH_NAME}"
fi
log_info "平台检查通过: Darwin x86_64"

# 3. 隔离工作区与清理 Trap 设置
TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/am-verify.XXXXXX")"
MOUNT_DIR="${TEMP_DIR}/mount"
APPS_DIR="${TEMP_DIR}/Applications"
ISOLATED_HOME="${TEMP_DIR}/home"
ISOLATED_DATA="${TEMP_DIR}/data"
APP_LOG="${TEMP_DIR}/app_exec.log"

MOUNTED=0
APP_PID=""

cleanup() {
  local exit_code=$?
  # 终止应用进程
  if [[ -n "${APP_PID:-}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    log_info "清理：终止运行中的测试 app 进程 (PID: $APP_PID)..."
    kill -TERM "$APP_PID" 2>/dev/null || true
    for _ in {1..10}; do
      if ! kill -0 "$APP_PID" 2>/dev/null; then
        break
      fi
      sleep 0.2
    done
    if kill -0 "$APP_PID" 2>/dev/null; then
      kill -9 "$APP_PID" 2>/dev/null || true
    fi
  fi

  # 卸载挂载点
  if [[ "$MOUNTED" -eq 1 ]]; then
    log_info "清理：卸载挂载点 ${MOUNT_DIR}..."
    hdiutil detach "$MOUNT_DIR" -force >/dev/null 2>&1 || true
    MOUNTED=0
  fi

  # 删除临时目录
  if [[ -d "${TEMP_DIR:-}" ]]; then
    rm -rf "$TEMP_DIR"
  fi

  exit "$exit_code"
}
trap cleanup EXIT INT TERM HUP

# 4. 只读挂载 DMG
log_info "挂载 DMG (只读、无自动打开)..."
mkdir -p "$MOUNT_DIR"
if ! hdiutil attach "$DMG_ABS" -mountpoint "$MOUNT_DIR" -readonly -nobrowse -noautoopen >/dev/null 2>&1; then
  fail "阶段失败 [挂载 DMG]：无法只读挂载 ${DMG_ABS}"
fi
MOUNTED=1

APP_ON_MOUNT="${MOUNT_DIR}/Agent Manager.app"
if [[ ! -d "$APP_ON_MOUNT" ]]; then
  fail "阶段失败 [检查 DMG 内容]：挂载点内未找到 Agent Manager.app，挂载目录内容:\n$(ls -la "$MOUNT_DIR" 2>/dev/null | head -n 10)"
fi

# 5. 复制 App 到仓库外临时 Applications 目录并立即卸载 DMG
log_info "使用 ditto 复制 Agent Manager.app 到仓库外隔离目录 ${APPS_DIR}..."
mkdir -p "$APPS_DIR"
COPIED_APP="${APPS_DIR}/Agent Manager.app"
if ! ditto "$APP_ON_MOUNT" "$COPIED_APP"; then
  fail "阶段失败 [复制 App]：无法将 app 复制到 ${APPS_DIR}"
fi

log_info "卸载 DMG..."
if ! hdiutil detach "$MOUNT_DIR" -force >/dev/null 2>&1; then
  fail "阶段失败 [卸载 DMG]：无法卸载挂载点 ${MOUNT_DIR}"
fi
MOUNTED=0

# 6. 元数据与包内容静态检查
INFO_PLIST="${COPIED_APP}/Contents/Info.plist"
if [[ ! -f "$INFO_PLIST" ]]; then
  fail "阶段失败 [元数据检查]：未找到 Info.plist: ${INFO_PLIST}"
fi

BUNDLE_ID="$(plutil -extract CFBundleIdentifier raw "$INFO_PLIST" 2>/dev/null || defaults read "${INFO_PLIST%.plist}" CFBundleIdentifier 2>/dev/null || echo "")"
BUNDLE_VER="$(plutil -extract CFBundleShortVersionString raw "$INFO_PLIST" 2>/dev/null || defaults read "${INFO_PLIST%.plist}" CFBundleShortVersionString 2>/dev/null || echo "")"
BUNDLE_EXEC="$(plutil -extract CFBundleExecutable raw "$INFO_PLIST" 2>/dev/null || defaults read "${INFO_PLIST%.plist}" CFBundleExecutable 2>/dev/null || echo "")"

if [[ -z "$BUNDLE_ID" ]]; then
  fail "阶段失败 [元数据检查]：无法提取 CFBundleIdentifier"
fi
if [[ "$BUNDLE_ID" != "com.agentmanager.dev" ]]; then
  fail "阶段失败 [元数据检查]：Bundle Identifier 不匹配，期望 com.agentmanager.dev，实际为 ${BUNDLE_ID}"
fi
log_info "Bundle Identifier 校验通过: ${BUNDLE_ID}"

if [[ -z "$BUNDLE_VER" ]]; then
  fail "阶段失败 [元数据检查]：无法提取 CFBundleShortVersionString"
fi
if [[ "$BUNDLE_VER" != "0.1.0" ]]; then
  fail "阶段失败 [元数据检查]：Bundle Version 不匹配，期望 0.1.0，实际为 ${BUNDLE_VER}"
fi
log_info "Bundle Version 校验通过: ${BUNDLE_VER}"

if [[ -z "$BUNDLE_EXEC" ]]; then
  fail "阶段失败 [元数据检查]：无法提取 CFBundleExecutable"
fi
log_info "CFBundleExecutable: ${BUNDLE_EXEC}"

APP_BIN="${COPIED_APP}/Contents/MacOS/${BUNDLE_EXEC}"
if [[ ! -f "$APP_BIN" ]]; then
  fail "阶段失败 [二进制检查]：未找到可执行文件 ${APP_BIN}"
fi
if [[ ! -x "$APP_BIN" ]]; then
  fail "阶段失败 [二进制检查]：文件缺少可执行权限 ${APP_BIN}"
fi

# Mach-O 架构检查
FILE_OUTPUT="$(file "$APP_BIN")"
if ! echo "$FILE_OUTPUT" | grep -q "x86_64"; then
  fail "阶段失败 [架构检查]：二进制不是 x86_64 架构: ${FILE_OUTPUT}"
fi
log_info "Mach-O 架构: x86_64 ($(echo "$FILE_OUTPUT" | cut -d: -f2- | xargs))"

# otool 动态依赖检查（严禁包含 Python）
OTOOL_OUTPUT="$(otool -L "$APP_BIN" 2>&1 || true)"
if echo "$OTOOL_OUTPUT" | grep -iq "python"; then
  LEAK_LIB="$(echo "$OTOOL_OUTPUT" | grep -i "python" | head -n 5)"
  fail "阶段失败 [动态依赖检查]：二进制存在 Python 动态依赖:\n${LEAK_LIB}"
fi
log_info "动态依赖检查通过: 无 Python 动态链接"

# 包内遗留资源检查（严禁 .py, .pyc, manage.sh, python 可执行资源）
PYTHON_ARTIFACTS="$(find "$COPIED_APP" \( -name "*.py" -o -name "*.pyc" -o -name "manage.sh" -o -name "python" -o -name "python3" \) 2>/dev/null || true)"
if [[ -n "$PYTHON_ARTIFACTS" ]]; then
  LEAK_FILES="$(echo "$PYTHON_ARTIFACTS" | head -n 10)"
  fail "阶段失败 [资源卫生检查]：app 包内发现 Python/旧脚本残留:\n${LEAK_FILES}"
fi
log_info "资源卫生检查通过: 无 .py/.pyc/manage.sh/python 遗留"

# 路径脱敏检查（严禁泄露本机构建 HOME 或源码根目录绝对路径）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CURRENT_REPO="$(cd "${SCRIPT_DIR}/.." && pwd)"

STRINGS_FILE="${TEMP_DIR}/app_bin_strings.txt"
strings "$APP_BIN" > "$STRINGS_FILE"

if grep -Fq "$CURRENT_REPO" "$STRINGS_FILE"; then
  SAMPLE="$(grep -F "$CURRENT_REPO" "$STRINGS_FILE" | head -n 3)"
  fail "阶段失败 [路径脱敏检查]：二进制中发现源码根目录路径泄露:\n${SAMPLE}"
fi

if grep -Fq "$HOME" "$STRINGS_FILE"; then
  SAMPLE="$(grep -F "$HOME" "$STRINGS_FILE" | head -n 3)"
  fail "阶段失败 [路径脱敏检查]：二进制中发现本机构建 HOME 路径泄露:\n${SAMPLE}"
fi
log_info "路径脱敏检查通过: 二进制不包含本机构建 HOME 或源码绝对路径"

# 检查代码签名状态
CODESIGN_DISPLAY="$(codesign -dvv "$COPIED_APP" 2>&1 || true)"
if echo "$CODESIGN_DISPLAY" | grep -q "code object is not signed at all"; then
  log_info "代码签名状态: 未签名"
elif echo "$CODESIGN_DISPLAY" | grep -q "Signature=adhoc"; then
  if ! codesign --verify --deep --strict "$COPIED_APP" >/dev/null 2>&1; then
    fail "阶段失败 [代码签名校验]：adhoc 签名深度严格验证失败"
  fi
  log_info "代码签名状态: adhoc (已通过深度严格校验)"
else
  AUTHORITY="$(echo "$CODESIGN_DISPLAY" | grep "Authority=" | head -n 1 | cut -d= -f2-)"
  if ! codesign --verify --deep --strict "$COPIED_APP" >/dev/null 2>&1; then
    fail "阶段失败 [代码签名校验]：检测到签名但深度严格验证失败"
  fi
  log_info "代码签名状态: 有效签名 (${AUTHORITY:-未知}) (已通过深度严格校验)"
fi
log_info "Apple 公证状态: 未验证（本地验证环境不要求 Apple 公证）"

# 7. 独立运行存活验收
log_info "准备隔离运行环境（工作目录 /, 隔离 HOME, 独立 DATA_DIR, 最小 PATH=/usr/bin:/bin）..."
mkdir -p "$ISOLATED_HOME" "$ISOLATED_DATA"

cd /

env -i \
  HOME="$ISOLATED_HOME" \
  AGENT_MANAGER_DATA_DIR="$ISOLATED_DATA" \
  PATH="/usr/bin:/bin" \
  "$APP_BIN" > "$APP_LOG" 2>&1 &
APP_PID=$!

log_info "已启动应用进程 (PID: ${APP_PID})，开始 5 秒持续存活检测..."

SURVIVAL_SECONDS=5
for ((i = 1; i <= SURVIVAL_SECONDS; i++)); do
  sleep 1
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    APP_PID=""
    LOG_SAMPLE="$(head -n 20 "$APP_LOG" 2>/dev/null || true)"
    fail "阶段失败 [独立运行存活检测]：应用在第 ${i} 秒异常退出！日志前 20 行:\n${LOG_SAMPLE}"
  fi
  # 若系统存在 lsof，检查进程 cwd 及已打开文件句柄，确保未引用源码仓库
  if command -v lsof >/dev/null 2>&1; then
    LSOF_FILE="${TEMP_DIR}/app_lsof.txt"
    lsof -p "$APP_PID" > "$LSOF_FILE" 2>/dev/null || true
    if grep -Fq "$CURRENT_REPO" "$LSOF_FILE"; then
      SAMPLE="$(grep -F "$CURRENT_REPO" "$LSOF_FILE" | head -n 3)"
      fail "阶段失败 [运行环境隔离检查]：应用进程 cwd 或打开的文件句柄引用了源码仓库目录:\n${SAMPLE}"
    fi
  fi
  log_info "存活健康检测正常 (${i}/${SURVIVAL_SECONDS}s)..."
done

if command -v lsof >/dev/null 2>&1; then
  log_info "运行隔离检查通过: 进程 cwd 与打开文件未引用源码仓库"
fi

log_info "应用已持续存活 ${SURVIVAL_SECONDS} 秒，发送 SIGTERM 优雅退出..."
kill -TERM "$APP_PID" 2>/dev/null || true

KILLED_BY_SIGKILL=0
for _ in {1..15}; do
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    break
  fi
  sleep 0.2
done

if kill -0 "$APP_PID" 2>/dev/null; then
  log_info "进程未在窗口期内退出，发送 SIGKILL..."
  kill -9 "$APP_PID" 2>/dev/null || true
  KILLED_BY_SIGKILL=1
fi
wait "$APP_PID" 2>/dev/null || true
APP_PID=""

if [[ "$KILLED_BY_SIGKILL" -eq 1 ]]; then
  log_info "验收进程已终止并清理。"
else
  log_info "应用进程已正常退出并清理。"
fi
log_info "================================================="
log_info "macOS DMG 安装包与独立运行验收全部通过！"
log_info "================================================="
