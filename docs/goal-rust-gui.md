# Rust 原生 GUI 目标与边界

## 当前结论

- 核心已就绪：Rust/SQLite/Tauri 共用库与 `native-manager.db` 唯一事实源。
- 已接线闭环：Agents、Skills、MCP、Rules、Hooks、Aliases 六个资源域，以及只读 `system:audit`、`secrets:status`。
- 2026-09-12 高级管理入口裁决已执行完毕：Aliases 原生闭环落地（`b7f30db`）；Tools、快照与 `system:converge` 退役清理完成（`9080c1d`）。
- 仍未完成：`system:refresh-upstream`、远程 HTTP MCP、当前代码的安装包与 GUI E2E、跨平台签名/公证/发布。

## 目标

让 CLI 与 Tauri 共享一个 Rust 原生库，并让 `native-manager.db` 成为本工具定义、分配和偏好的唯一事实源。设置保存在 `app_settings` 表，不引入可写 JSON settings 或 profile 层。

## 已完成

- `engine-rs` 的 native/store/envelope 核心服务 CLI 与 Tauri。
- SQLite 缺库读取零写、事务保存和重开读回。
- Tauri bridge 的命令、参数、run id 和异步边界。
- Policy 的 builtin/sqlite 来源和固定 `--layer local` 写入。
- Claude Code、Codex 官方 `native` 默认渠道；brew 只按实际检测来源处理。
- 根仓库 Python 与旧 `manage.sh` 入口清理。
- Runs 列表与详情使用 SQLite `operation_logs`，新增操作记录时清理过期记录；本地结构化诊断日志基于 `data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 日切写入（严格脱敏、Unix 0700/0600、只读零写、拒随软链接），在诊断日志写入后及应用启动时清理过期文件。两者共用 `log_retention_days`（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空任务记录，设置页支持 `logs:clear` 二次确认清理诊断日志，两类清理互不影响。
- Agent 原生定义、安装来源探测、生命周期 plan/apply、跨进程锁、结果核验与操作记录。
- Skills 原生域接线与完整闭环：内置集合只读默认，首次本地写入 `native-manager.db` 后本地优先；集合与 Skill×Agent 独立更新/启停/移除；内容寻址快照与安全软连接落地及回滚保护；status/plan/dry-run 零写保证；真实 apply 跨进程锁与操作审计。
- MCP 原生域接线与本地 stdio 闭环：Rust dispatcher 提供 `mcp:status`、`mcp:plan`、`mcp:apply`；定义与分配保存在 SQLite，支持 Claude JSON、Codex TOML、OpenCode JSON（需配置对应 Agent target）；逐助手支持 add/update/enable/disable/remove，库级支持 enable/disable/remove；status/plan/dry-run 零写，真实 apply 使用锁、审计、CAS 与回滚；env 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。
- `sources:get/set` 原生接线：规则来源固定为 `rules.shared`、`rules.codex`、`rules.claude` 三项，保存使用 revision CAS 且只写 SQLite，不执行任何同步动作。
- Rules 原生域接线：Rust dispatcher 提供 `rules:status`、`rules:plan`、`rules:apply`；固定目标为 `rules:codex_global`（`~/.codex/AGENTS.md`）与 `rules:claude_global`（`~/.claude/CLAUDE.md`），内容按共享与助手专用来源渲染；status/plan/dry-run 零写，真实 apply 使用全局排他锁、操作审计、文件字节 CAS 与事务/文件回滚，外部修改 fail-closed 拒绝覆盖，不承诺备份；不再有 rules/dist 构建链。Tauri bridge 参数白名单与 Rules 页面已对齐真实 schema，并支持逐目标 `--only`。
- Hooks 原生域接线：Rust dispatcher 提供 `hooks:status`、`hooks:plan`、`hooks:apply`；hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）使用 revision CAS 只写 SQLite；固定 Claude `~/.claude/settings.json` signature-set 合并（保留用户字段与非托管 hook）与 Codex `~/.codex/hooks.json` whole-file 两目标；status/plan/dry-run 零写，真实 apply 使用全局排他锁、Immediate 事务、精确旧字节 CAS、binding、审计与条件回滚，外部修改 fail-closed；前端两个目标与两个 JSON 编辑入口已接线。
- Aliases 原生域接线：Rust dispatcher 提供 `aliases:status`、`aliases:plan`、`aliases:apply`；aliases 专用 `sources:get/set`（aliases.shell）使用 revision CAS 只写 SQLite，alias 名/值结构校验与疑似凭据拒绝；固定 `~/.zshrc` 与 `~/.zprofile` 两目标的受管 marker 区块写入（区块外字节保留），status/plan/dry-run 零写，真实 apply 使用全局排他锁、Immediate 事务、整文件字节 CAS、binding、审计与条件回滚，外部修改 fail-closed，不承诺备份；前端 Aliases 页与来源面板零改动直接可用。
- 只读 `system:audit` 接线：聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态，支持 Tauri offline 桥接注入。
- 只读 `secrets:status` 接线：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。
- 2026-09-12 裁决退役已清理：Tools 域、快照（snapshots:list/restore）与 `system:converge` 的命令面、前端页面/类型/文案、契约 schema、种子文件与文档状态行全部清零；退役命令报错为 UNKNOWN_DOMAIN/UNKNOWN_VERB，六域 apply 输出不再携带 snapshot 字段，"自动备份"承诺文案改为 fail-closed 表述；`agent-tool/` 物理目录保留未动。

## 契约要求

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 必须支持新增、编辑/更新、启用、禁用、删除。保存定义不得执行动作；UI 按引擎 actions 遵循 plan → confirm → apply；移除管理与卸载分开。

## 验收基线（2026-09-12，第二次）

绑定提交 `9080c1d`（Aliases 闭环 `b7f30db` + 退役清理 `9080c1d`）。

- 自动化：`engine-rs` 测试 226 项、`app/src-tauri` 测试 11 项、前端 222 项，全部通过；`pnpm typecheck` 与 `pnpm build` 通过；`cargo fmt --check` 通过。
- 真实隔离 CLI：Aliases 十二项反例实测（缺库零写、plan/dry-run 零写不取锁、真实 apply 建库写区块 0600、重复 apply 幂等、外部修改 fail-closed 且字节不变、并发锁拒绝、`--only` 逐目标、非法名/疑似凭据/过期 revision 拒绝、audit 接线映射）；退役命令实测 UNKNOWN_DOMAIN/UNKNOWN_VERB，apply 输出无 snapshot 字段，audit 为 7 域 + runtime/storage 共 9 个 section。早前基线的 secrets/audit/MCP 反例结论继续有效。
- 前端：Chrome 本地 mock 页面完成真实交互与几何像素检查（早前基线）；本轮前端为零改动（Aliases）与删除面（退役），mock 测试全绿。
- 边界：以上不构成当前安装包的 Tauri 前后端真实业务 E2E。

## 剩余路线图（未排期）

各项均无排期、无指定负责人。

| 事项 | 前置 | 产出 | 完成标准 |
| --- | --- | --- | --- |
| `system:refresh-upstream` | 参与域已定为 agents/mcp/skills（2026-09-12 裁决） | 上游探测 + SQLite `upstream_probes` 缓存表；三域 status 回填 freshness；前端 plan/apply 补显式 `--action` | 不改配置，逐域探测结果可回读 |
| 远程 HTTP MCP | 本地 stdio 闭环（已完成） | 远程 HTTP MCP target | 与本地 stdio 同级的安全与脱敏验收通过 |
| 当前代码安装包与 GUI E2E | 上述参与域完成 | 当前代码构建的 macOS 安装包 | Rules/MCP/Hooks/Aliases/Secrets/system:audit 的真实 GUI 业务 E2E 通过 |
| 发布 | 当前安装包 E2E 通过 | 签名、公证、arm64、Linux、Windows、更新发布 | 各平台与渠道独立验收通过 |

2026-09-09 Darwin x86_64 DMG 已通过只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收，但该包早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

## 高级管理入口裁决记录（2026-09-12）

- Tools：退役。真实条目仅剩 playwright-cli（两条 npm 命令），实施为全新大域且投入产比低；退役后由用户手动管理。已于 `9080c1d` 清理完毕。
- Aliases：保留实施，为恢复后第一个最小闭环。已于 `b7f30db` 落地并通过主审独立验收。
- 更新中心：实施 `system:refresh-upstream`（参与域 agents/mcp/skills），退役 `system:converge`（已于 `9080c1d` 清理）；Updates 页逐域 plan→confirm→apply 编排已扶正为正式语义，前端调用待补显式 `--action`（随 refresh-upstream 实施）。
- 快照：退役。新体系既定方向为 fail-closed 拒绝覆盖 + 操作内事务回滚，不恢复"写前持久备份"模型；"自动备份"承诺文案已于 `9080c1d` 清理。
- 探测缓存：落 SQLite 新表（易失状态入库与 operation_logs 先例一致），不引入第二份可写状态文件。
- 实施者：全部使用 Kimi YOLO 模式（AGENTS.md 已同步）。

## 本次停止点与恢复入口

- 停止点：Aliases 闭环与三项退役清理已提交推送；下一步待启动。
- 恢复入口：实施 `system:refresh-upstream`（任务书由主审出具）；随后远程 HTTP MCP，然后当前代码安装包与 GUI E2E，最后发布。

## 验证

`cargo fmt --manifest-path engine-rs/Cargo.toml -- --check`、两套 Cargo test/check，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。
