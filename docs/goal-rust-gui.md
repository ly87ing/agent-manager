# Rust 原生 GUI 目标与边界

## 当前结论

- 核心已就绪：Rust/SQLite/Tauri 共用库与 `native-manager.db` 唯一事实源。
- 已接线闭环：Agents、Skills、MCP、Rules、Hooks、Aliases 六个资源域，以及只读 `system:audit`、`secrets:status`、`system:discover`（接管发现，见下）、`system:refresh-upstream`（探测宇宙 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness，前端收敛调用已补显式 `--action`）。
- 2026-09-12 高级管理入口裁决已执行完毕：Aliases 原生闭环落地（`b7f30db`）；Tools、快照与 `system:converge` 退役清理完成（`9080c1d`）。
- 远程 HTTP MCP 已落地（`b353aac`）：三客户端引用形式零明文落盘，与本地 stdio 同级安全语义。
- 2026-09-14 里程碑（历史）：基于 `5ae995b` 的干净 Darwin x86_64 DMG 通过主审独立验收；四平台发布源码管线（`a2cc864`）与 Windows 阻断 CI（`9f56f00`）就绪。
- 2026-09-15 里程碑：验收基线提交 `5a110be3` 的 x86_64 DMG 通过 verify 与主要真实 GUI E2E 主审独立验收；同源码 arm64 DMG 交叉构建静态验收通过（thin arm64、adhoc linker-signed，未在 arm64 设备运行）；完整矩阵 9/9 exit 0（Rust 269 项、Vitest 246 项、typecheck/build 通过且 0 warning）。
- 当前状态：公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝；status/query 只读零写；点击动作直接触发真实 apply；apply 内部保留 preflight/写前校验、精确 scope、排他锁、外部修改检测、CAS/条件回滚与审计。定义保存仍只保存；破坏性定义移除、清日志等独立确认不变。
- 本轮直接 apply 新交互已完成源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收：Engine 278 项、Tauri 11 项、Vitest 28 files / 248 tests、typecheck/build、8 份 schema 与隔离 smoke 全部通过；旧 DMG/旧 GUI E2E 不证明本轮新交互，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见下。仍未完成：MCP 新增服务表单真实 GUI 验收（macOS 自动化输入限制）；公开跨平台发布条件——GitHub billing、真实密钥、Developer ID/公证、Windows 签名、Linux/Windows/arm64 真机运行与安装、updater 真实签名下载安装回读；未创建 tag、GitHub Release 或对外安装包，现有授权不包含这些动作。
- 2026-09-16 发布就绪仓库内改造完成（三提交）：`d46b85a` 补齐 MIT 许可证、公开发布元数据、显式 Webview CSP 与应用图标，Bundle ID 定为 `io.github.ly87ing.agent-manager`；`47702ed` 增加前端顶层错误边界、自定义助手安装/升级确认与 MCP 空态引导，并清理退役代码；`14e6790` 将发布管线重构为草稿先行单写者编排（前置 create-release job 创建唯一 draft，矩阵以 releaseId 上传产物，收尾 job 单写 latest.json），支持 prerelease tag、逐平台 SHA256 checksums 与 engine-rs 版本一致性门禁，verify 脚本在已签名分支补充公证票据如实检查。对外发布仍未进行：GitHub billing、仓库 Secrets/Variables、签名与公证材料、Linux/Windows/arm64 真机验收、updater 真实链路均未就绪，未创建 tag 或 Release。

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
- Agent 原生定义、安装来源探测、生命周期 apply、跨进程锁、结果核验与操作记录。
- Skills 原生域接线与完整闭环：内置集合只读默认，首次本地写入 `native-manager.db` 后本地优先；集合与 Skill×Agent 独立更新/启停/移除；内容寻址快照与安全软连接落地及回滚保护；status 只读零写；真实 apply 跨进程锁与操作审计。
- MCP 原生域接线与本地 stdio 闭环：Rust dispatcher 提供 `mcp:status`（只读）与真实 `mcp:apply`；定义与分配保存在 SQLite，支持 Claude JSON、Codex TOML、OpenCode JSON（需配置对应 Agent target）；逐助手支持 add/update/enable/disable/remove，库级支持 enable/disable/remove；status 只读零写，真实 apply 使用锁、审计、CAS 与回滚；env 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。
- `sources:get/set` 原生接线：规则来源固定为 `rules.shared`、`rules.codex`、`rules.claude` 三项，保存使用 revision CAS 且只写 SQLite，不执行任何同步动作。
- Rules 原生域接线：Rust dispatcher 提供 `rules:status`（只读）与真实 `rules:apply`；固定目标为 `rules:codex_global`（`~/.codex/AGENTS.md`）与 `rules:claude_global`（`~/.claude/CLAUDE.md`），内容按共享与助手专用来源渲染；status 只读零写，真实 apply 使用全局排他锁、操作审计、文件字节 CAS 与事务/文件回滚，外部修改 fail-closed 拒绝覆盖，不承诺备份；不再有 rules/dist 构建链。Tauri bridge 参数白名单与 Rules 页面已对齐真实 schema，并支持逐目标 `--only`。
- Hooks 原生域接线：Rust dispatcher 提供 `hooks:status`（只读）与真实 `hooks:apply`；hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）使用 revision CAS 只写 SQLite；固定 Claude `~/.claude/settings.json` signature-set 合并（保留用户字段与非托管 hook）与 Codex `~/.codex/hooks.json` whole-file 两目标；status 只读零写，真实 apply 使用全局排他锁、Immediate 事务、精确旧字节 CAS、binding、审计与条件回滚，外部修改 fail-closed；前端两个目标与两个 JSON 编辑入口已接线。
- Aliases 原生域接线：Rust dispatcher 提供 `aliases:status`（只读）与真实 `aliases:apply`；aliases 专用 `sources:get/set`（aliases.shell）使用 revision CAS 只写 SQLite，alias 名/值结构校验与疑似凭据拒绝；固定 `~/.zshrc` 与 `~/.zprofile` 两目标的受管 marker 区块写入（区块外字节保留），status 只读零写，真实 apply 使用全局排他锁、Immediate 事务、整文件字节 CAS、binding、审计与条件回滚，外部修改 fail-closed，不承诺备份；前端 Aliases 页与来源面板零改动直接可用。
- 只读 `system:audit` 接线：聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态，支持 Tauri offline 桥接注入。
- 只读 `secrets:status` 接线：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败。
- 2026-09-12 裁决退役已清理：Tools 域、快照（snapshots:list/restore）与 `system:converge` 的命令面、前端页面/类型/文案、契约 schema、种子文件与文档状态行全部清零；退役命令报错为 UNKNOWN_DOMAIN/UNKNOWN_VERB，六域 apply 输出不再携带 snapshot 字段，"自动备份"承诺文案改为 fail-closed 表述；`agent-tool/` 物理目录保留未动。
- `system:refresh-upstream` 接线：探测宇宙为 enabled 的 npm 渠道 agents、MCP 的 npx 包与 uvx `git+repo@branch` 源、enabled 且有 url 的 skills 集合仓库；探测器（npm view / git ls-remote / git rev-parse / npx 缓存读取 / bin --version）不经 shell、参数数组执行、有界捕获、超时进程组终止（网络 ≤30s），单元素失败归 unknown+固定错误类别不拖垮整批；比较语义对齐旧版（semver 可比 → current/behind/ahead，hash 不可比 → 不同即 behind）；缓存落 SQLite `upstream_probes` 表（Immediate 事务逐元素 upsert，只存版本/commit 与错误类别），status 只读回填 freshness（缺表/缺行一律 unknown，status 零写零网络）；`--offline` 拒绝（OFFLINE_NETWORK_DENIED），`--only` 逐元素，不取 apply 锁；前端 Updates 页直接触发真实 apply（apply 内部保留 preflight/写前校验），调用已补显式 `--action`（agents→upgrade，mcp/skills→update）。
- 远程 HTTP MCP 接线：远程定义 = `url`（仅 https 或本机回环 http，错误不回显 url）+ 可选 `headers`（值仅 `${VAR}` 占位符），与 stdio 字段互斥，`type` 由 codec 推导不持久化；三 codec 全部引用形式落盘零明文——Claude `type:"http"` 写字面量 `${VAR}`、Codex 写 `env_http_headers` 变量名映射、OpenCode `type:"remote"` 写 `{env:VAR}` 且固定 `oauth:false`；受管 marker 迁至条目级 `_MANAGED_BY`（stdio 旧位置兼容）；保存期 policy:set 跑完整定义校验；status/apply 审计对 headers 与 env 同级 `<literal>` 掩码，secrets:status 纳入 headers 并区分 `kind`；缺失变量同级 fail-closed。
- Skills 发现失败可见（`8a428de`）：集合元素 observed 新增 `discovery_error`（类别 + skill_paths 项 + 固定消息），发现失败不再静默为空；前端「全部技能」空态按集合落地状态给出引导，集合卡片展示发现错误。
- MCP 现有配置发现导入与接管（`a4ae118`）：`mcp:status` 增量输出 `import_candidates`（只读扫描三格式目标文件的未受管条目，env/header 值一律 `<literal>` 掩码，Codex `env_http_headers` 引用直通；受管/库内同名/binding/非法名排除）；前端一键转换为 `${VAR}` 占位符定义导入库（明文全程不出渲染进程），保存不执行动作；新增显式 `--allow-takeover`（仅逐 agent add/update）替换未受管同名条目，默认 fail-closed 不变，apply 审计恒含 takeover 字段。
- macOS 候选包构建身份（`3b28f45`）：发布候选安装包嵌入可信构建身份（commit/date），严格拒绝与源码错配的产物。
- 定义编辑原文保存（`5ae995b`）：SourceDefinitionsPanel 禁用文本框自动替换并有单测，Rules/Hooks 原文保存不再被 smart quotes 等自动改写。

