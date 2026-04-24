# Official Recommended Agent Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align agent lifecycle management to each agent's official current recommended maintenance path, explicitly model unsupported official lifecycle actions, and expose that distinction in lifecycle behavior, doctor output, and docs.

**Architecture:** Keep the existing provider model and CLI entrypoints unchanged. Add a minimal provider-level `unsupported_actions` contract, enforce it in `agent-lifecycle/sync_agents.py`, and surface the same fact in `src/manage_doctor.py` as an `official_gap` warning instead of inventing unsupported commands. Lock the policy with contract tests and doc wording that defines compliance as "official recommended path" rather than "all native installers".

**Tech Stack:** Python 3, `unittest`, JSON config, Markdown docs, existing `manage.sh` / `make` entrypoints.

---

## Assumptions And Guardrails

- `unsupported_actions` stores logical lifecycle verbs: `install`, `update`, `uninstall`, `repair`.
- First rollout only needs one committed gap declaration: `config/agents.json -> cursor -> native -> unsupported_actions: ["uninstall"]`.
- `doctor` should surface official lifecycle gaps as `warning` entries with `category="official_gap"`. If maintainers later want `error`, keep the same detection hook and only change the asserted severity/message.
- Do not revert unrelated existing hunks in `README.md`, `config/agents.json`, `docs/TROUBLESHOOTING.md`, `tests/test_agent_lifecycle.py`, or `tests/test_project_contracts.py`. If execution does not happen in a clean worktree, preserve unrelated hunks before staging commits.

## File Structure

- Modify: `config/agents.json`
  Responsible for the committed provider matrix, `preferred_provider`, `managed`, `require_managed_provider`, and the new `unsupported_actions` contract.
- Modify: `agent-lifecycle/sync_agents.py`
  Responsible for lifecycle reconciliation (`install`, `update`, `uninstall`, `repair`) and for distinguishing "officially unsupported action" from "missing command".
- Modify: `src/manage_doctor.py`
  Responsible for report assembly, warning categories, and agent-level diagnostics such as `official_gap`.
- Modify: `tests/test_agent_lifecycle.py`
  Responsible for committed provider-matrix assertions and lifecycle fail-close behavior.
- Modify: `tests/test_manage_doctor.py`
  Responsible for `doctor` report expectations around disabled agents and official lifecycle gaps.
- Modify: `tests/test_project_contracts.py`
  Responsible for locking README / TROUBLESHOOTING wording so the repo does not drift back to "everything native".
- Modify: `README.md`
  Responsible for the top-level policy statement and the official lifecycle matrix shown to operators.
- Modify: `docs/TROUBLESHOOTING.md`
  Responsible for operator guidance when `doctor` or lifecycle encounters official gaps such as `Cursor CLI` uninstall.

### Task 1: Lock The Official Provider Matrix Contract

**Files:**
- Modify: `tests/test_agent_lifecycle.py`
- Modify: `config/agents.json`

- [ ] **Step 1: Write the failing config contract test**

```python
class CommittedAgentPolicyTests(unittest.TestCase):
    def test_committed_agent_policy_locks_officially_supported_actions_matrix(self):
        lifecycle_agents = json.loads((ROOT / "config" / "agents.json").read_text(encoding="utf-8"))

        cursor_native = lifecycle_agents["cursor"]["providers"]["native"]
        self.assertEqual(cursor_native["unsupported_actions"], ["uninstall"])

        self.assertEqual(lifecycle_agents["codex"]["preferred_provider"], "npm")
        self.assertEqual(lifecycle_agents["gemini-cli"]["preferred_provider"], "npm")
        self.assertEqual(
            lifecycle_agents["kimi-cli"]["providers"]["native"]["uninstall"],
            "uv tool uninstall kimi-cli",
        )
```

- [ ] **Step 2: Run the targeted test and verify it fails**

Run: `python3 -m unittest tests.test_agent_lifecycle.CommittedAgentPolicyTests.test_committed_agent_policy_locks_officially_supported_actions_matrix -v`

