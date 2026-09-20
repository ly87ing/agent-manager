# Engine Contract v1

`contract/engine-v1/` 描述 CLI 与 Tauri 共用的 JSON envelope、状态、动作和 policy 数据形状。实现入口是 `engine-rs`；根仓库不依赖 Python 或旧 shell 管线。

`native-manager.db` 是本工具定义、分配和偏好的唯一事实源。设置存储在 SQLite 的 `app_settings` 表。policy 只返回 `builtin` 或 `sqlite` 归属；不存在三层 JSON、可写 settings JSON 或 profile 同步层。

policy 写入请求使用 `--layer local`，其中 `local` 是写入目标参数，持久化结果来源是 `sqlite`。缺库读取和 status/query 只读零写。

Agent 集合、逐 Agent Skill、MCP 库、逐 Agent MCP 的动作契约覆盖新增、编辑/更新、启用、禁用、删除。保存定义不执行动作；`apply` 直接执行引擎返回的 actions，`apply` 内部包含 preflight、精确 scope、锁、外部修改检测、CAS/条件回滚与审计。移除管理和卸载是不同 action。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

只读 `system:discover` 扫描本机已存在但未纳入管理的 agents / skills / MCP 资源，支持 `--only agents|skills|mcp`，零写零网络、不取 apply 锁。每个候选带 `manage` 分类：`native`（官方路径与原生安装方案，只标识不接管）、`symlink`（外部资源以软链接纳管）、`collection`（走已定义集合的接管通道）、`register`（登记定义，助手 CLI 与 MCP 条目）、`cleanup`（断链等残留，只建议清理）。与 `system:audit` 同属只读聚合，data 形状由前端类型固定，契约 v2 才正式固化。

`profiles:status/apply` 管理助手实例：实例定义就是带 `profile` 段的 `agents.definitions.<id>` 条目，因此技能与 MCP 的逐助手分配直接适用。`status` 只读零写，只输出变量名与能否解析；`apply` 只写实例配置目录下本工具全权拥有的那个文件（Claude 为 `settings.json`，Codex 为 `agent-manager.config.toml` 并由启动参数 `-p` 叠加，避开归 MCP 域的 `config.toml`），目录 0700、文件 0600、原子换入、事务失败回滚，已有他人内容或被外部改过一律 fail-closed 需显式 `--allow-takeover`。鉴权取值只以 `${VAR}` 持久化：Claude 目标在 apply 时解析写入（客户端不展开引用，由文件权限保护），Codex 目标只写 `env_key` 变量名。配置目录必须位于 HOME 下，不得是主实例目录、数据目录内或符号链接。

技能集合定义新增 `source_path`（与 `url` 互斥）：本地来源集合不做内容快照，助手目录下的链接直接指向登记源目录，内容随外部源更新；登记源根进入受管基准后这些链接才可被覆盖或在停用时删除，未登记的外部路径仍 fail-closed。集合级 `update` 对本地来源集合不联网、不 clone，只重扫源目录、刷新指纹并修复已启用链接。

文件：`envelope.schema.json`（响应）、`element.schema.json`（元素和 actions）、`status.schema.json`（状态）、`apply-result.schema.json`（执行结果）、`policy.schema.json`（policy 结果）、`runs.schema.json`（SQLite 操作记录与清空结果）和 `sources.schema.json`（Rules/Hooks 来源定义）。`agents:configure` 把助手运行参数写进它自己的配置文件，返回 `{domain, action, element, target, takeover}`，与装卸升级的 `agents:apply` 分属两条路径。plan/profile/converge/snapshots 与 logs schema 均已退役，不属于当前契约。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。