## 契约要求

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 必须支持新增、编辑/更新、启用、禁用、删除。保存定义不得执行动作；UI 直接触发真实 apply（apply 内部保留 preflight/写前校验、精确 scope、锁、外部修改检测、CAS/条件回滚与审计），普通同步、启停、更新不二次确认；移除管理与卸载分开，破坏性动作独立确认。

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

## 验收基线（2026-09-15，第六次）

绑定验收提交：`5a110be3da66d1bedee906eb0c30f04b5b95b245`。

- 自动化完整矩阵：9/9 exit 0；`engine-rs` 258 项 + `app/src-tauri` 11 项 = Rust 269 项全部通过；Vitest 28 files / 246 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过，0 warning。
- x86_64 DMG（主审独立验收）：verify 脚本 exit 0；SHA256 `076d16b098ede8915802c0bc6dceec42dc06e0dcadddd137e7865681c7e90a68`，7632606 bytes；包内构建身份 commit/date 与源码精确一致；未签名。
- arm64 DMG（交叉构建，仅静态验收）：SHA256 `a9540a2690b9d2ede16ef627f310e277ca49130e608c518b876aeaed0bbfd3f8`，7298163 bytes；thin arm64；adhoc linker-signed（非 Developer ID）；未在 arm64 设备运行。
- 真实 GUI E2E（x86_64 安装包，主审独立验收；旧版 plan→confirm→apply 交互，现已失效，不证明本轮直接 apply 新交互）已覆盖：system:audit 缺库零写、Rules 取消零写/应用、Hooks 双目标、Aliases 双目标、MCP stdio+远程 HTTP 的 GUI plan/confirm/apply、Secrets 仅显示变量名与存在性、Runs、外部修改 plan fail-closed。MCP 定义使用共用 Rust CLI 写入隔离 SQLite 测试夹具；MCP 新增服务表单因 macOS 自动化输入限制未验收，不以测试夹具结果替代表单通过。
- 零写与脱敏回读（旧版 dry-run 交互证据，现已失效）：stdio dry-run 数据库与目标文件哈希不变，日志不落库；远程 header 在 Codex 目标以环境变量引用写入，合成值不出现在目标文件、诊断日志与操作日志。
- 外部状态（2026-09-15 复测）：GitHub Actions secrets=0、variables=0、本机有效 codesigning identities=0、GitHub Releases=0；GUI CI run 34936884572（该 run 绑定提交 `5a110be3`）六个 jobs 均 steps=[]，因账户付款失败或 spending limit 在启动前失败，非测试失败。
- 发布结论：仍未达到公开跨平台发布条件——billing、真实密钥、Developer ID/公证、Windows 签名、Linux/Windows/arm64 真机运行与安装、updater 真实签名下载安装回读均未完成；现有授权不包含 tag、Release 或公开安装包。

