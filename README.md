# Agent Manager

集中管理本机 AI 编程助手账号接入、运行配置与配套资源的桌面应用，基于 Tauri 2、React 与 Rust/SQLite。当前产品支持 Claude Code 与 Codex；桌面应用与命令行共用同一 Rust 引擎。

## 功能

- **助手与账号**：助手页以接入工作台为主，集中管理当前账号/供应商、模型、凭据变量和少量运行设置，不维护 AI CLI 的安装、升级或卸载脚本。
- **环境一览**：先选助手，同一页列出它的全部环境——默认环境（直接输入 `claude` / `codex`，只放官方账号，逐行写明账号、登录、模型与自动化）、各个独立环境（写明终端里输入什么启动），以及用来建独立环境的第三方接入。右上角「新建环境」可选第三方厂商（选预设、填密钥、建成独立环境）或另一个官方账号。
- **接入配置**：官方账号与第三方凭据完全分开放。主配置（`~/.claude/settings.json` / `~/.codex/config.toml`）只放官方账号，不写第三方接入；第三方接入保存成卡片，可测试、编辑、复制、导入导出、写备注，要用时在卡片菜单里「建成独立环境」，在自己的配置目录里使用，凭据只引用同一个变量名。主配置里还有旧版本留下的第三方接入时，工作台提示混用并提供「切回官方账号」（切之前先把这份配置保存成接入）。写主配置只改本工具拥有的键，同一文件里 hooks、MCP 与你自己的设置原样保留。
  - Claude：接口地址、令牌变量名、令牌怎么发（Bearer 或 x-api-key）、模型、opus/sonnet/haiku/fable 别名、超时（官方已弃用的「小任务模型」只保留旧值，建成独立环境时按官方替代写成 Haiku 别名）。令牌只登记变量名，取值在密钥页填，写配置时解析成真值。
  - Codex：接口地址（必填）、令牌变量名、模型、超时。令牌只登记变量名，取值在密钥页或接入表单里填；建成独立环境后写进实例 `config.toml` 的 `[model_providers.agent-manager]`（`experimental_bearer_token`，与 CC Switch 同一做法），启动时不用在终端导出变量。
  - 测一下：可单测一套，也可一键检查当前助手的全部接入。每套都发一个最小真实请求，卡片就地显示结果和延迟，全部完成后汇总可用数量与最快线路。这是全局唯一会主动联网的助手侧动作，不点就不发；检查本身只读，不碰配置文件也不改生效的那套。
  - 厂商预设：点「添加接入」先选智谱 GLM、Kimi 开放平台、Kimi Code 会员、DeepSeek、MiniMax、火山方舟 Coding Plan、小米 MiMo、美团 LongCat、阶跃 Step Plan（多数分国内/国际）或自定义；选中后给出厂商的密钥页与接入文档地址；地址、模型与凭据变量自动填好。两种形态的端点不通用（Claude 走 Anthropic 协议、Codex 走 Responses 协议），预设按形态分开给。取值来自各家官方文档，厂商自己发布了接入代码的以代码为准，会随厂商变动；各类中转站不在清单里，用「自定义接入」。表单可「获取模型列表」挑模型；DeepSeek 与 Kimi 开放平台的接入与独立环境可「查余额」（只用厂商文档写明的余额接口）。
