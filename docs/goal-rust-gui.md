# Rust 原生 GUI 目标与边界

## 当前结论

- 核心已就绪：Rust/SQLite/Tauri 共用库；2026-09-22 起数据模型改为「实际配置为准」（见下方裁决与 `GUI-REDESIGN.md`），源码、隔离实测与 x86_64 安装包真机验收已完成（见 2026-09-23 交付回执）。
- 2026-09-24 认证按官方标准全面校正（独立环境取消 shell 继承、登录检查只看目录自己的登录、Codex 致命写法识别、
  用量去重与实例统计等），源码、全量测试与真实 CLI 端到端已验证，安装包未重建，见「2026-09-24 按官方认证标准全面校正」。
- 同日独立复核后又修了 7 处（用量分叉归属、状态脱敏、失效定义、子进程隔离默认值与 Linux 限制等）并新增系统级
  设置提示，见「2026-09-24（续）独立复核后的修正」。
- 已接线闭环：Agents、Skills、MCP、Rules、Hooks、Aliases 六个资源域，以及只读 `system:audit`、`secrets:status`、`system:discover`（接管发现，见下）、`system:refresh-upstream`（探测宇宙 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表，三域 status 回填 freshness，前端收敛调用已补显式 `--action`）。
- 2026-09-12 高级管理入口裁决已执行完毕：Aliases 原生闭环落地（`b7f30db`）；Tools、快照与 `system:converge` 退役清理完成（`9080c1d`）。
- 远程 HTTP MCP 已落地（`b353aac`）：三客户端引用形式零明文落盘，与本地 stdio 同级安全语义。
- 2026-09-14 里程碑（历史）：基于 `5ae995b` 的干净 Darwin x86_64 DMG 通过主审独立验收；四平台发布源码管线（`a2cc864`）与 Windows 阻断 CI（`9f56f00`）就绪。
- 2026-09-15 里程碑：验收基线提交 `5a110be3` 的 x86_64 DMG 通过 verify 与主要真实 GUI E2E 主审独立验收；同源码 arm64 DMG 交叉构建静态验收通过（thin arm64、adhoc linker-signed，未在 arm64 设备运行）；完整矩阵 9/9 exit 0（Rust 269 项、Vitest 246 项、typecheck/build 通过且 0 warning）。
- 当前状态：公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝；status/query 只读零写；点击动作直接触发真实 apply；apply 内部保留 preflight/写前校验、精确 scope、排他锁、外部修改检测、CAS/条件回滚与审计。定义保存仍只保存；破坏性定义移除、清日志等独立确认不变。
- 本轮直接 apply 新交互已完成源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收：Engine 278 项、Tauri 11 项、Vitest 28 files / 248 tests、typecheck/build、8 份 schema 与隔离 smoke 全部通过；旧 DMG/旧 GUI E2E 不证明本轮新交互，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见下。仍未完成：MCP 新增服务表单真实 GUI 验收（macOS 自动化输入限制）；公开跨平台发布条件——GitHub billing、真实密钥、Developer ID/公证、Windows 签名、Linux/Windows/arm64 真机运行与安装、updater 真实签名下载安装回读；未创建 tag、GitHub Release 或对外安装包，现有授权不包含这些动作。
- 2026-09-16 发布就绪仓库内改造完成（三提交）：`d46b85a` 补齐 MIT 许可证、公开发布元数据、显式 Webview CSP 与应用图标，Bundle ID 定为 `io.github.ly87ing.agent-manager`；`47702ed` 增加前端顶层错误边界、自定义助手安装/升级确认与 MCP 空态引导，并清理退役代码；`14e6790` 将发布管线重构为草稿先行单写者编排（前置 create-release job 创建唯一 draft，矩阵以 releaseId 上传产物，收尾 job 单写 latest.json），支持 prerelease tag、逐平台 SHA256 checksums 与 engine-rs 版本一致性门禁，verify 脚本在已签名分支补充公证票据如实检查。对外发布仍未进行：GitHub billing、仓库 Secrets/Variables、签名与公证材料、Linux/Windows/arm64 真机验收、updater 真实链路均未就绪，未创建 tag 或 Release。
- 2026-09-20 用户友好闭环：助手页以接入工作台、运行设置和账号实例为主；AI CLI 的安装、升级、卸载脚本与完整定义编辑已撤出产品主路径。新增接入先保存完整定义与可选凭据，成功后才执行启用；客户端安装状态、版本与来源仅在设置的只读诊断页展示。数据库既有生命周期字段第一阶段兼容保留，不在本轮物理删除。

## 目标

让 CLI 与 Tauri 共享一个 Rust 原生库，生效配置以各助手真实文件为准，`native-manager.db` 保存偏好、归属记录与未生效的备选。设置保存在 `app_settings` 表，不引入可写 JSON settings 或 profile 层。

## 已完成