Expected: FAIL with `KeyError: 'unsupported_actions'` or an assertion failure showing that `cursor.native` does not yet declare the official uninstall gap.

- [ ] **Step 3: Add the minimal config implementation**

```json
{
  "cursor": {
    "display_name": "Cursor CLI",
    "bin": "cursor-agent",
    "preferred_provider": "native",
    "require_managed_provider": true,
    "providers": {
      "native": {
        "managed": true,
        "install": {
          "darwin": "curl https://cursor.com/install -fsS | bash",
          "linux": "curl https://cursor.com/install -fsS | bash"
        },
        "upgrade": "cursor-agent update",
        "unsupported_actions": ["uninstall"],
        "detect_paths": ["~/.local/bin/cursor-agent", "~/.cursor/bin/cursor-agent"]
      }
    }
  }
}
```

- [ ] **Step 4: Run the targeted test and verify it passes**

Run: `python3 -m unittest tests.test_agent_lifecycle.CommittedAgentPolicyTests.test_committed_agent_policy_locks_officially_supported_actions_matrix -v`

Expected: PASS and confirm the committed matrix still keeps `Codex` / `Gemini CLI` on `npm` and `Kimi Code CLI` on the official `install script + uv tool` path.

- [ ] **Step 5: Commit**

```bash
git add config/agents.json tests/test_agent_lifecycle.py
git commit -m "test: lock official provider matrix contract"
```

### Task 2: Enforce Officially Unsupported Actions In Lifecycle Reconciliation

**Files:**
- Modify: `tests/test_agent_lifecycle.py`
- Modify: `agent-lifecycle/sync_agents.py`

- [ ] **Step 1: Write the failing lifecycle behavior tests**

```python
class ReconcileAgentTests(unittest.TestCase):
    def test_install_reports_official_gap_for_unsupported_install(self):
        definition = {
            "display_name": "Cursor CLI",
            "bin": "cursor-agent",
            "providers": {
                "native": {
                    "managed": True,
                    "install": "cursor-agent install",
                    "unsupported_actions": ["install"],
                }
            },
        }
        policy = {"enabled": True, "auto_install": True, "preferred_provider": "native"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=[]),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="ERROR") as logs,
        ):
            result = sync_agents.reconcile_agent("cursor", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_not_called()
        self.assertTrue(any("官方当前未提供受管 install 动作" in line for line in logs.output), logs.output)

    def test_update_reports_official_gap_for_unsupported_update(self):
        definition = {
            "display_name": "Cursor CLI",
            "bin": "cursor-agent",
            "providers": {
                "native": {
                    "managed": True,
                    "upgrade": "cursor-agent update",
                    "unsupported_actions": ["update"],
                }
            },
        }
        policy = {"enabled": True, "auto_update": True, "preferred_provider": "native"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=["native"]),
            mock.patch.object(sync_agents, "is_agent_installed", return_value=True),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="ERROR") as logs,
        ):
            result = sync_agents.reconcile_agent("cursor", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_not_called()
        self.assertTrue(any("官方当前未提供受管 update 动作" in line for line in logs.output), logs.output)

    def test_uninstall_reports_official_gap_for_unsupported_uninstall(self):
        definition = {
            "display_name": "Cursor CLI",
            "bin": "cursor-agent",
            "providers": {
                "native": {
                    "managed": True,
                    "upgrade": "cursor-agent update",
                    "unsupported_actions": ["uninstall"],
                }
            },
        }
        policy = {"enabled": False, "preferred_provider": "native"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=["native"]),
            mock.patch.object(sync_agents, "is_agent_installed", return_value=True),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="ERROR") as logs,
        ):
            result = sync_agents.reconcile_agent("cursor", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_not_called()
        self.assertTrue(any("官方当前未提供受管 uninstall 动作" in line for line in logs.output), logs.output)

    def test_repair_fails_when_all_repair_paths_are_officially_unsupported(self):
        definition = {
            "display_name": "Cursor CLI",
            "bin": "cursor-agent",
            "providers": {
                "native": {
                    "managed": True,
                    "install": "cursor-agent install",
                    "upgrade": "cursor-agent update",
                    "unsupported_actions": ["repair", "update", "install"],
                }
            },
        }
        policy = {"enabled": True, "auto_update": True, "preferred_provider": "native"}

        with (
            mock.patch.object(sync_agents, "detect_all_providers", return_value=["native"]),
            mock.patch.object(sync_agents, "is_agent_installed", return_value=False),
            mock.patch.object(sync_agents, "run_cmd") as run_cmd,
            self.assertLogs(level="ERROR") as logs,
        ):
            result = sync_agents.reconcile_agent("cursor", definition, policy)

        self.assertFalse(result)
        run_cmd.assert_not_called()
        self.assertTrue(
            any("官方当前未提供受管 repair / update / install 动作" in line for line in logs.output),
            logs.output,
        )
```

