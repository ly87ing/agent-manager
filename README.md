# Agent Manager (Ultimate Edition)

🚀 **AI 助手环境一键管理中枢**。通过单一入口，对 `Claude Code` 与 `Codex` 做 Agent 生命周期、MCP、skills 和 rules 管理，并对 `Kimi Code` 做 MCP 与 skills 管理（不接管其生命周期与 rules）。退役 Agent 不再保留受管路径或分发逻辑。

## 核心规范速查表 (Authoritative Spec)

本项目严格遵循各 Agent 官方最新规范，确保配置注入的精准与安全：

```text
   ┌────────────────┬─────────────────────────┬──────────┬─────────────┐
   │ Agent 名称     │ 官方标准路径            │ 文件格式 │ 核心键名    │
   ├────────────────┼─────────────────────────┼──────────┼─────────────┤
   │ Claude Code    │ ~/.claude.json          │ JSON     │ mcpServers  │
   │ Codex          │ ~/.codex/config.toml    │ TOML     │ mcp_servers │
   │ Kimi Code      │ ~/.kimi-code/mcp.json   │ JSON     │ mcpServers  │
   └────────────────┴─────────────────────────┴──────────┴─────────────┘
```

## 受管运行组件（当前默认）

### MCP 服务器

- `chrome-devtools`
  - 用途：`Chrome DevTools` 调试、网络、性能分析。
  - 启动：`npx -y chrome-devtools-mcp@<固定版本> --isolated`（避免多个 agent 共享同一个 browser profile 导致互相污染）。
- `grok-search`
  - 用途：联网搜索与网页抓取。
  - 注意：真实 provider key 仅从 `secrets/` 注入。
- `serena`
  - 用途：**仅用于 JS/TS** 的 LSP 语义导航（`find_symbol` / 引用 / 符号概览）；需本机有 `uvx` 与 Node.js。
  - 配置：`make init`、`make apply` 和 `./manage.sh mcp:apply` 会把每个项目的 Serena 数据目录对齐到系统临时目录，避免在仓库内生成 `.serena/`；换机后重新执行即可生效。
  - 注意：Java 不托管给它（上游 `oraios/serena#1478` 未支持按项目注册 JDK runtime，Java 8 会被 JDK21 runtime 分析）。详见 `docs/MCP-CONFIG-GUIDE.md`。

### 其他受管工具

- `chrome-devtools`（MCP）
  - 由 `make bump` 刷新本机 `config/version-pins.local.json` 中的版本锁；各 Agent 实际运行时只消费固定版本。
默认受管 skill 仓库包括 `agent-skills` 与 `herdr`；仓库、升级策略和初始 commit 保留在 `config/skills.json`，真实运行 commit 由 `config/version-pins.local.json` 的本地锁覆盖。运行时同步只检出固定 commit，不直接跟随默认分支或动态 release。`agent-skills` 由 `make bump` 按 `latest-branch` 刷新；`agent-skills` 面向 `Claude Code`、`Codex` 与 `Kimi Code` 暴露，并通过显式 `skill_paths` 锁定为当前声明的 `safe-merge-review`、`change-discipline`、`frontend-verification`、`artifact-hygiene`、`reader-facing-writing` 这 5 个 skill 包。`herdr` 按 `latest-release` 刷新，向 `Claude Code`、`Codex` 与 `Kimi Code` 暴露官方 `skills/herdr`。两者都不会因为上游新增 skill 而自动扩大安装范围。
默认自动安装的 Agent 当前只包含 `Claude Code` 与 `Codex`。
`Claude Code` 与 `Codex` 默认走官方 npm 包，并使用 `config/agents.json` 中的命令模板加本机版本锁。历史 `brew` 安装仅用于识别并提示迁移，不再作为这些 CLI 的受管 provider。
`Claude Code native` 仍作为兼容 provider 保留官方 install / update / uninstall 语义，但默认不再使用这条动态 updater 路径。
`Claude Code local install` 不纳入 `agent-manager` lifecycle 托管，因为它依赖 shell alias 暴露入口，当前探测链路不会越权接管。
以上 lifecycle 口径统一按两个当前受管 Agent 的官方推荐方式维护。`Claude Code` 和 `Codex` 的 npm provider 使用 `config/agents.json` 中的命令模板，并由 `make bump` 刷新本机 `config/version-pins.local.json`（然后由 `make agents:apply` 或 `make init` 负责更新软件）。`agent-manager` 不改写用户 shell profile。

