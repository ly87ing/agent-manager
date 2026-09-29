#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
SECRETS_DIR="$ROOT_DIR/secrets"

echo "=========================================="
echo "  Agent Manager - Secrets 初始化"
echo "=========================================="
echo ""

if [ -d "$SECRETS_DIR" ]; then
    if [ -d "$SECRETS_DIR/.git" ]; then
        echo "✅ secrets 目录已存在且是 git 仓库"
        echo ""
        echo "正在拉取最新凭证..."
        cd "$SECRETS_DIR"
        git pull 2>&1 || echo "⚠️  拉取失败，请检查网络连接"
        echo ""
        echo "✅ secrets 已是最新"
        exit 0
    else
        echo "⚠️  secrets 目录存在但不是 git 仓库"
        echo "   路径：$SECRETS_DIR"
        exit 1
    fi
fi

echo "🔍 检测到 secrets 目录缺失"
echo ""

REPO_URL="${1:-${SECRETS_REPO_URL:-}}"

if [ -z "$REPO_URL" ]; then
    echo "❌ 未配置 secrets 仓库地址"
    echo ""
    echo "请通过以下任一方式提供："
    echo "  SECRETS_REPO_URL=https://example.com/your-secrets.git ./scripts/init-secrets.sh"
    echo "  ./scripts/init-secrets.sh <your-secrets-repo-url>"
    exit 1
fi

echo "正在克隆凭证仓库：$REPO_URL"
echo ""

cd "$ROOT_DIR"
if git clone "$REPO_URL" secrets 2>&1; then
    echo ""
    echo "✅ secrets 仓库克隆成功！"
    echo ""
    ls -la "$SECRETS_DIR"/*.env 2>/dev/null || echo "  (暂无 .env 文件)"
    echo ""
    echo "=========================================="
    echo "  初始化完成！请在 Agent Manager 的「设置 → 密钥」选择所需密钥文件。"
    echo "  若需安装助手，请使用各助手的官方安装方式。"
    echo "=========================================="
    exit 0
else
    echo ""
    echo "❌ 克隆失败"
    echo ""
    echo "请手动执行:"
    echo "  cd $ROOT_DIR"
    echo "  git clone <your-secrets-repo-url> secrets"
    exit 1
fi
