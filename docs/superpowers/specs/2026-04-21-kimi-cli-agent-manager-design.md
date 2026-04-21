# Kimi CLI 接入 Agent Manager 设计

## 目标

将 `Kimi Code CLI` 作为一等受管 agent 接入 `agent-manager`，覆盖以下能力：

- lifecycle：安装、升级、卸载、状态探测
- MCP：同步到 `~/.kimi/mcp.json`
- rules：同步到 `~/.kimi/AGENTS.md` 与 `~/.kimi/modules/`
- skills：同步到 `~/.kimi/skills/`

同时保持项目级规则入口不变，继续复用仓库根和目录级 `AGENTS.md`。

## 设计结论

采用与现有 agent 管理链路一致的最小可维护接入方案：

1. 新增稳定 agent id `kimi-cli`，显示名为 `Kimi Code CLI`
2. 官方安装源只托管 `uv` provider，不额外引入 npm 或 brew 的非官方安装路径
3. MCP 复用现有 JSON `mcpServers` 注入逻辑，不新增 Kimi 特例格式
4. skills 复用现有目录 link 管理逻辑，目标目录为 `~/.kimi/skills`
5. rules 为 Kimi 新增独立分发 target，生成 `~/.kimi/AGENTS.md` 与 `~/.kimi/modules`
6. Kimi 的 rules 产物不能直接复用 Codex 产物，必须把模块引用重写到 `~/.kimi/modules/*.md`
7. 本轮不托管 `~/.kimi/config.toml`，不接管 provider、login、API key 或 Beta hooks

## 假设与边界

假设：

- `Kimi Code CLI` 的可执行文件名是 `kimi`
- Kimi 官方推荐安装链路是 `uv tool install --python 3.13 kimi-cli`
- Kimi 官方 MCP 配置文件是 `~/.kimi/mcp.json`，顶层键为 `mcpServers`
- Kimi 官方 skills 目录是 `~/.kimi/skills`
- Kimi 官方会把项目根到当前目录逐层合并的 `AGENTS.md` 注入运行时上下文

边界：

- 不托管 `~/.kimi/config.toml`
- 不托管 Kimi provider 选择、model 配置、登录态或真实 secrets
- 不托管 Kimi Beta hooks
- 不新增 Kimi 专属 repo-native 项目配置目录
- 不把所有 `codex` 专属 skill 无差别开放给 `kimi-cli`

## 生命周期设计

需要修改以下入口：

1. [`config/agents.json`](/Users/ly/agent-manager/config/agents.json)
   - 新增 `kimi-cli`
   - `display_name` 设为 `Kimi Code CLI`
   - `bin` 设为 `kimi`
   - `preferred_provider` 设为 `uv`
   - `providers.uv.install` 使用 `uv tool install --python 3.13 kimi-cli`
   - `providers.uv.uninstall` 使用 `uv tool uninstall kimi-cli`
   - `providers.uv.upgrade` 使用 `uv tool upgrade kimi-cli --no-cache`
2. [`config/manager.json`](/Users/ly/agent-manager/config/manager.json)
   - 新增 `agents.kimi-cli`
   - 默认策略采用 `enabled: true`、`auto_install: false`、`auto_update: true`、`preferred_provider: uv`
3. [`src/agent_ids.py`](/Users/ly/agent-manager/src/agent_ids.py)
   - 增加 `kimi-cli` metadata、display name 和 aliases
4. [`src/manage_runtime_probe.py`](/Users/ly/agent-manager/src/manage_runtime_probe.py)
   - 新增 `uv` provider 探测
   - 优先通过 `uv tool list` 明确识别 `kimi-cli`
   - 未明确命中时保持 `system` 或未安装，不做隐式接管
5. [`agent-lifecycle/sync_agents.py`](/Users/ly/agent-manager/agent-lifecycle/sync_agents.py)
   - 复用既有 reconcile 流程，不新增 Kimi 专用分支

默认不自动安装 `Kimi Code CLI`，原因是当前仓库已对 `Cursor IDE` 与 `OpenCode` 采用相同的保守默认值。这样可以把 Kimi 纳入统一托管范围，但避免在所有机器上自动写入新的 CLI。

## MCP 设计

需要修改以下入口：

1. [`config/mcp.json`](/Users/ly/agent-manager/config/mcp.json)
   - 新增 `agent_paths.kimi-cli = "~/.kimi/mcp.json"`
   - 格式使用 JSON
2. [`agent-mcp/sync_mcp.py`](/Users/ly/agent-manager/agent-mcp/sync_mcp.py)
   - 复用标准 JSON `mcpServers` 逻辑
   - 不新增 Kimi 格式特例
3. [`src/manage_doctor.py`](/Users/ly/agent-manager/src/manage_doctor.py)
   - 将 Kimi 视作普通 JSON MCP target 审计

MCP 的冲突处理沿用现有托管规则：

- 只接管同名 server
- 继续使用 `_MANAGED_BY = "agent-manager"` 标记
- 非同名且非受管条目保留

## Skills 设计

需要修改以下入口：

1. [`config/skills.json`](/Users/ly/agent-manager/config/skills.json)
   - 新增 `agents.kimi-cli = "~/.kimi/skills"`
   - 将当前对 `claude-code` 开放的 skill 仓库同步开放给 `kimi-cli`
