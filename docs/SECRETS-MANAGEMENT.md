# Secrets 边界

本页描述当前 Agent Manager 的边界。生效配置以各助手真实文件为准，`native-manager.db` 只保存偏好、归属记录与未生效的备选；它不保存外部 secrets 仓库的明文凭据。设置位于 SQLite 的 `app_settings` 表。

## 当前规则

- 真实凭据只通过用户明确选择的外部私有 `.env` 文件或进程环境提供。Agent Manager 不把明文写入根仓库、SQLite、日志、配置备份或测试产物。
- SQLite definition/binding 只保存 `${VAR}` 占位符，`app_settings` 只保存外部文件路径。status、诊断和操作日志只报告变量名、来源与布尔状态，不保存或展示真实值。
- 外部文件必须已存在、是普通文件、Unix 权限为 `600`、内容为合法 `KEY=VALUE`，且不能位于应用数据目录。应用不会自动克隆、创建或猜测 secrets 仓库。
- apply 生成的目标客户端配置是否包含解析后的值取决于客户端格式，安全性依赖目标文件权限和客户端安全边界。
- 运行日志和错误响应不得包含 token、私钥、完整连接串或其它敏感值。
- 查询与 status 只读零写；公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

## MCP 与 Agent 配置

MCP 定义、库分配和启用偏好由本机 `native-manager.db` 管理。Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`。当前产品支持 Claude JSON 与 Codex TOML；旧 OpenCode codec 仅用于兼容既有数据。定义保存与执行动作分开；执行按钮直接触发真实 `apply`，`apply` 内部先做 preflight/写前校验，再按精确 scope、锁、外部修改检测、CAS/条件回滚执行并审计。

逐 Agent 支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`mcp:status` 只读零写；真实 `apply` 具备锁、审计、CAS 和回滚保护。外部文件中的取值优先，进程环境只作只读回退；没有可用外部文件时，界面填写和 MCP 明文接管 fail-closed。远程 HTTP MCP 的 `headers` 与 `env` 同级：定义中只持久化 `${VAR}` 占位符，并以客户端原生引用形式写入目标。

`secrets:source` 只保存路径；`secrets:set` 只更新指定变量；`secrets:adopt` 只补缺失值，同名不同值拒绝覆盖。旧版 `secret_values` 表只用于一次性迁移：页面显示数量但不读取回显，用户输入确认词后才执行；引擎先把全部旧值无覆盖写入外部文件，成功后再删除旧表，任一步失败都保留旧表。配置导出永不包含取值或外部文件路径，导入遇到旧版非空 `secrets` 段会拒绝。

Claude Code 与 Codex 的内置默认安装渠道是官方 `native` 命令。brew 只作为实际检测来源展示，不是默认渠道；凭据本身仍由对应工具和用户环境管理。

## 验证与报告

使用隔离的 `AGENT_MANAGER_DATA_DIR` 做本地验证，不覆盖 HOME。提交前运行：

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

若错误输出疑似包含敏感信息，应立即停止传播，清理临时产物，并改用脱敏后的错误类别和路径报告。