## 交付回执（2026-09-16，直接 apply 最终 clean DMG）

绑定最终提交：`a01d0f5b182c9da346a6663688b49ed4a83f31f4`（2026-09-15T17:32:20-07:00）。

- 自动化矩阵（2026-09-16 主审独立复跑）：`engine-rs` 278 项、`app/src-tauri` 11 项全部通过；Vitest 248 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过；两套 `cargo fmt --check` 通过。
- 最终 clean x86_64 DMG（最终提交后于 2026-09-16 本地重建，主审独立验收）：`verify:macos` 全部通过——平台与元数据、构建身份 commit/date 与 `a01d0f5` 精确一致、无 Python 动态链接与遗留、源码与 HOME 路径脱敏、从 `/` 隔离环境 5 秒存活并优雅退出；未签名。SHA256 `1ba291cc794c00a1c68152276dad0b04f792b1b8ae1139156f0e6c24decf9898`，7572035 bytes。
- GUI CI（外部阻塞复测）：run 35040441270（绑定 `a01d0f5`）六个 jobs 均 steps=[]，启动前失败（GitHub 账户付款失败或 spending limit），非测试失败。
- 外部状态（2026-09-16 复测）：GitHub Actions secrets=0、variables=0、本机有效 codesigning identities=0；公开发布阻塞项维持原状。
- 结论：直接 apply 改造的仓库内验收闭环完成；剩余未完成项仅为 MCP 新增服务表单真实 GUI 验收（macOS 自动化输入限制）与公开发布外部条件。

## 交付回执（2026-09-16，发布就绪改造最终 clean DMG）

绑定最终提交：`5a1582d004c7fe75d6744614b3a475efe3c75c12`（2026-09-16，含 `d46b85a`/`47702ed`/`14e6790`/`5a1582d` 四个发布就绪提交）。

