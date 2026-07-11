#!/usr/bin/env bash
# uninstall.sh — 完整卸载本机的 Claude Code(原生安装器版）及其全部「全局安装痕迹」。
#
# 覆盖范围(逐条为本机实测确认,均属全局安装痕迹):
#   本体      ~/.local/{share,state}/claude、软链 ~/.local/bin/claude
#   数据      ${CLAUDE_CONFIG_DIR:-~/.claude}、配置 ~/.claude.json(+ *.bak)
#   旧/XDG    ~/.claude-code、~/.config/claude*、~/.cache/claude*(本机通常无,存在才删)
#   桌面版    com.anthropic.claudefordesktop 整套 bundle:~/Library 下 Application Support/Claude、
#             Caches、HTTPStorages、Preferences plist、Saved Application State、WebKit、Cookies、Logs/Claude
#   CLI 缓存  ~/Library/Caches/claude-cli-nodejs
#   临时目录  /private/tmp/claude-<uid>
#   URL 处理器 ~/Applications/Claude Code URL Handler.app  + 重置 LaunchServices
#   JetBrains  …/IntelliJIdea*/plugins/claude-code-jetbrains-plugin(+ 任务残留)
#   Keychain   Claude Code-credentials(acct=当前用户)、Claude Safe Storage(acct=Claude Key)
#   shell 别名 ~/.zshrc / ~/.zprofile 里的 alias claude=…
#   IDE 设置   VS Code / Antigravity settings.json 里的 claudeCode 孤儿项
#   浏览器桥接 各浏览器 NativeMessagingHosts/com.anthropic.claude*_browser_extension.json(原生消息宿主 manifest)
#
# 明确不动(边界):
#   ~/.local/bin 目录、PATH、~/.local/bin/env(uv 等工具共用)
#   项目内 .claude/ 与 CLAUDE.md(各仓库内容)
#   第三方 session-viewer;用户自建 com.xiaoyu.skills.update 定时任务(末尾仅提示)
#   'Claude for Chrome' 浏览器扩展(经浏览器管理页卸载,强删目录会损坏 profile)、
#     claude.ai/claude.com 网站数据(cookie/IndexedDB/登录态,属正常浏览内容)——末尾仅提示
#
# 用法:
#   ./scripts/claude/uninstall.sh --dry-run                 预览计划,零改动
#   ./scripts/claude/uninstall.sh                           同预览(未加 --yes 不会真删)
#   ./scripts/claude/uninstall.sh --yes                     真正执行卸载
#   ./scripts/claude/uninstall.sh --yes --backup-memory     删前先把各项目 memory 备份到 ~/Desktop
# 经 manage.sh:
#   ./manage.sh claude:uninstall --dry-run | ./manage.sh claude:uninstall --yes
#
# 安全保证:
#   - 默认「预览」:不加 --yes 只打印计划、零改动(适配非交互的 supervised 运行)
#   - 只删「全局安装痕迹」,不碰共享目录与项目内文件
#   - 删 shell 别名 / IDE 设置前把原文件备份为 *.claude-bak
#
# 注:此处用 set -uo(不含 -e)——pkill/security/grep 返回非零是正常控制流,
#     不应中断整条卸载序列。
set -uo pipefail

DRY_RUN=false
ASSUME_YES=false
BACKUP_MEMORY=false
for arg in "$@"; do
    case "$arg" in
        --dry-run)       DRY_RUN=true ;;
        --yes|-y)        ASSUME_YES=true ;;
        --backup-memory) BACKUP_MEMORY=true ;;
        -h|--help)       sed -n '2,32p' "$0"; exit 0 ;;
        *) echo "未知参数: $arg(可用: --dry-run --yes --backup-memory)" >&2; exit 2 ;;
    esac
done

# 只有「明确 --yes 且非 --dry-run」才真正执行;否则一律预览。
EXECUTE=false
if [ "$ASSUME_YES" = true ] && [ "$DRY_RUN" = false ]; then EXECUTE=true; fi

# 尊重 CLAUDE_CONFIG_DIR:用户可能把配置目录从默认 ~/.claude 迁走,硬编码会漏删。
CFG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"

step() { echo; echo "$1"; }

# 删除若干路径(glob 由调用处展开);仅对真实存在的目标动手/打印。
do_rm() {
    local p
    for p in "$@"; do
        if [ -e "$p" ] || [ -L "$p" ]; then
            if [ "$EXECUTE" = true ]; then rm -rf "$p" && echo "   已删 $p"
            else echo "   [预览] 删除 $p"; fi
        fi
    done
}

