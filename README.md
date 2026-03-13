# Agent Manager (Ultimate Edition)

🚀 **AI 助手环境一键管理中枢**。通过单一入口，实现对 Claude Code, Gemini CLI, Cursor, GitHub Copilot, Codex, OpenCode 的全生命周期管理。

## 核心规范速查表 (Authoritative Spec)

本项目严格遵循各 Agent 官方最新规范，确保配置注入的精准与安全：

```text
  ┌────────────────┬──────────────────────────────────┬──────────┬─────────────┐
  │ Agent 名称     │ 官方标准路径                     │ 文件格式 │ 核心键名    │
  ├────────────────┼──────────────────────────────────┼──────────┼─────────────┤
  │ Claude Code    │ ~/.claude.json                   │ JSON     │ mcpServers  │
  │ Gemini CLI     │ ~/.gemini/settings.json          │ JSON     │ mcpServers  │
  │ Cursor         │ ~/.cursor/mcp.json               │ JSON     │ mcpServers  │
  │ GitHub Copilot │ ~/.copilot/mcp-config.json       │ JSON     │ mcpServers  │
  │ OpenCode       │ ~/.config/opencode/opencode.json │ JSON     │ mcp         │
  │ Codex          │ ~/.codex/config.toml             │ TOML     │ mcp_servers │
  └────────────────┴──────────────────────────────────┴──────────┴─────────────┘
```

## Agent 安装源依据 (Official Evidence)

本项目的生命周期管理逻辑严格对齐以下官方发布渠道：

| 助手名称 | 二进制标识 | NPM 官方源 (Primary) | Homebrew 官方源 (macOS) | 官方依据 (Proof) |
| :--- | :--- | :--- | :--- | :--- |
| **Claude Code** | `claude` | `@anthropic-ai/claude-code` | `cask: claude-code` | [Brew Cask Info](https://formulae.brew.sh/cask/claude-code) |
| **Gemini CLI** | `gemini` | `@google/gemini-cli` | `formula: gemini-cli` | [Brew Core Info](https://formulae.brew.sh/formula/gemini-cli) |
| **Cursor IDE** | `cursor` | - | `cask: cursor` | [Brew Cask Info](https://formulae.brew.sh/cask/cursor) |
| **Copilot CLI** | `copilot` | - | `cask: copilot-cli` | [Brew Cask Info](https://formulae.brew.sh/cask/copilot-cli) |
| **OpenCode** | `opencode` | `@opencode/cli` | `tap: anomalyco/tap/opencode` | [Official GitHub](https://github.com/anomalyco/opencode) |

## 核心健全性特性

### 1. 策略与定义分离 (Policy vs. Definition)
- **统一意愿清单 (`manager.json`)**：作为唯一决策入口，控制 `enabled`、`auto_install`、`auto_update` 以及 `preferred_provider`。
- **模块化定义 (`agents.json`等)**：保存官方指令细节，支持 Policy 的强制覆盖。

### 2. 事务级数据安全
- **原子化写入 (Atomic Write)**：所有配置文件及规则文件均采用“临时文件原子替换”模式，杜绝损坏风险。
- **MD5 漂移审计**：实时监控物理文件与定义仓库的内容偏离。

### 3. 环境自愈与清理
- **一键大扫除 (Cleanup)**：精准回收备份、碎片、孤儿仓库及冗余日志。
- **失效洗刷 (Scrubbing)**：自动清理 Agent 目录中的死亡软链。

## 管理指令集

- `make all`: 一键执行全量环境同步与健康认证。
- `make doctor` / `make status`: 执行全系统深度体检。
- `make restore`: 交互式回滚历史快照。
- `make ui`: 启动交互式配置管理中心。

## 许可证
Apache-2.0
