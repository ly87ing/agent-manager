# Rust 原生 GUI 目标与边界

## 当前结论

- 核心已就绪：Rust/SQLite/Tauri 共用库与 `native-manager.db` 唯一事实源。
- 已接线闭环：Agents、Skills、MCP、Rules、Hooks、Aliases 六个资源域，以及只读 `system:audit`、`secrets:status`、`system:refresh-upstream`（探测宇宙 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness，前端收敛调用已补显式 `--action`）。
- 2026-09-12 高级管理入口裁决已执行完毕：Aliases 原生闭环落地（`b7f30db`）；Tools、快照与 `system:converge` 退役清理完成（`9080c1d`）。
- 远程 HTTP MCP 已落地（`b353aac`）：三客户端引用形式零明文落盘，与本地 stdio 同级安全语义。
- 2026-09-14 里程碑：基于 `5ae995b` 的干净 Darwin x86_64 DMG 已通过主审独立验收；后续 `a2cc864`、`9f56f00` 仅改工作流，该包仍代表当前产品代码。四平台发布源码管线（`a2cc864`）与 Windows 阻断 CI（`9f56f00`）已就绪。
- 仍未完成：完整 GUI 业务 E2E（宿主 AX/computer-use permission_denied，8 个业务场景未启动，属验收基础设施阻塞）；签名、公证、arm64/Linux/Windows 真实包与更新发布（外部密钥材料与 GitHub billing 未就绪）；未创建 tag、GitHub Release 或对外安装包。

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
- `system:refresh-upstream` 接线：探测宇宙为 enabled 的 npm 渠道 agents、MCP 的 npx 包与 uvx `git+repo@branch` 源、enabled 且有 url 的 skills 集合仓库；探测器（npm view / git ls-remote / git rev-parse / npx 缓存读取 / bin --version）不经 shell、参数数组执行、有界捕获、超时进程组终止（网络 ≤30s），单元素失败归 unknown+固定错误类别不拖垮整批；比较语义对齐旧版（semver 可比 → current/behind/ahead，hash 不可比 → 不同即 behind）；缓存落 SQLite `upstream_probes` 表（Immediate 事务逐元素 upsert，只存版本/commit 与错误类别），status 只读回填 freshness（缺表/缺行一律 unknown，status 零写零网络）；`--offline` 拒绝（OFFLINE_NETWORK_DENIED），`--only` 逐元素，不取 apply 锁；前端 Updates 页收敛调用已补显式 `--action`（agents→upgrade，mcp/skills→update）。
- 远程 HTTP MCP 接线：远程定义 = `url`（仅 https 或本机回环 http，错误不回显 url）+ 可选 `headers`（值仅 `${VAR}` 占位符），与 stdio 字段互斥，`type` 由 codec 推导不持久化；三 codec 全部引用形式落盘零明文——Claude `type:"http"` 写字面量 `${VAR}`、Codex 写 `env_http_headers` 变量名映射、OpenCode `type:"remote"` 写 `{env:VAR}` 且固定 `oauth:false`；受管 marker 迁至条目级 `_MANAGED_BY`（stdio 旧位置兼容）；保存期 policy:set 跑完整定义校验；status/plan/审计对 headers 与 env 同级 `<literal>` 掩码，secrets:status 纳入 headers 并区分 `kind`；缺失变量同级 fail-closed。
- Skills 发现失败可见（`8a428de`）：集合元素 observed 新增 `discovery_error`（类别 + skill_paths 项 + 固定消息），发现失败不再静默为空；前端「全部技能」空态按集合落地状态给出引导，集合卡片展示发现错误。
- MCP 现有配置发现导入与接管（`a4ae118`）：`mcp:status` 增量输出 `import_candidates`（只读扫描三格式目标文件的未受管条目，env/header 值一律 `<literal>` 掩码，Codex `env_http_headers` 引用直通；受管/库内同名/binding/非法名排除）；前端一键转换为 `${VAR}` 占位符定义导入库（明文全程不出渲染进程），保存不执行动作；新增显式 `--allow-takeover`（仅逐 agent add/update）替换未受管同名条目，默认 fail-closed 不变，plan 标注 takeover、审计恒含 takeover 字段。
- macOS 候选包构建身份（`3b28f45`）：发布候选安装包嵌入可信构建身份（commit/date），严格拒绝与源码错配的产物。
- 定义编辑原文保存（`5ae995b`）：SourceDefinitionsPanel 禁用文本框自动替换并有单测，Rules/Hooks 原文保存不再被 smart quotes 等自动改写。