- 自动化矩阵（2026-09-16 主审复跑）：两套 `cargo fmt --check` 通过；`engine-rs` 与 `app/src-tauri` 11 项 cargo test 全部通过；Vitest 29 files / 259 tests 全部通过（较基线 +11：ErrorBoundary 4、MCP 空态 3、自定义助手确认 4）；`pnpm typecheck` 与 `pnpm build` 通过。
- 最终 clean x86_64 DMG（最终提交后于 2026-09-16 本地重建，主审独立验收）：`verify:macos` 全部通过——Bundle Identifier `io.github.ly87ing.agent-manager` 与版本 0.1.0 校验通过、构建身份 commit/date 与 `5a1582d` 精确一致、无 Python 动态链接与遗留、路径脱敏、从 `/` 隔离环境 5 秒存活并优雅退出；未签名（公证票据检查按脚本语义报告"不适用"）。SHA256 `5936656ad180f2ec42e9aa6829f762acbcfd153c45c753bb3e9b2e7dbe27a175`，7572750 bytes。
- 发布管线（`14e6790`）仅做 YAML/脚本级静态验收与正反例实测（tag 正则、版本门禁、latest.json 组装、checksums、stapler 分支），未真实执行；外部阻塞项（billing、secrets、签名/公证材料、真机验收、updater 链路）维持原状，未创建 tag 或 Release。
- 结论：对外发布的仓库内就绪改造全部完成并验收；剩余未完成项不变——MCP 新增服务表单真实 GUI 验收与公开发布外部条件。

## 交付回执（2026-09-17，精品化 UI 改造）

绑定最终提交：`bcdcb6e`（2026-09-17，含 `e4debd2`/`8b4e753`/`6255aa1`/`bcdcb6e` 四个精品化提交；借鉴 cc-switch 的设计与便捷性做法）。

- 改造内容：设计令牌体系（CSS 变量 + 语义色 token 全组件覆盖）与浅色/深色/跟随系统三态主题（含原生标题栏同步与首屏防闪烁）；系统托盘驻留（关窗入托盘、托盘唤回/退出、macOS Dock Reopen、退出显式摘图标）、窗口位置尺寸记忆（有意排除可见性/全屏）、单实例唤回；页面与弹窗过渡动画（prefers-reduced-motion 降级）、骨架屏加载态、组件化空态引导；助手/技能/MCP/操作记录四页搜索（与既有筛选叠加）、全局快捷键 ⌘,/⌘F、设置入口更新提示角标（启动延迟检查、按版本忽略、未配置不检查）。
- 自动化矩阵（2026-09-17 主审复跑）：两套 `cargo fmt --check` 通过；`engine-rs` cargo test 全部通过；`app/src-tauri` 15 项通过（+4：主题解析、窗口 flags、退出分类）；Vitest 35 files / 298 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过（一条无害的 dynamic import 提示）。
- 最终 clean x86_64 DMG（2026-09-17 本地重建，主审独立验收）：`verify:macos` 全部通过——构建身份 commit/date 与 `bcdcb6e` 精确一致、隔离环境 5 秒存活并优雅退出（覆盖托盘启动路径）；未签名。SHA256 `29188131ae0f46c1112a8f0b12987e3da82c21bff0887ff61555a15f68a9d809`，7873725 bytes。
- 未验收项如实说明：托盘点击交互、Dock Reopen、关窗驻留的真机 GUI 验收未做（避免在宿主打扰桌面；隔离验收只覆盖启动与存活）；暗色观感经 dev server 截图抽查。
- 结论：精品化仓库内改造完成并验收；公开发布外部条件（billing、签名/公证材料、真机验收、updater 链路、tag/Release 授权）维持原状。

### 2026-09-17 追加：Skills 计划外链接语义修复（`847d703` + `b313716`）

- 起因（真机使用反馈）：「全部技能」矩阵中旧体系遗留链接行显示"已链接 + 计划外链接 + 添加"，语义自相矛盾，且非受管链接点「添加」必然被外部冲突保护 fail-closed 拒绝。
- 改动：extra-link 单元格动作扩展为「纳入管理」（add）与「取消链接」（remove）；`skills:apply` 的逐 skill add/remove 支持显式 `--allow-takeover` 接管通道（仅限非受管符号链接，真实文件/目录仍 fail-closed；审计恒含 takeover 字段，对齐 MCP 先例）；status 单元格新增 `managed` 字段；Tauri 桥白名单放行该旗标；前端 extra-link 行标签改为「纳入管理/取消链接」，非受管链接先弹确认，「当前分配版本」列按 state 判定链接存在性（修掉 not-assigned 行误显"已链接"）。
- 验证：`engine-rs` 279 项（新增 extra-link takeover 全链路正反例）、`app/src-tauri` 15 项、Vitest 303 项全部通过；typecheck/build 通过。
- DMG（`847d703` 重建，主审独立验收）：`verify:macos` 全部通过；SHA256 `555b744ef97c85614bcdce750bdf822cf043b052a64484882427a8d9ddbe8830`。过程中发现单实例改造使验收副本在已有实例运行时秒退（存活检测误判），`b313716` 为验收脚本增加运行实例检出守卫后复验通过——该 DMG 不含 `b313716`（仅验收脚本，不影响产品代码）。