### Herdr 会话管理

`config/herdr.json` 保存 [Herdr 官方安装器](https://herdr.dev/install.sh)与集成目标，`config/manager.json` 保存默认启用策略。`apply` / `init` 缺少 Herdr CLI 时才调用官方安装器；已有 CLI 不会被静默升级。随后通过 `herdr integration install claude|codex` 幂等收敛官方会话集成，并由状态回读确认版本为 `current`。`agent-manager` 不会自动启动或停止 Herdr 服务，也不改写 Herdr 自身运行参数。

官方 Herdr Skill 固定到已发布 release 的 commit，并继续使用本机当前 Codex 实际读取的 `~/.codex/skills`；Claude Code 使用 `~/.claude/skills`，Kimi Code 使用 `~/.kimi-code/skills`（Kimi Code 仅作 skills / MCP target，不经 `agent-manager` 安装或升级）。可单独运行 `make herdr:apply` 收敛 CLI 与集成，再运行 `make skills:apply` 收敛 Skill。已打开的 Agent 进程不会被追溯注入，需从 Herdr 发起新会话后使用。

## Official CLI Lifecycle Policy

| CLI | 默认受管 provider | 兼容受管 provider | detect-only 历史来源 | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| `Claude Code` | `npm` | `native` | `brew` | 默认使用官方 npm 包固定版本；native installer 仅作为兼容 provider 保留。 |
| `Codex` | `npm` | - | `brew` | 使用官方 npm 包；安装和升级命令固定到 `config/agents.json` 的版本锁，历史 brew 安装只用于审计和迁移提示。 |

## Agent 安装源依据 (Official Channels Catalog)

下表列出上游公开安装渠道，但不等于 `agent-manager` 会默认或继续托管这些渠道；对 `Claude Code` 与 `Codex` 的实际受管策略以上一节为准。

| 助手名称 | 二进制标识 | 官方 npm / 安装器 | Homebrew 官方源 (macOS) | 备注 |
| :--- | :--- | :--- | :--- | :--- |
| **Claude Code** | `claude` | `@anthropic-ai/claude-code` / 官方 native installer | `cask: claude-code` | 受管 lifecycle 默认走 npm 固定版本；`brew` 仅保留 detect-only 语义。 |
| **Codex** | `codex` | `@openai/codex` | `cask: codex` | 受管 lifecycle 默认走 npm。 |

## 核心健全性特性

### 1. 策略与定义分离 (Policy vs. Definition)
- **仓库基线 (`manager.json`)**：保存可审计的默认策略。
- **本地覆盖 (`manager.local.json`)**：运行时与 `manager.json` 深度合并；`make ui` 只写这一层，适合机器本地偏好与私有配置。
- **版本锁 (`version_lock`) / skill commit**：`config/agents.json`、`config/tools.json`、`config/mcp.json` 与 `config/skills.json` 保存命令模板、包名、仓库和升级策略；`make bump` 查询 npm / Git ref / skill 仓库分支的最新版本并写入本机 `config/version-pins.local.json`。同步到 Codex / Claude Code 的实际运行配置会先套用本地 lock，不得使用 `@latest`、分支名或未固定的 skill 仓库。
- **Herdr 默认策略**：`herdr.enabled`、`herdr.auto_install` 与 `herdr.integrations` 控制 CLI 缺失安装和 Claude Code / Codex 官方集成；禁用策略不会擅自卸载用户已有集成。
- **浏览器前置开关**：`browser_us_sync.enabled` 控制 `apply` / `init` / `browser:apply` 是否执行浏览器 Google US 前置同步。
- **模块化定义 (`agents.json`等)**：保存官方指令细节，支持 Policy 的强制覆盖。

### 2. 事务级数据安全
- **原子化写入 (Atomic Write)**：配置文件采用“临时文件原子替换”模式；规则同步中的单文件分发同样使用原子替换，目录型 rules 则采用收敛式同步。
- **内容/结构漂移审计**：实时监控物理文件与定义仓库的文件内容偏离，以及规则目录中的陈旧残留。

### 3. 极速探测引擎 (Ultra-fast Detection)
- **全局状态缓存**：审计系统启动时会一次性全量抓取 Homebrew 和 GH 的包状态并在内存中构建哈希索引，避免了对底层包管理器的频繁 I/O 调用。
- **物理路径穿透**：即使包管理器未记录，探测器也能通过精准的 `PATH` 扫描与 `.app` 目录兜底识别出系统已有的残留环境，从而达成 10 倍以上的性能提升。

### 4. 环境自愈与清理
- **一键大扫除 (Cleanup)**：精准回收受管备份、碎片及冗余日志，仅覆盖当前声明的 Claude Code 与 Codex 目标。
- **失效洗刷 (Scrubbing)**：自动清理 Agent 目录中的死亡软链。

### 5. 安全默认值
- **安全入口**：直接执行 `./manage.sh` 只显示帮助，不触发同步；只给全局 flag 不给命令（如 `./manage.sh --offline`）会报错退出，不会隐式跑 `init`。
- **不接管终端配置**：除 shell alias 的受管区块（marker 圈定、只写区块内）会同步到 `~/.zshrc` 与 `~/.zprofile` 外，`agent-manager` 不维护 `~/.tmux.conf`、iTerm2 动态配置等用户级终端文件，避免把机器/个人偏好写死后扩散到其他电脑。
- **主仓库禁存 secrets**：真实凭证只允许存在于独立的 `secrets/` 私有仓库或系统环境变量中；`make secrets-check`、`make smoke`、CI 与 `make doctor` 会扫描主仓库文件并阻止明文敏感信息落入 `agent-manager`。

### 6. Rules 源码与分发分层
- **可编辑源码**：跨 Agent 的共享规则片段只维护在 `agent-rule/rules/src/`，这是仓库内唯一可编辑的 rule source。
- **共享 core 职责**：`agent-rule/rules/src/shared/core.md` 只放所有 Agent 都应始终生效的短规则；这里优先约束“非平凡或有歧义任务先显式说明假设、优先最简单本地改动、保持 surgical edits、按 verification point 推进”。
- **生成产物**：面向各 Agent 实际分发的规则文件只维护在 `agent-rule/rules/dist/`，由构建脚本生成；仓库内不再保留第二套 `rules/global/*` source tree。
- **按 Agent 原生格式分发**：Codex 生成 `AGENTS.md`，Claude 生成 `CLAUDE.md`。
- **低频专题进 skill**：原 `architecture`、`style`、`frontend`、`artifact-hygiene`、`writing` 等低频专题细节不再通过 `modules/` 分发，而是在 `agent-skills` 中维护为 `change-discipline`、`frontend-verification`、`artifact-hygiene`、`reader-facing-writing`。
- **分层原则**：always-on 规则只保留触发条件、协作边界和最小行为约束；执行步骤、检查清单、失败边界和验证细节进入 skill；Agent 私有差异只放 `agent-rule/rules/src/agents/*/adapter.md`，不要把这些内容回灌进 shared core。
- **迁移清理**：原生分发会清理旧的 `~/.<agent>/rules/global.md` 和旧 `rules/modules` 残留，避免新旧入口并存。
- **Claude 记忆分层**：根目录 `CLAUDE.md` 只保留仓库级高频记忆，`agent-rule/CLAUDE.md` 和 `tests/CLAUDE.md` 承载目录级维护细节。
- **Claude hooks**：`config/claude-hooks.json` 生成仓库级 `.claude/settings.json`，用 `make claude:hooks:apply` 刷新 deterministic hook 配置。
- **Codex hooks**：`config/codex-hooks.json` 生成仓库级 `.codex/hooks.json`，用 `make codex:hooks:apply` 刷新 deterministic Codex hook 配置；当前包含 `PreToolUse` 的生成产物写保护和 `Stop` 的规则构建一致性校验。
- **指令质量评测**：`make eval-rules` 现在同时输出任务成功率、verification、skill exposure 和 always-on instruction budget 指标，并强制校验 `min_case_count`、`required_agents`、`success / verification 阈值`。
- **分发清单**：`config/rules-targets.list` 只引用 `rules/dist/*` 产物，不直接引用源码片段。
- **维护入口**：修改 `agent-rule/rules/src/` 后，先执行 `make rules-build`，再执行 `make rules:apply`。

## 平台支持 (Platform Support)

- 本项目当前以 **macOS** 为主要验证平台；**Linux** 路径保持 POSIX 兼容设计，但仍建议在目标环境自行回归验证。  
- 在 **Windows** 上，建议使用 **WSL2、Git‑Bash 或 Cygwin** 等提供 POSIX 兼容层的环境，以获得完整功能。  
- 若希望在原生 PowerShell 或 CMD 中直接使用，需要对脚本进行额外的跨平台改造（将外部命令替换为 PowerShell 等价实现），此工作尚未完成。

## 管理指令集

- `./manage.sh`: 显示帮助（只列日常入口 + 进阶原语）；这是脚本的安全默认入口。完整命令清单执行 `./manage.sh help:all` 或 `make help:all`。
- `make update`: 日常最常用。把本机更新并收敛到最新（= `bump` + `apply`，secrets 只拉一次）；不安装缺失 Agent。
- `make apply`: 把当前锁定态分发/收敛到本机（rules 构建分发、skills 链接、mcp 注入、tools、Herdr、browser、shell alias、secrets 拉取）；不刷新版本锁、不安装 Agent。

- `make bump`: 纯粹刷新版本锁与 skill commit（更新元数据到锁定文件），不执行本地环境应用。
- `make agents:apply`: 按 `config/version-pins.local.json` 里的**当前版本锁**升级/修复本地已安装 Agent，不强制安装缺失 Agent。它自身**不刷新版本锁**，所以锁没变时通常是幂等空转；要升到上游最新，先跑 `make bump`（或 `make update`）刷锁再跑本命令，或直接用 `make init` 一步到位。
- `make init`: 执行全量安装、更新与同步（= `bump` + Agent 生命周期 + `apply` + 失效引用审计），并等待 Agent 生命周期安装/升级完成，适合新机器初始化。
- `make plan`: 预演一次完整 `init`。
- `make sys:upgrade`: 仅更新 agent-manager 仓库自身逻辑。
- `make smoke`: 运行跨平台安全预演 smoke 校验，基于最小策略文件 `config/manager.smoke.json` 验证主入口命令的 dry-run / preview 契约。
- `make secrets-check`: 扫描主仓库受控文件，拒绝真实 token / 私钥等明文敏感信息进入 `agent-manager`；`secrets/` 私有仓库和忽略缓存不在扫描范围内。
- `make eval-rules`: 运行离线规则效果评测，读取 `scripts/evals/fixtures/` 中的固定 JSON 样例并输出稳定的机器可读摘要。
- `make instructions:check`: 共享指令、skills exposure 和项目内 hooks 变更前的统一回归门（硬门禁）；会串行跑 instruction-related project contracts、rules budget/layout、skill exposure、rules sync / Claude/Codex managed hooks 契约、eval schema/metric 契约，以及 baseline 规则评测。
- `make doctor` / `make status`: 执行全系统深度体检，并把 warning 细分为 `可选工具未就绪` 与 `托管产物尚未生成` 两类提示项；同时扫描主仓库中是否存在疑似明文敏感信息。追加 `--json` 可输出机器可读诊断报告。
- `make rules-build`: 从 `agent-rule/rules/src/` 生成 `agent-rule/rules/dist/` 下的可分发规则产物。
- `make rules:apply`: 将 `agent-rule/rules/dist/` 中的规则产物分发到各 Agent 原生入口。
- `make herdr:apply`: 缺失时安装 Herdr CLI，并幂等收敛 Claude Code / Codex 官方会话集成；不自动升级已有 CLI，不启动服务。
- `make codex:hooks:apply`: 刷新仓库级 `.codex/hooks.json`。
- `make fetch-models`: 探测并获取 API 端点支持的模型列表 (支持 OpenAI / Anthropic 格式)。
- `make state:restore`: 交互式回滚历史快照。
- `make ui`: 启动交互式配置管理中心。

## Repo-native Guidance

- 根目录 `AGENTS.md` 提供仓库级的 setup、测试命令、done-when 规则与生成文件边界。
- `agent-rule/AGENTS.md` 只覆盖 `agent-rule/rules/src/` 到 `agent-rule/rules/dist/` 的编辑和重建流程。
- `tests/AGENTS.md` 只覆盖测试命令、fixtures 和 contract-test 的写法。

## Secrets 初始化

- 不再内置固定的 secrets 仓库地址。
- 真实的 `Tailscale` / `SSH` / 远程访问信息也必须只存放在外部私有 `secrets/` 仓库；当前仓库仅允许占位符和通用说明。
- 需要通过环境变量 `SECRETS_REPO_URL` 或脚本参数显式提供仓库地址，例如：

```bash
SECRETS_REPO_URL=https://example.com/your-secrets.git make bump
# 或
./scripts/init-secrets.sh https://example.com/your-secrets.git
```

## 许可证
Apache-2.0
