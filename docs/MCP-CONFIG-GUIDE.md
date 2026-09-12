# MCP 配置指南

当前 MCP 定义、库分配和启用偏好的事实源是本机 `native-manager.db`。设置位于 `app_settings` 表；不要把 JSON 文件当作 Agent Manager 的可写状态。

Rust dispatcher 已实现本地 stdio MCP 的 `mcp:status`、`mcp:plan` 和 `mcp:apply`。定义与分配写入 SQLite；支持 Claude JSON、Codex TOML、OpenCode JSON，OpenCode 必须为对应 Agent 配置 `mcp_config` 目标。远程 HTTP MCP 尚未实现。

逐 Agent MCP 支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。移除管理定义不等于卸载外部软件；卸载必须是引擎单独返回并单独确认的高风险 action。

表单提交只写 SQLite 定义。执行先 `mcp:plan` 预览，再经用户确认调用 `mcp:apply`；按钮只按引擎 actions 渲染。

`status`、`plan` 和 `mcp:apply --dry-run` 零写；真实 `apply` 具备排他锁、操作审计、CAS 写入和回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。policy 写入固定使用 `--layer local`，返回持久化来源为 `sqlite`。Claude Code 与 Codex 默认官方渠道为 `native`，brew 只表示检测到的实际来源。

Rules 原生 `status/plan/apply` 与规则专用 `sources:get/set` 已接线；Hooks 原生 status/plan/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。设置页高级管理可达总览、Tools、Aliases、Secrets、更新中心和快照页面；其 `system:audit`、`system:refresh-upstream`、`system:converge`、`tools:status/plan/apply`、`aliases:status/plan/apply`、`secrets:status`、`snapshots:list/restore` 虽存在 Tauri 白名单或 React 页面入口，但 Rust dispatcher 尚未接线，不等同后端实现，待产品裁决实现或从新版 UI 退役。当前候选安装包尚未完成包含最新 Rules/MCP/Hooks 的真实业务 E2E；2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 Rules/MCP/Hooks 实现，不能证明当前 Rules/MCP/Hooks GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。
