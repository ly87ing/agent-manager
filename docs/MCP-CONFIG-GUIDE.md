# MCP 配置指南

当前 MCP 定义、库分配和启用偏好的事实源是本机 `native-manager.db`。设置位于 `app_settings` 表；不要把 JSON 文件当作 Agent Manager 的可写状态。

SQLite 核心、Tauri bridge 和 MCP UI 已完成，但 MCP 原生域尚未注册到 Rust dispatcher；status/plan/apply 入口目前不代表真实执行能力，未跟踪草稿不算接线完成。

MCP 库和逐 Agent MCP 要求新增、编辑/更新、启用、禁用、删除。移除管理定义不等于卸载外部软件；卸载必须是引擎单独返回并单独确认的高风险 action。

表单提交只写 SQLite 定义。执行先 `mcp:plan` 预览，再经用户确认调用 `mcp:apply`；按钮只按引擎 actions 渲染。

查询、plan 和 dry-run 零写；policy 写入固定使用 `--layer local`，返回持久化来源为 `sqlite`。Claude Code 与 Codex 默认官方渠道为 `native`，brew 只表示检测到的实际来源。

完成原生域后，需要在隔离数据目录验证完整 CRUD、启停、plan/apply、失败回滚、dry-run 和外部修改保护。
