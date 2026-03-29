# Agent Rule Best-Practices Redesign

## Goal

将当前仓库的 rules / hooks / eval 治理从“工程上已经整洁可用”提升到“接近 Anthropic Engineering 与 OpenAI Codex 官方推荐的最佳实践”：

- `agent-rule/rules/src/` 成为唯一可编辑 truth source
- `agent-rule/rules/dist/` 成为唯一可分发 artifact source
- Claude 与 Codex 都具备 repo-managed、可回归验证的原生 guardrail artifact
- `make instructions-check` 从示意级回归门升级为真正的 instruction quality gate

## Current State

当前仓库已经具备一组很强的基础设施：

- layered rule source 已存在：`agent-rule/rules/src/shared/**` 与 `agent-rule/rules/src/agents/**`
- generated distribution 已存在：`agent-rule/rules/dist/**`
- `build-agent-rules.py` 可 deterministic 生成 `dist/`
- `config/rules-targets.list` 已分发 `dist/*` 到各 agent 目标路径
- Claude 侧已有 managed hooks、sync 脚本和完整测试
- `make instructions-check` 已聚合 budgets、layout、sync、hooks 与 offline eval

但距离“最佳实践完成态”还有三类明显差距：

### 1. 仓库内仍存在双轨 source

虽然主流程已经切到 `rules/dist/*`，但仓库里仍保留 `agent-rule/rules/global/*`，且部分测试 fixture 仍依赖这些 legacy 路径。这会带来两个问题：

- 读者无法快速判断真正的 source-of-truth
- 后续修改容易出现 `src -> dist` 与 `global/*` 双向漂移

### 2. 平台原生治理不对称

Claude 已有 deterministic hook enforcement，但 Codex 侧目前主要依赖 `AGENTS.md` 文本约束，缺少 repo-managed native hook artifact。这样会导致：

- Claude 与 Codex 的 enforcement 不一致
- `rules/src` 改动后，Codex 会话缺少与 Claude 类似的自动继续/阻止机制

### 3. eval 仍是骨架，不是质量门

当前 offline eval 能输出稳定 JSON 摘要，但仍是“轻量样例回归”：

- case 数量过少
- baseline 允许失败样本存在
- 缺少 suite 级 coverage / threshold 契约

这与 Anthropic 强调的 evaluation-driven iteration、held-out regression 和 explicit quality bar 仍有明显差距。

## Design Principles

### 1. Single source of truth 必须收口

仓库内对 rule text 的唯一语义源应当是：

- `agent-rule/rules/src/**`

仓库内对 rule artifact 的唯一分发源应当是：

- `agent-rule/rules/dist/**`

任何 legacy 镜像路径如果继续存在，都必须是由 build 产物生成并受一致性校验；否则应删除。

### 2. 把 advisory instruction 和 deterministic enforcement 分层

最佳实践不是把所有约束都塞进 prompt 文本，而是：

- 高频、跨任务、需长期遵守的行为进入 `AGENTS.md` / shared core
- 低频、任务触发型细节进入 task modules
- “必须零例外”的动作进入 native hooks

### 3. 只管理可共享、可审计的 Codex 原生产物

Codex 的 repo-level best practice 不等于把所有个人环境都仓库化。这里应明确区分：

- 应受管：repo-native `AGENTS.md`、repo-native `hooks.json`
- 不应在本仓库强制受管：用户级 approval `.rules` allowlist

原因是 approval allowlist 强烈依赖本机环境、团队信任边界和个人习惯，不适合作为项目内统一政策。

### 4. eval 需要有“失败成本”

如果 eval 可以在存在已知失败样本时继续通过，它就只是监控，不是质量门。这里的目标是：

- baseline 必须全部成功
- verification 不能低于门槛
- case coverage 不足时直接失败

## Scope

本次改造包含四个工作流。

### Workstream A: Rules source 收口

- 移除仓库内对 `agent-rule/rules/global/*` 的运行时依赖
- 将测试、doctor 审计与 manifest fixture 统一切到 `agent-rule/rules/dist/*`
- 保留“用户家目录 legacy Codex global path 清理”，因为这是向后兼容，不是仓库 source

### Workstream B: Codex native hooks

- 新增 repo-managed Codex hooks definition，例如 `config/codex-hooks.json`
- 新增 sync 脚本，把 definition 渲染到仓库级 `.codex/hooks.json`
- 至少接入一个 deterministic `Stop` hook：当 `rules/src` 与 `rules/dist` 不一致时，阻止自然结束并要求继续处理
- 为 Codex hooks 增加 committed artifact 与 fresh sync 一致性测试

