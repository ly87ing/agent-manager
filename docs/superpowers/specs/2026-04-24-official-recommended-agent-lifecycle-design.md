# 全量 Agent 官方推荐生命周期管理设计

## 目标

将 `agent-manager` 的 Agent lifecycle 标准从“尽量原生”收口为“**按各 Agent 官方当前推荐方式维护**”，并满足以下要求：

- 不再把“是否 native installer”当成唯一正确标准
- 允许 `npm`、`uv` 等在上游官方文档中被明确推荐的维护路径继续作为合规默认 provider
- 对上游**未明确提供**的生命周期动作保持 `fail-close`，不伪造命令
- 保持现有 `manage.sh` / `Makefile` 主入口不变
- 不自动迁移、自动卸载或自动接管外部安装

## 设计结论

采用“**逐 Agent 跟随官方当前主推路径**”方案：

1. `Claude Code` 默认走官方 `native installer`
2. `Codex` 默认走官方 `npm`
3. `Gemini CLI` 默认走官方 `npm`
4. `Cursor CLI` 默认走官方 `native install script`
5. `OpenCode` 默认走官方 `native install script`
6. `Kimi Code CLI` 默认走官方推荐的组合路径
   - 前台安装：官方 install script
   - 更新 / 卸载：官方 `uv tool`
7. 对官方文档未明确给出的 lifecycle 动作显式建模为“当前不受管支持”，而不是补一个猜测命令

这意味着仓库的判断标准应从：

- “是否原生 installer”

调整为：

- “是否与该 Agent **当前官方推荐维护路径**一致”

## 官方事实基线

以下结论基于 `2026-04-24` 核对的官方资料：

- `Claude Code`
  - 官方主推 `native installer`
  - 官方文档同时提供 `update` 与 `uninstall`
- `Codex`
  - OpenAI 官方 Codex CLI 当前主推 `npm install -g @openai/codex`
- `Gemini CLI`
  - Google 官方 Gemini CLI 文档当前标准安装路径为 `npm install -g @google/gemini-cli`
- `Cursor CLI`
  - 官方当前主推 `cursor-agent` install script
  - 官方文档明确 `install` / `update`
  - 当前未看到同等明确的受管 `uninstall` 文档
- `OpenCode`
  - 官方当前“最简单/推荐”的安装路径是 install script
  - 官方文档明确 `upgrade` 与 `uninstall`
- `Kimi Code CLI`
  - 官方主推 install script
  - 官方维护动作仍通过 `uv tool` 表达

参考来源：

- <https://code.claude.com/docs/en/getting-started>
- <https://developers.openai.com/codex/cli>
- <https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/index.md>
- <https://docs.cursor.com/en/cli/installation>
- <https://opencode.ai/docs/>
- <https://www.kimi.com/code/docs/en/kimi-cli/guides/getting-started.html>

## 核心原则

### 1. 官方推荐优先，不做形式主义统一

`agent-manager` 不再要求所有 Agent 都长成同一种 provider 形态。

只要某个 provider 是该 Agent 在官方文档中当前主推的维护方式，它就应被视为默认合规 provider：

- `native installer`
- `install script`
- `npm`
- `uv tool`

### 2. 不伪造生命周期动作

如果官方文档没有给出足够明确、可稳定自动化的 lifecycle 动作，则：

- 不补猜测命令
- 不根据安装路径手搓删除逻辑
- 不因为“看起来能删”就把它纳入受管 `uninstall`

这样可以避免仓库把自己的假设冒充成官方标准。

### 3. fail-close 优于 silent fallback

如果某个默认 provider 缺少官方支持的动作：

- `doctor` / `status` 应能明确暴露该缺口
- 禁用 agent 时不应静默“假装卸载成功”
- 仓库需要把“官方未覆盖”与“配置缺失”区分开

## 生命周期矩阵

### `Claude Code`

- 默认 provider：`native`
- 合规原因：官方当前主推 native installer
- 受管动作：
  - `install`
  - `update`
  - `uninstall`
  - `repair`
- 兼容 provider：
  - `npm` 作为 legacy-compatible 受管 provider
- detect-only 历史来源：
  - `brew`

### `Codex`

- 默认 provider：`npm`
- 合规原因：OpenAI 官方当前主推 npm
- 受管动作：
  - `install`
  - `update`
  - `uninstall`
  - `repair`
- detect-only 历史来源：
  - `brew`

### `Gemini CLI`

- 默认 provider：`npm`
- 合规原因：Google 官方当前 standard install path 为 npm
- 受管动作：
  - `install`
  - `update`
  - `uninstall`
  - `repair`
- detect-only 历史来源：
  - `brew`

### `Cursor CLI`

- 默认 provider：`native`
- 合规原因：官方当前主推 `cursor-agent` install script
- 受管动作：
  - `install`
  - `update`
  - `repair`
- 当前未受管动作：
  - `uninstall`
- 原因：
  - 官方文档当前只明确了 install / update
  - 仓库不伪造 `uninstall`

### `OpenCode`

- 默认 provider：`native`
- 合规原因：官方当前推荐 install script
- 受管动作：
  - `install`
  - `update`
  - `uninstall`
  - `repair`
- detect-only 历史来源：
  - `npm`
  - `brew`

### `Kimi Code CLI`

- 默认 provider：`native`
- 合规原因：官方前台安装主推 install script
- 受管动作：
  - `install`：install script
  - `update`：`uv tool upgrade`
  - `uninstall`：`uv tool uninstall`
  - `repair`
- 说明：
  - 这是“官方推荐的组合路径”
  - 不是“纯原生 installer 全链路”

## 配置模型设计

### 继续保留的字段

- `preferred_provider`
- `managed`
- `require_managed_provider`

