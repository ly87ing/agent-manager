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
  - 启动：`chrome-devtools-mcp --isolated --no-usage-statistics`（运行时二进制由 `tools.json` 纳管、npm 全局安装且不锁版本，`make update` 探测并升级到上游最新；`--isolated` 避免多个 agent 共享同一个 browser profile 导致互相污染；`--no-usage-statistics` 关闭上游默认开启的使用统计上报）。
- `grok-search`
  - 用途：联网搜索与网页抓取。
  - 运行时：`uvx --from git+...@grok-with-tavily` 跟踪上游分支，`make update` 以 `uvx --refresh` 预热缓存使其对齐最新。
  - 注意：真实 provider key 仅从 `secrets/` 注入。
- `serena`
  - 用途：**仅用于 JS/TS** 的 LSP 语义导航（`find_symbol` / 引用 / 符号概览）；需本机有 `uvx` 与 Node.js。
  - 配置：`make init` 与 `make update` 会把每个项目的 Serena 数据目录对齐到系统临时目录，避免在仓库内生成 `.serena/`；换机后重新执行即可生效。
  - 注意：Java 不托管给它（上游 `oraios/serena#1478` 未支持按项目注册 JDK runtime，Java 8 会被 JDK21 runtime 分析）。详见 `docs/MCP-CONFIG-GUIDE.md`。

### 其他受管工具

- `chrome-devtools-mcp`（chrome-devtools MCP 的运行时二进制）
  - 不锁版本；由 `tools.json` 纳管为 npm 全局工具，`make update` 探测 npm 上游并升级（探测失败默认跳过，可 `--force-refresh` 强制）。
- `playwright-cli`
  - 不锁版本；同为 `tools.json` 纳管的 npm 全局工具，浏览器运行时由 `playwright-cli install-browser` 初始化。
默认受管 skill 仓库为 `agent-skills`，跟随上游 `main` 分支最新（不固定 commit），由 `make update` 同步 clone/pull。`agent-skills` 面向 `Claude Code`、`Codex` 与 `Kimi Code` 暴露，并通过显式 `skill_paths` 锁定为当前声明的 `safe-merge-review`、`change-discipline`、`frontend-verification`、`artifact-hygiene`、`reader-facing-writing`、`technical-diagramming` 这 6 个 skill 包，不会因为上游新增 skill 而自动扩大安装范围。
默认自动安装的 Agent 当前只包含 `Claude Code` 与 `Codex`。
`Claude Code` 与 `Codex` 默认走官方 npm 包，`config/agents.json` 中的安装/升级命令刻意不锁版本，`make agents:update` 每次都升级到上游最新。历史 `brew` 安装仅用于识别并提示迁移，不再作为这些 CLI 的受管 provider。
`Claude Code native` 仍作为兼容 provider 保留官方 install / update / uninstall 语义，但默认不再使用这条动态 updater 路径。
`Claude Code local install` 不纳入 `agent-manager` lifecycle 托管，因为它依赖 shell alias 暴露入口，当前探测链路不会越权接管。
以上 lifecycle 口径统一按两个当前受管 Agent 的官方推荐方式维护。`Claude Code` 和 `Codex` 的 npm provider 使用 `config/agents.json` 中的命令模板（不锁版本），由 `make agents:update` 或 `make init` 直接升级到上游最新。`agent-manager` 不改写用户 shell profile。

## Official CLI Lifecycle Policy

| CLI | 默认受管 provider | 兼容受管 provider | detect-only 历史来源 | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| `Claude Code` | `npm` | `native` | `brew` | 默认使用官方 npm 包（不锁版本，随升级命令取上游最新）；native installer 仅作为兼容 provider 保留。 |
| `Codex` | `npm` | - | `brew` | 使用官方 npm 包；安装和升级命令不锁版本、始终取上游最新，历史 brew 安装只用于审计和迁移提示。 |

## Agent 安装源依据 (Official Channels Catalog)

下表列出上游公开安装渠道，但不等于 `agent-manager` 会默认或继续托管这些渠道；对 `Claude Code` 与 `Codex` 的实际受管策略以上一节为准。

