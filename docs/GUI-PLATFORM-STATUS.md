# GUI 平台状态

本页以当前分支的真实 Rust/Tauri 实现为基准；只有已接线并经过对应验证的能力才标记为完成。

| 范围 | 状态 |
| --- | --- |
| SQLite 核心 | 已完成：`native-manager.db`、settings、policy、runs 和读写边界 |
| CLI / Tauri 共用库 | 已完成：两者调用同一 `engine-rs` |
| Tauri bridge / React 基础 | 已完成：原生桥接、设置与 policy 页面、主要资源页面布局和 plan/apply 交互 |
| Python 清理 | 已完成：根仓库不再依赖 Python 或 `manage.sh` |
| Agent 原生域 | 已完成：定义、实际来源、状态、plan/apply、跨进程锁、结果核验和操作记录 |
| Skills 原生域 | 已完成：已接入 dispatcher；内置默认只读、写入进 SQLite；集合与单技能支持更新/启停/移除；内容快照与安全软连接；零写预览与 apply 排他锁/审计 |
| Runs 操作记录与清理 | 已完成：列表与详情读取 SQLite `operation_logs`；新增操作记录时按设置（`log_retention_days`，0 为不清理）删除过期记录；提供 `runs:clear` 二次确认手动清空（清空自身零审计且不删库改配置） |
| 本地结构化诊断日志与清理 | 已完成：`data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 日切写入；仅记录脱敏元数据；Unix 权限 0700/0600；只读与演练零写；诊断日志写入后及应用启动时按 `log_retention_days` 清理过期文件；提供 `logs:clear` 原生命令并在设置页支持二次确认手动清空（不碰 SQLite 任务记录） |
| MCP 原生域 | 未完成：尚未接入 dispatcher |
| Rules/Hooks 统一编辑与端侧落地 | 未完成 |
| 独立包真实 E2E | 未完成 |
| 跨平台发布、签名、更新 | 未完成 |

缺库读取不建库，保存才初始化数据库；设置在 `app_settings` 表。policy 持久化来源为 `sqlite`，写入参数固定 `--layer local`。Claude Code 与 Codex 默认均为官方 `native` 命令，brew 仅按实际检测来源处理。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。这些不替代未完成域和发布包 E2E。