- **状态栏**：可选用本工具维护的那份（显示模型、目录、上下文与额度），也可以继续用你自己定义的——后者原样不动。用本工具这份时，助手推给状态栏的实时额度会存一份，卡片上因此能显示与状态栏同屏的数字。
- **跳过权限确认**：一个开关写进助手的安全默认值，改它需要二次确认。
- **客户端诊断**：在「设置 → 客户端诊断」只读查看 AI CLI 是否可用、当前版本与安装来源；安装和更新继续使用客户端官方安装器或系统包管理器。
- **Skills**：管理 Skill 集合与单个技能，按助手逐一启用、停用、更新或移除。
- **MCP 服务器**：直接列出各助手配置文件（Claude `~/.claude.json`、Codex `config.toml`，以及配置过的 OpenCode）里已有的本地 stdio 与远程 HTTP 条目，无需导入；逐条新增、编辑、启停、复制到另一个助手或删除，只改这一条，条目里表单不管的官方字段原样保留，写入前核对是否已被外部修改。停用时 Codex/OpenCode 写官方 `enabled = false`；Claude Code 没有全局停用字段，条目移出配置、保存在本工具里，启用时写回。
- **Rules**：直接查看和编辑 `~/.codex/AGENTS.md`、`~/.claude/CLAUDE.md` 与各实例的规则文件；规则模板只存在本机；推荐规则来自技能来源（agent-skills 的 `rules/`），不随程序发布，来源更新后提示可更新。应用模板、来源规则或复制前先把对不上已知内容的原内容另存为备份。
- **Hooks**：直接查看和编辑 Claude（`~/.claude/settings.json` 的 `hooks` 键，同一文件里的接入、运行参数与你自己的设置原样保留）、Codex（`~/.codex/hooks.json`）与各实例的实际 hooks；文件打开后被别处改过，保存会被拒绝。旧版本写过、现已不用的 hook 列为清理建议，旧版本存在库里还没生效的 hooks 列为建议，都要逐条确认才会动。
- **助手实例**：为同一个助手配多套独立配置（不同账号或接口），各有自己的配置目录、登录、会话、技能与 MCP，多个实例可以同时开着互不干扰；全局 `~/.claude` 与 `~/.codex` 不受影响。鉴权只登记环境变量名，取值不进本工具的库，写入时解析成真值，只写进这个实例自己的配置文件（权限 600）。每个实例按官方写法挡住 shell 里导出的凭据、接口地址、云厂商开关与附加请求头（Claude 在实例 `settings.json` 里置空，Codex 在启动命令前清空），第三方实例只用自己的凭据、官方账号实例只用目录里的登录；第三方 Claude 实例默认让 Bash 工具、hooks 与 MCP 子进程读不到凭据（官方 `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`；Linux 上官方要求先装 bubblewrap，因此默认不开）。系统级设置（组织下发的 Claude 设置、`/etc/codex` 下的文件）对每个目录都生效、实例挡不住，主配置与实例卡片会逐条说明。主配置与实例都可按需用官方命令检查登录状态（只看这个目录自己的登录），登录与退出命令只复制、不代为执行。新建实例时可勾选「从默认环境带上」权限规则、状态栏、Hooks、规则文件与 MCP 服务（卡片上也可随时再带）：复制一次，只补实例里还没有的；凭据（env、apiKeyHelper、含明文凭据的 MCP 条目）不带。
- **实例启动命令**：实例可维护自己的启动命令；通用终端快捷命令放在「设置 → 终端快捷命令」，以 `~/.zshrc` 与 `~/.zprofile` 中由本工具维护的那一段为准，逐条编辑即写入，文件打开后被别处改过，保存会被拒绝；该段以外你自己写的 alias 只读列出、不受影响。推荐的快捷命令来自技能来源（agent-skills 的 `aliases/`），不随程序发布；旧版本存在库里还没生效的列为建议，都要逐条确认。
- **导入现有配置**：侧栏常驻入口扫描本机已有的助手与技能，勾选后先核对影响，再批量纳入管理（MCP 条目直接在 MCP 页查看和编辑，不需要导入）。官方路径与原生安装方案（助手自带的技能同步目录、Homebrew、官方安装器）只做标识，继续用它们各自的方式升级卸载；其余外部资源以软链接纳管——技能登记为本地来源集合后直接链到源目录，内容仍由原处维护并随其更新，停用只删链接。
- **Hooks**：按「事件 + 匹配器 + 命令」逐行填写，不用照着 JSON 结构手写；不展示的字段原样保留，也可切到原文编辑。
- **密钥与配置备份**：先在「密钥」页选择一个已存在、权限为 `600` 的外部 `.env` 文件；应用只在数据库保存文件路径，密钥取值不进入数据库、日志或配置备份。配置备份只导出定义、来源与 MCP 的已保存项（停用快照等，只含变量引用），换机器时外部密钥文件需单独迁移。
- **额度**：读本机文件显示 Claude Code 与 Codex 的用量与重置时刻，不联网、不用再登录。Claude 侧想要与状态栏同屏的实时值，见[故障排查](docs/TROUBLESHOOTING.md)里的一段状态栏落盘配置。
- **用量**：读本机会话记录累加 token（主配置与各独立环境分开记），可按时间、助手与模型联动筛选，再按天与模型看谁吃掉了用量，并按可编辑的单价表折算成美元。汇总、未定价提示与明细始终使用同一筛选范围。分叉会话抄进新文件的旧回复、Codex 重发的同一累计值只算一次。折算是「同样的用量按 API 单价要花多少」，不是账单——订阅制下该付的是订阅费；没有单价的模型只报 token 不报钱。增量扫描，第二次起只读新增部分。
- **更新检查**：只读探测助手、MCP 与 Skills 的上游新版本；AI CLI 的更新不由本工具执行。
- **其他**：密钥状态只显示变量名、来源与是否存在，不显示值；另有任务操作记录。