| 助手名称 | 二进制标识 | 官方 npm / 安装器 | Homebrew 官方源 (macOS) | 备注 |
| :--- | :--- | :--- | :--- | :--- |
| **Claude Code** | `claude` | `@anthropic-ai/claude-code` / 官方 native installer | `cask: claude-code` | 受管 lifecycle 默认走 npm（不锁版本）；`brew` 仅保留 detect-only 语义。 |
| **Codex** | `codex` | `@openai/codex` | `cask: codex` | 受管 lifecycle 默认走 npm。 |

## 核心健全性特性

### 1. 策略与定义分离 (Policy vs. Definition)
- **仓库基线 (`manager.json`)**：保存可审计的默认策略。
- **本地覆盖 (`manager.local.json`)**：运行时与 `manager.json` 深度合并，适合机器本地偏好与私有配置（手工编辑维护）。
- **不锁版本**：所有受管组件（Agent、tools、MCP、skills）都跟随上游最新——npm 包命令不带版本号、git 依赖跟踪分支头、skill 仓库跟随默认分支；`make update` 即把本机收敛到上游当前状态（MCP 中 npm 型运行时由 tools 纳管升级，git 分支型由管线 `uvx --refresh` 预热缓存）。
- **浏览器前置开关**：`browser_us_sync.enabled` 控制 `update` / `init` 是否执行浏览器 Google US 前置同步。
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
- **主仓库禁存 secrets**：真实凭证只允许存在于独立的 `secrets/` 私有仓库或系统环境变量中；`scripts/check_no_secrets.py`、`make smoke`、CI 与 `make doctor` 会扫描主仓库文件并阻止明文敏感信息落入 `agent-manager`。

### 6. Rules 源码与分发分层
- **可编辑源码**：跨 Agent 的共享规则片段只维护在 `agent-rule/rules/src/`，这是仓库内唯一可编辑的 rule source。
- **共享 core 职责**：`agent-rule/rules/src/shared/core.md` 只放所有 Agent 都应始终生效的短规则；这里优先约束“非平凡或有歧义任务先显式说明假设、优先最简单本地改动、保持 surgical edits、按 verification point 推进”。
- **生成产物**：面向各 Agent 实际分发的规则文件只维护在 `agent-rule/rules/dist/`，由构建脚本生成；仓库内不再保留第二套 `rules/global/*` source tree。
- **按 Agent 原生格式分发**：Codex 生成 `AGENTS.md`，Claude 生成 `CLAUDE.md`。
- **低频专题进 skill**：原 `architecture`、`style`、`frontend`、`artifact-hygiene`、`writing` 等低频专题细节不再通过 `modules/` 分发，而是在 `agent-skills` 中维护为 `change-discipline`、`frontend-verification`、`artifact-hygiene`、`reader-facing-writing`；技术图的建模与交付由 `technical-diagramming` 单独负责。
- **分层原则**：always-on 规则只保留触发条件、协作边界和最小行为约束；执行步骤、检查清单、失败边界和验证细节进入 skill；Agent 私有差异只放 `agent-rule/rules/src/agents/*/adapter.md`，不要把这些内容回灌进 shared core。
- **迁移清理**：原生分发会清理旧的 `~/.<agent>/rules/global.md` 和旧 `rules/modules` 残留，避免新旧入口并存。
- **Claude 记忆分层**：根目录 `CLAUDE.md` 只保留仓库级高频记忆，`agent-rule/CLAUDE.md` 和 `tests/CLAUDE.md` 承载目录级维护细节。
- **Claude hooks**：`config/claude-hooks.json` 生成仓库级 `.claude/settings.json`，随 `make update` 刷新 deterministic hook 配置。
- **Codex hooks**：`config/codex-hooks.json` 生成仓库级 `.codex/hooks.json`，随 `make update` 刷新 deterministic Codex hook 配置；当前包含 `PreToolUse` 的生成产物写保护和 `Stop` 的规则构建一致性校验。
- **分发清单**：`config/rules-targets.list` 只引用 `rules/dist/*` 产物，不直接引用源码片段。
- **维护入口**：修改 `agent-rule/rules/src/` 后运行 `make update`，规则构建（`agent-rule/scripts/build-agent-rules.py`）与分发随管线自动完成。

## 平台支持 (Platform Support)

