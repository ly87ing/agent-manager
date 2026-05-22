# Agent Manager (Ultimate Edition)

🚀 **AI 助手环境一键管理中枢**。通过单一入口，只对 `Claude Code`、`Codex`、`Gemini CLI` 做 Agent 生命周期、MCP、skills 和 rules 管理。退役 Agent 的旧路径只保留必要的清理/迁移逻辑，不再作为受管目标维护。

## 核心规范速查表 (Authoritative Spec)

本项目严格遵循各 Agent 官方最新规范，确保配置注入的精准与安全：

```text
   ┌────────────────┬─────────────────────────┬──────────┬─────────────┐
   │ Agent 名称     │ 官方标准路径            │ 文件格式 │ 核心键名    │
   ├────────────────┼─────────────────────────┼──────────┼─────────────┤
   │ Claude Code    │ ~/.claude.json          │ JSON     │ mcpServers  │
   │ Gemini CLI     │ ~/.gemini/settings.json │ JSON     │ mcpServers  │
   │ Codex          │ ~/.codex/config.toml    │ TOML     │ mcp_servers │
   └────────────────┴─────────────────────────┴──────────┴─────────────┘
```

默认启用的 MCP 服务器当前包含 `chrome-devtools`（浏览器调试 / network / performance，使用官方推荐的 `npx -y chrome-devtools-mcp@latest --isolated`，避免多个 agent 共享同一个 browser profile 时互相冲突）和 `grok-search`（联网搜索与网页抓取，真实 provider key 只从 secrets 注入）。`augment-context-engine` 的定义仍保留，但在缺少 `AUGMENT_SESSION_AUTH` 时默认不启用。
默认受管浏览器工具当前包含 `playwright-cli` 和 `agent-browser`；二者通过官方 npm `@latest` 通道受管安装与更新。`playwright-cli` 在安装或升级后会继续执行 `install-browser` 同步 Playwright browsers，并用 Playwright cache 的真实 payload 做就绪重验；`agent-browser` 优先复用系统已有的 Chrome/Chromium，也会接受 Playwright cache 中的 Chromium 作为可复用运行时，仅在缺少这些浏览器运行时时才需要执行 `agent-browser install`。`graphify` 通过官方 PyPI 包 `graphifyy` 由 `uv tool` 受管，并把 `/graphify` skill 安装到 Claude、Codex、Gemini 三个声明的 Agent 目标；graphify 按需使用，不再通过 shared rules 或 Claude/Codex hooks 默认提示读取 graphify 报告。`chrome-devtools` 继续作为 MCP server 通过 `npx ...@latest` 在运行时跟随上游最新版本。
浏览器区域前置由 `./manage.sh browser:sync` 统一收敛；`update` / `bootstrap` / `all` 会自动调用它。该步骤会同步已发现 Chromium-family profile 的 `en-US` 语言、account-scoped 语言偏好、Google US 搜索入口、WebRTC 非代理 UDP 限制，并阻断普通 profile 的真实 geolocation 泄漏、站点级 geolocation 例外和站点级 reduced Accept-Language 例外；同时写出并审计专用隔离 profile `~/.config/agent-manager/browser-us-profile`，在该隔离 profile 内额外强制 Chrome variations US 国家一致性、Preferences install country、camera、microphone、notifications 默认阻断和站点级设备权限例外清空，再生成 `browser-us-context.json` / `browser-us.env` / `browser-us-chrome.sh` 供自动化浏览器和手动 Chrome 启动复用 US locale、timezone、geolocation、隔离 `--user-data-dir` 和 Chrome 启动参数。`browser-us-chrome.sh` 默认使用该隔离 profile，无 URL 启动时默认打开 `google.com/ncr`，并禁用 sync、extensions、QUIC、DNS prefetch、Web Speech API、WebGPU adapter、Local Font Access、WebBluetooth、WebUSB、WebHID、WebXR、权限请求提示和 Chrome `ReduceAcceptLanguage` 特性，避免复用普通 Chrome 的账号、cookie、扩展状态以及减少浏览器侧 UDP/预解析路径，同时避免页面枚举本机多语言语音列表、GPU adapter、本机字体清单、XR runtime 和已授权本机硬件设备，并保持 `en-US,en;q=0.9` 语言头不被站点级缩减；headless runtime 额外用 Chrome `--screen-info` 固定虚拟屏幕为 US desktop 尺寸，避免暴露默认 `800x600` headless 屏幕；普通 profile 的 install-time country、Local State variations country 和设备权限例外不作为硬性收敛项，因为正在运行的 Chrome 或账号状态会回写这些易变字段，专用隔离 profile 会继续强制并审计这些字段。脚本还会写出 `browser-us-verification.json` 记录本次可强制 US 特征的审计结果。公网出口 profile 会在线探测并在日志中打印 IP、国家、region、city、timezone 和坐标；检测到非 US 或无法确认出口 profile 时默认失败，不写浏览器状态；检测到 US 且服务返回 timezone / 经纬度时，会按当前 VPN 出口 IP 派生自动化浏览器的时区和 geolocation。同步后会等待一个短稳定期，再读回 `Local State` 和 profile `Preferences` 做实际状态审计，防止正在运行的 Chrome 或账号同步把 US 偏好写回非 US 值；随后用临时 cookie 访问 `google.com/ncr` 后检查 Google 首页不再出现 `google.com.hk` / Hong Kong 标记，并用临时 user-data-dir 启动 headless Chrome 访问本地探针页，验证 `navigator.language`、`navigator.languages`、`navigator.webdriver`、UA Client Hints brands / platform / mobile / high-entropy desktop surface、桌面 viewport / screen avail / orientation / color depth / DPR / pointer / hover / touch / PDF viewer / plugins、Network Information 4g / saveData=false、Web Speech API 禁用状态、Local Font Access 禁用状态、WebGPU adapter 不可用状态、WebBluetooth / WebUSB / WebHID 入口禁用状态、WebXR 入口禁用状态、Serial 已授权端口为 0、Gamepad 设备数为 0、MIDI 权限未授予且无输入/输出设备暴露、clipboard / idle / storage / payment / sync 等敏感权限未授予、Window Management 权限未授予且无屏幕 label 暴露、Sensors 权限未授予、媒体设备不暴露 label / deviceId / groupId、自动化上下文中的 US QWERTY 键盘布局、hardwareConcurrency / deviceMemory 桌面值、自动化 WebGL vendor / renderer、常见中文/CJK 字体的 Font Loading API 阻断和 canvas 字体指标归一化、Intl locale / region、US week info、US locale timezone list、gregory/latn/h12 Intl 设置、US 数字与列表格式、JS timezone、Date / Intl timeZoneName、实际 HTTP `Accept-Language` 与 `Sec-CH-UA-*` 请求头、geolocation permission 阻断状态，以及隔离 profile 的 camera / microphone / notifications 默认阻断状态。任一审计失败都会退出失败。脚本按幂等收敛设计，不写时间戳，运行态自检只使用临时目录，重复运行不会继续改写已收敛文件。
默认受管 skill 仓库当前包含 `agent-skills`、`oa-skill`、`anthropic-frontend-design-skill`、`playwright-best-practices-skill`、`superpowers` 和 `ui-ux-pro-max-skill`；它们在未显式声明 `commit` / `ref` 时，会在同步时跟随上游默认分支最新状态；其中 `agent-skills` 面向 `Claude Code` 与 `Codex` 暴露，并通过显式 `skill_paths` 锁定为当前声明的 3 个 skill 包，避免上游新增 skill 被默认带入；`oa-skill` 当前通过显式 `skill_paths` 默认接入仓库内唯一的 `xyai-manager` skill；`anthropic-frontend-design-skill` 仅面向 `Claude Code` 暴露，并通过显式 `skill_paths` 锁定 Anthropic 官方 `skills/frontend-design`。OpenAI 官方仓库当前最新主分支已不再提供 `frontend-skill`，因此默认清单不再引用该路径，避免 `make all` 在跟随上游最新状态时失败。
默认自动安装的 Agent 当前只包含 `Claude Code`、`Codex` 和 `Gemini CLI`。
`Claude Code` 当前默认走官方 native installer；`Codex` 与 `Gemini CLI` 默认走官方 npm 包。历史 `brew` 安装仅用于识别并提示迁移，不再作为这三个 CLI 的受管 provider。
`Claude Code native` 当前已经补齐官方 uninstall 路径，因此 `install / update / uninstall` 都可继续走官方 installer 语义。
`Claude Code` 仍保留 `npm` 作为 legacy-compatible 受管 provider；`Claude Code local install` 不纳入 `agent-manager` lifecycle 托管，因为它依赖 shell alias 暴露入口，当前探测链路不会越权接管。
以上 lifecycle 口径统一按三个当前受管 Agent 的官方推荐方式维护。`Claude Code` 优先使用官方 native installer 和 `claude update`；`Codex` 使用官方 npm 包并通过 `npm install -g @openai/codex@latest` 升级；`Gemini CLI` 使用官方 npm stable 包并通过 `npm install -g @google/gemini-cli@latest` 升级。放弃 `brew` 或不用 `brew` 不等于所有 agent 都必须 native installer。

