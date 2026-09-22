# MCP 配置指南

当前 MCP 定义、库分配和启用偏好的事实源是本机 `native-manager.db`。设置位于 `app_settings` 表；不要把 JSON 文件当作 Agent Manager 的可写状态。

Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`；公共 `mcp:plan` 与 `--dry-run` 入口已退役并拒绝。定义与分配写入 SQLite；当前产品支持 Claude JSON 与 Codex TOML，旧 OpenCode codec 仅用于兼容既有数据。远程定义含 `url`（必填；仅允许 `https://`，或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符，不超过 20 个），与 command/args/env/startup_timeout_sec 互斥；headers 以客户端原生引用形式写入目标，解析后明文不落盘；受管标记统一为条目级 `_MANAGED_BY`（stdio 旧位置 env 内标记继续兼容）。

逐 Agent MCP 支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。移除管理定义不等于卸载外部软件；卸载必须是引擎单独返回并单独确认的高风险 action。

表单提交只写 SQLite 定义，不执行安装或同步。执行按钮直接触发真实 `mcp:apply`；`apply` 内部先做 preflight/写前校验，再按精确 scope、排他锁、外部修改检测、CAS/条件回滚执行并审计；普通同步、启停、更新不二次确认。

`mcp:status` 只读零写；真实 `apply` 具备排他锁、操作审计、CAS 写入和回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化 `${VAR}` 引用。变量取值优先来自用户在「密钥」页选择的外部 `.env` 文件，找不到时回退进程环境；数据库只保存路径，配置备份不含路径或取值。未选择可用文件时，界面填写与 MCP 明文接管会 fail-closed；同名不同值拒绝接管覆盖。policy 写入固定使用 `--layer local`，返回持久化来源为 `sqlite`。Claude Code 与 Codex 默认官方渠道为 `native`，brew 只表示检测到的实际来源。

`secrets:status` 只读报告变量是否能由外部文件或进程环境解析；`secrets:set` 写外部文件，`secrets:adopt` 接管客户端已有明文，二者都不把取值写进 SQLite。旧数据库取值只能经用户确认的 `secrets:migrate-legacy` 迁移，先写文件、成功后删旧表。配置导出不含密钥或文件路径，旧版含非空 `secrets` 段的备份拒绝导入。
