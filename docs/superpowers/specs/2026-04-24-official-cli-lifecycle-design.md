# Claude Code / Codex / Gemini CLI 官方生命周期管理设计

## 目标

将 `Claude Code`、`Codex`、`Gemini CLI` 纳入 `agent-manager` 的统一 lifecycle 管理，并满足以下要求：

- 放弃以 `brew` 作为这三者的默认或推荐管理方式
- 尽量按各工具当前官方推荐的安装 / 更新语义建模，而不是为了统一而牺牲准确性
- 保持 `agent-manager` 现有 `manage.sh` / `Makefile` 主入口不变
- 对历史 `brew` 安装 fail-close，要求用户显式迁移，而不是静默跳过
- 不自动卸载用户现有安装，不越权接管外部安装

## 设计结论

采用“按工具官方语义分别建模 + 最小扩展现有 provider 模型”的方案：

1. `Claude Code` 改为 `native` 默认 provider，保留 `npm` 作为 legacy-compatible 受管 provider，`brew` 在 `agent-manager` 内降级为 deprecated detect-only provider
2. `Codex` 新增为受管 agent，默认且仅托管 `npm` provider；`brew` 在 `agent-manager` 内仅保留 detect-only 语义
3. `Gemini CLI` 改为默认且仅托管 `npm` stable provider；`brew` 在 `agent-manager` 内仅保留 detect-only 语义
4. provider 模型新增最小字段 `managed: true|false`
5. agent 定义新增最小字段 `require_managed_provider: true|false`，只对需要严格收敛 provider 的 agent 开启
6. runtime probe 新增声明式路径识别能力，用于识别 `Claude Code` 的官方 native installer 路径
7. provider 命令值允许使用“单字符串”或“按平台分发的命令映射”，用于准确表达 `Claude Code native` 的跨平台安装命令
8. `doctor`、`bootstrap`、`update` 对 deprecated provider 与 unmanaged external install 采用 fail-close 策略
9. 不托管 `Claude Code local install`，因为其官方暴露方式依赖 shell alias，不适合当前非交互式 PATH 探测框架稳定接管

## 官方事实基线

以下结论基于 `2026-04-24` 核对的官方资料：

- `Claude Code`
  - Anthropic 官方 GitHub README 已将 npm 描述为 deprecated，并推荐 native installer
  - Anthropic 官方 setup / troubleshooting 文档仍保留 `npm install -g @anthropic-ai/claude-code`、`claude update`、`claude migrate-installer`
  - 这说明最准确的建模不是“只认 native”，而是“native-first，npm-legacy”
- `Codex`
  - OpenAI 官方 Codex CLI 文档当前主路径仍是 `npm install -g @openai/codex`
  - 升级仍按 npm latest 建模最稳妥
- `Gemini CLI`
  - Google 官方 Gemini CLI README 同时支持 `npx`、`npm`、`brew`
  - 对 `agent-manager` 这种持久化 lifecycle 托管场景，`npm` stable 比 `npx` 更适合，也比 `brew` 更贴近跨平台官方主路径

参考来源：

- <https://github.com/anthropics/claude-code>
- <https://docs.anthropic.com/en/docs/claude-code/setup>
- <https://docs.anthropic.com/en/docs/claude-code/troubleshooting>
- <https://developers.openai.com/codex/cli>
- <https://github.com/google-gemini/gemini-cli>

## 假设与边界

假设：

- `Claude Code` 官方 native installer 暴露的稳定用户级入口可按 `~/.local/bin/claude` 识别
- `Codex` 的稳定受管入口是 `npm install -g @openai/codex`
- `Gemini CLI` 的稳定受管入口是 `npm install -g @google/gemini-cli`
- 当前仓库对 provider 的 install / upgrade / uninstall 均通过命令字符串驱动，无需额外引入 provider 类或新模块
- `Claude Code native` fresh install 官方命令分平台不同，因此现有“单个字符串命令”模型需要最小扩展

边界：

- 不修改 `manage.sh` / `Makefile` 对外命令面
- 不实现自动迁移、自动卸载 `brew`、自动从 `npm` 切换到 `native`
- 不托管 `Claude Code local install`
- 不把本轮抽象扩展成通用“多维 lifecycle 框架”
- 不修改 skills / MCP / rules 主逻辑，除非 provider 语义变更引发现有测试失败

