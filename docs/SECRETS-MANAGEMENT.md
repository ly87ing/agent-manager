# Secrets 边界

本页描述当前 Agent Manager 的边界。`native-manager.db` 是本工具定义、分配和偏好的唯一事实源；它不保存外部 secrets 仓库的明文凭据。设置位于 SQLite 的 `app_settings` 表。

## 当前规则

- 真实凭据只通过用户环境或外部私有 secrets 仓库提供；Agent Manager 不把明文写入根仓库、契约、日志或测试产物。apply 可能按目标 codec 将解析后的值写入用户目标配置，安全性依赖该文件权限和客户端安全边界。
- SQLite definition/binding、status、诊断和操作日志不保存或展示解析后的真实值；apply 生成的目标客户端配置可能包含解析后的值，安全性依赖文件权限和客户端安全边界。
- Agent Manager 的 SQLite definition/binding 只保存字面量 `${VAR}` 占位符；外部注入由用户环境与目标客户端负责。apply 生成的目标客户端配置可能包含解析后的值。
- 运行日志和错误响应不得包含 token、私钥、完整连接串或其它敏感值。
- 查询与 status 只读零写；公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

## MCP 与 Agent 配置

MCP 定义、库分配和启用偏好由本机 `native-manager.db` 管理。Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`。支持 Claude JSON、Codex TOML、OpenCode JSON，OpenCode 需配置对应 Agent target。定义保存与执行动作分开；执行按钮直接触发真实 `apply`，`apply` 内部先做 preflight/写前校验，再按精确 scope、锁、外部修改检测、CAS/条件回滚执行并审计。

逐 Agent 支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`mcp:status` 只读零写；真实 `apply` 具备锁、审计、CAS 和回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP MCP 的 `headers` 与 `env` 同级：只持久化 `${VAR}` 占位符，三 codec 以引用形式写入目标（Claude `${VAR}`、Codex `env_http_headers`、OpenCode `{env:VAR}`），任何解析后明文不写入目标文件。

Rules 原生 `status/apply` 与规则专用 `sources:get/set` 已接线；Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。`system:audit` 已接入 Rust dispatcher 及其 Tauri offline 桥接：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。设置页高级管理可达 Aliases、更新中心等页面，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness）。验收基线提交 `5a110be3` 的候选包已完成主要真实 GUI E2E（旧版 plan→confirm 交互，现已失效，不证明本轮直接 apply 新交互）；2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

Claude Code 与 Codex 的内置默认安装渠道是官方 `native` 命令。brew 只作为实际检测来源展示，不是默认渠道；凭据本身仍由对应工具和用户环境管理。

## 验证与报告

使用隔离的 `AGENT_MANAGER_DATA_DIR` 做本地验证，不覆盖 HOME。提交前运行：

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

若错误输出疑似包含敏感信息，应立即停止传播，清理临时产物，并改用脱敏后的错误类别和路径报告。
