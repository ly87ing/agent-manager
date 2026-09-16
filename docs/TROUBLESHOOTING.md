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

设置必须通过 Tauri 设置页保存，并由 Rust bridge 写入 `app_settings` 表。检查数据目录、数据库写权限，并在保存后重新打开设置页或重启应用读回。查询与 status 只读零写。

## Policy 来源不符合预期

Policy UI 只显示 `builtin` 和 `sqlite`。编辑请求固定使用 `--layer local`，其持久化结果显示为 `sqlite`。旧共享层、同步层和 JSON settings 不再参与有效值计算。

## 执行动作失败

保存定义不会执行安装、卸载、链接或同步。执行问题按页面显示的引擎 action 排查：

1. 确认 `status` 返回的 observed 状态。
2. 执行按钮点击即真实 `apply`；`apply` 内部 preflight（写前校验）检测不适用、未接管与外部改动并 fail-closed，envelope 错误会给出拒绝类别。
3. 查看页面错误详情和 operation log；不要手动拼接旧命令。

确认弹窗只用于定义移除、清空日志等破坏性动作；普通同步、启停、更新没有二次确认步骤。

Agent、Skills、MCP、Rules 和 Hooks 原生域已接线；实际安装来源不可管理、Agent 已禁用、offline 阻断或动作不适用时，引擎不会提供或执行对应 action。Skills 集合与技能分配通过内容快照与安全软连接落地，status 只读零写，遇外部非受管内容或冲突时 fail-closed 拒绝覆盖。MCP 支持本地 stdio 与远程 HTTP，dispatcher 提供 `mcp:status`（只读）与真实 `mcp:apply`；codec 支持 Claude JSON、Codex TOML、OpenCode JSON，OpenCode 需配置对应 Agent target，`env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP 定义含 `url`（仅 `https://` 或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（仅 `${VAR}` 占位符，引用形式写入、明文不落盘），与 stdio 字段互斥。Runs 操作记录基于 SQLite `operation_logs` 提供历史审计与二次确认清空；结构化诊断日志基于 `data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 提供元数据排查与设置页二次确认清理；两者保留期由 `log_retention_days` 统一控制。Rules 原生 `status/apply` 与规则专用 `sources:get/set` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝；执行前请用 `<domain>:status` 确认状态。

验收基线提交 `5a110be3` 的候选包只证明旧版 plan→confirm 交互，不证明本轮直接 apply 新交互。本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；最终 clean DMG 在最终提交后重建并记录于交付回执。2026-09-09 Darwin x86_64 DMG 已通过只读挂载、仓库外复制、无 Python、路径脱敏和从 `/` 隔离环境 5 秒启动验收；该包早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。设置页高级管理可达 Aliases、更新中心等页面，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness）。

## 安装渠道

Claude Code 与 Codex 的内置默认渠道是官方 `native` 命令。brew 只在实际检测结果中作为来源展示，不是默认渠道；若 observed 与预期不符，应检查本机 PATH 和实际安装来源。

## 验证

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
./scripts/ci-smoke.sh
```
