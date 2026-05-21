#!/usr/bin/env bash
# Claude Code status line. Reads session JSON on stdin, writes one line.
#
# Zero network, zero transcript IO. All data comes from stdin; one local
# `git` invocation when the workspace is a checkout. Output shape:
#
#   model | effort | output-style | dir ⎇ branch (+a,-b) | Ctx X% bar | $cost | clock

set -u

input=$(cat)

# ASCII Unit Separator (\x1f) keeps empty fields visible when bash `read`
# splits — bash collapses runs of whitespace IFS chars, which would silently
# drop columns whose source was missing in stdin.
US=$'\x1f'

# Resolve fields in a single jq pass.
fields=$(printf '%s' "$input" | jq -rj --arg sep "$US" '
  [
    (.model.display_name // .model.id // "claude"),
    (.effort // ""),
    (.output_style.name // ""),
    (.workspace.current_dir // .cwd // "."),
    (.cost.total_cost_usd // ""),
    (.cost.total_duration_ms // ""),
    ((.context_window.current_usage
        | if type == "object"
          then (.input_tokens // 0)
               + (.cache_read_input_tokens // 0)
               + (.cache_creation_input_tokens // 0)
          else . end) // ""),
    (.context_window.context_window_size // "")
  ] | map(tostring) | join($sep)
' 2>/dev/null)

IFS="$US" read -r model effort style cwd cost_usd dur_ms ctx_used ctx_size <<<"$fields"

dir=$(basename "$cwd")

branch=""
changes=""
if git -C "$cwd" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    branch=$(git -C "$cwd" branch --show-current 2>/dev/null)
    if status=$(git -C "$cwd" status --porcelain 2>/dev/null); then
        adds=$(printf '%s\n' "$status" | grep -cE '^( M|M |A |\?\?)' || true)
        dels=$(printf '%s\n' "$status" | grep -cE '^( D|D )' || true)
        if [ "$adds" -gt 0 ] || [ "$dels" -gt 0 ]; then
            changes=" (+${adds},-${dels})"
        fi
    fi
fi

# ANSI helpers (256-color). Skip if NO_COLOR=1.
if [ "${NO_COLOR:-}" = "1" ]; then
    c() { printf '%s' "$2"; }
else
    c() { printf '\033[%sm%s\033[0m' "$1" "$2"; }
fi

parts=()
parts+=("$(c '38;5;30'  "$model")")
[ -n "$effort" ] && parts+=("$(c '38;5;140' "$effort")")
[ -n "$style" ]  && parts+=("$(c '38;5;80'  "$style")")

dirpart="$(c '38;5;33' "$dir")"
if [ -n "$branch" ]; then
    dirpart="${dirpart} $(c '38;5;80' "⎇ ${branch}")"
fi
if [ -n "$changes" ]; then
    dirpart="${dirpart}$(c '38;5;220' "${changes}")"
fi
parts+=("$dirpart")

# Context usable% (0-80% raw → 0-100% usable). Pure-bash arithmetic; no IO.
if [ -n "${ctx_used:-}" ] && [ -n "${ctx_size:-}" ] && [ "$ctx_size" -gt 0 ] 2>/dev/null; then
    raw_pct=$(( ctx_used * 100 / ctx_size ))
    usable=$(( raw_pct * 100 / 80 ))
    [ "$usable" -gt 100 ] && usable=100
    [ "$usable" -lt 0 ] && usable=0
    filled=$(( usable / 10 ))
    [ "$filled" -gt 10 ] && filled=10
    bar=""
    i=0
    while [ "$i" -lt 10 ]; do
        if [ "$i" -lt "$filled" ]; then bar="${bar}▓"; else bar="${bar}░"; fi
        i=$(( i + 1 ))
    done
    if   [ "$usable" -ge 90 ]; then ctx_color='1;31'
    elif [ "$usable" -ge 70 ]; then ctx_color='1;33'
    else                            ctx_color='32'
    fi
    parts+=("$(c "$ctx_color" "Ctx ${usable}% ${bar}")")
fi

if [ -n "${cost_usd:-}" ]; then
    cost_fmt=$(awk -v v="$cost_usd" 'BEGIN{printf("$%.2f", v)}')
    parts+=("$(c '38;5;70' "$cost_fmt")")
fi

if [ -n "${dur_ms:-}" ]; then
    secs=$(awk -v v="$dur_ms" 'BEGIN{printf("%d", v/1000)}')
    if   [ "$secs" -lt 60 ];    then clock="${secs}s"
    elif [ "$secs" -lt 3600 ];  then clock="$(( secs / 60 ))m"
    else                              clock="$(( secs / 3600 ))h$(( (secs % 3600) / 60 ))m"
    fi
    parts+=("$(c '38;5;30' "$clock")")
fi

printf '%s' "${parts[0]}"
for ((i=1; i<${#parts[@]}; i++)); do
    printf ' | %s' "${parts[i]}"
done