## Official CLI Lifecycle Policy

| CLI | 默认受管 provider | 兼容受管 provider | detect-only 历史来源 | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| `Claude Code` | `native` | `npm` | `brew` | 默认使用官方 installer；若已是 npm 版，可继续受管或按官方 `claude migrate-installer` 手工迁移；官方 uninstall 路径也已纳入托管。 |
| `Codex` | `npm` | - | `brew` | 使用官方 npm 包；升级固定为 `npm install -g @openai/codex@latest`，历史 brew 安装只用于审计和迁移提示。 |
| `Gemini CLI` | `npm` | - | `brew` | 使用官方 npm stable 包；升级固定为 `npm install -g @google/gemini-cli@latest`，brew 只做历史识别。 |

## Agent 安装源依据 (Official Channels Catalog)

下表列出上游公开安装渠道，但不等于 `agent-manager` 会默认或继续托管这些渠道；对 `Claude Code`、`Codex`、`Gemini CLI` 的实际受管策略以上一节为准。

| 助手名称 | 二进制标识 | 官方 npm / 安装器 | Homebrew 官方源 (macOS) | 备注 |
| :--- | :--- | :--- | :--- | :--- |
| **Claude Code** | `claude` | 官方 native installer / `@anthropic-ai/claude-code` | `cask: claude-code` | `brew` 仅保留 detect-only 语义。 |
| **Codex** | `codex` | `@openai/codex` | `cask: codex` | 受管 lifecycle 默认走 npm。 |
| **Gemini CLI** | `gemini` | `@google/gemini-cli` | `formula: gemini-cli` | 受管 lifecycle 默认走 npm stable。 |

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
- **一键大扫除 (Cleanup)**：精准回收受管备份、碎片及冗余日志；覆盖当前受管目标和 Gemini 临时 JSON 备份。
- **失效洗刷 (Scrubbing)**：自动清理 Agent 目录中的死亡软链。

