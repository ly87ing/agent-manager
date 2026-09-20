# Agent Manager

集中管理本机 AI 编程助手的桌面应用，基于 Tauri 2、React 与 Rust/SQLite。内置定义为 Claude Code 与 Codex，其余助手可自行添加定义；MCP 另外支持写入 OpenCode 格式的配置。桌面应用与命令行共用同一 Rust 引擎。

## 功能

- **助手管理**：安装、升级、卸载内置助手（Claude Code、Codex）与自行添加的助手，如实检测实际安装来源。
- **接入配置**：助手页顶部提供独立接入工作台，先选助手，再用卡片查看当前接入并一键启用、测试、编辑、复制或删除；复制会自动生成不冲突的名称，默认只保存而不切换，也不会读取凭据取值。当前项需先切走才可删除，避免留下悬空配置。安装、版本和启动命令收在下方维护区。Claude Code 与 Codex 都能保存多套接入方式（官方或自定义中转），同一时刻一套生效。切换写进助手自己的配置文件（`~/.claude/settings.json` / `~/.codex/config.toml`），从终端、IDE、任何入口启动都一样生效；只改本工具拥有的键，同一文件里 hooks、MCP 与你自己的设置原样保留。
  - Claude：接口地址、令牌变量名、令牌怎么发（Bearer 或 x-api-key）、模型、小任务模型、opus/sonnet/haiku 别名、超时。令牌只登记变量名，取值在密钥页填，写配置时解析成真值。
  - Codex：接口地址（必填）、令牌所在的环境变量名、模型、超时，写成 `[model_providers.<名字>]`。令牌不落盘——Codex 启动时自己去环境里读那个变量。
  - 测一下：配完点一次，发一个最小真实请求，一次问清地址通不通、令牌有没有效、模型名存不存在、慢不慢。这是全局唯一会主动联网的助手侧动作，不点就不发；只读，不碰配置文件也不改生效的那套。
  - 厂商预设：点「添加接入」先选智谱 GLM、Kimi、DeepSeek（国内/国际）或自定义；地址、模型与凭据变量自动填好。两种形态的端点不通用（Claude 走 Anthropic 协议、Codex 走 Responses 协议），预设按形态分开给。取值来自各家官方文档，会随厂商变动；各类中转站不在清单里，用「自定义接入」。
- **状态栏**：可选用本工具维护的那份（显示模型、目录、上下文与额度），也可以继续用你自己定义的——后者原样不动。用本工具这份时，助手推给状态栏的实时额度会存一份，卡片上因此能显示与状态栏同屏的数字。
- **跳过权限确认**：一个开关写进助手的安全默认值，改它需要二次确认。
- **Skills**：管理 Skill 集合与单个技能，按助手逐一启用、停用、更新或移除。
- **MCP 服务器**：管理本地 stdio 与远程 HTTP 两类 MCP 服务器，按助手分配并写入各自客户端配置（Claude JSON、Codex TOML、OpenCode JSON）；可从客户端现有配置发现导入。
- **Rules**：编辑共享与助手专用规则，应用到 `~/.codex/AGENTS.md`、`~/.claude/CLAUDE.md` 与 `~/.config/opencode/AGENTS.md`。
- **Hooks**：管理 Claude `~/.claude/settings.json` 与 Codex `~/.codex/hooks.json` 的钩子配置。
- **助手实例**：为同一个助手配多套独立配置（不同账号或接口），各有自己的配置目录、登录、会话、技能与 MCP，多个实例可以同时开着互不干扰；全局 `~/.claude` 与 `~/.codex` 不受影响。鉴权只登记环境变量名，取值不进本工具的库。
- **启动命令**：为每个助手设定固定的启动方式（时区、语言、每次都带上的扩展参数），助手卡片上直接配置与同步；写入 `~/.zshrc` 与 `~/.zprofile` 中由本工具维护的那一段，该段以外的内容不受影响。
- **接管中心**：扫描本机已有的助手、技能与 MCP 资源并按来源分类，勾选后批量纳入管理。官方路径与原生安装方案（助手自带的技能同步目录、Homebrew、官方安装器）只做标识，继续用它们各自的方式升级卸载；其余外部资源以软链接纳管——技能登记为本地来源集合后直接链到源目录，内容仍由原处维护并随其更新，停用只删链接。
- **Hooks**：按「事件 + 匹配器 + 命令」逐行填写，不用照着 JSON 结构手写；不展示的字段原样保留。
- **配置备份**：导出本机定义与来源，换机器或重装时导回来；备份里没有密钥（只有变量名）。
- **额度**：读本机文件显示 Claude Code 与 Codex 的用量与重置时刻，不联网、不用再登录。Claude 侧想要与状态栏同屏的实时值，见[故障排查](docs/TROUBLESHOOTING.md)里的一段状态栏落盘配置。
- **用量**：读本机会话记录累加 token，按天与模型看谁吃掉了用量，并按可编辑的单价表折算成美元。折算是「同样的用量按 API 单价要花多少」，不是账单——订阅制下该付的是订阅费；没有单价的模型只报 token 不报钱。增量扫描，第二次起只读新增部分。
- **更新检查**：探测助手、MCP 与 Skills 的上游新版本。
- **其他**：Secrets 状态（仅显示环境变量名与是否存在，不显示值）、任务操作记录。

