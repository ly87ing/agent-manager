# GUI 重设计：当前契约

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置写入 `app_settings` 表，定义和本机 policy 写入 SQLite。内置默认值由 Rust 编译提供，只读；policy 写入固定使用 `--layer local`，界面不提供配置原文编辑器。

查询与 status 必须只读零写；缺库读取不能创建数据库。表单保存只保存定义，不执行安装、卸载、链接或刷新。执行按钮直接触发真实 `apply`；`apply` 内部先做 preflight/写前校验，再按精确 scope、排他锁、外部修改检测、CAS/条件回滚执行并审计；普通同步、启停、更新不二次确认；定义移除、清日志等破坏性动作仍独立确认。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 都要求新增、编辑/更新、启用、禁用、删除。移除管理定义与卸载软件是不同动作。

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
| 已安装但无定义的助手 CLI | 已知 bin 在 PATH 或 `~/.local/bin` 解析到 | 登记定义；只有能从全局 `node_modules` 反查出包名的 npm 渠道登记后可由本工具升级卸载，官方安装器与 Homebrew 来源只登记观测 |
| 助手配置里未受管的 MCP 条目 | 复用 `mcp:status` 同一扫描与掩码路径 | 导入库定义（密钥只以 `${VAR}` 引用入库）后写回受管标记 |

`system:discover` 只读零写零网络，不取 apply 锁；助手候选沿用 `agents:status` 同一来源分类器与有界 `--version` 探测。接管本身不是新写路径：助手登记只写定义，技能与 MCP 都走各自域既有的真实 `apply`（锁、外部修改检测、CAS/条件回滚与审计不变）。

本地来源集合（技能集合定义的 `source_path`，与 `url` 互斥）不做内容快照：助手目录下的链接直接指向登记的源目录，内容仍由外部工具维护并随其更新；停用只删链接，外部源不被删除或改写。登记源根后，指向该源根的链接才进入受管基准（可被覆盖、停用时可删除），未登记的外部路径仍 fail-closed。源目录必须是已存在的真实目录，不得是符号链接、不得落在本工具数据目录内、不得与任一助手技能目录重叠。

## MCP 原生域状态

Rust dispatcher 已实现本地 stdio 与远程 HTTP MCP 的 `mcp:status`（只读）与真实 `mcp:apply`。定义、库级开关和逐助手分配保存在 `native-manager.db` 的 SQLite 表中；支持 Claude JSON、Codex TOML 和 OpenCode JSON，OpenCode 需在对应 Agent definition 配置 `mcp_config` 目标。

逐助手支持 `add`、`update`、`enable`、`disable`、`remove`；库级支持 `enable`、`disable`、`remove`。`mcp:status` 只读零写；真实 `apply` 具备排他锁、操作审计、CAS 和回滚保护；`env` 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护。远程 HTTP MCP 已实现，与本地 stdio 同级：远程定义含 `url`（必填；仅允许 `https://`，或本机回环 `http://localhost` / `http://127.0.0.1`）与可选 `headers`（值只允许 `${VAR}` 占位符），与 stdio 字段互斥；headers 以引用形式写入目标（Claude 字面量 `${VAR}`、Codex `env_http_headers`、OpenCode `{env:VAR}`），解析后明文不落盘；受管标记统一条目级 `_MANAGED_BY`；OpenCode 远程条目固定 `"oauth": false`；远程定义不参与上游探测，freshness 恒 not-applicable。

Claude Code 和 Codex 的内置默认渠道均为官方 `native` 命令；brew 只在 observed 检测结果中展示，不是默认。

## Rules 与来源原生域状态

`sources:get/set` 已原生接线：规则来源固定为 `rules.shared`、`rules.codex`、`rules.claude`、`rules.opencode` 四项，保存使用 revision CAS 且只写 SQLite，不执行任何同步动作。

Rust dispatcher 已实现 `rules:status`（只读）与真实 `rules:apply`。固定目标为 `rules:codex_global`（`~/.codex/AGENTS.md`）、`rules:claude_global`（`~/.claude/CLAUDE.md`）与 `rules:opencode_global`（`~/.config/opencode/AGENTS.md`），内容由共享与助手专用来源渲染；`rules:status` 只读零写；真实 `apply` 具备全局排他锁、操作审计、文件字节 CAS 与事务/文件回滚保护；目标被外部修改时 fail-closed 拒绝覆盖，不承诺备份。没有中间产物目录，来源直接渲染写入目标。Tauri bridge 参数白名单与 Rules 页面已对齐真实 schema，详情与逐目标操作均以 `--only <目标>` 精确限定。

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

## 助手运行参数

`agents:configure` 把助手的运行参数写进它自己的配置文件，与装卸升级的 `agents:apply`
分属两条路径（那条执行 shell 命令，这条写文件）。目前只支持 Claude Code 形态
（按可执行名判定），目标固定为 `~/.claude/settings.json`。

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

界面上这几项在助手编辑弹窗里，按 2026-09-20 裁决保存即写入：保存定义成功后立即执行
一次真实写入，失败时定义已存但目标文件不动，并说明原因。卡片上另有接入配置下拉，
选中即切换。

Codex 形态（`~/.codex/config.toml` 的 `model_providers` / `approval_policy`）尚未接入。

按 2026-09-19 产品裁决，界面只保留能改变下一步动作的信息与开关，以下一并退役：

- 引擎从不读取的 `agents.<id>.auto_install` / `auto_update` 偏好，以及写入它们的「个性化配置」页（policy 原文编辑器）；`policy:get/set` 本身保留，继续服务定义增删与启停。
- 助手 `observed` 的 `providers`、`classified`、`preferred_active` 三个派生字段合并为单值 `install_source`；能否由本工具管理仍只由 `state`（`unmanaged-detected`）表达。
- 手动离线模式与 `upstream_ttl_sec` 偏好：`AppSettings` 只剩 `log_retention_days`，旧 `app_settings` 行读取时跳过、下次保存时删除；Tauri 不再注入 `--offline`。引擎 CLI 的 `--offline`、envelope `mode.offline` 与 `actions[].offline_ok` 保留不变。
- 更新中心页并入总览清单（可更新项在清单内给「去处理」入口）；总览不再展示 `system:audit` 明细分段；接管中心从一级导航移到首次使用引导、命令面板与托盘入口。
- JSONL 结构化诊断日志整套退役：写入与轮转、`logs:clear`、envelope `log_file`、启动维护清理与设置页的日志目录/清理入口一并移除；错误消息出库前仍统一脱敏（`sanitize` 模块），排查依赖 `operation_logs`，`log_retention_days` 只管它。

详见 [当前架构](ARCHITECTURE.md)、[平台状态](GUI-PLATFORM-STATUS.md) 和 [Engine Contract](../contract/README.md)。
