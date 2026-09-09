# Agent Manager

Agent Manager 是 Rust 原生引擎、Tauri 2 和 React 组成的本机管理工具。CLI 与 Tauri 共享 `engine-rs` 库；Python 已从根仓库清除。

## 当前事实源

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源，位于应用数据目录。设置保存在数据库 `app_settings` 表。缺库读取只返回 Rust 内置只读默认值，不创建文件；首次保存才初始化数据库。policy 可编辑写入固定使用 `--layer local`，持久化来源为 `sqlite`。

Claude Code 与 Codex 的内置默认渠道都是官方 `native` 命令。brew 只表示实际检测到的来源，不改变默认渠道。

定义保存与执行动作分开：保存只写数据库；按钮只按引擎返回的 actions，遵循 plan → 用户确认 → apply。Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除；移除管理与卸载是不同动作。

SQLite 核心、Tauri 桥、设置与 Policy 页面、主要资源页面布局、Runs SQLite 操作记录、Agent 原生生命周期和 Python 清理已完成。MCP/Skills 原生域尚未接线，未跟踪草稿不算完成；Rules/Hooks 和独立包真实 E2E 待完成。

## 开发

```sh
make test
make check
make build
make smoke
```

直接验证可用 `cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。隔离写入请设置临时 `AGENT_MANAGER_DATA_DIR`，不要覆盖 HOME。

## 目录

- [engine-rs](engine-rs/)：CLI/Tauri 共用 Rust 库和 SQLite 存储。
- [app](app/)：Tauri 桌面应用和 React 前端。
- [contract](contract/)：Engine Contract v1 schema。
- [当前架构](docs/ARCHITECTURE.md) · [平台状态](docs/GUI-PLATFORM-STATUS.md)。