这些字段已经足够表达“官方推荐 provider”和“detect-only 历史来源”。

### 需要新增的最小字段

在 provider 定义中新增：

- `unsupported_actions`
  - 类型：字符串数组
  - 可选值：`install` / `update` / `uninstall` / `repair`

语义：

- 用于表达“这个 provider 当前是官方推荐路径，但某些动作官方未覆盖，因此 agent-manager 不支持自动执行”
- 与“命令缺失”不同，它是**显式策略声明**

示例：

- `Cursor CLI native`
  - `unsupported_actions: ["uninstall"]`

这样可以把以下两种情况分开：

1. 配置漏写了命令
2. 上游官方本来就没给这个动作

## Lifecycle 执行设计

涉及文件：

- [`agent-lifecycle/sync_agents.py`](/Users/ly/agent-manager/agent-lifecycle/sync_agents.py)
- [`config/agents.json`](/Users/ly/agent-manager/config/agents.json)

### 安装

保持现有逻辑：

- 若默认 provider 有 `install` 且当前动作未被标记为 unsupported，则执行
- 若官方未支持该动作，则 fail-close 报错并给出明确原因

### 更新

保持现有逻辑：

- 若 provider 有 `upgrade`，正常执行
- 若声明为 unsupported，则 fail-close

### 卸载

需要增强：

- 当前逻辑只区分“有无 `uninstall` 命令”
- 新逻辑应优先检查：
  1. 该动作是否在 `unsupported_actions`
  2. 若是，则报错：`官方当前未提供受管 uninstall 动作`
  3. 若不是，再检查是否真的漏配了 `uninstall`

预期效果：

- `Cursor CLI` 在禁用时不会报模糊的“缺少 uninstall 命令”
- 而是明确告诉用户：这是官方未覆盖，不是配置写漏

### 修复

保持现有 `repair -> upgrade -> install` 回退链，但增加约束：

- 如果 `repair` / `upgrade` / `install` 对应动作被声明为 unsupported，则该分支不能作为回退候选
- 若最终无可用动作，fail-close

## Doctor / Status 设计

涉及文件：

- [`src/manage_doctor.py`](/Users/ly/agent-manager/src/manage_doctor.py)

需要新增一种 agent 诊断状态：

- `official_gap`
  - 含义：当前默认 provider 是官方推荐路径，但某 lifecycle 动作官方未提供受管支持

最小落地方式：

- `doctor` 默认继续聚焦当前安装态
- 但当策略中某 agent 为 `enabled: false` 且当前 provider 命中一个 `unsupported_actions` 包含 `uninstall` 的默认 provider 时，报告中应明确标出：
  - 该 agent 当前无法按官方推荐路径自动卸载
  - 这是“官方动作缺口”，不是配置错误

如果不想在第一轮把 `doctor` 逻辑做得太重，也至少要在 lifecycle 执行报错时输出这个区分信息。

## 文档设计

涉及文件：

- [`README.md`](/Users/ly/agent-manager/README.md)
- [`docs/TROUBLESHOOTING.md`](/Users/ly/agent-manager/docs/TROUBLESHOOTING.md)

需要统一以下口径：

1. 仓库标准是“按官方当前推荐方式维护”
2. `Codex` / `Gemini CLI` 的 `npm` 是**官方合规默认路径**
3. `Kimi Code CLI` 的 “install script + uv tool” 是**官方合规组合路径**
4. `Cursor CLI` 当前官方只明确 install / update，仓库不伪造 uninstall
5. “不用 brew”不等于“所有 agent 都必须 native installer”

## 测试设计

需要补充或收紧的测试：

### `tests/test_agent_lifecycle.py`

- 锁定全量 agent 的默认 provider 矩阵
- 锁定 `unsupported_actions` 的契约
- 新增：
  - `Cursor CLI` 禁用时应因 `official unsupported uninstall` fail-close
  - `Codex` / `Gemini CLI` 的 `npm` 继续被视为官方推荐默认值
  - `Kimi` 的组合路径继续被视为合规默认值

### `tests/test_project_contracts.py`

- 锁定 README / TROUBLESHOOTING 的新口径
- 防止文档又回到“全部原生 installer”叙事

### `tests/test_manage_doctor.py`

- 如果本轮扩展 `doctor` 语义：
  - 锁定 `official_gap` 或等价文案输出

## 非目标

- 不修改 `manage.sh` / `Makefile` 对外命令面
- 不把 `Cursor CLI uninstall` 用非官方删除脚本硬补出来
- 不把 `Codex` / `Gemini CLI` 从官方 `npm` 路径强行改成伪 native
- 不自动迁移用户现有安装
- 不实现“跨 provider 自动切换”

## 风险与权衡

### 方案 A 的主要优点

- 与官方文档保持一致
- 不会为了“统一”而偏离上游
- 用户看到的策略说明更真实

### 主要代价

- lifecycle 能力在不同 agent 上不完全对称
- `Cursor CLI` 会保留一个明确的官方动作缺口

这个代价是可接受的，因为它反映的是**上游事实差异**，不是仓库设计缺陷。

## 验证计划

实现阶段至少需要验证：

1. `tests.test_agent_lifecycle`
2. `tests.test_project_contracts`
3. 若修改 `doctor`，运行 `tests.test_manage_doctor`
4. 若改动 README / config 影响入口预演，运行 `make smoke`

## 最终结论

后续实现必须围绕以下一句话推进：

> `agent-manager` 对 Agent lifecycle 的合规标准，是“按各 Agent 官方当前推荐方式维护”，而不是“所有 Agent 都必须表现为 native installer”。

这条结论优先于“形式上的统一”，也优先于“补齐看起来漂亮但并不官方的 lifecycle 动作”。