- [ ] **Step 2: Run the new lifecycle tests and verify they fail**

Run: `python3 -m unittest tests.test_agent_lifecycle.ReconcileAgentTests.test_install_reports_official_gap_for_unsupported_install tests.test_agent_lifecycle.ReconcileAgentTests.test_update_reports_official_gap_for_unsupported_update tests.test_agent_lifecycle.ReconcileAgentTests.test_uninstall_reports_official_gap_for_unsupported_uninstall tests.test_agent_lifecycle.ReconcileAgentTests.test_repair_fails_when_all_repair_paths_are_officially_unsupported -v`

Expected: FAIL because `sync_agents.reconcile_agent()` still treats these states as generic missing-command cases and does not yet distinguish official gaps.

- [ ] **Step 3: Implement minimal unsupported-action helpers and branch checks**

```python
SUPPORTED_UNSUPPORTED_ACTIONS = {"install", "update", "uninstall", "repair"}


def get_unsupported_actions(provider_cfg):
    raw = provider_cfg.get("unsupported_actions", []) if provider_cfg else []
    if not isinstance(raw, list):
        return set()
    return {
        action
        for action in raw
        if isinstance(action, str) and action in SUPPORTED_UNSUPPORTED_ACTIONS
    }


def action_is_officially_unsupported(provider_cfg, action):
    return action in get_unsupported_actions(provider_cfg)


def log_official_gap(label, provider_name, action):
    logging.error(
        f"[AGENT: {label}] Provider '{provider_name}' 的官方推荐路径当前未提供受管 {action} 动作。"
    )
```

```python
def get_repair_cmd(provider_name, provider_cfg):
    if not provider_cfg:
        return None
    if not action_is_officially_unsupported(provider_cfg, "repair"):
        explicit = resolve_provider_command(provider_cfg.get("repair"))
        if explicit:
            return explicit

    install_cmd = None
    if not action_is_officially_unsupported(provider_cfg, "install"):
        install_cmd = resolve_provider_command(provider_cfg.get("install"))
        if provider_name.startswith("brew"):
            pkg_name = get_pkg_name(install_cmd or "")
            if pkg_name:
                if provider_name == "brew-cask":
                    return f"brew reinstall --cask {pkg_name}"
                return f"brew reinstall {pkg_name}"

    if not action_is_officially_unsupported(provider_cfg, "update"):
        upgrade_cmd = resolve_provider_command(provider_cfg.get("upgrade"))
        if upgrade_cmd:
            return upgrade_cmd
    return install_cmd
```

```python
        if action_is_officially_unsupported(p_cfg, "install"):
            log_official_gap(label, preferred, "install")
            return False
        install_cmd = resolve_provider_command(p_cfg.get("install")) if p_cfg else None

        if action_is_officially_unsupported(p_cfg, "uninstall"):
            log_official_gap(label, provider, "uninstall")
            all_ok = False
            continue

        if action_is_officially_unsupported(p_cfg, "update"):
            log_official_gap(label, active_provider, "update")
            return False

        if not repair_cmd:
            if {"repair", "update", "install"}.issubset(get_unsupported_actions(p_cfg)):
                logging.error(
                    f"[AGENT: {label}] Provider '{active_provider}' 的官方推荐路径当前未提供受管 repair / update / install 动作，无法修复。"
                )
            else:
                logging.error(
                    f"[AGENT: {label}] Provider '{active_provider}' 缺少 repair/install/upgrade 命令，无法修复。"
                )
            return False
```

