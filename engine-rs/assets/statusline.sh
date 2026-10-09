#!/bin/bash
# Agent Manager 内置状态栏。
#
# 显示：模型 effort · 目录(分支) · 上下文占用 · 本会话增删行 · 花费 · 5h/7d 额度与重置倒计时。
# 顺带把额度存一份到 Agent Manager 数据目录（即本脚本所在目录）下的
# usage/<编码后的配置目录>.json：Claude Code 每次渲染状态栏都把 rate_limits 通过 stdin
# 推给本脚本，但它自己不写任何文件，外部进程读不到；存下来 Agent Manager 才能显示与
# 这一行同屏的实时额度。不往助手的配置目录里放任何文件。
# 配置目录编码：先 % -> %25，再 / -> %2F，不同目录不会撞名；Agent Manager 按同一规则读取。
#
# 主体纯 bash 内建（read / [[ =~ ]] / printf / 参数展开），无 jq、无 git、无网络；
# 仅在有重置时刻时 fork 一次 date、落盘时 fork 一次 mv，usage 目录还不存在时再
# fork 一次 mkdir。兼容 macOS 自带的 bash 3.2。
# 任一字段缺失时对应段自动省略。本文件由 Agent Manager 部署，手改会在下次应用时被覆盖。

IFS= read -rd '' I   # 读全部 stdin

model=""; effort=""; cwd=""; transcript=""; ctx=""; added=""; removed=""; cost=""; five=""; week=""
five_reset=""; week_reset=""

re_model='"display_name"[[:space:]]*:[[:space:]]*"([^"]+)"'
# effort 只在当前模型支持时出现：{"effort":{"level":"xhigh"}}
re_effort='"effort"[[:space:]]*:[[:space:]]*\{[^}]*"level"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_cwd='"cwd"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_transcript='"transcript_path"[[:space:]]*:[[:space:]]*"([^"]+)"'
# 仅 context_window 段的 used_percentage 后面跟着 remaining_percentage，据此与额度区分
re_ctx='"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)[^}]*"remaining_percentage"'
re_added='"total_lines_added"[[:space:]]*:[[:space:]]*([0-9]+)'
re_removed='"total_lines_removed"[[:space:]]*:[[:space:]]*([0-9]+)'
re_cost='"total_cost_usd"[[:space:]]*:[[:space:]]*([0-9.eE+-]+)'
re_five='"five_hour"[[:space:]]*:[[:space:]]*\{[^}]*"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)'
re_week='"seven_day"[[:space:]]*:[[:space:]]*\{[^}]*"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)'
re_five_reset='"five_hour"[[:space:]]*:[[:space:]]*\{[^}]*"resets_at"[[:space:]]*:[[:space:]]*([0-9]+)'
re_week_reset='"seven_day"[[:space:]]*:[[:space:]]*\{[^}]*"resets_at"[[:space:]]*:[[:space:]]*([0-9]+)'

[[ $I =~ $re_model      ]] && model=${BASH_REMATCH[1]}
[[ $I =~ $re_effort     ]] && effort=${BASH_REMATCH[1]}
[[ $I =~ $re_cwd        ]] && cwd=${BASH_REMATCH[1]}
[[ $I =~ $re_transcript ]] && transcript=${BASH_REMATCH[1]}
[[ $I =~ $re_ctx        ]] && ctx=${BASH_REMATCH[1]}
[[ $I =~ $re_added      ]] && added=${BASH_REMATCH[1]}
[[ $I =~ $re_removed    ]] && removed=${BASH_REMATCH[1]}
[[ $I =~ $re_cost       ]] && cost=${BASH_REMATCH[1]}
[[ $I =~ $re_five       ]] && five=${BASH_REMATCH[1]}
[[ $I =~ $re_week       ]] && week=${BASH_REMATCH[1]}
[[ $I =~ $re_five_reset ]] && five_reset=${BASH_REMATCH[1]}
[[ $I =~ $re_week_reset ]] && week_reset=${BASH_REMATCH[1]}