## 体验

- 浅色 / 深色 / 跟随系统三种主题，切换即时生效并同步原生窗口标题栏。
- 关闭窗口驻留系统托盘，托盘可唤回主界面或退出；窗口位置与尺寸跨会话记忆；单实例运行（二次启动唤回已有窗口）。
- 命令面板 `⌘K`：一个入口搜助手、接入、实例、技能、MCP、页面与常用动作；搜到某套接入即可一步切换，当前项只展示不重复写，全键盘完成。列表页另有页内搜索（`⌘F`），`⌘,` 打开设置。
- 托盘常驻：可唤回主界面、直接打开命令面板或接管中心；托盘只做导航，写操作仍在界面上确认。
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

打开应用后，每个页面先以只读方式展示当前状态（status 零写入）。**保存定义只写入本地数据库，不安装软件、不修改任何配置文件**；只有点击页面上的执行按钮（应用/安装/同步等）才触发真实变更。

执行（apply）内置安全语义：

- 写前 preflight 校验，精确限定变更范围（scope）；
- 排他锁防止并发写；
- 外部修改检测：目标被本工具以外改动过时 fail-closed 拒绝覆盖；
- 条件回滚与操作审计，每次执行可在任务记录页回查。

卸载、移除定义、清空日志等破坏性动作需二次确认；"移除管理"与"卸载软件"是独立动作。

## 数据与安全

- **唯一事实源**：应用数据目录下的 `native-manager.db`（SQLite）保存全部定义、分配与偏好，没有第二份可写配置文件；查询与状态读取只读，缺库时不创建文件。
- **写入范围**：仅写入受管目标——助手安装、客户端配置中的受管条目、`~/.codex/AGENTS.md`、`~/.claude/CLAUDE.md`、`~/.claude/settings.json`、`~/.codex/hooks.json`，以及 `~/.zshrc` / `~/.zprofile` 的受管区块。
- **密钥模型**：MCP 定义中的密钥只保存 `${VAR}` 形式的环境变量占位符，应用到客户端配置时仍以引用形式写入；明文不出现在本工具的数据库与日志中。Secrets 页只显示变量名与是否存在，不读取外部密钥仓库。
- **不收集数据**：无遥测上传；日志仅写入本机应用数据目录，严格脱敏并按保留期自动清理。

## 文档

- [架构](docs/ARCHITECTURE.md)：数据流与调用边界
- [MCP 配置指南](docs/MCP-CONFIG-GUIDE.md)：MCP 定义字段与密钥占位符模型
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

Agent Manager is a desktop app (Tauri 2 + React + Rust/SQLite) for centrally managing local AI coding agents. Claude Code and Codex ship as built-in definitions; other agents can be added manually. MCP configuration can additionally be written in OpenCode's format. The UI is in Simplified Chinese.

- **Features**: install/upgrade/uninstall agents (built-in and custom); manage Skill collections with per-agent enablement; manage local stdio and remote HTTP MCP servers across three client config formats (Claude JSON, Codex TOML, OpenCode JSON); edit Rules (`~/.codex/AGENTS.md`, `~/.claude/CLAUDE.md`, `~/.config/opencode/AGENTS.md`), Hooks, and managed shell alias blocks in `~/.zshrc` / `~/.zprofile`; check for upstream updates; view secrets status (variable names and presence only) and task history.
- **Install**: no public GitHub Release yet — packages are unavailable until external prerequisites (signing keys, Apple Developer ID and notarization, Windows signing, per-platform hardware acceptance) are met; see [platform status](docs/GUI-PLATFORM-STATUS.md). To build from source on macOS, install the Rust toolchain, Node.js, and pnpm, clone without `--recursive` (the private `secrets/` submodule is not needed), then run `pnpm --dir app install --frozen-lockfile && pnpm --dir app run build:macos`.
- **Safety model**: saving a definition only writes the local SQLite database (`native-manager.db` in the app data directory); nothing is installed or modified until you click execute. Apply runs with preflight checks, exact scope, an exclusive lock, fail-closed external-modification detection, conditional rollback, and audit logging; destructive actions require confirmation. MCP secrets are stored as `${VAR}` placeholders and written to client configs by reference — plaintext never lands in this tool's database or logs. No telemetry.
- **License**: [MIT](LICENSE).
- **Experience**: light/dark/system themes with native title bar sync, system tray with window recall, remembered window size/position, single instance, per-page search, `⌘,` / `⌘F` shortcuts, update badge, skeleton loading and transition animations.
