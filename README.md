# Agent Manager

集中管理本机 AI 编程助手的桌面应用，基于 Tauri 2、React 与 Rust/SQLite，内置支持 Claude Code、Codex 和 OpenCode。桌面应用与命令行共用同一 Rust 引擎。

## 功能

- **助手管理**：安装、升级、卸载内置助手（Claude Code、Codex、OpenCode）与自定义助手，如实检测实际安装来源。
- **Skills**：管理 Skill 集合与单个技能，按助手逐一启用、停用、更新或移除。
- **MCP 服务器**：管理本地 stdio 与远程 HTTP 两类 MCP 服务器，按助手分配并写入各自客户端配置（Claude JSON、Codex TOML、OpenCode JSON）；可从客户端现有配置发现导入。
- **Rules**：编辑共享与助手专用规则，应用到 `~/.codex/AGENTS.md` 与 `~/.claude/CLAUDE.md`。
- **Hooks**：管理 Claude `~/.claude/settings.json` 与 Codex `~/.codex/hooks.json` 的钩子配置。
- **Shell Aliases**：在 `~/.zshrc` 与 `~/.zprofile` 中维护受管别名区块，区块外内容不受影响。
- **接管中心**：扫描本机已有的助手、技能与 MCP 资源并按来源分类，勾选后批量纳入管理。官方路径与原生安装方案（助手自带的技能同步目录、Homebrew、官方安装器）只做标识，继续用它们各自的方式升级卸载；其余外部资源以软链接纳管——技能登记为本地来源集合后直接链到源目录，内容仍由原处维护并随其更新，停用只删链接。
- **更新检查**：探测助手、MCP 与 Skills 的上游新版本。
- **其他**：Secrets 状态（仅显示环境变量名与是否存在，不显示值）、任务操作记录、本地诊断日志管理。

## 体验

- 浅色 / 深色 / 跟随系统三种主题，切换即时生效并同步原生窗口标题栏。
- 关闭窗口驻留系统托盘，托盘可唤回主界面或退出；窗口位置与尺寸跨会话记忆；单实例运行（二次启动唤回已有窗口）。
- 列表页内置搜索（助手 / 技能 / MCP / 操作记录）；快捷键 `⌘,` 打开设置、`⌘F` 聚焦页内搜索。
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

Agent Manager is a desktop app (Tauri 2 + React + Rust/SQLite) for centrally managing local AI coding agents, with built-in support for Claude Code, Codex, and OpenCode. The UI is in Simplified Chinese.

- **Features**: install/upgrade/uninstall agents (built-in and custom); manage Skill collections with per-agent enablement; manage local stdio and remote HTTP MCP servers across three client config formats (Claude JSON, Codex TOML, OpenCode JSON); edit Rules (`~/.codex/AGENTS.md`, `~/.claude/CLAUDE.md`), Hooks, and managed shell alias blocks in `~/.zshrc` / `~/.zprofile`; check for upstream updates; view secrets status (variable names and presence only) and task history.
- **Install**: no public GitHub Release yet — packages are unavailable until external prerequisites (signing keys, Apple Developer ID and notarization, Windows signing, per-platform hardware acceptance) are met; see [platform status](docs/GUI-PLATFORM-STATUS.md). To build from source on macOS, install the Rust toolchain, Node.js, and pnpm, clone without `--recursive` (the private `secrets/` submodule is not needed), then run `pnpm --dir app install --frozen-lockfile && pnpm --dir app run build:macos`.
- **Safety model**: saving a definition only writes the local SQLite database (`native-manager.db` in the app data directory); nothing is installed or modified until you click execute. Apply runs with preflight checks, exact scope, an exclusive lock, fail-closed external-modification detection, conditional rollback, and audit logging; destructive actions require confirmation. MCP secrets are stored as `${VAR}` placeholders and written to client configs by reference — plaintext never lands in this tool's database or logs. No telemetry.
- **License**: [MIT](LICENSE).
- **Experience**: light/dark/system themes with native title bar sync, system tray with window recall, remembered window size/position, single instance, per-page search, `⌘,` / `⌘F` shortcuts, update badge, skeleton loading and transition animations.