## Provider 模型设计

### 新字段

需要在 provider 定义中引入一个最小新字段：

- `managed`
  - `true`：agent-manager 可以 install / update / uninstall / repair
  - `false`：仅用于检测、审计、报错与迁移提示；不允许 lifecycle 操作。这里的 deprecated 指 `agent-manager` 策略弃用，不等同于上游官方一定已废弃该来源

需要在 agent 定义中引入一个最小新字段：

- `require_managed_provider`
  - `true`：若只检测到 deprecated provider 或 `system-only`，视为不合规状态，`doctor` 与 lifecycle 均 fail-close
  - `false`：保持现有宽松语义，允许外部安装存在但不托管

新增字段的原因：

- 只靠“provider 是否存在”无法区分“可托管的 npm / native”与“必须迁移掉的 brew”
- 不能把所有 `system-only` 都一刀切为错误，否则会误伤本来就是外部应用存在即可的 agent
- 新字段数量控制在 2 个，能满足本轮需求，不引入新的共享抽象层

### 命令值模型

当前 `install` / `upgrade` / `uninstall` 命令值是单字符串。为了准确表达 `Claude Code native` 的跨平台安装命令，需要允许命令值同时支持以下两种形式：

- 字符串
- 平台映射
  - `default`
  - `posix`
  - `darwin`
  - `linux`
  - `windows`

解析规则：

1. 若命令值是字符串，保持现状
2. 若命令值是平台映射，按 `darwin > linux > windows > posix > default` 的优先级选取当前平台命令
3. 若当前平台没有可用命令，fail-close 报错

这样可以避免为单个 agent 新增专用安装脚本，也避免把平台分支写死在 `sync_agents.py` 里。

### Claude Code 定义

`Claude Code` 调整为：

- `native`
  - `managed: true`
  - 默认 provider
  - `install`: 按平台分发的官方 native installer 命令
  - `upgrade`: `claude update`
  - `uninstall`: `claude uninstall`
  - `detect_paths`: `["~/.local/bin/claude"]`
- `npm`
  - `managed: true`
  - 非默认，保留 legacy-compatible 路径
  - `install`: `npm install -g @anthropic-ai/claude-code`
  - `upgrade`: `npm install -g @anthropic-ai/claude-code@latest`
- `brew`
  - `managed: false`
  - 仅用于识别历史 `brew install --cask claude-code`
  - 不执行 install / update / uninstall

同时开启：

- `require_managed_provider: true`

### Codex 定义

新增 `codex` agent：

- `display_name`: `Codex`
- `bin`: `codex`
- `require_managed_provider: true`

providers：

- `npm`
  - `managed: true`
  - 默认 provider
  - `install`: `npm install -g @openai/codex`
  - `upgrade`: `npm install -g @openai/codex@latest`
  - `uninstall`: `npm uninstall -g @openai/codex`
- `brew`
  - `managed: false`
  - 仅用于识别历史 brew 安装并阻止继续托管

### Gemini CLI 定义

`Gemini CLI` 调整为：

- `npm`
  - `managed: true`
  - 默认 provider
  - `install`: `npm install -g @google/gemini-cli`
  - `upgrade`: `npm install -g @google/gemini-cli@latest`
  - `uninstall`: `npm uninstall -g @google/gemini-cli`
- `brew`
  - `managed: false`
  - 仅用于识别历史 `brew install gemini-cli`

同时开启：

- `require_managed_provider: true`

## Runtime Probe 设计

需要修改以下入口：

1. [`src/manage_runtime_probe.py`](/Users/ly/agent-manager/src/manage_runtime_probe.py)
   - 保留现有 `brew`、`npm`、`gh`、`uv` 检测顺序
   - 新增 provider 级 `detect_paths` 支持
   - 当 `which(bin)` 或 `realpath(which(bin))` 命中 provider 的 `detect_paths` 时，将该 provider 记为命中
   - `detect_paths` 必须显式声明，fail-close，不做模糊推断
2. 需要新增共享命令解析辅助逻辑
   - 生命周期执行前先把 provider 命令值解析为当前平台对应的字符串
   - runtime commands 收集也必须复用同一套解析逻辑
