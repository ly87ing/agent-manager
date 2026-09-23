# GUI 重设计：当前契约

## 数据模型：实际配置为准

各助手的真实文件与目录是**生效配置的唯一事实源**。本工具的数据按三层划分：

| 层 | 内容 | 位置 |
| --- | --- | --- |
| 实际配置 | 当前生效的全部配置：接入与运行参数、MCP 条目、规则文件、hooks、技能目录、别名区块 | 助手真实文件/目录 |
| 已保存未生效 | 真实环境表达不了、用户主动保存的备选：未激活的接入配置、Claude 侧停用的 MCP、未启用的规则模板、实例定义 | `native-manager.db` |
| 内置推荐 | 官方接入预设、默认规则模板、推荐别名、推荐技能来源、hooks 清理建议 | 编译内置，只读，从不自动写入库或助手环境 |

`native-manager.db` 另外只保存应用偏好（`app_settings`）、助手识别定义与定价等本工具自身知识，以及归属记录（定位 + 摘要）。它不是生效配置的来源，也不为已生效的内容另存一份当作「当前配置」。

- **读取**：界面展示、状态与表单预填一律读实际配置；查询与 status 只读零写。
- **写入**：编辑生效项即写回实际文件，只改本工具负责的键、条目或区块；写入前重读实际内容，与打开编辑时的内容比对，不一致即提示「已在外部修改」并刷新，不静默覆盖。
- **切换**：启用另一个已保存备选（接入配置、规则模板）时，先把实际内容回填进当前项（对不上任何已保存项时另存为备份），再写入目标项。外部修改因此默认保留，不再以「接管 = 用库覆盖」作为唯一出路。
- **内置推荐**：只以「推荐 · 应用」出现，逐项确认、应用前展示与实际内容的差异；不计入「待同步」，不进入批量同步，不在首次运行时固化进库。
- **首次运行**：已有的实际配置就是当前配置——读取保持零写，接入配置对不上任何已保存项时显示为「当前实际配置（未保存）」，在用户保存或切换时才入库；不因为库里没有记录就把内置默认当作生效配置。

设置写入 `app_settings` 表；policy 写入固定使用 `--layer local`，界面不提供配置原文编辑器。

**助手配置零标记。** 写进各助手配置文件的只能是该助手官方支持的字段与文件：不写自定义键（如 `_MANAGED_BY`），不往 `env`/`environment` 塞管理用变量，不写认领用注释，也不在助手配置目录放管理器文件。「哪些内容由本工具写入」只记在 `native-manager.db`：每个写入位置记定位（文件、键路径、条目名、签名或 provider id），用来知道下次写入时要撤掉哪些已不在定义里的项。所有写入都是用户对具体条目或文件的显式操作，并以 status 给出的写入基准 `base` 比对，打开后被外部改过即拒绝；本工具不做批量覆盖，也不改动用户没有让它改的键。旧版本留下的标记只在读取时识别剥离，不再代表归属，也不再写入；仅在用户显式应用该条目时顺带清理，不做启动时批量改写。唯一例外是 Shell 别名：rc 文件不是助手配置、没有可用的结构化字段，仍以成对区块注释界定受管区块。

查询与 status 必须只读零写；缺库读取不能创建数据库。保存「已保存未生效」的备选只写库，不执行链接或刷新；编辑生效项按上节直接写回实际配置。资源执行按钮直接触发真实 `apply`；`apply` 内部先做 preflight/写前校验，再按精确 scope、排他锁、外部修改检测、CAS/条件回滚执行并审计；普通同步、启停、更新不二次确认；定义移除、清日志等破坏性动作仍独立确认。

Skill、MCP 等资源及其逐助手分配要求新增、编辑/更新、启用、禁用、删除。AI CLI 定义是例外：普通界面只维护接入、运行参数和账号实例；安装、升级、卸载及完整定义编辑不再作为产品入口，软件生命周期交给原安装渠道。既有生命周期字段与引擎能力第一阶段仅作兼容保留，不代表当前产品承诺。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

## 导入现有配置（system:discover）

「导入现有配置」页一次列出本机已存在、但还没纳入管理的助手 CLI 与技能，按来源分类后批量接管（MCP 条目以各助手配置文件为准，直接在 MCP 页查看和编辑，不经这里导入）。它是侧栏常驻入口：安装新工具或在客户端新增配置后都可再次扫描，不依赖首次引导、命令面板或托盘才能找到。分类直接决定可做什么：

