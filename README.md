# Agent Manager

Agent Manager 是 Rust 原生引擎、Tauri 2 和 React 组成的本机管理工具。CLI 与 Tauri 共享 `engine-rs` 库；Python 已从根仓库清除。

## 当前事实源

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源，位于应用数据目录。设置保存在数据库 `app_settings` 表。缺库读取只返回 Rust 内置只读默认值，不创建文件；首次保存才初始化数据库。policy 可编辑写入固定使用 `--layer local`，持久化来源为 `sqlite`。

Claude Code 与 Codex 的内置默认渠道都是官方 `native` 命令。brew 只表示实际检测到的来源，不改变默认渠道。

定义保存与执行动作分开：保存只写数据库；按钮只按引擎返回的 actions，遵循 plan → 用户确认 → apply。Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除；移除管理与卸载是不同动作。

SQLite 核心、Tauri 桥、设置与 Policy 页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域与规则来源、Hooks 原生域与 hooks 来源、Runs SQLite 操作记录、结构化诊断日志及 Python 清理已完成。Skills 内置集合只读默认，首次本地写入 `native-manager.db` 后本地优先；集合与 Skill×Agent 均支持更新、启停与移除，启用状态通过内容快照与安全软连接落地，status/plan 保持零写，真实 apply 具备进程锁与操作审计。两类日志协同运作：SQLite `operation_logs` 记录可审计任务历史，并在新增操作记录时清理过期记录；`data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 记录结构化诊断元数据（严格脱敏、Unix 0700/0600、只读零写、拒随软链接），并在诊断日志写入后及应用启动时清理过期文件。两类日志共用 `log_retention_days`（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空任务记录，设置页支持 `logs:clear` 二次确认清理诊断日志，两类手动清理互不影响。Rules 原生 status/plan/apply 与规则专用 `sources:get/set` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/plan/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed；当前候选安装包尚未完成包含最新 Rules/MCP/Hooks 的真实业务 E2E。

## MCP 原生域状态

Rust dispatcher 已实现本地 stdio MCP 的 `mcp:status`、`mcp:plan` 和 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；支持 Claude JSON、Codex TOML 和 OpenCode JSON，OpenCode 必须在对应 Agent definition 中配置 `mcp_config` 目标。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`status`、`plan` 和 `mcp:apply --dry-run` 零写；真实 `apply` 使用排他锁、操作审计、CAS 写入和回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。

远程 HTTP MCP 尚未实现。

`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。设置页高级管理可达 Aliases、更新中心等页面，Aliases 原生 `aliases:status/plan/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 尚未实现，保留在路线图。

## 开发

```sh
make test
make check
make build
make smoke
```

直接验证可用 `cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。隔离写入请设置临时 `AGENT_MANAGER_DATA_DIR`，不要覆盖 HOME。

macOS 打包与独立包验收（当前环境限定 Darwin x86_64，构建自动启用源码根目录与主目录路径脱敏重映射）：

- 打包 app 与 dmg：`pnpm --dir app run build:macos`（产物位于 `app/src-tauri/target/release/bundle/dmg/`）
- 独立包运行验收：`pnpm --dir app run verify:macos` 或 `./scripts/verify-macos-dmg.sh app/src-tauri/target/release/bundle/dmg/Agent\ Manager_0.1.0_x64.dmg`（只读挂载、隔离复制、元数据/架构/无 Python 依赖校验、源码路径脱敏校验、从 `/` 隔离目录 5 秒存活运行验收）。该 2026-09-09 Darwin x86_64 DMG 早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环；签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

## 目录

- [engine-rs](engine-rs/)：CLI/Tauri 共用 Rust 库和 SQLite 存储。
- [app](app/)：Tauri 桌面应用和 React 前端。
- [contract](contract/)：Engine Contract v1 schema。
- [当前架构](docs/ARCHITECTURE.md) · [平台状态](docs/GUI-PLATFORM-STATUS.md)。
