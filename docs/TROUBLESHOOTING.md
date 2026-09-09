# 故障排查

本文面向当前 Rust/Tauri 实现。Agent Manager 的唯一事实源是 `native-manager.db`；设置在 `app_settings` 表。旧 Python、`manage.sh`、三层 JSON 和 profile 同步入口已退役，不是排查步骤。

## 先确认开发环境

在仓库根目录运行：

```sh
cargo --version
node --version
pnpm --version
cargo check --manifest-path engine-rs/Cargo.toml --all-targets
cargo check --manifest-path app/src-tauri/Cargo.toml --all-targets
```

前端问题在 `app/` 运行 `pnpm typecheck`、`pnpm test:unit run` 或 `pnpm build`。

## 数据库不存在或读取为空

使用隔离目录检查，不覆盖 HOME：

```sh
export AGENT_MANAGER_DATA_DIR="$(mktemp -d)"
cargo run --manifest-path engine-rs/Cargo.toml -- db:status --json
cargo run --manifest-path engine-rs/Cargo.toml -- policy:get --json
```

缺库时 `db:status` 应报告不存在，`policy:get` 应返回 Rust 内置默认值且不创建数据库。保存才会初始化 `native-manager.db`。

## 设置没有保存

设置必须通过 Tauri 设置页保存，并由 Rust bridge 写入 `app_settings` 表。检查数据目录、数据库写权限，并在保存后重新打开设置页或重启应用读回。dry-run 按约定不写入。

## Policy 来源不符合预期

Policy UI 只显示 `builtin` 和 `sqlite`。编辑请求固定使用 `--layer local`，其持久化结果显示为 `sqlite`。旧共享层、同步层和 JSON settings 不再参与有效值计算。

## 执行动作失败

保存定义不会执行安装、卸载、链接或同步。执行问题按页面显示的引擎 action 排查：

1. 确认 `status` 返回的 observed 状态。
2. 检查 `plan` 中的目标、风险和 action。
3. 用户确认后才会调用 `apply`。
4. 查看页面错误详情和 operation log；不要手动拼接旧命令。

Agent 与 Skills 原生域已接线；实际安装来源不可管理、Agent 已禁用、offline 阻断或动作不适用时，引擎不会提供或执行对应 action。Skills 集合与技能分配通过内容快照与安全软连接落地，status 与 plan 保证零写，遇外部非受管内容或冲突时 fail-closed 拒绝覆盖。MCP 原生域仍未接线，相关页面出现“未提供 action”属于已知实现边界。Rules/Hooks 统一编辑与端侧落地、日志自动保留与手动清理、独立安装包真实 E2E 仍待完成。

## 安装渠道

Claude Code 与 Codex 的内置默认渠道是官方 `native` 命令。brew 只在实际检测结果中作为来源展示，不是默认渠道；若 observed 与预期不符，应检查本机 PATH 和实际安装来源。

## 验证

```sh
make test
make check
make build
make smoke
```
