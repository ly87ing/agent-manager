# GUI 重设计：当前契约

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置写入 `app_settings` 表，定义和本机 policy 写入 SQLite。内置默认值由 Rust 编译提供，只读；Policy UI 只显示 builtin 与 sqlite，写入固定使用 `--layer local`。

查询、预览和 dry-run 必须零写；缺库读取不能创建数据库。表单保存只保存定义，不执行安装、卸载、链接或刷新。执行按钮只来自引擎 actions，写操作遵循 plan → 确认 → apply。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除。移除管理定义与卸载软件是不同动作。

Claude Code 和 Codex 的内置默认渠道均为官方 `native` 命令；brew 只在 observed 检测结果中展示，不是默认。

SQLite 核心、Tauri bridge、设置与 Policy 页面、主要资源页面布局、Runs SQLite 操作记录和 Python 清理已完成。Agent/MCP/Skills 原生域尚未接线，未跟踪草稿不算完成；Rules、Hooks 和独立包真实 E2E 待完成。

详见 [当前架构](ARCHITECTURE.md)、[平台状态](GUI-PLATFORM-STATUS.md) 和 [Engine Contract](../contract/README.md)。
