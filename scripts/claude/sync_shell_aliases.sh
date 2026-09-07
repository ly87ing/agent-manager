#!/usr/bin/env bash
# sync_shell_aliases.sh — 幂等地将 agent-manager 托管的 shell alias 同步到
# ~/.zshrc(interactive shell 读)和 ~/.zprofile(login shell 读)。
#
# 为什么两个文件:
#   ~/.zshrc 只在「交互式」zsh 里加载,别名放这里时,非交互的 login shell
#   (如某些远程终端 / UU 远程 term)读不到,别名就失效。~/.zprofile 是
#   login shell(含非交互 login)会读的文件,把同一份受管区块也写进去,
#   非交互 login shell 也能用别名。
#
# 原理:
#   1. 从 config/shell-aliases.json 读取 alias 定义
#   2. 在每个目标文件里用标记注释圈定一个「受管区块」
#   3. 每次执行时用声明的内容替换该区块(幂等)
#   4. 支持 --dry-run 模式
#
# 用法:
#   ./scripts/claude/sync_shell_aliases.sh [--dry-run]
#
# 安全保证:
#   - 仅修改标记注释之间的内容,不触碰用户手写的其它行
#   - 首次运行时追加区块到文件末尾
#   - 每个目标文件独立幂等处理
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_FILE="$ROOT_DIR/config/shell-aliases.json"
# .zshrc:交互式 shell;.zprofile:login shell(含非交互 login,如 UU 远程 term)
RC_FILES=("${HOME}/.zshrc" "${HOME}/.zprofile")

MARKER_BEGIN="# >>> agent-manager shell aliases >>>"
MARKER_END="# <<< agent-manager shell aliases <<<"

DRY_RUN=false
TARGETS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=true; shift ;;
        --target)
            TARGETS+=("$2"); shift 2 ;;
        --only)
            TARGETS+=("$2"); shift 2 ;;
        *)
            echo "未知参数: $1" >&2
            exit 2 ;;
    esac
done

if [[ ${#TARGETS[@]} -gt 0 ]]; then
    RC_FILES=()
    for t in "${TARGETS[@]}"; do
        if [[ "$t" == *":"* && "$t" != "aliases:"* ]]; then
            echo "跨域元素 scope 拒绝: $t" >&2
            exit 2
        fi
        norm="${t#aliases:}"
        case "$norm" in
            zshrc) RC_FILES+=("${HOME}/.zshrc") ;;
            zprofile) RC_FILES+=("${HOME}/.zprofile") ;;
            *)
                echo "未知 alias 目标: $norm" >&2
                exit 2 ;;
        esac
    done
fi

# ---------- 1. 读取声明式配置 ----------

if [ ! -f "$CONFIG_FILE" ]; then
    echo "⚠️  配置文件不存在: $CONFIG_FILE,跳过 shell alias 同步。"
    exit 0
fi

# 用 python 解析 JSON,输出 alias 行(name<TAB>value 格式)
ALIAS_LINES="$(python3 - "$CONFIG_FILE" <<'PY'
import json, sys
cfg = json.loads(open(sys.argv[1], encoding="utf-8").read())
timezone = cfg.get("timezone", "America/Los_Angeles")
lang = cfg.get("lang", "en_US.UTF-8")
aliases = cfg.get("aliases", {})
for name, value in sorted(aliases.items()):
    # Replace placeholders if present
    value = value.replace("{timezone}", timezone)
    value = value.replace("{lang}", lang)
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

# ---------- 3. per-file 同步(幂等) ----------

sync_managed_block_to_file() {
    local rc_file="$1"
    local current_content new_content

    # 确保文件存在
    if [ ! -f "$rc_file" ]; then
        if [ -z "$MANAGED_BLOCK" ]; then
            return 0
        fi
        if $DRY_RUN; then
            echo "预览模式:计划创建 $rc_file 并写入受管 alias 区块。"
            return 0
        fi
        touch "$rc_file"
    fi

    current_content="$(cat "$rc_file")"

    if echo "$current_content" | grep -qF "$MARKER_BEGIN"; then
        # 区块已存在 → 替换
        new_content="$(python3 - "$MARKER_BEGIN" "$MARKER_END" "$rc_file" <<PY
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
            echo "✅ $rc_file 无需同步(alias 列表为空且无受管区块)。"
            return 0
        fi
        # 确保追加前有空行分隔
        if [ -n "$current_content" ] && [ "$(echo "$current_content" | tail -c1)" != "" ]; then
            new_content="$current_content

$MANAGED_BLOCK
"
        else
            new_content="${current_content}
$MANAGED_BLOCK
"
        fi
    fi

    # 幂等检查
    if [ "$current_content" = "$(echo -n "$new_content")" ] || [ "$current_content
" = "$new_content" ]; then
        echo "✅ $rc_file 的 shell alias 已是最新,无需修改。"
        return 0
    fi

    if $DRY_RUN; then
        echo "预览模式:计划更新 $rc_file 中的受管 alias 区块:"
        echo "---"
        echo "$MANAGED_BLOCK"
        echo "---"
        return 0
    fi

    printf '%s' "$new_content" > "$rc_file"
    echo "✅ 已同步 shell alias 到 $rc_file"
}

for rc in "${RC_FILES[@]}"; do
    sync_managed_block_to_file "$rc"
done
