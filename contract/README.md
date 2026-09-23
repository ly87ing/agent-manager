# Engine Contract v1

`contract/engine-v1/` 描述 CLI 与 Tauri 共用的 JSON envelope、状态、动作和 policy 数据形状。实现入口是 `engine-rs`；根仓库不依赖 Python 或旧 shell 管线。

`native-manager.db` 保存本工具的定义、偏好与「已保存未生效」的备选；各助手配置文件里的实际内容是生效配置的事实源。设置存储在 SQLite 的 `app_settings` 表。policy 只返回 `builtin` 或 `sqlite` 归属；不存在三层 JSON、可写 settings JSON 或 profile 同步层。

policy 写入请求使用 `--layer local`，其中 `local` 是写入目标参数，持久化结果来源是 `sqlite`。缺库读取和 status/query 只读零写。

Agent 集合与逐 Agent Skill 的动作契约覆盖新增、编辑/更新、启用、禁用、删除。保存定义不执行动作；`apply` 直接执行引擎返回的 actions，`apply` 内部包含 preflight、精确 scope、锁、外部修改检测、CAS/条件回滚与审计。移除管理和卸载是不同 action。

MCP 以各助手配置文件里的实际条目为准：`mcp:status` 的 `targets` 列出各助手的 MCP 配置文件，`elements` 按服务名汇总各助手里的条目（env/headers 引用原样、字面量掩码为 `<literal>`，未建模的官方字段非凭据类原样、其余 `<hidden>`）与每条的摘要 `base`。`mcp:apply` 逐条操作：`add`（`--spec`）、`edit`（`--spec --base`）、`enable`/`disable`/`remove`（`--base`）、`copy`（`--to --base`，覆盖同名需 `--replace --to-base`）、`forget`（删除未分配的已保存定义）；写入前在锁内重读并比对 `--base`，不一致报「已在外部修改」且文件与库不变。停用按 `targets[].disable_mode`：Codex/OpenCode 写官方 `enabled = false`，Claude Code 移出条目并存停用快照（`mcp_saved`，不存疑似凭据明文）。附带提示（转存了哪些变量、复制时未带过去的字段）经 envelope `warnings` 返回。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

只读 `system:discover` 扫描本机已存在但未纳入管理的 agents / skills 资源，支持 `--only agents|skills`，零写零网络、不取 apply 锁（MCP 条目直接在 `mcp:status` 里列出，不经发现与导入）。每个候选带 `manage` 分类：`native`（官方路径与原生安装方案，只标识不接管）、`symlink`（外部资源以软链接纳管）、`collection`（走已定义集合的接管通道）、`register`（登记定义，助手 CLI）、`cleanup`（断链等残留，只建议清理）。与 `system:audit` 同属只读聚合，data 形状由前端类型固定，契约 v2 才正式固化。

`profiles:status/apply` 管理助手实例：实例定义就是带 `profile` 段的 `agents.definitions.<id>` 条目，因此技能的逐助手分配与 MCP 的逐助手条目直接适用。`status` 只读零写，只输出变量名与能否解析；`apply` 只写实例配置目录下本工具全权拥有的那个文件（Claude 为 `settings.json`，Codex 为 `agent-manager.config.toml` 并由启动参数 `-p` 叠加，避开归 MCP 域的 `config.toml`），目录 0700、文件 0600、原子换入、事务失败回滚，已有他人内容或被外部改过一律 fail-closed 需显式 `--allow-takeover`。鉴权取值只以 `${VAR}` 持久化：Claude 目标在 apply 时解析写入（客户端不展开引用，由文件权限保护），Codex 目标只写 `env_key` 变量名。配置目录必须位于 HOME 下，不得是主实例目录、数据目录内或符号链接。

技能集合定义新增 `source_path`（与 `url` 互斥）：本地来源集合不做内容快照，助手目录下的链接直接指向登记源目录，内容随外部源更新；登记源根进入受管基准后这些链接才可被覆盖或在停用时删除，未登记的外部路径仍 fail-closed。集合级 `update` 对本地来源集合不联网、不 clone，只重扫源目录、刷新指纹并修复已启用链接。

