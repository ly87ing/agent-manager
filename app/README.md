# Agent Manager 桌面应用

本目录是 Tauri 2 应用。React UI 通过 bridge 调用 `engine-rs`；CLI 与桌面端共享同一 Rust 原生库。运行时不依赖 Python、`manage.sh` 或源码树资源。

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源，设置在 `app_settings` 表。查询、plan 和 dry-run 不创建或改变数据库；定义保存只写 SQLite。

Claude Code 与 Codex 的内置默认渠道是官方 `native` 命令。Homebrew 仅作为检测到的实际来源展示，不是默认。

保存定义不执行安装或链接。执行按钮来自引擎 actions，写操作遵循 plan → 确认 → apply。管理删除（移除定义）与卸载软件是不同动作。

Agent、MCP、Skills 页面已按目标交互组织；Agent 与 Skills 生命周期已接入 Rust 原生域。Skills 支持集合与单技能更新、启停与移除，采用内容快照与安全软连接落地，status/plan 保持零写。SQLite 核心、Tauri bridge、设置与 Policy 页面、Runs SQLite 操作记录与清理、本地 JSONL 结构化诊断日志与清理（按日滚动、严格脱敏、Unix 0700/0600；`log_retention_days` 统一控制两类日志保留期；Runs 页二次确认清空任务记录，设置页二次确认清理诊断日志，两类清理边界分明）和 Python 清理已完成。MCP 原生域、Rules/Hooks 统一编辑与端侧落地及独立包真实 E2E 待完成。

## 本地开发

```sh
pnpm install --frozen-lockfile
pnpm test:unit run
pnpm typecheck
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml --all-targets
```

CLI 查询在仓库根目录运行。测试数据库使用临时 `AGENT_MANAGER_DATA_DIR`，不覆盖 HOME。
