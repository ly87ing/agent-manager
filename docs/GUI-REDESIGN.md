# GUI 重设计：当前契约

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置写入 `app_settings` 表，定义和本机 policy 写入 SQLite。内置默认值由 Rust 编译提供，只读；Policy UI 只显示 builtin 与 sqlite，写入固定使用 `--layer local`。

查询、预览和 dry-run 必须零写；缺库读取不能创建数据库。表单保存只保存定义，不执行安装、卸载、链接或刷新。执行按钮只来自引擎 actions，写操作遵循 plan → 确认 → apply。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除。移除管理定义与卸载软件是不同动作。

## MCP 原生域状态

Rust dispatcher 已实现本地 stdio MCP 的 `mcp:status`、`mcp:plan` 和 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；支持 Claude JSON、Codex TOML 和 OpenCode JSON，OpenCode 需在对应 Agent definition 配置 `mcp_config` 目标。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`status`、`plan` 和 `mcp:apply --dry-run` 零写；真实 `apply` 具备排他锁、操作审计、CAS 和回滚保护；`env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP MCP 尚未实现。

Claude Code 和 Codex 的内置默认渠道均为官方 `native` 命令；brew 只在 observed 检测结果中展示，不是默认。

设置页高级管理可达总览、Tools、Aliases、Secrets、更新中心和快照页面；其 `system:audit`、`system:refresh-upstream`、`system:converge`、`tools:status/plan/apply`、`aliases:status/plan/apply`、`secrets:status`、`snapshots:list/restore` 虽存在 Tauri 白名单或 React 页面入口，但 Rust dispatcher 尚未接线，不等同后端实现，待产品裁决实现或从新版 UI 退役。

SQLite 核心、Tauri bridge、设置与 Policy 页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Runs SQLite 操作记录、结构化诊断日志及 Python 清理已完成。Skills 内置集合只读默认，首次编辑存入 `native-manager.db` 后本地优先；集合与 Skill×Agent 均支持独立更新、启停和移除；启用通过内容寻址快照与安全软连接落地；status/plan 保持零写，真实 apply 具备进程锁与操作审计。任务记录基于 `operation_logs` 审计实际操作；结构化诊断日志按日写入 `data_dir/logs`（严格脱敏、只读零写、Unix 0700/0600）；`log_retention_days` 统一管理两类日志保留期（0 为不自动清理）；执行中心与设置页分别提供二次确认清空任务记录和清理诊断日志能力，两类手动清理互不交叉。Rules/Hooks 原生 `status/plan/apply` 与 `sources:get/set` 尚未接线。当前候选安装包尚未完成真实业务 E2E；2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 MCP 实现，不能证明当前 MCP GUI 业务闭环；签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

详见 [当前架构](ARCHITECTURE.md)、[平台状态](GUI-PLATFORM-STATUS.md) 和 [Engine Contract](../contract/README.md)。
