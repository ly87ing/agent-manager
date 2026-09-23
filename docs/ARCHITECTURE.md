# 当前架构

Rust 原生库读写 `native-manager.db`，CLI 和 Tauri bridge 共同调用它，React UI 通过 bridge 展示结果并提交请求。

## 事实源

生效配置以各助手真实文件与目录为准（见 `GUI-REDESIGN.md`「数据模型：实际配置为准」）。`native-manager.db` 保存应用偏好、助手识别定义、归属记录与未生效的备选；设置位于 `app_settings` 表。没有第二份可写 JSON settings，也没有共享 profile 层。缺库读取不建库，写入通过 SQLite 事务初始化 schema 并记录 operation log。

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

MCP 以各助手配置文件里的实际条目为准（Claude `~/.claude.json` 顶层 `mcpServers`、Codex `config.toml` 的 `[mcp_servers.*]`，以及配置了 `mcp_config` 的其他助手与实例）。`mcp:status` 只读零写，直接列出各助手里的 stdio 与远程条目：env/headers 引用原样、字面量掩码，未建模的官方字段原样保留，每条给出写入基准 `base`。

`mcp:apply` 逐条 `add`、`edit`、`enable`、`disable`、`remove`、`copy`、`forget`，每次只动一个助手里的一个条目；锁内重读并核对 `--base`，外部改过则拒绝（「已在外部修改」），写前自检、原子替换、回读与条件回滚、审计只记动作与范围。停用：Codex/OpenCode 写官方 `enabled = false`；Claude Code 移出条目并存停用快照（`mcp_saved`），疑似凭据的字面量转存到用户选择的外部 `.env` 文件、快照里只留 `${VAR}`。变量取值来自外部文件，找不到时回退进程环境；SQLite 只保存外部文件路径与「已保存未生效」项，status、诊断与操作日志不展示或保存真实值。旧版服务库定义与分配表在首次 MCP 写入时迁移，迁移不改配置文件。细节见 [MCP 配置指南](MCP-CONFIG-GUIDE.md)。

## 默认渠道与状态

Claude Code 和 Codex 的内置默认都是官方 `native` 命令；brew 只作为实际检测来源，不改变默认。

`system:audit` 已接入 Rust dispatcher：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。`secrets:status` 只读输出变量名、来源与是否可解析；`secrets:source` 验证并保存外部 `.env` 路径，要求普通文件、Unix 权限 `600`、合法 `KEY=VALUE`，且不能位于应用数据目录；`secrets:set` 只写该文件；MCP 停用 Claude 条目时转存疑似凭据也只写该文件，同名不同值拒绝覆盖。`secrets:migrate-legacy` 由用户显式触发，先写外部文件，全部成功后才删除历史 `secret_values` 表。设置页「配置工具」可达客户端诊断、Aliases 与 Secrets，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，agents/mcp/skills 三域 status 回填 freshness）。

SQLite 核心、Tauri bridge、设置页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域与规则专用 `sources:get/set`、Hooks 原生域与 hooks 专用 `sources:get/set`、Runs SQLite `operation_logs` 及 Python 清理已完成。Agent 与 Skills 的 apply 均使用跨进程文件锁与结构化操作审计；内置技能仓库只作推荐来源，添加后才成为用户集合；集合与 Skill×Agent 均支持更新、启停与移除，并通过内容快照与安全软连接落地；status 只读零写。SQLite `operation_logs` 记录任务操作历史，并在新增操作记录时按 `log_retention_days` 清理过期记录（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空。按 2026-09-19 裁决，JSONL 结构化诊断日志与 `logs:clear` 已退役，envelope 不再带 `log_file`；错误消息出库前仍统一走脱敏。Rules 原生 `status/apply` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。旧版 DMG/GUI E2E 不证明本轮直接 apply 新交互；本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（[交付回执](goal-rust-gui.md)）。签名、公证、arm64 真机运行、Linux、Windows 和更新发布仍未验证。详见 [契约](../contract/README.md) 与 [平台状态](GUI-PLATFORM-STATUS.md)。
