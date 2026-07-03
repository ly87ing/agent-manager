#!/usr/bin/env bash
# sync_shell_aliases.sh — 幂等地将 agent-manager 托管的 shell alias 同步到 ~/.zshrc
#
# 原理：
#   1. 从 config/shell-aliases.json 读取 alias 定义
#   2. 在 ~/.zshrc 中用标记注释圈定一个「受管区块」
#   3. 每次执行时用声明的内容替换该区块（幂等）
#   4. 支持 --dry-run 模式
#
# 用法：
#   ./scripts/claude/sync_shell_aliases.sh [--dry-run]
#
# 安全保证：
#   - 仅修改标记注释之间的内容，不触碰用户手写的其它行
#   - 首次运行时追加区块到文件末尾
#   - 区块为空时自动清理标记
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_FILE="$ROOT_DIR/config/shell-aliases.json"
RC_FILE="${HOME}/.zshrc"

MARKER_BEGIN="# >>> agent-manager shell aliases >>>"
MARKER_END="# <<< agent-manager shell aliases <<<"

DRY_RUN=false
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=true ;;
    esac
done

# ---------- 1. 读取声明式配置 ----------

if [ ! -f "$CONFIG_FILE" ]; then
    echo "⚠️  配置文件不存在: $CONFIG_FILE，跳过 shell alias 同步。"
    exit 0
fi

# 用 python 解析 JSON，输出 alias 行（name<TAB>value 格式）
ALIAS_LINES="$(python3 - "$CONFIG_FILE" <<'PY'
import json, sys
cfg = json.loads(open(sys.argv[1], encoding="utf-8").read())
timezone = cfg.get("timezone", "America/Los_Angeles")
aliases = cfg.get("aliases", {})
for name, value in sorted(aliases.items()):
    # Replace timezone placeholder if present
    value = value.replace("{timezone}", timezone)
    print(f"alias {name}='{value}'")
PY
)"

# ---------- 2. 生成受管区块内容 ----------

if [ -z "$ALIAS_LINES" ]; then
    MANAGED_BLOCK=""
else
    MANAGED_BLOCK="$MARKER_BEGIN
$ALIAS_LINES
$MARKER_END"
fi

# ---------- 3. 确保 RC 文件存在 ----------

if [ ! -f "$RC_FILE" ]; then
    if $DRY_RUN; then
        echo "预览模式：计划创建 $RC_FILE 并写入受管 alias 区块。"
        exit 0
    fi
    touch "$RC_FILE"
fi

# ---------- 4. 读取当前文件内容 ----------

CURRENT_CONTENT="$(cat "$RC_FILE")"

# ---------- 5. 计算新文件内容 ----------

if echo "$CURRENT_CONTENT" | grep -qF "$MARKER_BEGIN"; then
    # 区块已存在 → 替换
    NEW_CONTENT="$(python3 - "$MARKER_BEGIN" "$MARKER_END" "$RC_FILE" <<PY
import sys

marker_begin = sys.argv[1]
marker_end   = sys.argv[2]
rc_path      = sys.argv[3]
managed_block = """$MANAGED_BLOCK"""

with open(rc_path, encoding="utf-8") as f:
    lines = f.readlines()

result = []
inside = False
replaced = False
for line in lines:
    stripped = line.rstrip("\n")
    if stripped == marker_begin:
        inside = True
        if managed_block and not replaced:
            result.append(managed_block + "\n")
            replaced = True
        continue
    if stripped == marker_end:
        inside = False
        continue
    if inside:
        continue
    result.append(line)

print("".join(result), end="")
PY
)"
else
    # 区块不存在 → 追加
    if [ -z "$MANAGED_BLOCK" ]; then
        echo "✅ 无需同步（alias 列表为空且 RC 文件中无受管区块）。"
        exit 0
    fi
    # 确保追加前有空行分隔
    if [ -n "$CURRENT_CONTENT" ] && [ "$(echo "$CURRENT_CONTENT" | tail -c1)" != "" ]; then
        NEW_CONTENT="$CURRENT_CONTENT

$MANAGED_BLOCK
"
    else
        NEW_CONTENT="${CURRENT_CONTENT}
$MANAGED_BLOCK
"
    fi
fi

# ---------- 6. 幂等检查 ----------

if [ "$CURRENT_CONTENT" = "$(echo -n "$NEW_CONTENT")" ] || [ "$CURRENT_CONTENT
" = "$NEW_CONTENT" ]; then
    echo "✅ Shell alias 已是最新，无需修改。"
    exit 0
fi

# ---------- 7. 写入 ----------

if $DRY_RUN; then
    echo "预览模式：计划更新 $RC_FILE 中的受管 alias 区块："
    echo "---"
    echo "$MANAGED_BLOCK"
    echo "---"
    exit 0
fi

printf '%s' "$NEW_CONTENT" > "$RC_FILE"
echo "✅ 已同步 shell alias 到 $RC_FILE"
