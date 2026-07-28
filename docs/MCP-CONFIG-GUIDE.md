# MCP 配置完全指南

> 本文档记录了在配置 MCP (Model Context Protocol) 服务器过程中遇到的所有问题和解决方案。

---

## 目录

1. [快速开始](#快速开始)
2. [各 Agent 的 MCP 配置格式](#各-agent-的-mcp-配置格式)
3. [当前仓库默认受管 MCP 服务器](#当前仓库默认受管-mcp-服务器)
4. [凭证管理](#凭证管理)
5. [为什么 Antigravity CLI 显示 "Disconnected"](#为什么-antigravity-cli-显示-disconnected)
6. [chrome-devtools 专项说明](#chrome-devtools-专项说明)
7. [常见问题与解决方案](#常见问题与解决方案)
8. [调试技巧](#调试技巧)

---

## 快速开始

### 自动配置（推荐）

使用 Agent Manager 自动同步所有 Agent 的 MCP 配置：

```bash
# 全量同步（包含 Agent 安装、资源更新和 MCP 配置）
make init

# 本地环境收敛（会更新受管 MCP 资源并准备运行时）
make apply

# 仅收敛 MCP 配置到本机
./manage.sh mcp:apply
```

### 手动配置

如果自动配置失败，参考下文各 Agent 的配置格式手动添加。

---

## 当前仓库默认受管 MCP 服务器

| Server | 用途 | 受管命令 | 凭证 | 默认策略 |
|--------|------|----------|------|----------|
| `chrome-devtools` | Chrome DevTools 浏览器调试、性能分析、network / console / screenshot | `npx -y chrome-devtools-mcp@<固定版本>` | 无 | 启用 |
| `grok-search` | web_search / web_fetch 联网搜索与网页抓取 | `uvx --from git+https://github.com/GuDaStudio/GrokSearch@<固定commit> grok-search` | `GROK_API_URL`, `GROK_API_KEY`, `GROK_MODEL`, `TAVILY_ENABLED`, `TAVILY_API_KEY`, `TAVILY_API_URL`, `FIRECRAWL_API_KEY`, `FIRECRAWL_API_URL` | 启用 |
| `serena` | JS/TS 语义级代码导航（LSP：find_symbol / 引用 / 符号概览） | `uvx --from git+https://github.com/oraios/serena@<固定commit> serena start-mcp-server --context ide-assistant` | 无 | 启用 |

**维护约定**:
- 受管 MCP 的实际命令必须固定版本或固定 commit；`@latest`、分支名只允许出现在 `version_lock` 的查询元数据里。
- `config/mcp.json` 保存服务器定义、命令模板和查询元数据；`make bump` / `make update` / `make init` 会先运行 `scripts/update_version_pins.py`，查询 npm 最新版本或 Git ref 最新 commit，再写入本机 `config/version-pins.local.json`。
- 首次调用 `chrome-devtools` 时，`npx` 可能会下载锁定版本的包；需要本机有可用的 Node.js / npm 运行时。
- `chrome-devtools` 的 Codex 启动超时按 120 秒维护，覆盖默认 30 秒，避免 `npx` 冷启动或缓存失效时被误判为 MCP 启动失败。
- `make doctor` 会在 `chrome-devtools` 处于启用状态时预检 `npx` 运行时是否可用；实际包版本以 `config/mcp.json` 套用本地版本锁后的有效配置为准。
- 如果后续需要统一增加 `--browserUrl`、`--headless`、`--isolated` 等参数，应修改 `config/mcp.json`，不要手改单个 Agent 的 MCP 配置文件。
- 本项目不管理 GitHub Copilot，因此不会再向其配置目录注入托管 MCP，也不会再生成 Copilot 专属 repo-native rules 文件。

---

## 各 Agent 的 MCP 配置格式

**关键发现**：不同 Agent 使用不同的 MCP 配置格式！这是最大的坑。

### 格式对比表

| Agent | 配置文件路径 | 根键名 | command 格式 | 环境变量键 | 特殊要求 |
|-------|-------------|--------|-------------|-----------|----------|
| **Claude Code** | `~/.claude.json` | `mcpServers` | 字符串 + args 数组 | `env` | - |
| **Antigravity CLI** | `~/.gemini/config/mcp_config.json` | `mcpServers` | 字符串 + args 数组 | `env` | workspace 配置 `.agents/mcp_config.json` 可补充 |
| **Codex** | `~/.codex/config.toml` | `[mcp_servers.*]` | 字符串 + args 数组 | `[mcp_servers.*.env]` | TOML 字符串转义 |

---

### Claude Code

**配置文件**: `~/.claude.json`

```json
{
  "mcpServers": {
    "grok-search": {
      "command": "uvx",
      "args": ["--from", "git+https://github.com/GuDaStudio/GrokSearch@<固定commit>", "grok-search"],
      "env": {
        "GROK_API_URL": "https://...",
        "GROK_API_KEY": "sk-..."
      }
    }
  }
}
```

**注意事项**:
- JSON 中的 JSON 字符串需要正确转义双引号
- 支持全局配置和项目配置

---

### Antigravity CLI

**配置文件**: `~/.gemini/config/mcp_config.json`（全局）或 `./.agents/mcp_config.json`（workspace）

> **不要写 `~/.gemini/antigravity-cli/mcp_config.json`**。`~/.gemini/antigravity-cli/` 是 agy 的 app data 目录（settings.json、conversations、log 都在那儿），唯独 MCP 全局配置不在。2026-06-01 至 2026-07-27 期间受管路径误指向该目录，结果 Antigravity 一直读不到受管配置、跑的是更早遗留的未固定版本命令。判定方法：两份配置的服务器差集挑一个（当时是 `serena`），跑 `agy --print`，用 `pgrep` 看它拉起哪些 MCP 子进程即可确认实际读取源。

```json
{
  "mcpServers": {
    "grok-search": {
      "command": "uvx",
      "args": ["--from", "git+https://github.com/GuDaStudio/GrokSearch@<固定commit>", "grok-search"],
      "env": {
        "GROK_API_URL": "https://...",
        "GROK_API_KEY": "sk-..."
      }
    }
  }
}
```

**重要**:
- Antigravity CLI 使用独立的 sparse MCP 配置，不再复用旧 Gemini CLI 的 `settings.json`
- workspace 本地配置放在 `.agents/mcp_config.json`
- "Disconnected" 状态通常是正常的，stdio MCP 按需启动

**验证命令**:
```bash
agy
# 在 TUI 中输入 /mcp
```

---

### Codex ⚠️

**配置文件**: `~/.codex/config.toml`

```toml
[mcp_servers.grok-search]
command = "uvx"
args = ["--from", "git+https://github.com/GuDaStudio/GrokSearch@<固定commit>", "grok-search"]
startup_timeout_sec = 120

[mcp_servers.grok-search.env]
GROK_API_KEY = 'sk-...'
_MANAGED_BY = "agent-manager"
```

**特殊要求**:
1. TOML 格式，注意语法
2. 包含双引号的字符串值必须用**单引号**包裹（TOML literal string）
3. 否则会导致解析错误

**常见错误**:
```toml
# ❌ 错误：双引号未转义
SOME_JSON_CONFIG = "{"key":"value"}"

# ✅ 正确：使用单引号
SOME_JSON_CONFIG = '{"key":"value"}'
```

**验证命令**:
```bash
codex mcp list
```

---

## 凭证管理

凭证存储在 **独立的私有 git 仓库** `secrets/mcp.env`：

```bash
# secrets/mcp.env
GROK_API_URL='YOUR_GROK_API_URL'
GROK_API_KEY='YOUR_GROK_API_KEY'
GROK_MODEL='grok-4-fast'
TAVILY_ENABLED='true'
TAVILY_API_KEY='YOUR_TAVILY_API_KEY'
TAVILY_API_URL='https://api.tavily.com'
FIRECRAWL_API_KEY='YOUR_FIRECRAWL_API_KEY'
FIRECRAWL_API_URL='https://api.firecrawl.dev/v2'
```

#### 首次设置（自动化）

```bash
# 1. 克隆主仓库
git clone https://github.com/ly87ing/agent-manager.git
cd agent-manager

# 2. 运行 make init
make init
```

**自动化流程**：
- `make bump` / `make update` / `make init` 会在顶层检查 `secrets/` 是否存在
- `mcp:apply` 默认会准备 MCP 运行时：同步本地 secrets 仓库、按需 clone/pull MCP repo，并执行 setup/build
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
- ✅ `mcp:apply` 默认会把配置与运行时一起收敛，减少“配置已写入但运行时未准备”的半完成状态
- ✅ 凭证版本化、可审计
- ✅ 权限分离（私有仓库）

**安全实践**:
1. ✅ secrets 仓库必须设为 **private**
2. ✅ 定期轮换凭证
3. ✅ 限制访问人员
4. ❌ 不要将凭证提交到主仓库

---

## 为什么 Antigravity CLI 显示 "Disconnected"

**这是正常行为！**

Antigravity CLI 的 stdio MCP 服务器采用**按需启动**策略：
- `/mcp` 显示 "Disconnected" 表示当前没有活跃连接
- 实际使用 MCP 功能时（如调用 `web_search`），服务器会自动启动
- 使用后会自动关闭以节省资源

**验证方法**:
```bash
# 实际使用测试
agy -p "请用 web_search 查一下今天的日期"
```

---

## chrome-devtools 专项说明

### 什么是 chrome-devtools

`chrome-devtools` 是 Chrome 官方提供的 MCP server，用于让 Agent 直接连接和控制 Chrome，执行页面调试、network 观察、console 检查、性能 trace、截图和其他 DevTools 级别诊断。

### 当前维护策略

本项目默认通过以下命令受管分发给所有支持 MCP 的 Agent：

```bash
npx -y chrome-devtools-mcp@<固定版本>
```

这样把版本查询集中在 `make bump`，避免每个 Agent 在真正启动 MCP 时各自解析 `@latest`。

### 使用约束

- `chrome-devtools` 不依赖额外 secrets，占位符注入链不会为它加载专用凭证。
- `npx` 会在首次运行或本地缓存失效时解析并下载最新版本，因此应将版本波动视为上游变更，而不是本仓库内的 drift。
- 如果需要连接已运行的 Chrome、统一打开 headless、接受自签名证书或追加其他浏览器参数，应在 `config/mcp.json` 的 `args` 中集中维护，而不是在各 Agent 侧手工追加。

### 官方参考

- Chrome 官方博客：<https://developer.chrome.com/blog/chrome-devtools-mcp>
- 官方仓库：<https://github.com/mcp/chromedevtools/chrome-devtools-mcp>

---

## serena 专项说明

### 什么是 serena

`serena`（<https://github.com/oraios/serena>）是一个基于 LSP 的 MCP server，为 Agent 提供符号级代码理解与操作（`find_symbol`、`find_referencing_symbols`、`get_symbols_overview` 等），比 grep/read 更精确。stdio 按需拉起，不调用 Agent 工具时不启动，因此不引入常驻开销（这也是当年 graphify 常驻 hook 被移除的教训）。

### 当前维护策略：仅用于 JS/TS

本项目**仅把 serena 定位为 JS/TS 的语义导航工具**：

- JS/TS 走 `typescript-language-server`（serena 按需自动管理），无版本坑，只需本机有 **Node.js 18+**。
- serena 默认自带跑 server 的 JRE，首次用到某语言时自动下载对应语言服务器，需要一次联网。

### 为什么 Java 不依赖 serena

serena 的 Java 后端（solidlsp `eclipse_jdtls.py`）把 jdtls 的 `java.configuration.runtimes` **写死成单个 `JavaSE-21`（捆绑 JRE），不与用户配置合并、也没有暴露注册其它 JDK 的配置项**（上游 issue oraios/serena#1478，截至 2026 年中仍 open、未合并）。

后果：Java 8 项目会被用 JDK 21 runtime 分析——导航/改代码可用，但**不会报"Java 8 里误用高版本 API"**。因此本仓不把 Java 语义分析托管给 serena，Java 仍走 grep/read + 本地 IDE；等上游 #1478 合并支持多 runtime 后再评估接入。

### 使用约束

- `serena` 不依赖额外 secrets，`env` 为空。
- 命令固定 commit，版本波动通过 `version_lock`（`ref: main`）在 `make bump` 时集中解析写入 `config/version-pins.local.json`。
- `make init`、`make apply` 和 `./manage.sh mcp:apply` 会复用该固定版本的 Serena 配置加载器，生成或更新 `~/.serena/serena_config.yml`，并把 `project_serena_folder_location` 对齐为 `/tmp/serena$projectDir/.serena`。重复执行只在值变化时保存，并保留其他 Serena 配置；换机后重新运行任一命令即可恢复该策略。
- 例如项目根目录为 `/YOUR_WORKSPACE/YOUR_PROJECT` 时，Serena 数据会写入 `/tmp/serena/YOUR_WORKSPACE/YOUR_PROJECT/.serena`。`/tmp` 可能被系统清理，因此索引、缓存和 Serena memories 不保证持久保留。
- Serena 为兼容旧项目，会在临时目标尚不存在时回退到仓库内已有的 `.serena/`。自动配置不迁移旧数据；已有项目如需切换，应先把旧目录迁到对应临时路径或确认无需保留后再移除。
- 需要本机有可用的 `uvx`（uv）运行时；JS/TS 语义能力额外需要 Node.js。
- 各工作流启动前的「环境探测与健康自检」会打印一行 `serena (uvx:… / Node≥18:…)`，直接反映上述两个前置是否就绪（serena 是按需拉起的 MCP，不会在探测阶段真正启动 server）。

---

## 常见问题与解决方案

### Q1: Codex 报错 "unexpected key or value"

**原因**: TOML 中 JSON 字符串的双引号未正确处理

**解决**: 使用单引号包裹包含双引号的值：
```toml
# ❌ 错误
KEY = "{"nested":"value"}"

# ✅ 正确
KEY = '{"nested":"value"}'
```

---

### Q2: Antigravity CLI 配置不生效

**原因**: workspace 配置和全局配置不一致，或仍在维护旧 Gemini CLI 配置路径

**解决**: 检查并更新项目配置：
```bash
# 检查是否存在项目配置
ls -la ./.agents/mcp_config.json

# 如果存在，需要同时更新项目配置
# 或者删除项目配置使用全局配置
rm ./.agents/mcp_config.json
```

---

### Q3: MCP 服务器无法启动

**排查步骤**:

1. **检查命令是否可用**:
   ```bash
   which uvx npx
   uvx --version
   ```

2. **手动测试命令**:
   ```bash
   GROK_API_KEY='...' \
     uvx --from git+https://github.com/GuDaStudio/GrokSearch@<固定commit> grok-search
   ```

3. **检查环境变量是否正确传递**:
   - JSON 中的 JSON 字符串需要正确转义
   - 检查配置文件中的转义字符

4. **查看 Agent 日志**:
   - Claude Code: 检查对话错误信息
   - Antigravity CLI: 在 TUI 中打开 `/mcp`

---

### Q4: 凭证过期/失效

**症状**: MCP 服务器启动失败，提示认证错误

**解决**:

1. 获取新的凭证
2. 更新 `secrets/mcp.env`:
   ```bash
   # 编辑文件
   vim secrets/mcp.env
   
   # 或使用 echo
   echo 'GROK_API_KEY='\''new_token'\''' > secrets/mcp.env
   ```

3. 重新收敛配置:
   ```bash
   ./manage.sh mcp:apply
   # 或
   cd agent-mcp && python3 sync_mcp.py --config ../config/manager.json --def-file ../config/mcp.json --prepare-runtime
   ```

---

### Q6: 忘记 clone secrets 仓库

**症状**: 运行时提示找不到凭证文件

**解决**:

```bash
# 方法 1: 自动初始化（推荐）
SECRETS_REPO_URL=https://example.com/your-secrets.git make bump
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
export GROK_API_KEY='...'
uvx --from git+https://github.com/GuDaStudio/GrokSearch@<固定commit> grok-search
```

### 各 Agent 专用调试

```bash
# Claude Code
# 在对话中询问 MCP 状态

# Antigravity CLI
agy
# 在 TUI 中输入 /mcp

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
| Antigravity CLI | TUI `/mcp` 面板 |
| Codex | 终端输出 |

---

## 附录：Agent Manager MCP 同步流程

```
默认模式（mcp:apply / make apply / make update / make bump / make init 中的 MCP 步骤）:
1. 读取 config/mcp.json 获取服务器定义
   ↓
2. `make bump` / `make init` 入口先刷新 config/version-pins.local.json；`apply` 只消费套用本地 lock 后的固定版本
   ↓
3. 按固定优先级加载 MCP 凭证: root/.env → secrets/mcp.env → 系统环境变量回退
   ↓
4. 替换 ${VAR} 占位符；若真实执行时仍有未解析占位符，则直接失败
   ↓
5. 根据各 Agent 格式生成配置
   ├─ JSON 格式 (Claude/Antigravity)
   └─ TOML 格式 (Codex，带字符串转义)
   ↓
6. 校验现有配置可解析、生成结果合法、且仅删除 _MANAGED_BY=agent-manager 的托管段
   ↓
7. 原子写入各配置文件并记录 _MANAGED_BY = "agent-manager" 标记
8. 同步本地 secrets Git 仓库（非 offline 且非 dry-run）
9. clone/pull MCP repo，按需要执行 setup/build
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
