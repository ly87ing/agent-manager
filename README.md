# Agent Manager

让同一台 Mac 上的多个 Claude Code / Codex 账号同时使用：每个账号一个独立配置目录、一条启动命令，一套技能、MCP 与规则默认分给全部账号。基于 Tauri 2、React 与 Rust/SQLite，桌面应用与命令行共用同一 Rust 引擎。

## 功能

- **账号**：按助手列出全部账号——主账号（`claude` / `codex`，只放官方登录）和各个独立账号（例如 `cc2`、`ds`），每行显示启动命令、官方还是 API、登录状态与 5 小时 / 每周额度。「添加账号」只问官方还是 API、叫什么；API 账号可选厂商预设，API Key 只写进外部密钥文件。建好后自动写好目录、启动命令（沿用主账号命令里的时区与参数），带上主账号的权限、状态栏、hooks、规则与 MCP，并链接全部在用的技能。编辑 API 账号时登记与目录里的配置一起写；移除只删启动命令与登记，目录、登录和会话都保留。登录只复制官方命令，由你在终端执行。
- **实时额度**：一键为全部 Claude 账号装上 Agent Manager 的用量状态栏，它每次刷新时把额度记在本工具的数据目录里，账号页与托盘据此显示实时额度；Codex 的额度直接读它自己的会话记录。
- **技能**：以集合（git 仓库或本机目录）管理技能，软链接到各账号的技能目录。在用的技能默认给全部账号，「补齐」一键链接到还缺它的账号；某个账号上停用是例外，补齐不会加回。新发现的技能点「添加」就加到全部账号。
- **MCP**：直接读写各账号配置文件里的条目，一服务一行汇总在哪些账号上启用；同一种助手内可一键补齐，跨助手用「复制到…」。编辑、启停、删除都先预览完整文件并核对是否被外部改过。
- **规则与 Hooks**：直接编辑各账号的 `CLAUDE.md` / `AGENTS.md` 与 hooks；规则模板只存在本机，推荐规则来自技能来源（agent-skills 的 `rules/`）；「补齐」只给还没有规则文件的账号复制主账号的规则，已有内容一律不动。
- **用量**：读本机会话记录按账号累加 token，可按时间、账号与模型筛选，按单价表折算（不是账单）。
- **设置**：导入本机已有的配置、操作记录、密钥文件、终端快捷命令、客户端诊断、主题与配置备份。

## 体验

- 顶栏入口：账号、技能、MCP、规则（含自动命令 Hooks）、命令、来源；右侧是「搜索 ⌘K」、主题切换、用量与设置。每页首屏就是列表，一行一项。
- `⌘K` 搜页面、账号与资源，选中账号即复制它的启动命令；`⌘⇧K` 只列账号。`⌘F` 页内搜索，`⌘,` 打开设置。
- 托盘按助手列出全部账号（名称 · 启动命令 · 额度），点一下唤回命令面板并定位到它，按 Enter 复制启动命令；托盘不写任何配置。
- 浅色 / 深色 / 跟随系统主题；窗口位置与尺寸跨会话记忆；单实例运行；新版本提示。

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

1. 打开应用，账号页列出每个助手的主账号与已有的独立账号（读取只读，不写任何文件）；顶部「开始使用」提示还没做的几步（登录主账号；添加技能来源，可选，不用也行），做完或关掉就不再出现。
2. 点「添加账号」：官方账号建好后点「登录」复制登录命令，到终端执行一次；API 账号填好地址与 API Key 即可。
3. 新开一个终端，输入账号的启动命令（例如 `cc2`），即可与其他账号同时使用。
4. 在技能、MCP、规则页点「补齐」，把在用的资源分给新账号；需要实时额度时在账号页点「开启实时额度」。

AI CLI 的安装、升级与卸载不在本工具内执行。

执行（apply）内置安全语义：

- 写前 preflight 校验，精确限定变更范围（scope）；
- 排他锁防止并发写；
- 外部修改检测：目标被本工具以外改动过时 fail-closed 拒绝覆盖；
- 条件回滚与操作审计，每次执行可在任务记录页回查。

移除定义、清空日志等破坏性动作需二次确认。

## 数据与安全

- **以实际配置为准**：各账号的真实文件与目录是生效配置的唯一事实源；应用数据目录下的 `native-manager.db`（SQLite）只保存偏好、账号登记、归属记录和未生效的备选（如 Claude 侧停用的 MCP），没有第二份可写配置文件；查询与状态读取只读，缺库时不创建文件。内置默认只作推荐，确认后才应用。
- **写入范围**：仅写入受管目标——各账号配置目录里的受管条目（`settings.json` / `config.toml` 的本工具负责的键、MCP 条目、规则与 hooks 文件、技能链接），以及 `~/.zshrc` / `~/.zprofile` 的受管区块。主账号只放官方登录，本工具不往主配置写第三方接入；登录凭据只由助手自己的 `/login` 保存，本工具从不读写。
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

Agent Manager is a desktop app (Tauri 2 + React + Rust/SQLite) for running several Claude Code and Codex accounts side by side on one Mac. Each account is its own config directory plus a shell launch command; skills, MCP servers and rules are shared with every account by default. The desktop app and CLI share one Rust engine. The UI is in Simplified Chinese.

- **Features**: an accounts page listing the main account and every extra account with launch command, official/API type, login state and live quota; add, edit and remove accounts in a side drawer (API keys go to an external secrets file only); skill collections symlinked into every account with one-click "fill"; MCP entries edited in each account's own config file; rules and hooks edited in place; one-click live quota via the built-in status line; local usage totals per account.
- **Install**: no public GitHub Release yet; see [platform status](docs/GUI-PLATFORM-STATUS.md). To build from source on macOS, install the Rust toolchain, Node.js and pnpm, clone without `--recursive`, then run `pnpm --dir app install --frozen-lockfile && pnpm --dir app run build:macos`.
- **Safety model**: the real files in each account's directory are the only source of truth; every write re-reads the file and refuses if it changed since you looked, runs under an exclusive lock with rollback and an audit log, and never touches login credentials. The main account keeps only the official login. No telemetry.
- **License**: [MIT](LICENSE).