echo "==================================================================="
if [ "$EXECUTE" = true ]; then
    echo " 正在【彻底卸载 Claude Code】并删除全部全局数据、配置、缓存、凭证。"
else
    echo " 预览模式:以下为将删除的全局痕迹,当前【不会改动任何文件】。"
fi
echo "==================================================================="

# ①* 可选:删除 ~/.claude 前先备份各项目 memory
if [ "$BACKUP_MEMORY" = true ]; then
    step "①* 备份各项目 memory 到 ~/Desktop"
    PROJ="$CFG_DIR/projects"
    if [ -d "$PROJ" ]; then
        mems=()
        while IFS= read -r d; do mems+=("${d#"$PROJ"/}"); done \
            < <(find "$PROJ" -maxdepth 2 -type d -name memory 2>/dev/null)
        if [ "${#mems[@]}" -gt 0 ]; then
            OUT="$HOME/Desktop/claude-memory-backup.tgz"
            if [ "$EXECUTE" = true ]; then
                tar -czf "$OUT" -C "$PROJ" "${mems[@]}" && echo "   已备份 ${#mems[@]} 个 memory 目录 -> $OUT"
            else
                echo "   [预览] 打包 ${#mems[@]} 个 memory 目录 -> $OUT"
            fi
        else
            echo "   未发现 memory 目录,跳过"
        fi
    else
        echo "   $PROJ 不存在,跳过"
    fi
fi

step "② 结束正在运行的 claude 进程"
if [ "$EXECUTE" = true ]; then
    pkill -f "\.local/share/claude/versions" 2>/dev/null || true
    pkill -x claude 2>/dev/null || true
    echo "   done"
else
    echo "   [预览] pkill 运行中的 claude 进程"
fi

step "③ 删除本体 / 数据 / 配置 / 缓存 / 临时目录 / URL 处理器"
do_rm "$HOME/.local/share/claude" "$HOME/.local/state/claude" "$HOME/.local/bin/claude"
do_rm "$CFG_DIR" "$HOME/.claude.json"
do_rm "$HOME"/.claude.json.bak.*
do_rm "$HOME/Library/Caches/claude-cli-nodejs"
do_rm "/private/tmp/claude-$(id -u)"
do_rm "$HOME/Applications/Claude Code URL Handler.app"
# 旧版 / XDG 残留(本机通常无,do_rm 存在才删)。
do_rm "$HOME/.claude-code"
do_rm "$HOME/.config/claude"*
do_rm "$HOME/.cache/claude"*
# Claude 桌面版(com.anthropic.claudefordesktop)整套 bundle 痕迹:按明确要求整目录清理。
# 注意——含桌面版应用数据(聊天/会话等);本机未装桌面版,通常仅残留少数缓存,do_rm 存在才删。
do_rm "$HOME/Library/Application Support/Claude"
do_rm "$HOME/Library/Caches/com.anthropic.claudefordesktop"
do_rm "$HOME/Library/HTTPStorages/com.anthropic.claudefordesktop"
do_rm "$HOME/Library/Preferences/com.anthropic.claudefordesktop.plist"
do_rm "$HOME/Library/Saved Application State/com.anthropic.claudefordesktop.savedState"
do_rm "$HOME/Library/WebKit/com.anthropic.claudefordesktop"
do_rm "$HOME/Library/Cookies/com.anthropic.claudefordesktop.binarycookies"
do_rm "$HOME/Library/Logs/Claude"
LSREG="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [ "$EXECUTE" = true ]; then
    [ -x "$LSREG" ] && "$LSREG" -kill -r -domain local -domain system -domain user >/dev/null 2>&1
    echo "   已重置 LaunchServices(清 claude:// 与 .mcpb/.dxt/.skill 绑定)"
else
    echo "   [预览] 重置 LaunchServices(清 claude:// 绑定)"
fi

step "④ 删除 JetBrains 插件与任务残留"
do_rm "$HOME/Library/Application Support/JetBrains/IntelliJIdea"*"/plugins/claude-code-jetbrains-plugin"
do_rm "$HOME/Library/Application Support/JetBrains/IntelliJIdea"*"/tasks/claude_skill_performance_troubleshoot."*
echo "   (更稳妥:也可在 IDEA → Settings → Plugins 卸载 'Claude Code')"

