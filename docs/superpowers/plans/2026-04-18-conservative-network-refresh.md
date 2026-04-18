# Conservative Network Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 tools / skills / mcp 在远端探测失败时默认跳过刷新，并提供 `--force-refresh` 显式强刷开关。

**Architecture:** 这次变更只改同步引擎和主入口透传，不引入新的配置层。默认路径改为“warning + skip”，显式 `--force-refresh` 恢复旧行为；对应测试覆盖 tool 升级、repo pull 和 manage.sh 参数透传。

**Tech Stack:** Python unittest, Bash, argparse

---

### Task 1: 固定默认跳过行为

**Files:**
- Modify: `tests/test_sync_tools.py`
- Modify: `tests/test_link_skills.py`
- Modify: `tests/test_sync_mcp.py`

- [ ] **Step 1: 写失败中的 tool 测试**

```python
def test_install_based_tool_probe_failure_skips_upgrade_by_default(self):
    ...
```

- [ ] **Step 2: 写失败中的 repo 测试**

```python
def test_clone_or_update_repo_probe_failure_skips_pull_by_default(self):
    ...
```

- [ ] **Step 3: 跑目标测试确认先失败**

Run: `python3 -m unittest tests.test_sync_tools.SyncToolsTests.test_install_based_tool_probe_failure_skips_upgrade_by_default tests.test_link_skills.LinkSkillsTests.test_clone_or_update_repo_probe_failure_skips_pull_by_default tests.test_sync_mcp.SyncMcpTests.test_clone_or_update_repo_probe_failure_skips_pull_by_default -v`

Expected: FAIL，当前默认仍会在 `unknown` 时执行升级或拉取

### Task 2: 保留显式强刷入口

**Files:**
- Modify: `tests/test_sync_tools.py`
- Modify: `tests/test_link_skills.py`
- Modify: `tests/test_sync_mcp.py`
- Modify: `tests/test_manage.py`

- [ ] **Step 1: 写 `force_refresh=True` 仍强刷的测试**

```python
def test_install_based_tool_probe_failure_force_refresh_runs_upgrade(self):
    ...
```

- [ ] **Step 2: 写 `manage.sh --force-refresh` 透传测试**

```python
def test_update_force_refresh_propagates_to_tools_skills_and_mcp(self):
    ...
```

### Task 3: 实现脚本与帮助文案

**Files:**
- Modify: `agent-tool/sync_tools.py`
- Modify: `agent-skill/link_skills.py`
- Modify: `agent-mcp/sync_mcp.py`
- Modify: `manage.sh`
- Modify: `docs/ARCHITECTURE.md`

- [ ] **Step 1: 增加 `force_refresh` 参数与 argparse 开关**
- [ ] **Step 2: 将默认 `unknown` 行为改为 warning + skip**
- [ ] **Step 3: 在 `--force-refresh` 下恢复旧的网络刷新路径**
- [ ] **Step 4: 更新帮助和架构说明**

### Task 4: 运行回归验证

**Files:**
- Verify: `tests/test_sync_tools.py`
- Verify: `tests/test_link_skills.py`
- Verify: `tests/test_sync_mcp.py`
- Verify: `tests/test_manage.py`

- [ ] **Step 1: 跑目标测试**

Run: `python3 -m unittest tests.test_sync_tools tests.test_link_skills tests.test_sync_mcp tests.test_manage -v`

Expected: 与本次改动相关的用例全部通过

- [ ] **Step 2: 复查 diff**

确认范围只包含同步脚本、主入口、文档和测试，没有额外行为改动