| 来源 | 判定 | 处理方式 |
| --- | --- | --- |
| 助手官方技能目录内的真实目录 | 非符号链接（Claude `synced/` 等） | 只标识，继续走其原生安装方案；不接管、不覆盖 |
| 外部路径的符号链接 | 目标不在自管基目录、名称不属已定义集合 | 按源根聚合为本地来源集合，软链接纳管 |
| 已定义集合的计划外链接 | 名称命中集合 `skill_paths` | 走既有集合接管通道（`--allow-takeover`） |
| 自管基目录内的无绑定链接 | 目标在 `repositories`/`skill-versions` 内 | 旧体系残留，只建议清理 |
| 断链 | 目标不存在 | 只建议清理，不自动删除 |
| 已安装但无定义的助手 CLI | 已知 bin 在 PATH 或 `~/.local/bin` 解析到 | 只登记识别与配置所需定义；安装、升级、卸载继续由原渠道负责 |

`system:discover` 只读零写零网络，不取 apply 锁；助手候选沿用 `agents:status` 同一来源分类器与有界 `--version` 探测。接管本身不是新写路径：助手登记只写定义，技能走技能域既有的真实 `apply`（锁、外部修改检测、CAS/条件回滚与审计不变）。

本地来源集合（技能集合定义的 `source_path`，与 `url` 互斥）不做内容快照：助手目录下的链接直接指向登记的源目录，内容仍由外部工具维护并随其更新；停用只删链接，外部源不被删除或改写。登记源根后，指向该源根的链接才进入受管基准（可被覆盖、停用时可删除），未登记的外部路径仍 fail-closed。源目录必须是已存在的真实目录，不得是符号链接、不得落在本工具数据目录内、不得与任一助手技能目录重叠。

## MCP 原生域状态

MCP 以各助手配置文件里的实际条目为准（详见 [MCP 配置指南](MCP-CONFIG-GUIDE.md)）。`mcp:status` 只读零写：`targets` 列出各助手的 MCP 配置文件，`elements` 按服务名汇总各助手里的同名条目（内容可以各不相同），每条给出脱敏后的已建模字段（env/headers 引用原样、字面量掩码）、原样保留的未建模官方字段、启停状态与摘要 `base`。无需导入即可看到并操作用户已有的全部条目。

`mcp:apply` 逐条操作：`add`、`edit`、`enable`、`disable`、`remove`、`copy`（复制到另一个助手，目标同名需显式替换）、`forget`（删除还没写进任何助手的已保存定义）。每次写入都带打开时该条目的 `--base`，锁内重读核对，不一致报「已在外部修改」且文件与库不变；编辑只改已建模字段，条目里其他官方字段、注释与其他表原样保留（Codex 用 `toml_edit` 原位修改）。删除独立确认。

停用：Codex/OpenCode 写官方 `enabled = false`，条目留在文件；Claude Code 没有全局逐条停用字段，停用 = 移出条目并存停用快照（`mcp_saved`），启用 = 按快照写回，目标已有同名条目则拒绝。快照不存疑似凭据明文：env 字面量命中疑似凭据检测的、以及全部 header 字面量，转存到用户选择的外部密钥文件并改为 `${VAR}`（未选可用文件则拒绝停用）；未命中检测的普通字面量（如 `NODE_ENV=production`）原样存入快照。表单可以新写普通 env 取值（直接写进配置文件、不入库），命中疑似凭据检测时拒绝并提示改用 `${VAR}`。

