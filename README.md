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
  - 启动：`npx -y chrome-devtools-mcp --isolated --no-usage-statistics`（不锁版本，随 npx 解析上游最新；`--isolated` 避免多个 agent 共享同一个 browser profile 导致互相污染；`--no-usage-statistics` 关闭上游默认开启的使用统计上报）。
- `grok-search`
  - 用途：联网搜索与网页抓取。
  - 注意：真实 provider key 仅从 `secrets/` 注入。
- `serena`
  - 用途：**仅用于 JS/TS** 的 LSP 语义导航（`find_symbol` / 引用 / 符号概览）；需本机有 `uvx` 与 Node.js。
  - 配置：`make init` 与 `make update` 会把每个项目的 Serena 数据目录对齐到系统临时目录，避免在仓库内生成 `.serena/`；换机后重新执行即可生效。
  - 注意：Java 不托管给它（上游 `oraios/serena#1478` 未支持按项目注册 JDK runtime，Java 8 会被 JDK21 runtime 分析）。详见 `docs/MCP-CONFIG-GUIDE.md`。

### 其他受管工具

- `chrome-devtools`（MCP）
  - 不锁版本；各 Agent 启动时由 npx 解析上游最新。
默认受管 skill 仓库为 `agent-skills`，跟随上游 `main` 分支最新（不固定 commit），由 `make update` 同步 clone/pull。`agent-skills` 面向 `Claude Code`、`Codex` 与 `Kimi Code` 暴露，并通过显式 `skill_paths` 锁定为当前声明的 `safe-merge-review`、`change-discipline`、`frontend-verification`、`artifact-hygiene`、`reader-facing-writing`、`technical-diagramming` 这 6 个 skill 包，不会因为上游新增 skill 而自动扩大安装范围。
默认自动安装的 Agent 当前只包含 `Claude Code` 与 `Codex`。
`Claude Code` 与 `Codex` 默认走官方原生独立安装器（`native`）。`Claude Code` 原生安装器支持后台自动更新与 `claude update`；`Codex` 使用官方独立安装脚本进行安装与升级。
已有 `npm` 或 `brew` 来源沿用升级，不进行隐式迁移或删除既有安装。
Windows 平台支持：`Claude Code` 支持原生 Windows（PowerShell / CMD）；`Codex` 不声明未核实的 Windows 原生独立安装支持。
`Claude Code local install` 不纳入 `agent-manager` lifecycle 托管，因为它依赖 shell alias 暴露入口，当前探测链路不会越权接管。
以上 lifecycle 口径统一按两个当前受管 Agent 的官方推荐方式维护。`Claude Code` 和 `Codex` 的 npm/brew provider 保留受管升级命令模板。`agent-manager` 不改写用户 shell profile。

自定义 Agent 的生命周期定义使用 `providers.custom`，显式设置 `managed: true`，并填写 `bin` 与安装、升级命令。只有该可执行文件存在且没有识别出其他安装来源时，才归为自定义管理；已有 npm、brew 等来源保持原分类。安装和升级执行后重新探测，脚本成功但未找到程序仍判为失败。卸载命令可选，未填写时拒绝自动卸载。

桌面端“添加助手”可直接填写名称、命令路径、安装和升级命令，卸载命令可选；高级选项支持分别编辑各系统命令。已有助手通过“编辑命令”修改，保存不会自动安装或升级，取消表单不写配置。助手列表区分当前与默认安装方式；安装、升级、卸载使用明确动作按钮。

显式动作使用 `./manage.sh agents:plan --only agents:YOUR_AGENT --action install --json` 预览，将 `plan` 换为 `apply` 执行；动作可选 `install`、`upgrade`、`uninstall`，卸载还需 `--allow-uninstall`。`--dry-run` 不运行安装脚本或修改策略。安装只用于未安装的助手；升级和卸载要求唯一、受管的实际来源及有效命令，不会把安装请求自动变成升级。成功安装或卸载并复核后才保存启用状态，失败不预先改策略；升级保留原启用状态。外部安装、多个来源或缺少对应命令时不提供不可执行的按钮。

## Official CLI Lifecycle Policy

