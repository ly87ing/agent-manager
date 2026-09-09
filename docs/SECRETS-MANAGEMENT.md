# Secrets 边界

本页描述当前 Agent Manager 的边界。`native-manager.db` 是本工具定义、分配和偏好的唯一事实源；它不保存外部 secrets 仓库的明文凭据。设置位于 SQLite 的 `app_settings` 表。

## 当前规则

- 真实凭据只存放在用户或外部私有 secrets 仓库，不写入根仓库、契约、日志或测试产物。
- Agent Manager 不把 Claude Code、Codex 或 MCP 的凭据注入用户配置。
- 配置中的凭据位置只能使用占位符或外部注入机制。
- 运行日志和错误响应不得包含 token、私钥、完整连接串或其它敏感值。
- dry-run、plan 和查询不得写入数据库或外部配置。

## MCP 与 Agent 配置

MCP 定义、库分配和启用偏好由本机 `native-manager.db` 管理。定义保存与执行动作分开；执行按钮只按引擎 actions，并遵循 plan → 确认 → apply。MCP 原生域当前尚未接线，未跟踪草稿不构成完成。

Claude Code 与 Codex 的内置默认安装渠道是官方 `native` 命令。brew 只作为实际检测来源展示，不是默认渠道。凭据本身仍由对应工具和用户环境管理。

## 验证与报告

使用隔离的 `AGENT_MANAGER_DATA_DIR` 做本地验证，不覆盖 HOME。提交前运行：

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

若错误输出疑似包含敏感信息，应立即停止传播，清理临时产物，并改用脱敏后的错误类别和路径报告。