旧版「服务库定义 + `mcp_bindings` 分配」按 [MCP 配置指南](MCP-CONFIG-GUIDE.md#旧库迁移) 的规则迁移：读路径即时推导，第一次 MCP 写入时落库；迁移不改配置文件。库级开关、「按库版本更新」、导入候选与 MCP 接管流程已退役。总览里 MCP 不计入待同步。

Claude Code 和 Codex 的内置默认渠道均为官方 `native` 命令；brew 只在 observed 检测结果中展示，不是默认。

## Rules 与来源原生域状态

规则以各助手的规则文件为准。规则目标是主 Claude（`~/.claude/CLAUDE.md`）、主 Codex（`~/.codex/AGENTS.md`）与每个实例配置目录下对应的 `CLAUDE.md` / `AGENTS.md`。`rules:status` 只读零写，逐目标输出路径、是否存在、实际全文、`base`（文件字节 sha256，文件不存在为 `missing`）以及与哪个已保存模板或内置推荐逐字节一致，另附已保存模板 `templates` 与内置推荐 `recommended`。停用的助手或实例不探测；符号链接、非普通文件、读不了或非 UTF-8 的目标只报原因，不提供写入。

`rules:apply` 每次写一个目标的实际文件，必须带 `--only` 与 `--base`，新内容四选一：直接编辑（`--value`）、已保存模板（`--template`）、内置推荐（`--recommended`）、另一目标的实际正文（`--from`）。写入前重读实际内容与 `base` 比对，不一致即拒绝并提示「已在外部修改」，文件与库不变；写入取全局排他锁，做文件字节 CAS、失败回滚与审计，审计只记 id 与摘要。用模板、推荐或复制覆盖前先回填：目标现有内容非空且对不上任何已保存模板或内置推荐时，自动另存为备份模板（名称含目标与时间）；现有内容疑似含凭据时不入库，改为拒绝覆盖。直接编辑不另存备份。规则没有「同步」与「接管」，不产生「待同步」。

规则模板（`rule_templates`）由 `rules:save-template`（修改带 revision CAS）与 `rules:delete-template` 维护，只写库、不动文件、不取 apply 锁。内置推荐是编译内置的 `agent-rule/rules/src/shared/core.md`，只读、从不入库，界面应用前展示与目标现有内容的差异。`profiles:apply` 不再渲染或检测实例规则文件；新建实例不自动写规则文件，可在规则页从主助手复制。

旧的共享 + 助手专用来源（`rules.shared` / `rules.claude` / `rules.codex`）已退役；hooks 与别名来源同样已退役（见「Aliases 原生域状态」「Hooks 原生域状态」），`sources:get/set` 已整体移除。旧库 `rule_sources` 中用户改过、且与内置不同的内容在写路径迁移成「旧规则来源（Claude）」「旧规则来源（Codex）」模板（按旧规则渲染共享 + 对应专用来源），读路径即时推导同一结果；迁移不改实际文件、不删旧表，旧 `rule_target_bindings` 不再作为写入门槛。配置备份导出 `rule_templates`，旧备份里的 `rules.*` 来源导入时同样转成旧来源模板。

## Aliases 原生域状态

终端快捷命令以 `~/.zshrc` 与 `~/.zprofile` 里成对注释界定的受管区块为准：区块里的 `alias` 行就是生效的快捷命令；区块外用户自己的 `alias` 行只读列出，标为「其他来源（不由本工具写入）」，从不改动。`aliases:status` 只读零写，逐文件输出区块内的别名（本工具写法 `alias 名称='取值'` 给出取值，其他写法取值为 `null`、原行照列）、区块外的别名与 `base`（区块正文摘要，没有区块为 `missing`）；顶层 `aliases` 按名合并两个文件（出现在哪些文件、写法是否一致），`alias_recommendations` 是内置推荐，`alias_legacy` 是旧版本保存、还没生效的别名。rc 文件是符号链接（例如由 dotfiles 仓库管理）时读写最终目标文件、链接本身不变；断开的链接、非普通文件、非 UTF-8 或起止注释不成对的文件只报原因，不提供写入。

`aliases:apply` 每次按名改一条：新增或改写（`--set <名称> --value <取值>`，可带 `--rename-from <旧名>` 原位改名）、删除（`--remove`）、添加一条推荐（`--recommended`）或加入一条旧版别名（`--legacy`）。要写哪些文件由 `--zshrc-base` / `--zprofile-base` 决定，界面对能写的文件都带上，两个文件保持同一写法；每个文件独立比对 `base`，任一不一致即整体拒绝并提示「已在外部修改」，文件与库不变。只替换或删除定义了该名称的行，区块内其他行与区块外字节原样保留；区块空了连同两行注释一起移除。取值不能含单引号、换行或疑似凭据（改用 `${VAR}`）。写入取全局排他锁，逐文件原子 CAS，失败回滚已写文件，审计只记名称与摘要。别名没有「同步」与「接管」，不产生「待同步」；界面也不再有全局时区与语言，写进文件的都是字面量，新建时预填内置默认值。

内置的 claude、codex、grok、chrome 四条只作「推荐 · 添加」：展示按内置时区与语言解析后的那一行，逐条确认才写入；任一文件里（区块内外）已有同名别名就不再推荐。旧库 `alias_sources` 里用户存过、与内置推荐不同、而实际区块里没有同名同值的别名，以「已保存未生效」出现，可加入（`aliases:apply --legacy`）或丢弃（`aliases:discard-legacy --legacy <id>`，只写库，删掉这一条，没有剩余时整行删除）；区块里已有同名不同值时附上现在那一行，加入即替换。读路径即时推导，写路径清理：写入时把写入前后已在区块里的旧版别名从库里删掉；旧表损坏时只是不出建议。旧 `alias_target_bindings` 不再作为写入门槛，原样留在库里。配置备份不带实际别名，只以 `aliases.shell` 来源带上仍有未生效别名的旧行，导入后同样作为建议出现。

## Hooks 原生域状态

Hooks 以各助手的实际配置为准。目标是主 Claude（`~/.claude/settings.json` 的 `hooks` 键）、主 Codex（`~/.codex/hooks.json`）与每个实例配置目录下对应的 `settings.json` / `hooks.json`。`hooks:status` 只读零写，逐目标输出路径、是否存在、实际 hooks 结构、`base` 与两类建议；`base` 在 Claude 只对 `hooks` 子树取摘要（没有该键为 `missing`），用户改 settings.json 的其他键不影响它，Codex 取 hooks.json 字节摘要（文件不存在为 `missing`）。停用的助手或实例不探测。目标是符号链接时与 `agents:configure` 同一裁决：解析到最终目标读写，原子写落在目标所在目录，CAS 比对目标字节，链接本身不变；断链、指向目录、非普通文件、非 JSON 对象或 hooks 骨架非法的目标只报原因，不提供写入。hook 的 `type` 除 `command` 外还有官方的 `http`、`mcp_tool`、`prompt`、`agent` 与将来的类型：结构校验只看「事件名 → 数组 → matcher/hooks」骨架，`command` 类型另要求非空 `command`；其他类型不阻塞目标，如实展示，按行编辑时只读、写回时整条原样保留，原文编辑可直接改。

`hooks:apply` 每次写一个目标的实际文件，必须带 `--only` 与 `--base`，新内容三选一：直接编辑（`--value` 完整 hooks 对象，只做上述骨架校验）、一条清理建议（`--cleanup`）、一条旧版本保存的 hook（`--legacy`）。写入前重读实际内容与 `base` 比对，不一致即拒绝并提示「已在外部修改」，文件与库不变；写入取全局排他锁，做文件字节 CAS、失败回滚与审计，审计只记 id 与摘要、不记命令。Claude 只替换 `hooks` 键（原位替换，其余键语义保留；为空时移除该键），与 `agents:configure` 各管各的键、互不覆盖；Codex 写 hooks.json 的 `hooks` 键，其余根键保留。Hooks 没有「同步」与「接管」，不产生「待同步」。

内置 removed_signatures 只作「清理建议」：实际 hooks 里命中旧版本签名的 `command` 类型条目逐条列出，确认「清理」才移除那一条（同样带 `base`）；没有命中就不显示。旧库 `hook_sources` 里用户存过、与内置不同、而主助手实际配置里没有的 hook，以「旧版已保存的 hooks（未生效）」出现，可逐条「加入实际配置」（`hooks:apply --legacy`）或「丢弃」（`hooks:discard-legacy --legacy <id>`，只写库，删掉库里这一条，来源空了整行删除）；已在实际配置里的不重复提示。读路径即时推导，写路径清理：对主助手目标写入时，把写入前后已在实际配置里的条目从库里删掉；旧表损坏时只是不出建议，不挡住 Hooks 页。旧 `hook_target_bindings` 不再作为写入门槛。hooks 来源键随 `sources:get/set` 一并退役。`profiles:apply` 不再渲染实例 hooks（不写 hooks.json，也不碰实例 settings.json 的 `hooks` 键）；实例 hooks 在 Hooks 页按实际文件编辑，旧版本含 hooks 的实例写入指纹照常认。配置备份不带实际 hooks，只以 `hooks.*` 来源带上仍未生效的旧版 hooks，导入后同样作为未生效建议出现。

`system:audit` 已接入 Rust dispatcher：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。密钥页先明确显示保存位置，再列变量与来源；用户选择权限为 `600` 的外部 `.env` 文件后才能填写或接管取值。SQLite 只保存路径，状态、日志与配置备份都不含取值；历史数据库值只显示数量并通过输入确认词显式迁移，写文件成功后才删旧表。设置页「配置工具」可达客户端诊断、Aliases 与 Secrets，Aliases 以 rc 文件受管区块为准（见「Aliases 原生域状态」）。按 2026-09-12 产品裁决，Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线 Rust dispatcher：探测宇宙为 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表，三域 status 只读回填 freshness（缺表/缺行一律 unknown，status 零写零网络），`--offline` 拒绝。

SQLite 核心、Tauri bridge、设置页面、主要资源页面布局、Agent 原生生命周期、Skills 原生域、MCP 原生域、Rules 原生域、Hooks 原生域、Aliases 原生域、Runs SQLite 操作记录及 Python 清理已完成。内置技能仓库只作推荐来源（`skills:status.recommendations`），添加后才成为用户集合且不自动分配给任何助手；技能页另展示各助手技能目录的实际条目（`skills:status.installed`）；集合与 Skill×Agent 均支持独立更新、启停和移除；启用通过内容寻址快照与安全软连接落地；status 只读零写，真实 apply 具备进程锁与操作审计。任务记录基于 `operation_logs` 审计实际操作，保留期由 `log_retention_days` 控制（0 为不自动清理），执行中心提供二次确认清空。Hooks 原生域以实际配置为准（见「Hooks 原生域状态」），hooks 专用 `sources:get/set` 已退役。验收基线提交 `5a110be3` 的候选包已完成主要真实 GUI E2E（旧版 plan→confirm 交互，现已失效，不证明本轮直接 apply 新交互）；2026-09-09 Darwin x86_64 DMG 的只读挂载、仓库外复制、无 Python、路径脱敏和 5 秒启动验收早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环；签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

旧 DMG/旧 GUI E2E 不证明本轮新交互。本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（提交、SHA256 与验证结果见 [交付回执](goal-rust-gui.md)）。

## 额度来源

助手卡片的额度只读本机文件，不联网、不调 API、不装 hook。

Claude 侧两个来源取 `checked_at` 较新的那个：`<data_dir>/usage/<编码后的配置目录>.json`
（编码先把 `%` 换成 `%25` 再把 `/` 换成 `%2F`，不往助手配置目录放文件）
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

界面在本地日聚合后按「时间 → 助手 → 模型」联动筛选；汇总卡、未定价提示与明细表
必须共用同一结果集。切换时间范围会清除助手和模型，切换助手会清除模型，避免保留已不适用的
下游条件。筛选只作用于已读结果，不触发扫描或写入。

费用是折算而非账单：`usage.pricing` 是美元/百万 token，内置 Anthropic 官方三个模型
（官方定价页，2026-09-20 核对，缓存写入 1.25×、缓存读取 0.1×），第三方留空由用户填。
缺任一相关单价时整桶不报，不拿 0 冒充。模型名里常有点号（`gpt-5.6-sol`），
而 policy 的键路径以点号分隔，因此单价只能整张表写回 `usage.pricing`，不能按模型名建子键。

## 助手运行参数

`agents:configure` 把助手的运行参数写进它自己的配置文件，与装卸升级的 `agents:apply`
分属两条路径（那条执行 shell 命令，这条写文件）。支持 Claude Code 与 Codex 两种形态
（按可执行名判定），目标分别是 `~/.claude/settings.json` 与定义里 `mcp_config` 指向的
`config.toml`（缺省 `~/.codex/config.toml`）。

**接入配置**：`settings.profiles` 存多套（官方、各家中转站），同一时刻只有一套写进
目标文件。每套含地址、令牌变量名、令牌走哪个头（`token` → `ANTHROPIC_AUTH_TOKEN` 的
Bearer，`api_key` → `ANTHROPIC_API_KEY` 的 x-api-key，中转站两种都有）、模型、小任务模型、
opus/sonnet/haiku 别名与超时；地址允许 `https://` 与任意主机的 `http://`（界面对非 https
提示令牌将明文传输）。当前是哪一套以目标文件为准：`agents:status` 把实际接入与每套已保存
接入按渲染结果逐项比较，给出 `current_config.current`（`profile` 为匹配到的那套，`default`
为助手默认账号，都不是即「当前实际配置（未保存）」）。库里的 `settings.active` 只是上次写入
记录，只在内容相同的候选之间裁决。切换时先清空接入键再写目标那套，上一套留下的变量
不会残留。`active` 指向不存在的配置一律当场拒绝。

**写入**：`agents:configure --only <助手> --base <摘要>` 加一种写法——`--runtime <json>`
只改补丁里的运行参数（`null`、`""`、`false` 撤掉该项，未出现的项不碰）；`--use <名字>` /
`--use-default` 只改接入键，可带 `--connection <json>` 先替换目标那套（编辑当前接入）；
`--save-current` 只把实际配置存成接入配置，不写助手文件。`--base` 取 status 的
`current_config.base`（本工具负责部分的摘要，Codex 还算上当前引用的 provider 表）；重读后
对不上即返回 `AGENT_CONFIG_CHANGED`（「已在外部修改」），文件与库都不动。文件读不了、不是
普通文件或解析失败时 status 不给 base，界面据此禁止写入；目标是符号链接时写到最终目标文件，
链接本身不变。密钥文件、助手配置与库在同一事务里写，任一步失败全部回滚。

**切换前回填**：实际接入对不上任何已保存项时先存进库，再切换。身份（Claude：地址 + 认证头；
Codex：地址 + 令牌变量且仍是本工具写的那张表，或引用同一个接入方式）与上次写入的那套一致，
就把模型、别名、超时回填进它；否则新建「导入的配置（时间）」。Codex 用户自己的表或内置接入
方式只以 `provider_ref` 引用，不复制也不认领，切到它时只写 `model_provider` / `model`，
引用的表不在了就拒绝。Claude 的实际令牌没有对应已保存变量时：已选外部密钥文件就写进去并
生成变量名（结果里返回变量名），否则返回 `AGENT_LIVE_TOKEN_UNSAVED`，由用户去密钥页选文件或
以 `--discard-live-token` 放弃。回填后无法原样渲染回实际内容（如 AUTH_TOKEN 与 API_KEY 同时
存在）时返回 `AGENT_LIVE_UNREPRESENTABLE` 并列出原因，用户确认后以 `--discard-live` 放弃
回填直接切换。库与操作记录只存变量名，不存令牌。

**受管粒度**按键路径而不是整个顶层键：本工具只拥有 `env.ANTHROPIC_BASE_URL`、
`env.ANTHROPIC_AUTH_TOKEN`、`env.ANTHROPIC_API_KEY`、`env.ANTHROPIC_MODEL`、
`env.ANTHROPIC_SMALL_FAST_MODEL`、`env.ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`、
`env.API_TIMEOUT_MS`、`env.ENABLE_TOOL_SEARCH`、`env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`、
`env.CLAUDE_CODE_EFFORT_LEVEL`、`env.CLAUDE_CODE_DISABLE_ARTIFACT`、`permissions.defaultMode`
与 `attribution.{commit,pr}`。同一文件里 hooks 域的内容、
用户自己的 `permissions.allow/deny` 与别的 `env` 变量原样保留；写入基准也只算
这几条的摘要。从没写过而这几项已有内容时，表单已按实际值预填，带基准写入即可。

**状态栏**是条件受管：运行参数 `statusline` 设为 `builtin` 时部署内置脚本到
`<data_dir>/statusline.sh`（0755）并把 `statusLine` 指过去（Claude Code 在 shell 中执行该命令，路径按单引号引用；旧版写入的裸路径仍识别为本工具的，并报 `builtin-legacy`，界面给出「修复状态栏」入口）；设回保持现状时
只撤掉我们自己指过去的那条，用户手配的原样留着。判据是 `statusLine.command` 是否指向
本工具部署的那份——指向别处的是用户的，既不清空也不算外部修改。内置脚本除了显示，
还会把助手推给状态栏的 `rate_limits` 落盘（见「额度来源」）。

**密钥**语义与实例、MCP 一致：库里只存变量名，写目标文件时解析成真值——`settings.json`
的 `env` 是字面量映射，Claude Code 不对它做变量展开，引用写进去不生效。变量取不到值时
拒绝写入而不是落一个空令牌。落地文件按 0600 写。

自动化以 `settings.automation_mode` 表达手动、智能自动、完全放开三档，由 Claude/Codex
适配器分别渲染成客户端支持的字段。完全放开会改掉助手的安全默认值，因此 `--allow-yolo`
由引擎强制：补丁把档位设为完全放开时少了这个标志一律拒绝，界面在此之前弹二次确认。写入取排他锁、记操作审计，
审计只记改了哪几项与变量名，不记取值。

助手页只服务运行时任务：先选助手，再选默认环境或独立环境。默认环境按「接入 Provider / 自动化
与权限 / 启动命令 / 实际配置」分成四个上下文页；Provider 的创建、编辑和应用互相独立，点「应用」才
调用 `agents:configure`，成功回读 status 后再提示完成。每套非默认接入可单独执行只读可用性
检查，也可由用户显式触发当前助手的全部接入并发检查；单项失败不阻断其他结果，完成后汇总
可用数量与最快线路。检查不切换接入、不写配置；最快线路不是当前项时提供显式启用入口，切换
仍复用 `agents:configure` 与状态回读。独立环境在同一工作区单独创建，用于独立账号、
登录和会话；它不是新增 Provider 的另一种写法。独立环境的定义保存在库里（真实环境里只有目录
与启动命令），它生成的配置文件以实际内容为准：卡片逐字段列出实例定义与实际文件的差异，
用户在「用定义覆盖实际配置」（先确认，只改实例负责的 env 变量与模型）和「以实际配置更新定义」
（只写库，凭据不以明文入库）之间明确选择，写入都带状态给出的基准，外部改过即拒绝并刷新。启动命令若携带权限参数，界面明确标记其会覆盖环境
自动化；实际配置页按 Provider 与运行设置分组展示目标文件和脱敏后的最终读取值，运行设置逐项展示
客户端实际值，不把内部 JSON 直接交给用户判断。
普通页面不展示 AI CLI 的安装、升级、卸载动作、生命周期脚本或完整 alias。

AI CLI 软件生命周期不再由产品界面维护。客户端是否可用、当前版本和安装来源只在
「设置 → 客户端诊断」只读展示，不联网更新、不执行脚本；实际安装和更新交给官方安装器或系统
包管理器。第一阶段不迁移或删除数据库中既有 `providers.*.install/upgrade/uninstall` 字段，旧定义
仍可无损读回；这些字段只是兼容保留，不再提供用户编辑入口。后续物理清理必须单独完成调用、CLI、
打包和迁移审计。

「添加 Provider」先展示按助手形态筛过的厂商预设，也允许自定义；选中后地址、模型、凭据变量和
模型别名自动填入，用户仍可修改。提交时先保存完整定义且保留未知字段、其他接入与原生设置，
不会提前改 `settings.active`。Claude 可在同一弹窗选填凭据取值，取值通过 `secrets:set`
单独保存；Codex 只登记环境变量名，不提供明文取值框。创建完成后仍需用户显式点击应用；后一步
失败不把新 Provider 冒充为当前项。

接入工作台中的「自动化与权限」使用短弹窗，按客户端实际值预填，只拥有 `automation_mode`、
适配器声明的运行字段与 Claude 的 `statusline`。提交只把改动过的字段作为 `--runtime` 补丁交给
`agents:configure`，由引擎在同一事务里写文件并更新库里的上次写入记录；未改动的项（包括本工具
不提供的取值，如 `acceptEdits`，表单显示为「保持当前」）原样保留。开启完全放开仍由引擎要求
`--allow-yolo` 并经独立二次确认。

目标文件被外部改过时不再提供「用软件配置覆盖」：引擎按写入基准拒绝后，界面刷新为实际值，
由用户确认后重来。实际配置对不上任何已保存接入时，接入列表首位显示「当前实际配置（未保存）」，
可「保存为接入配置」（`--save-current`），切换到别的接入时也会先自动保存；令牌还没保存或实际
内容存不下时弹窗说明原因，由用户选择去密钥页或显式放弃。

接入卡片可直接编辑和删除，不再要求进入完整助手定义。编辑当前项即写入：`--use <名字>
--connection <json>` 一条命令写文件并更新库，被拒时两边都不动；编辑非当前项只保存，不暗中
改变当前项。内部唯一名称在编辑时固定，
避免一次编辑变成复制并遗留旧项。删除只允许非当前项，先确认再从定义中移除目标 profile；凭据
变量可能被其他接入复用，因此不随 profile 自动删除。目标 profile 中表单尚不认识的字段保留，
已知可选字段仍允许清空。每套接入可保存不超过 200 字的备注；卡片直接展示，命令面板把备注纳入
搜索。备注只存数据库定义，不参与 Claude/Codex 配置渲染；复制与无密钥分享会一并携带。

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
地址、模型、凭据是否配置及运行参数放进脱敏的 `observed.current_config`，并给出写入基准 `base`
与按内容识别的 `current`（另含 `last_written`、令牌无法核对的 `unverified`、身份与上次写入一致的
`drifted_from`）。Claude 的 `credential.saved_var` 说明实际令牌与哪个已保存变量一致；自动化档位
不在预设里时报 `custom`，原值放在 `automation_raw`。status 零写，不返回 token/API key 正文，
不生成数据库 profile；符号链接按最终目标读，断链、非普通文件、超限或畸形内容按固定状态
fail-closed。界面的当前接入、运行参数与表单预填全部读实际值；写入预览只列这次改动的键。
`agents:configure` 返回成功后，必须回读到 `current` 是目标、或补丁里每一项都已是实际值，才向
用户提示成功。

`⌘K` 命令面板同时索引默认账号和每套接入，并匹配接入名称、内部名、地址、模型及备注。status 的
`current` 指向的那项标为「当前接入」且不可重复执行。选择其他项直接走带基准的
`agents:configure --use`（切回默认走 `--use-default`），写入带操作记录 ID。成功必须重新读取
`agents:status`，确认 `current` 已是目标；普通失败保留真实状态并说明原因。外部修改被拒时刷新并
提示重新选择；令牌未保存或实际配置存不下时弹窗说明，确认后带 `--discard-live-token` /
`--discard-live` 重试。托盘按助手分组展示默认账号与每套接入，当前项带勾且不可重复执行，实际配置
未保存时子菜单标为「实际配置（未保存）」；点击目标会唤回已精确过滤的命令面板，用户再按一次
Enter 或点击后才写入。托盘不静默切换，也不绕开可见反馈、锁、审计和确认路径。

`⌘⇧K` 使用同一命令面板进入“接入快切”范围，初始只展示 Claude/Codex 的默认账号与自定义接入，
仍可按名称、备注、地址或模型过滤。Esc 关闭后不保留该范围，下一次 `⌘K` 恢复完整页面、资源与
动作；快捷键在文本输入语境中不抢按键。快切只减少查找步骤，不新增写路径或跳过确认与双回读。

**Codex 侧**写同一份 `config.toml`——MCP 域也写它，因此按段落分工：本工具在这里拥有
`model_provider`、`model`、`approval_policy`、`sandbox_mode`、`approvals_reviewer`、
`model_reasoning_effort`、`model_context_window`、`model_auto_compact_token_limit`、`web_search`
九个顶层键，以及 `agent_settings_bindings.owned_providers` 记下的本工具写入的
`[model_providers.*]` 段（含子表）；文件里不写任何管理注释。引用型接入（`provider_ref`）
只写 `model_provider` / `model`，被引用的表不归本工具，既不写也不删。旧版本写在 provider 段前的
`# [agent-manager:managed]` 只在迁移时用于认领，下次写入即随旧段撤掉。其余行（含注释、用户自己的键与
provider、MCP 域写的 `[mcp_servers.*]`）逐行原样保留。写入是文本级编辑：顶层键就地替换或插在第一个
表头之前（越过紧贴表头的注释），表段整段替换。

Codex 的令牌不落盘：provider 用 `env_key` 指向环境变量名，由 Codex 启动时自己去读，
因此这里只写变量名——与 Claude 那边被迫写真值不同。`wire_api` 固定 `responses`
（当前唯一支持值）。`openai` / `ollama` / `lmstudio` 是 Codex 内置 id，用作接入配置名会被
拒绝（引用它们的 `provider_ref` 可以）。用户自己已有同名 provider 表时拒绝写入，提示给接入换名。自定义接入方式没有默认地址，只填令牌变量名而不填地址同样拒绝。
跳过权限确认在 Codex 侧写 `approval_policy = "never"` 与 `sandbox_mode = "danger-full-access"`。

状态栏只有 Claude 侧有（Codex 没有这个机制）。

**可用性检查**（`agents:check --only <助手> [--use <接入配置>]`）是全局唯一会主动联网的
助手侧命令，只在调用方点它时才发请求，其余命令一律零网络。它只读：不碰配置文件、
不改生效的接入配置，除审计外不写库。缺地址、缺模型、缺变量名或变量没取值时在发请求之前
就拒绝并说清楚缺什么。引用型 Codex 接入（`provider_ref`）的地址与令牌变量从 `config.toml` 里
被引用的那张表读取，表不在时内置接入方式用已知默认值（openai 读 `OPENAI_API_KEY`，本机
ollama、lmstudio 不带令牌），其余明确报「已不在 config.toml 中」。请求按各自协议拼（Claude `{base}/v1/messages`、
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
- 更新中心页并入总览清单（可更新项在清单内给「去处理」入口）；总览不再展示 `system:audit` 明细分段；“导入现有配置”保留一级导航，并同时支持命令面板与托盘入口。全新安装直接进入 Provider 列表，不再展示需要阅读后才能继续的长篇首次引导。
- JSONL 结构化诊断日志整套退役：写入与轮转、`logs:clear`、envelope `log_file`、启动维护清理与设置页的日志目录/清理入口一并移除；错误消息出库前仍统一脱敏（`sanitize` 模块），排查依赖 `operation_logs`，`log_retention_days` 只管它。

详见 [当前架构](ARCHITECTURE.md)、[平台状态](GUI-PLATFORM-STATUS.md) 和 [Engine Contract](../contract/README.md)。