3. [`tests/test_manage_runtime_probe.py`](/Users/ly/agent-manager/tests/test_manage_runtime_probe.py)
   - 新增 `Claude Code native` 路径识别
   - 新增 deprecated `brew` provider 仍可被识别
   - 新增 `detect_paths` 与 `system` fallback 的优先级用例

探测优先级要求：

1. 显式 managed / deprecated provider 识别优先
2. 只有在未识别到任何已声明 provider 时，才退回 `system`
3. `system` 仍代表“可执行入口存在，但 agent-manager 无法判定其来源”

这样可以避免把历史 `brew` 安装误报成普通 `system`，也避免把 Claude 的官方 native installer 误报成“外部安装”。

## Lifecycle 设计

需要修改以下入口：

1. [`agent-lifecycle/sync_agents.py`](/Users/ly/agent-manager/agent-lifecycle/sync_agents.py)
2. [`config/agents.json`](/Users/ly/agent-manager/config/agents.json)
3. [`config/manager.json`](/Users/ly/agent-manager/config/manager.json)

### 状态分类

当前状态分类是：

- `managed`
- `system_only`
- `unsupported`

本轮改为：

- `managed`
- `deprecated`
- `system_only`
- `unsupported`

分类规则：

- `provider in definition.providers` 且 `managed: true` => `managed`
- `provider in definition.providers` 且 `managed: false` => `deprecated`
- `provider == system` => `system_only`
- 其它 => `unsupported`

### install / update / uninstall 规则

#### install

- 未安装时：
  - 按默认 provider 安装
  - `Claude Code` 默认 `native`
  - `Codex` / `Gemini CLI` 默认 `npm`
- `Claude Code native.install` 必须使用平台解析后的官方命令
- 若当前检测到 `deprecated` provider：
  - 不自动覆盖安装
  - 直接失败并输出迁移提示

#### update

- 仅当当前存在 `managed` provider 时才执行升级
- 对 `deprecated` provider：
  - 直接失败
  - 输出需要先手工迁移的提示
- 对 `system_only` 且 `require_managed_provider: true`：
  - 直接失败
  - 输出“检测到未受管外部安装，agent-manager 不会继续更新”的提示

#### uninstall

- 仅对 `managed: true` 的 provider 执行
- 对 `managed: false` provider 不执行卸载
- 理由是 detect-only provider 代表历史或外部安装，agent-manager 不能越权接管

#### repair

- 仅当当前是 `managed` provider 且运行入口缺失时才执行 repair
- `deprecated` 与 `system_only` 不执行 repair

### 为什么不自动迁移

不自动迁移的原因：

- `brew -> npm/native` 迁移涉及用户 PATH、shell 缓存与既有全局环境
- `Claude Code npm -> native` 官方已有 `claude migrate-installer`，但这类操作不应由仓库在同步时静默触发
- 最稳妥的策略是 fail-close + 清晰迁移指引，而不是 silent mutation

## Doctor 设计

需要修改以下入口：

1. [`src/manage_doctor.py`](/Users/ly/agent-manager/src/manage_doctor.py)
2. [`tests/test_manage_doctor.py`](/Users/ly/agent-manager/tests/test_manage_doctor.py)

诊断规则：

- `managed` provider 且运行入口存在 => `ok`
- `managed` provider 但运行入口缺失 => `error`
- `deprecated` provider => `error`
  - 文案需明确为“检测到历史 / 已废弃安装来源，需迁移”
- `system_only` 且 `require_managed_provider: true` => `error`
  - 文案需明确为“检测到外部安装，未纳入 agent-manager 托管”
- `system_only` 且 `require_managed_provider: false` => 保持现有宽松语义

这样可以确保“放弃 brew”在诊断层是强约束，而不是仅仅改默认值。

## 运行时依赖收集设计

需要调整 [`src/manage_runtime_probe.py`](/Users/ly/agent-manager/src/manage_runtime_probe.py) 中的 `collect_required_runtime_commands(...)`：

- 只把当前策略会执行的 provider 命令头计入 required runtime commands
- `managed: false` 的 deprecated provider 不能再把 `brew` 纳入强依赖

预期效果：