- [ ] **Step 4: Run the new lifecycle tests and the broader lifecycle suite**

Run: `python3 -m unittest tests.test_agent_lifecycle.ReconcileAgentTests.test_install_reports_official_gap_for_unsupported_install tests.test_agent_lifecycle.ReconcileAgentTests.test_update_reports_official_gap_for_unsupported_update tests.test_agent_lifecycle.ReconcileAgentTests.test_uninstall_reports_official_gap_for_unsupported_uninstall tests.test_agent_lifecycle.ReconcileAgentTests.test_repair_fails_when_all_repair_paths_are_officially_unsupported -v`

Expected: PASS and confirm `run_cmd()` is never called for unsupported lifecycle actions.

Run: `python3 -m unittest tests.test_agent_lifecycle -v`

Expected: PASS and confirm the pre-existing install/update/uninstall/repair behavior still holds for supported providers.

- [ ] **Step 5: Commit**

```bash
git add agent-lifecycle/sync_agents.py tests/test_agent_lifecycle.py
git commit -m "feat: enforce official lifecycle action gaps"
```

### Task 3: Surface Official Gaps In Doctor Output

**Files:**
- Modify: `tests/test_manage_doctor.py`
- Modify: `src/manage_doctor.py`

- [ ] **Step 1: Write the failing doctor test**

```python
class ManageDoctorReportTests(unittest.TestCase):
    def test_report_flags_official_gap_for_disabled_cursor_without_managed_uninstall(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {\n    "cursor": { "enabled": false, "auto_install": false, "auto_update": true, "preferred_provider": "native" }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "cursor": {\n    "display_name": "Cursor CLI",\n    "bin": "cursor-agent",\n    "preferred_provider": "native",\n    "providers": {\n      "native": {\n        "managed": true,\n        "upgrade": "cursor-agent update",\n        "unsupported_actions": ["uninstall"]\n      }\n    }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text('{"agent_paths": {}, "servers": {}}', encoding="utf-8")
            (temp_root / "config" / "skills.json").write_text('{"repositories": {}, "agents": {}}', encoding="utf-8")
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            with (
                mock.patch("src.manage_doctor.detect_all_providers", return_value=["native"]),
                mock.patch("src.manage_doctor.is_agent_installed", return_value=True),
            ):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

        agent_names = {entry["name"]: entry for entry in report["sections"]["agents"]}
        self.assertEqual(report["status"], "warning")
        self.assertEqual(agent_names["Cursor CLI"]["status"], "warning")
        self.assertEqual(agent_names["Cursor CLI"]["category"], "official_gap")
        self.assertIn("官方当前未提供受管 uninstall 动作", agent_names["Cursor CLI"]["message"])
        self.assertEqual(report["summary"]["warning_categories"]["official_gaps"], 1)
```

- [ ] **Step 2: Run the targeted doctor test and verify it fails**

Run: `python3 -m unittest tests.test_manage_doctor.ManageDoctorReportTests.test_report_flags_official_gap_for_disabled_cursor_without_managed_uninstall -v`

Expected: FAIL because `build_report()` still collapses this state into a generic drift/error path and does not emit `official_gap`.

- [ ] **Step 3: Implement the minimal doctor warning/category support**

```python
def make_entry(name: str, status: str, message: str, *, category: str | None = None) -> dict[str, str]:
    entry = {"name": name, "status": status, "message": message}
    if category:
        entry["category"] = category
    return entry


def append_entry(report: dict, section: str, entry: dict) -> None:
    report["sections"].setdefault(section, []).append(entry)
    if entry["status"] == "error":
        report["summary"]["errors"] += 1
    elif entry["status"] == "warning":
        report["summary"]["warnings"] += 1
        category = entry.get("category")
        if category == "optional_tool":
            report["summary"]["warning_categories"]["optional_tools"] += 1
        elif category == "unmaterialized":
            report["summary"]["warning_categories"]["unmaterialized"] += 1
        elif category == "official_gap":
            report["summary"]["warning_categories"]["official_gaps"] += 1
```

