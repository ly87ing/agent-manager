# GUI 重设计：当前契约

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置写入 `app_settings` 表，定义和本机 policy 写入 SQLite。内置默认值由 Rust 编译提供，只读；policy 写入固定使用 `--layer local`，界面不提供配置原文编辑器。

查询与 status 必须只读零写；缺库读取不能创建数据库。表单保存只保存定义，不执行链接或刷新。资源执行按钮直接触发真实 `apply`；`apply` 内部先做 preflight/写前校验，再按精确 scope、排他锁、外部修改检测、CAS/条件回滚执行并审计；普通同步、启停、更新不二次确认；定义移除、清日志等破坏性动作仍独立确认。

Skill、MCP 等资源及其逐助手分配要求新增、编辑/更新、启用、禁用、删除。AI CLI 定义是例外：普通界面只维护接入、运行参数和账号实例；安装、升级、卸载及完整定义编辑不再作为产品入口，软件生命周期交给原安装渠道。既有生命周期字段与引擎能力第一阶段仅作兼容保留，不代表当前产品承诺。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

## 接管中心（system:discover）

「接管中心」页一次列出本机已存在、但还没纳入管理的助手 CLI、技能与 MCP 条目，按来源分类后批量接管。分类直接决定可做什么：

| 来源 | 判定 | 处理方式 |
| --- | --- | --- |
| 助手官方技能目录内的真实目录 | 非符号链接（Claude `synced/` 等） | 只标识，继续走其原生安装方案；不接管、不覆盖 |
| 外部路径的符号链接 | 目标不在自管基目录、名称不属已定义集合 | 按源根聚合为本地来源集合，软链接纳管 |
| 已定义集合的计划外链接 | 名称命中集合 `skill_paths` | 走既有集合接管通道（`--allow-takeover`） |
| 自管基目录内的无绑定链接 | 目标在 `repositories`/`skill-versions` 内 | 旧体系残留，只建议清理 |
| 断链 | 目标不存在 | 只建议清理，不自动删除 |
| 已安装但无定义的助手 CLI | 已知 bin 在 PATH 或 `~/.local/bin` 解析到 | 只登记识别与配置所需定义；安装、升级、卸载继续由原渠道负责 |
| 助手配置里未受管的 MCP 条目 | 复用 `mcp:status` 同一扫描与掩码路径 | 导入库定义（密钥只以 `${VAR}` 引用入库）后写回受管标记 |

`system:discover` 只读零写零网络，不取 apply 锁；助手候选沿用 `agents:status` 同一来源分类器与有界 `--version` 探测。接管本身不是新写路径：助手登记只写定义，技能与 MCP 都走各自域既有的真实 `apply`（锁、外部修改检测、CAS/条件回滚与审计不变）。

本地来源集合（技能集合定义的 `source_path`，与 `url` 互斥）不做内容快照：助手目录下的链接直接指向登记的源目录，内容仍由外部工具维护并随其更新；停用只删链接，外部源不被删除或改写。登记源根后，指向该源根的链接才进入受管基准（可被覆盖、停用时可删除），未登记的外部路径仍 fail-closed。源目录必须是已存在的真实目录，不得是符号链接、不得落在本工具数据目录内、不得与任一助手技能目录重叠。

## MCP 原生域状态

Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；当前产品只向 Claude Code 与 Codex 暴露分配入口。内部保留的旧 codec 只用于兼容既有数据，不构成 OpenCode 产品支持承诺。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`mcp:status` 只读零写；真实 `apply` 具备排他锁、操作审计、CAS 和回滚保护；`env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude 或 Codex 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP MCP 已实现，与本地 stdio 同级：远程定义含 `url`（必填；仅允许 `https://`，或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符），与 stdio 字段互斥；headers 以引用形式写入目标（Claude 字面量 `${VAR}`、Codex `env_http_headers`），解析后明文不落盘；受管标记统一条目级 `_MANAGED_BY`；远程定义不参与上游探测，freshness 恒 not-applicable。

Claude Code 和 Codex 的内置默认渠道均为官方 `native` 命令；brew 只在 observed 检测结果中展示，不是默认。

## Rules 与来源原生域状态

`sources:get/set` 已原生接线：规则来源固定为 `rules.shared`、`rules.codex`、`rules.claude` 三项，保存使用 revision CAS 且只写 SQLite，不执行任何同步动作。