model=${model#Claude }
model=${model/ (1M context)/ (1M)}
dir=${cwd##*/}

# git 分支：从 cwd 向上找，纯读文件不 fork。普通检出的 .git 是目录，worktree 的是文件。
branch=""
d=$cwd
while [[ -n $d ]]; do
  if [[ -r $d/.git/HEAD ]]; then
    read -r head < "$d/.git/HEAD"
    if [[ $head == ref:* ]]; then branch=${head#ref: refs/heads/}; else branch=${head:0:7}; fi
    break
  elif [[ -f $d/.git ]]; then
    read -r gitdir < "$d/.git"
    gitdir=${gitdir#gitdir: }
    [[ $gitdir != /* ]] && gitdir=$d/$gitdir
    if [[ -r $gitdir/HEAD ]]; then
      read -r head < "$gitdir/HEAD"
      if [[ $head == ref:* ]]; then branch=${head#ref: refs/heads/}; else branch=${head:0:7}; fi
    fi
    break
  fi
  d=${d%/*}
done

# 倒计时要当前时刻：仅当存在重置时刻时 fork 一次 date（AM_STATUSLINE_NOW 供测试注入）
now=""
if [[ -n $five_reset || -n $week_reset ]]; then now=${AM_STATUSLINE_NOW:-$(date +%s 2>/dev/null)}; fi

# ---- 额度落盘 ----
# 写临时文件再 mv 覆盖：多个窗口并发也不会让 Agent Manager 读到半截内容。
# 文件名跟随当前会话的配置目录，实例各存各的。
if [[ -n $five || -n $week ]]; then
  am_dir=$CLAUDE_CONFIG_DIR
  # 开了 CLAUDE_CODE_SUBPROCESS_ENV_SCRUB 时子进程拿不到 CLAUDE_CONFIG_DIR：会话记录在
  # <配置目录>/projects/ 下，从 transcript_path 反推，实例的额度仍记在它自己名下
  if [[ -z $am_dir && $transcript == */projects/* ]]; then am_dir=${transcript%/projects/*}; fi
  [[ -z $am_dir ]] && am_dir=$HOME/.claude
  # 启动命令写成 CLAUDE_CONFIG_DIR="~/..." 时引号里的 ~ 不会被 shell 展开，这里补上，
  # 与 Agent Manager 读取时展开后的路径对得上
  [[ $am_dir == "~" ]] && am_dir=$HOME
  [[ $am_dir == "~/"* ]] && am_dir=$HOME/${am_dir#"~/"}
  # Windows（Git Bash）：/c/Users/... 与 C:\Users\... 统一成 c:/users/...，与 Agent Manager 一致
  am_win=
  if [[ $am_dir =~ ^/([a-zA-Z])/(.*)$ ]]; then
    case $(uname -s 2>/dev/null) in MINGW*|MSYS*|CYGWIN*) am_dir="${BASH_REMATCH[1]}:/${BASH_REMATCH[2]}" ;; esac
  fi
  if [[ $am_dir =~ ^[a-zA-Z]: ]]; then
    am_win=1
    am_dir=${am_dir//\\//}
    am_dir=${am_dir,,}
  fi
  am_pct='%'
  am_key=${am_dir//$am_pct/%25}
  am_key=${am_key//\//%2F}
  [[ -n $am_win ]] && am_key=${am_key//:/%3A}
  am_self=${BASH_SOURCE[0]}
  am_self=${am_self//\\//}
  if [[ $am_self == */* ]]; then am_usage=${am_self%/*}/usage; else am_usage=usage; fi
  [[ -d $am_usage ]] || mkdir -p "$am_usage" 2>/dev/null
  am_file=$am_usage/$am_key.json
  am_tmp=$am_file.$$
  {
    printf '{"at":%s' "${now:-0}"
    [[ -n $five ]] && printf ',"five_hour":{"used_percentage":%s,"resets_at":%s}' "$five" "${five_reset:-null}"
    [[ -n $week ]] && printf ',"seven_day":{"used_percentage":%s,"resets_at":%s}' "$week" "${week_reset:-null}"
    printf '}\n'
  } > "$am_tmp" 2>/dev/null && mv -f "$am_tmp" "$am_file" 2>/dev/null || rm -f "$am_tmp" 2>/dev/null
fi

# ---- 输出 ----
shown=0
sep() { (( shown )) && printf ' · '; shown=1; }

fmt_dur() {   # 秒 -> 紧凑倒计时，结果写入 REPLY（不开子 shell）
  local s=$1 dd hh mm
  (( s < 0 )) && s=0
  dd=$(( s/86400 )); hh=$(( (s%86400)/3600 )); mm=$(( (s%3600)/60 ))
  if   (( dd > 0 )); then printf -v REPLY '%dd%dh' "$dd" "$hh"
  elif (( hh > 0 )); then printf -v REPLY '%dh%dm' "$hh" "$mm"
  else                    printf -v REPLY '%dm' "$mm"; fi
}

if [[ -n $model ]]; then
  sep; printf '\033[1;36m%s\033[0m' "$model"
  [[ -n $effort ]] && printf ' \033[36m%s\033[0m' "$effort"
fi

if [[ -n $dir ]]; then
  sep; printf '\033[1m%s\033[0m' "$dir"
  [[ -n $branch ]] && printf ' \033[32m(%s)\033[0m' "$branch"
fi

if [[ -n $ctx ]]; then                       # 上下文占用：>=85 红 >=60 黄 否则绿
  c=32; (( ctx >= 60 )) && c=33; (( ctx >= 85 )) && c=31
  sep; printf '\033[%dmctx %s%%\033[0m' "$c" "$ctx"
fi

if (( ${added:-0} > 0 || ${removed:-0} > 0 )); then
  sep; printf '\033[32m+%s\033[0m/\033[31m-%s\033[0m' "${added:-0}" "${removed:-0}"
fi

[[ -n $cost ]] && { sep; printf '$%.2f' "$cost"; }

if [[ -n $five ]]; then
  c=32; (( five >= 60 )) && c=33; (( five >= 85 )) && c=31
  sep; printf '\033[%dm5h %s%%\033[0m' "$c" "$five"
  if [[ -n $five_reset && -n $now ]]; then fmt_dur $(( five_reset - now )); printf ' ↻%s' "$REPLY"; fi
fi

if [[ -n $week ]]; then
  c=32; (( week >= 60 )) && c=33; (( week >= 85 )) && c=31
  sep; printf '\033[%dm7d %s%%\033[0m' "$c" "$week"
  if [[ -n $week_reset && -n $now ]]; then fmt_dur $(( week_reset - now )); printf ' ↻%s' "$REPLY"; fi
fi

exit 0   # 状态栏脚本始终以 0 退出