```python
        "summary": {
            "errors": 0,
            "warnings": 0,
            "warning_categories": {
                "optional_tools": 0,
                "unmaterialized": 0,
                "official_gaps": 0,
            },
        },
```

```python
        preferred_cfg = definition.get("providers", {}).get(preferred, {})
        preferred_gap_actions = {
            action
            for action in preferred_cfg.get("unsupported_actions", [])
            if isinstance(action, str)
        }
        preferred_is_active = any(
            canonical_provider_name(provider) == canonical_provider_name(preferred)
            for provider in state["managed"]
        )

        if (
            not desired
            and preferred_is_active
            and "uninstall" in preferred_gap_actions
        ):
            append_entry(
                report,
                "agents",
                make_entry(
                    label,
                    "warning",
                    "官方当前未提供受管 uninstall 动作；agent-manager 不会伪造删除命令",
                    category="official_gap",
                ),
            )
        elif is_active == desired:
            ...
```

```python
    categories = report["summary"]["warning_categories"]
    if report["summary"]["warnings"] > 0 and (
        categories["optional_tools"] > 0
        or categories["unmaterialized"] > 0
        or categories["official_gaps"] > 0
    ):
        lines.append("")
        lines.append("[ 提示项分类 ]")
        if categories["optional_tools"] > 0:
            lines.append(f"  ⚠ 可选工具未就绪: {categories['optional_tools']} 项")
        if categories["unmaterialized"] > 0:
            lines.append(f"  ⚠ 托管产物尚未生成: {categories['unmaterialized']} 项")
        if categories["official_gaps"] > 0:
            lines.append(f"  ⚠ 官方生命周期动作缺口: {categories['official_gaps']} 项")
```

- [ ] **Step 4: Run the targeted doctor test and the broader doctor suite**

Run: `python3 -m unittest tests.test_manage_doctor.ManageDoctorReportTests.test_report_flags_official_gap_for_disabled_cursor_without_managed_uninstall -v`

Expected: PASS and show `warning` severity plus `official_gap` category.

Run: `python3 -m unittest tests.test_manage_doctor -v`

Expected: PASS and confirm the new warning category does not regress existing `optional_tool` / `unmaterialized` behavior.

- [ ] **Step 5: Commit**

```bash
git add src/manage_doctor.py tests/test_manage_doctor.py
git commit -m "feat: surface official lifecycle gaps in doctor"
```

### Task 4: Lock Documentation Contracts And Update Operator Docs

**Files:**
- Modify: `tests/test_project_contracts.py`
- Modify: `README.md`
- Modify: `docs/TROUBLESHOOTING.md`

- [ ] **Step 1: Write the failing documentation contract assertions**

```python
def test_readme_documents_official_cli_lifecycle_policy(self):
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    self.assertIn("按各 Agent 官方当前推荐方式维护", readme)
    self.assertIn("`Codex` / `Gemini CLI` 的 `npm` 是官方合规默认路径", readme)
    self.assertIn("`Kimi Code CLI` 的 `install script + uv tool` 是官方合规组合路径", readme)
    self.assertIn("放弃 `brew` 不等于所有 agent 都必须 native installer", readme)


def test_troubleshooting_documents_cli_provider_migration_paths(self):
    troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
    self.assertIn("官方动作缺口", troubleshooting)
    self.assertIn("不是所有 agent 都必须 native installer", troubleshooting)
    self.assertIn("`Kimi Code CLI`：官方 install script + `uv tool`", troubleshooting)
```

- [ ] **Step 2: Run the targeted contract tests and verify they fail**

Run: `python3 -m unittest tests.test_project_contracts.ProjectContractTests.test_readme_documents_official_cli_lifecycle_policy tests.test_project_contracts.ProjectContractTests.test_troubleshooting_documents_cli_provider_migration_paths -v`

Expected: FAIL because README / TROUBLESHOOTING still describe pieces of the policy but do not yet state the full "official recommended path" rule explicitly enough.

