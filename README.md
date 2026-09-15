# Agent Manager

Agent Manager 是 Rust 原生引擎、Tauri 2 和 React 组成的本机管理工具。CLI 与 Tauri 共享 `engine-rs` 库；Python 已从根仓库清除。

## 当前事实源

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源，位于应用数据目录。设置保存在数据库 `app_settings` 表。缺库读取只返回 Rust 内置只读默认值，不创建文件；首次保存才初始化数据库。policy 可编辑写入固定使用 `--layer local`，持久化来源为 `sqlite`。

Claude Code 与 Codex 的内置默认渠道都是官方 `native` 命令。brew 只表示实际检测到的来源，不改变默认渠道。

定义保存与执行动作分开：保存只写数据库；按钮只按引擎返回的 actions，遵循 plan → 用户确认 → apply。Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除；移除管理与卸载是不同动作。

SQLite 核心、Tauri 桥、设置与 Policy 页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域与规则来源、Hooks 原生域与 hooks 来源、Runs SQLite 操作记录、结构化诊断日志及 Python 清理已完成。Skills 内置集合只读默认，首次本地写入 `native-manager.db` 后本地优先；集合与 Skill×Agent 均支持更新、启停与移除，启用状态通过内容快照与安全软连接落地，status/plan 保持零写，真实 apply 具备进程锁与操作审计。两类日志协同运作：SQLite `operation_logs` 记录可审计任务历史，并在新增操作记录时清理过期记录；`data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 记录结构化诊断元数据（严格脱敏、Unix 0700/0600、只读零写、拒随软链接），并在诊断日志写入后及应用启动时清理过期文件。两类日志共用 `log_retention_days`（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空任务记录，设置页支持 `logs:clear` 二次确认清理诊断日志，两类手动清理互不影响。Rules 原生 status/plan/apply 与规则专用 `sources:get/set` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/plan/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed；验收基线提交 `5a110be3` 的候选安装包已完成主要真实 GUI E2E 验收（含 Rules/MCP/Hooks/Aliases/Secrets/Runs/system:audit，MCP 新增服务表单因 macOS 自动化输入限制未覆盖）。

## MCP 原生域状态

Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`、`mcp:plan` 和 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；支持 Claude JSON、Codex TOML 和 OpenCode JSON，OpenCode 必须在对应 Agent definition 中配置 `mcp_config` 目标。远程定义含 `url`（必填；仅允许 `https://`，或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符），与 stdio 字段互斥；headers 以引用形式写入目标，明文不落盘。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`status`、`plan` 和 `mcp:apply --dry-run` 零写；真实 `apply` 使用排他锁、操作审计、CAS 写入和回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。

远程 HTTP MCP 已实现：远程定义含 `url`（必填；仅允许 `https://`，或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符），与 stdio 字段互斥；headers 以引用形式写入目标（Claude 字面量 `${VAR}`、Codex `env_http_headers`、OpenCode `{env:VAR}`），解析后明文不落盘；受管标记统一为条目级 `_MANAGED_BY`；远程定义 freshness 恒 not-applicable。

`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。设置页高级管理可达 Aliases、更新中心等页面，Aliases 原生 `aliases:status/plan/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，agents/mcp/skills 三域 status 回填 freshness）。

## 开发

```sh
make test
make check
make build
make smoke
```

直接验证可用 `cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。隔离写入请设置临时 `AGENT_MANAGER_DATA_DIR`，不要覆盖 HOME。

macOS 打包与独立包验收（构建环境为 Darwin x86_64，构建自动启用源码根目录与主目录路径脱敏重映射；arm64 为交叉构建，仅静态验收）：

- 打包 app 与 dmg：`pnpm --dir app run build:macos`（产物位于 `app/src-tauri/target/release/bundle/dmg/`）
- 独立包运行验收：`pnpm --dir app run verify:macos` 或 `./scripts/verify-macos-dmg.sh app/src-tauri/target/release/bundle/dmg/Agent\ Manager_0.1.0_x64.dmg`（只读挂载、隔离复制、元数据/架构/无 Python 依赖校验、源码路径脱敏校验、从 `/` 隔离目录 5 秒存活运行验收）。历史验收（2026-09-14）：基于 `5ae995b` 的干净 Darwin x86_64 DMG（SHA256 `71a386d6ab8e1a7f85ef59c82f379411e9aa1dccaa3c3166309b96b971fce26b`，7632599 bytes，非仓库 HEAD 构建包）。当前验收（2026-09-15 主审独立验收）：验收基线提交 `5a110be3` 的 Darwin x86_64 DMG verify 脚本 exit 0，包内构建身份 commit/date 与源码精确一致，未签名，SHA256 `076d16b098ede8915802c0bc6dceec42dc06e0dcadddd137e7865681c7e90a68`，7632606 bytes；同源码交叉构建 thin arm64 DMG 静态验收通过，SHA256 `a9540a2690b9d2ede16ef627f310e277ca49130e608c518b876aeaed0bbfd3f8`，7298163 bytes，adhoc linker-signed（非 Developer ID），未在 arm64 设备运行。当前 x86_64 安装包主要真实 GUI E2E 已验收：system:audit 缺库零写、Rules 取消零写/应用、Hooks 双目标、Aliases 双目标、MCP stdio+远程 HTTP 的 plan/confirm/apply、Secrets 仅显示变量名与存在性、Runs、外部修改 plan fail-closed；MCP 定义经共用 Rust CLI 写入隔离 SQLite 测试夹具，MCP 新增服务表单因 macOS 自动化输入限制未验收。签名、公证、Linux、Windows、arm64 真机运行与安装、updater 真实签名下载安装回读仍未验证（2026-09-15 复测：GitHub Actions secrets=0、variables=0，本机有效 codesigning identities=0，GitHub Releases=0），未创建 tag、GitHub Release 或对外安装包。

## 目录

- [engine-rs](engine-rs/)：CLI/Tauri 共用 Rust 库和 SQLite 存储。
- [app](app/)：Tauri 桌面应用和 React 前端。
- [contract](contract/)：Engine Contract v1 schema。
- [当前架构](docs/ARCHITECTURE.md) · [平台状态](docs/GUI-PLATFORM-STATUS.md)。
