#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/sync-agent-rules.sh [--mode copy|symlink] [--group all|global] [--target NAME] [--dry-run] [--no-backup]
EOF
  exit 1
}

mode="copy"
group_filter="all"
target_filter=""
dry_run="false"
backup_enabled="true"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode) mode="$2"; shift 2 ;;
    --group) group_filter="$2"; shift 2 ;;
    --target) target_filter="$2"; shift 2 ;;
    --dry-run) dry_run="true"; shift ;;
    --no-backup) backup_enabled="false"; shift ;;
    *) usage ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
manifest_path="$repo_root/../config/rules-targets.list"

[ -f "$manifest_path" ] || { echo "Manifest not found: $manifest_path" >&2; exit 1; }

expand_home() {
  local value="$1"
  value="${value//\{\{HOME\}\}/$HOME}"
  if [[ "$value" == "~/"* ]]; then
    value="${HOME}/${value#\~/}"
  elif [[ "$value" == "~" ]]; then
    value="${HOME}"
  fi
  printf '%s' "$value"
}

backup_target() {
  local target="$1"
  if [ "$backup_enabled" != "true" ] || [ ! -e "$target" ]; then return 0; fi
  local timestamp="$(date +%Y%m%d%H%M%S)"
  local backup_path="${target}.bak.${timestamp}"
  echo "backup: $target -> $backup_path"
  if [ "$dry_run" = "false" ]; then
    mv "$target" "$backup_path"
    ls -dt "${target}.bak."* 2>/dev/null | tail -n +11 | while read -r old; do rm -rf "$old"; done
  fi
}

sync_copy() {
  local src="$1" dst="$2"
  if [ -d "$src" ]; then
    [ -e "$dst" ] && [ ! -d "$dst" ] && backup_target "$dst"
    mkdir -p "$dst"
    find "$src" -maxdepth 1 -not -path "$src" | while read -r item; do
      sync_copy "$item" "$dst/${item#$src/}"
    done
  else
    if [ -e "$dst" ]; then
      local s_inode=$(stat -f %i "$src" 2>/dev/null || stat -c %i "$src")
      local d_inode=$(stat -f %i "$dst" 2>/dev/null || stat -c %i "$dst")
      [ "$s_inode" == "$d_inode" ] && return 0
      cmp -s "$src" "$dst" && { [ "$dry_run" = "false" ] && echo "unchanged: $dst"; return 0; }
      backup_target "$dst"
    fi
    echo "copy(atomic): $src -> $dst"
    if [ "$dry_run" = "false" ]; then
      mkdir -p "$(dirname "$dst")"
      local tmp="${dst}.tmp.$$"
      cp "$src" "$tmp" && mv -f "$tmp" "$dst"
    fi
  fi
}

sync_symlink() {
  local src="$1" dst="$2"
  [ -e "$dst" ] || [ -L "$dst" ] && backup_target "$dst"
  echo "symlink: $src -> $dst"
  if [ "$dry_run" = "false" ]; then
    mkdir -p "$(dirname "$dst")"
    ln -s "$src" "$dst"
  fi
}

# --- 核心增强：路径冲突预检 ---
awk -F'|' '
  BEGIN { err=0 }
  /^#/ || NF<4 { next }
  {
    dst = $4; sub(/\{\{HOME\}\}/, ENVIRON["HOME"], dst)
    if (seen[dst]) {
      print "❌ 关键错误: 规则路径冲突! " $1 " 与 " seen[dst] " 均指向: " dst > "/dev/stderr"
      err=1
    }
    seen[dst] = $1
  }
  END { exit err }
' "$manifest_path" || exit 1

# --- 执行同步 ---
while IFS='|' read -r name group src_rel dst_pat; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ "$group_filter" != "all" && "$group_filter" != "$group" ]] && continue
  [[ -n "$target_filter" && "$target_filter" != "$name" ]] && continue

  src_abs="$repo_root/$src_rel"
  dst_abs="$(expand_home "$dst_pat")"

  if [ "$mode" == "copy" ]; then sync_copy "$src_abs" "$dst_abs"
  else sync_symlink "$src_abs" "$dst_abs"; fi
  echo "INFO: [AGENT: Rules] $name 同步完成"
done < "$manifest_path"

echo "Done. mode=$mode dry_run=$dry_run"