### 2026-09-17 追加：技能矩阵批量操作（`26b94b3`）

- 全部技能矩阵新增勾选列与批量操作栏：批量启用/禁用（直执）与批量纳入管理/取消链接（含非受管行时一次汇总确认并带 `--allow-takeover`）；前端编排为逐行顺序 apply（每行独立 run_id/锁/审计/回滚），单行失败不中断，汇总 toast 反馈（执行通道新增 `silent` 选项抑制单行 toast，单行操作行为不变）。
- 验证：Vitest 36 files / 313 tests 全部通过（新增批量用例 8 + 单行回归 1）；typecheck/build 通过；`26b94b3` 重建 DMG 通过 `verify:macos` 独立验收。

### 2026-09-17 追加：动作执行反馈与版本提示（`0419d3c`）

- 起因（真机使用反馈）：点击「升级」无任何即时提示，不知道是否点上，也不知道从什么版本升到什么版本。
- 改动：助手卡片动作点击立即转 spinner「正在执行…」且同卡其余按钮互斥禁用（含卸载/自定义确认弹窗路径的执行态串联）；升级 label 与 title 带版本区间「当前 → 最新」（缺失侧「未知」占位，git hash 缩写），进入 toast 与任务记录；UpdatesPage 行内/批量更新同样补版本与执行态。
- 顺手修复：jsdom 套件在默认并行度下因资源竞争批量超时的既有 flaky（vite.config.ts testTimeout/hookTimeout 放宽至 20s），修复后默认并行 36 文件 320 用例全绿。
- 验证：Vitest 320 全绿、typecheck/build 通过；`0419d3c` 重建 DMG 通过 `verify:macos` 独立验收；SHA256 `05f1018010a0cecc1cfbc318ad790778be9865b0bacc34b2e0d3f323d5fa6f73`。

### 2026-09-17 追加：接管中心（`04c99e6` + `f5e0273` + `763a988` + `d3faf30`）

- 起因（用户需求）：一键接管机器上的 agent / mcp / skill 资源并分类标识；官方路径与各自原生安装方案继续用原生管理方式，其余用软链接方案。
- 现状差距（实施前核对）：agents 只对已定义助手探测安装来源，不发现未定义助手；skills 只在已定义集合的矩阵格里看到计划外链接，目标目录里不属任何集合的条目无法发现，且集合必须有 git url，外部本地技能没有登记途径；MCP 已有 `import_candidates` 但逐条手工、无来源分类。
- 引擎改动：新增只读 `system:discover`（`--only agents|skills|mcp`，零写零网络、不取 apply 锁），三域候选统一带 `manage` 分类（native / symlink / collection / register / cleanup）；skills 集合定义新增 `source_path`（与 `url` 互斥）的本地来源集合——直链登记源目录、不做内容快照，登记源根进入受管基准，集合级 update 不联网只重扫修链；agents 发现已知 CLI 并按来源给建议定义（仅 npm 反查成功者给管理命令）；MCP 复用既有掩码扫描并补 origin/manage。Tauri 命令面与参数白名单同步放行 `system:discover`。
- 前端改动：新增「接管中心」页（导航第五项），分类展示 + 勾选批量 + 一次汇总确认，逐项走统一执行通道（独立 run_id / 执行记录 / 审计），单项失败不中断后续项；助手登记只写定义，技能与 MCP 走各自域既有真实 apply。
- 顺带修复（独立提交 `04c99e6`）：`definitionKeyPrefix("agents")` 写的顶层 `agent_definitions` 引擎不读，自定义助手定义保存后不生效，改为引擎真实读取的 `agents.definitions`。
- 补完（`763a988` / `d3faf30`）：dev mock 覆盖 `system:discover`（否则 `?mock=1` 下接管中心页只会报错）；技能集合表单改为来源二选一（git 仓库 / 本机目录），否则接管生成的本地来源集合在技能页点「编辑」会被 git 地址校验挡住、保存还会把 url 写回去破坏定义。
- 自动化验证：`engine-rs` 289 项、`app/src-tauri` 15 项、Vitest 37 files / 330 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过；两套 `cargo fmt --check` 通过。
- 隔离环境真实 CLI 验收（隔离 HOME + AGENT_MANAGER_DATA_DIR，非 mock）：技能侧复刻本机形态（外部工具目录链接 + 官方目录内真实目录 + 断链），发现分类、按源根聚合建议集合、登记后逐助手 add、链接直指外部源、无内容快照、内容跟随外部更新、停用只删链接且外部源完好、已纳管项不再是候选、只读命令库哈希不变共 11 项检查全通过；MCP 侧发现掩码、导入接管、stdio 标记写入 env、SQLite 定义零明文且保留变量引用、status 零明文、用户其他字段保留、缺 `--allow-takeover` 时对未受管同名条目 fail-closed 共 7 项检查全通过；agents 侧 `agents.definitions.<id>` 登记后 `agents:status` 即出现该助手（state=unmanaged-detected、definition_origin=custom、无升级卸载动作）。
- 最终 clean x86_64 DMG（`d3faf30` 后重建，主审独立验收）：`verify:macos` 全部通过——Bundle Identifier 与版本、Mach-O 架构、无 Python 动态链接与遗留、路径脱敏、构建身份 commit/date 与 `d3faf30` 精确一致、从 `/` 隔离环境（隔离 HOME、独立 DATA_DIR、最小 PATH）5 秒持续存活并优雅退出；未签名（公证按脚本语义报告"不适用"）。SHA256 `51c33f05882170f2f01e1711beeb79ecc27781c6de4517209d89393ed3131687`，7942641 bytes。
- 浏览器验收（dev server + mock，Chrome DevTools）：接管中心页六个分区按来源正确渲染，「全选可接管项」得到 6 项、确认弹窗如实声明「其中 3 项会替换助手里的同名外部条目」，执行后 toast「接管完成：成功 6，失败 0」且勾选清空；控制台除 dev server 的 favicon 404 外无报错，页面无横向滚动。该验收证明页面渲染与编排链路，不证明真实引擎写入。
- 未做/未验证，如实说明：断链与旧体系残留只标识，不提供清理动作（需手工删除）；助手发现基于固定已知 CLI 清单，清单外的助手不会被发现；安装包内的接管中心真机点击链路未验收（避免在宿主桌面执行真实写入），真实写入语义由隔离 CLI 验收覆盖。

