# Engine Contract v1

`contract/engine-v1/` 描述 CLI 与 Tauri 共用的 JSON envelope、状态、计划、动作和 policy 数据形状。实现入口是 `engine-rs`；根仓库不依赖 Python 或旧 shell 管线。

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置存储在 SQLite 的 `app_settings` 表。policy 只返回 `builtin` 或 `sqlite` 归属；不存在三层 JSON、可写 settings JSON 或 profile 同步层。

policy 写入请求使用 `--layer local`，其中 `local` 是写入目标参数，持久化结果来源是 `sqlite`。缺库读取和 dry-run 必须零写。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 的动作契约覆盖新增、编辑/更新、启用、禁用、删除。保存定义不执行动作；`plan` 只生成预览，`apply` 才执行用户确认的引擎 actions。移除管理和卸载是不同 action。

文件：`envelope.schema.json`（响应）、`element.schema.json`（元素和 actions）、`status.schema.json`（状态）、`plan.schema.json`（计划）、`apply-result.schema.json`（执行结果）、`converge.schema.json`（系统动作结果）、`policy.schema.json`（policy 结果）和 `runs.schema.json`（SQLite 操作记录与清空结果）。profile schema 已退役并不属于当前契约。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。