Rust dispatcher 已实现 `rules:status`（只读）与真实 `rules:apply`。固定目标为 `rules:codex_global`（`~/.codex/AGENTS.md`）与 `rules:claude_global`（`~/.claude/CLAUDE.md`），内容由共享与助手专用来源渲染；`rules:status` 只读零写；真实 `apply` 具备全局排他锁、操作审计、文件字节 CAS 与事务/文件回滚保护；目标被外部修改时 fail-closed 拒绝覆盖，不承诺备份。没有中间产物目录，来源直接渲染写入目标。Tauri bridge 参数白名单与 Rules 页面已对齐真实 schema，详情与逐目标操作均以 `--only <目标>` 精确限定。

`system:audit` 已接入 Rust dispatcher：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。只读 `secrets:status` 已接入 Rust dispatcher：仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，按「先本机已保存的取值、再回退当前进程环境」判断该变量能否取到值，输出变量名与布尔（另给 `stored` 说明取值是否在本机库里）；`secrets:set` 写入取值，`secret_values` 表以明文保存，仅 apply 渲染目标配置时使用；不扫描、定位或克隆外部密钥仓库，任何路径都不序列化或显示取值本身；畸形定义固定脱敏失败。设置页高级管理可达 Aliases 与 Secrets 两页，Aliases 原生 `aliases:status/apply` 已接线。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线 Rust dispatcher：探测宇宙为 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表，三域 status 只读回填 freshness（缺表/缺行一律 unknown，status 零写零网络），`--offline` 拒绝。

SQLite 核心、Tauri bridge、设置页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域与 `sources:get/set`、Hooks 原生域、Runs SQLite 操作记录及 Python 清理已完成。Skills 内置集合只读默认，首次编辑存入 `native-manager.db` 后本地优先；集合与 Skill×Agent 均支持独立更新、启停和移除；启用通过内容寻址快照与安全软连接落地；status 只读零写，真实 apply 具备进程锁与操作审计。任务记录基于 `operation_logs` 审计实际操作，保留期由 `log_retention_days` 控制（0 为不自动清理），执行中心提供二次确认清空。Hooks 原生 status/apply 与 hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）已接线：固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed。验收基线提交 `5a110be3` 的候选包已完成主要真实 GUI E2E（旧版 plan→confirm 交互，现已失效，不证明本轮直接 apply 新交互）；2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环；签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

旧 DMG/旧 GUI E2E 不证明本轮新交互。本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（提交、SHA256 与验证结果见 [交付回执](goal-rust-gui.md)）。

## 额度来源

助手卡片的额度只读本机文件，不联网、不调 API、不装 hook。

Claude 侧两个来源取 `checked_at` 较新的那个：`<配置目录>/.agent-manager-usage.json`
（状态栏脚本落盘的实时值，字段沿用 Claude Code 推给状态栏的 `used_percentage` 与
Unix 秒 `resets_at`）与 `~/.claude.json` 的 `cachedUsageUtilization`（Claude Code 主动查
额度时才刷新，常规对话不更新）。落盘文件由用户自愿在自己的状态栏脚本里加几行产生，
本工具不生成、不依赖、也不要求它存在；缺失或内容损坏都回退到缓存那份。
配置目录随 `CLAUDE_CONFIG_DIR` 走，实例各读各的。

Codex 侧读 `<CODEX_HOME>/sessions/**/rollout-*.jsonl` 里最后一条 `rate_limits`，
记账时刻取那条事件自带的 `timestamp`（文件 mtime 会被后续事件顶新，用它会谎报新鲜度）。

展示口径：过了重置点的窗口不报百分比（那是上一轮的残值，Claude Code 自己的状态栏
同样一到重置点就丢掉该窗口）；整份记账超 24 小时的，百分比转灰加问号。

## 用量统计

`usage:status` 只读已聚合的结果（缺库不建库），`usage:scan` 才去读会话文件并写聚合，
`usage:clear` 清空重来。数据源是两家自己的会话记录，不联网：Claude 的 `assistant` 事件
`message.usage` 是本次增量；Codex 的 `token_count` 事件里 `total_token_usage` 是**会话累计**，
必须取 `last_token_usage`，模型名不在这个事件里、要从前面的 `turn_context` 记住并随扫描状态
一起持久化（增量续扫时读不到前面的上下文）。

增量按字节偏移：JSONL 只追加，偏移一直有效；文件被截断（size < 上次偏移）就从头重来。
只处理到最后一个换行符为止——助手正在写的那一行是半截的。同一段字节内按
`message.id` 去重，跨段由偏移保证不重复读。本机实测 1.27GB / 181 个文件首扫 32 秒，
之后增量在 1 秒内。