### 5. 安全默认值
- **安全入口**：直接执行 `./manage.sh` 只显示帮助，不触发同步。
- **不接管终端配置**：`agent-manager` 不再维护 `~/.zshrc`、`~/.tmux.conf`、iTerm2 动态配置等用户级终端文件，避免把机器/个人偏好写死后扩散到其他电脑。
- **主仓库禁存 secrets**：真实凭证只允许存在于独立的 `secrets/` 私有仓库或系统环境变量中；`make secrets-check`、`make smoke`、CI 与 `make doctor` 会扫描主仓库文件并阻止明文敏感信息落入 `agent-manager`。

### 6. Rules 源码与分发分层
- **可编辑源码**：跨 Agent 的共享规则片段只维护在 `agent-rule/rules/src/`，这是仓库内唯一可编辑的 rule source。
- **共享 core 职责**：`agent-rule/rules/src/shared/core.md` 只放所有 Agent 都应始终生效的短规则；这里优先约束“非平凡或有歧义任务先显式说明假设、优先最简单本地改动、保持 surgical edits、按 verification point 推进”。
- **生成产物**：面向各 Agent 实际分发的规则文件只维护在 `agent-rule/rules/dist/`，由构建脚本生成；仓库内不再保留第二套 `rules/global/*` source tree。
- **按 Agent 原生格式分发**：Codex 生成 `AGENTS.md`，Claude 生成 `CLAUDE.md`，Gemini 生成 `GEMINI.md`。
- **原生 modules 位置**：全局文件旁边的 `modules/` 目录也会跟着落到原生同级位置，例如 `~/.codex/modules`、`~/.claude/modules`、`~/.gemini/modules`。
- **Codex 路径特例**：由于 Codex 会把用户级规则合并进当前工作区指令视图，生成的 `~/.codex/AGENTS.md` 会直接引用 `config/rules-targets.list` 中 `codex_modules` 配置的目标路径（默认是 `~/.codex/modules/*.md`），避免把相对 `modules/` 误解析为仓库内路径。
- **分层原则**：低频、专题或工具边界细节放 `agent-rule/rules/src/shared/modules/`；Agent 私有差异只放 `agent-rule/rules/src/agents/*/adapter.md`，不要把这些内容回灌进 shared core。
- **迁移清理**：原生分发会清理旧的 `~/.<agent>/rules/global.md` 和旧 `rules/modules` 残留，避免新旧入口并存。
- **Claude 记忆分层**：根目录 `CLAUDE.md` 只保留仓库级高频记忆，`agent-rule/CLAUDE.md` 和 `tests/CLAUDE.md` 承载目录级维护细节。
- **Claude hooks**：`config/claude-hooks.json` 生成仓库级 `.claude/settings.json`，用 `make claude-hooks-sync` 刷新 deterministic hook 配置。
- **Codex hooks**：`config/codex-hooks.json` 生成仓库级 `.codex/hooks.json`，用 `make codex-hooks-sync` 刷新 deterministic Codex hook 配置；当前包含 `PreToolUse` 的生成产物写保护和 `Stop` 的规则构建一致性校验。
- **graphify 启用**：`config/tools.json` 中的 `graphify` 安装官方 `graphifyy` CLI，`scripts/setup-graphify.py` 将 skill 分发到 Claude、Codex、Gemini 目标；graphify 按需使用，不再通过 shared rules 或 Claude/Codex hooks 默认提示读取 graphify 报告。
- **指令质量评测**：`make eval-rules` 现在同时输出任务成功率、verification、skill exposure 和 always-on instruction budget 指标，并强制校验 `min_case_count`、`required_agents`、`success / verification 阈值`。
- **分发清单**：`config/rules-targets.list` 只引用 `rules/dist/*` 产物，不直接引用源码片段。
- **维护入口**：修改 `agent-rule/rules/src/` 后，先执行 `make rules-build`，再执行 `make rules:sync`。

