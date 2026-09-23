#!/usr/bin/env bash
# scripts/verify-macos-dmg.sh
# 独立验收 macOS DMG 安装包与已安装 app 的运行健康度。
# 用法: verify-macos-dmg.sh [path-to-dmg] [expected-commit]
#   path-to-dmg      缺省按当前 tauri.conf 版本与宿主架构定位标准产物路径
#   expected-commit  缺省取当前仓库 HEAD（app/ 或 engine-rs/ 有源改动时加 -dirty 后缀）
#   期望构建日期恒取 HEAD commit 的 ISO 日期（git show -s --format=%cI HEAD，可公开信息）
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TAURI_CONF="${REPO_ROOT}/app/src-tauri/tauri.conf.json"

# 1. 平台与架构检查（仅 Darwin；期望架构按宿主 x86_64/arm64 动态核对）
OS_NAME="$(uname -s)"
ARCH_NAME="$(uname -m)"
if [[ "$OS_NAME" != "Darwin" ]]; then
  fail "平台拒绝：本验收脚本仅支持 Darwin，当前系统为 ${OS_NAME} ${ARCH_NAME}"
fi
case "$ARCH_NAME" in
  x86_64)
    MACHO_ARCH="x86_64"
    DMG_ARCH_SUFFIX="x64"
    ;;
  arm64)
    MACHO_ARCH="arm64"
    DMG_ARCH_SUFFIX="aarch64"
    ;;
  *)
    fail "平台拒绝：不支持的宿主架构 ${ARCH_NAME}"
    ;;
esac
log_info "平台检查通过: Darwin ${ARCH_NAME}（期望 Mach-O ${MACHO_ARCH} / DMG 后缀 ${DMG_ARCH_SUFFIX}）"

# 2. 期望元数据：版本 / Bundle ID / 产品名读自当前 tauri.conf（不再硬编码）
EXPECTED_VER="$(plutil -extract version raw -o - "$TAURI_CONF" 2>/dev/null || true)"
EXPECTED_BID="$(plutil -extract identifier raw -o - "$TAURI_CONF" 2>/dev/null || true)"
PRODUCT_NAME="$(plutil -extract productName raw -o - "$TAURI_CONF" 2>/dev/null || true)"
if [[ -z "$EXPECTED_VER" ]] || [[ -z "$EXPECTED_BID" ]] || [[ -z "$PRODUCT_NAME" ]]; then
  fail "元数据读取失败：无法从当前 tauri.conf 提取 version/identifier/productName"
fi
log_info "期望元数据（来自 tauri.conf）: version=${EXPECTED_VER} identifier=${EXPECTED_BID} product=${PRODUCT_NAME}"

# 3. DMG 路径：参数优先，缺省按版本与宿主架构定位标准产物路径
DMG_PATH="${1:-${REPO_ROOT}/app/src-tauri/target/release/bundle/dmg/${PRODUCT_NAME}_${EXPECTED_VER}_${DMG_ARCH_SUFFIX}.dmg}"
if [[ ! -f "$DMG_PATH" ]]; then
  fail "DMG 文件不存在: ${DMG_PATH}"
fi
DMG_ABS="$(cd "$(dirname "$DMG_PATH")" && pwd)/$(basename "$DMG_PATH")"

# 4. 期望构建身份：commit 参数优先，缺省从当前仓库推导（完整 HEAD ± app//engine-rs/
#    源脏标记）；期望日期恒取 HEAD commit 的 ISO 日期（%cI，可公开信息），空值即失败。
HEAD_SHA="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || true)"
if [[ ! "$HEAD_SHA" =~ ^[0-9a-f]{40}$ ]]; then
  fail "无法确定期望构建身份（git HEAD 无效），请在有效仓库内运行或检查 HEAD"
fi
EXPECTED_DATE="$(git -C "$REPO_ROOT" show -s --format=%cI HEAD 2>/dev/null || true)"
if [[ -z "$EXPECTED_DATE" ]]; then
  fail "无法确定期望构建日期（git show -s --format=%cI HEAD 为空）"
fi
EXPECTED_COMMIT="${2:-}"
if [[ -z "$EXPECTED_COMMIT" ]]; then
  EXPECTED_COMMIT="$HEAD_SHA"
  if [[ -n "$(git -C "$REPO_ROOT" status --porcelain -- app engine-rs 2>/dev/null)" ]]; then
    EXPECTED_COMMIT="${HEAD_SHA}-dirty"
  fi