| CLI | 默认受管 provider | 兼容受管 provider | 说明 |
| :--- | :--- | :--- | :--- |
| `Claude Code` | `native` | `npm`, `brew` | 新安装默认使用官方原生安装器（`curl -fsSL https://claude.ai/install.sh \| bash`，Windows 支持 PowerShell/CMD）；已有 `npm` 或 `brew` 来源沿用升级，不隐式迁移。 |
| `Codex` | `native` | `npm`, `brew` | 新安装默认使用官方独立安装脚本（`curl -fsSL https://chatgpt.com/codex/install.sh \| sh`）；升级采用官方更新脚本；已有 `npm` 或 `brew` 来源沿用升级，不隐式迁移。Windows 不声明未核实支持。 |

## Agent 安装源依据 (Official Channels Catalog)

下表列出上游公开安装渠道，对 `Claude Code` 与 `Codex` 的实际受管策略以上一节为准。

| 助手名称 | 二进制标识 | 官方 native 安装器 / 升级 | 官方 npm | Homebrew 官方源 (macOS) | 备注 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Claude Code** | `claude` | `curl -fsSL https://claude.ai/install.sh \| bash` / `claude update` | `@anthropic-ai/claude-code` | `cask: claude-code` | 默认走官方 native 安装器；已有 npm/brew 安装沿用升级。 |
| **Codex** | `codex` | `curl -fsSL https://chatgpt.com/codex/install.sh \| sh` / 官方安装脚本升级 | `@openai/codex` | `cask: codex` | 默认走官方 native 独立脚本；已有 npm/brew 安装沿用升级；Windows 不声明未核实支持。 |

## 核心健全性特性

### 1. 本地数据与内置定义
- **本地数据库 (`manager.db`)**：GUI 与 CLI 共用应用数据目录；macOS 默认位于 `~/Library/Application Support/com.agentmanager.dev/`，可用 `AGENT_MANAGER_DATA_DIR` 指定目录。设备偏好继续使用 `settings.json`。
- **首次迁移**：`./manage.sh db:migrate --json` 显式导入旧配置；首次策略写入、`update`、`init` 或域级 `apply` 也会初始化。导入包含 baseline/synced/local 的有效策略、定义及删除标记，不执行安装或重新启用。迁移使用临时数据库，校验后原子切换；损坏或不支持版本的已有数据库会被拒绝。
- **只读与预演**：`help`、状态、计划、`policy:get` 和 `--dry-run` 不创建数据库。未初始化时仍读取旧配置；数据库激活后，策略写入只更新 SQLite，适配器读取临时生成的配置。
- **旧文件保留**：迁移不删除 `manager.json`、`manager.synced.json` 和 `manager.local.json`，之后编辑旧策略文件不再影响数据库。规则、Hooks 与 alias 正文当前仍走原有源文件编辑链路，尚未全部迁入数据库；旧 profile 拉取、推送在数据库激活后禁用，避免同步过期状态。
- **回退边界**：旧版本只认识旧文件。回退程序前应保留数据库，核对迁移后的修改；直接运行旧版本不会把这些新修改自动还原到旧 JSON。当前没有自动反向迁移。
- **资源更新与使用版本分开**：Agent、tools 与 MCP 程序沿用各自上游更新方式。Skill 仓库缓存跟踪分支头；SQLite 模式下，已分配的 Skill 使用内容摘要标识的不可变版本，更新集合不自动更新各 Agent 的使用版本。
- **浏览器前置开关**：`browser_us_sync.enabled` 控制 `update` / `init` 是否执行浏览器 Google US 前置同步。
- **模块化定义 (`agents.json`等)**：保存官方指令细节，支持 Policy 的强制覆盖。

### Skill 分配适配器（SQLite 模式）

Skill 分配保存集合、技能、Agent、版本及启用状态，正文快照位于应用数据目录的 `skill-versions/`。首次同步会在刷新仓库前固定已有受管链接的内容；新发现的技能只进入资源库，不自动分配。禁用集合后再启用会保留各 Agent 原来的开关，上游删除技能也不删除已选版本。

`agent-skill/link_skills.py` 提供以下独立动作，必须指定 `--agent YOUR_AGENT` 和单个 `--only skill:YOUR_COLLECTION/YOUR_SKILL`，并使用已初始化的数据库：

| 参数 | 效果 |
| --- | --- |
| `--add-assignment` | 添加分配，默认启用；已有分配需使用更新动作 |
| `--update-version` | 采用当前资源库内容，只更新指定 Agent；禁用的分配保持禁用 |
| `--enable-assignment` | 恢复已选版本的入口 |
| `--disable-assignment` | 撤下入口，保留分配和版本 |
| `--remove-assignment` | 移除分配及其入口，普通同步不会重新添加 |