## 契约要求

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 必须支持新增、编辑/更新、启用、禁用、删除。保存定义不得执行动作；UI 按引擎 actions 遵循 plan → confirm → apply；移除管理与卸载分开。

## 验收基线（2026-09-12，第四次）

绑定提交：Aliases 闭环 `b7f30db`、退役清理 `9080c1d`、refresh-upstream `7d9e83b`、远程 HTTP MCP `b353aac`。

- 自动化：`engine-rs` 测试 242 项、`app/src-tauri` 测试 11 项、前端 229 项，全部通过；`pnpm typecheck` 与 `pnpm build` 通过；`cargo fmt --check` 通过。
- 真实隔离 CLI：Aliases 十二项反例、退役命令 UNKNOWN_DOMAIN/UNKNOWN_VERB、refresh-upstream shim 探测矩阵实测同前；远程 HTTP MCP 实测：保存期三层反例（http url / 字段混给 / headers 明文）USAGE_ERROR 且不回显正文、三 codec 零明文渲染（哨兵值 grep 为零）、缺失变量 fail-closed、删 marker 拒绝接管、条目内未受管字段 fail-closed、`_delete` 不误伤、secrets:status 含 `kind:"header"`。
- 前端：mock 测试全绿（含 Updates 页行内收敛 `--action upgrade`、远程 MCP 表单与卡片掩码用例）。
- 边界：以上不构成当前安装包的 Tauri 前后端真实业务 E2E；三真实客户端对条目级 marker 与引用展开的联调属安装包 E2E 项。

## 验收基线（2026-09-14，第五次）

绑定提交：安装包基线 `5ae995b`；发布源码管线 `a2cc864`；Windows 阻断 CI `9f56f00`。

- 独立包（主审独立验收）：基于 `5ae995b` 的干净 Darwin x86_64 DMG，`verify:macos` 通过——只读挂载、仓库外复制、架构与版本、无 Python、源码与 HOME 路径脱敏、从 `/` 隔离环境 5 秒存活；包内构建身份 commit/date 与源码一致；文件 SHA256 `71a386d6ab8e1a7f85ef59c82f379411e9aa1dccaa3c3166309b96b971fce26b`，7632599 bytes。后续 `a2cc864` 与 `9f56f00` 仅改工作流，该包仍代表当前产品代码，但不包含这两份工作流，不是仓库 HEAD 构建包。
- GUI 业务 E2E 未启动：宿主 AX/computer-use 对 Agent Manager 与 Finder 均 permission_denied，8 个业务场景未执行；隔离 HOME/data 确认无 DB 或目标写入；属验收基础设施阻塞，不构成产品通过或产品失败。
- 发布管线（仅源码层验收）：`a2cc864` 的 gui-release.yml 覆盖 macOS arm64+x86_64、Linux x86_64、Windows x86_64，产物形态 app/dmg、AppImage/deb、NSIS/MSI，含 updater 签名与 latest.json、Apple/Windows 签名及 Apple 公证接线，缺密钥/占位符/脏树/来源异常 fail-closed，临时证书清理，Release 固定 draft。2026-09-14 本地独立验收仅证明 YAML/结构、门禁正反例、配置合并、路径 remap 模拟与 cargo fmt；未做真实构建、签名、公证或更新发布。
- Windows 阻断 CI（`9f56f00`）：Windows 引擎与应用升级为不可容错门禁（cargo fmt/test/check、engine release build、NSIS/MSI build，无 continue-on-error）。远程 GUI CI run 34935947828 六个 jobs 均 steps=[]，因 GitHub 账户付款失败或 spending limit 在启动前失败，非测试失败。
- 外部状态：GitHub Actions secrets=0、variables=0、本机有效 codesigning identities=0；签名、公证、arm64/Linux/Windows 真实包与更新发布均未验证。未创建 tag、GitHub Release 或对外发布安装包，现有授权不包含这些动作。

