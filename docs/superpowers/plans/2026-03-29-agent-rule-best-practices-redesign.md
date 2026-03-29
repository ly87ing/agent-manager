# Agent Rule Best-Practices Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将本仓库的 agent rules 治理升级为单一事实源、具备 Codex/Claude 原生 hooks、并由硬性 eval 门禁保护的最佳实践形态。

**Architecture:** 规则文本继续由 `agent-rule/rules/src/**` 生成到 `agent-rule/rules/dist/**`，仓库内不再保留第二条 rule text 流。平台原生 enforcement 通过 repo-managed hook definitions 生成到 Claude 与 Codex 各自消费的 artifact，offline eval 则升级为带 suite thresholds 的质量门。

**Tech Stack:** Python 3, Bash, JSON, TOML-aware tooling, unittest, Make

---

### Task 1: 收口 Legacy Rules Source

**Files:**
- Modify: `tests/test_sync_rules.py`
- Modify: `tests/test_manage.py`
- Modify: `src/manage_doctor.py`
- Delete: `agent-rule/rules/global/main.md`
- Delete: `agent-rule/rules/global/modules/architecture.md`
- Delete: `agent-rule/rules/global/modules/style.md`
- Delete: `agent-rule/rules/global/modules/frontend.md`
- Delete: `agent-rule/rules/global/modules/artifact-hygiene.md`

- [ ] **Step 1: 先写/改失败测试，要求所有 runtime fixture 只引用 `rules/dist/*`**

重点改动：
- `tests/test_sync_rules.py` 中目录同步、symlink、doctor fixture 全部从 `rules/global/*` 切到 `rules/dist/*`
- `tests/test_manage.py` 中 doctor / sync preview fixture 全部从 `rules/global/main.md` 切到 `rules/dist/claude/main.md`

- [ ] **Step 2: 运行最窄测试，确认旧实现下出现失败**

Run:
- `python3 -m unittest tests.test_sync_rules -v`
- `python3 -m unittest tests.test_manage.ManageScriptTests.test_doctor_warns_for_missing_generated_targets_when_agent_is_not_installed -v`

Expected:
- 旧的 legacy 路径断言或 fixture 会失败，提示需要切换 source

- [ ] **Step 3: 修改 doctor / tests，统一使用 `dist/*`**

实现要点：
- `src/manage_doctor.py` 继续从 `config/rules-targets.list` 读取 source，不再依赖 legacy tree
- 所有测试 fixture 用 `rules/dist/claude/main.md` 与 `rules/dist/modules`

- [ ] **Step 4: 删除仓库内 `agent-rule/rules/global/*`**

删除前确认：
- 没有运行时代码继续引用该目录
- 没有测试仍以该目录作为 fixture source

- [ ] **Step 5: 重新运行收口相关测试**

Run:
- `python3 -m unittest tests.test_rule_layout tests.test_sync_rules -v`

- [ ] **Step 6: 检查 `rg -n "rules/global" .` 只剩文档历史或明确兼容清理说明**

Run:
- `rg -n "rules/global" .`

Expected:
- 不再有生产代码和活动测试依赖 `agent-rule/rules/global/*`

### Task 2: 增加 Managed Codex Hooks

**Files:**
- Create: `config/codex-hooks.json`
- Create: `.codex/hooks.json`
- Create: `agent-rule/scripts/sync-codex-hooks.py`
- Create: `tests/test_sync_codex_hooks.py`
- Modify: `manage.sh`
- Modify: `Makefile`
- Modify: `tests/test_project_contracts.py`

- [ ] **Step 1: 写失败测试，锁定 Codex hooks sync 期望**

测试应覆盖：
- 从 `config/codex-hooks.json` 渲染 `.codex/hooks.json`
- committed `.codex/hooks.json` 与 fresh sync 一致
- managed hook 指向 `agent-rule/hooks/check-rules-build.sh`
- `make codex-hooks-sync` / `manage.sh codex:hooks:sync` 可用

- [ ] **Step 2: 跑窄测试，确认当前缺少实现**

Run:
- `python3 -m unittest tests.test_sync_codex_hooks -v`

Expected:
- FAIL，因为当前还没有 codex hook 定义、sync 脚本与 make/manage 入口

- [ ] **Step 3: 实现最小可审计 hook pipeline**

实现要点：
- `config/codex-hooks.json` 定义 repo-managed Codex `Stop` hook
- `.codex/hooks.json` 为 committed generated artifact
- `agent-rule/scripts/sync-codex-hooks.py` 负责 deterministic 输出
- hook command 复用 `agent-rule/hooks/check-rules-build.sh`