## 体验

- 浅色 / 深色 / 跟随系统三种主题，切换即时生效并同步原生窗口标题栏；窄窗口自动把固定侧栏收为顶部横向导航，主内容保持可读宽度。
- 关闭窗口驻留系统托盘，托盘可唤回主界面或退出；窗口位置与尺寸跨会话记忆；单实例运行（二次启动唤回已有窗口）。
- 命令面板 `⌘K`：一个入口搜助手、实例、技能、MCP、页面与常用动作；每个主助手有一项「官方账号」，主配置里有第三方接入时可一步切回，已是官方账号时只展示不重复写。`⌘⇧K` 直接进入只含这些「官方账号」项的账号快切，全键盘完成。列表页另有页内搜索（`⌘F`），`⌘,` 打开设置。
- 托盘常驻：按 Claude Code/Codex 列出「官方账号」和可信当前项；点目标会唤回已精确过滤的命令面板，再按一次 Enter 即可沿安全切换链路切回官方账号。也可直接打开完整命令面板或“导入现有配置”；托盘不静默改配置。
- 执行成功后说明何时生效（助手要重启、终端要新开窗口）；移除定义后可限时撤销。
- 新版本提示：启动后自动检查（更新服务已配置时），设置入口显示角标，可按版本忽略。
- 骨架屏加载态、组件化空态引导、页面与弹窗过渡动画，遵循系统减弱动态效果设置。

界面语言为简体中文。

## 安装

### 下载安装包

尚未发布首个 GitHub Release，安装包暂未提供。公开发布仍需完成仓库外部条件：GitHub Actions 计费、签名密钥、Apple Developer ID 与公证、Windows 签名材料，以及 Linux/Windows/arm64 真机验收。当前进展见 [平台状态](docs/GUI-PLATFORM-STATUS.md)。发布后将提供 macOS（dmg）、Linux（AppImage/deb）与 Windows（NSIS/MSI）安装包。

### 从源码构建

前置条件：Rust 工具链、Node.js 与 pnpm。克隆时不要加 `--recursive`：仓库含私有子模块 `secrets/`（不对外），跳过它不影响构建与使用。

macOS：

```sh
git clone https://github.com/ly87ing/agent-manager.git
cd agent-manager
pnpm --dir app install --frozen-lockfile
pnpm --dir app run build:macos
```

产物位于 `app/src-tauri/target/release/bundle/dmg/`。独立包验收：`pnpm --dir app run verify:macos`。Linux 与 Windows 安装包由 [.github/workflows/gui-release.yml](.github/workflows/gui-release.yml) 构建；本地构建脚本目前仅面向 macOS。

## 快速上手

打开应用后，每个页面先以只读方式展示当前状态（status 零写入）。**保存定义只写入本地数据库，不安装软件、不修改任何配置文件**；只有点击明确的写入或同步按钮才触发真实变更。AI CLI 的安装、升级与卸载不在本工具内执行。

执行（apply）内置安全语义：

- 写前 preflight 校验，精确限定变更范围（scope）；
- 排他锁防止并发写；
- 外部修改检测：目标被本工具以外改动过时 fail-closed 拒绝覆盖；
- 条件回滚与操作审计，每次执行可在任务记录页回查。

移除定义、清空日志等破坏性动作需二次确认。

## 数据与安全

