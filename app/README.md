# Agent Manager 桌面应用

本目录是 Tauri 2 应用。React UI 通过 bridge 调用 `engine-rs`；CLI 与桌面端共享同一 Rust 原生库。运行时不依赖 Python、`manage.sh` 或源码树资源。

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源，设置在 `app_settings` 表。查询、plan 和 dry-run 不创建或改变数据库；定义保存只写 SQLite。

Claude Code 与 Codex 的内置默认渠道是官方 `native` 命令。Homebrew 仅作为检测到的实际来源展示，不是默认。

保存定义不执行安装或链接。执行按钮来自引擎 actions，写操作遵循 plan → 确认 → apply。管理删除（移除定义）与卸载软件是不同动作。

Agent、MCP、Skills 页面已按目标交互组织；Agent、Skills 和 MCP 生命周期已接入 Rust 原生域。Skills 支持集合与单技能更新、启停与移除，采用内容快照与安全软连接落地，status/plan 保持零写。SQLite 核心、Tauri bridge、设置与 Policy 页面、Runs SQLite 操作记录与清理、本地 JSONL 结构化诊断日志与清理（按日滚动、严格脱敏、Unix 0700/0600；`log_retention_days` 统一控制两类日志保留期；Runs 页二次确认清空任务记录，设置页二次确认清理诊断日志，两类清理边界分明）和 Python 清理已完成。Rules/Hooks 原生 status/plan/apply 与 `sources:get/set` 尚未接线；当前候选安装包尚未完成真实业务 E2E。

Rust dispatcher 已实现本地 stdio MCP 的 `mcp:status`、`mcp:plan` 和 `mcp:apply`。定义与分配保存在 `native-manager.db` 的 SQLite 表中，支持 Claude JSON、Codex TOML 和 OpenCode JSON；OpenCode 需在对应 Agent definition 配置 `mcp_config` 目标。逐助手支持 `add`、`update`、`enable`、`disable`、`remove`，库级支持 `enable`、`disable`、`remove`；`status`、`plan` 和 `mcp:apply --dry-run` 零写，真实 `apply` 具备排他锁、操作审计、CAS 和回滚保护，`env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP MCP 尚未实现。

设置页高级管理可达总览、Tools、Aliases、Secrets、更新中心和快照页面；其 `system:audit`、`system:refresh-upstream`、`system:converge`、`tools:status/plan/apply`、`aliases:status/plan/apply`、`secrets:status`、`snapshots:list/restore` 虽存在 Tauri 白名单或 React 页面入口，但 Rust dispatcher 尚未接线，不等同后端实现，待产品裁决实现或从新版 UI 退役。

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

### macOS 打包与独立验收（限定 Darwin x86_64）

```sh
pnpm run build:macos
# 产物生成在 src-tauri/target/release/bundle/dmg/，可直接运行验证脚本或使用 npm script：
pnpm run verify:macos
# 或显式指定 DMG 路径：
../scripts/verify-macos-dmg.sh src-tauri/target/release/bundle/dmg/Agent\ Manager_0.1.0_x64.dmg
```

打包脚本自动配置 `--remap-path-prefix` 剥离本机构建 HOME 与源码根路径。验收脚本完成只读挂载、脱离仓库隔离复制、无 Python 动态与静态依赖检查、脱敏校验，以及在 `/` 下使用最小 PATH 与隔离目录的 5 秒存活、源码零引用和退出检测。2026-09-09 已在 Darwin x86_64 实机构建 DMG 并通过上述独立运行验收；该包早于当前 MCP 实现，不能证明当前 MCP GUI 业务闭环。Apple 签名、公证、arm64、Linux、Windows 和更新发布仍未验证。