`skills:status` 另有两个只读字段：`recommendations` 是编译内置的推荐技能来源（不是元素，不计入待同步、不参与上游检查、不自动下载；添加 = 把其中的 `definition` 保存为普通集合定义，不限助手、不分配），`installed` 是各助手技能目录里的实际条目（与 `system:discover` 同一扫描与分类，另以 `managed` 标出本工具链接）。旧版本首次写库固化的 `agent-skills` 集合若仍为原值且从未生效（无 `skill_bindings`），读路径按推荐处理，写路径在 `open_write` 内移出 policy 并记 `skills:retire-builtin` 操作日志；已生效或改过的保持为用户集合。

`rules:status` 逐目标（主 Claude/Codex 与各实例规则文件）输出实际全文、`base`（文件字节 sha256，不存在为 `missing`）与匹配的模板，另带 `templates`（已保存模板）与 `recommended`（内置推荐，不入库）。`rules:apply` 必须带 `--only` 与 `--base`，新内容取 `--value` / `--template` / `--recommended` / `--from` 之一；`base` 不一致即拒绝（「已在外部修改」）；用模板、推荐或复制覆盖前，对不上任何模板的现有内容先另存为备份模板。`rules:save-template`（新建 `--name --value`，修改另带 `--template --revision`）返回 `{domain, template}`，`rules:delete-template --template` 返回 `{domain, deleted}`，两者只写库。`sources:get/set` 的 `rules.*` 键已退役。

`hooks:status` 逐目标（主 Claude 的 settings.json `hooks` 键、主 Codex 的 hooks.json、各实例对应文件）在 `observed` 里输出实际 `hooks`、`hook_count`、`base`（Claude 只对 hooks 子树取摘要、Codex 取文件字节摘要，没有时为 `missing`）、`cleanup`（命中内置旧签名的清理建议）与 `legacy`（旧库保存、主助手实际配置里没有的 hooks，仅主助手目标）；`spec.scope` 为 `hooks-key` 或 `whole-file`。`hooks:apply` 必须带 `--only` 与 `--base`，新内容取 `--value <hooks JSON>` / `--cleanup <id>` / `--legacy <id>` 之一，`base` 不一致即拒绝（「已在外部修改」）；`hooks:discard-legacy --legacy <id>` 只写库，返回 `{domain, discarded, source_removed}`。目标是符号链接时解析到最终文件读写（断链或指向目录拒绝）；hook `type` 不限于 `command`，其他类型只做骨架校验并原样保留，清理建议只匹配 `command` 类型。`--allow-takeover` 与 `sources:get/set` 的 `hooks.*` 键已退役。

文件：`envelope.schema.json`（响应）、`element.schema.json`（元素和 actions）、`status.schema.json`（状态）、`apply-result.schema.json`（执行结果）、`policy.schema.json`（policy 结果）、`runs.schema.json`（SQLite 操作记录与清空结果）和 `sources.schema.json`（别名来源定义）。`agents:configure` 把助手运行参数写进它自己的配置文件，与装卸升级的 `agents:apply` 分属两条路径：必须带 `agents:status` 给出的写入基准 `--base <current_config.base>`，写法四选一（`--runtime <json>` 运行参数补丁、`--use <接入配置>` 可带 `--connection <json>`、`--use-default`、`--save-current`），可选 `--allow-yolo`、`--discard-live`、`--discard-live-token`；返回 `{domain, action, element, target, mode, active, current, changed, backfill, discarded_live, base, statusline}`。拒绝码：`AGENT_CONFIG_CHANGED`（已在外部修改）、`AGENT_LIVE_TOKEN_UNSAVED`（实际令牌还没保存）、`AGENT_LIVE_UNREPRESENTABLE`（实际接入无法完整保存），拒绝时文件与库都不动。plan/profile/converge/snapshots 与 logs schema 均已退役，不属于当前契约。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。
