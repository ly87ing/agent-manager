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

## 默认渠道与状态

Claude Code 和 Codex 的内置默认都是官方 `native` 命令；brew 只作为实际检测来源，不改变默认。

SQLite 核心、Tauri bridge、设置与 Policy 页面、主要资源页面布局和 Python 清理已完成。Agent/MCP/Skills 原生域尚未接线；Runs 页面仍需从文件日志模型切换到 SQLite `operation_logs`；Rules/Hooks 和独立包真实 E2E 待完成。详见 [契约](../contract/README.md) 与 [平台状态](GUI-PLATFORM-STATUS.md)。
