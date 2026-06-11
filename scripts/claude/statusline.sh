#!/bin/bash
# Claude Code status line：model · effort · 目录 · git分支 · 上下文 · 行数增删 · 成本 · 5h/7d 用量+重置倒计时 · 输出风格。
# 主体纯 bash 内建（read / [[ =~ ]] / printf / 参数展开），无 jq/git、无网络；仅在 5h/7d resets_at 存在时
# fork 一次 date +%s 取当前时刻（两窗口共用，纯 bash 算倒计时），其余零 fork，约 6-9ms。
# 数据全部来自 Claude Code 注入 stdin 的 JSON + 本地 .git/HEAD；任一字段缺失时对应段自动省略。
# 兼容 macOS /bin/bash 3.2（不使用 4.x 语法）。由 scripts/claude/install_statusline.py 部署到 ~/.claude/。

IFS= read -rd '' I   # 读全部 stdin（bash 内建）

# ---- 解析字段（bash 正则；[^}]* 不跨嵌套花括号） ----
model=""; effort=""; fast=""; cwd=""; ctx=""; added=""; removed=""; cost=""; five=""; week=""; ostyle=""
five_reset=""; week_reset=""   # rate_limits.*.resets_at（Unix epoch 秒；用量重置时刻）

re_model='"display_name"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_effort='"effort"[[:space:]]*:[[:space:]]*\{[^}]*"level"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_fast='"fast_mode"[[:space:]]*:[[:space:]]*true'
re_cwd='"cwd"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_ctx='"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)[^}]*"remaining_percentage"'  # 仅 context_window 段后接 remaining_percentage
re_added='"total_lines_added"[[:space:]]*:[[:space:]]*([0-9]+)'
re_removed='"total_lines_removed"[[:space:]]*:[[:space:]]*([0-9]+)'
re_cost='"total_cost_usd"[[:space:]]*:[[:space:]]*([0-9.eE+-]+)'
re_five='"five_hour"[[:space:]]*:[[:space:]]*\{[^}]*"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)'
re_week='"seven_day"[[:space:]]*:[[:space:]]*\{[^}]*"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)'
re_five_reset='"five_hour"[[:space:]]*:[[:space:]]*\{[^}]*"resets_at"[[:space:]]*:[[:space:]]*([0-9]+)'
re_week_reset='"seven_day"[[:space:]]*:[[:space:]]*\{[^}]*"resets_at"[[:space:]]*:[[:space:]]*([0-9]+)'
re_ostyle='"output_style"[[:space:]]*:[[:space:]]*\{[^}]*"name"[[:space:]]*:[[:space:]]*"([^"]+)"'

[[ $I =~ $re_model   ]] && model=${BASH_REMATCH[1]}
[[ $I =~ $re_effort  ]] && effort=${BASH_REMATCH[1]}
[[ $I =~ $re_fast    ]] && fast=1
[[ $I =~ $re_cwd     ]] && cwd=${BASH_REMATCH[1]}
[[ $I =~ $re_ctx     ]] && ctx=${BASH_REMATCH[1]}
[[ $I =~ $re_added   ]] && added=${BASH_REMATCH[1]}
[[ $I =~ $re_removed ]] && removed=${BASH_REMATCH[1]}
[[ $I =~ $re_cost    ]] && cost=${BASH_REMATCH[1]}
[[ $I =~ $re_five    ]] && five=${BASH_REMATCH[1]}
[[ $I =~ $re_week    ]] && week=${BASH_REMATCH[1]}
[[ $I =~ $re_five_reset ]] && five_reset=${BASH_REMATCH[1]}
[[ $I =~ $re_week_reset ]] && week_reset=${BASH_REMATCH[1]}
[[ $I =~ $re_ostyle  ]] && ostyle=${BASH_REMATCH[1]}

