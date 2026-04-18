# OpenCode Default Install Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让仓库默认策略不再自动安装 OpenCode，同时保留受管定义与显式启用能力。

**Architecture:** 这次变更沿用 Cursor 的同型方案，只调整默认策略、README 说明和一条契约测试，不改动安装源定义或运行时分支逻辑。测试先固定 `config/manager.json` 的默认值，再让文档与实现保持一致。

**Tech Stack:** Python unittest, JSON config, Markdown

---

### Task 1: 固定 OpenCode 默认策略契约

**Files:**
- Modify: `tests/test_agent_lifecycle.py`

- [ ] **Step 1: 写一条失败中的默认策略测试**

```python
def test_committed_agent_policy_disables_default_opencode_auto_install(self):
    manager = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

    self.assertFalse(manager["agents"]["opencode"]["auto_install"])
```

- [ ] **Step 2: 运行目标测试确认它先失败**

Run: `python3 -m unittest tests.test_agent_lifecycle.CommittedAgentPolicyTests.test_committed_agent_policy_disables_default_opencode_auto_install -v`

Expected: FAIL，断言 `opencode.auto_install` 当前仍为 `true`

### Task 2: 调整默认策略与说明

**Files:**
- Modify: `config/manager.json`
- Modify: `README.md`

- [ ] **Step 1: 调整 OpenCode 默认策略**

```json
"opencode": { "enabled": true, "auto_install": false, "auto_update": true, "preferred_provider": "brew" }
```

- [ ] **Step 2: 补充 README 默认自动安装说明**

```md
默认自动安装的 Agent 当前包含 `Claude Code` 和 `Gemini CLI`；`Cursor IDE` 与 `OpenCode` 仍受管，但默认不自动安装，需要时可通过策略显式开启。
```

### Task 3: 运行相关验证

**Files:**
- Verify: `tests/test_agent_lifecycle.py`
- Verify: `tests/test_project_contracts.py`

- [ ] **Step 1: 重新运行目标测试**

Run: `python3 -m unittest tests.test_agent_lifecycle.CommittedAgentPolicyTests.test_committed_agent_policy_disables_default_opencode_auto_install -v`

Expected: PASS

- [ ] **Step 2: 运行相关模块测试**

Run: `python3 -m unittest tests.test_agent_lifecycle tests.test_project_contracts -v`

Expected: PASS

- [ ] **Step 3: 复查变更范围**

确认 diff 仅包含默认策略、README 说明、测试和本次设计/计划文档，没有额外行为改动。