### 2026-09-17 追加：启动命令界面化与易用性审查（`873c63f` + `d01a18e`）

- 起因（用户需求）：界面上要能设置每个助手的启动参数别名，并支持选择时区与设置扩展参数。
- 易用性审查（按「客户完成一件事要几步、要不要手写结构化文本」核对六域）：

  | 域 | 新增 / 编辑 | 启停 | 删除 | 执行 | 编辑形态 | 结论 |
  | --- | --- | --- | --- | --- | --- | --- |
  | 助手 | 表单 | 卡片上只读 | 移除管理 | 安装/升级/卸载直执 | 结构化 | 启停与自动安装/自动更新改不了（见下） |
  | 技能 | 表单 | 集合级 + 逐助手矩阵（含勾选批量） | 有 | 直执 | 结构化 | 完整 |
  | MCP | 表单 | 库级 + 逐助手 | 有 | 直执 | 结构化 | 完整 |
  | 规则 | 文本编辑 | — | 清空即移除 | 同步 | 文本（正文本就是 Markdown） | 合适 |
  | Hooks | 手写 JSON | — | — | 同步 | 裸 JSON | 仍需照着 schema 手写（见下） |
  | 启动命令 | 本轮补齐 | — | 本轮补齐 | 同步 | 结构化 | 本轮补齐 |

- 本轮改动：助手卡片新增「启动命令」区（展示同步后实际生效的那一行、按项编辑、已配置时可直接同步）；编辑弹窗按项填写名称/命令/时区/语言/扩展参数并实时预览，常用时区置顶，读到非「环境变量+命令」写法时自动切原文编辑不改写用户写法；设置里的「终端快捷命令」从通用同步页换成结构化页（全局时区语言 + 逐条增删改 + 按需展开的原文编辑）。引擎侧零改动——时区/语言记号替换、注入校验与只改受管区块本就具备。
- 自动化验证：Vitest 39 files / 360 tests 全部通过（新增纯函数 13、终端快捷命令页 9、助手卡片启动命令 4）；`pnpm typecheck` 与 `pnpm build` 通过。
- 隔离环境真实 CLI 验收（隔离 HOME + 数据目录）：保存不写 shell 配置、同步后跟随全局的时区被替换、单独指定时区的条目不受全局影响、用户原有内容与 zprofile 同步、改全局时区后跟随项跟着变而指定项不变、含单引号的取值被引擎拒绝、移除条目后该行从配置消失而其余保留，共 12 项检查全通过。
- 浏览器验收（dev server + mock）：助手卡片启动命令区与编辑弹窗回填正确，终端快捷命令页的全局默认与逐条列表渲染正确，控制台无报错。
- 最终 clean x86_64 DMG（`d01a18e` 后重建，主审独立验收）：`verify:macos` 全部通过，包内构建身份 commit/date 与 `d01a18e` 精确一致，隔离环境 5 秒存活并优雅退出；未签名。SHA256 `4d901ab66775383ca7d5fb183d215567f458922272162bf106665c29d63af517`，7947490 bytes。
- 审查发现但本轮未做（留待裁决，不擅自扩大范围）：Hooks 仍只能手写 JSON，没有按事件/命令填写的结构化表单；助手卡片上的启用停用、自动安装、自动更新是只读展示，要改只能去「个性化配置」手写；接管中心的断链与旧体系残留仍只标识、不提供清理动作。

