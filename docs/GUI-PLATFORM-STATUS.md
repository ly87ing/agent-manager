# GUI 平台状态

本页以当前分支的真实 Rust/Tauri 实现为基准；只有已接线并经过对应验证的能力才标记为完成。

| 范围 | 状态 |
| --- | --- |
| SQLite 核心 | 已完成：`native-manager.db`、settings、policy、runs 和读写边界 |
| CLI / Tauri 共用库 | 已完成：两者调用同一 `engine-rs` |
| Tauri bridge / React 基础 | 已完成：原生桥接、设置与 policy 页面、主要资源页面布局和 plan/apply 交互 |
| Python 清理 | 已完成：根仓库不再依赖 Python 或 `manage.sh` |
| Agent 原生域 | 已完成：定义、实际来源、状态、plan/apply、跨进程锁、结果核验和操作记录 |
| MCP/Skills 原生域 | 未完成：草稿尚未注册 dispatcher |
| Runs 操作记录 | 已完成：列表与详情均读取 SQLite `operation_logs` |
| Rules/Hooks | 未完成 |
| 独立包真实 E2E | 未完成 |
| 跨平台发布、签名、更新 | 未完成 |

缺库读取不建库，保存才初始化数据库；设置在 `app_settings` 表。policy 持久化来源为 `sqlite`，写入参数固定 `--layer local`。Claude Code 与 Codex 默认均为官方 `native` 命令，brew 仅按实际检测来源处理。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。这些不替代未完成域和发布包 E2E。
