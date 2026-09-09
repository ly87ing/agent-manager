# GUI 重设计：当前契约

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置写入 `app_settings` 表，定义和本机 policy 写入 SQLite。内置默认值由 Rust 编译提供，只读；Policy UI 只显示 builtin 与 sqlite，写入固定使用 `--layer local`。

查询、预览和 dry-run 必须零写；缺库读取不能创建数据库。表单保存只保存定义，不执行安装、卸载、链接或刷新。执行按钮只来自引擎 actions，写操作遵循 plan → 确认 → apply。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除。移除管理定义与卸载软件是不同动作。

Claude Code 和 Codex 的内置默认渠道均为官方 `native` 命令；brew 只在 observed 检测结果中展示，不是默认。

SQLite 核心、Tauri bridge、设置与 Policy 页面、主要资源页面布局、Runs SQLite 操作记录与清理、Agent 原生生命周期、Skills 原生域及 Python 清理已完成。Skills 内置集合只读默认，首次编辑存入 `native-manager.db` 后本地优先；集合与 Skill×Agent 均支持独立更新、启停和移除；启用通过内容寻址快照与安全软连接落地；status/plan 保持零写，真实 apply 具备进程锁与操作审计。任务记录基于 `operation_logs` 实现了设置项保留天数自动清理（0 为不自动清理）与二次确认手动清空（清空自身零审计且不删库改配置；当前 `data/logs` 无写出器，不虚构文件日志压缩）。MCP 原生域、Rules/Hooks 统一编辑与端侧落地、独立包真实 E2E 待完成。

详见 [当前架构](ARCHITECTURE.md)、[平台状态](GUI-PLATFORM-STATUS.md) 和 [Engine Contract](../contract/README.md)。
