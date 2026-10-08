#!/usr/bin/env bash
# 可选：把用户自己的私有 secrets 仓库克隆到项目外。应用不负责 Git 同步。
set -euo pipefail
umask 077

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
REPO_URL="${1:-${SECRETS_REPO_URL:-}}"
DESTINATION="${2:-${HOME}/.config/agent-manager/secrets}"

if [[ -z "$REPO_URL" ]]; then
  echo '用法：scripts/init-secrets.sh <YOUR_PRIVATE_REPO_URL> [外部目录]' >&2
  echo '不使用 Git 时，直接在应用「设置 → 密钥」新建文件即可。' >&2
  exit 1
fi
if [[ "$DESTINATION" != /* ]]; then
  echo '目标目录必须是绝对路径。' >&2
  exit 1
fi
if [[ -e "$DESTINATION" || -L "$DESTINATION" ]]; then
  echo '目标目录已存在；未拉取或修改。请手动管理 Git，再在应用中选择文件。' >&2
  exit 1
fi
if [[ ! -d "$(dirname "$DESTINATION")" ]]; then
  echo '请先创建目标目录的上级目录，再重试。' >&2
  exit 1
fi
DESTINATION="$(cd "$(dirname "$DESTINATION")" && pwd -P)/$(basename "$DESTINATION")"
case "$DESTINATION/" in
  "$REPO_ROOT/"*) echo '密钥仓库必须放在主项目之外。' >&2; exit 1 ;;
esac
# Git 错误可能回显带凭据的 URL，失败时只给出不含原始地址的提示。
if ! git clone --quiet -- "$REPO_URL" "$DESTINATION" >/dev/null 2>&1; then
  echo '克隆失败，请检查仓库地址及本机 Git 权限；不要在 URL 中填写 Token。' >&2
  exit 1
fi
printf '已克隆到：%s\n' "$DESTINATION"
echo '在「设置 → 密钥」选择其中的 mcp.env（或其他 .env 文件）。'
echo '应用只读写所选文件，不会自动提交或推送密钥。'