- [ ] **Step 4: 暴露命令入口**

实现：
- `manage.sh` 新增 `codex:hooks:sync`
- `Makefile` 新增 `codex-hooks-sync` 与 `codex\\:hooks\\:sync`

- [ ] **Step 5: 运行 Codex hook 相关测试**

Run:
- `python3 -m unittest tests.test_sync_codex_hooks tests.test_project_contracts -v`

### Task 3: 将 Eval 升级为硬性质量门

**Files:**
- Modify: `scripts/evals/fixtures/rule_effectiveness_cases.json`
- Modify: `scripts/evals/fixtures/rule_effectiveness_baseline.json`
- Modify: `scripts/evals/run_rule_effectiveness_eval.py`
- Modify: `tests/test_rule_effectiveness_eval.py`
- Modify: `tests/test_project_contracts.py`
- Modify: `Makefile`

- [ ] **Step 1: 扩写 eval fixture schema 测试**

新增或修改断言，要求：
- suite 顶层支持 requirements / thresholds
- case 数量存在最小门槛
- baseline 中所有 case 必须 success 且 verified
- runs 覆盖 required agents

- [ ] **Step 2: 扩充 cases 与 baseline**

要求：
- case 数量提升到能代表当前治理面的规模
- 至少覆盖 `codex` 与 `claude-code`
- baseline 全部为成功且已验证

- [ ] **Step 3: 修改 eval runtime，失败时返回非零**

实现：
- 校验 suite-level requirements
- 校验 aggregate success / verification thresholds
- 校验 required agent coverage

- [ ] **Step 4: 对齐 Makefile 质量门**

要求：
- `make instructions-check` 继续调用 eval
- 当 eval threshold 不达标时，整个命令失败

- [ ] **Step 5: 运行 eval 相关测试与脚本**

Run:
- `python3 -m unittest tests.test_rule_effectiveness_eval -v`
- `python3 scripts/evals/run_rule_effectiveness_eval.py`

Expected:
- 输出稳定 JSON
- 所有 cases success/verified

### Task 4: 文档与契约对齐

**Files:**
- Modify: `README.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `AGENTS.md`
- Modify: `agent-rule/AGENTS.md`
- Modify: `CLAUDE.md`
- Modify: `agent-rule/CLAUDE.md`
- Modify: `tests/test_project_contracts.py`

- [ ] **Step 1: 更新文档中的 source-of-truth 叙述**

明确：
- `src/**` 是唯一可编辑源
- `dist/**` 是唯一可分发产物
- 仓库内不再保留 `rules/global/*`

- [ ] **Step 2: 更新 Codex hooks 说明**

明确：
- `.codex/hooks.json` 是 repo-managed artifact
- `make codex-hooks-sync` 是刷新入口
- repo 不统一管理用户级 approval `.rules`

- [ ] **Step 3: 更新 instruction quality gate 说明**

明确：
- `eval-rules` 带 suite thresholds
- `instructions-check` 是硬门禁而不是摘要工具

- [ ] **Step 4: 跑文档契约测试**

Run:
- `python3 -m unittest tests.test_project_contracts -v`

### Task 5: 统一回归与清理

**Files:**
- Modify: `tests/test_rule_layout.py`
- Modify: `tests/test_sync_claude_hooks.py`
- Modify: `tests/test_sync_rules.py`
- Modify: `tests/test_rule_budgets.py`
- Modify: `tests/test_skill_exposure_contracts.py`
- Modify: `tests/test_manage.py`

- [ ] **Step 1: 运行指令治理相关 unittest 切片**

Run:
- `python3 -m unittest tests.test_rule_layout tests.test_rule_budgets tests.test_sync_rules tests.test_sync_claude_hooks tests.test_sync_codex_hooks tests.test_rule_effectiveness_eval tests.test_project_contracts -v`

- [ ] **Step 2: 运行统一质量门**

Run:
- `make instructions-check`

Expected:
- 全部通过
- eval 输出满足新的 suite thresholds

- [ ] **Step 3: 检查工作区与生成产物**

Run:
- `git status --short`

确认：
- 没有误改 unrelated files
- generated artifact 已与 source 对齐

- [ ] **Step 4: 记录未覆盖风险**

若有无法自动化验证的部分，在最终汇报里明确说明，例如：
- Codex hooks 的真实客户端行为只能以 fixture / schema / committed artifact 间接验证
