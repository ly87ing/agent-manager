#!/bin/bash
# Claude Code status line: model · effort · directory · git branch · context · line diff · cost · 5h/7d usage + reset countdown · output style.
# The body is pure bash builtins (read / [[ =~ ]] / printf / parameter expansion), no jq/git, no network; only when 5h/7d resets_at is present does it
# fork `date +%s` once to get the current time (shared by both windows, countdown computed in pure bash); zero forks otherwise, about 6-9ms.
# All data comes from the JSON Claude Code injects on stdin + local git HEAD; any missing field causes its segment to be omitted automatically.
# Compatible with macOS /bin/bash 3.2 (avoids 4.x syntax). Deployed to ~/.claude/ by scripts/claude/install_statusline.py.

IFS= read -rd '' I   # Read all of stdin (bash builtin)

# ---- Parse fields (bash regex; [^}]* does not cross nested braces) ----
model=""; effort=""; fast=""; cwd=""; ctx=""; added=""; removed=""; cost=""; five=""; week=""; ostyle=""
five_reset=""; week_reset=""   # rate_limits.*.resets_at (Unix epoch seconds; usage reset time)

re_model='"display_name"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_effort='"effort"[[:space:]]*:[[:space:]]*\{[^}]*"level"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_fast='"fast_mode"[[:space:]]*:[[:space:]]*true'
re_cwd='"cwd"[[:space:]]*:[[:space:]]*"([^"]+)"'
re_ctx='"used_percentage"[[:space:]]*:[[:space:]]*([0-9]+)[^}]*"remaining_percentage"'  # only the context_window segment is followed by remaining_percentage
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

# Shorten the model name: drop the "Claude " prefix, "(1M context)" → "(1M)"
model=${model#Claude }
model=${model/ (1M context)/ (1M)}

# Last path segment of the directory
dir=${cwd##*/}

# git branch: walk up from cwd to find HEAD, reading the file directly (no git fork).
# Normal checkouts keep HEAD at .git/HEAD; worktrees keep .git as a "gitdir: <path>" file.
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

# ---- Output (printf builtin; fork `date +%s` once for now only when resets_at is present) ----
shown=0
sep() { (( shown )) && printf ' · '; shown=1; }   # the separator uses the terminal's default foreground color (readable on dark/light backgrounds)

# epoch difference (seconds) -> compact countdown, result written to REPLY (pure bash, no $() subshell)
fmt_dur() {
  local s=$1 d h m
  (( s < 0 )) && s=0
  d=$(( s/86400 )); h=$(( (s%86400)/3600 )); m=$(( (s%3600)/60 ))
  if   (( d > 0 )); then printf -v REPLY '%dd%dh' "$d" "$h"
  elif (( h > 0 )); then printf -v REPLY '%dh%dm' "$h" "$m"
  else                   printf -v REPLY '%dm' "$m"; fi
}

# The countdown needs the current time: fork `date +%s` once only when either resets_at is present (shared by both windows; CC_STATUSLINE_NOW lets tests inject it)
now=""
if [[ -n $five_reset || -n $week_reset ]]; then now=${CC_STATUSLINE_NOW:-$(date +%s 2>/dev/null)}; fi

if [[ -n $model ]]; then
  sep; printf '\033[1;36m%s\033[0m' "$model"
  (( fast )) && printf ' \033[33m⚡fast\033[0m'
fi

[[ -n $effort ]] && { sep; printf '\033[35meffort:%s\033[0m' "$effort"; }

if [[ -n $dir ]]; then                       # directory (bold, background-agnostic) + branch (green, in parentheses)
  sep; printf '\033[1m%s\033[0m' "$dir"
  [[ -n $branch ]] && printf ' \033[32m(%s)\033[0m' "$branch"
fi

if [[ -n $ctx ]]; then                       # context usage: >=85 red, >=60 yellow, otherwise green
  c=32; (( ctx >= 60 )) && c=33; (( ctx >= 85 )) && c=31
  sep; printf '\033[%dmctx %s%%\033[0m' "$c" "$ctx"
fi

if (( ${added:-0} > 0 || ${removed:-0} > 0 )); then   # lines added/removed this session
  sep; printf '\033[32m+%s\033[0m/\033[31m-%s\033[0m' "${added:-0}" "${removed:-0}"
fi

[[ -n $cost ]] && { sep; printf '$%.2f' "$cost"; }   # cost uses the default foreground color

if [[ -n $five ]]; then                      # 5-hour limit usage (+ reset countdown)
  c=32; (( five >= 60 )) && c=33; (( five >= 85 )) && c=31
  sep; printf '\033[%dm5h %s%%\033[0m' "$c" "$five"
  if [[ -n $five_reset && -n $now ]]; then fmt_dur $(( five_reset - now )); printf ' ↻%s' "$REPLY"; fi
fi

if [[ -n $week ]]; then                      # 7-day weekly limit usage (+ reset countdown)
  c=32; (( week >= 60 )) && c=33; (( week >= 85 )) && c=31
  sep; printf '\033[%dm7d %s%%\033[0m' "$c" "$week"
  if [[ -n $week_reset && -n $now ]]; then fmt_dur $(( week_reset - now )); printf ' ↻%s' "$REPLY"; fi
fi

# Output style: show only when not default (rare, takes no space normally)
[[ -n $ostyle && $ostyle != default ]] && { sep; printf '[%s]' "$ostyle"; }

exit 0   # the statusline always exits with 0 (even if the [[ ]] on the previous line was false)