## 平台支持 (Platform Support)

- 本项目当前以 **macOS** 为主要验证平台；**Linux** 路径保持 POSIX 兼容设计，但仍建议在目标环境自行回归验证。  
- 在 **Windows** 上，建议使用 **WSL2、Git‑Bash 或 Cygwin** 等提供 POSIX 兼容层的环境，以获得完整功能。  
- 若希望在原生 PowerShell 或 CMD 中直接使用，需要对脚本进行额外的跨平台改造（将外部命令替换为 PowerShell 等价实现），此工作尚未完成。

## 管理指令集

- `./manage.sh`: 显示帮助；这是脚本的安全默认入口。
- `make sync`: 同步当前配置到 rules，并将 skills / MCP 资源对齐到最新；不处理 Agent / Tool 生命周期。
- `make update`: 更新远端资源并收敛本地环境，等待 Agent 生命周期更新完成，但不安装缺失 Agent。
- `make bootstrap`: 执行全量安装、更新与同步，并等待 Agent 生命周期安装/升级完成，适合新机器初始化。
- `make all`: `make bootstrap` 的兼容别名。
- `make plan`: 预演一次完整 `bootstrap`。
- `make self-update`: 仅更新 agent-manager 仓库自身逻辑。
- `make smoke`: 运行跨平台安全预演 smoke 校验，基于最小策略文件 `config/manager.smoke.json` 验证主入口命令的 dry-run / preview 契约。
- `make secrets-check`: 扫描主仓库受控文件，拒绝真实 token / 私钥等明文敏感信息进入 `agent-manager`；`secrets/` 私有仓库和忽略缓存不在扫描范围内。
- `make eval-rules`: 运行离线规则效果评测，读取 `scripts/evals/fixtures/` 中的固定 JSON 样例并输出稳定的机器可读摘要。
- `make instructions:check`: 共享指令、skills exposure 和项目内 hooks 变更前的统一回归门（硬门禁）；会串行跑 instruction-related project contracts、rules budget/layout、skill exposure、rules sync / Claude/Codex managed hooks 契约、eval schema/metric 契约，以及 baseline 规则评测。
- `make doctor` / `make status`: 执行全系统深度体检，并把 warning 细分为 `可选工具未就绪` 与 `托管产物尚未生成` 两类提示项；同时扫描主仓库中是否存在疑似明文敏感信息。追加 `--json` 可输出机器可读诊断报告。
- `make rules-build`: 从 `agent-rule/rules/src/` 生成 `agent-rule/rules/dist/` 下的可分发规则产物。
- `make rules:sync`: 将 `agent-rule/rules/dist/` 中的规则产物分发到各 Agent 原生入口。
- `make codex-hooks-sync`: 刷新仓库级 `.codex/hooks.json`。
- `make fetch-models`: 探测并获取 API 端点支持的模型列表 (支持 OpenAI / Anthropic 格式)。
- `make restore`: 交互式回滚历史快照。
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
SECRETS_REPO_URL=https://example.com/your-secrets.git make update
# 或
./scripts/init-secrets.sh https://example.com/your-secrets.git
```

## CC-Bridge Runtime 管理

- `make cc-bridge-install`: 同步本地 `secrets/` 后，按最新稳定 release 完成首次安装并启动。
- `make cc-bridge-start`: 启动前会先检查最新稳定版本；如果发现新版本，会先升级再启动。
- `make cc-bridge-stop`: 停止当前 `cc-bridge` 容器，但保留宿主机数据目录和状态文件。
- `make cc-bridge-deploy`: 同步本地 `secrets/` 后，自动查询 `cc-bridge` 最新稳定 release tag，渲染宿主机运行目录并完成首次部署。
- `make cc-bridge-update`: 每次执行都自动读取 GitHub `releases/latest`，将宿主机 bind mount 数据目录备份后升级到最新稳定版本；不需要手工查 tag。
- `make cc-bridge-seed`: 从 `secrets/cc-bridge.accounts.json` 和 `secrets/cc-bridge.api-tokens.json` 导入账号池与网关 token。
- `make cc-bridge-rollback`: 回滚到最近一次成功部署前记录的上一个 tag。
- `make cc-bridge-status`: 输出当前 tag、最新稳定 tag、容器主进程 PID、访问 URL、端口、Docker 运行状态，以及 Claude Code 的接入环境变量示例。
- `cc-bridge` 实际运行的不是裸上游镜像，而是基于显式 release tag 生成的美国化派生镜像，用来强制提供 `America/Los_Angeles` 时区和 `en_US.UTF-8` locale。
- `cc-bridge` 只有在上游 tag、本地派生镜像构建上下文或本地派生镜像缓存发生变化时才会重新 build；没有变化时会直接复用现有派生镜像。
- `cc-bridge` 运行时默认强制使用 `America/Los_Angeles` 时区和 `en_US.UTF-8` 美国英语 locale；如果容器内实际做不到这些设置，`install/start/update/rollback` 会直接失败。
- `cc-bridge` 数据目录固定走宿主机挂载，不使用 Docker named volume；默认挂载目标是运行目录下的 `data/` 到容器 `/app/data`。
- 具体文件布局、secret 格式和运行命令见 [docs/CC-BRIDGE-MANAGEMENT.md](./docs/CC-BRIDGE-MANAGEMENT.md)。

## 许可证
Apache-2.0
