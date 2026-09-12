# Rust 原生 GUI 目标与边界

## 目标

让 CLI 与 Tauri 共享一个 Rust 原生库，并让 `native-manager.db` 成为本工具定义、分配和偏好的唯一事实源。设置保存在 `app_settings` 表，不引入可写 JSON settings 或 profile 层。

## 已完成

- `engine-rs` 的 native/store/envelope 核心服务 CLI 与 Tauri。
- SQLite 缺库读取零写、事务保存和重开读回。
- Tauri bridge 的命令、参数、run id 和异步边界。
- Policy 的 builtin/sqlite 来源和固定 `--layer local` 写入。
- Claude Code、Codex 官方 `native` 默认渠道；brew 只按实际检测来源处理。
- 根仓库 Python 与旧 `manage.sh` 入口清理。
- Runs 列表与详情使用 SQLite `operation_logs`，新增操作记录时清理过期记录；本地结构化诊断日志基于 `data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 日切写入（严格脱敏、Unix 0700/0600、只读零写、拒随软链接），在诊断日志写入后及应用启动时清理过期文件。两者共用 `log_retention_days`（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空任务记录，设置页支持 `logs:clear` 二次确认清理诊断日志，两类清理互不影响。
- Agent 原生定义、安装来源探测、生命周期 plan/apply、跨进程锁、结果核验与操作记录。
- Skills 原生域接线与完整闭环：内置集合只读默认，首次本地写入 `native-manager.db` 后本地优先；集合与 Skill×Agent 独立更新/启停/移除；内容寻址快照与安全软连接落地及回滚保护；status/plan/dry-run 零写保证；真实 apply 跨进程锁与操作审计。
- MCP 原生域接线与本地 stdio 闭环：Rust dispatcher 提供 `mcp:status`、`mcp:plan`、`mcp:apply`；定义与分配保存在 SQLite，支持 Claude JSON、Codex TOML、OpenCode JSON（需配置对应 Agent target）；逐助手支持 add/update/enable/disable/remove，库级支持 enable/disable/remove；status/plan/dry-run 零写，真实 apply 使用锁、审计、CAS 与回滚；env 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。
- `sources:get/set` 原生接线：规则来源固定为 `rules.shared`、`rules.codex`、`rules.claude` 三项，保存使用 revision CAS 且只写 SQLite，不执行任何同步动作。
- Rules 原生域接线：Rust dispatcher 提供 `rules:status`、`rules:plan`、`rules:apply`；固定目标为 `rules:codex_global`（`~/.codex/AGENTS.md`）与 `rules:claude_global`（`~/.claude/CLAUDE.md`），内容按共享与助手专用来源渲染；status/plan/dry-run 零写，真实 apply 使用全局排他锁、操作审计、文件字节 CAS 与事务/文件回滚，外部修改 fail-closed 拒绝覆盖，不承诺备份；不再有 rules/dist 构建链。Tauri bridge 参数白名单与 Rules 页面已对齐真实 schema，并支持逐目标 `--only`。
- Hooks 原生域接线：Rust dispatcher 提供 `hooks:status`、`hooks:plan`、`hooks:apply`；hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）使用 revision CAS 只写 SQLite；固定 Claude `~/.claude/settings.json` signature-set 合并（保留用户字段与非托管 hook）与 Codex `~/.codex/hooks.json` whole-file 两目标；status/plan/dry-run 零写，真实 apply 使用全局排他锁、Immediate 事务、精确旧字节 CAS、binding、审计与条件回滚，外部修改 fail-closed；前端两个目标与两个 JSON 编辑入口已接线。

## 契约要求

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 必须支持新增、编辑/更新、启用、禁用、删除。保存定义不得执行动作；UI 按引擎 actions 遵循 plan → confirm → apply；移除管理与卸载分开。

## 尚未完成

远程 HTTP MCP 尚未实现。`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks 与 runtime/storage 状态；tools/aliases/secrets 尚未接线，在审计结果中以固定 warning 呈现。设置页高级管理可达 Tools、Aliases、Secrets、更新中心和快照页面；其 `system:refresh-upstream`、`system:converge`、`tools:status/plan/apply`、`aliases:status/plan/apply`、`secrets:status`、`snapshots:list/restore` 虽存在 Tauri 白名单或 React 页面入口，但 Rust dispatcher 尚未接线，不等同后端实现，待产品裁决实现或从新版 UI 退役。当前候选安装包尚未完成包含最新 Rules/MCP/Hooks 与 `system:audit` 的真实业务 E2E；macOS 独立安装包仍使用 `pnpm --dir app run build:macos` 与 `scripts/verify-macos-dmg.sh` 校验，2026-09-09 Darwin x86_64 DMG 已通过只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收，但该包早于当前 Rules/MCP/Hooks 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks 与 `system:audit` GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

验证：`cargo fmt --manifest-path engine-rs/Cargo.toml -- --check`、两套 Cargo test/check，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。