- `engine-rs` 的 native/store/envelope 核心服务 CLI 与 Tauri。
- SQLite 缺库读取零写、事务保存和重开读回。
- Tauri bridge 的命令、参数、run id 和异步边界。
- Policy 的 builtin/sqlite 来源和固定 `--layer local` 写入。
- Claude Code、Codex 官方 `native` 默认渠道；brew 只按实际检测来源处理。
- 根仓库 Python 与旧 `manage.sh` 入口清理。
- Runs 列表与详情使用 SQLite `operation_logs`，新增操作记录时清理过期记录；本地结构化诊断日志基于 `data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 日切写入（严格脱敏、Unix 0700/0600、只读零写、拒随软链接），在诊断日志写入后及应用启动时清理过期文件。两者共用 `log_retention_days`（0 禁用自动清理）；执行中心支持 `runs:clear` 二次确认清空任务记录，设置页支持 `logs:clear` 二次确认清理诊断日志，两类清理互不影响。
- Agent 原生运行配置与只读安装来源探测；既有生命周期 apply、锁与审计仅作兼容保留，普通 GUI 不提供入口。
- Skills 原生域接线与完整闭环：内置技能仓库只作推荐来源，添加后才成为用户集合（2026-09-22 起；此前为首次写库固化）；集合与 Skill×Agent 独立更新/启停/移除；内容寻址快照与安全软连接落地及回滚保护；status 只读零写；真实 apply 跨进程锁与操作审计。
- MCP 原生域接线与本地 stdio 闭环：Rust dispatcher 提供 `mcp:status`（只读）与真实 `mcp:apply`；定义与分配保存在 SQLite，支持 Claude JSON、Codex TOML、OpenCode JSON（需配置对应 Agent target）；逐助手支持 add/update/enable/disable/remove，库级支持 enable/disable/remove；status 只读零写，真实 apply 使用锁、审计、CAS 与回滚；env 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。
- `sources:get/set` 原生接线：规则来源固定为 `rules.shared`、`rules.codex`、`rules.claude`、`rules.opencode` 四项，保存使用 revision CAS 且只写 SQLite，不执行任何同步动作。（2026-09-22 起规则键已退役，规则改为实际文件 + 模板，见「实际配置为准裁决」。）
- Rules 原生域接线：Rust dispatcher 提供 `rules:status`（只读）与真实 `rules:apply`；固定目标为 `rules:codex_global`（`~/.codex/AGENTS.md`）、`rules:claude_global`（`~/.claude/CLAUDE.md`）与 `rules:opencode_global`（`~/.config/opencode/AGENTS.md`），内容按共享与助手专用来源渲染；status 只读零写，真实 apply 使用全局排他锁、操作审计、文件字节 CAS 与事务/文件回滚，外部修改 fail-closed 拒绝覆盖，不承诺备份；没有中间产物目录，来源直接渲染写入目标。Tauri bridge 参数白名单与 Rules 页面已对齐真实 schema，并支持逐目标 `--only`。
- Hooks 原生域接线：Rust dispatcher 提供 `hooks:status`（只读）与真实 `hooks:apply`；hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）使用 revision CAS 只写 SQLite；固定 Claude `~/.claude/settings.json` signature-set 合并（保留用户字段与非托管 hook）与 Codex `~/.codex/hooks.json` whole-file 两目标；status 只读零写，真实 apply 使用全局排他锁、Immediate 事务、精确旧字节 CAS、binding、审计与条件回滚，外部修改 fail-closed；前端两个目标与两个 JSON 编辑入口已接线。
- Aliases 原生域接线：Rust dispatcher 提供 `aliases:status`（只读）与真实 `aliases:apply`；aliases 专用 `sources:get/set`（aliases.shell）使用 revision CAS 只写 SQLite，alias 名/值结构校验与疑似凭据拒绝；固定 `~/.zshrc` 与 `~/.zprofile` 两目标的受管 marker 区块写入（区块外字节保留），status 只读零写，真实 apply 使用全局排他锁、Immediate 事务、整文件字节 CAS、binding、审计与条件回滚，外部修改 fail-closed，不承诺备份；前端 Aliases 页与来源面板零改动直接可用。
- 只读 `system:audit` 接线：聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态，支持 Tauri offline 桥接注入。
- 密钥闭环：`secrets:status` 只读输出变量名、来源与是否可解析；用户显式选择权限为 `600` 的外部 `.env` 文件后，填写和接管只写该文件，SQLite 只存路径，配置备份不含路径或取值。历史 `secret_values` 仅用于显式迁移，先写文件成功再删旧表。
- 2026-09-12 裁决退役已清理：Tools 域、快照（snapshots:list/restore）与 `system:converge` 的命令面、前端页面/类型/文案、契约 schema、种子文件与文档状态行全部清零；退役命令报错为 UNKNOWN_DOMAIN/UNKNOWN_VERB，六域 apply 输出不再携带 snapshot 字段，"自动备份"承诺文案改为 fail-closed 表述；`agent-tool/` 物理目录保留未动。
- `system:refresh-upstream` 接线：探测宇宙为 enabled 的 npm 渠道 agents、MCP 的 npx 包与 uvx `git+repo@branch` 源、enabled 且有 url 的 skills 集合仓库；探测器（npm view / git ls-remote / git rev-parse / npx 缓存读取 / bin --version）不经 shell、参数数组执行、有界捕获、超时进程组终止（网络 ≤30s），单元素失败归 unknown+固定错误类别不拖垮整批；比较语义对齐旧版（semver 可比 → current/behind/ahead，hash 不可比 → 不同即 behind）；缓存落 SQLite `upstream_probes` 表（Immediate 事务逐元素 upsert，只存版本/commit 与错误类别），status 只读回填 freshness（缺表/缺行一律 unknown，status 零写零网络）；`--offline` 拒绝（OFFLINE_NETWORK_DENIED），`--only` 逐元素，不取 apply 锁；前端 Updates 页直接触发真实 apply（apply 内部保留 preflight/写前校验），调用已补显式 `--action`（agents→upgrade，mcp/skills→update）。
- 远程 HTTP MCP 接线：远程定义 = `url`（仅 https 或本机回环 http，错误不回显 url）+ 可选 `headers`（值仅 `${VAR}` 占位符），与 stdio 字段互斥，`type` 由 codec 推导不持久化；三 codec 全部引用形式落盘零明文——Claude `type:"http"` 写字面量 `${VAR}`、Codex 写 `env_http_headers` 变量名映射、OpenCode `type:"remote"` 写 `{env:VAR}` 且固定 `oauth:false`；受管 marker 迁至条目级 `_MANAGED_BY`（stdio 旧位置兼容）；保存期 policy:set 跑完整定义校验；status/apply 审计对 headers 与 env 同级 `<literal>` 掩码，secrets:status 纳入 headers 并区分 `kind`；缺失变量同级 fail-closed。
- Skills 发现失败可见（`8a428de`）：集合元素 observed 新增 `discovery_error`（类别 + skill_paths 项 + 固定消息），发现失败不再静默为空；前端「全部技能」空态按集合落地状态给出引导，集合卡片展示发现错误。
- MCP 现有配置发现导入与接管（`a4ae118`）：`mcp:status` 增量输出 `import_candidates`（只读扫描三格式目标文件的未受管条目，env/header 值一律 `<literal>` 掩码，Codex `env_http_headers` 引用直通；受管/库内同名/binding/非法名排除）；前端一键转换为 `${VAR}` 占位符定义导入库（明文全程不出渲染进程），保存不执行动作；新增显式 `--allow-takeover`（仅逐 agent add/update）替换未受管同名条目，默认 fail-closed 不变，apply 审计恒含 takeover 字段。
- macOS 候选包构建身份（`3b28f45`）：发布候选安装包嵌入可信构建身份（commit/date），严格拒绝与源码错配的产物。
- 定义编辑原文保存（`5ae995b`）：SourceDefinitionsPanel 禁用文本框自动替换并有单测，Rules/Hooks 原文保存不再被 smart quotes 等自动改写。

## 契约要求

Skill、MCP 等资源及其逐助手分配必须支持新增、编辑/更新、启用、禁用、删除。保存定义不得执行动作；UI 直接触发真实 apply（apply 内部保留 preflight/写前校验、精确 scope、锁、外部修改检测、CAS/条件回滚与审计），普通同步、启停、更新不二次确认，破坏性动作独立确认。AI CLI 普通界面只维护接入、运行参数和账号实例；不提供安装、升级、卸载和完整定义编辑入口，软件生命周期交给原安装渠道。既有生命周期字段与引擎能力第一阶段兼容保留。

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

### 2026-09-17 追加：交互效率、缺口补齐与撤销（`44c4bf7`…`a270499`）

- 起因：借鉴 cc-switch 与业界做法后，用户裁决「全部处理」——交互效率三件套、上轮三个易用性缺口、总览前置与撤销。
- 总览与生效提示（`44c4bf7`）：总览从设置→高级管理提到侧栏第一项；执行成功后按目标补一句何时生效（助手下次启动、终端新开窗口、实例下次用该命令启动；规则还区分是哪个助手的目标），安装类动作本身即时可见不额外解释。
- 助手偏好与撤销（`544e4f3`）：「自动安装 / 自动更新 / 已启用」从只读标签改为可点开关，只写本机偏好、不触发安装或同步；移除定义（助手、实例、快捷命令、MCP、技能集合）后给限时撤销，文案分清能恢复什么——只写定义的可完全复原，已清理过目标配置的撤销只加回定义、要再点一次应用。
- 命令面板（`99e8216`）：`⌘K` 统一搜页面、助手、实例、技能集合、MCP 与常用动作，选资源跳到它所在页面（「某助手的技能」直接带预选），选动作走与页面相同的执行通道；方向键/回车/Esc 全键盘可完成，资源只在面板打开时才发起首次查询。
- 残留链接清理（`b3a0182`）：引擎新增 `skills:apply --action remove-link`（按 `--agent` 与 `--link` 指名，不属于任何集合所以不要 `--only`）——只删链接本身、绝不跟随链接删源，真实文件/目录 fail-closed，仍有分配记录的条目要走所属集合的 remove，审计写不进去就把链接放回原样；接管中心里这些条目改为可勾选批量清理，确认文案把「纳管」与「删链接」分开说。
- Hooks 结构化编辑（`bc9ecbb`）：按「事件 + 匹配器 + 命令」逐行填写，保存回填落盘结构并合并同事件同匹配器的命令；不理解的字段（timeout）与不展示的字段（removed_signatures）以读到的原值为基底原样保留并在界面标出，写入带版本号比对；原文编辑收进按需展开的入口。
- 托盘常用入口（`ae0a1b8`）：新增「搜索与执行…」（唤回窗口并打开命令面板）与「接管中心」。托盘只负责把人带到能做这件事的地方，不直接触发写操作，锁与审计因此仍走同一条既有路径。
- 自动化验证：`engine-rs` 305 项、`app/src-tauri` 15 项、Vitest 47 files / 408 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过；两套 `cargo fmt --check` 通过。
- 浏览器验收（dev server + mock）：`⌘K` 面板分组与资源条目正确渲染（30 项），Hooks 页两个结构化编辑器与空态正确，控制台无报错。
- 最终 clean x86_64 DMG（`a270499` 后重建，主审独立验收）：`verify:macos` 全部通过，构建身份 commit/date 与 `a270499` 精确一致；SHA256 `9be8e340f3052b23fa013cc22a216a94c11b0a3f446f3a4fd7ba29f2a33c6775`，8004560 bytes；未签名。过程中先构建出过一个 `-dirty` 包（三处测试断言与 fmt 结果漏提交），补提交后重建，未以 dirty 包充当验收结论。
- 未做/未验证，如实说明：托盘菜单的真机点击未验收（避免在宿主桌面弹出交互），托盘侧只验证了编译与事件接线、界面侧订阅有单测覆盖；命令面板与 Hooks 编辑的验证基于 mock 数据，未在安装包内做真机点击。

### 2026-09-18 追加：短板修正与真机 GUI 验收（`52e6bd6`…`25388e9`）

用户裁决「全部修正」后逐项处理上一轮自评列出的短板。

- 实例补规则与 Hooks（`52e6bd6`）：此前称「完整实例」但规则与 Hooks 的目标是固定的主实例路径。改为 profiles 域负责实例目录下的全部受管文件（Claude：settings.json 合并写入 + CLAUDE.md；Codex：agent-manager.config.toml + AGENTS.md + hooks.json），一个目录一个域，避免两个域整文件写入互相抹掉；binding 改为逐文件记录指纹。
- 启动参数预设（`05b9263`，用户追加需求）：按实际命令给常用参数，Claude 全自动、Codex 全自动与「保留沙箱」两档。取值来自本机实测 `--help`；本机这版 Codex 没有 `--full-auto`，因此不提供。危险档选中后明确写出后果。
- OpenCode 纳入内置定义（`81f7798`）：技能矩阵与 MCP 分配因此覆盖它；不给安装命令（无可靠核对过的官方安装方式），只登记为观测渠道。
- 配置备份（`5f50c12`）：`config:export`（只读）与 `config:import`（整体替换，一个事务）。导入走与逐条保存同一道闸门；分配记录与写入指纹不进备份——恢复后仍要各自应用一次才落地。
- MCP 命令可用性（`25388e9`）：status 多报一个 stdio 命令能否在本机解析，找不到时卡片直接提示；远程不做网络探测，恒「不适用」。
- **真机 GUI 验收（本轮补上此前完全缺失的一环）**：隔离 HOME/数据目录启动打包产物，AX 固定窗口位置后只截该窗口矩形（不做全屏截图），键盘驱动验证：应用启动与窗口渲染、`⌘K` 命令面板打开与搜索、无匹配时空态、回车跳转到接管中心、`⌘,` 打开设置、再次经命令面板跳转到 MCP 页；隔离环境数据目录与 HOME 均 0 项写入，应用日志无 panic/error。此前最后一次安装包内点击验收是 2026-09-15 的旧交互，其后所有新功能都只有隔离 CLI 与 mock 单测。
- 自动化：`engine-rs` 310 项、`app/src-tauri` 15 项、Vitest 49 files / 422 tests 全部通过；typecheck/build、两套 `cargo fmt --check` 通过。

本轮未做与做不到，如实记录：

- **Linux / Windows 验证本机做不到**：交叉编译缺 C 工具链（`aarch64-linux-gnu-gcc`、`x86_64-w64-mingw32-gcc` 均不存在），需要 CI 或真机，而 GUI CI 仍因 GitHub 账户计费在启动前失败。arm64 macOS 目标 `cargo check` 通过（只证明跨架构可编译，不等于真机运行验收）。
- **技能版本回滚：裁决不做**。内容寻址快照按哈希存放，没有 skill 维度的历史记录，要做得先加一张历史表；而本地来源集合是直链模式本就没有版本概念，git 集合的回退在源仓库侧操作更自然。投入产比不成立，需要时再议。
- **一个待用户裁决的观察**：内置默认的终端快捷命令里，`claude` 预置了 `--dangerously-skip-permissions`，即全新环境默认就是全自动。本轮加的参数预设已让用户可主动选择该档，但改内置默认会影响正在依赖它的现有用法，因此保持原样待裁决。
- 实例里的技能加载仍未端到端实证（新配置目录要先在该实例下登录才能起会话）。

### 2026-09-17 追加：启动参数编排、配置备份差异与 OpenCode 规则（`3478f54`…本次提交）

用户要求「常见参数由用户自己选、要看得见是否真的生效」，以及把上一轮自评列出的剩余项一次处理完。

- 高权限参数改为主动勾选（`3478f54`）：内置默认不再预置 `--dangerously-skip-permissions`；`aliases:status` 逐条回读期望取值与终端配置里的实际取值，界面按「已生效 / 还是旧的 / 区块里没有 / 还没写过」显示。只回答配置文件里现在是什么，不替用户断言终端会话已重新加载。
- 全局时区的位置与记号含义（`ce4a28f`）：有跟随项时提示改哪里；包装写法的原文编辑里写明 `{timezone}` / `{lang}` 会在写入时替换成什么。
- 配置导入前的差异（`a12cf0c`）：导入是整体替换且不可撤销，现在先读当前配置按类别列出新增、改动与消失的条目。看差异只读不写。
- OpenCode 成为规则域的第三个目标（本次提交）：`rules:status/apply` 固定目标由 Codex/Claude 扩到 Codex/Claude/OpenCode，目标文件 `~/.config/opencode/AGENTS.md`（[官方文档](https://opencode.ai/docs/rules/)：全局规则就在该路径）；来源新增 `rules.opencode`，默认留空——仓库里没有核对过的 OpenCode 专用适配正文，共享规则照常生效，专用内容由用户自己填。OpenCode 不加 Hooks 目标：它的扩展机制是 JS/TS 插件，不是 JSON 配置。
- 接管中心批量进度（本次提交）：批量执行时显示「第几项 / 共几项、正在做哪一项」与进度条，并说明失败那项跳过、后面继续。
- 首次使用引导（本次提交）：本地配置层为空（= 还没在这里保存过任何东西）时，默认落地页给出三步——先扫描接管、再分配、最后点「立即同步」才真正写文件，并直达接管中心。点「不再提示」后不再出现。
- 自动化：`engine-rs` 313 项、`app/src-tauri` 15 项、Vitest 51 files / 439 tests 全部通过；`pnpm typecheck` 与 `pnpm build` 通过；两套 `cargo fmt --check` 通过。
- 隔离 CLI 实证（OpenCode 规则目标）：隔离 HOME + 数据目录下 `rules:status` 列出三目标且都为「缺失」、零写；`rules:apply` 生成 `~/.config/opencode/AGENTS.md`（内容与其余目标同源共享段）；二次 apply 无动作；手工往该文件追加一行后 status 变为「已被外部修改」，再 apply 被 preflight 拒绝且用户那一行原样保留。
- 安装包内真机验收（隔离 HOME + 空数据目录启动打包产物，AX 固定窗口后只截该窗口矩形）：全新环境落地页按真实引擎数据显示首次使用引导，`⌘K` 跳转接管中心正常（各分类为空态）；隔离数据目录与 HOME 均 0 项写入，日志无 panic/error。截图同时印证内置默认的 claude 启动命令已不含跳过权限确认的参数，且逐条回读显示「还没写入终端配置」。
- 最终 clean x86_64 DMG（`31440f9` 后重建）：`verify:macos` 全部通过，构建身份 commit/date 与 `31440f9` 精确一致；SHA256 `f8a2ac041c272c877ef27d106e4ca1d84de948a99473020c7646175ca95982c5`，8034356 bytes；未签名。
- 浏览器验收（dev server + `?mock=fresh` 全新机器数据）：首次使用引导按三步渲染、「去接管中心」跳转正确、「不再提示」点后消失并记住；接管中心批量执行逐项捕获到 7 条进度文案（第 1/7 → 第 7/7），结束后进度条消失、按钮复位；900px 窗口无横向滚动；控制台无报错与警告。接管完成后本地配置层不再为空，引导自动消失（不依赖用户手动关闭）。

本轮未做与做不到，如实记录：

- **OpenCode 实例（多套独立配置）做不到，不做**。Claude 与 Codex 各有一个官方的「换一个配置目录」变量（`CLAUDE_CONFIG_DIR`、`CODEX_HOME`），实例模型建立在此之上。OpenCode 没有等价物：按[官方配置文档](https://opencode.ai/docs/config/)，`OPENCODE_CONFIG` 只换一个配置**文件**，`OPENCODE_CONFIG_DIR` 只是额外搜索 agents/commands/modes/plugins 的目录，两者都不迁移全局 `~/.config/opencode/AGENTS.md`，也不隔离登录凭据与会话（那些在 `${XDG_DATA_HOME:-~/.local/share}/opencode/`）。要凑出隔离得同时设 `XDG_CONFIG_HOME` 与 `XDG_DATA_HOME` 两个通用变量，且本机未安装 opencode、无法实测。因此不提供 OpenCode 实例，等官方给出隔离变量再议。
- **「助手页 6 条并发查询」不是真问题，不改**。实测本机引擎各只读命令（`agents:status` / `policy:get` / `aliases:status` / `profiles:status` / `system:audit` / `sources:get`）热态各约 10 毫秒，六条并发合计约 10 毫秒；首次 1.05 秒是二进制冷启动，与并发无关。桌面端引擎还是进程内调用，连启动开销都没有。此前测试里的超时是 jsdom 下的等待时限，不是引擎慢。
- Linux / Windows 真机、签名公证、实例内技能加载端到端实证，与上一轮记录一致，仍未做。

### 2026-09-18 追加：可用性实测与六项修正（`64cd48d`…`0aa5669`）

用户要求「测试和评估各个功能是否简单易用」，实测后逐项修正。评估用带存量资源的隔离环境走真实旅程，不看代码推断。

实测发现并已修的：

- **快捷命令：区块外的日常编辑会把同步永久卡死**（`64cd48d`）。外部修改检测比的是整个文件的摘要，用户往 `~/.zshrc` 里加一行 `export EDITOR=vim` 就再也同步不了——而这个域的全部承诺就是「只换受管区块、区块外逐字节保留」。改成只比受管区块；binding 补 `applied_block_digest`，旧行按空串回退整文件语义，补列放在写路径，status 仍零写。
- **实例启动命令是坏的**（`a0e33a6`）。生成的是 `CLAUDE_CONFIG_DIR="~/.claude-x"`，双引号里的 `~` 不展开，助手会在当前工作目录下另建一个名叫 `~` 的文件夹，而本工具写配置用的是绝对路径——两边不是同一个目录，实例功能实际不通。改成 `"$HOME/..."`，已写出去的坏命令回读时认得出来并给「订正」。
- **实例只能挂一条启动命令、别名不能改名不能复制**（`a0e33a6`）。现在实例卡片列出指向该配置目录的全部命令（按 `CLAUDE_CONFIG_DIR` / `CODEX_HOME` 反查，不加字段，别名仍只有别名域一个写者），每条可编辑/复制/删除；别名可改名（真正重命名，不留两条）与复制；实例整体也能复制，复制时默认换掉配置目录。
- **规则与快捷命令被手改过之后是死路**（`2e5c105`）。状态变「已被外部修改」、可用动作为空、同步永久失败，引擎不接受 `--allow-takeover`，页面也没有入口；而同页面的 Hooks 有。照 hooks 给两个域各补三处，只对「当前内容读得到」的冲突放行，错误文案点名是哪个目标、下一步点什么。
- **密钥只认进程环境，GUI 启动基本拿不到**（最初由 `4d60857` 修复，后续安全边界已收紧）。现在「密钥」页先选择外部私有 `.env` 文件，再逐个变量填写；解析顺序为先外部文件、再回退进程环境，明文不再进入本机数据库。
- **接管技能目录只收录已链接过的那几个**（`0aa5669`）。源目录里其余含 SKILL.md 的技能一并收进来，逐条标注哪些已在用。
- **矩阵里「不支持」会被读成「这个助手用不了技能」**（`0aa5669`），改「未分配」，原因文案直接说下一步。

- 自动化：`engine-rs` 317 项、`app/src-tauri` 15 项、Vitest 51 files / 451 tests 全部通过；typecheck、build、两套 `cargo fmt --check` 通过。
- 最终 clean x86_64 DMG（`7e9cf47` 后重建）：`verify:macos` 全部通过，构建身份 commit/date 与 `7e9cf47` 精确一致；SHA256 `8761d7f2bc6ae0d2a11792265a87eac57c38054bf0ae14c07273438a23616adb`，8048422 bytes；未签名。
- 安装包内真机验收：隔离 HOME + 空数据目录启动打包产物，界面正常渲染，隔离数据目录与 HOME 均 0 项写入，日志无 panic/fatal。
- 浏览器验收（dev server + mock）：⌘K 搜「密钥」直达该页，逐变量给出「已填写 / 已配置（来自环境）/ 未配置」与填写入口；快捷命令每条都有复制，复制预填 `claude-copy`，编辑态名字可改；控制台无报错。
- 隔离 CLI 实证（每项都在隔离 HOME + 空数据目录下真跑）：区块外加行后状态仍 managed-ok 且能同步、用户那行原样保留，区块内被改则拒绝并可接管；规则外部改动后 `--allow-takeover` 精确到单目标接管成功、多目标接管被拒；密钥填写后 status 的 `resolved/stored` 正确、apply 真的用上该值、操作记录与配置备份均不含明文、清空即撤销；接管扫描对「源目录 3 个技能只链接了 1 个」给出全部 3 个。

本轮未做与做不到，如实记录：

- **接管 MCP 时需要保持原配置可用**。发现结果仍不携带明文；用户确认接管时，引擎只读目标服务的字面量取值并转存到“设置 → 密钥”选定的外部文件，回执只报变量名。同名不同值或外部文件不可用时整项停止，不继续覆盖助手配置。
- **配置备份不带密钥取值**（本轮按此默认实施，用户未另行指定）。换机后需重填一次，好处是备份文件本身不是密钥载体。
- **「助手页 6 条并发查询」不是真问题，未改**。实测本机各只读命令热态各约 10 毫秒、六条并发合计约 10 毫秒；首次 1.05 秒是二进制冷启动。
- Linux / Windows 真机、签名公证、实例内技能加载端到端实证，与上一轮记录一致，仍未做。

### 2026-09-18 追加：合并写入目标的检测口径与接管保值（`f1b7123`…）

上一轮评估后用户要求继续处理剩余项。核心是同一个 bug 家族的另外两处。

- **Hooks 的 Claude settings.json**（`f1b7123`）：signature-set 合并的前提就是「用户可以有
  自己的字段」，但外部修改检测比的是整文件，用户动一次 model/permissions/statusLine，hooks
  就被判「外部修改」、每次都要点接管。而同一产品里 MCP 早就是条目级比对——不是做不到，是没
  统一。改成只比受管 hook 切片的摘要；Codex 的 hooks.json 整份归本工具，仍比整文件。
- **实例的 settings.json**（`e406c0d`）：同样是合并写入（本工具只管 env/model/hooks），
  同样拿整文件做检测。改成只比这三个键；拒绝文案原来说「不是本工具写入的那份」也是错的，
  那份就是本工具写的、用户后来加了字段。
- 两处都用同一套兼容：旧绑定存的是整文件摘要，整文件没变也照常放行，下次写入时自动升级为
  切片摘要——不改表结构，存量用户不会被误报一次。
- **接管 MCP 现在真的保值**（`e69d443`）：新增 `secrets:adopt --agent --server`，只读该助手
  自己的配置、只取明文字面量（已是 `${VAR}` 的与 `_MANAGED_BY` 跳过），按与界面一致的规则
  推变量名存进本机库。界面接管流程改为「先逐助手 adopt、再写定义、最后写回」，顺序有断言——
  晚接就会用另一个值同步。此前只是在确认框里提示要自己补填。
- 命令面板补齐设置下的四个二级页（密钥、终端快捷命令、更新中心、策略）。
- `agent-rule/CLAUDE.md` 里几节仍把 `tests/*.py` 当作在跑的门禁，而 `tests/` 早已随 Python
  实现清理删除。改为「纪律仍然有效，但没有自动门禁替你把关」，并标明工具需从 git 历史恢复。

**实例内技能加载：端到端实证补上了**（此前一直记为未验证）。此前认为需要先在实例目录下登录
才能验，实际不需要——Claude Code 的技能发现发生在鉴权之前。用隔离 HOME 建实例、把技能分配
给它、再用该实例的配置目录起 `claude -p` 并收调试日志，日志写明：

```
Loading skills from: ... user=~/.claude-work/skills, project=[]
Loaded 1 unique skills (... user: 1 ...)
Watching for changes in skill/command directories: ~/.claude-work/skills
```

即分配给实例的技能确实被该实例加载，主实例目录 `~/.claude/skills` 根本不存在、未被牵连。
`profileSkillsDir(configDir) = <configDir>/skills` 这个假设由此有了实证。

- 最终 clean x86_64 DMG（`c7f58c3` 后重建）：`verify:macos` 全部通过；SHA256 `31ef45d8ba79d7a4030a289058bd549ad2dd1829e48485a046ffb05fc915e2d5`，8080588 bytes；未签名。
- 上一版 DMG（`04555d3` 后重建）：`verify:macos` 全部通过；SHA256 `520ab39ffbe94df44ae039d5119cd0e978ace04adb4e97d259b633b33565b084`，8077673 bytes；未签名。

- **配置备份改为带上凭据取值**（`c7f58c3`，用户裁决）：此前不带、换机需重填。现在 export 带
  `secrets` 段、import 整体恢复，A 机导出 → B 机全新导入 → 变量当场可解析 → 直接同步，写进
  助手配置的值与 A 机一致。代价是这份文件等同密钥文件：界面因此不再把备份 JSON 铺在屏幕上
  （只报「N 条定义 · M 份来源 · K 个凭据取值」加保管提示），复制到剪贴板拿到的仍是完整备份；
  导入差异新增「凭据取值」一节，只列变量名与新增/改动/消失。status、操作记录、差异预览均不
  含取值。顺带补上一个真缺口：备份的来源清单一直漏了 `rules.opencode`。

其余实测：Hooks 与实例都验了「改自己的东西照常同步且内容保留」「动了本工具写的那部分才拒绝、
接管后用户内容仍在」；接管保值验了「写回的值与原值一致」且 status/备份/回执均不含明文。

### 2026-09-18 追加：真机试用反馈的功能修复与界面重做（`575bbc6`…`b0f8519`）

用户在真机上逐页点过，提了功能失败、参数覆盖、缺全局同步、以及「这几个页面相当于完全没有
设计」四类问题。逐条查证后处理。

**真机点出来的三处功能失败**（`cc5880e`）。查操作记录与诊断日志拿到全部原因，不靠猜：

- 接管 MCP 报「失败 4」：3 条是「已存在残留受管条目，add 不适用，请先执行 remove 清理」。
  那些条目是本工具以前自己写的，接管中心认出来还打了徽标，却仍只发 add——而界面上根本走不到
  remove 那一步。改为带 `--allow-takeover` 时直接替换（残留内容不是用户的外部内容，覆盖它
  零风险），不带仍拒绝且文案说清下一步。第 4 条是「凭证变量未配置」，属实需要用户补填。
- 更新中心对 MCP 发库级 `--action update`，被引擎拒绝——MCP 的 update 是逐助手动作。改为按
  `observed.agents` 里 assigned 的助手逐个展开；一个都没分配时直说「先分配再更新」，不再发
  一条必失败的请求。
- 接管失败只报「失败 4，详情见任务记录」等于什么都没说，改为直接列出每条的对象与原因。

**界面**：toast 从右上移到右下（右上正好盖住各页标题与主操作按钮，而执行结果恰恰是点完那些
按钮才弹出来的）；启动命令不再从 `LC_ALL` 中间断开，改横向滚动；更新中心两个时间列合并。

**启动参数预设扩到五个助手**（`0785797`）。参数全部来自本机实测 `--help`，各家叫法完全不同，
照抄别家会写出根本不存在的参数：Claude/Antigravity `--dangerously-skip-permissions`；
Codex `--dangerously-bypass-approvals-and-sandbox` 与 `--ask-for-approval never --sandbox
workspace-write`；Grok `--permission-mode bypassPermissions|acceptEdits`（取值不是开关）；
Kimi `--auto` 与 `-y`。每家一个全自动档加一个折中档，危险档标出后果。

**全局同步中心**（`0785797`）。配置分散在六个域，挨个页面看差异再各点一次同步太费事。总览
最上方汇总，并把两类分开：可一键做的（引擎给了可执行动作）走批量、失败跳过、结束汇总；
被外部改过或旧体系留下的绝不混进批量，列出原因并给「去处理」直达该页。判定只依据引擎的
state 与 actions，不自己推断；用户主动停用的不催。syncPlan 是纯函数，六个用例覆盖。

**三个页面按同一套原则重做**（`b0f8519`）。用户说这几页「相当于完全没有设计」。把所有页面
按同一把尺子量过，问题确实集中在这三个，其余（助手、接管中心、规则与 Hooks、操作记录、
服务库）是健康的。共同根因是把引擎的数据结构原样铺成表：

| 页面 | 改前 | 改后 |
| --- | --- | --- |
| 总览 | 2.2 屏 / 11 区块，Agents/MCP/Skills 各出现两次 | 1 屏 / 8 区块 |
| 全部技能 | 25 行（5 技能 × 5 助手） | 5 行 |
| 助手中的服务 | 35 行（7 服务 × 5 助手） | 7 行 |

三条原则（其余页面本来就符合）：① 资源 × 助手用矩阵，不铺笛卡尔积——新增共用
`ResourceAgentMatrix`，一行一个资源、每个助手一格，点开那一行才出逐助手操作，可同时展开多行
对比，能力一个不丢；② 拿不到值的列不占位——删掉整列「未获取」的版本列，以及列名写「版本」
内容却是「已链接/未配置」的那列（「当前配置版本=未配置」和「状态=未配置」并排谁都看不懂），
细节进悬停；③ 不把引擎内部说法给用户——「当前引擎未提供操作」改为有阻断原因就显示原因，
没有才说「这里没有可做的」。总览按用户选的「我现在要不要做事」定位重做：版本部分与更新中心
完全重复，压成一句结论加一个入口；体检明细默认收起。

另外「残留」在界面上改叫「以前配过」——那个词对用户没有意义。

- 自动化：`engine-rs` 326 项、`app/src-tauri` 15 项、Vitest 52 files / 472 tests 全部通过；
  typecheck、build、两套 `cargo fmt --check` 通过。
- 最终 clean x86_64 DMG（`07583cc` 后重建）：`verify:macos` 全部通过；SHA256 `311805bebb50908f1e6b71164f4cdf63680fa9331c559de080ca7c5c72c3e259`，8085021 bytes；未签名。
- 实测：接管残留条目在隔离环境验到 `leftover → managed-ok, assigned=true`；三个页面的行数、
  屏数、零信息列与黑话出现次数在浏览器里逐项量过（数字见上表）。

### 2026-09-20 追加：助手接入工作台第一闭环

> 历史里程碑：其中「安装与维护」和「管理助手」入口已被同日第五闭环撤下。

- 起因：助手接入原来藏在完整定义弹窗与卡片下拉里，新增、切换、测试分散，默认页面先展示安装
  维护信息。用户要求借鉴 cc-switch 的低操作成本，并继续做到更完整、可信的使用闭环。
- 改动：助手页顶部新增任务式接入工作台，先选助手，再以卡片展示默认账号和每套接入；当前项、
  地址、模型、凭据变量与测试结果就地可见，非当前项一键启用。下方单独保留「安装与维护」区，
  完整定义编辑入口改名为「管理助手」。新增接入弹窗先选按 Claude/Codex 形态过滤的厂商预设或
  自定义，再确认必要字段；高级字段按需展开。
- 状态边界：新增接入只替换目标 profile，保留未知字段、其他接入和当前 active。Claude 可选填
  凭据取值并通过 `secrets:set` 单独保存；Codex 只登记环境变量名。勾选立即启用时，定义与可选
  凭据成功后才调用 `agents:configure --use`；切换成功回读 status 后才提示完成。
- 自动化：Vitest 55 files / 501 tests、Rust `agent_settings` 集成测试 24 项、`native_secrets`
  5 项与 Tauri 14 项全部通过；`pnpm build` 通过。新增用例覆盖完整字段保留、当前/非当前卡片、
  一键切回默认、执行中互斥、只读测试、预设新增、凭据写入顺序与失败重试、取消零写入、校验错误、
  助手切换、非支持助手只读边界与焦点恢复。
- 浏览器验收：dev server + `?mock=1` 下验证 Claude/Kimi 与 Codex/Kimi 两条预设路径；Claude
  自动填 Anthropic 端点并提供可选凭据框，Codex 自动填 Responses 端点且不出现明文凭据框；
  Esc 关闭后焦点回到「添加接入」。900×800 和默认视口均无横向溢出，控制台无错误或警告。
- 边界：本闭环只改前端编排、测试与文档，复用已验收的 Rust `agents:configure`、`agents:check`
  与 `secrets:set` 契约；尚未重建安装包，也不把浏览器 mock 验收称为安装包真实写入验收。

### 2026-09-20 追加：命令面板接入快切第二闭环

- 起因：第一闭环把新增、查看、测试和切换收进助手页，但用户仍要先导航到该页。cc-switch 的高频
  优势之一是供应商切换随手可达；本项目已有跨资源的 `⌘K` 与托盘搜索入口，因此复用它，不再
  增加一套绕开确认和审计的托盘写路径。
- 改动：命令面板索引 Claude Code、Codex 的默认账号与每套接入，支持按显示名、内部名、地址、
  模型和「供应商/接入/切换」搜索。当前项就地标明并禁用；选择其他项直接执行切换，切回默认使用
  `--use-default`。写操作进入统一操作记录，成功后回读 `agents:status`，只有 active 与目标一致才
  显示成功；外部修改拒绝时才弹接管确认并以 `--allow-takeover` 重试，其他失败不改变界面事实。
- 无账号验收：DEV mock 新增一套 Codex/Kimi 示例，并实现 mock 内存态的 `agents:configure` 与
  `agents:check`；它只在开发构建下加载，生产构建会剔除。真实厂商连通性因当前没有合适测试账号
  跳过，不以模拟响应代替。
- 自动化：Vitest 55 files / 506 tests、Rust `agent_settings` 24 项与 Tauri 14 项全部通过，
  `pnpm build` 通过。命令面板新增用例覆盖 profile 切换、切回默认、操作记录 ID、状态回读、
  当前项禁用、外部修改二次确认和普通失败；DEV mock 另覆盖测试、切换与回读。
- 浏览器验收：`?mock=1` 下用键盘打开命令面板，搜索 Kimi 后回车切换，成功提示明确说明重启生效；
  再次搜索时 Kimi 显示为当前项且不可重复执行。900×800 无横向溢出，控制台无错误或警告。
- 边界：本闭环没有改 Rust 命令、数据库结构和托盘事件；尚未重建安装包，浏览器 DEV mock 不能
  证明真实助手配置文件已写入。

### 2026-09-20 追加：接入就地编辑与删除第三闭环

- 起因：接入工作台已经能新增、切换和测试，但改地址、模型或删除旧接入仍要进入完整助手定义，
  高频任务又回到了低频维护界面。
- 改动：每张非默认接入卡片新增「编辑」「删除」。编辑直接预填现有字段；当前项默认保存后更新
  助手配置，非当前项默认只保存，不暗中切换。内部唯一名称在编辑时固定。删除只允许非当前项，
  当前项按钮禁用并说明先切换；确认删除只移除目标 profile，凭据变量保留供其他接入复用。
- 数据边界：编辑目标 profile 时，先剥离表单拥有的字段再合并新值，使可选项可真正清空，同时
  保留当前表单不认识的 profile 字段。删除保留其他 profiles、active、运行参数和定义未知字段；
  helper 仍防御性清理悬空 active，但界面不会走到删除当前项。
- 自动化：Vitest 55 files / 512 tests 与 `pnpm build` 全部通过。定义 helper 与助手页新增用例
  覆盖未知字段保留、可选字段清空、当前/非当前编辑语义、删除确认、凭据不删除、当前项禁删和
  零误写。Rust 契约与 Tauri 本批未改，沿用同一轮已通过的 `agent_settings` 24 项与 Tauri 14 项。
- 浏览器验收：`?mock=1` 的 Codex/Kimi 卡片完成编辑预填、高级字段、Esc 焦点恢复、删除确认与
  切为当前后删除禁用验证；900×800 无横向溢出，控制台无错误或警告。
- 边界：未改 Rust 契约与数据库结构，未重建安装包；没有合适账号，未做真实厂商联网检查。

### 2026-09-20 追加：接入安全复制第四闭环

- 起因：新增相似线路仍要重新选择预设并逐项填写；cc-switch 已把复制作为厂商卡片的日常动作，
  本项目需要借鉴这一低摩擦入口，同时避免把凭据与当前生效状态一起隐式复制。
- 改动：每张接入卡片新增「复制」。副本预填地址、模型、认证方式、凭据变量名与高级参数，名称
  自动使用 `<原名>-copy` / `<原名>-copy-2` 避让，64 字符长名称会先安全截断。副本唯一名称可编辑，
  默认只保存、不切换；用户显式勾选后才启用。
- 数据与安全边界：复制来源 profile 中表单尚不认识的扩展字段，但不读取凭据值，凭据输入框始终
  为空；来源 profile、active 和其他定义保持不变。
- 验证：定义 helper 与助手页定向测试共 48 项通过，覆盖名称碰撞、64 字符上限、未知字段复制、
  来源不变、凭据为空、默认不切换与零 `secrets:set`；Vitest 全量 55 files / 515 tests 和
  `pnpm build` 通过。浏览器 DEV mock 完成预填、唯一名称可编辑、重名阻止保存、Esc 焦点恢复验证；
  900×800 无横向溢出，控制台无警告或错误。
- 边界：未改 Rust 契约与数据库结构，未重建安装包；没有合适账号，未做真实厂商联网检查。

### 2026-09-20 追加：运行时优先与客户端诊断第五闭环

- 起因：真实窗口截图显示助手页仍被安装维护卡片、完整定义和命令文本占据，运行时与账号配置反而
  不够直观。对照 cc-switch 的高频路径后，将产品职责收敛为 Provider/接入、变量、运行设置与账号
  实例；AI CLI 软件安装和更新继续使用官方安装器、Homebrew、npm 等原渠道。
- 改动：助手页撤下安装/升级/卸载卡片、完整 alias、完整定义编辑及助手增删停用入口；接入工作台
  新增紧凑的「运行设置」，Claude 可控制状态栏与权限模式，Codex 控制权限模式。设置页新增只读
  「客户端诊断」，集中展示是否安装、当前版本和识别到的安装来源，不提供生命周期动作。接管中心和
  总览同步移除“由本工具升级”暗示及 AI CLI 版本更新展示。
- 数据边界：`native-manager.db` 仍是唯一事实源；保存运行设置只拥有 `settings.yolo/statusline`，完整
  保留 profiles、active、未知扩展字段及既有生命周期字段。第一阶段不改 Rust schema，也不物理删除
  兼容能力；后续清理必须先审计调用方、CI、打包、文档和迁移边界。
- 验证：Vitest 56 files / 495 tests、`pnpm build`、engine-rs 373 tests、Tauri 14 tests 全部通过。
  在 1792×1044 原生 Tauri 窗口实测助手主内容一屏完整显示，运行设置弹窗无需滚动，客户端诊断页
  仅有只读状态且无安装/更新按钮。
- 边界：本轮没有合适的真实厂商账号，联网接入测试按授权跳过；未重建安装包。原生窗口验收证明
  当前开发构建的布局与交互，不等同于安装包验收。

### 2026-09-20 追加：CC Switch 能力迁移与认证安全闭环

- 范围：当前产品入口只服务 Claude Code 与 Codex；OpenCode 从内置发现、Skills/MCP 分配和 Rules
  当前目标中移除。数据库里的旧定义与内部兼容 codec 不主动删除，避免升级时破坏既有数据。
- Provider：保留既有接入卡片、预设、新增、切换、测试、编辑、删除、复制与命令面板快切，并补齐
  排序、无密钥分享 JSON 导入导出。排序、导入和普通保存只改定义；导入不携带 `apiKey`、不自动
  启用，不照搬 CC Switch 深链可带密钥并立即切换的危险语义。
- 认证：新增脱敏只读状态；固定文件、拒绝符号链接与异常内容，只报告认证方式和可用性。普通配置
  路径不写 Claude `.credentials.json` 或 Codex `auth.json`。登录、退出按钮只复制官方命令，应用永不
  后台执行；实例命令带隔离的 `CLAUDE_CONFIG_DIR` / `CODEX_HOME`，由用户核对后在终端明确执行。
- 多账号实例：继续以独立配置目录隔离登录、会话、Skills、MCP 和启动命令；Skills/MCP 页面从
  `profiles:status` 回读实例真实形态，既保留 Claude/Codex 实例，又不误纳其他客户端。
- Skills：保留本项目更强的内容寻址快照、受控软连接、逐实例分配、外部修改 fail-closed 与可逆停用；
  不引入第二套 ZIP/历史备份模型。MCP 同样保留变量占位符、精确作用域、未受管接管保护、CAS、
  条件回滚与审计，仅吸收更直接的筛选和批量交互。
- 验证边界：自动化与浏览器 mock 只证明命令生成、状态展示及零误写编排；本轮未执行任何真实
  `claude auth login/logout` 或 `codex login/logout`，也不以模拟结果冒充真实账号认证验收。
- 验证结果：Vitest 57 files / 521 tests、前端类型检查与生产构建、engine-rs 384 tests、Tauri
  14 tests 全部通过。浏览器 mock 实见 Claude/Codex 脱敏认证状态、只复制不执行入口、Codex
  `auth.json` 保留提示、接入排序/导入导出，以及实例仍出现在 Skills/MCP 助手矩阵中。
- 本机配置回读修正：此前接入工作台只读数据库定义，把空 `settings` 显示为“当前使用”，没有解析
  已存在的 Claude/Codex 客户端配置。现在 `agents:status` 从固定目标只读返回脱敏观察值；界面分开
  显示客户端实际配置与已纳管选择；运行设置弹窗分成可编辑表单、写入预览和不可保存的“本机
  实际值”。读取零写、拒绝符号链接和异常文件，Claude 明文 token 只报告“已配置”，不进入状态
  JSON、日志或数据库。
- 本机配置一致性与就地修复：接入工作台对比可安全观察的受管 provider 字段，区分“一致”“不一致”
  与“无法比较”；漂移时仅在用户显式点击后重新应用已纳管选择，复用既有外部修改确认、审计与状态
  回读，不自动接管。扩展后的验证为 Vitest 57 files / 523 tests、engine-rs 384 tests、Tauri 14 tests，
  前端类型检查与生产构建全部通过；浏览器 mock 实见漂移修复后转为一致，320 px 宽度无横向溢出且
  控制台无错误。本轮仍未在真实 HOME 写入客户端配置，未执行真实 provider 联网、认证或安装包验收。
- 多接入批量检查：用户可显式并发检查当前助手的全部自定义接入；卡片分别保留成功、失败与延迟，
  单项失败不阻断其他结果，全部完成后汇总可用数量与最快线路。检查只调用既有只读
  `agents:check`，不切换当前接入、不写配置。Vitest 扩展为 57 files / 524 tests，前端类型检查与生产
  构建通过；浏览器 mock 实见“检查全部接入”到最快线路汇总。本轮没有真实 Provider 凭据，未把
  mock 结果视为真实联网验收；后端与 Tauri 本批未修改。
- 窄窗口布局修正：320 px 实图检查发现固定侧栏挤压主内容，虽无页面横向溢出但正文已变成竖排。
  窄屏现改为顶部横向导航，桌面侧栏保持不变；复验页面宽度与滚动宽度均为 320 px，主内容恢复
  正常排版且控制台无错误。导航与快捷键相关目标测试 33/33、前端全量 57 files / 524 tests、类型
  检查与生产构建通过。
- 最快接入直达与切换回读门禁：批量检查完成后，若最快成功线路不是当前项，可直接启用；该动作不
  新增写路径，仍走 `agents:configure`、外部修改确认和 `agents:status` 回读。助手页此前虽执行
  `refetch()`，却未核对回读结果；现在同时校验数据库 `active` 和客户端脱敏
  `observed.current_config`。任一侧未与目标一致都保留真实界面并报错，不再误报切换或重新应用成功。
  Vitest 为 57 files / 525 tests，类型检查与生产构建通过；浏览器 mock 实见 Claude 漂移重新应用后
  恢复一致，以及默认账号经 Kimi Code 128 ms 推荐切换为 Kimi Code 后两侧均一致，控制台无错误。
  未用真实 Provider 凭据执行联网与切换验收。
- 命令面板切换统一双回读：`⌘K` 快切不再只核对数据库 `active`，而是与助手页共用完整 profile
  解析及脱敏客户端配置比对；`active` 已变化但客户端实际配置未生效时不再提示成功。Vitest 扩展为
  57 files / 526 tests；浏览器 mock 以键盘完成 Codex/Kimi 切换，再次搜索时当前项禁用，助手页回读
  纳管选择与客户端实际配置一致，控制台无错误。后端、Tauri 与写入协议本批未修改。
- 命令面板就地恢复当前接入：已纳管选择仍指向某接入、但客户端实际配置不一致或无法确认时，不再
  把该项误标为不可操作的「当前接入」，而是显示「配置不一致 · 重新应用」并允许用户显式执行；
  双回读确认一致后才恢复为禁用的当前项。Vitest 扩展为 57 files / 527 tests；浏览器 DEV mock
  实见 Claude 默认账号从不一致、可点击，到重新应用后助手页显示一致且命令项禁用，控制台无错误。
  本批未改后端、Tauri 与写入协议，也未用真实客户端配置或安装包验证该交互。
- 托盘接入直达：托盘动态按 Claude Code/Codex 分组展示默认账号与自定义接入，可信当前项带勾且
  禁用，不一致项提示重新应用。点击非当前项只唤回窗口并把命令面板精确过滤到该目标；用户按一次
  Enter 或点击后才进入既有 `agents:configure`、外部修改确认、审计与双回读，保持 `ae0a1b8` 的
  “托盘不静默写配置”安全裁决。Vitest 扩展为 57 files / 529 tests，Tauri 为 15 tests；新增用例
  证明托盘事件本身零写入、确认后才切换，以及外部修改不会被静默覆盖。另以隔离数据目录和前端
  DEV mock 实际点击同源 Tauri 应用的 macOS 状态栏菜单：Codex/Kimi Code 项能唤回窗口并精确过滤
  命令面板，点击托盘后未自动切换，按 Enter 显式确认后出现成功回执；再次展开原生菜单时，标题已
  更新为 Kimi Code，目标项带勾且禁用。该验收未读取或写入真实 `~/.claude`、`~/.codex`，也未
  操作正在运行的 Claude/Codex；安装包、真实客户端配置和 Provider 联网仍未验证。
- 接入备注与检索：每套接入可保存不超过 200 字的备注，卡片直接展示，`⌘K` 同时按备注检索；备注
  只存数据库定义，不参与 Claude/Codex 配置渲染，复制与无密钥分享会一并携带。Vitest 为 57 files /
  530 tests，类型检查与生产构建通过；浏览器 DEV mock 实见卡片备注，并以“月底到期”唯一过滤到
  Codex/Kimi Code 接入，未执行切换。未启动原生应用，也未读取、写入或操作真实 Claude/Codex。
- 键盘接入快切：新增 `⌘⇧K`，直接打开只含 Claude/Codex 默认账号与自定义接入的范围；输入继续按
  名称、备注、地址或模型过滤。Esc 后再按 `⌘K` 会恢复完整命令面板，文本输入语境不抢快捷键；
  执行仍复用既有外部修改确认、锁、审计和双回读。Vitest 为 57 files / 531 tests，类型检查与生产
  构建通过；浏览器 DEV mock 实见 3 个接入项、备注唯一过滤及完整面板范围恢复，未执行切换，未启动
  原生应用或操作真实 Claude/Codex。
- 用量联动筛选：用量页新增「助手 → 模型」下拉条件，与时间范围共同驱动汇总卡、未定价提示和
  明细表；切换时间会清除助手与模型，切换助手会清除模型，避免统计口径残留。筛选只处理
  `usage:status` 的已读结果，不触发扫描或写入。Vitest 扩展为 57 files / 533 tests，类型检查与生产
  构建通过；浏览器 DEV mock 实见助手筛选后汇总和明细同步变化。本批未启动原生应用、未点重新统计，
  也未读取、写入或操作真实 Claude/Codex。
- Provider 与运行管理收口：Provider 创建、编辑、应用彼此独立；使用中的自定义
  Provider 必须先切换或重置才能删除，删除有二次确认；官方默认 Provider 只提供带确认的重置。
  Claude/Codex 自动化改为手动、智能自动、完全放开三档，按适配器写入真实客户端字段。主 Agent
  页面直接审查、新增、编辑、删除和显式同步它自己的运行别名，隔离实例仍按配置目录归属；别名定义
  与 shell 落地保持两个动作，事实源仍是 `aliases.shell`。Vitest 57 files / 539 tests、前端类型检查与
  生产构建、engine-rs 386 tests、Tauri 15 tests 全部通过。隔离 HOME 实际完成 Provider 创建/应用、
  `automation_mode=auto` 写入、官方重置、重置后删除，以及两条别名同步并回读 `managed-ok`；真实
  原生窗口只读确认了运行别名和官方重置界面，并打开后取消两类确认框。未操作真实 Claude/Codex，
  未构建或验收安装包。
- 环境化信息架构：助手页固定为「Agent → 默认环境/独立环境 → 当前环境内容」。默认
  环境拆成接入 Provider、自动化与权限、启动命令、实际配置四页；添加 Provider 使用明确文字入口，
  独立环境从弹窗迁到同一工作区并按当前 Agent 过滤，创建入口与 Provider 完全分离。启动命令含权限
  参数时显示“覆盖环境自动化”及具体效果；实际配置直接展示目标文件和脱敏后的回读值。未改变 SQLite、
  Provider、profile 或 alias 协议，`native-manager.db` 仍是唯一事实源。Vitest 57 files / 542 tests、
  前端生产构建、engine-rs 与 Tauri 15 tests 全部通过；原生 Tauri 窗口只读验收了默认环境四页与
  独立环境空态，未点击任何应用、同步或确认动作，未改动真实 Claude/Codex 配置。
- Provider 接管差异审查：外部修改拒绝覆盖后，确认框不再要求用户盲选，逐项展示软件配置与客户端
  当前值；用户可只覆盖受管字段，也可通过专用精简表单把客户端当前地址、模型及脱敏凭据元数据
  保存为新 Provider。内部名称隐藏，缺少必填项时明确提示并禁止保存；未读取或保存凭据明文，取消
  仍保持客户端文件零写。
- 全局操作可理解性审查：恢复“导入现有配置”的侧栏常驻入口，并同步命令面板与托盘用词；
  总览“全部同步”改为先列出本次将写入的待处理项目，再由用户确认，标为“要你决定”的项目不会
  混入执行；MCP 导入页如实说明本机引擎转存原配置字面量密钥的边界，不再同时声称“拿不到明文”。
- 运行配置逐项回读与结构化审查：实际配置页不再展示内部 JSON，改为按 Provider 与运行设置分组；
  运行设置逐项并列软件保存值和客户端实际值，并把非自动化字段的漂移同样显示在默认环境摘要与
  “自动化与权限”页。Claude 状态回读补齐内置/自定义状态栏及隐藏署名开关；写入成功只有在目标文件
  回读为 ready、Provider 和适配器全部运行字段均一致后才提示完成。按项编辑页移除重复的底层键路径，
  真实键值统一在“写入预览”集中审查；窄窗口对照改为纵向字段卡，避免三列表格挤压。Vitest 全量
  58 files / 546 tests、前端类型检查与生产构建、Rust engine 390 tests、Tauri 16 tests 均通过；浏览器
  mock 实见桌面与 320px 窄窗布局，控制台无错误或告警。本批未启动原生应用，也未读取或写入真实
  Claude/Codex 配置。
- 首次使用入口收口：全新安装继续直接进入 Provider 列表，“导入现有配置”由一级侧栏、命令面板与
  托盘提供直达；删除已经没有调用方的旧三步长文案组件，保留测试防止它重新挡在日常工作台之前。

## 剩余路线图（未排期）

各项均无排期、无指定负责人。

| 事项 | 前置 | 产出 | 完成标准 |
| --- | --- | --- | --- |
| 当前代码安装包与 GUI E2E | 参与域全部完成（已达） | 验收基线提交 `5a110be3` 的 x86_64 DMG 与主要真实 GUI E2E 已验收（2026-09-15，旧版交互）；本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 已验收 | 最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见上；MCP 新增服务表单待 macOS 自动化输入条件具备后补验 |
| 发布 | GitHub billing 解决；updater/Apple/Windows 签名材料在仓库外与 Secrets 配齐（GitHub Actions secrets/variables 与本机 codesigning identities 当前均为 0） | 签名、公证、arm64、Linux、Windows、更新发布（发布管线 `14e6790` 草稿先行单写者编排已就绪，取代 `a2cc864`） | GUI CI 对当前产品代码基线真实跑绿；各平台真机安装/运行与 updater 真实签名下载安装回读独立验收通过（Release/打 tag 未授权，到达时报主审决定） |

2026-09-15 验收基线提交 `5a110be3` 的 x86_64 DMG 已通过 verify 与主要真实 GUI E2E 主审独立验收（旧版 plan→confirm 交互），arm64 DMG 交叉构建静态验收通过。本轮直接 apply 新交互的源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；旧 DMG/旧 GUI E2E 不证明本轮新交互，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见上。公开发布仍受外部条件阻塞：GitHub billing、真实密钥、Developer ID/公证、Windows 签名、Linux/Windows/arm64 真机运行与安装、updater 真实签名下载安装回读均未就绪。

## 高级管理入口裁决记录（2026-09-12）

- Tools：退役。真实条目仅剩 playwright-cli（两条 npm 命令），实施为全新大域且投入产比低；退役后由用户手动管理。已于 `9080c1d` 清理完毕。
- Aliases：保留实施，为恢复后第一个最小闭环。已于 `b7f30db` 落地并通过主审独立验收。
- 更新中心：`system:refresh-upstream` 已实施（参与域 agents/mcp/skills，探测缓存落 SQLite `upstream_probes` 表，三域 status 只读回填 freshness，`--offline` 拒绝，不取 apply 锁）；`system:converge` 已于 `9080c1d` 退役清理；Updates 页直接触发真实 apply（apply 内部保留 preflight/写前校验），前端调用已补显式 `--action`（agents→upgrade，mcp/skills→update）。
- 快照：退役。新体系既定方向为 fail-closed 拒绝覆盖 + 操作内事务回滚，不恢复"写前持久备份"模型；"自动备份"承诺文案已于 `9080c1d` 清理。
- 探测缓存：落 SQLite 新表（易失状态入库与 operation_logs 先例一致），不引入第二份可写状态文件。
- 实施者：当时决定全部使用 Kimi YOLO 模式；2026-09-28 因 Kimi 无实施权限，当前协作规则已在 [AGENTS.md](../AGENTS.md) 更新。

## 2026-09-15 公共 plan/dry-run 退役裁决

- 公共 `*:plan`（`mcp:plan`、`rules:plan`、`hooks:plan`、`aliases:plan` 等）与 `--dry-run` 入口完全退役，请求一律被拒绝（plan 动词 UNKNOWN_VERB、`--dry-run` USAGE_ERROR），不再作为入口。
- status、query、诊断等只读路径保持零写；六域写操作仅通过真实 apply 完成。
- apply 内部保留 preflight/写前校验、精确 scope、排他锁、外部修改检测、CAS/条件回滚与操作审计。
- 定义保存仍只保存，不执行安装/链接/同步；破坏性定义移除、清空日志等高风险动作保持独立二次确认。
- MCP secrets 边界保留：SQLite 字面量 `${VAR}`；status/诊断/操作日志不暴露解析值；apply-time 可能按目标 codec 写入下游配置。

## 2026-09-22 助手配置零标记裁决

- 触发：Codex 对 MCP 远程条目中的 `_MANAGED_BY` 报告无法识别的配置；盘点发现 MCP 三个 codec 都写该标记（stdio 写进 `env`/`environment`，会传给 MCP 进程），Codex MCP 条目与 `[model_providers.*]` 段前还写 `# [agent-manager:managed]` 注释，状态栏脚本在 Claude 配置目录写 `.agent-manager-usage.json`。
- 裁决：以 `docs/GUI-REDESIGN.md`「助手配置零标记」为准。归属只记在 `native-manager.db`，与 rules、hooks、Claude 运行配置已采用的「定位 + 摘要」做法统一；本节取代上文远程 HTTP MCP 接线中「受管 marker 迁至条目级 `_MANAGED_BY`」的要求。
- MCP：归属 = `mcp_bindings` 已分配且启用；旧标记只读剥离；无记录的同名条目（含带旧标记的残留）按用户条目处理，需显式接管。
- Codex 运行配置：`agent_settings_bindings.owned_providers` 记录本工具写入的 provider id；旧注释仅用于一次性迁移识别。
- 状态栏额度缓存迁至数据目录 `usage/`，不再读写助手配置目录内的缓存文件。
- Shell 别名区块注释保留（rc 文件不是助手配置，无结构化字段可用）。Codex 实例的 `agent-manager.config.toml` 是 Codex 官方 profile 文件形式，保留。

## 2026-09-22 实际配置为准裁决

- 触发：用户指出除内置默认（需确认后应用）外，所有配置都应直接读取用户真实环境。盘点确认现状相反：内置 policy 首次写库即固化；规则、hooks、别名、MCP 卡片、运行参数表单展示库内副本；外部修改的唯一出路是「接管 = 用库覆盖」；内置别名、技能仓库、hooks 被计为待同步。
- 参考：CC Switch（`farion1231/cc-switch@8e478b2`）首启把实际配置导入为当前项、官方预设只追加不激活（`src-tauri/src/lib.rs:728-782`），切换前回填实际配置（`services/provider/mod.rs:5836-5884`），提示词读取以实际文件刷新已启用项（`services/prompt.rs:62-85`）。不照搬其整份覆盖 settings.json、MCP 导入后以库为准、读取时写库三点。
- 裁决：采用 `GUI-REDESIGN.md`「数据模型：实际配置为准」三层模型；规则取消 shared + adapter 拼接，改为每个助手一个实际文件，另提供「复制到另一个助手」；Codex MCP 停用使用官方 `mcp_servers.<id>.enabled = false`。
- 实施阶段与回执（均已推送 `rust`）：契约 `1ce4a15` → 技能内置仓库改推荐 `1e60a7b` → 接入与运行参数（实际值预填、切换前回填、令牌只经外部密钥文件）`1acc84a` → 规则（实际文件、模板、回填，取消拼接）`e89abc5` → MCP（实际条目为主、官方字段停用）`e318d8d` → Hooks（实际读写、清理建议）`94037ef` → 别名（实际区块、推荐项）`b2eb229` → 实例（生成文件以实际为准、覆盖或采纳二选一）`4268bad`。
- 验收方式：每阶段两套 cargo test、前端单测、typecheck、build 全绿；关键修复做反向验证；隔离 HOME 用编译出的 CLI 实测回读（含用改动前二进制造旧库再升级）；接入、MCP、实例另在 DEV mock 下浏览器验收；Codex 停用由真实 `codex mcp list` 回读。安装包真机验收见 2026-09-23 交付回执。

## 2026-09-23 运行设置与配置原文裁决

- 触发：用户真机试用时问运行设置能否扩展更多参数；随后指出「实际配置」页与整文件编辑把所有值都摆出来，应像 CC Switch 那样区分对用户有意义、需要调整和展示的部分；并要求编辑框直接显示明文。
- 参考：CC Switch（`farion1231/cc-switch@8e478b2`）主界面只有供应商卡片（名称、地址、额度），配置细节都在编辑面板里按重要性排：API Key、地址在前，「高级选项」默认折叠，配置 JSON 放最后且默认 3 行高，上方 6 个快捷开关直接改这段 JSON（`src/components/providers/forms/CommonConfigEditor.tsx`）；编辑当前供应商时 JSON 是整个 live `settings.json`（`EditProviderDialog.tsx` 读 `read_live_provider_settings`），令牌为明文。不照搬其整文件覆盖与通用配置片段合并。
- 裁决：推理强度与 Artifact 开关写 Claude Code 官方持久键（`effortLevel`、`enableArtifact`），不写单次会话覆盖变量，旧变量由用户点「清理覆盖变量」撤掉；助手页去掉「实际配置」页签，只留接入 Provider / 运行设置 / 启动命令；运行设置表单直接在页签里；表单外的设置在运行设置底部「直接编辑配置文件」里改全文，默认收起，显示与写回都是原文，整文件摘要防覆盖，库与审计不存全文与取值。凭据名判断参照 CC Switch 按整类匹配。
- 提交：官方持久键 `1b5ed7a` → 原文编辑命令与界面 `05cf21d` → 凭据名整类判断 `d643243` → 原文显示明文 `503839c` → 页签收敛与运行设置内嵌 `dba3f27` → 执行锁等过子进程短暂持有窗口 `f7060ba` → 禁止自动更新开关 `82e8170` → CodeMirror 编辑器（与本条同一提交）。
- 执行锁：同一进程里别的线程启动子进程时，子进程在 exec 前握着锁文件描述符的副本，刚释放的锁被短暂占住，紧接着的写动作被误判为并发。高负载下 `tests/agent_settings.rs` 40 次失败 7 次、单线程 0 次；取锁在 500ms 内重试后同样负载 0/40。
- 验收状态：各提交两套 cargo test、前端单测、typecheck、build 全绿；原文编辑与禁止自动更新另在隔离 HOME 用 CLI 实测回读；编辑器在 DEV mock 浏览器里实见高亮、行号与行内报错；页签收敛后的安装包已用真实环境启动供用户试用，真机逐项验收待做。

## 2026-09-23 规则内容迁出裁决

- 触发：用户指出默认规则跟着二进制走，改一条就得重新发包。盘点发现「内置推荐规则」`core.md` 就是作者本人的全局规则（改过 32 次），属于个人内容，不是产品默认；而接入预设、hooks 清理签名、状态栏脚本等才是跟代码走的工具知识。
- 裁决：规则内容迁到 agent-skills 仓库 `rules/`（subtree 保留历史，另补体积与锚点门禁和行为评测执行器），本工具作为技能来源读取，复用技能的集合定义、镜像目录与「更新资源库」；二进制不留兜底规则，只留旧库迁移用的冻结副本。推荐别名（固定时区与语言）同属个人内容，同样迁到 agent-skills `aliases/`，由别名页从来源读取。
- 提交：agent-skills `001ae92`（迁入规则历史）、`948aaa0`（整理与门禁）、`50a8cde`（迁入别名）；本仓 `7c6eee7`（来源规则）→ `2b0f9ac`（删除 `agent-rule/`）→ `0bba6fe`（推荐来源生成挪到技能模块）→ `329e2b1`（来源别名）→ `107c30a`（删除 `config/shell-aliases.json`）→ `37c6933`（Tauri 桥放行 `--source-rule`）。均已推送。
- 验收中发现并已修复：`7c6eee7` 起桥的命令白名单没放行 `rules:apply --source-rule`，桌面端点「应用来源规则」只显示「写入失败」；引擎、前端测试都不经过桥，是对安装包逐步点击时暴露的（`37c6933` 修复并补桥测试）。`329e2b1` 提交前漏跑 Tauri 测试，桥直连的别名状态测试仍按内置推荐断言，一并修正。规则页「更新来源」按钮起初用错技能 id 前缀（`skills:` 应为 `skill:`），单元测试与代码错在同一处，隔离 CLI 实测抓出后修正。
- 验收：两套 cargo test（引擎 387、Tauri 20）、前端 558 项、typecheck、build 全绿；agent-skills 39 项测试全绿，门禁各做一次反例。隔离 HOME 用 CLI 以 agent-skills 临时克隆为上游，规则走通「推荐来源 → 登记 → 取回 → 应用（先备份）→ 上游出新版 → 可更新 → 一键更新不另存备份」，别名走通「推荐来源 → 取回 → 推荐带来源 → 添加 → 两个 rc 文件写入、`zsh -n` 通过」，审计记规则 id、摘要、上游提交号或别名来源，不含正文。`37c6933` 安装包 `verify:macos` 全部通过；从 DMG 取出的 .app 在隔离 HOME、隔离数据目录、替身 CLI 下经 System Events 逐步点击：规则页「取回 agent-skills」→ 出现来源规则 →「应用到…」写入且与来源 core.md 逐字节一致 → 上游提交新版后「更新来源」→「来源有新版本」→「更新到新版本」写入新版、无备份模板；终端快捷命令页推荐标「来自 agent-skills」，添加 grok 后两个 rc 文件出现区块、`zsh -n` 通过、审计记来源；运行设置页有「禁止自动更新」，展开「直接编辑配置文件」为 CodeMirror 编辑器（原文含令牌、格式化与查看改动在）。全程真实 `~/.claude`、`~/.codex`、rc 文件摘要不变。真实环境里 agent-skills 镜像尚未取回，由用户在规则页点「取回 agent-skills」。

## 2026-09-23 agent-skills 目录整理裁决

- 触发：用户指出 agent-skills 目录乱：七个技能与 `rules/`、`aliases/`、`tests/` 平铺在顶层，`tests/` 里混着调用模型的评测执行器，33KB 的 README 同时装着目录、实测记录、读数规则与投稿约定。
- 裁决：agent-skills 把技能收进 `skills/`，调用模型的执行器与内置技能夹具挪到 `tools/evals/`，`tests/` 只留离线门禁；README 原文拆到 `docs/authoring.md`（设计原则、技能包结构、运行时适配、投稿约定）与 `docs/evaluation.md`（执行器、读数规则、宿主截断、实测记录、已试死路），README 只留范围、目录、结构、验证与用法。本工具：推荐定义改为 `skill_paths: ["skills"]`；旧定义里的单段技能名在顶层找不到时按 `skills/<名>` 找（只读，不改库，旧定义与旧库备份照常可用）；接管扫描按集合实际发现的技能名判定计划外链接，否则目录定义下同名链接会被当成外部链接另建集合；推荐卡片与确认框的「含 N 个技能」改为「技能取自 skills/」，因为一个 `skill_paths` 项可以装多个技能。
- 提交：agent-skills `fb35311`（执行器与文档拆分）、`8566c8e`（技能收进 `skills/`）；本仓 `30a624e`。均已推送。
- 验收：引擎全量 cargo test、Tauri 20 项、前端 558 项、build 全绿，新增用例在撤掉回退或名称匹配改动后各自失败；agent-skills 39 项全绿，新增「顶层不得出现技能包」门禁、英文检查覆盖 `docs/`，各做一次反例；四个执行器从任意目录都能找到 7 个技能、196 条触发用例与规则评测文件（只加载，未调用模型）。隔离 HOME 以本地裸仓库为上游：旧布局取回并启用 `verification` → 上游前进到整理后的提交再更新 → 旧定义仍发现 7 个技能、链接目标不变、来源规则与推荐别名照常 → 定义改为 `["skills"]` 后结果一致。
- 真实环境：先备份库（`before-reorg`），经 CLI 执行与「更新资源库」相同的集合更新（镜像 `cbf1fd2` → `8566c8e`），再把集合定义改为 `["skills"]`（url、agents 不变）。前后 `skills:status` 逐行一致，`~/.claude/skills`、`~/.codex/skills` 共 24 条链接目标一致，各配置文件摘要一致；规则页两份规则文件匹配来源规则 `agent-skills/core`；四条推荐别名已在两个 rc 文件里，故不再推荐。CLI 更新镜像不刷新上游探测缓存，集合卡片仍显示 3 小时前探到的 `cbf1fd2`，另跑一次 `system:refresh-upstream --only skill:agent-skills`（只写探测缓存）后显示 `8566c8e`、已是最新。`30a624e` 安装包 SHA256 `d1cb0ae5d782ec47fb2da328ca0e9c1b314aa9542a317cba20e4b51259dba74b`（8689586 bytes），`verify:macos` 全部通过（未签名、只验 x86_64）；换成该版本后技能页推荐卡片显示「技能取自 skills/ · 已添加」，集合卡片「正常 · 已收录 7 个技能」，无发现失败。遗留：库里仍有已并入 `verification` 的 `frontend-verification` 绑定（两个助手均 missing-link），整理前即存在，未处理。

## 2026-09-23 官方账号与第三方凭据隔离

- 触发：用户要求官方账号体系的凭证与第三方凭证完全隔离、不掺杂使用，不触碰官方账号，一切以各助手官方标准为准。实测（`claude auth status` 注入假凭据）发现同一配置目录里两种凭据并存时，Claude 在「只有地址没有令牌」或「API key 未确认」时会退回官方登录，同时请求照样发往第三方地址；调研 CC Switch、claude-code-router、ccs、ccm、opencode、Cline 后确认：做得稳的都不碰官方登录，按会话或配置目录隔离，并清理抢优先级的键。
- 裁决：任一配置目录只放一种凭据。主配置（`~/.claude`、`~/.codex`）只放官方账号，本工具不再写入第三方接入（`agents:configure --use` 以 `AGENT_MAIN_OFFICIAL_ONLY` 拒绝），第三方接入一律在独立环境（实例）里用；从不读写钥匙串、`.credentials.json`、`auth.json` 或 `~/.claude.json` 的账号字段。
- 提交（均已推送 `rust`）：`f4c4cbc` 外部命令执行器可取退出码与 stderr（重构）；`812a81c` 第三方实例写入前用官方命令确认目录无官方登录、Claude 定义规则、置空取消 shell 继承、Codex 启动前缀清空 `CODEX_API_KEY`/`CODEX_ACCESS_TOKEN`；`f918db6` 接入一键建成独立环境、实例表单支持 x-api-key 且保留其他 env；`192121b` 主配置只放官方账号（界面、命令面板、托盘同步，⌘⇧K 改名账号快切）；`c75c993` 清理切到指定接入的死路径（重构）；`7dbd73b` 补正 `c75c993` 漏改的用例；`2808579` 登录状态改用官方命令按需查询（`agents:auth-status`），status 不再读认证文件，第三方实例里的官方登录标为混用；`9a3af04` 旧 Codex 第三方启动命令缺前缀时提示与订正；`1578b76` 提示会压过主配置官方账号的外部设置（只读）。
- 依据：Claude 官方文档（认证优先级、网关接入、settings `env` 置空取消 shell 变量、钥匙串按 `CLAUDE_CONFIG_DIR` 区分、`~/.claude.json` 保存登录会话）；Codex `rust-v0.156.0` 源码（`env_key` 优先且缺失即报错不退回登录、`requires_openai_auth` 只在无 `env_key` 时走官方身份路径、keyring 按 `CODEX_HOME` 摘要区分、`CODEX_API_KEY`/`CODEX_ACCESS_TOKEN` 优先于存储的登录）与官方配置参考。Codex 第三方表的 `requires_openai_auth` 渲染保留，只用于识别旧版本写进主配置的表。
- 验收：每次推送前 engine-rs 全量、Tauri 20 项、Vitest 与 build 全绿（最终 57 files / 572 tests）。隔离 HOME 加真实 claude 2.1.281 / codex 0.156.0（Claude 以 `CLAUDE_CONFIG_DIR` 指向隔离目录，钥匙串按路径隔离）：第三方实例新目录写入、已存在目录经真实 `auth status` 探测后写入、出现登录会话记录后拒绝；真实 Claude 确认实例里的空串挡住 shell 导出的 `CLAUDE_CODE_OAUTH_TOKEN`、`ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN`；真实 `codex login status` 在环境带 `CODEX_API_KEY` 时仍报未登录；主配置拒绝写入第三方接入且文件不变，切回官方把令牌转存密钥文件、主配置清空、数据目录无明文。本机真实 HOME 只读运行 `agents:auth-status`：Claude 主配置由此前误报的「缺失」变为已登录，Codex 已登录（chatgpt）；`credential_overrides` 两边为空；临时数据目录均保持为空。
- 过程问题：`c75c993` 推送前全量测试的输出筛选（`grep | head`）截掉了 FAILED 行，一个用例未跟着改就推送了，`7dbd73b` 补正；之后复核改为只看 `test result` 与 `FAILED`。
- 未做：桌面安装包里的点击验收（混用警告、切回官方、建成独立环境、检查登录、启动命令订正）；隔离环境里无法产生真实官方登录，「官方命令报告已登录」一支只由替身测试与真实主配置的只读查询覆盖；未抓包确认请求头；未与真实第三方中转联调。

## 2026-09-23 凭据隔离闭环审查后的修复

- 触发：闭环审查发现三处阻断：桌面应用从访达启动时 PATH 只有系统目录，Homebrew/npm 装的助手找不到，已有第三方实例的写入因「无法确认登录」被拒（复现）；第三方 Codex 实例只写 `env_key`，启动命令不导出该变量时真实 codex 报 `Missing environment variable`，状态却显示正常（复现）；保存实例只写启动命令不写配置，空目录里一运行就引导登录官方账号，之后第三方凭据写不进来。用户要求避开与版本绑定紧的做法、学习 CC Switch 等成熟产品；必须与版本挂钩时不考虑老版本。
- 裁决：Codex 实例照 CC Switch「保留官方登录」模式，把凭据写进实例 `config.toml` 的 `[model_providers.agent-manager].experimental_bearer_token`（Codex 0.48 起支持，官方说明取值作为 `Authorization: Bearer` 发出、供以程序方式配置时使用），与 Claude 实例写 `settings.json` 同一做法，库里只存变量名。不用 `model_providers.<id>.auth` 命令（0.118 起、需随包提供助手命令），也不再用 `-p` 叠加 `agent-manager.config.toml`（profile-v2，0.134 起替换 `-p`）。命令查找照 CC Switch 补一组常见安装目录，执行时把命令所在目录放到 PATH 最前。本机真实库与 rc 文件里没有独立环境或 `-p agent-manager` 命令，不做旧写法迁移。
- 提交（均已推送 `rust`）：`ef9cc86` 命令查找补 `~/.npm-global/bin`、`~/.volta/bin`、`~/.bun/bin`、`/opt/homebrew/bin`、`/usr/local/bin`，版本探测与登录检查执行时补 PATH；`27fa934` 上一提交的格式化；`d4210e9` Codex 实例改写 `config.toml`（按键原位修改，只动 `model`、`model_provider` 与本工具那张接入表，写入基准只取这三项，MCP 条目变化不算外部修改）、凭据写 `experimental_bearer_token`、启动命令去掉 `-p`、官方账号实例不写 `model_provider`、凭据变量必须和地址一起设置、Codex 接入表单可选填取值；`c41c558` 新建第三方环境目标文件不存在时先写配置再写启动命令，写不进去时启动命令也先不写；`6a4a5f6` 实例临时文件以 0600 新建、残留链接先删除；`3b4b9a0` 删除无人调用的测试辅助函数。
- 验收：每次推送前跑改动涉及的全量套件；最终提交 `3b4b9a0` 上 engine-rs 全量 401 项、Tauri 20 项、Vitest 57 files / 574 tests、build 与 `cargo fmt --check` 全绿；新增用例在撤掉对应修复后各自失败（npm 目录查找与同目录解释器、Codex 表单规则、保存顺序两例、临时文件链接）。隔离 HOME 用引擎 CLI 保存并写入第三方 Codex 实例（登录检查走真实 codex 0.156.0）：`config.toml` 权限 0600、状态 managed-ok、状态输出不含取值；按启动命令运行真实 `codex exec`，shell 里故意导出假的 `CODEX_API_KEY`/`CODEX_ACCESS_TOKEN`/`OPENAI_API_KEY`，本地抓包收到 `POST /v1/responses`，唯一凭据头为 `Authorization: Bearer <实例假令牌>`，shell 取值未出现；对照去掉清空前缀时 0.156 仍用实例令牌（前缀保留为纵深防御，并避免官方命令把 shell 变量报成登录）；实例目录没有生成 `auth.json`；真实 `~/.codex/auth.json`、`config.toml` 元数据前后一致。
- 未做：桌面安装包点击验收（Finder 启动下的命令查找、建成独立环境后的保存顺序）；未与真实第三方中转联调；新实例不自动带上主配置的 MCP、技能、规则；外部密钥文件的备份责任未在界面提示；官方账号实例未拦截 shell 里导出的第三方变量。

## 2026-09-24 按官方认证标准全面校正

- 触发：用户要求学习 CC Switch 等业界工具，把本工具做到无可挑剔，认证部分一律遵从各助手官方标准。
- 依据与方法：Claude Code 官方文档（认证优先级、设置参考 `env` 与 `apiKeyHelper`、环境变量参考、托管设置、
  WIF 参考）与 2.1.281 二进制；Codex `rust-v0.156.1` 源码（npm latest 0.156.1）；CC Switch `f8788719`、
  ccs、ccm 等源码；厂商预设能从厂商自己发布的代码读到的以代码为准（`@z_ai/coding-helper`、`MiniMax-AI/cli`、
  `MoonshotAI/kimi-code`）。每条行为结论都用真实 claude 2.1.281 / codex 0.156.1 实测：本地假接口（抓请求头、
  回 Anthropic Messages 与 Responses 流）、隔离 HOME 与配置目录。云端容器会给 root 注入自己的 Claude 登录，
  凡涉及「未登录」的实测都以无权读取该令牌的普通用户运行；引擎权限类用例同样以普通用户跑全量（root 下两例
  因目录只读限制对 root 无效而失败，普通用户全通过）。
- 实测发现并已修复的问题：
  - 独立环境没挡住 shell 继承：shell 里的 `CLAUDE_CODE_USE_BEDROCK=1` 把第三方实例改走 Bedrock，shell 的地址
    与 key 把官方账号实例发往中转，shell 的 `ANTHROPIC_CUSTOM_HEADERS` 随请求发给实例接口；官方账号 Codex 实例
    会被 shell 的 `CODEX_ACCESS_TOKEN` 压过目录登录（`f13cd7a`）。
  - 登录检查有误判：共享的 Anthropic 联合身份档案让官方命令对任何目录都报已登录，会误拒所有第三方实例写入；
    藏在第三方令牌下面的 Console 登录被当成未登录；只存 MCP 登录的 `.credentials.json` 被当成已登录。改为经
    官方 `--settings` 取消目录声明的凭据、档案目录指向不存在处，按 `apiKeySource` 识别 Console 登录（`b210305`）。
  - Codex：`wire_api = "chat"` 已是硬错误却仍被接受（`fdd294c`）；连同它共九类写法会让 Codex 所有命令起不来，
    原文编辑可以写进去；可用性检查为内置 openai 读 0.156 已不用的 `OPENAI_API_KEY`；名为 `OpenAI` 的实例接入表会被
    当成内置 openai（`ac4209b`）。
  - 官方已弃用的 `ANTHROPIC_SMALL_FAST_MODEL` 仍在写（`4c6341b`）；官方新增的 `fable` 别名切回官方账号时不撤
    （`3c61fac`）。
  - 实例 `settings.json` 的 env 令牌会进入 Bash 工具、hooks 与 MCP 子进程；官方 `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`
    实测可挡住，第三方实例默认开启（`3c61fac`）。
  - 用量：Claude `--fork-session` 把旧回复抄进新文件导致重复计数；第三方接入只在实例里用，但实例用量完全没统计
    （`cda5940`）。
  - 厂商预设：Kimi Code 会员与开放平台两套密钥混用同一变量、出处写错；智谱模型落后一代（`af8eacf`）。
- 另补：主配置「官方账号」卡片的检查登录与只复制的登录 / 退出命令、登录方式中文说法、实例换目录与移除时说明
  登录不随行、x-api-key 首次确认说明（`9dc43f1`）；外部来源提示补齐另两个云厂商开关、命名档案与联合身份、附加
  请求头、托管设置（含 `forceLoginMethod` / `forceLoginOrgUUID`）与 Codex `openai_base_url`（`b210305`）。
  契约细节见 `GUI-REDESIGN.md` 相应各节。
- 保留不改（有官方依据）：Codex 实例凭据仍写 `experimental_bearer_token`（源码注释称以程序方式配置时使用，
  0600 文件，CC Switch 同法）；`auth` 命令式凭据需随包提供取值命令，维持前次裁决不用。Claude 用户级
  `.claude.json` 的 MCP 头 `${VAR}` 实测会被展开，远程 MCP 零明文写法有效。
- 验证：每个提交推送前跑全量——最终 engine-rs 420 项（普通用户）、Tauri 20 项、Vitest 57 files / 588 tests、
  `pnpm build` 与 `cargo fmt --check` 全绿（更正：该次 Vitest 另有 1 个未处理的错误、退出码为 1，当时漏看，
  见下一节）；新增用例均在撤掉对应修复后失败。真实 CLI 端到端：引擎 CLI 写出的
  实例文件在恶意 shell 环境下，第三方 Claude 实例只把实例令牌发往自己的地址、官方账号实例零外发；第三方 Codex
  实例 `codex exec` 只带实例令牌；引擎 CLI 扫描真实 CLI 写出的会话（含分叉）计数正确、重扫不增加。浏览器 mock
  实见主配置登录检查一行与 360px 无横向溢出。
- 未做：桌面安装包重建与真机点击验收（本环境为 Linux 容器，无 macOS）；未与真实第三方中转、真实官方账号联调；
  Codex 写入后未用官方二进制再加载一次校验（只做源码确认的静态识别）；VS Code 扩展里使用实例的环境变量片段
  未提供；未发布 tag 或 Release。

## 2026-09-24（续）独立复核后的修正

- 触发：对上一节 `a4fdbad..790ea74` 的改动做独立对抗式复核（只读，逐条给出失败场景），并自查系统级设置。
  每条结论都先复现再改；复核里不成立的部分如实记下。
- 新增（`70c48f3`）：实例目录挡不住的系统级设置。真实 CLI 实测：`/etc/codex/managed_config.toml` 的
  `openai_base_url` 压过主配置、官方凭据发往它；`/etc/codex/requirements.toml` 的 `model_provider` 让第三方实例
  改走组织指定的地址；系统 `forced_login_method` 拒绝另一种登录；Claude 托管设置的 `env` 压过实例
  `settings.json` 的地址与令牌，`forceLoginMethod` 让第三方实例启动即被拦下。主配置与每个实例的状态只报来源与
  键名，卡片说明后果。
- 复核成立并已修复：
  - 建成独立环境时查实例状态失败没人接住，弹窗停住无提示（`d7b90f0`）；上一节所说 Vitest 全绿其实带着它。
  - Codex 分叉的分叉、或父会话恢复过：前缀里夹着父会话自己的标记，原先第一条就当结束，父会话用量被再算一遍；
    正写到一半的分叉被当成旧版本整份计入；截断后停在半截行漏算一条（`91343c4`，标记按 `thread_id` 归属判断）。
  - 实例状态原样带出 `ANTHROPIC_CUSTOM_HEADERS` 与带 `user:pass@` 的地址；检查登录把整份 `.claude.json`（含
    Console key）解析进内存（`8ee4f47`，改为隐藏、打码、定义不存这类地址、只记键在不在）。
  - `requires_openai_auth = false` 被误判为与 `auth` 冲突；漏了空的 `auth.command`（`9c2df8e`，真实 0.156.1 复现）。
  - 按新规则不再合法的旧定义从实例列表里消失、无法编辑或移除；实例文件自己设的隔离项被说成 shell 导出、写入
    静默置空（`169032f`）。
  - 从接入卡片建成的第三方环境没有默认隔离子进程凭据，已有环境写的 `true` 显示为关、保存被删（`f9a158a`）。
    修复时另发现：2.1.281 的 Linux 版开了这项要用 bubblewrap，没装 `bwrap` 时 Claude 拒绝启动（实测；本容器
    恰好装了 `bwrap`，上一节的子进程隔离实测因此没暴露）；下载 macOS 版核对，其代码里沙箱可用恒为否、没有这条
    报错。Linux 上因此默认不开并说明。
  - 厂商预设国内与国际两套账号共用一个密钥变量（`d4dfd4f`，MiniMax 照它的命令行、Z.ai 照它的配置助手命名）。
- 复核不成立或部分成立：检查登录会让官方命令在从没启动过 Claude 的目录里补上它自己的首次启动记录（实测，之后
  再查不变），这点成立，契约已改写得更准确；但「之后 MCP 保存被当成外部修改拒绝」不成立——MCP 按条目内容核对。
  致命写法按 0.156 起的规则判断，旧版 Codex 上可能过严，提示与拒绝说明已写明依据版本，未按已安装版本放宽。
- 另补：shell 启动文件导出了 `CLAUDE_CONFIG_DIR` / `CODEX_HOME` 时提示主配置不生效（`517dd9e`，本工具写的
  启动别名不算）。
- 另核对：全新第三方实例第一次交互启动依次是主题、安全说明、信任目录，不出现官方登录选择（实例令牌生效）。
- 验证：推送前在干净的 `9acf29e` 上跑全量——engine-rs 428 项（普通用户；`517dd9e` 上 429 项）、Tauri 20 项、Vitest 58 files /
  598 tests（无未处理错误、退出码 0）、`pnpm build`、`cargo fmt --check`；各新增用例在撤掉对应修复后失败（分叉
  归属、前缀等待、接入建成的默认值、`true` 回填、失效定义等）。真实 CLI 端到端：引擎 CLI 在隔离 HOME 与
  `/etc/codex`、`/etc/claude-code` 下报出系统级设置且不含取值、查询不在 HOME 下建任何文件；用量扫描真实会话
  （含恢复与分叉）计数不变、重扫不增加；第三方 Claude 实例在恶意 shell 下只把实例令牌发往自己的地址、官方账号
  实例零外发。
- 未做：macOS 真机上开启子进程隔离的实际运行（只静态核对了 macOS 版代码）；复制型分叉只按源码与协议注释
  构造样本，没有用 CLI 生成真实文件（`exec fork` 是引用型）；macOS 描述文件下发的 Claude / Codex 设置不检查；
  安装包未重建。

## 2026-09-24（再续）借鉴 CC Switch 补齐接入与认证管理

- 触发：用户要求重点分析助手配置管理（尤其 provider 与认证）是否完善，借鉴 CC Switch 的优点。
- 对照：CC Switch 3.20.4（`f878871`）源码里的接入管理、预设、用量与余额脚本、测速、模型列表、通用配置片段与
  MCP/技能同步；逐项判断在「实际配置为准、主配置只放官方账号、第三方只进独立环境」的前提下能不能用、怎么用。
- 采纳（均已推送 `rust`）：
  - `b07f643` 独立环境可「测一下」：`agents:check --only profiles:<实例>` 按实例定义发一次最小真实请求，只读。
  - `2068bf8` 密钥页列出接入与独立环境引用的凭据变量、用在哪里、有没有取值，改了取值要重新写入哪些环境。
  - `0de6349` 接入卡片列出由它建成的独立环境，没建成的直接给入口；添加接入可一步「保存并建成独立环境」。
  - `b9ebb7c` 接入与实例表单可获取模型列表（`agents:models`，凭据不落盘）；别名一键指向默认模型；新增子代理
    模型（官方 `CLAUDE_CODE_SUBAGENT_MODEL`）。
  - `2e89b68` 厂商预设扩到 25 条并给出密钥页与接入文档；DeepSeek 与 Kimi 开放平台可查余额
    （`agents:balance`，只用厂商文档写明的余额接口）。
  - `6e52095` 独立环境可从默认环境带上权限、状态栏、Hooks、规则与 MCP（`profiles:seed`）：只补没有的、凭据
    不带，Hooks/规则/MCP 走各自域的写入；新建表单默认勾选，卡片上可再带。
- 不采纳及原因：本地代理与自动故障转移（本工具会变成凭据中转、改写请求，官方 CLI 不再直连）；代管官方 OAuth、
  读写 `auth.json` 与钥匙串（登录只给官方命令）；库里明文存密钥；只测 TCP/HTTP 连通的「测速」（改为最小真实
  请求，能发现令牌与模型错误）；执行用户 JS 脚本查用量；深链接携带密钥导入；整文件快照切换（会覆盖外部修改）。
  CC Switch 的「通用配置片段」对应这里的 `profiles:seed`，但只复制一次、不持续合并。
- 验证：推送 `6e52095` 前在干净工作区跑全量——engine-rs 442 项（普通用户）、Tauri 20 项、Vitest 60 files /
  619 tests（退出码 0）、`pnpm build`、两套 `cargo fmt --check`；新增用例覆盖参数白名单、凭据跳过、已有不动、
  损坏文件失败但其他项照常、目录 0700、重复执行零写。隔离 HOME 用编译出的引擎 CLI 端到端：Claude 与 Codex
  实例五项都带上，主配置字节不变，实例文件 0600、目录 0700，实例与数据目录里没有主配置的凭据；真实
  `claude mcp list`、`codex mcp list` 在实例目录只列出不含明文凭据的条目，Codex 正常加载合并后的配置；
  再带一次全部 `exists`/`skipped`、实例文件字节不变。前几批的模型列表、实例检查、凭据变量使用方同样用本地
  假接口与隔离 HOME 实测过（模型列表逐个候选地址尝试，令牌不写盘）。
- 未做：macOS 真机与安装包重建；阿里云百炼、腾讯、百度等预设（厂商文档站点被本环境出口策略拦截，无法核实
  地址与模型，宁缺不猜）；Coding Plan 类套餐额度（没有找到厂商文档写明的查询接口）；多线路并发测速排序；
  VS Code 插件侧的配置；Codex 写入后调用已安装二进制做加载校验（目前按 0.156 起的规则静态自检）；技能不随
  `profiles:seed` 带（在技能页按环境分配）。

## 2026-09-24 用户反馈：默认环境看不出状态与风险

- 触发：用户截图（旧安装包）里 Codex 默认环境显示「当前实际配置（未保存）· 当前使用」和一张带「应用」的
  「助手默认账号」，问：看不出现在是什么状态、该怎么操作、有没有风险，点默认账号的「应用」认证会不会丢。
- 复现（隔离 HOME、真实 codex 0.156.1、假的 API key 登录）：配置只有 `model = "gpt-6-luna"` 与运行参数，
  引擎报 `default: false`（未保存）；执行切回后 `auth.json` 字节不变、`codex login status` 前后都是已登录，
  但 `model` 被删、库里多了一条「导入的配置」。当前代码在同样情况下还会误报「主配置里有第三方接入」。
- 修正（`136dff5`）：账号按地址与凭据判定，模型等只是官方账号上的设置（`current.official`）；已是官方账号时
  切回零写；默认环境逐行写明账号、登录保存在哪、模型与来源、自动化；接入页签分「账号」与「第三方接入 · 只在
  独立环境里用」；切回只在混用时出现，确认框逐条列出改动范围。
- 验证：engine-rs 443 项（普通用户）、Tauri 20 项、Vitest 61 files / 625 tests（退出码 0）、`pnpm build`、两套
  `cargo fmt --check`；新增引擎用例（Codex 与 Claude 官方账号带模型零写）与界面用例（截图场景、确认框内容）；
  新引擎重跑上面的复现：状态报官方账号，切回零写，`model` 保留，登录不变；浏览器 mock 截图核对两种状态。
- 未做：官方账号的默认模型还不能在界面里改（要到「运行设置」底部直接编辑配置文件）；安装包未重建。

## 2026-09-24 用户反馈：用 CC Switch 不会困惑，这里会

- 判断：CC Switch 只有一种东西（供应商卡片）、永远正好一张在用（启动时把实际配置导入成 default 卡片，另预置
  官方卡片）、一个动作（启用＝改写同一个配置目录）。这里的困惑来自结构：默认环境里摆着不能在默认环境里用的
  Provider 卡片，真正能用的独立环境在另一个页签；「接入」「独立环境」「实例」「默认账号」几个概念同时出现。
  官方账号与第三方凭据分目录隔离是用户此前的要求，所以不照搬「启用＝改写主配置」，改为每张卡片都是能直接
  启动的环境、写明终端命令。
- 修正（`9290dec`）：去掉「默认环境 / 独立环境」页签，先选助手，同页依次列出默认环境、独立环境与用来建独立
  环境的第三方接入；右上角「新建环境」（第三方厂商，或另一个官方账号）为主入口，第三方表单的主按钮是「下一步：
  建成独立环境」；独立环境卡片头部写明账号类型与终端命令。
- 验证：Vitest 61 files / 626 tests（退出码 0）、`pnpm build`、`tsc`；导航相关用例改为不再切页签，新增「新建环境」
  两条路径与卡片终端命令的用例；浏览器 mock 截图核对三节布局、菜单与第三方表单，360px 宽无横向溢出。引擎与
  Tauri 未改动（上一轮全量：engine-rs 443、Tauri 20）。
- 未做：「新建环境 → 第三方」仍是两步（先保存接入，再在预填的独立环境表单里确认命令名与带上的设置），没有合成
  一张表单；安装包未重建，桌面应用里没有点过。

## 2026-09-24 用户问：现在还会有授权凭证的疑问吗

- 走查结论（按用户会走的路径）：官方登录一侧已说清（保存在哪、本工具不读不改、按需用官方命令检查）；第三方
  API Key 一侧还有四处会卡住或让人疑惑：第一次填 Key 必须先自己建一个权限 600 的 `.env` 文件，否则提示「尚未
  选择外部密钥文件」；表单同时要「变量名」和「Key」，看不出这个变量现在有没有值；换了 Key 之后卡片只显示笼统的
  「实际配置与实例定义不同」，还给出对凭据没有意义的「以实际配置更新定义」；卡片平时不说 Key 写在哪。
- 修正（`ac894ac`）：`secrets:source --create`（0600 独占新建、已存在不动、不进数据目录、校验不过即删）；
  `secrets:status` 报文件里已有的变量名；接入表单 API Key 在前、一句话写明有没有值与写到哪，没有密钥文件时保存前
  自动新建 `~/.config/agent-manager/secrets.env`；密钥页可「新建一个」；`profiles:status` 把第三方凭据标为
  `credential`，卡片写明来自哪个变量、写在哪个文件、是否一致，换了 Key 时按钮是「写入新密钥」且直接写。
- 验证：engine-rs 445 项（普通用户）、Tauri 20 项、Vitest 61 files / 632 tests（退出码 0）、`pnpm build`、两套
  `cargo fmt --check`；新增引擎用例（新建密钥文件的权限、不覆盖、数据目录与相对路径拒绝；凭据换值后的状态与动作）
  与界面用例（自动新建顺序、有无值的说明、卡片「写入新密钥」不弹确认、密钥页新建）。隔离 HOME 端到端：新建的
  文件 0600、目录 0700；真实 claude 2.1.281 对本地假接口第一次发出第一个 Key；换 Key 后状态报凭据不一致、只给写入、
  不含取值；写入后真实 claude 发出新 Key；数据目录里没有任何 Key。
- 未做：Codex 第三方环境换 Key 的端到端只由引擎用例覆盖（同一段比较逻辑），没有跑真实 codex；安装包未重建。

## 2026-09-27 CC Switch 可吸收体验清单（持续实施）

目标限定在助手配置与 Provider 的日常使用：保留独立配置目录、官方账号与第三方凭据隔离、实际文件为准和逐目标写前比对。前面「借鉴 CC Switch 补齐接入与认证管理」记录的预设、复制、排序、导入导出、真实请求检查、模型列表、余额与初次带入设置已有源码与测试；本清单不把那些历史验证当成当前安装包验收。

| 待补闭环 | 当前代码证据 | 完成条件 |
| --- | --- | --- |
| 从日常入口使用独立环境 | `⌘⇧K` 的「账号与环境」范围集中列出官方账号、独立环境与已有启动命令；环境只定位卡片，命令只复制启动名。卡片上的「复制启动名」与「复制一条」分开；托盘逐条列出实际启动名，缺命令时打开卡片。托盘事件只打开精确入口，不切换或启动助手 | 已在隔离原生应用中创建官方独立环境，托盘点到对应命令面板项；重复点击前后实例配置、rc 文件和数据库摘要一致。`⌘⇧K` 新范围有前端交互用例，尚无本轮原生窗口操作；多条命令、缺命令及未写入的分支有局部用例；安装包与不同平台的实际菜单尚未验收 |
| 修改 Provider 后维护已建环境 | 编辑时列出来源关联的独立环境与逐字段差异，默认不勾选；仍与旧 Provider 相同的字段可随本次修改带入，已自行修改或上次未勾选的字段默认保留，后续可在同一面板逐字段选择补同步。旧环境按原地址与凭据变量推断，多重匹配拒绝自动关联。`profiles:apply-definition` 同时比对实际文件和定义摘要，锁内写文件、定义与归属，失败回滚；界面逐环境回读定义与实际字段 | 隔离原生界面已分别验本次更新和上次未勾选后的补同步，回读文件与数据库；多目标失败隔离和旧环境歧义尚无原生窗口实测。旧环境已脱离原值且缺少来源标识时仍需手工核对 |
| 维护环境之间的共用设置 | 新建时 `profiles:seed` 仍只补缺项；已建环境卡片增加 `profiles:shared-status` / `profiles:shared-apply`，按权限、状态栏、Hooks、规则和每条 MCP 比较实际文件，默认不选，来源与目标摘要同时校验，逐项写后回读。固定选项可预览两边取值，自由文本只显示数量或隐藏提示 | 隔离原生窗口继续覆盖 Hooks、规则、MCP、Codex 与多项顺序更新，检查失败隔离；已安装客户端加载 MCP 已在隔离目录验证，其他设置的运行效果和安装包仍待验收；不做后台同步 |
| 扩大接入覆盖时维持可信度 | 当前接入适配器只支持 Claude Code/Codex；预设只收有厂商来源的协议与端点，余额只查有官方接口的厂商 | 逐客户端确认原生配置格式、认证隔离和真实请求路径后再扩；每条新预设有厂商文档、可编辑字段和隔离环境验证；不能确认的能力不展示为可用 |

实施按表中顺序分批提交并推送；每批先跑定向检查，再跑受影响的完整 Rust/前端检查。界面 mock 只能证明交互，真实客户端行为在隔离 HOME、数据目录和测试助手路径下回读；当前安装包另行构建并验收。前述不采纳项仍按 2026-09-24 的安全与产品裁决处理，不因清单标题扩大成全局配置覆盖、OAuth 代管或本地代理。

入口闭环验证：前端 61 个测试文件、636 项通过，`pnpm build` 通过；Tauri 21 项和引擎完整测试通过。原生调试应用以隔离 HOME、数据目录及替身助手启动，实际创建独立环境后，`⌘K` 能定位并聚焦卡片；系统托盘显示环境与启动名，点击后打开对应命令面板项。托盘点击前后只读回读四个隔离文件摘要不变。该记录只证明入口闭环；没有运行真实 Claude/Codex，也没有重建安装包。

Provider 逐环境更新验证：引擎新增三项隔离用例，前端完整测试为 61 个文件、642 项，`pnpm build`、引擎完整测试、Tauri 21 项及两套格式检查通过。另用隔离 HOME、数据目录和替身助手启动原生调试窗口：编辑 Provider 地址后，界面列出差异，勾选一个独立环境并保存，界面回报该环境已更新并回读；再直接读取隔离实例文件、数据库定义与 `profiles:status`，地址一致且状态为 `managed-ok`，隔离主配置目录未生成，数据库不含测试凭据取值。没有运行真实 Claude/Codex，也没有重建安装包；默认不勾选、多目标失败隔离和旧环境歧义目前只有局部用例，尚无原生窗口实测。

Provider 补同步验证：前端完整测试为 61 个文件、644 项，`pnpm build` 通过。隔离原生调试窗口预置 Provider 新地址、已建环境旧地址，再次打开编辑面板时不改 Provider 本身；旧地址差异默认不勾选，显式勾选该字段后更新成功。直接回读独立环境文件、数据库定义与 `profiles:status` 均为新地址且状态 `managed-ok`；隔离主配置未生成，数据库不含测试凭据取值。没有运行真实 Claude/Codex 或重建安装包。

Provider 多环境失败隔离补验（2026-09-28）：隔离 HOME、数据目录和替身 Claude CLI 中建立两个受管环境，先用 `profiles:preview` 确认两个目标的新地址预览均零写；随后在预览后外部改动第一个环境的受管 API 地址。按同一批写入基准逐个执行 `profiles:apply-definition`，第一个明确拒绝旧基准、保留外部地址，第二个成功写入新地址与模型并回读为 `managed-ok`；主环境 `settings.json` 的 SHA256 不变。前端新增回归用例，确认首个写入失败后继续更新第二个并报告「1 个未确认」，且旧环境同时匹配两个 Provider 时禁用自动关联。前端全量 652 项与构建通过。原生调试窗口可被系统列出，但辅助功能持续拒绝读取（权限查询显示已授权），因此本项仍缺原生界面操作证据；未运行真实 Claude/Codex，也未重建安装包。

共用设置更新验证：隔离 Rust 用例覆盖 Claude 五类资源中的权限、状态栏、Hooks、规则、MCP 同名覆盖及含明文凭据 MCP 拦截，并覆盖来源/目标旧摘要拒绝；Codex 用例覆盖审批、沙箱与状态栏更新时保留实例接入表和模拟登录文件。前端完整测试为 61 个文件、646 项，`pnpm build`、引擎全量测试、Tauri 21 项和两套格式检查通过。隔离原生调试窗口中，已有 Claude 环境的权限与状态栏均显示差异且默认不勾选；只勾权限后，界面回读一致，文件中权限与来源一致、状态栏仍是环境自己的值，主配置和模拟登录文件摘要不变。预览后外部改变来源状态栏，原生窗口更新被拒，环境状态栏仍是原值。原生窗口尚未逐项操作 Hooks、规则、MCP 和 Codex；没有运行真实 Claude/Codex，也没有重建安装包。

共用设置取值预览增量：`profiles:shared-status` 对审批、沙箱等固定选项显示来源与目标取值；自由文本只显示数量或隐藏提示，不把命令、规则正文、MCP 参数与凭据送到界面。隔离 CLI 在两个独立环境逐项更新 Claude 的 Hooks、规则、MCP 和 Codex 的权限、状态栏、规则，六项写后状态均为 `same`，七个主配置与模拟登录文件摘要不变；前端定向用例验证多项顺序执行时单项失败不阻止后续更新，并回读两项状态。本轮原生调试窗口已启动，但计算机辅助功能读取被系统拒绝，未取得新界面操作证据；真实客户端加载和安装包仍未验收。

账号与环境快捷入口增量：`⌘⇧K` 将官方账号、独立环境及其已存在的启动命令放在同一范围；前端定向用例覆盖搜索、复制启动名、定位卡片、未写入或缺命令时不误示可启动，并断言这些动作不调用配置写入。前端全量 59 个文件、631 项和构建通过；本轮未取得原生窗口操作证据，真实 Claude/Codex 配置未参与验证。

Codex 独立环境换 Key 验收（2026-09-27）：用当前源码编译的 Rust CLI、隔离 HOME/数据目录、真实 `codex-cli 0.156.0` 和仅监听 `127.0.0.1` 的测试接口执行。第一次写入后，Codex 向 `/v1/responses` 发出使用测试凭据 A 的请求；在外部密钥文件改为 B 后，`profiles:status` 报 `drifted`，显式重写并回读实例 `config.toml` 后，Codex 再次请求时使用 B，文件不再含 A。测试服务只记录凭据类别；隔离数据库不含 A/B 取值，隔离默认 `config.toml` 与 `auth.json` 均未生成。同一流程再次运行时，宿主 Claude/Codex 三个配置及认证文件的前后摘要一致。此项只证明本机隔离的换 Key 请求路径；未验收原生窗口、安装包、真实厂商接口或完整对话响应。

共用设置真实客户端加载验证：在一次性隔离 HOME、应用数据目录和模拟登录文件中建立 Claude/Codex 独立环境，将主环境的权限、状态栏、Hooks、规则及各一条 MCP 逐项复制到目标。`profiles:shared-status` 写后回读均为 `same`；隔离主配置和模拟登录文件的七份摘要不变。随后以隔离配置目录运行本机已安装的 Claude Code 2.1.283 和 Codex 0.156.0，分别执行 `claude mcp list` 与 `codex mcp list`，两者退出码均为 0，输出均包含复制后的测试 MCP；运行前后真实主配置和认证文件摘要不变。此项只证明两款客户端可从独立目录加载 MCP，未验证权限、状态栏、Hooks、规则的运行效果、模型请求、原生窗口或安装包。

运行设置完整文件即时预览（2026-09-28）：`agents:preview-runtime` 只读重读目标文件，并与 `agents:configure` 共用运行参数渲染函数；输入变化后先显示改动键，随后显示当前实际文件合并后的完整 JSON/TOML，旧请求结果不会盖掉新输入。隔离 Rust 用例确认 Claude/Codex 的预览与随后写入逐字节一致、Codex 注释与用户字段保留、旧基准拒绝且文件不变、缺失文件预览不创建文件或数据库。前端全量 650 项、引擎和 Tauri 全套测试、构建与格式检查通过；DEV mock 浏览器回读到 Claude/Codex 完整文件。该阶段浏览器 mock 只证明界面交互，尚未在当前安装包或原生窗口验收；未读取或写入真实 Claude/Codex 配置。

原生窗口补验（2026-09-28，当前源码调试构建）：以隔离 HOME、`AGENT_MANAGER_DATA_DIR` 和只回版本号的替身 `claude`/`codex` 启动 Tauri；在 Claude 运行设置选择「智能自动」，窗口自动显示包含原有 `theme`、`env` 与新增 `permissions.defaultMode` 的完整 `settings.json` 候选；在 Codex 选择同项，窗口自动显示保留原注释、模型和 `history` 的完整 `config.toml` 候选，并列出 `approval_policy`、`sandbox_mode` 与 `approvals_reviewer`。两份目标文件预览前后 SHA256 相同，隔离数据目录为空。在「启动命令」新增表单切换 Codex「智能自动」，alias 预览即时出现 `--approve-for-me`；取消后隔离 `.zshrc`、`.zprofile` 均未创建。仅验当前源码的调试构建与这些交互；安装包仍待单独验收，未操作真实 Claude/Codex 配置。

## 交付回执（2026-09-23，实际配置为准 x86_64 DMG 真机验收）

绑定提交：`c0065c090e2ed89e6adedac60bc025a159baefda`（产品代码；其后 `0ceb7e0` 只改验收脚本）。

- 安装包：本机重建 clean x86_64 DMG，SHA256 `56cd1ab2a10da097d6a321215948fbbbb1150e00be4b3cf6218af5554de3358d`，8514164 bytes；`verify:macos` 全部通过（元数据、构建身份 commit/date 精确一致、无 Python、路径脱敏、隔离环境 5 秒存活并优雅退出）；未签名、未公证。
- 验收环境：从 DMG 复制出的 .app 直接以隔离 HOME、隔离 `AGENT_MANAGER_DATA_DIR`、只含替身 `claude`/`codex`（只回版本号）的 PATH 启动；经 macOS 辅助功能（System Events/JXA）点击原生窗口、窗口区域截图，文件与库逐项回读。全程不运行真实 Claude/Codex、不点登录/退出/可用性检查/安装升级。
- 通过的场景：启动与各页 status 零写（数据目录为空、HOME 与夹具逐字节一致）；总览不把内置项计入待同步；技能页显示真实技能目录、内置仓库只作推荐；MCP 直接列出手写条目，GUI 停用 Codex 条目只加 `enabled = false`（注释、`cwd`、`enabled_tools`、用户 provider 表保留），Claude 条目含明文令牌且未选密钥文件时拒绝停用并提示；规则页显示实际文件，外部抢先写入后「写入」被拒且保留外部内容，重开后写入内置推荐并先把外部内容另存为备份模板；Hooks 显示 `prompt` 与 `command` 两类且不阻塞；终端快捷命令内置项只作推荐，添加后两个 rc 文件出现区块、区块外用户 alias 保留、`zsh -n` 通过；助手切到默认账号时先弹「令牌未保存」，显式放弃后只撤掉 `ANTHROPIC_*` 三键（hooks、permissions、theme 保留），原实际配置存为导入项且无令牌；独立环境卡片逐字段列出差异，「用定义覆盖」经确认后合并写入（用户 env 与 theme 保留），外部改模型后「以实际配置更新定义」只改库、文件不动。
- 验收中发现并已修复：Claude MCP 停用快照仅按取值格式判定凭据，`GITHUB_TOKEN` 等无厂商前缀的令牌会以明文进库（`c0065c0` 改为变量名或取值像凭据即转存，回归用例覆盖）；验收脚本的构建身份整行匹配依赖字符串布局，会误报（`0ceb7e0` 改为按标记格式提取比对，反例保留）。
- 真实环境核对：验收前后 `~/.claude/settings.json`、`~/.claude/CLAUDE.md`、`~/.claude.json` 的 `mcpServers`、`~/.codex/config.toml`/`AGENTS.md`/`hooks.json`、`~/.zshrc`/`~/.zprofile` 摘要一致，`~/.codex/auth.json` 只比元数据（未读内容）一致，应用 Application Support 目录一致。例外：WebKit 按真实主目录存放本应用 webview 数据，`~/Library/WebKit/io.github.ly87ing.agent-manager` 的访问统计与 localStorage WAL/共享内存文件被触碰（`localstorage.sqlite3` 主文件未变、WAL 为 0 字节），与 Claude/Codex 账号和配置无关。
- 局限：替身 CLI 位于非标准路径，两个助手在总览显示「要你决定」（安装来源 system），属验收环境现象；独立环境定义经 CLI 写入隔离库后在 GUI 操作，新建表单的键盘输入未走真机；未签名包；只验 x86_64。

## 本次停止点与恢复入口

- 停止点：验收基线提交 `5a110be3` 的旧版 x86_64 DMG 与 GUI E2E、arm64 静态验收均已完成；直接 apply 改造已完成源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收，最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify，交付回执见上；本轮三提交（`d46b85a`/`47702ed`/`14e6790`）完成对外发布就绪的仓库内改造（LICENSE/元数据/CSP/图标/错误边界/自定义助手确认/MCP 空态/管线草稿先行单写者重构），发布管线（`14e6790`）与 Windows 阻断 CI（`9f56f00`）已就绪，公开发布仍受外部条件阻塞，未创建 tag 或 Release。
- 恢复入口（按序）：解决 GitHub billing；在仓库外/Secrets 配齐 updater/Apple/Windows 签名材料；让 GUI CI 对当前产品代码基线真实跑绿；Linux/Windows/arm64 各平台真机安装与运行独立验收；MCP 新增服务表单在 macOS 自动化输入条件具备时补验；由用户单独授权后打 tag（含 `-` 后缀时产出 prerelease draft），release workflow 创建 draft Release、上传四平台产物与 checksums 并单写 latest.json；逐平台签名/公证/updater 真实下载安装回读独立验收通过后才公开。

## 验证

`cargo fmt --manifest-path engine-rs/Cargo.toml -- --check`、两套 Cargo test/check，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。
