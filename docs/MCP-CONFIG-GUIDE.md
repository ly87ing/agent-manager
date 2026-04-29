# MCP 配置完全指南

> 本文档记录了在配置 MCP (Model Context Protocol) 服务器过程中遇到的所有问题和解决方案。

---

## 目录

1. [快速开始](#快速开始)
2. [各 Agent 的 MCP 配置格式](#各-agent-的-mcp-配置格式)
3. [当前仓库默认受管 MCP 服务器](#当前仓库默认受管-mcp-服务器)
4. [augment-context-engine 专项说明](#augment-context-engine-专项说明)
5. [chrome-devtools 专项说明](#chrome-devtools-专项说明)
6. [常见问题与解决方案](#常见问题与解决方案)
7. [调试技巧](#调试技巧)

---

## 快速开始

### 自动配置（推荐）

使用 Agent Manager 自动同步所有 Agent 的 MCP 配置：

```bash
# 全量同步（包含 Agent 安装、资源更新和 MCP 配置）
make bootstrap

# 本地环境收敛（会更新受管 MCP 资源并准备运行时）
make sync

# 仅同步 MCP 配置
./manage.sh mcp:sync
```

### 手动配置

如果自动配置失败，参考下文各 Agent 的配置格式手动添加。

---

## 当前仓库默认受管 MCP 服务器

| Server | 用途 | 受管命令 | 凭证 | 默认策略 |
|--------|------|----------|------|----------|
| `augment-context-engine` | 代码库检索、语义搜索、codebase-retrieval | `auggie --mcp --mcp-auto-workspace` | `AUGMENT_SESSION_AUTH` | 启用 |
| `chrome-devtools` | Chrome DevTools 浏览器调试、性能分析、network / console / screenshot | `npx -y chrome-devtools-mcp@latest` | 无 | 启用 |

**维护约定**:
- `chrome-devtools` 跟随官方 `@latest`，source-of-truth 维护在 `config/mcp.json`，由 `sync_mcp.py` 统一分发到所有受支持 Agent。
- 首次调用 `chrome-devtools` 时，`npx` 可能会下载最新包版本；需要本机有可用的 Node.js / npm 运行时。
- `make doctor` 会在 `chrome-devtools` 处于启用状态时预检 `npx` 运行时是否可用；实际包版本仍以首次调用或缓存失效后的 `@latest` 解析结果为准。
- 如果后续需要统一增加 `--browserUrl`、`--headless`、`--isolated` 等参数，应修改 `config/mcp.json`，不要手改单个 Agent 的 MCP 配置文件。
- 本项目不管理 GitHub Copilot，因此不会再向其配置目录注入托管 MCP，也不会再生成 Copilot 专属 repo-native rules 文件。

---

## 各 Agent 的 MCP 配置格式

**关键发现**：不同 Agent 使用不同的 MCP 配置格式！这是最大的坑。

### 格式对比表

| Agent | 配置文件路径 | 根键名 | command 格式 | 环境变量键 | 特殊要求 |
|-------|-------------|--------|-------------|-----------|----------|
| **Claude Code** | `~/.claude.json` | `mcpServers` | 字符串 + args 数组 | `env` | - |
| **Gemini CLI** | `~/.gemini/settings.json` | `mcpServers` | 字符串 + args 数组 | `env` | 项目配置优先 |
| **Cursor** | `~/.cursor/mcp.json` | `mcpServers` | 字符串 + args 数组 | `env` | - |
| **OpenCode** | `~/.config/opencode/opencode.json` | `mcp` | **数组** | `environment` | 必须有 `type: "local"` |
| **Codex** | `~/.codex/config.toml` | `[mcp_servers.*]` | 字符串 + args 数组 | `[mcp_servers.*.env]` | TOML 字符串转义 |

---

### Claude Code

**配置文件**: `~/.claude.json`

```json
{
  "mcpServers": {
    "augment-context-engine": {
      "command": "auggie",
      "args": ["--mcp", "--mcp-auto-workspace"],
      "env": {
        "AUGMENT_SESSION_AUTH": "{\"accessToken\":\"...\",\"tenantURL\":\"...\",\"scopes\":[]}"
      }
    }
  }
}
```

**注意事项**:
- JSON 中的 JSON 字符串需要正确转义双引号
- 支持全局配置和项目配置

---

### Gemini CLI

**配置文件**: `~/.gemini/settings.json`（全局）或 `./.gemini/settings.json`（项目）

```json
{
  "mcpServers": {
    "augment-context-engine": {
      "command": "auggie",
      "args": ["--mcp", "--mcp-auto-workspace"],
      "env": {
        "AUGMENT_SESSION_AUTH": "{\"accessToken\":\"...\",\"tenantURL\":\"...\",\"scopes\":[]}"
      }
    }
  }
}
```

**重要**:
- **项目配置优先于全局配置**
- 如果项目目录存在 `.gemini/settings.json`，会覆盖全局配置
- "Disconnected" 状态是正常的，stdio MCP 按需启动

**验证命令**:
```bash
gemini mcp list
```

---

### Cursor

**配置文件**: `~/.cursor/mcp.json`

```json
{
  "mcpServers": {
    "augment-context-engine": {
      "command": "auggie",
      "args": ["--mcp", "--mcp-auto-workspace"],
      "env": {
        "AUGMENT_SESSION_AUTH": "{\"accessToken\":\"...\"}"
      }
    }
  }
}
```

---

### OpenCode ⚠️

**配置文件**: `~/.config/opencode/opencode.json`

```json
{
  "mcp": {
    "augment-context-engine": {
      "type": "local",
      "command": ["auggie", "--mcp", "--mcp-auto-workspace"],
      "environment": {
        "AUGMENT_SESSION_AUTH": "{\"accessToken\":\"...\"}",
        "_MANAGED_BY": "agent-manager"
      }
    }
  }
}
```

**特殊要求**:
1. 根键是 `mcp` 而不是 `mcpServers`
2. `command` 必须是**数组**（不是字符串 + args）
3. 环境变量键是 `environment` 而不是 `env`
4. **必须**有 `type: "local"` 字段

**验证命令**:
```bash
opencode mcp list
```

**常见错误**:
```
Error: Configuration is invalid
↳ Invalid input mcp.augment-context-engine
```
→ 缺少 `type: "local"` 或 `command` 格式错误

---

### Codex ⚠️

**配置文件**: `~/.codex/config.toml`

```toml
[mcp_servers.augment-context-engine]
command = "auggie"
args = ["--mcp", "--mcp-auto-workspace"]
startup_timeout_sec = 120

[mcp_servers.augment-context-engine.env]
AUGMENT_SESSION_AUTH = '{"accessToken":"...","tenantURL":"...","scopes":[]}'
_MANAGED_BY = "agent-manager"
```

**特殊要求**:
1. TOML 格式，注意语法
2. 包含双引号的字符串值必须用**单引号**包裹（TOML literal string）
3. 否则会导致解析错误

**常见错误**:
```toml
# ❌ 错误：双引号未转义
AUGMENT_SESSION_AUTH = "{"accessToken":"..."}"

# ✅ 正确：使用单引号
AUGMENT_SESSION_AUTH = '{"accessToken":"..."}'
```

**验证命令**:
```bash
codex mcp list
```

---

## augment-context-engine 专项说明

### 什么是 augment-context-engine

`augment-context-engine` 是一个 MCP 服务器，提供代码库检索（codebase-retrieval）工具。它通过 `auggie` 命令行工具运行。

### 认证方式演进

历史上有两种认证方式：

#### 旧方式（已废弃）
```json
{
  "env": {
    "AUGMENT_API_TOKEN": "ace_...",
    "AUGMENT_API_URL": "https://..."
  }
}
```

#### 新方式（当前使用）
```json
{
  "env": {
    "AUGMENT_SESSION_AUTH": "{\"accessToken\":\"ace_...\",\"tenantURL\":\"...\",\"scopes\":[]}"
  }
}
```

**区别**:
- 旧方式使用 API Token，需要单独的 URL
- 新方式使用完整的 Session Auth JSON，包含 accessToken、tenantURL 和 scopes

### 凭证管理

凭证存储在 **独立的私有 git 仓库** `secrets/mcp.env`：

```bash
# secrets/mcp.env
AUGMENT_SESSION_AUTH='{"accessToken":"ace_...","tenantURL":"...","scopes":[]}'
```

#### 首次设置（自动化）

```bash
# 1. 克隆主仓库
git clone https://github.com/ly87ing/agent-manager.git
cd agent-manager

# 2. 运行 make bootstrap
make bootstrap
```

**自动化流程**：
- `make update` / `make bootstrap` 会在顶层检查 `secrets/` 是否存在
- `mcp:sync` 默认会准备 MCP 运行时：同步本地 secrets 仓库、按需 clone/pull MCP repo，并执行 setup/build
- 如需阻止联网拉取或安装，请追加 `--offline`

#### 手动初始化

如需手动控制：

```bash
cd agent-manager
./scripts/init-secrets.sh
```

#### 更新凭证

```bash
cd secrets
# 编辑凭证文件
vim mcp.env

# 提交并推送
git add mcp.env
git commit -m "Update credentials"
git push origin main
```

**优势**：
- ✅ 简单易用，无 submodule 复杂性
- ✅ `mcp:sync` / `mcp:prepare` 默认都会把配置与运行时一起收敛，减少“配置已写入但运行时未准备”的半完成状态
- ✅ 凭证版本化、可审计
- ✅ 权限分离（私有仓库）

**安全实践**:
1. ✅ secrets 仓库必须设为 **private**
2. ✅ 定期轮换凭证
3. ✅ 限制访问人员
4. ❌ 不要将凭证提交到主仓库

### 工作原理

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────────┐
│   Agent     │────▶│  auggie (MCP)    │────▶│  codebase-retrieval │
│  (Claude/   │◀────│  Server          │◀────│  Tool               │
│   Gemini/   │     │                  │     │                     │
│   etc.)     │     │  env:            │     │                     │
└─────────────┘     │  AUGMENT_SESSION │     └─────────────────────┘
                    │  _AUTH           │
                    └──────────────────┘
```

### 为什么 Gemini CLI 显示 "Disconnected"

**这是正常行为！**

Gemini CLI 的 stdio MCP 服务器采用**按需启动**策略：
- `gemini mcp list` 显示 "Disconnected" 表示当前没有活跃连接
- 实际使用 MCP 功能时（如调用 `codebase-retrieval`），服务器会自动启动
- 使用后会自动关闭以节省资源

**验证方法**:
```bash
# 实际使用测试
gemini -p "请用 codebase-retrieval 查看当前项目的结构"
```

---

## chrome-devtools 专项说明

### 什么是 chrome-devtools

`chrome-devtools` 是 Chrome 官方提供的 MCP server，用于让 Agent 直接连接和控制 Chrome，执行页面调试、network 观察、console 检查、性能 trace、截图和其他 DevTools 级别诊断。

### 当前维护策略

本项目默认通过以下命令受管分发给所有支持 MCP 的 Agent：

```bash
npx -y chrome-devtools-mcp@latest
```

这样可以跟随 Chrome DevTools 官方 MCP server 的最新发布，而不需要在每个 Agent 的本地配置里分别升级版本。

### 使用约束

- `chrome-devtools` 不依赖额外 secrets，占位符注入链不会为它加载专用凭证。
- `npx` 会在首次运行或本地缓存失效时解析并下载最新版本，因此应将版本波动视为上游变更，而不是本仓库内的 drift。
- 如果需要连接已运行的 Chrome、统一打开 headless、接受自签名证书或追加其他浏览器参数，应在 `config/mcp.json` 的 `args` 中集中维护，而不是在各 Agent 侧手工追加。

### 官方参考

- Chrome 官方博客：<https://developer.chrome.com/blog/chrome-devtools-mcp>
- 官方仓库：<https://github.com/mcp/chromedevtools/chrome-devtools-mcp>

---

## 常见问题与解决方案

### Q1: OpenCode 报错 "Invalid input mcp.*"

**原因**: 配置格式不符合 OpenCode 的 schema 要求

**解决**: 确保包含以下三个要素：
```json
{
  "mcp": {
    "server-name": {
      "type": "local",           // ← 必需
      "command": ["cmd", "..."],  // ← 必须是数组
      "environment": {}           // ← 不是 env
    }
  }
}
```

---

### Q2: Codex 报错 "unexpected key or value"

**原因**: TOML 中 JSON 字符串的双引号未正确处理

**解决**: 使用单引号包裹包含双引号的值：
```toml
# ❌ 错误
KEY = "{"nested":"value"}"

# ✅ 正确
KEY = '{"nested":"value"}'
```

---

### Q3: Gemini CLI 配置不生效

**原因**: 项目配置覆盖了全局配置

**解决**: 检查并更新项目配置：
```bash
# 检查是否存在项目配置
ls -la ./.gemini/settings.json

# 如果存在，需要同时更新项目配置
# 或者删除项目配置使用全局配置
rm ./.gemini/settings.json
```

---

### Q4: MCP 服务器无法启动

**排查步骤**:

1. **检查命令是否可用**:
   ```bash
   which auggie
   auggie --version
   ```

2. **手动测试命令**:
   ```bash
   AUGMENT_SESSION_AUTH='{"accessToken":"..."}' \
     auggie --mcp --mcp-auto-workspace
   ```

3. **检查环境变量是否正确传递**:
   - JSON 中的 JSON 字符串需要正确转义
   - 检查配置文件中的转义字符

4. **查看 Agent 日志**:
   - Claude Code: 检查对话错误信息
   - Gemini CLI: `gemini debug mcp`
   - OpenCode: 启动时添加 `--debug` 标志

---

### Q5: 凭证过期/失效

**症状**: MCP 服务器启动失败，提示认证错误

**解决**:

1. 获取新的凭证
2. 更新 `secrets/mcp.env`:
   ```bash
   # 编辑文件
   vim secrets/mcp.env
   
   # 或使用 echo
   echo 'AUGMENT_SESSION_AUTH='\''{"accessToken":"new_token"}'\''' > secrets/mcp.env
   ```

3. 重新同步配置:
   ```bash
   ./manage.sh mcp:sync
   # 或
   cd agent-mcp && python3 sync_mcp.py --config ../config/manager.json --def-file ../config/mcp.json --prepare-runtime
   ```

---

### Q6: 忘记 clone secrets 仓库

**症状**: 运行时提示找不到凭证文件

**解决**:

```bash
# 方法 1: 自动初始化（推荐）
SECRETS_REPO_URL=https://example.com/your-secrets.git make update
# 会自动检测并克隆 secrets 仓库

# 方法 2: 手动运行初始化脚本
./scripts/init-secrets.sh https://example.com/your-secrets.git

# 方法 3: 手动克隆
cd agent-manager
git clone https://example.com/your-secrets.git secrets
```

---

## 调试技巧

### 通用调试命令

```bash
# 1. 检查配置文件语法
python3 -m json.tool ~/.claude.json > /dev/null && echo "JSON 有效"

# 2. 检查 MCP 配置提取
python3 -c "import json; d=json.load(open('~/.claude.json')); print(json.dumps(d.get('mcpServers'), indent=2))"

# 3. 手动启动 MCP 服务器测试
export AUGMENT_SESSION_AUTH='{"accessToken":"..."}'
auggie --mcp --mcp-auto-workspace
```

### 各 Agent 专用调试

```bash
# Claude Code
# 在对话中询问 MCP 状态

# Gemini CLI
gemini mcp list
gemini debug mcp  # 需要登录

# OpenCode
opencode mcp list
opencode --print "test mcp"  # 测试模式

# Codex
codex mcp list
# 检查 TOML 语法
python3 - <<'PY'
import os
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
with open(os.path.expanduser("~/.codex/config.toml"), "rb") as fh:
    tomllib.load(fh)
print("Codex 配置有效")
PY
# Python 3.10 及以下若提示缺少 tomli，请先执行: python3 -m pip install tomli
```

### 日志位置

| Agent | 日志位置 |
|-------|---------|
| Claude Code | 终端输出 |
| Gemini CLI | `gemini debug mcp` |
| OpenCode | `--debug` 标志 |
| Codex | 终端输出 |
| Cursor | 开发者工具 |

---

## 附录：Agent Manager MCP 同步流程

```
默认模式（mcp:sync / make sync / make update / make bootstrap 中的 MCP 步骤）:
1. 读取 config/mcp.json 获取服务器定义
   ↓
2. 按固定优先级加载凭证: root/.env → secrets/*.env(按文件名排序覆盖) → 系统环境变量回退
   ↓
3. 替换 ${VAR} 占位符；若真实执行时仍有未解析占位符，则直接失败
   ↓
4. 根据各 Agent 格式生成配置
   ├─ JSON 格式 (Claude/Gemini/Cursor)
   ├─ OpenCode 格式 (type + command 数组 + environment)
   └─ TOML 格式 (Codex，带字符串转义)
   ↓
5. 校验现有配置可解析、生成结果合法、且仅删除 _MANAGED_BY=agent-manager 的托管段
   ↓
6. 原子写入各配置文件并记录 _MANAGED_BY = "agent-manager" 标记
7. 同步本地 secrets Git 仓库（非 offline 且非 dry-run）
8. clone/pull MCP repo，按需要执行 setup/build
```

---

## 更新历史

| 日期 | 变更内容 |
|------|---------|
| 2026-03-13 | 初始版本，记录所有踩坑经验 |

---

## 相关文档

- [Agent Manager 架构说明](./ARCHITECTURE.md)
- [凭证管理指南](./SECRETS-MANAGEMENT.md)
- [故障排查手册](./TROUBLESHOOTING.md)