- **以实际配置为准**：各助手真实文件与目录是生效配置的唯一事实源；应用数据目录下的 `native-manager.db`（SQLite）只保存偏好、归属记录和未生效的备选（如未激活的接入配置），没有第二份可写配置文件；查询与状态读取只读，缺库时不创建文件。内置默认只作推荐，确认后才应用。
- **写入范围**：仅写入受管目标——客户端配置中的受管条目、`~/.codex/AGENTS.md`、`~/.claude/CLAUDE.md`、`~/.claude/settings.json`、`~/.codex/hooks.json`，以及 `~/.zshrc` / `~/.zprofile` 的受管区块。
- **密钥模型**：MCP 的生效条目就在各助手配置文件里，本工具只在「已保存未生效」项（如 Claude 侧停用的条目）里存东西：密钥一律改成 `${VAR}` 引用、取值转存到外部文件，只有不像凭据的普通取值（如 `NODE_ENV=production`）原样保存。用户在「密钥」页显式选择一个外部 `.env` 文件后，应用会校验它是普通文件、权限为 `600` 且内容是合法 `KEY=VALUE`；填写和接管的取值只写该文件。页面不回显取值，数据库只保存文件路径，配置备份不包含路径或取值；未选择可用文件时写入操作 fail-closed。
- **不收集数据**：无遥测上传；日志仅写入本机应用数据目录，严格脱敏并按保留期自动清理。

## 文档

- [架构](docs/ARCHITECTURE.md)：数据流与调用边界
- [MCP 配置指南](docs/MCP-CONFIG-GUIDE.md)：MCP 条目的读写、启停方式与密钥规则
- [Secrets 边界](docs/SECRETS-MANAGEMENT.md)
- [故障排查](docs/TROUBLESHOOTING.md)
- [平台状态](docs/GUI-PLATFORM-STATUS.md)：各平台构建与验收状态
- [变更日志](CHANGELOG.md)
- 面向贡献者：[GUI 重设计契约](docs/GUI-REDESIGN.md)、[目标与验收](docs/goal-rust-gui.md)、[Engine 契约](contract/README.md)、[桌面应用开发说明](app/README.md)

## 开发

仓库结构：`engine-rs/`（CLI 与桌面端共用的 Rust 库与 SQLite 存储）、`app/`（Tauri 应用与 React 前端）、`contract/`（Engine Contract v1 schema）、`scripts/`（构建与验收脚本）。

验证命令（仓库根目录）：

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

冒烟测试 `./scripts/ci-smoke.sh` 在隔离数据目录运行，不触碰真实环境。完整验证要求见 [docs/goal-rust-gui.md](docs/goal-rust-gui.md) 的"验证"一节。

## 许可证

[MIT](LICENSE)

## English Summary

Agent Manager is a desktop app (Tauri 2 + React + Rust/SQLite) for centrally managing Claude Code and Codex. The desktop app and CLI share one Rust engine. The UI is in Simplified Chinese.

- **Features**: manage providers, isolated account instances, runtime settings, Skills, local stdio and remote HTTP MCP servers, Rules (edit `~/.codex/AGENTS.md`, `~/.claude/CLAUDE.md` and per-instance rules files in place, with saved templates), Hooks (edit the real hooks in `~/.claude/settings.json` — only its `hooks` key — `~/.codex/hooks.json` and per-instance files in place), and shell aliases (edit the aliases in the managed blocks of `~/.zshrc` / `~/.zprofile` in place; built-in aliases are recommendations only); check for upstream updates; view redacted authentication and secrets status, local usage, and task history. AI CLI installation and updates remain with the official installer or system package manager.
- **Install**: no public GitHub Release yet — packages are unavailable until external prerequisites (signing keys, Apple Developer ID and notarization, Windows signing, per-platform hardware acceptance) are met; see [platform status](docs/GUI-PLATFORM-STATUS.md). To build from source on macOS, install the Rust toolchain, Node.js, and pnpm, clone without `--recursive` (the private `secrets/` submodule is not needed), then run `pnpm --dir app install --frozen-lockfile && pnpm --dir app run build:macos`.
- **Safety model**: saving a definition only writes the local SQLite database (`native-manager.db` in the app data directory); nothing is installed or modified until you click execute. Apply runs with preflight checks, exact scope, an exclusive lock, fail-closed external-modification detection, conditional rollback, and audit logging; destructive actions require confirmation. MCP entries are read from and edited in each client's own config file (one entry at a time, fail-closed if changed externally); the only MCP data this tool stores are saved-but-inactive items such as Claude entries you disabled, where secrets are kept as `${VAR}` references with values moved to your external secrets file — plaintext secrets never land in this tool's database or logs. No telemetry.
- **License**: [MIT](LICENSE).
- **Experience**: light/dark/system themes with native title bar sync, system tray with window recall, remembered window size/position, single instance, per-page search, `⌘,` / `⌘F` shortcuts, update badge, skeleton loading and transition animations.