fi
if [[ ! "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}(-dirty)?$ ]]; then
  fail "expected-commit 形状非法（需要 40 位小写 hex，可选 -dirty 后缀）"
fi
log_info "期望构建身份: commit=${EXPECTED_COMMIT} date=${EXPECTED_DATE}"

# 5. 隔离工作区与清理 Trap 设置
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

# 6. 只读挂载 DMG
log_info "挂载 DMG (只读、无自动打开)..."
mkdir -p "$MOUNT_DIR"
if ! hdiutil attach "$DMG_ABS" -mountpoint "$MOUNT_DIR" -readonly -nobrowse -noautoopen >/dev/null 2>&1; then
  fail "阶段失败 [挂载 DMG]：无法只读挂载 ${DMG_ABS}"
fi
MOUNTED=1

APP_ON_MOUNT="${MOUNT_DIR}/${PRODUCT_NAME}.app"
if [[ ! -d "$APP_ON_MOUNT" ]]; then
  fail "阶段失败 [检查 DMG 内容]：挂载点内未找到 ${PRODUCT_NAME}.app，挂载目录内容:\n$(ls -la "$MOUNT_DIR" 2>/dev/null | head -n 10)"
fi

# 7. 复制 App 到仓库外临时 Applications 目录并立即卸载 DMG
log_info "使用 ditto 复制 ${PRODUCT_NAME}.app 到仓库外隔离目录 ${APPS_DIR}..."
mkdir -p "$APPS_DIR"
COPIED_APP="${APPS_DIR}/${PRODUCT_NAME}.app"
if ! ditto "$APP_ON_MOUNT" "$COPIED_APP"; then
  fail "阶段失败 [复制 App]：无法将 app 复制到 ${APPS_DIR}"
fi

log_info "卸载 DMG..."
if ! hdiutil detach "$MOUNT_DIR" -force >/dev/null 2>&1; then
  fail "阶段失败 [卸载 DMG]：无法卸载挂载点 ${MOUNT_DIR}"
fi
MOUNTED=0

# 8. 元数据与包内容静态检查
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
if [[ "$BUNDLE_ID" != "$EXPECTED_BID" ]]; then
  fail "阶段失败 [元数据检查]：Bundle Identifier 不匹配，期望 ${EXPECTED_BID}（当前 tauri.conf），实际为 ${BUNDLE_ID}"
fi
log_info "Bundle Identifier 校验通过: ${BUNDLE_ID}"

if [[ -z "$BUNDLE_VER" ]]; then
  fail "阶段失败 [元数据检查]：无法提取 CFBundleShortVersionString"
fi
if [[ "$BUNDLE_VER" != "$EXPECTED_VER" ]]; then
  fail "阶段失败 [元数据检查]：Bundle Version 不匹配，期望 ${EXPECTED_VER}（当前 tauri.conf），实际为 ${BUNDLE_VER}"
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

# Mach-O 架构检查（按宿主动态核对）
FILE_OUTPUT="$(file "$APP_BIN")"
if ! echo "$FILE_OUTPUT" | grep -q "$MACHO_ARCH"; then
  fail "阶段失败 [架构检查]：二进制不含宿主架构 ${MACHO_ARCH}: ${FILE_OUTPUT}"
fi
log_info "Mach-O 架构: ${MACHO_ARCH} ($(echo "$FILE_OUTPUT" | cut -d: -f2- | xargs))"

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
CURRENT_REPO="$REPO_ROOT"

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

# 构建身份核验（发布候选 fail-closed）：包内必须有且仅有 1 处与期望一致的
# build-commit 标记；clean 期望不得命中 -dirty 标记，反之亦然；缺失、不匹配或
# 不唯一均拒绝。标记是不带分隔符的字符串常量，编译器可能把它与相邻常量排在一起，
# strings 输出里同一行会连着别的文字，因此按标记格式精确提取取值再比对，不依赖整行。
EXPECTED_MARKER="agent-manager-build-commit:${EXPECTED_COMMIT}"
MARKERS_FILE="${TEMP_DIR}/build_commit_markers.txt"
grep -aoE 'agent-manager-build-commit:[0-9a-f]{40}(-dirty)?' "$STRINGS_FILE" > "$MARKERS_FILE" || true
MARKER_COUNT="$(grep -Fxc "$EXPECTED_MARKER" "$MARKERS_FILE" || true)"
if [[ "$MARKER_COUNT" == "0" ]]; then
  FOUND_MARKER="$(head -n 1 "$MARKERS_FILE" || true)"
  if [[ -n "$FOUND_MARKER" ]]; then
    fail "阶段失败 [build identity 核验]：包内构建身份与期望不匹配，期望 commit=${EXPECTED_COMMIT}，实际标记: ${FOUND_MARKER}"
  fi
  fail "阶段失败 [build identity 核验]：包内缺少构建身份标记（非发布候选构建或未注入构建元数据）"
fi
if [[ "$(wc -l < "$MARKERS_FILE" | tr -d ' ')" != "1" ]]; then
  MARKER_COUNT="$(wc -l < "$MARKERS_FILE" | tr -d ' ')"
fi
if [[ "$MARKER_COUNT" != "1" ]]; then
  fail "阶段失败 [build identity 核验]：包内构建身份标记不唯一（${MARKER_COUNT} 处）"
fi
log_info "build identity 核验通过: commit=${EXPECTED_COMMIT}"

# 构建日期核验（与 commit 同级 fail-closed）：agent-manager-build-date: 整行
# 精确匹配期望日期且恰好 1 处；缺失、不匹配、不唯一均拒绝。
EXPECTED_DATE_MARKER="agent-manager-build-date:${EXPECTED_DATE}"
DATE_MARKERS_FILE="${TEMP_DIR}/build_date_markers.txt"
grep -aoE 'agent-manager-build-date:[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(Z|[+-][0-9]{2}:[0-9]{2})' "$STRINGS_FILE" > "$DATE_MARKERS_FILE" || true
DATE_MARKER_COUNT="$(grep -Fxc "$EXPECTED_DATE_MARKER" "$DATE_MARKERS_FILE" || true)"
if [[ "$DATE_MARKER_COUNT" == "0" ]]; then
  FOUND_DATE_MARKER="$(head -n 1 "$DATE_MARKERS_FILE" || true)"
  if [[ -n "$FOUND_DATE_MARKER" ]]; then
    fail "阶段失败 [build identity 核验]：包内构建日期与期望不匹配，期望 date=${EXPECTED_DATE}，实际标记: ${FOUND_DATE_MARKER}"
  fi
  fail "阶段失败 [build identity 核验]：包内缺少构建日期标记（非发布候选构建或未注入构建元数据）"
fi
if [[ "$(wc -l < "$DATE_MARKERS_FILE" | tr -d ' ')" != "1" ]]; then
  DATE_MARKER_COUNT="$(wc -l < "$DATE_MARKERS_FILE" | tr -d ' ')"
fi
if [[ "$DATE_MARKER_COUNT" != "1" ]]; then
  fail "阶段失败 [build identity 核验]：包内构建日期标记不唯一（${DATE_MARKER_COUNT} 处）"
fi
log_info "build date 核验通过: date=${EXPECTED_DATE}"

# 检查代码签名状态
CODESIGN_DISPLAY="$(codesign -dvv "$COPIED_APP" 2>&1 || true)"
if echo "$CODESIGN_DISPLAY" | grep -q "code object is not signed at all"; then
  log_info "代码签名状态: 未签名"
  log_info "Apple 公证状态: 不适用（未签名包不做公证票据检查，本地验收不要求 Apple 公证）"
elif echo "$CODESIGN_DISPLAY" | grep -q "Signature=adhoc"; then
  if ! codesign --verify --deep --strict "$COPIED_APP" >/dev/null 2>&1; then
    fail "阶段失败 [代码签名校验]：adhoc 签名深度严格验证失败"
  fi
  log_info "代码签名状态: adhoc (已通过深度严格校验)"
  log_info "Apple 公证状态: 不适用（adhoc 签名不做公证票据检查，本地验收不要求 Apple 公证）"
else
  AUTHORITY="$(echo "$CODESIGN_DISPLAY" | grep "Authority=" | head -n 1 | cut -d= -f2-)"
  if ! codesign --verify --deep --strict "$COPIED_APP" >/dev/null 2>&1; then
    fail "阶段失败 [代码签名校验]：检测到签名但深度严格验证失败"
  fi
  log_info "代码签名状态: 有效签名 (${AUTHORITY:-未知}) (已通过深度严格校验)"
  # 已签名分支追加公证票据检查：stapler validate 未通过只如实报告，
  # 不改变本地验收结论（本地验证环境不强制 Apple 公证）。
  if STAPLER_OUTPUT="$(xcrun stapler validate "$COPIED_APP" 2>&1)"; then
    log_info "Apple 公证票据检查通过: $(echo "$STAPLER_OUTPUT" | tail -n 1)"
  else
    log_info "Apple 公证票据状态: 未装订有效票据或校验未通过（本地验收不强制 Apple 公证）: $(echo "$STAPLER_OUTPUT" | head -n 1)"
  fi
fi

# 9. 独立运行存活验收
# 前置守卫：应用启用单实例插件，若宿主机已有同标识实例在运行（含托盘驻留），
# 验收副本会立即转发唤回并自行退出，存活检测必然误判失败。提前检出并如实拒绝。
RUNNING_PID="$(pgrep -f "agent-manager-app" | head -n 1 || true)"
if [[ -n "$RUNNING_PID" ]]; then
  fail "阶段失败 [独立运行存活检测]：检测到正在运行的 Agent Manager 实例 (PID: ${RUNNING_PID})。单实例语义下验收副本会被立即唤回退出，请先通过托盘菜单「退出」该实例后重跑验收"
fi
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
