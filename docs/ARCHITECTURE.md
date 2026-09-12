# 当前架构

Rust 原生库读写 `native-manager.db`，CLI 和 Tauri bridge 共同调用它，React UI 通过 bridge 展示结果并提交请求。

## 唯一事实源

`native-manager.db` 保存本工具的定义、分配、policy 和应用偏好；设置位于 `app_settings` 表。没有第二份可写 JSON settings，也没有共享 profile 层。缺库读取不建库，写入通过 SQLite 事务初始化 schema 并记录 operation log。

内置 policy 是编译进 Rust 的只读默认值。`policy:get` 只返回 builtin 与 sqlite 归属；写入参数固定为 `--layer local`，持久化层名为 `sqlite`。

## 调用边界

```text
CLI ───────┐
           ├── engine-rs (native / store / envelope) ── native-manager.db
Tauri ─────┘
React UI ── Tauri commands
```

入口是 `execute(Context, Request) -> Envelope`。Tauri 先验证命令、参数和 run id，再在阻塞线程调用共享库。

## 保存与执行

定义保存只更新 SQLite，不安装、卸载、链接或执行外部动作。`status` 读取状态，`plan` 生成 actions，用户确认后 `apply` 执行并记录日志。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 的契约动作均要求新增、编辑/更新、启用、禁用、删除。移除管理与卸载必须分开表达和确认。

## MCP 原生域

Rust dispatcher 已接入本地 stdio MCP 的 `mcp:status`、`mcp:plan` 和 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；codec 支持 Claude JSON、Codex TOML 和 OpenCode JSON，OpenCode 需要在对应 Agent definition 中配置 `mcp_config` 目标。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`status`、`plan` 和 `mcp:apply --dry-run` 零写；真实 `apply` 具备排他锁、操作审计、CAS 写入与回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP MCP 尚未实现。

## 默认渠道与状态

Claude Code 和 Codex 的内置默认都是官方 `native` 命令；brew 只作为实际检测来源，不改变默认。

`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks 与 runtime/storage 状态；tools/aliases/secrets 尚未接线，在审计结果中以固定 warning 呈现。设置页高级管理可达 Tools、Aliases、Secrets、更新中心和快照页面；其 `system:refresh-upstream`、`system:converge`、`tools:status/plan/apply`、`aliases:status/plan/apply`、`secrets:status`、`snapshots:list/restore` 虽存在 Tauri 白名单或 React 页面入口，但 Rust dispatcher 尚未接线，不等同后端实现，待产品裁决实现或从新版 UI 退役。

SQLite 核心、Tauri bridge、设置与 Policy 页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域与规则专用 `sources:get/set`、Hooks 原生域与 hooks 专用 `sources:get/set`、Runs SQLite `operation_logs`、结构化诊断日志及 Python 清理已完成。Agent 与 Skills 的 apply 均使用跨进程文件锁与结构化操作审计；Skills 内置集合只读默认并在首次写入后本地优先，集合与 Skill×Agent 均支持更新、启停与移除，并通过内容快照与安全软连接落地；status 与 plan 全程零写。两类日志协同运作：SQLite `operation_logs` 记录任务操作历史，并在新增操作记录时清理过期记录；`data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 记录结构化诊断元数据（严格脱敏、Unix 0700/0600、只读零写、拒随软链接），并在诊断日志写入后及应用启动时清理过期文件。两类日志共用 `log_retention_days`（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空任务记录，设置页支持 `logs:clear` 二次确认清理诊断日志，两类手动清理互不影响。Rules 原生 `status/plan/apply` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/plan/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 Rules/MCP/Hooks 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks 与 `system:audit` GUI 业务闭环；签名、公证、arm64、Linux、Windows 和更新发布仍未验证。详见 [契约](../contract/README.md) 与 [平台状态](GUI-PLATFORM-STATUS.md)。