# 精简模型名：去掉 "Claude " 前缀，"(1M context)" → "(1M)"
model=${model#Claude }
model=${model/ (1M context)/ (1M)}

# 目录末段
dir=${cwd##*/}

# git 分支：从 cwd 向上找 .git/HEAD，纯读文件（不 fork git）。.git 为文件(worktree)时跳过。
branch=""
d=$cwd
while [[ -n $d ]]; do
  if [[ -r $d/.git/HEAD ]]; then
    read -r head < "$d/.git/HEAD"
    if [[ $head == ref:* ]]; then branch=${head#ref: refs/heads/}; else branch=${head:0:7}; fi
    break
  fi
  d=${d%/*}
done

# ---- 输出（printf 内建；仅当有 resets_at 时 fork 一次 date +%s 取 now） ----
shown=0
sep() { (( shown )) && printf ' · '; shown=1; }   # 分隔符用终端默认前景色（深/浅背景皆可读）

# epoch 差（秒）-> 紧凑倒计时，结果写入 REPLY（纯 bash，无 $() 子shell）
fmt_dur() {
  local s=$1 d h m
  (( s < 0 )) && s=0
  d=$(( s/86400 )); h=$(( (s%86400)/3600 )); m=$(( (s%3600)/60 ))
  if   (( d > 0 )); then printf -v REPLY '%dd%dh' "$d" "$h"
  elif (( h > 0 )); then printf -v REPLY '%dh%dm' "$h" "$m"
  else                   printf -v REPLY '%dm' "$m"; fi
}

# 倒计时需当前时刻：仅当任一 resets_at 存在时 fork 一次 date +%s（两窗口共用；CC_STATUSLINE_NOW 供测试注入）
now=""
if [[ -n $five_reset || -n $week_reset ]]; then now=${CC_STATUSLINE_NOW:-$(date +%s 2>/dev/null)}; fi

if [[ -n $model ]]; then
  sep; printf '\033[1;36m%s\033[0m' "$model"
  (( fast )) && printf ' \033[33m⚡fast\033[0m'
fi

[[ -n $effort ]] && { sep; printf '\033[35meffort:%s\033[0m' "$effort"; }

if [[ -n $dir ]]; then                       # 目录（加粗，背景无关）+ 分支（绿，括号内）
  sep; printf '\033[1m%s\033[0m' "$dir"
  [[ -n $branch ]] && printf ' \033[32m(%s)\033[0m' "$branch"
fi

if [[ -n $ctx ]]; then                       # 上下文占用：>=85 红 >=60 黄 否则绿
  c=32; (( ctx >= 60 )) && c=33; (( ctx >= 85 )) && c=31
  sep; printf '\033[%dmctx %s%%\033[0m' "$c" "$ctx"
fi

if (( ${added:-0} > 0 || ${removed:-0} > 0 )); then   # 本会话行数增删
  sep; printf '\033[32m+%s\033[0m/\033[31m-%s\033[0m' "${added:-0}" "${removed:-0}"
fi

[[ -n $cost ]] && { sep; printf '$%.2f' "$cost"; }   # 成本用默认前景色

if [[ -n $five ]]; then                      # 5 小时时限用量（+ 重置倒计时）
  c=32; (( five >= 60 )) && c=33; (( five >= 85 )) && c=31
  sep; printf '\033[%dm5h %s%%\033[0m' "$c" "$five"
  if [[ -n $five_reset && -n $now ]]; then fmt_dur $(( five_reset - now )); printf ' ↻%s' "$REPLY"; fi
fi

if [[ -n $week ]]; then                      # 7 天周限用量（+ 重置倒计时）
  c=32; (( week >= 60 )) && c=33; (( week >= 85 )) && c=31
  sep; printf '\033[%dm7d %s%%\033[0m' "$c" "$week"
  if [[ -n $week_reset && -n $now ]]; then fmt_dur $(( week_reset - now )); printf ' ↻%s' "$REPLY"; fi
fi

# 输出风格：仅非 default 时提示（罕见，平时不占位）
[[ -n $ostyle && $ostyle != default ]] && { sep; printf '[%s]' "$ostyle"; }

exit 0   # statusline 始终以 0 退出（即使本行之前的 [[ ]] 为假）
