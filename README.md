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
- **仓库基线 (`manager.json`)**：保存可审计的默认策略。
- **本地覆盖 (`manager.local.json`)**：运行时与 `manager.json` 深度合并；`make ui` 只写这一层，适合机器本地偏好与私有配置。
- **模块化定义 (`agents.json`等)**：保存官方指令细节，支持 Policy 的强制覆盖。

### 2. 事务级数据安全
- **原子化写入 (Atomic Write)**：配置文件采用“临时文件原子替换”模式；规则同步中的单文件分发同样使用原子替换，目录型 rules 则采用收敛式同步。
- **内容/结构漂移审计**：实时监控物理文件与定义仓库的文件内容偏离，以及规则目录中的陈旧残留。

### 3. 极速探测引擎 (Ultra-fast Detection)
- **全局状态缓存**：审计系统启动时会一次性全量抓取 Homebrew 和 GH 的包状态并在内存中构建哈希索引，避免了对底层包管理器的频繁 I/O 调用。
- **物理路径穿透**：即使包管理器未记录，探测器也能通过精准的 `PATH` 扫描与 `.app` 目录兜底识别出系统已有的残留环境，从而达成 10 倍以上的性能提升。

### 4. 环境自愈与清理
- **一键大扫除 (Cleanup)**：精准回收受管备份、碎片及冗余日志。
- **失效洗刷 (Scrubbing)**：自动清理 Agent 目录中的死亡软链。

### 5. 安全默认值
- **安全入口**：直接执行 `./manage.sh` 只显示帮助，不触发同步。
- **终端配置默认关闭**：`sync` / `update` / `bootstrap` 默认不会改写 `~/.zshrc`、`~/.tmux.conf` 或 iTerm2 动态配置；如需启用，请在 `config/manager.local.json` 中设置 `"terminal": { "enabled": true }`。

### 6. Rules 源码与分发分层
- **可编辑源码**：跨 Agent 的共享规则片段维护在 `agent-rule/rules/src/`。
- **生成产物**：面向各 Agent 实际分发的规则文件维护在 `agent-rule/rules/dist/`，由构建脚本生成。
- **Codex 原生文件名**：Codex 全局规则产物生成为 `agent-rule/rules/dist/codex/AGENTS.md`，并同步到 `~/.codex/AGENTS.md`；其它 Agent 继续使用各自的 `main.md` / `global.md` 布局。
- **迁移清理**：Codex 同步到 `~/.codex/AGENTS.md` 时，会清理旧的 `~/.codex/rules/global.md`，避免遗留冲突规则。
- **Claude 记忆分层**：根目录 `CLAUDE.md` 只保留仓库级高频记忆，`agent-rule/CLAUDE.md` 和 `tests/CLAUDE.md` 承载目录级维护细节。
- **Claude hooks**：`config/claude-hooks.json` 生成仓库级 `.claude/settings.json`，用 `make claude-hooks-sync` 刷新 deterministic hook 配置。
- **分发清单**：`config/rules-targets.list` 只引用 `rules/dist/*` 产物，不直接引用源码片段。
- **维护入口**：修改 `agent-rule/rules/src/` 后，先执行 `make rules-build`，再执行 `make rules:sync`。

## 平台支持 (Platform Support)

- 本项目当前以 **macOS** 为主要验证平台；**Linux** 路径保持 POSIX 兼容设计，但仍建议在目标环境自行回归验证。  
- 在 **Windows** 上，建议使用 **WSL2、Git‑Bash 或 Cygwin** 等提供 POSIX 兼容层的环境，以获得完整功能。  
- 若希望在原生 PowerShell 或 CMD 中直接使用，需要对脚本进行额外的跨平台改造（将外部命令替换为 PowerShell 等价实现），此工作尚未完成。

## 管理指令集

- `./manage.sh`: 显示帮助；这是脚本的安全默认入口。
- `make sync`: 仅同步当前配置到 rules / skills / MCP，本地收敛，不联网、不安装。
- `make update`: 更新远端资源并收敛本地环境，但不安装缺失 Agent。
- `make bootstrap`: 执行全量安装、更新与同步，适合新机器初始化。
- `make all`: `make bootstrap` 的兼容别名。
- `make plan`: 预演一次完整 `bootstrap`。
- `make self-update`: 仅更新 agent-manager 仓库自身逻辑。
- `make smoke`: 运行跨平台安全预演 smoke 校验，基于最小策略文件 `config/manager.smoke.json` 验证主入口命令的 dry-run / preview 契约。
- `make eval-rules`: 运行离线规则效果评测，读取 `scripts/evals/fixtures/` 中的固定 JSON 样例并输出稳定的机器可读摘要。
- `make doctor` / `make status`: 执行全系统深度体检，并把 warning 细分为 `可选工具未就绪` 与 `托管产物尚未生成` 两类提示项；追加 `--json` 可输出机器可读诊断报告。
- `make rules-build`: 从 `agent-rule/rules/src/` 生成 `agent-rule/rules/dist/` 下的可分发规则产物。
- `make rules:sync`: 将 `agent-rule/rules/dist/` 中的规则产物分发到各 Agent 规则目录。
- `make fetch-models`: 探测并获取 API 端点支持的模型列表 (支持 OpenAI / Anthropic 格式)。
- `make restore`: 交互式回滚历史快照。
- `make ui`: 启动交互式配置管理中心。

## Repo-native Guidance

- 根目录 `AGENTS.md` 提供仓库级的 setup、测试命令、done-when 规则与生成文件边界。
- `agent-rule/AGENTS.md` 只覆盖 `agent-rule/rules/src/` 到 `agent-rule/rules/dist/` 的编辑和重建流程。
- `tests/AGENTS.md` 只覆盖测试命令、fixtures 和 contract-test 的写法。

## Secrets 初始化

- 不再内置固定的 secrets 仓库地址。
- 需要通过环境变量 `SECRETS_REPO_URL` 或脚本参数显式提供仓库地址，例如：

```bash
SECRETS_REPO_URL=https://example.com/your-secrets.git make update
# 或
./scripts/init-secrets.sh https://example.com/your-secrets.git
```

## 许可证
Apache-2.0