适配器仍接收引擎生成的 `--config`、`--def-file` 与缓存 `--base-dir`；不要把旧策略 JSON 当作数据库的可编辑副本。`--dry-run` 完成校验但不写数据库、快照或链接。共享物理目录不支持单 Agent 隔离操作；外部内容冲突会被拒绝，失败恢复也不覆盖外部新增内容。当前这五个动作已在适配器实现，统一引擎命令与新版页面仍在接入；现有页面状态尚不能作为不可变分配的完整验收入口。

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

桌面客户端已有 macOS 本地候选包（2026-09-08，Darwin x86_64），已在隔离用户目录、最小 PATH 和禁止读取源码的环境下完成实际应用验收。内置 Python/Rust 运行时随包交付；业务所需的 Agent、Node/git 仍需另行准备。设置可持久保存离线模式、预览默认演练和缓存 TTL。当前未签名发布，真实签名自更新尚未验收。

桌面包的平台范围与发布条件见 [GUI 平台状态](docs/GUI-PLATFORM-STATUS.md)，本地构建和复跑入口见 [应用说明](app/README.md)。Linux 桌面尚待验证；Windows 桌面包不支持独立 Bash 运行链，以下 POSIX 兼容层说明仅针对脚本使用，不代表 Windows 安装器已可用。

- 本项目当前以 **macOS** 为主要验证平台；**Linux** 路径保持 POSIX 兼容设计，但仍建议在目标环境自行回归验证。  
- 在 **Windows** 上，脚本运行需要 **WSL2、Git‑Bash 或 Cygwin** 等 POSIX 兼容环境，具体命令仍须在目标环境验证。
- 若希望在原生 PowerShell 或 CMD 中直接使用，需要对脚本进行额外的跨平台改造（将外部命令替换为 PowerShell 等价实现），此工作尚未完成。

## 管理指令集

桌面客户端支持以下基本操作。Agent、MCP 和技能集合表单只保存本机定义，不自动安装、启用或同步；取消表单不会保存。MCP 和技能集合的“允许分配的助手”只限制后续分配范围，不会为选中的助手立即添加资源。工具表单仍在保存后展示安装预览，取消预览不会撤销已保存的工具定义。

| 元素 | 添加、修改、移除 | 更新 |
| --- | --- | --- |
| Agent | 添加助手、编辑安装定义、移除管理；卸载是独立操作 | 按助手升级 |
| MCP | 添加、编辑、移除服务 | 按服务更新源码运行时并同步配置；通过 npx/uvx 启动的包由启动器解析版本 |
| skills | 添加、编辑、移除技能集合；逐技能控制助手授权 | 按集合更新仓库并同步链接 |
| tools | 添加、编辑、移除工具定义 | 按工具安装或升级 |
| rules | 编辑共享和助手专用规则源码，增删对应文本 | 重新生成并分发 |
| Hooks、aliases | 编辑来源 JSON，增改或删除对应键 | 同步到目标配置 |

Agent、tools、MCP 和 skills 的本机定义由 SQLite 保存。编辑 MCP 或技能集合时保留表单未展示的字段，清空可选字段后不会恢复仓库默认值；旧配置首次提交编辑时先迁移到本机数据库，取消不迁移。MCP 表单只展示环境变量名，保留已有取值或引用；无法还原的脱敏值会阻止保存。移除条目写入 `_delete` 标记，避免下层同名定义重新出现；需要恢复时，在个性化配置中删除该标记。移除管理定义与卸载软件分开，系统包不会因移除定义而自动卸载；后续同步仍会清理本工具拥有的孤立缓存目录。

规则、Hooks 和 aliases 的来源编辑通过 `sources:get/set` 完成。保存时检查读取版本、扫描明文敏感信息并创建快照；失败会保留错误供处理。独立客户端升级会保留通过该入口编辑过的来源。可从「备份与恢复」恢复修改前内容，然后重新同步。

- `./manage.sh`: 显示帮助；这是脚本的安全默认入口，不会触发任何同步。
- `make update`: 日常最常用。把所有受管组件收敛到本机（secrets 拉取、shell alias、tools、browser、rules 构建分发、skills 链接、MCP 注入、Claude/Codex hooks 同步）；不安装、也不升级 Agent 软件。
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