按 UTC 小时聚合进 `usage_hourly`，界面再按本地时区分组成天：存储里不掺时区，
换个时区看同一份数据也不会错位。

费用是折算而非账单：`usage.pricing` 是美元/百万 token，内置 Anthropic 官方三个模型
（官方定价页，2026-09-20 核对，缓存写入 1.25×、缓存读取 0.1×），第三方留空由用户填。
缺任一相关单价时整桶不报，不拿 0 冒充。模型名里常有点号（`gpt-5.6-sol`），
而 policy 的键路径以点号分隔，因此单价只能整张表写回 `usage.pricing`，不能按模型名建子键。

## 助手运行参数

`agents:configure` 把助手的运行参数写进它自己的配置文件，与装卸升级的 `agents:apply`
分属两条路径（那条执行 shell 命令，这条写文件）。支持 Claude Code 与 Codex 两种形态
（按可执行名判定），目标分别是 `~/.claude/settings.json` 与定义里 `mcp_config` 指向的
`config.toml`（缺省 `~/.codex/config.toml`）。

**接入配置**：`settings.profiles` 存多套（官方、各家中转站），`settings.active` 指向
当前生效的那套，同一时刻只有一套写进目标文件。每套含地址、令牌变量名、令牌走哪个头
（`token` → `ANTHROPIC_AUTH_TOKEN` 的 Bearer，`api_key` → `ANTHROPIC_API_KEY` 的
x-api-key，中转站两种都有）、模型、小任务模型、opus/sonnet/haiku 别名与超时。
渲染时先把受管的每条路径清空再写当前这套，切换后上一套留下的变量不会残留。
`--use <名字>` 切换、`--use-default` 切回助手自带的账号与地址，切换与写入是同一条
命令：库里的 `active` 与目标文件要么一起变，要么都不变。`active` 指向不存在的配置
一律当场拒绝。

**受管粒度**按键路径而不是整个顶层键：本工具只拥有 `env.ANTHROPIC_BASE_URL`、
`env.ANTHROPIC_AUTH_TOKEN`、`env.ANTHROPIC_API_KEY`、`env.ANTHROPIC_MODEL`、
`env.ANTHROPIC_SMALL_FAST_MODEL`、`env.ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`、
`env.API_TIMEOUT_MS` 与 `permissions.defaultMode`。同一文件里 hooks 域的内容、
用户自己的 `permissions.allow/deny` 与别的 `env` 变量原样保留；外部修改检测也只比
这几条的摘要。从没写过而这几项已有内容时按外部内容对待，同样要 `--allow-takeover`。

**状态栏**是条件受管：`settings.statusline` 为 `builtin` 时部署内置脚本到
`<data_dir>/statusline.sh`（0755）并把 `statusLine` 指过去；为 `untouched`（默认）时
只撤掉我们自己指过去的那条，用户手配的原样留着。判据是 `statusLine.command` 是否指向
本工具部署的那份——指向别处的是用户的，既不清空也不算外部修改。内置脚本除了显示，
还会把助手推给状态栏的 `rate_limits` 落盘（见「额度来源」）。

**密钥**语义与实例、MCP 一致：库里只存变量名，写目标文件时解析成真值——`settings.json`
的 `env` 是字面量映射，Claude Code 不对它做变量展开，引用写进去不生效。变量取不到值时
拒绝写入而不是落一个空令牌。落地文件按 0600 写。

`permissions.defaultMode` 会改掉助手的安全默认值，因此 `--allow-yolo` 由引擎强制：
少了这个标志一律拒绝，界面在此之前弹二次确认。写入取排他锁、记操作审计，
审计只记改了哪几项与变量名，不记取值。

助手页只服务运行时任务：先选助手，再从卡片看当前接入、地址、模型和凭据变量；点「启用」才
调用 `agents:configure`，成功回读 status 后再提示完成。每套非默认接入可单独执行只读可用性
检查。实例紧随其后，用于独立账号、登录和会话。普通页面不展示 AI CLI 的安装、升级、卸载
动作、生命周期脚本或完整 alias。

