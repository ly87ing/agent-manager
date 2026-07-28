#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/sync-agent-rules.sh [--mode copy|symlink] [--group all|global] [--target NAME ...] [--dry-run] [--no-backup]
EOF
  exit 1
}

mode="copy"
group_filter="all"
target_filters=()
dry_run="false"
backup_enabled="true"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode) mode="$2"; shift 2 ;;
    --group) group_filter="$2"; shift 2 ;;
    --target) target_filters+=("$2"); shift 2 ;;
    --dry-run) dry_run="true"; shift ;;
    --no-backup) backup_enabled="false"; shift ;;
    *) usage ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
workspace_root="$(cd "$repo_root/.." && pwd -P)"
manifest_path="$repo_root/../config/rules-targets.list"
python_cmd="python3"
command -v python3 >/dev/null 2>&1 || python_cmd="python"

[ -f "$manifest_path" ] || { echo "Manifest not found: $manifest_path" >&2; exit 1; }

# Infer the agent from the target path prefix and check whether it is installed
# Returns 0 if installed (or cannot be inferred, allowed by default), returns 1 if not installed
agent_installed_for_target() {
  local dst_pat="$1"
  local expanded
  expanded="$(expand_target_path "$dst_pat")"
  case "$expanded" in
    "$HOME"/.claude/*|"$HOME"/.claude)
      command -v claude >/dev/null 2>&1 ;;
    "$HOME"/.gemini/antigravity-cli/*|"$HOME"/.gemini/antigravity-cli)
      command -v agy >/dev/null 2>&1 ;;
    "$HOME"/.gemini/*|"$HOME"/.gemini)
      command -v gemini >/dev/null 2>&1 ;;
    "$HOME"/.codex/*|"$HOME"/.codex)
      # codex has no standard bin, so check whether the config directory exists
      [ -d "$HOME/.codex" ] ;;
    *)
      # Cannot be inferred, allowed by default
      return 0 ;;
  esac
}

expand_target_path() {
  local value="$1"
  value="${value//\{\{REPO_ROOT\}\}/$workspace_root}"
  value="${value//\{\{HOME\}\}/$HOME}"
  if [[ "$value" == "~/"* ]]; then
    value="${HOME}/${value#\~/}"
  elif [[ "$value" == "~" ]]; then
    value="${HOME}"
  fi
  printf '%s' "$value"
}

target_selected() {
  local name="$1"
  local target
  if [ "${#target_filters[@]}" -eq 0 ]; then
    return 0
  fi
  for target in "${target_filters[@]}"; do
    if [ "$target" = "$name" ]; then
      return 0
    fi
  done
  return 1
}

validate_manifest_entry() {
  local name="$1" src_rel="$2" dst_pat="$3"
  local src_abs="$repo_root/$src_rel"

  if [[ -z "$name" || -z "$src_rel" || -z "$dst_pat" ]]; then
    echo "❌ 关键错误: manifest 条目字段不能为空: $name|$src_rel|$dst_pat" >&2
    return 1
  fi

  if [[ "$src_rel" == *".."* ]]; then
    echo "❌ 关键错误: manifest source 禁止包含 '..': $src_rel" >&2
    return 1
  fi

  if [[ "$dst_pat" != /* && "$dst_pat" != "~"* && "$dst_pat" != "{{HOME}}"* && "$dst_pat" != "{{REPO_ROOT}}"* ]]; then
    echo "❌ 关键错误: manifest target 必须是绝对路径或以 ~ / {{HOME}} / {{REPO_ROOT}} 开头: $dst_pat" >&2
    return 1
  fi

  if [[ ! -e "$src_abs" ]]; then
    echo "❌ 关键错误: manifest source 不存在: $src_rel" >&2
    return 1
  fi
}

backup_target() {
  local target="$1"
  if is_repo_native_target "$target"; then
    echo "remove(existing repo-native): $target"
    if [ "$dry_run" = "false" ]; then
      rm -rf "$target"
    fi
    return 0
  fi
  if [ "$backup_enabled" != "true" ] || { [ ! -e "$target" ] && [ ! -L "$target" ]; }; then return 0; fi
  local timestamp="$(date +%Y%m%d%H%M%S)"
  local backup_path="${target}.bak.${timestamp}"
  echo "backup: $target -> $backup_path"
  if [ "$dry_run" = "false" ]; then
    mv "$target" "$backup_path"
    ls -dt "${target}.bak."* 2>/dev/null | tail -n +11 | while read -r old; do rm -rf "$old"; done
  fi
}

remove_target() {
  local target="$1"
  echo "remove(stale): $target"
  if [ "$dry_run" = "false" ]; then
    if is_ephemeral_artifact "$target"; then
      rm -rf "$target"
    elif [ "$backup_enabled" = "true" ] && { [ -e "$target" ] || [ -L "$target" ]; }; then
      backup_target "$target"
    else
      rm -rf "$target"
    fi
  fi
}

cleanup_legacy_target_paths() {
  local name="$1"
  local dst_abs="$2"
  local legacy_paths=("")

  case "$name" in
    codex_global)
      if [ "$dst_abs" = "$HOME/.codex/AGENTS.md" ]; then
        legacy_paths=("$HOME/.codex/rules/global.md" "$HOME/.codex/rules/modules")
      fi
      ;;
    claude_global)
      legacy_paths=("$HOME/.claude/rules/global.md" "$HOME/.claude/rules/modules")
      ;;
    antigravity_rules)
      # The first four are Gemini-CLI-era paths. The fifth is the leftover this
      # tool made itself: the module tree stopped being built in 35bfe9b, but the
      # copies already distributed next to the plugin's `rules/` were never in any
      # cleanup list, so they outlived the layout by months. It is derived from the
      # target rather than hardcoded so a plugin-path change carries it along.
      legacy_paths=(
        "$HOME/.gemini/GEMINI.md"
        "$HOME/.gemini/rules/global.md"
        "$HOME/.gemini/modules"
        "$HOME/.gemini/rules/modules"
        "$(dirname "$dst_abs")/modules"
      )
      ;;
  esac

  for legacy_path in "${legacy_paths[@]}"; do
    if [ -n "$legacy_path" ] && { [ -e "$legacy_path" ] || [ -L "$legacy_path" ]; }; then
      remove_target "$legacy_path"
    fi
  done
}

cleanup_repo_native_backups() {
  local target="$1"
  local stale_backup=""
  is_repo_native_target "$target" || return 0

  for stale_backup in "${target}.bak."*; do
    if [ -e "$stale_backup" ] || [ -L "$stale_backup" ]; then
      remove_target "$stale_backup"
    fi
  done
}

replace_existing_target() {
  # Overwrite backups are redundant with git: the target is rewritten from a
  # tracked dist artifact, so any prior state is recoverable from history.
  # Replace in place without a .bak. (remove_target still backs up stale/legacy
  # files before deletion — git cannot recover a file the tool stops managing.)
  local target="$1"
  echo "remove(existing): $target"
  if [ "$dry_run" = "false" ]; then
    rm -rf "$target"
  fi
}

is_ephemeral_artifact() {
  local target_name
  target_name="$(basename "$1")"
  [[ "$target_name" == *.bak.* || "$target_name" == *.tmp_* ]]
}

is_repo_native_target() {
  local target="$1"
  case "$target" in
    "$workspace_root"/.github/*|"$workspace_root"/AGENTS.md|"$workspace_root"/CLAUDE.md|"$workspace_root"/GEMINI.md)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

sync_copy() {
  local src="$1" dst="$2"
  if [ -d "$src" ]; then
    if [ -L "$dst" ] || { [ -e "$dst" ] && [ ! -d "$dst" ]; }; then
      replace_existing_target "$dst"
    fi
    if [ "$dry_run" = "false" ]; then
      mkdir -p "$dst"
    fi
    find "$src" -maxdepth 1 -not -path "$src" | while read -r item; do
      sync_copy "$item" "$dst/${item#$src/}"
    done
    if [ -d "$dst" ]; then
      find "$dst" -maxdepth 1 -mindepth 1 | while read -r existing; do
        local rel="${existing#$dst/}"
        if [ ! -e "$src/$rel" ] && [ ! -L "$src/$rel" ]; then
          remove_target "$existing"
        fi
      done
    fi
  else
    if [ -L "$dst" ]; then
      replace_existing_target "$dst"
    elif [ -e "$dst" ]; then
      local s_inode=$(stat -f %i "$src" 2>/dev/null || stat -c %i "$src")
      local d_inode=$(stat -f %i "$dst" 2>/dev/null || stat -c %i "$dst")
      [ "$s_inode" == "$d_inode" ] && return 0
      cmp -s "$src" "$dst" && { [ "$dry_run" = "false" ] && echo "unchanged: $dst"; return 0; }
      replace_existing_target "$dst"
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
  if [ -e "$dst" ] || [ -L "$dst" ]; then
    replace_existing_target "$dst"
  fi
  echo "symlink: $src -> $dst"
  if [ "$dry_run" = "false" ]; then
    mkdir -p "$(dirname "$dst")"
    ln -s "$src" "$dst"
  fi
}

# --- Core enhancement: path conflict pre-check ---
MANIFEST_PATH="$manifest_path" WORKSPACE_ROOT="$workspace_root" HOME_DIR="$HOME" "$python_cmd" - <<'PY' || exit 1
import os
import sys
from pathlib import Path

manifest_path = Path(os.environ["MANIFEST_PATH"])
workspace_root = os.environ["WORKSPACE_ROOT"]
home_dir = os.environ["HOME_DIR"]
seen: dict[str, str] = {}

for raw_line in manifest_path.read_text(encoding="utf-8").splitlines():
    line = raw_line.strip()
    if not line or line.startswith("#"):
        continue

    parts = [field.strip() for field in raw_line.split("|", 3)]
    if len(parts) < 4:
        continue

    name, _group, _source, dst = parts
    dst = dst.replace("{{REPO_ROOT}}", workspace_root)
    dst = dst.replace("{{HOME}}", home_dir)
    if dst.startswith("~/"):
        dst = os.path.join(home_dir, dst[2:])
    elif dst == "~":
        dst = home_dir

    normalized = os.path.realpath(os.path.normpath(dst))
    previous = seen.get(normalized)
    if previous is not None:
        print(f"❌ 关键错误: 规则路径冲突! {name} 与 {previous} 均指向: {normalized}", file=sys.stderr)
        sys.exit(1)
    seen[normalized] = name
PY

while IFS='|' read -r name group src_rel dst_pat; do
  name=$(echo "$name" | awk '{$1=$1;print}')
  group=$(echo "$group" | awk '{$1=$1;print}')
  src_rel=$(echo "$src_rel" | awk '{$1=$1;print}')
  dst_pat=$(echo "$dst_pat" | awk '{$1=$1;print}')

  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ "$group_filter" != "all" && "$group_filter" != "$group" ]] && continue
  target_selected "$name" || continue

  validate_manifest_entry "$name" "$src_rel" "$dst_pat" || exit 1
done < "$manifest_path"

# --- Execute the sync ---
while IFS='|' read -r name group src_rel dst_pat; do
  # Trim whitespaces
  name=$(echo "$name" | awk '{$1=$1;print}')
  group=$(echo "$group" | awk '{$1=$1;print}')
  src_rel=$(echo "$src_rel" | awk '{$1=$1;print}')
  dst_pat=$(echo "$dst_pat" | awk '{$1=$1;print}')

  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ "$group_filter" != "all" && "$group_filter" != "$group" ]] && continue
  target_selected "$name" || continue

  src_abs="$repo_root/$src_rel"
  dst_abs="$(expand_target_path "$dst_pat")"

  if ! agent_installed_for_target "$dst_pat"; then
    echo "INFO: [AGENT: Rules] $name 目标 agent 未安装，跳过。"
    continue
  fi

  if [ "$mode" == "copy" ]; then sync_copy "$src_abs" "$dst_abs"
  else sync_symlink "$src_abs" "$dst_abs"; fi
  cleanup_repo_native_backups "$dst_abs"
  cleanup_legacy_target_paths "$name" "$dst_abs"
  echo "INFO: [AGENT: Rules] $name 同步完成"
done < "$manifest_path"

echo "Done. mode=$mode dry_run=$dry_run"