### 2026-09-17 追加：助手实例与启动命令自定义（`2204621` + `8893e91`）

- 起因（用户需求与裁决）：借鉴 cc-switch 做多账号，但用户指出它「改写全局配置来切换」的范式同一时刻只能用一个账号；用户提出改用独立配置目录 + 别名启动（`CLAUDE_CONFIG_DIR=… claude …`），可以多端同时工作。主审核实后采纳该范式。
- 技术前提（实测，非文档推断）：`CLAUDE_CONFIG_DIR` 指向空目录后 `claude mcp list` 返回「未配置」并在该目录独立生成 `.claude.json`，配置与凭据确实隔离；`--settings <file-or-json>` 存在；Codex 侧 `$CODEX_HOME` 与 `-p/--profile <name>`（叠加 `$CODEX_HOME/<name>.config.toml`）存在。官方文档进一步说明 `CLAUDE_CONFIG_DIR` 下 settings、skills、agents、plugins、凭据、历史全部随之迁移，且官方给出的多账号示例就是这种别名写法。
- 范式取舍如实记录：相比改写全局配置，隔离目录方案支持并发、不动全局文件、无切换竞态、删目录即回退；代价是每个实例各自登录一次、各存一份会话与技能/MCP 配置。
- 实例定义即 agent 定义（多一个 `profile` 段），技能矩阵与 MCP 的逐助手分配因此天然适用，不另造分配模型。
- 引擎新增 profiles 域：status 只读零写、只报变量名与能否解析；apply 只写实例目录下本工具全权拥有的那个文件（Claude `settings.json`；Codex 用 `agent-manager.config.toml` + 启动 `-p` 叠加，避开归 MCP 域的 `config.toml` 被整文件写入互相抹掉），目录 0700、文件 0600、原子换入、事务失败回滚，已有他人内容或被外部改过一律 fail-closed 需显式 `--allow-takeover`；鉴权只以 `${VAR}` 入库，Claude 写入时解析（客户端不展开引用），Codex 只写变量名；配置目录必须在 HOME 下且不得是主实例目录、数据目录内或符号链接。
- 前端：助手页新增「实例」区（新建/编辑表单含启动方式预览，保存时同时把启动命令存成同名快捷命令；卡片支持写入配置、接管写入、同步启动命令、管理技能、移除），启动命令弹窗补自定义时区/语言与任意环境变量增删改。
- 自动化验证：`engine-rs` 303 项、`app/src-tauri` 15 项、Vitest 41 files / 379 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过；两套 `cargo fmt --check` 通过。
- 隔离环境真实 CLI 验收（19 项检查全通过）：保存定义零落地；实例配置写入后含解析值而全局 `~/.claude/settings.json` 字节未变；目录 0700 / 文件 0600；**技能分配到实例后链接落在实例技能目录且主实例未被波及**；**MCP 分配写进实例的 `.claude.json` 且全局 `~/.claude.json` 未被创建**；主实例与实例的启动命令共存于受管区块；外部改动后拒绝覆盖且原文件不变，显式接管后方可写入。启动命令侧另有 12 项检查（自定义时区、跟随全局与单独指定的分别生效、移除后从配置消失）。
- 最终 clean x86_64 DMG（`8893e91` 后重建，主审独立验收）：`verify:macos` 全部通过，构建身份与 `8893e91` 精确一致；SHA256 `cdec9c38375ae1f4dcc2cbdc709a2f2f6d483ac6a9ea1eef1132b5bd451f914c`，7984635 bytes；未签名。
- 未做/未验证，如实说明：实例的技能解析依据官方文档与本机 `.claude.json`/凭据隔离实测，**未对「Claude 真的从实例目录加载 skills」做端到端实证**（新配置目录需先在该实例下登录才能起会话）；实例目前只支持 Claude Code 与 Codex；cc-switch 的用量查询、测速、代理与云同步不在本工具定位内，不做。

## 剩余路线图（未排期）

各项均无排期、无指定负责人。

