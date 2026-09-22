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

定义保存只更新 SQLite，不链接或执行外部动作。`status` 只读读取状态；资源执行按钮直接触发真实 `apply`，`apply` 内部包含 preflight/写前校验、精确 scope、排他锁、外部修改检测、CAS/条件回滚与审计，并记录 operation log。

Skill、MCP 等资源及其逐助手分配支持新增、编辑/更新、启用、禁用、删除。AI CLI 普通界面只维护接入、运行参数与账号实例；安装、升级、卸载和完整定义编辑不再作为产品入口，软件生命周期由原安装渠道负责。既有生命周期字段和引擎能力第一阶段兼容保留。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

## MCP 原生域

Rust dispatcher 已接入本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；当前产品向 Claude Code 与 Codex 暴露分配入口，旧 OpenCode codec 仅用于兼容既有数据。远程定义含 `url`（仅 `https://` 或本机回环 http）与可选 `headers`（`${VAR}` 占位符引用），与 stdio 字段互斥。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`mcp:status` 只读零写；真实 `apply` 具备排他锁、操作审计、CAS 写入与回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化 `${VAR}` 引用；变量取值来自用户显式选择的外部 `.env` 文件，找不到时才回退进程环境。SQLite 只保存外部文件路径，配置备份不含路径或取值；status、诊断与操作日志不展示或保存真实值。真实 apply 按目标 codec 写入 Claude 或 Codex 的目标客户端配置，目标文件是否出现明文取决于客户端格式并由其文件权限保护。远程 HTTP 与本地 stdio 同级：headers 只持久化 `${VAR}` 占位符并以客户端原生引用形式写入目标。

## 默认渠道与状态

Claude Code 和 Codex 的内置默认都是官方 `native` 命令；brew 只作为实际检测来源，不改变默认。

`system:audit` 已接入 Rust dispatcher：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。`secrets:status` 只读输出变量名、来源与是否可解析；`secrets:source` 验证并保存外部 `.env` 路径，要求普通文件、Unix 权限 `600`、合法 `KEY=VALUE`，且不能位于应用数据目录；`secrets:set` 与 `secrets:adopt` 只写该文件，同名不同值拒绝接管覆盖。`secrets:migrate-legacy` 由用户显式触发，先写外部文件，全部成功后才删除历史 `secret_values` 表。设置页高级管理可达 Aliases 与 Secrets 两页，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，agents/mcp/skills 三域 status 回填 freshness）。

SQLite 核心、Tauri bridge、设置页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域与规则专用 `sources:get/set`、Hooks 原生域与 hooks 专用 `sources:get/set`、Runs SQLite `operation_logs` 及 Python 清理已完成。Agent 与 Skills 的 apply 均使用跨进程文件锁与结构化操作审计；Skills 内置集合只读默认并在首次写入后本地优先，集合与 Skill×Agent 均支持更新、启停与移除，并通过内容快照与安全软连接落地；status 只读零写。SQLite `operation_logs` 记录任务操作历史，并在新增操作记录时按 `log_retention_days` 清理过期记录（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空。按 2026-09-19 裁决，JSONL 结构化诊断日志与 `logs:clear` 已退役，envelope 不再带 `log_file`；错误消息出库前仍统一走脱敏。Rules 原生 `status/apply` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。旧版 DMG/GUI E2E 不证明本轮直接 apply 新交互；本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（[交付回执](goal-rust-gui.md)）。签名、公证、arm64 真机运行、Linux、Windows 和更新发布仍未验证。详见 [契约](../contract/README.md) 与 [平台状态](GUI-PLATFORM-STATUS.md)。