## 剩余路线图（未排期）

各项均无排期、无指定负责人。

| 事项 | 前置 | 产出 | 完成标准 |
| --- | --- | --- | --- |
| 当前代码安装包与 GUI E2E | 参与域全部完成（已达）；macOS DMG 已验收（2026-09-14），待宿主 AX/computer-use 权限恢复 | 当前代码构建的 macOS 安装包（已产出并验收） | Rules/MCP（stdio+远程 HTTP）/Hooks/Aliases/Secrets/system:audit 的 8 个真实 GUI 业务场景 E2E 通过 |
| 发布 | 当前安装包 E2E 通过；GitHub billing 恢复；updater/Apple/Windows 材料在仓库外与 Secrets 配齐 | 签名、公证、arm64、Linux、Windows、更新发布（源码管线 `a2cc864` 已就绪） | 各平台与渠道独立验收通过（Release/打 tag 未授权，到达时报主审决定） |

2026-09-14 基于 `5ae995b` 的干净 Darwin x86_64 DMG 已通过主审独立验收（只读挂载、仓库外复制、架构与版本、无 Python、源码与 HOME 路径脱敏、从 `/` 隔离环境 5 秒存活，包内构建身份 commit/date 与源码一致），代表当前产品代码；完整 GUI 业务 E2E 因宿主 AX/computer-use 权限阻塞未启动。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

## 高级管理入口裁决记录（2026-09-12）

- Tools：退役。真实条目仅剩 playwright-cli（两条 npm 命令），实施为全新大域且投入产比低；退役后由用户手动管理。已于 `9080c1d` 清理完毕。
- Aliases：保留实施，为恢复后第一个最小闭环。已于 `b7f30db` 落地并通过主审独立验收。
- 更新中心：`system:refresh-upstream` 已实施（参与域 agents/mcp/skills，探测缓存落 SQLite `upstream_probes` 表，三域 status 只读回填 freshness，`--offline` 拒绝，不取 apply 锁）；`system:converge` 已于 `9080c1d` 退役清理；Updates 页逐域 plan→confirm→apply 编排已扶正为正式语义，前端调用已补显式 `--action`（agents→upgrade，mcp/skills→update）。
- 快照：退役。新体系既定方向为 fail-closed 拒绝覆盖 + 操作内事务回滚，不恢复"写前持久备份"模型；"自动备份"承诺文案已于 `9080c1d` 清理。
- 探测缓存：落 SQLite 新表（易失状态入库与 operation_logs 先例一致），不引入第二份可写状态文件。
- 实施者：全部使用 Kimi YOLO 模式（AGENTS.md 已同步）。

## 本次停止点与恢复入口

- 停止点：基于 `5ae995b` 的 macOS DMG 已通过主审独立验收（2026-09-14）；完整 GUI 业务 E2E 因宿主 AX/computer-use permission_denied 阻塞未启动；发布源码管线（`a2cc864`）与 Windows 阻断 CI（`9f56f00`）已就绪但外部/真实验收未完成。
- 恢复入口（按序）：恢复 Orca/宿主 AX 后完成 8 个业务场景真实 GUI E2E；解决 GitHub billing；在仓库外/Secrets 配齐 updater/Apple/Windows 材料；让 GUI CI 真正跑绿；由用户单独授权后打 tag，release workflow 产出草稿 Release；逐平台安装/签名/公证/updater 独立验收通过后才公开。

## 验证

`cargo fmt --manifest-path engine-rs/Cargo.toml -- --check`、两套 Cargo test/check，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。