- 本项目当前以 **macOS** 为主要验证平台；**Linux** 路径保持 POSIX 兼容设计，但仍建议在目标环境自行回归验证。  
- 在 **Windows** 上，建议使用 **WSL2、Git‑Bash 或 Cygwin** 等提供 POSIX 兼容层的环境，以获得完整功能。  
- 若希望在原生 PowerShell 或 CMD 中直接使用，需要对脚本进行额外的跨平台改造（将外部命令替换为 PowerShell 等价实现），此工作尚未完成。
桌面客户端支持以下基本操作。修改定义后先保存，再预览安装或同步；取消预览不会撤销已保存的定义。

| 元素 | 添加、修改、移除 | 更新 |
| --- | --- | --- |
| Agent | 添加助手、编辑安装定义、移除管理；卸载是独立操作 | 按助手升级 |
| MCP | 添加、编辑、移除服务 | 按服务更新源码运行时并同步配置；通过 npx/uvx 启动的包由启动器解析版本 |
| skills | 添加、编辑、移除技能集合；逐技能控制助手授权 | 按集合更新仓库并同步链接 |
| tools | 添加、编辑、移除工具定义 | 按工具安装或升级 |
| rules | 编辑共享和助手专用规则源码，增删对应文本 | 重新生成并分发 |
| Hooks、aliases | 编辑来源 JSON，增改或删除对应键 | 同步到目标配置 |

Agent 和 tools 的自定义定义分别写入 `agent_definitions`、`tool_definitions` 策略键；MCP 和 skills 沿用定义覆盖层。移除条目写入 `_delete` 标记，避免下层同名定义重新出现；需要恢复时，在个性化配置中删除该标记。移除管理定义与卸载软件分开，系统包不会因移除定义而自动卸载；后续同步仍会清理本工具拥有的孤立缓存目录。

规则、Hooks 和 aliases 的来源编辑通过 `sources:get/set` 完成。保存时检查读取版本、扫描明文敏感信息并创建快照；失败会保留错误供处理。独立客户端升级会保留通过该入口编辑过的来源。可从「备份与恢复」恢复修改前内容，然后重新同步。


## 管理指令集

- `./manage.sh`: 显示帮助；这是脚本的安全默认入口，不会触发任何同步。
- `make update`: 日常最常用。除 Agent 本体外全部收敛到最新（secrets 拉取、shell alias、tools 安装/升级、browser、rules 构建分发、skills 链接、MCP 运行时刷新与配置注入、Claude/Codex hooks 同步）；不安装、也不升级 Agent 软件。
- `make init`: 新机器初始化或全量收敛（= Agent 生命周期（安装缺失 Agent + 升级已装 Agent）+ `update`），并等待 Agent 生命周期安装/升级完成。
- `make agents:update`: 升级/修复本地已安装 Agent 到上游最新（`config/agents.json` 不锁版本），不强制安装缺失 Agent。
- `make doctor`: 执行全系统深度体检，并把 warning 细分为 `可选工具未就绪` 与 `托管产物尚未生成` 两类提示项；同时扫描主仓库中是否存在疑似明文敏感信息。追加 `--json` 可输出机器可读诊断报告。
- `make smoke`: 运行跨平台安全预演 smoke 校验，基于最小策略文件 `config/manager.smoke.json` 验证主入口命令的 dry-run / preview 契约。

## Repo-native Guidance

- 根目录 `AGENTS.md` 提供仓库级的 setup、测试命令、done-when 规则与生成文件边界。
- `agent-rule/AGENTS.md` 只覆盖 `agent-rule/rules/src/` 到 `agent-rule/rules/dist/` 的编辑和重建流程。
- `tests/AGENTS.md` 只覆盖测试命令、fixtures 和 contract-test 的写法。

## Secrets 初始化

- 不再内置固定的 secrets 仓库地址。
- 真实的 `Tailscale` / `SSH` / 远程访问信息也必须只存放在外部私有 `secrets/` 仓库；当前仓库仅允许占位符和通用说明。
- 需要通过环境变量 `SECRETS_REPO_URL` 或脚本参数显式提供仓库地址，例如：

```bash
SECRETS_REPO_URL=https://example.com/your-secrets.git make update
# 或
./scripts/init-secrets.sh https://example.com/your-secrets.git
```

## 许可证
Apache-2.0