step "⑤ 删除 Keychain 凭证"
kc_del() {  # $1=service $2=account
    if [ "$EXECUTE" = true ]; then
        if security delete-generic-password -s "$1" -a "$2" >/dev/null 2>&1; then echo "   已删 $1"
        else echo "   $1 未找到(可能已删)"; fi
    else
        if security find-generic-password -s "$1" -a "$2" >/dev/null 2>&1; then echo "   [预览] 删除 Keychain: $1(acct=$2)"
        else echo "   [预览] Keychain $1 不存在,跳过"; fi
    fi
}
kc_del "Claude Code-credentials" "$(whoami)"
kc_del "Claude Safe Storage" "Claude Key"

step "⑥ 移除 shell 别名(改动前备份为 *.claude-bak)"
for rc in "$HOME/.zshrc" "$HOME/.zprofile"; do
    if [ -f "$rc" ] && grep -q 'alias claude=' "$rc"; then
        if [ "$EXECUTE" = true ]; then sed -i.claude-bak '/alias claude=/d' "$rc" && echo "   已处理 $rc(备份 $rc.claude-bak)"
        else echo "   [预览] 从 $rc 删除 alias claude= 行"; fi
    fi
done
echo "   注意:未改动 PATH / .local/bin/env 等共享配置。"

step "⑦ 清理 IDE 里残留的 claudeCode 设置(改动前备份)"
for s in "$HOME/Library/Application Support/Code/User/settings.json" \
         "$HOME/Library/Application Support/Antigravity/User/settings.json"; do
    if [ -f "$s" ] && grep -q 'claudeCode' "$s"; then
        if [ "$EXECUTE" = true ]; then sed -i.claude-bak '/claudeCode/d' "$s" && echo "   已清 $s(备份 $s.claude-bak)"
        else echo "   [预览] 从 $s 删除 claudeCode 设置行"; fi
    fi
done

step "⑧ 清理浏览器原生消息宿主 manifest(Claude 浏览器扩展的 native messaging 桥接)"
# 仅删 Anthropic 自有文件名的指针 JSON——安全(非 HMAC 签名,删掉不会损坏浏览器 profile);
# 任意深度扫(如 Arc 在 …/Arc/User Data/NativeMessagingHosts/)。浏览器扩展本体不强删,见末尾。
nmh_found=false
while IFS= read -r nmh; do
    nmh_found=true; do_rm "$nmh"
done < <(find "$HOME/Library/Application Support" -type f \
    \( -name 'com.anthropic.claude_browser_extension.json' \
       -o -name 'com.anthropic.claude_code_browser_extension.json' \) 2>/dev/null)
[ "$nmh_found" = false ] && echo "   未发现 Claude 原生消息宿主 manifest(本机未用过浏览器连接、未装桌面版)"

echo
echo "-------------------------------------------------------------------"
echo "【需你手动决定,脚本不动】"
echo "  • ~/Library/LaunchAgents/com.xiaoyu.skills.update.plist"
echo "    你自建定时任务,会往 ~/.claude/skills 写。弃用 Claude 后建议:"
echo "      launchctl unload ~/Library/LaunchAgents/com.xiaoyu.skills.update.plist && \\"
echo "      rm ~/Library/LaunchAgents/com.xiaoyu.skills.update.plist"
echo "  • 项目内 .claude/ 与 CLAUDE.md、第三方 session-viewer —— 均保留"
echo "  • ~/.local/bin、PATH、.local/bin/env —— uv 等共用,保留"
echo "  • 'Claude for Chrome' 浏览器扩展(若装过):请在浏览器扩展管理页卸载。"
echo "    脚本不强删——Chrome 扩展状态 HMAC 签在 Secure Preferences,直接删目录会"
echo "    留悬挂项/'扩展已损坏',且可能被账号同步重装。(已扫本机 9 款浏览器×全 profile,当前未装)"
echo "  • claude.ai / claude.com 的网站数据(cookie、IndexedDB、登录态)—— 属正常浏览内容,非安装痕迹,保留"
echo "-------------------------------------------------------------------"

echo
if [ "$EXECUTE" = true ]; then
    echo "✅ 卸载完成。请【重开终端】后执行  which claude  验证(应无输出)。"
    echo "   确认无误后可删备份: rm -f ~/.zshrc.claude-bak ~/.zprofile.claude-bak"
elif [ "$DRY_RUN" = true ]; then
    echo "🔎 空转预览结束(--dry-run),未改动任何文件。"
else
    echo "🔎 以上为预览,未删除任何内容。确认无误后加 --yes 真正执行:"
    echo "     ./manage.sh claude:uninstall --yes"
fi