- [ ] **Step 3: Update the user-facing docs with the new policy wording**

```markdown
仓库的 Agent lifecycle 合规标准是“按各 Agent 官方当前推荐方式维护”，不是“所有 agent 都必须 native installer”。
因此 `Codex` / `Gemini CLI` 的 `npm`、`Kimi Code CLI` 的 `install script + uv tool` 都属于官方合规默认路径；放弃 `brew` 只表示不再把历史 `brew` 当默认受管来源。
```

```markdown
`Cursor CLI` 当前属于“官方动作缺口”场景：官方文档只明确 install / update，未给出受管 uninstall；`agent-manager` 会 fail-close，而不是伪造删除命令。
“不用 brew”不等于“所有 agent 都必须 native installer”；`Codex` / `Gemini CLI` 继续以官方 npm 为默认受管路径，`Kimi Code CLI` 继续以官方 install script + `uv tool` 为默认组合路径。
```

- [ ] **Step 4: Run the targeted contract tests and the broader project-contract suite**

Run: `python3 -m unittest tests.test_project_contracts.ProjectContractTests.test_readme_documents_official_cli_lifecycle_policy tests.test_project_contracts.ProjectContractTests.test_troubleshooting_documents_cli_provider_migration_paths -v`

Expected: PASS and confirm the repo now documents the same policy used by config and runtime code.

Run: `python3 -m unittest tests.test_project_contracts -v`

Expected: PASS and confirm the wording changes did not regress unrelated README / doc contracts.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/TROUBLESHOOTING.md tests/test_project_contracts.py
git commit -m "docs: clarify official recommended agent lifecycle policy"
```

### Task 5: Run Final Verification And Finish Cleanly

**Files:**
- Modify: `config/agents.json`
- Modify: `agent-lifecycle/sync_agents.py`
- Modify: `src/manage_doctor.py`
- Modify: `tests/test_agent_lifecycle.py`
- Modify: `tests/test_manage_doctor.py`
- Modify: `tests/test_project_contracts.py`
- Modify: `README.md`
- Modify: `docs/TROUBLESHOOTING.md`

- [ ] **Step 1: Re-run the focused suites in final order**

Run: `python3 -m unittest tests.test_agent_lifecycle -v`

Expected: PASS

Run: `python3 -m unittest tests.test_manage_doctor -v`

Expected: PASS

Run: `python3 -m unittest tests.test_project_contracts -v`

Expected: PASS

- [ ] **Step 2: Run the broader smoke check**

Run: `make smoke`

Expected: PASS and confirm the lifecycle / doctor / documentation changes did not break the repo-level smoke contract.

- [ ] **Step 3: Inspect the owned diff before any final push / handoff**

Run: `git diff --stat -- config/agents.json agent-lifecycle/sync_agents.py src/manage_doctor.py tests/test_agent_lifecycle.py tests/test_manage_doctor.py tests/test_project_contracts.py README.md docs/TROUBLESHOOTING.md`

Expected: Only the planned lifecycle-gap, doctor, test, and doc hunks are present. If unrelated hunks still exist in these files, stage only the owned changes.

- [ ] **Step 4: Verify staging cleanliness for owned files**

Run: `git status --short`

Expected: No unstaged owned changes remain after the per-task commits, and any remaining dirty files are clearly unrelated pre-existing work that was intentionally left untouched.

## Spec Coverage Check

- `unsupported_actions` config model: covered by Task 1 and Task 2.
- Lifecycle distinction between official gap and missing command: covered by Task 2.
- `doctor` exposure of `official_gap`: covered by Task 3.
- README / TROUBLESHOOTING policy reset from "all native" to "official recommended path": covered by Task 4.
- Verification plan (`tests.test_agent_lifecycle`, `tests.test_manage_doctor`, `tests.test_project_contracts`, `make smoke`): covered by Task 5.

## Implementation Notes

- Do not change `manage.sh` or `Makefile` entrypoints in this plan.
- Do not invent a `Cursor CLI` uninstall command.
- Do not migrate existing user installs automatically.
- Do not convert `Codex` / `Gemini CLI` away from official `npm`.
