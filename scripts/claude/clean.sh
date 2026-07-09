#!/usr/bin/env bash
# clean.sh — 清理 Claude Code 的会话/历史/缓存等「可删除」数据,保留配置·记忆·技能·插件。
#
# 清理(会自动重建,或已随会话结束而失效):
#   会话记录  projects/<proj>/*.jsonl 及会话 UUID 子目录
#   撤销快照  file-history/<session>
#   任务/作业 tasks/<session>、jobs/<session>
#   会话运行态 session-env/*、sessions/*、shell-snapshots/*
#   缓存      cache、image-cache、paste-cache、stats-cache.json、mcp-needs-auth-cache.json
#   备份/日志 backups、daemon + daemon.log、history.jsonl、telemetry、downloads、ide、plans
#   状态标记  .last-update-result.json、.last-cleanup
#
# 保留(绝不动):
#   projects/*/memory、skills、plugins、modules、
#   settings.json、settings.local.json、statusline.sh、CLAUDE.md
#
# 保护当前/活动会话:自动排除 $CLAUDE_CODE_SESSION_ID,以及最近 N 分钟(默认 10)内改动过的条目,
#   因此在会话运行中执行也不会误删正在用的会话。
#
# 用法:
#   ./scripts/claude/clean.sh --dry-run            预览,零改动
#   ./scripts/claude/clean.sh                       同预览(未加 --yes 不删)
#   ./scripts/claude/clean.sh --yes                 执行清理
#   ./scripts/claude/clean.sh --yes --keep <id>     额外保留某会话(可重复)
#   ./scripts/claude/clean.sh --yes --recent-min 30 放宽"近期保护"到 30 分钟
# 经 manage.sh:
#   ./manage.sh claude:clean [--dry-run|--yes]
#
# 注:set -uo(不含 -e)——对不存在项的删除返回非零属正常,不应中断清理序列。
set -uo pipefail

DRY_RUN=false; ASSUME_YES=false; RECENT_MIN=10
KEEP_IDS=""
[ -n "${CLAUDE_CODE_SESSION_ID:-}" ] && KEEP_IDS="$CLAUDE_CODE_SESSION_ID"
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)    DRY_RUN=true ;;
        --yes|-y)     ASSUME_YES=true ;;
        --keep)       shift; [ -n "${1:-}" ] && KEEP_IDS="$KEEP_IDS ${1:-}" ;;
        --recent-min) shift; RECENT_MIN="${1:-10}" ;;
        -h|--help)    sed -n '2,30p' "$0"; exit 0 ;;
        *) echo "未知参数: $1(可用: --dry-run --yes --keep <id> --recent-min <N>)" >&2; exit 2 ;;
    esac
    shift
done

EXECUTE=false
if [ "$ASSUME_YES" = true ] && [ "$DRY_RUN" = false ]; then EXECUTE=true; fi
CLA="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
[ -d "$CLA" ] || { echo "未找到 $CLA,无需清理。"; exit 0; }

before=$(du -sk "$CLA" 2>/dev/null | cut -f1)

# 返回 0 = 应保留(当前/指定会话,或近 RECENT_MIN 分钟内改动过)
protected() {
    local p="$1" base id
    base="$(basename "$p")"
    for id in $KEEP_IDS; do case "$base" in "$id"*) return 0;; esac; done
    [ -n "$(find "$p" -mmin "-$RECENT_MIN" 2>/dev/null | head -1)" ] && return 0
    return 1
}

rm_path() {  # 删除或预览单个路径(仅对存在项动手)
    local p="$1"
    [ -e "$p" ] || [ -L "$p" ] || return 0
    if [ "$EXECUTE" = true ]; then rm -rf "$p" && echo "   已删 $p"
    else echo "   [预览] 删除 $p"; fi
}

# 逐条清理目录内容,保护当前/活动会话条目
prune_dir() {
    local d="$1" child
    [ -d "$d" ] || return 0
    for child in "$d"/* "$d"/.[!.]*; do
        [ -e "$child" ] || [ -L "$child" ] || continue
        if protected "$child"; then
            [ "$EXECUTE" = false ] && echo "   [保留] $child"
        else
            rm_path "$child"
        fi
    done
}

echo "==============================================================="
if [ "$EXECUTE" = true ]; then
    echo " 正在清理 Claude 会话/缓存等可删除数据…"
else
    echo " 预览:以下为将清理项(当前不改动任何文件)。"
fi
echo " 保护会话: ${KEEP_IDS:-（无,靠近期保护）}   近期保护: ${RECENT_MIN} 分钟"
echo "==============================================================="

echo; echo "[1] 缓存 / 日志 / 状态(整体删除)"
for x in backups cache daemon downloads ide plans telemetry; do rm_path "$CLA/$x"; done
for x in daemon.log history.jsonl mcp-needs-auth-cache.json stats-cache.json .last-update-result.json .last-cleanup; do
    rm_path "$CLA/$x"
done

echo; echo "[2] 会话键目录(保护当前/活动会话)"
for d in file-history tasks jobs session-env shell-snapshots sessions paste-cache image-cache; do
    prune_dir "$CLA/$d"
done

echo; echo "[3] projects/:保留 memory,清理会话记录"
for P in "$CLA"/projects/*/; do
    [ -d "$P" ] || continue
    for jf in "$P"*.jsonl; do
        [ -e "$jf" ] || continue
        if protected "$jf"; then [ "$EXECUTE" = false ] && echo "   [保留] $jf"; else rm_path "$jf"; fi
    done
    for sub in "$P"*/; do
        [ -d "$sub" ] || continue
        b="$(basename "$sub")"
        [ "$b" = memory ] && continue
        case "$b" in
            [0-9a-f]*-[0-9a-f]*-[0-9a-f]*-[0-9a-f]*-[0-9a-f]*)
                if protected "$sub"; then [ "$EXECUTE" = false ] && echo "   [保留] $sub"; else rm_path "$sub"; fi ;;
        esac
    done
done

echo
if [ "$EXECUTE" = true ]; then
    after=$(du -sk "$CLA" 2>/dev/null | cut -f1)
    echo "✅ 清理完成: $((before/1024))MB → $((after/1024))MB(释放 ≈ $(((before-after)/1024))MB)"
    echo "   保留: memory / skills / plugins / modules / settings*.json / statusline.sh / CLAUDE.md"
elif [ "$DRY_RUN" = true ]; then
    echo "🔎 空转预览结束(--dry-run),未改动任何文件。"
else
    echo "🔎 以上为预览,未删除任何内容。执行清理请加 --yes:"
    echo "     ./manage.sh claude:clean --yes"
fi
