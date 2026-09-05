# MCP 配置完全指南

> 本文档记录了在配置 MCP (Model Context Protocol) 服务器过程中遇到的所有问题和解决方案。

---

## 目录

1. [快速开始](#快速开始)
2. [各 Agent 的 MCP 配置格式](#各-agent-的-mcp-配置格式)
3. [当前仓库默认受管 MCP 服务器](#当前仓库默认受管-mcp-服务器)
4. [凭证管理](#凭证管理)
5. [chrome-devtools 专项说明](#chrome-devtools-专项说明)
6. [常见问题与解决方案](#常见问题与解决方案)
7. [调试技巧](#调试技巧)

---

## 快速开始

### 自动配置（推荐）

使用 Agent Manager 自动同步所有 Agent 的 MCP 配置：

```bash
# 全量同步（包含 Agent 安装、资源更新和 MCP 配置）
make init

# 本地环境收敛（会更新受管 MCP 资源并准备运行时，MCP 注入随该管线执行）
make update
```

### 手动配置

如果自动配置失败，参考下文各 Agent 的配置格式手动添加。

---

## 当前仓库默认受管 MCP 服务器

| Server | 用途 | 受管命令 | 凭证 | 默认策略 |
|--------|------|----------|------|----------|
| `chrome-devtools` | Chrome DevTools 浏览器调试、性能分析、network / console / screenshot | `chrome-devtools-mcp`（运行时由 `tools.json` 纳管，npm 全局安装、不锁版本） | 无 | 启用 |
| `grok-search` | web_search / web_fetch 联网搜索与网页抓取 | `uvx --from git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily grok-search` | `GROK_API_URL`, `GROK_API_KEY`, `GROK_MODEL`, `TAVILY_ENABLED`, `TAVILY_API_KEY`, `TAVILY_API_URL`, `FIRECRAWL_API_KEY`, `FIRECRAWL_API_URL` | 启用 |
| `serena` | JS/TS 语义级代码导航（LSP：find_symbol / 引用 / 符号概览） | `uvx --from git+https://github.com/oraios/serena@main serena start-mcp-server`，按客户端追加 context 参数 | 无 | 启用 |

**维护约定**:
- 受管 MCP 不锁版本，但版本生命周期按运行时类型各有归属：`chrome-devtools` 的二进制 `chrome-devtools-mcp` 由 `config/tools.json` 纳管为 npm 全局工具，`make update` 在 tools 同步步骤中探测 npm 上游并升级；`grok-search` / `serena` 为 uvx git 分支源（`grok-with-tavily` / `main`），`make update` 的 MCP 步骤以 `uvx --refresh` 预热缓存对齐分支头。
- `config/mcp.json` 只保存接线定义（命令、参数、凭证占位符、agent 差异），`make update` / `make init` 按其中的定义收敛本机配置；版本波动视为上游变更。
- `chrome-devtools` 需要本机有可用的 Node.js / npm 运行时（供 tools 同步安装 `chrome-devtools-mcp`）；未安装时 doctor 的 Tools 就绪审计会报错。
- `chrome-devtools` 的 Codex 启动超时按 120 秒维护，覆盖默认 30 秒，避免冷启动时被误判为 MCP 启动失败。
- `make doctor` 会报告各启用中 MCP server 的运行时供给状态：`chrome-devtools` 显示实装版本，uvx git 型显示所跟踪分支与缓存刷新入口；实际命令以 `config/mcp.json` 中的定义为准。
- 如果后续需要统一增加 `--browserUrl`、`--headless`、`--isolated` 等参数，应修改 `config/mcp.json`，不要手改单个 Agent 的 MCP 配置文件。
- 本项目不管理 GitHub Copilot，因此不会再向其配置目录注入托管 MCP，也不会再生成 Copilot 专属 repo-native rules 文件。

---

## 各 Agent 的 MCP 配置格式

**关键发现**：不同 Agent 使用不同的 MCP 配置格式！这是最大的坑。

### 格式对比表

| Agent | 配置文件路径 | 根键名 | command 格式 | 环境变量键 | 特殊要求 |
|-------|-------------|--------|-------------|-----------|----------|
| **Claude Code** | `~/.claude.json` | `mcpServers` | 字符串 + args 数组 | `env` | - |
| **Codex** | `~/.codex/config.toml` | `[mcp_servers.*]` | 字符串 + args 数组 | `[mcp_servers.*.env]` | TOML 字符串转义 |

---

### Claude Code

**配置文件**: `~/.claude.json`

```json
{
  "mcpServers": {
    "grok-search": {
      "command": "uvx",
      "args": ["--from", "git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily", "grok-search"],
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

### Codex ⚠️

**配置文件**: `~/.codex/config.toml`

```toml
[mcp_servers.grok-search]
command = "uvx"
args = ["--from", "git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily", "grok-search"]
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
- `make update` / `make init` 会在顶层检查 `secrets/` 是否存在
- MCP 注入随 `make update` 管线执行，默认会准备 MCP 运行时：同步本地 secrets 仓库、按需 clone/pull MCP repo，并执行 setup/build
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
- ✅ `make update` 默认会把配置与运行时一起收敛，减少“配置已写入但运行时未准备”的半完成状态
- ✅ 凭证版本化、可审计
- ✅ 权限分离（私有仓库）

**安全实践**:
1. ✅ secrets 仓库必须设为 **private**
2. ✅ 定期轮换凭证
3. ✅ 限制访问人员
4. ❌ 不要将凭证提交到主仓库

---

## chrome-devtools 专项说明

### 什么是 chrome-devtools

`chrome-devtools` 是 Chrome 官方提供的 MCP server，用于让 Agent 直接连接和控制 Chrome，执行页面调试、network 观察、console 检查、性能 trace、截图和其他 DevTools 级别诊断。

### 当前维护策略

`chrome-devtools` 的运行时二进制 `chrome-devtools-mcp` 由 `config/tools.json` 纳管为 npm 全局工具（安装/升级命令不锁版本），`config/mcp.json` 只保留接线定义，直连已安装的二进制：

```bash
chrome-devtools-mcp --isolated --no-usage-statistics
```

版本生命周期归 `agent-tool/sync_tools.py`：`make update` / `make init` 时用 `npm list -g` 对比 npm 上游版本，落后才执行 `npm install -g chrome-devtools-mcp` 升级；探测失败默认跳过，可用 `--force-refresh` 强制。这样 MCP 版本升级有明确入口、doctor 可观测实装版本，且 Agent 启动 MCP 不再依赖 npx 运行时解析缓存。

### 使用约束

- `chrome-devtools` 不依赖额外 secrets，占位符注入链不会为它加载专用凭证。
- 版本升级由 tools 同步显式完成，因此应将版本波动视为上游变更，而不是本仓库内的 drift。
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
- Serena 的 context 会改变系统提示词和可用工具，必须与客户端匹配。公共启动参数由 `args` 维护，`extra_args_by_agent` 再按客户端追加以下参数；同步器与 doctor 使用同一套解析规则：

  | 客户端 | 追加参数 |
  |--------|----------|
  | Claude Code | `--context claude-code --project-from-cwd` |
  | Codex | `--context codex --project-from-cwd` |

- 命令跟踪上游 `main` 分支最新（`git+https://github.com/oraios/serena@main`），不固定 commit。
- `make init` 和 `make update` 会复用 Serena 配置加载器，生成或更新 `~/.serena/serena_config.yml`，并把 `project_serena_folder_location` 对齐为 `/tmp/serena$projectDir/.serena`。重复执行只在值变化时保存，并保留其他 Serena 配置；换机后重新运行任一命令即可恢复该策略。
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

### Q2: MCP 服务器无法启动

**排查步骤**:

1. **检查命令是否可用**:
   ```bash
   which uvx chrome-devtools-mcp
   uvx --version
   ```

2. **手动测试命令**:
   ```bash
   GROK_API_KEY='...' \
     uvx --from git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily grok-search
   ```

3. **检查环境变量是否正确传递**:
   - JSON 中的 JSON 字符串需要正确转义
   - 检查配置文件中的转义字符

4. **查看 Agent 日志**:
   - Claude Code: 检查对话错误信息
   - Codex: 检查终端输出或运行 `codex mcp list`

---

### Q3: 凭证过期/失效

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
   make update
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
export GROK_API_KEY='...'
uvx --from git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily grok-search
```

### 各 Agent 专用调试

```bash
# Claude Code
# 在对话中询问 MCP 状态

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
| Codex | 终端输出 |

---

## 附录：Agent Manager MCP 同步流程

```
默认模式（make update / make init 中的 MCP 步骤）:
1. 读取 config/mcp.json 获取服务器定义（命令不锁版本，跟随上游最新）
   ↓
2. 按固定优先级加载 MCP 凭证: root/.env → secrets/mcp.env → 系统环境变量回退
   ↓
3. 替换 ${VAR} 占位符；若真实执行时仍有未解析占位符，则直接失败
   ↓
4. 根据各 Agent 格式生成配置
   ├─ JSON 格式 (Claude Code)
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
