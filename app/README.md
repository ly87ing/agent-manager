# Agent Manager 桌面应用

本目录是 Tauri 2 应用。React UI 通过 bridge 调用 `engine-rs`；CLI 与桌面端共享同一 Rust 原生库。运行时不依赖 Python、`manage.sh` 或源码树资源。

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源，设置在 `app_settings` 表。查询与 status 只读零写；定义保存只写 SQLite。

Claude Code 与 Codex 的内置默认渠道是官方 `native` 命令。Homebrew 仅作为检测到的实际来源展示，不是默认。

保存定义不执行安装或链接。执行按钮直接触发真实 `apply`；`apply` 内部先做 preflight/写前校验，再按精确 scope、排他锁、外部修改检测、CAS/条件回滚执行并审计；普通同步、启停、更新不二次确认。管理删除（移除定义）与卸载软件是不同动作，破坏性操作保持独立确认。

Agent、MCP、Skills 页面已按目标交互组织；Agent、Skills 和 MCP 生命周期已接入 Rust 原生域。Skills 支持集合与单技能更新、启停与移除，采用内容快照与安全软连接落地，status 只读零写。SQLite 核心、Tauri bridge、设置与 Policy 页面、Runs SQLite 操作记录与清理、本地 JSONL 结构化诊断日志与清理（按日滚动、严格脱敏、Unix 0700/0600；`log_retention_days` 统一控制两类日志保留期；Runs 页二次确认清空任务记录，设置页二次确认清理诊断日志，两类清理边界分明）和 Python 清理已完成。Rules 原生 status/apply 与规则专用 `sources:get/set` 已接线（固定 Codex/Claude 两目标，外部修改 fail-closed）；Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed；验收基线提交 `5a110be3` 的候选包仅证明旧版 plan→confirm 交互。本轮直接 apply 新交互的源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（交付回执见 ../docs/goal-rust-gui.md）。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`。定义与分配保存在 `native-manager.db` 的 SQLite 表中，支持 Claude JSON、Codex TOML 和 OpenCode JSON；OpenCode 需在对应 Agent definition 配置 `mcp_config` 目标。逐助手支持 `add`、`update`、`enable`、`disable`、`remove`，库级支持 `enable`、`disable`、`remove`；`mcp:status` 只读零写，真实 `apply` 具备排他锁、操作审计、CAS 和回滚保护，`env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程定义含 `url`（仅允许 `https://` 或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符），与 stdio 字段互斥；headers 以引用形式写入（Claude `${VAR}`、Codex `env_http_headers`、OpenCode `{env:VAR}`），明文不落盘。

`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。设置页高级管理可达 Aliases、更新中心等页面，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness）。

## 本地开发

```sh
pnpm install --frozen-lockfile
pnpm test:unit run
pnpm typecheck
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml --all-targets
```

CLI 查询在仓库根目录运行。测试数据库使用临时 `AGENT_MANAGER_DATA_DIR`，不覆盖 HOME。

### macOS 打包与独立验收（限定 Darwin x86_64）

```sh
pnpm run build:macos
# 产物生成在 src-tauri/target/release/bundle/dmg/，可直接运行验证脚本或使用 npm script：
pnpm run verify:macos
# 或显式指定 DMG 路径：
../scripts/verify-macos-dmg.sh src-tauri/target/release/bundle/dmg/Agent\ Manager_0.1.0_x64.dmg
```

打包脚本自动配置 `--remap-path-prefix` 剥离本机构建 HOME 与源码根路径。验收脚本完成只读挂载、脱离仓库隔离复制、无 Python 动态与静态依赖检查、脱敏校验，以及在 `/` 下使用最小 PATH 与隔离目录的 5 秒存活、源码零引用和退出检测。2026-09-09 已在 Darwin x86_64 实机构建 DMG 并通过上述独立运行验收；该包早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环。更新的安装包验收状态见根 README（验收基线提交 `5a110be3`，旧版交互证据）。Apple 签名、公证、arm64 真机运行、Linux、Windows 和更新发布仍未验证。