### Workstream C: Rule effectiveness gate 升级

- 扩展 `rule_effectiveness_cases.json`
- baseline runs 全部切为 `success=true` 与 `verified=true`
- 在 eval schema 中加入 suite-level requirements，例如：
  - 最小 case 数
  - 必须覆盖 `codex` 与 `claude-code`
  - 最小 success / verification rate
- 让 `make instructions-check` 对这些契约失败时直接报错

### Workstream D: Docs / contracts 对齐

- 更新 `README.md`
- 更新 `docs/ARCHITECTURE.md`
- 更新相关 project contracts
- 补充 Codex hook sync 的 Make / manage 入口说明

## Proposed Architecture

### 1. Rule text pipeline

保留现有 pipeline，并显式声明唯一数据流：

`src/shared + src/agents -> build-agent-rules.py -> dist/* -> rules-targets.list -> user/agent targets`

仓库内不再保留第二条平行的规则文本流。

### 2. Native hook pipeline

新增 Codex hook pipeline，并与 Claude hook pipeline 保持结构一致：

- source definition: `config/codex-hooks.json`
- renderer: `agent-rule/scripts/sync-codex-hooks.py`
- committed artifact: `.codex/hooks.json`
- runtime hook script: 复用或包装 `agent-rule/hooks/check-rules-build.sh`

这个设计的关键点是“definition file + generated artifact + committed snapshot + tests”四件套一起存在，避免手改 `.codex/hooks.json`。

### 3. Eval pipeline

offline eval 仍保持 fixture-driven，避免引入不稳定外部依赖，但要升级为带 suite contract 的 gate：

- case schema 负责定义任务覆盖面
- baseline schema 负责定义每个 case 的受期望结果
- eval runtime 负责校验：
  - fixtures schema 正确
  - runs 与 cases 一一对应
  - required agent coverage 存在
  - aggregate thresholds 达标

## Non-Goals

本次改造明确不做以下事情：

- 不在仓库内统一管理用户级 Codex approval `.rules`
- 不把 Claude 的 PreToolUse file-write enforcement 强行映射到 Codex，因为 Codex hooks 目前主要面向 `Bash` / `Stop` 事件，能力边界不同
- 不引入联网或 live agent eval；本次仍坚持 deterministic offline fixture 模式
- 不对现有 rule 文案做大规模风格重写；重点在治理架构与质量门

## Risks And Mitigations

### Risk 1: 删除 legacy `rules/global/*` 触发测试大面积漂移

Mitigation:

- 先改测试和 doctor fixture，再删除 legacy 路径
- 通过 `test_rule_layout`、`test_sync_rules`、`test_manage` 做分层回归

### Risk 2: Codex hooks 语义与 Claude hooks 不完全一致

Mitigation:

- 只落地 Codex 官方文档明确支持的 `Stop` continuation 模式
- 避免承诺 Codex 当前做不到的 file-write denial

### Risk 3: eval 门槛过高导致当前基线直接红

Mitigation:

- 同一提交中同步扩充 cases、更新 baseline、修改测试断言
- 所有质量门以 committed fixtures 为准，避免引入外部抖动

## Success Criteria

改造完成后，必须同时满足以下条件：

1. 仓库内 rule source 只剩 `src/**`，artifact 只剩 `dist/**`；不再有 legacy `rules/global/*` 依赖。
2. Codex 与 Claude 都有 repo-managed native hook artifact，并有 fresh-sync 一致性测试。
3. `make instructions-check` 在本仓库中成为硬门禁；任何 rule quality regression 会导致失败。
4. `rule_effectiveness` baseline 不再包含失败样本。
5. docs 与 contracts 明确描述新的治理边界，且断言与实现一致。

## Verification Strategy

至少运行以下验证：

- `python3 -m unittest tests.test_rule_layout -v`
- `python3 -m unittest tests.test_sync_rules -v`
- `python3 -m unittest tests.test_sync_claude_hooks -v`
- `python3 -m unittest tests.test_sync_codex_hooks -v`
- `python3 -m unittest tests.test_rule_effectiveness_eval -v`
- `python3 -m unittest tests.test_project_contracts -v`
- `make instructions-check`

## Rollout Order

推荐实施顺序：

1. 先补测试，锁定期望行为
2. 收口 legacy `rules/global/*`
3. 加入 Codex hooks pipeline
4. 升级 eval schema 与 baseline
5. 更新文档与统一回归

这个顺序可以尽早把 source-of-truth 问题收口，并把 Codex hooks 与 eval quality gate 放进同一轮验证里。
