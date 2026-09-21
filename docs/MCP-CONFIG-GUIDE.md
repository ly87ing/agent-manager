# MCP 配置指南

当前 MCP 定义、库分配和启用偏好的事实源是本机 `native-manager.db`。设置位于 `app_settings` 表；不要把 JSON 文件当作 Agent Manager 的可写状态。

Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`；公共 `mcp:plan` 与 `--dry-run` 入口已退役并拒绝。定义与分配写入 SQLite；当前产品支持 Claude JSON 与 Codex TOML，旧 OpenCode codec 仅用于兼容既有数据。远程定义含 `url`（必填；仅允许 `https://`，或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符，不超过 20 个），与 command/args/env/startup_timeout_sec 互斥；headers 以客户端原生引用形式写入目标，解析后明文不落盘；受管标记统一为条目级 `_MANAGED_BY`（stdio 旧位置 env 内标记继续兼容）。

逐 Agent MCP 支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。移除管理定义不等于卸载外部软件；卸载必须是引擎单独返回并单独确认的高风险 action。

表单提交只写 SQLite 定义，不执行安装或同步。执行按钮直接触发真实 `mcp:apply`；`apply` 内部先做 preflight/写前校验，再按精确 scope、排他锁、外部修改检测、CAS/条件回滚执行并审计；普通同步、启停、更新不二次确认。

`mcp:status` 只读零写；真实 `apply` 具备排他锁、操作审计、CAS 写入和回滚保护。MCP `env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；变量取值另存 `secret_values` 表（用户在界面上填写，明文，仅 apply 渲染目标配置时使用，并随配置备份一同导出——那份备份文件因此等同凭据文件）；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude 或 Codex 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。policy 写入固定使用 `--layer local`，返回持久化来源为 `sqlite`。Claude Code 与 Codex 默认官方渠道为 `native`，brew 只表示检测到的实际来源。

Rules 原生 `status/apply` 与规则专用 `sources:get/set` 已接线；Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。`system:audit` 已接入 Rust dispatcher：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，按「先本机已保存的取值、再回退当前进程环境」判断该变量能否取到值，输出变量名与布尔（另给 `stored` 说明取值是否在本机库里）；`secrets:set` 写入取值，`secret_values` 表以明文保存，仅 apply 渲染目标配置时使用；不扫描、定位或克隆外部密钥仓库，任何路径都不序列化或显示取值本身；畸形定义固定脱敏失败。设置页高级管理可达 Aliases 与 Secrets 两页，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness）。验收基线提交 `5a110be3` 的候选包已完成主要真实 GUI E2E（旧版 plan→confirm 交互，现已失效，不证明本轮直接 apply 新交互）；2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。