- 如果用户只使用新的官方受管路径，`doctor` 不应再因为缺少 `brew` 而把它当成必须 runtime
- 历史 `brew` 安装仍能被识别，但 `brew` 不再是本轮目标状态所需的依赖

## 配置变更设计

### [`config/agents.json`](/Users/ly/agent-manager/config/agents.json)

需要：

1. 重写 `claude-code` provider 定义
2. 重写 `gemini-cli` provider 定义
3. 新增 `codex`
4. 为这三个 agent 增加 `require_managed_provider: true`

### [`config/manager.json`](/Users/ly/agent-manager/config/manager.json)

需要：

1. `claude-code.preferred_provider = native`
2. `gemini-cli.preferred_provider = npm`
3. 新增 `codex`
   - `enabled: true`
   - `auto_install: true`
   - `auto_update: true`
   - `preferred_provider: npm`

## 文档设计

需要更新以下文件：

- [`README.md`](/Users/ly/agent-manager/README.md)
- [`docs/TROUBLESHOOTING.md`](/Users/ly/agent-manager/docs/TROUBLESHOOTING.md)
- 如有必要，补充 [`docs/ARCHITECTURE.md`](/Users/ly/agent-manager/docs/ARCHITECTURE.md)

需要明确写清：

- 三者的官方受管路径分别是什么
- 为什么 `brew` 只保留识别、不再托管
- 为什么 `Claude Code local install` 不纳入 lifecycle
- 历史 `brew` 安装出现时，用户应该如何处理

建议文档中的迁移指引：

- `Claude Code`
  - `brew` 用户：先手工卸载 brew 版本，再让 agent-manager 走 native 安装
  - `npm` 用户：允许继续受管；若要切到 native，按官方 `claude migrate-installer` 路径手工迁移
- `Codex`
  - `brew` 用户：手工卸载 brew 版本，再执行 npm 安装
- `Gemini CLI`
  - `brew` 用户：手工卸载 brew 版本，再执行 npm 安装

## 失败模式与处理

1. `Claude Code native` 实际安装路径与文档不一致
   - `detect_paths` 设计必须 fail-close
   - 不做“看起来像 native 就算 native”的启发式推断
2. `Claude Code native` 安装命令在不同平台上发生变化
   - 平台命令解析必须以配置显式声明为准
   - 不在 Python 代码中硬编码远端安装命令字符串
3. 用户机器上同时存在 `brew` 和 `npm/native`
   - 继续报告多源冲突
   - 若其中包含 deprecated provider，则报错而不是静默成功
4. 用户只存在 `system` 入口
   - 对这三个 agent 报错并提示其来源未受管
5. 文档或官方推荐路径后续变化
   - provider 定义与迁移文案必须基于官方资料显式更新
   - 不能继续依赖旧的 README 记忆

## 回滚策略

1. 配置层回滚
   - 恢复 [`config/agents.json`](/Users/ly/agent-manager/config/agents.json) 与 [`config/manager.json`](/Users/ly/agent-manager/config/manager.json) 的旧 provider 语义
2. 逻辑层回滚
   - 恢复 `sync_agents.py`、`manage_runtime_probe.py`、`manage_doctor.py` 的旧状态分类
3. 行为层回滚
   - 若发现 `require_managed_provider` 过于严格，可只对单个 agent 关闭，而无需整体回退 provider 模型

## 验证方式

定向验证：

1. `python3 -m unittest tests.test_manage_runtime_probe -v`
2. `python3 -m unittest tests.test_agent_lifecycle -v`
3. `python3 -m unittest tests.test_manage_doctor -v`

更宽回归：

1. `python3 -m unittest discover -s tests -p 'test*.py' -v`

必要时补充：

1. `python3 -m unittest tests.test_project_contracts -v`

## 验收标准

- `Claude Code` 默认按 `native` 受管
- `Codex` 被纳入正式 lifecycle 管理，默认按 `npm` 受管
- `Gemini CLI` 默认按 `npm` stable 受管
- `brew` 不再是这三者的默认或推荐 provider
- 历史 `brew` 安装会被准确识别并阻止继续托管，而不是被误报成普通 `system`
- `doctor` 能清楚区分 `managed`、`deprecated`、`system_only`
- README 与故障排查文档与实现一致
- 不会因为本轮变更自动卸载或覆盖用户的现有安装