AI CLI 软件生命周期不再由产品界面维护。客户端是否可用、当前版本和安装来源只在
「设置 → 客户端诊断」只读展示，不联网更新、不执行脚本；实际安装和更新交给官方安装器或系统
包管理器。第一阶段不迁移或删除数据库中既有 `providers.*.install/upgrade/uninstall` 字段，旧定义
仍可无损读回；这些字段只是兼容保留，不再提供用户编辑入口。后续物理清理必须单独完成调用、CLI、
打包和迁移审计。

「添加接入」先展示按助手形态筛过的厂商预设，也允许自定义；选中后地址、模型、凭据变量和
模型别名自动填入，用户仍可修改。提交时先保存完整定义且保留未知字段、其他接入与原生设置，
不会提前改 `settings.active`。Claude 可在同一弹窗选填凭据取值，取值通过 `secrets:set`
单独保存；Codex 只登记环境变量名，不提供明文取值框。勾选「保存后立即启用」时，定义和可选
凭据保存成功后才执行 `agents:configure --use`；后一步失败不把新接入冒充为当前项。

接入工作台中的「运行设置」使用短弹窗，只拥有 `settings.yolo` 与 Claude 的
`settings.statusline`。保存时深拷贝完整定义，只替换这两个键，`profiles`、`active`、未知设置、
定义扩展字段和兼容保留的生命周期字段原样保留；随后执行一次真实 `agents:configure`。开启
`yolo` 仍由引擎要求 `--allow-yolo` 并经独立二次确认；外部修改仍需 `--allow-takeover`，取消或
失败时已保存定义与未写入目标文件的边界必须明确说明。

接入卡片可直接编辑和删除，不再要求进入完整助手定义。编辑当前项默认勾选保存后更新，写入和
状态回读沿用切换路径；编辑非当前项默认只保存，不暗中改变当前项。内部唯一名称在编辑时固定，
避免一次编辑变成复制并遗留旧项。删除只允许非当前项，先确认再从定义中移除目标 profile；凭据
变量可能被其他接入复用，因此不随 profile 自动删除。目标 profile 中表单尚不认识的字段保留，
已知可选字段仍允许清空。

接入卡片也可直接复制。副本预填地址、模型、凭据变量名和来源 profile 的未知扩展字段，唯一名称
按 `<原名>-copy`、`<原名>-copy-2` 依次避让且不超过 64 字符；不会读取或复制凭据取值，默认只保存
而不切换当前项。用户仍可在提交前修改副本名称与所有可见参数。

接入还支持排序及无密钥分享 JSON 的导入、导出。排序和导入都只保存定义，不切换当前接入、不写
助手配置；分享内容只含连接参数与凭据变量名，不含凭据取值、认证文件或当前启用状态。导入先做
格式、助手形态、名称、URL、模型与变量名校验，再明确以“保存但不启用”提交。

认证状态只读固定的 Claude `.credentials.json` 或 Codex `auth.json`，只返回“已就绪/缺失/不可读”及
认证方式，不返回账号、令牌、文件正文或绝对路径。普通接入保存、切换、排序、导入导出及运行设置
都不得写认证文件；Codex 第三方 provider 仍写 `requires_openai_auth = true`，保留客户端对官方登录的
识别。界面的登录和退出入口只复制官方 CLI 命令，从不代用户执行；账号实例命令显式带各自的
`CLAUDE_CONFIG_DIR` 或 `CODEX_HOME`，用户须在终端核对目标实例后手动运行。

`agents:status` 还会只读解析 Claude `settings.json` 与 Codex `config.toml`，把本机实际 provider、
地址、模型、凭据是否配置及运行参数放进脱敏的 `observed.current_config`。它不返回 token/API key
正文，不自动生成数据库 profile，也不接管外部配置；符号链接、非普通文件、超限或畸形内容按固定
状态 fail-closed。界面将“本机当前配置”与“Agent Manager 已纳管选择”分开；运行设置弹窗也把
可编辑的“已纳管 JSON”和不可保存的“本机实际值”分开，避免把数据库期望值冒充客户端实际值。
接入工作台只比较本工具拥有且能安全观察的 provider 字段，并明确显示“一致”“不一致”或“无法
比较”；不一致时由用户显式点击“重新应用已纳管选择”。修复继续复用 `agents:configure` 的外部
修改接管确认、锁、审计和成功后状态回读，不后台自动覆盖客户端配置，也不比较或暴露凭据取值。