| 事项 | 前置 | 产出 | 完成标准 |
| --- | --- | --- | --- |
| 当前代码安装包与 GUI E2E | 参与域全部完成（已达） | 验收基线提交 `5a110be3` 的 x86_64 DMG 与主要真实 GUI E2E 已验收（2026-09-15，旧版交互）；本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 已验收 | 最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见上；MCP 新增服务表单待 macOS 自动化输入条件具备后补验 |
| 发布 | GitHub billing 解决；updater/Apple/Windows 签名材料在仓库外与 Secrets 配齐（GitHub Actions secrets/variables 与本机 codesigning identities 当前均为 0） | 签名、公证、arm64、Linux、Windows、更新发布（发布管线 `14e6790` 草稿先行单写者编排已就绪，取代 `a2cc864`） | GUI CI 对当前产品代码基线真实跑绿；各平台真机安装/运行与 updater 真实签名下载安装回读独立验收通过（Release/打 tag 未授权，到达时报主审决定） |
| 易用性补齐：Hooks 结构化编辑 | 无 | 按事件与命令填写的表单，替代手写 JSON | 常用 hook 不需照 schema 手写即可配置并同步 |
| 易用性补齐：助手开关 | 无 | 卡片上可改启用停用、自动安装、自动更新 | 不必进「个性化配置」手写即可完成 |
| 接管中心：失效链接清理 | 无 | 断链与旧体系残留的清理动作 | 一键清理且对真实文件仍 fail-closed |

2026-09-15 验收基线提交 `5a110be3` 的 x86_64 DMG 已通过 verify 与主要真实 GUI E2E 主审独立验收（旧版 plan→confirm 交互），arm64 DMG 交叉构建静态验收通过。本轮直接 apply 新交互的源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；旧 DMG/旧 GUI E2E 不证明本轮新交互，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见上。公开发布仍受外部条件阻塞：GitHub billing、真实密钥、Developer ID/公证、Windows 签名、Linux/Windows/arm64 真机运行与安装、updater 真实签名下载安装回读均未就绪。

## 高级管理入口裁决记录（2026-09-12）

- Tools：退役。真实条目仅剩 playwright-cli（两条 npm 命令），实施为全新大域且投入产比低；退役后由用户手动管理。已于 `9080c1d` 清理完毕。
- Aliases：保留实施，为恢复后第一个最小闭环。已于 `b7f30db` 落地并通过主审独立验收。
- 更新中心：`system:refresh-upstream` 已实施（参与域 agents/mcp/skills，探测缓存落 SQLite `upstream_probes` 表，三域 status 只读回填 freshness，`--offline` 拒绝，不取 apply 锁）；`system:converge` 已于 `9080c1d` 退役清理；Updates 页直接触发真实 apply（apply 内部保留 preflight/写前校验），前端调用已补显式 `--action`（agents→upgrade，mcp/skills→update）。
- 快照：退役。新体系既定方向为 fail-closed 拒绝覆盖 + 操作内事务回滚，不恢复"写前持久备份"模型；"自动备份"承诺文案已于 `9080c1d` 清理。
- 探测缓存：落 SQLite 新表（易失状态入库与 operation_logs 先例一致），不引入第二份可写状态文件。
- 实施者：全部使用 Kimi YOLO 模式（AGENTS.md 已同步）。

## 2026-09-15 公共 plan/dry-run 退役裁决

- 公共 `*:plan`（`mcp:plan`、`rules:plan`、`hooks:plan`、`aliases:plan` 等）与 `--dry-run` 入口完全退役，请求一律被拒绝（plan 动词 UNKNOWN_VERB、`--dry-run` USAGE_ERROR），不再作为入口。
- status、query、诊断等只读路径保持零写；六域写操作仅通过真实 apply 完成。
- apply 内部保留 preflight/写前校验、精确 scope、排他锁、外部修改检测、CAS/条件回滚与操作审计。
- 定义保存仍只保存，不执行安装/链接/同步；破坏性定义移除、清空日志等高风险动作保持独立二次确认。
- MCP secrets 边界保留：SQLite 字面量 `${VAR}`；status/诊断/操作日志不暴露解析值；apply-time 可能按目标 codec 写入下游配置。

## 本次停止点与恢复入口

- 停止点：验收基线提交 `5a110be3` 的旧版 x86_64 DMG 与 GUI E2E、arm64 静态验收均已完成；直接 apply 改造已完成源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见上；本轮三提交（`d46b85a`/`47702ed`/`14e6790`）完成对外发布就绪的仓库内改造（LICENSE/元数据/CSP/图标/错误边界/自定义助手确认/MCP 空态/管线草稿先行单写者重构），发布管线（`14e6790`）与 Windows 阻断 CI（`9f56f00`）已就绪，公开发布仍受外部条件阻塞，未创建 tag 或 Release。
- 恢复入口（按序）：解决 GitHub billing；在仓库外/Secrets 配齐 updater/Apple/Windows 签名材料；让 GUI CI 对当前产品代码基线真实跑绿；Linux/Windows/arm64 各平台真机安装与运行独立验收；MCP 新增服务表单在 macOS 自动化输入条件具备时补验；由用户单独授权后打 tag（含 `-` 后缀时产出 prerelease draft），release workflow 创建 draft Release、上传四平台产物与 checksums 并单写 latest.json；逐平台签名/公证/updater 真实下载安装回读独立验收通过后才公开。

## 验证

`cargo fmt --manifest-path engine-rs/Cargo.toml -- --check`、两套 Cargo test/check，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。