2. [`agent-skill/link_skills.py`](/Users/ly/agent-manager/agent-skill/link_skills.py)
   - 复用普通目录 target 逻辑
   - 不新增 Kimi 特判

skill 暴露策略：

- 目标是让 Kimi 具备与 `Claude Code` 同级别的受管 skills
- 仅扩展当前已经对 `claude-code` 开放的 skill scope
- 不自动把 `codex` 专属 skill 扩展给 `kimi-cli`

## Rules 设计

需要修改以下入口：

1. [`agent-rule/scripts/build-agent-rules.py`](/Users/ly/agent-manager/agent-rule/scripts/build-agent-rules.py)
   - 新增 `kimi` target
   - 生成文件名为 `AGENTS.md`
   - 规则主体以现有共享 core 为基础
   - 模块引用重写到 `~/.kimi/modules/*.md`
2. [`config/rules-targets.list`](/Users/ly/agent-manager/config/rules-targets.list)
   - 新增 `kimi_global|global|rules/dist/kimi/AGENTS.md|~/.kimi/AGENTS.md`
   - 新增 `kimi_modules|global|rules/dist/modules|~/.kimi/modules`
3. [`agent-rule/scripts/sync-agent-rules.sh`](/Users/ly/agent-manager/agent-rule/scripts/sync-agent-rules.sh)
   - 补充 `~/.kimi` 目标路径的 agent 安装判定
   - 使用 `command -v kimi` 判断用户级 target 是否应参与同步
4. [`src/manage_runtime_probe.py`](/Users/ly/agent-manager/src/manage_runtime_probe.py)
   - 扩展 rule target 前缀映射，纳入 `kimi`
5. [`src/manage_doctor.py`](/Users/ly/agent-manager/src/manage_doctor.py)
   - 扩展 rule target 前缀映射，纳入 `kimi`

项目级 rules 保持不变：

- 仓库根 `AGENTS.md` 继续作为 Kimi 的项目级入口
- 目录级 `AGENTS.md` 继续由 Kimi 官方的逐层合并机制消费
- 不新增 `.kimi/AGENTS.md` 这种 repo-native 副本

## 文档与契约设计

需要同步更新以下文件：

- [`README.md`](/Users/ly/agent-manager/README.md)
- [`docs/ARCHITECTURE.md`](/Users/ly/agent-manager/docs/ARCHITECTURE.md)

需要明确记录：

- `Kimi Code CLI` 已成为受管 agent
- 用户级托管路径为 `~/.kimi/mcp.json`、`~/.kimi/skills`、`~/.kimi/AGENTS.md`、`~/.kimi/modules`
- `~/.kimi/config.toml` 不在本轮托管范围

## 失败模式与处理

1. `uv tool list` 输出格式不符合预期
   - provider 探测必须 fail-close
   - 只有明确命中 `kimi-cli` 时才判为 `uv`
2. 用户已手工维护 `~/.kimi/mcp.json`
   - 仅接管同名条目
   - 非受管条目保留
3. 用户已手工维护 `~/.kimi/skills/`
   - 仅清理 agent-manager 管理的 links
   - 遇到未知冲突目录时直接报错
4. 用户已存在 `~/.kimi/AGENTS.md` 或 `~/.kimi/modules`
   - 按现有 rules sync 语义先备份再替换
5. Kimi 对模块引用的消费行为与预期不同
   - `AGENTS.md` 主体必须自洽
   - 后续若验证不兼容，再退回单文件内联方案

## 回滚策略

1. lifecycle 回滚
   - 关闭 [`config/manager.json`](/Users/ly/agent-manager/config/manager.json) 中的 `agents.kimi-cli.enabled`
   - 再执行同步
2. MCP 回滚
   - 仅删除受管条目
   - 可用 `~/.kimi/mcp.json.bak.*` 恢复
3. skills 回滚
   - 仅删除 agent-manager 建立的 links
4. rules 回滚
   - 使用 `~/.kimi/AGENTS.md.bak.*` 和 `~/.kimi/modules.bak.*` 恢复

## 验证方式

定向验证：

1. `python3 -m unittest tests.test_agent_lifecycle -v`
2. `python3 -m unittest tests.test_manage_runtime_probe -v`
3. `python3 -m unittest tests.test_sync_mcp -v`
4. `python3 -m unittest tests.test_link_skills -v`
5. `python3 -m unittest tests.test_sync_rules -v`
6. `python3 -m unittest tests.test_manage_doctor -v`
7. `python3 -m unittest tests.test_skill_exposure_contracts -v`
8. `python3 -m unittest tests.test_project_contracts -v`

构建与更宽回归：

1. `make rules-build`
2. `python3 -m unittest tests.test_project_contracts -v`

## 验收标准

- `agent-manager` 能识别并管理 `Kimi Code CLI`
- `~/.kimi/mcp.json` 能收到受管 MCP
- `~/.kimi/skills/` 能收到与 `Claude Code` 同级别的受管 skills scope
- `~/.kimi/AGENTS.md` 与 `~/.kimi/modules/` 能被稳定生成与同步
- README 和架构文档与实现保持一致
- 本轮实现不修改 `~/.kimi/config.toml`