`⌘K` 命令面板同时索引默认账号和每套接入。当前项标为「当前接入」且不可重复执行；选择其他项
直接走 `agents:configure --use`（切回默认走 `--use-default`），写入带操作记录 ID。成功必须重新读取
`agents:status` 并确认 active 已变化；普通失败保留真实状态并说明原因，只有引擎明确以外部修改拒绝
覆盖时才弹出接管确认，确认后携带 `--allow-takeover` 重试。托盘仍只负责打开命令面板，不绕开
可见反馈、锁、审计和确认路径。

**Codex 侧**写同一份 `config.toml`——MCP 域也写它，因此按段落分工：本工具在这里拥有
四个顶层键（`model_provider`、`model`、`approval_policy`、`sandbox_mode`）与带受管标记
`# [agent-manager:managed]` 的 `[model_providers.*]` 段，其余行（含注释、用户自己的键、
MCP 域写的 `[mcp_servers.*]`）逐行原样保留。写入是文本级编辑：顶层键就地替换或插在第一个
表头之前（越过紧贴表头的注释，否则会把受管标记与它的表段拆散），表段整段替换。

Codex 的令牌不落盘：provider 用 `env_key` 指向环境变量名，由 Codex 启动时自己去读，
因此这里只写变量名——与 Claude 那边被迫写真值不同。`wire_api` 固定 `responses`
（当前唯一支持值）。`openai` / `ollama` / `lmstudio` 是 Codex 内置 id，用作接入配置名会被
拒绝。自定义接入方式没有默认地址，只填令牌变量名而不填地址同样拒绝。
跳过权限确认在 Codex 侧写 `approval_policy = "never"` 与 `sandbox_mode = "danger-full-access"`。

状态栏只有 Claude 侧有（Codex 没有这个机制）。

**可用性检查**（`agents:check --only <助手> [--use <接入配置>]`）是全局唯一会主动联网的
助手侧命令，只在调用方点它时才发请求，其余命令一律零网络。它只读：不碰配置文件、
不改生效的接入配置，除审计外不写库。缺地址、缺模型、缺变量名或变量没取值时在发请求之前
就拒绝并说清楚缺什么。请求按各自协议拼（Claude `{base}/v1/messages`、
Codex `{base}/responses`），HTTP 结果翻成可执行的话（401 → 令牌无效、404 → 端点形状不对、
429 → 地址与令牌都对但被限流，算通过）。令牌经 0600 临时文件交给 curl 的 `--config`
读取，不进命令行（`ps` 对同机进程可见），上游错误正文出库前走脱敏。

**厂商预设**（`app/src/lib/connectionPresets.ts`）只是填表助手，不参与落地：选中即把地址、
模型与模型别名填进表单，用户可继续改。收录标准是「厂商自营 + 官方文档写明端点」，
聚合中转站一概不收——那类地址随运营变动，这份表没有维护通道，过期的地址比没有更糟。
两种形态的端点不通用（Anthropic 协议 vs Responses 协议），因此每条都标形态、按形态筛选，
单测锁住「Claude 的都以 /anthropic 结尾、Codex 的都不带它，同厂商两侧地址不相同」。

按 2026-09-19 产品裁决，界面只保留能改变下一步动作的信息与开关，以下一并退役：

- 引擎从不读取的 `agents.<id>.auto_install` / `auto_update` 偏好，以及写入它们的「个性化配置」页（policy 原文编辑器）；`policy:get/set` 本身保留，继续服务定义增删与启停。
- 助手 `observed` 的 `providers`、`classified`、`preferred_active` 三个派生字段合并为单值 `install_source`；能否由本工具管理仍只由 `state`（`unmanaged-detected`）表达。
- 手动离线模式与 `upstream_ttl_sec` 偏好：`AppSettings` 只剩 `log_retention_days`，旧 `app_settings` 行读取时跳过、下次保存时删除；Tauri 不再注入 `--offline`。引擎 CLI 的 `--offline`、envelope `mode.offline` 与 `actions[].offline_ok` 保留不变。
- 更新中心页并入总览清单（可更新项在清单内给「去处理」入口）；总览不再展示 `system:audit` 明细分段；接管中心从一级导航移到首次使用引导、命令面板与托盘入口。
- JSONL 结构化诊断日志整套退役：写入与轮转、`logs:clear`、envelope `log_file`、启动维护清理与设置页的日志目录/清理入口一并移除；错误消息出库前仍统一脱敏（`sanitize` 模块），排查依赖 `operation_logs`，`log_retention_days` 只管它。

详见 [当前架构](ARCHITECTURE.md)、[平台状态](GUI-PLATFORM-STATUS.md) 和 [Engine Contract](../contract/README.md)。
