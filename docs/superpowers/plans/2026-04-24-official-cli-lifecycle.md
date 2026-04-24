# Official CLI Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `Claude Code`、`Codex`、`Gemini CLI` 按各自官方当前推荐路径受管 lifecycle，同时把历史 `brew` 安装收敛为 detect-only + fail-close。

**Architecture:** 这次改动复用现有 `manage_runtime_probe.py` + `sync_agents.py` + `manage_doctor.py` 主链路，不新增 CLI 入口，也不做自动迁移。核心做法是给 provider 增加最小管理语义、给 agent 增加严格托管开关、让 runtime probe 支持平台命令解析和显式 native 路径识别，然后在 lifecycle / doctor 层对 deprecated provider 和 `system-only` 外部安装 fail-close。

**Tech Stack:** Python unittest, JSON config, Markdown docs

---

## File Map

- Modify: `config/agents.json`
  - 负责 `claude-code` / `gemini-cli` / `codex` 的 provider 定义、默认 provider、严格托管开关
- Modify: `config/manager.json`
  - 负责默认策略是否启用、是否自动安装、默认 `preferred_provider`
- Modify: `src/manage_runtime_probe.py`
  - 负责 provider 探测、命令头收集、平台命令解析的共享逻辑
- Modify: `agent-lifecycle/sync_agents.py`
  - 负责 provider 状态分类与 install / update / uninstall / repair 语义
- Modify: `src/manage_doctor.py`
  - 负责 `doctor` 报告里的 provider 漂移、deprecated provider、外部安装诊断
- Modify: `tests/test_manage_runtime_probe.py`
  - 锁定 native path 识别、平台命令解析、runtime command 收集契约
- Modify: `tests/test_agent_lifecycle.py`
  - 锁定 config 默认值、deprecated provider fail-close、strict managed provider 语义
- Modify: `tests/test_manage_doctor.py`
  - 锁定 `doctor` 对 deprecated provider / unmanaged external install 的报错行为
- Modify: `tests/test_project_contracts.py`
  - 锁定 README / troubleshooting 的关键文案不再漂移
- Modify: `README.md`
  - 说明三种 CLI 的官方受管 provider、Codex 已进入 lifecycle、brew 仅做历史识别
- Modify: `docs/TROUBLESHOOTING.md`
  - 写清 deprecated provider、迁移处理和 `Claude Code` native / npm 的边界

## Task 1: 固定配置契约与默认策略

**Files:**
- Modify: `tests/test_agent_lifecycle.py`
- Modify: `config/agents.json`
- Modify: `config/manager.json`

- [ ] **Step 1: 先写失败中的配置契约测试**

```python
class AgentIdConsistencyTests(unittest.TestCase):
    def test_config_uses_stable_agent_ids(self):
        manager_agents = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))["agents"]
        lifecycle_agents = json.loads((ROOT / "config" / "agents.json").read_text(encoding="utf-8"))
        skill_targets = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))["agents"]
        mcp_agent_paths = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))["agent_paths"]

        managed_agent_ids = {
            "claude-code",
            "gemini-cli",
            "cursor",
            "opencode",
            "codex",
            "kimi-cli",
        }

        self.assertEqual(set(manager_agents), managed_agent_ids)
        self.assertEqual(set(lifecycle_agents), managed_agent_ids)
        self.assertEqual(set(skill_targets), managed_agent_ids)
        self.assertEqual(set(mcp_agent_paths), managed_agent_ids)


class CommittedAgentPolicyTests(unittest.TestCase):
    def test_committed_agent_policy_tracks_official_cli_providers(self):
        manager = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))
        lifecycle_agents = json.loads((ROOT / "config" / "agents.json").read_text(encoding="utf-8"))

        self.assertEqual(manager["agents"]["claude-code"]["preferred_provider"], "native")
        self.assertEqual(manager["agents"]["codex"]["preferred_provider"], "npm")
        self.assertEqual(manager["agents"]["gemini-cli"]["preferred_provider"], "npm")
        self.assertTrue(manager["agents"]["codex"]["auto_install"])

        self.assertTrue(lifecycle_agents["claude-code"]["require_managed_provider"])
        self.assertTrue(lifecycle_agents["codex"]["require_managed_provider"])
        self.assertTrue(lifecycle_agents["gemini-cli"]["require_managed_provider"])
```

- [ ] **Step 2: 运行目标测试，确认它先失败**

Run: `python3 -m unittest tests.test_agent_lifecycle.AgentIdConsistencyTests tests.test_agent_lifecycle.CommittedAgentPolicyTests -v`

Expected: FAIL，至少会出现 `codex` 还未进入 lifecycle 管理、`claude-code` / `gemini-cli` 仍默认 `brew`

- [ ] **Step 3: 修改生命周期定义和默认策略**

```json
{
  "claude-code": {
    "display_name": "Claude Code",
    "bin": "claude",
    "preferred_provider": "native",
    "require_managed_provider": true,
    "providers": {
      "native": {
        "managed": true,
        "install": {
          "darwin": "curl -fsSL https://claude.ai/install.sh | bash",
          "linux": "curl -fsSL https://claude.ai/install.sh | bash",
          "windows": "curl -fsSL https://claude.ai/install.cmd -o install.cmd && install.cmd && del install.cmd"
        },
        "upgrade": "claude update",
        "detect_paths": ["~/.local/bin/claude"]
      },
      "npm": {
        "managed": true,
        "install": "npm install -g @anthropic-ai/claude-code",
        "uninstall": "npm uninstall -g @anthropic-ai/claude-code",
        "upgrade": "npm install -g @anthropic-ai/claude-code@latest"
      },
      "brew": {
        "managed": false,
        "install": "brew install --cask claude-code"
      }
    }
  },
  "codex": {
    "display_name": "Codex",
    "bin": "codex",
    "preferred_provider": "npm",
    "require_managed_provider": true,
    "providers": {
      "npm": {
        "managed": true,
        "install": "npm install -g @openai/codex",
        "uninstall": "npm uninstall -g @openai/codex",
        "upgrade": "npm install -g @openai/codex@latest"
      },
      "brew": {
        "managed": false,
        "install": "brew install --cask codex"
      }
    }
  },
  "gemini-cli": {
    "display_name": "Gemini CLI",
    "bin": "gemini",
    "preferred_provider": "npm",
    "require_managed_provider": true,
    "providers": {
      "npm": {
        "managed": true,
        "install": "npm install -g @google/gemini-cli",
        "uninstall": "npm uninstall -g @google/gemini-cli",
        "upgrade": "npm install -g @google/gemini-cli@latest"
      },
      "brew": {
        "managed": false,
        "install": "brew install gemini-cli"
      }
    }
  }
}
```

```json
{
  "agents": {
    "claude-code": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "native" },
    "gemini-cli": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "npm" },
    "cursor": { "enabled": true, "auto_install": false, "auto_update": true, "preferred_provider": "brew" },
    "opencode": { "enabled": true, "auto_install": false, "auto_update": true, "preferred_provider": "brew" },
    "codex": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "npm" },
    "kimi-cli": { "enabled": true, "auto_install": false, "auto_update": true, "preferred_provider": "uv" }
  }
}
```

- [ ] **Step 4: 重新运行配置契约测试**

Run: `python3 -m unittest tests.test_agent_lifecycle.AgentIdConsistencyTests tests.test_agent_lifecycle.CommittedAgentPolicyTests -v`

Expected: PASS

- [ ] **Step 5: 提交配置契约变更**

```bash
git add tests/test_agent_lifecycle.py config/agents.json config/manager.json
git commit -m "feat: adopt official cli provider defaults"
```

## Task 2: 扩展 runtime probe 的平台命令解析与 native path 探测

**Files:**
- Modify: `src/manage_runtime_probe.py`
- Modify: `tests/test_manage_runtime_probe.py`

- [ ] **Step 1: 先写失败中的 runtime probe 测试**

```python
def test_resolve_provider_command_prefers_platform_specific_value(self):
    command = manage_runtime_probe.resolve_provider_command(
        {
            "default": "echo default",
            "posix": "echo posix",
            "darwin": "echo darwin",
        },
        platform_name="darwin",
    )

    self.assertEqual(command, "echo darwin")


def test_detect_all_providers_matches_declared_native_paths_before_system_fallback(self):
    inventory = manage_runtime_probe.ProviderInventory(
        run_cmd=lambda cmd, timeout=10: (False, ""),
        which=lambda binary: "/Users/test/.local/bin/claude" if binary == "claude" else None,
        path_exists=lambda _path: False,
        realpath=lambda path: path,
    )

    providers = manage_runtime_probe.detect_all_providers(
        "claude",
        {
            "providers": {
                "native": {
                    "managed": true,
                    "detect_paths": ["/Users/test/.local/bin/claude"],
                }
            }
        },
        inventory=inventory,
    )

    self.assertEqual(providers, ["native"])


def test_collect_required_runtime_commands_skips_unmanaged_agent_providers(self):
    required = manage_runtime_probe.collect_required_runtime_commands(
        {
            "agents": {
                "claude-code": {"enabled": True, "auto_install": True, "auto_update": True, "preferred_provider": "native"}
            },
            "tools": {},
            "skills": {},
            "mcp": {},
        },
        {
            "claude-code": {
                "providers": {
                    "native": {
                        "managed": true,
                        "install": {"darwin": "curl -fsSL https://claude.ai/install.sh | bash"},
                        "upgrade": "claude update"
                    },
                    "brew": {
                        "managed": false,
                        "install": "brew install --cask claude-code"
                    }
                }
            }
        },
        {},
        {"repositories": {}},
        {"servers": {}},
    )

    self.assertEqual(required, ["curl", "bash", "claude"])
```

- [ ] **Step 2: 运行 runtime probe 测试，确认它先失败**

Run: `python3 -m unittest tests.test_manage_runtime_probe -v`

Expected: FAIL，报出 `resolve_provider_command` 不存在、`native` 还无法被识别、required runtime commands 仍会把 deprecated provider 算进去

- [ ] **Step 3: 实现共享命令解析和显式 provider 探测**

```python
def provider_is_managed(provider_cfg):
    return (provider_cfg or {}).get("managed", True)


def canonical_provider_name(provider_name):
    if provider_name.startswith("brew"):
        return "brew"
    return provider_name


def classify_provider_state(actual_providers, definition):
    providers = definition.get("providers", {})
    managed = []
    deprecated = []
    system_only = []
    unsupported = []

    for provider in actual_providers:
        canonical = canonical_provider_name(provider)
        if provider == "system":
            system_only.append(provider)
        elif canonical in providers:
            if provider_is_managed(providers[canonical]):
                managed.append(provider)
            else:
                deprecated.append(provider)
        else:
            unsupported.append(provider)

    return {
        "managed": managed,
        "deprecated": deprecated,
        "system_only": system_only,
        "unsupported": unsupported,
    }


def resolve_provider_command(command_value, platform_name=None):
    if not command_value:
        return None
    if isinstance(command_value, str):
        return command_value
    if not isinstance(command_value, dict):
        return None

    current = platform_name or sys.platform
    candidates = []
    if current.startswith("darwin"):
        candidates.extend(["darwin", "posix", "default"])
    elif current.startswith("linux"):
        candidates.extend(["linux", "posix", "default"])
    elif current.startswith("win"):
        candidates.extend(["windows", "default"])
    else:
        candidates.extend(["posix", "default"])

    for key in candidates:
        value = command_value.get(key)
        if value:
            return value
    return None


def detect_all_providers(self, bin_name, config=None):
    found = []
    config = config or {}

    bin_path = self._which(bin_name)
    resolved_bin_path = self._realpath(bin_path) if bin_path else ""
    normalized_candidates = {
        normalize_path_for_match(os.path.expanduser(path))
        for path in (bin_path, resolved_bin_path)
        if path
    }

    # 现有 brew / npm / gh / uv 逻辑保留

    for provider_name, provider_cfg in config.get("providers", {}).items():
        detect_paths = {
            normalize_path_for_match(os.path.expanduser(path))
            for path in provider_cfg.get("detect_paths", [])
            if path
        }
        if detect_paths and normalized_candidates & detect_paths and provider_name not in found:
            found.append(provider_name)

    if (config.get("app_path") and self._path_exists(config["app_path"])) or bin_path:
        if not found:
            found.append("system")

    return found
```

```python
for agent_name, policy in config.get("agents", {}).items():
    if not policy.get("enabled", True):
        continue
    definition = agent_defs.get(agent_name, {})
    providers = definition.get("providers", {})
    preferred = policy.get("preferred_provider") or definition.get("preferred_provider")
    candidate_providers = []
    if preferred and preferred in providers:
        candidate_providers.append(providers[preferred])
    else:
        candidate_providers.extend(providers.values())
    for provider in candidate_providers:
        if not provider_is_managed(provider):
            continue
        install_cmd = resolve_provider_command(provider.get("install"))
        upgrade_cmd = resolve_provider_command(provider.get("upgrade"))
        if policy.get("auto_install", True):
            for head in iter_command_heads(install_cmd):
                add(head)
        if policy.get("auto_update", True):
            for head in iter_command_heads(upgrade_cmd):
                add(head)
```

- [ ] **Step 4: 重新运行 runtime probe 测试**

Run: `python3 -m unittest tests.test_manage_runtime_probe -v`

Expected: PASS

- [ ] **Step 5: 提交 runtime probe 变更**

```bash
git add src/manage_runtime_probe.py tests/test_manage_runtime_probe.py
git commit -m "feat: add official provider probe semantics"
```

## Task 3: 收紧 lifecycle reconcile 语义

**Files:**
- Modify: `agent-lifecycle/sync_agents.py`
- Modify: `tests/test_agent_lifecycle.py`

- [ ] **Step 1: 先写失败中的 reconcile 测试**

```python
def test_deprecated_provider_fails_when_agent_requires_managed_provider(self):
    definition = {
        "display_name": "Claude Code",
        "bin": "claude",
        "require_managed_provider": True,
        "providers": {
            "native": {"managed": True, "upgrade": "claude update"},
            "brew": {"managed": False, "install": "brew install --cask claude-code"},
        },
    }
    policy = {"enabled": True, "auto_update": True, "preferred_provider": "native"}

    with (
        mock.patch.object(sync_agents, "detect_all_providers", return_value=["brew-cask"]),
        mock.patch.object(sync_agents, "is_agent_installed", return_value=True),
        mock.patch.object(sync_agents, "run_cmd") as run_cmd,
        self.assertLogs(level="ERROR") as logs,
    ):
        result = sync_agents.reconcile_agent("claude-code", definition, policy)

    self.assertFalse(result)
    run_cmd.assert_not_called()
    self.assertTrue(any("deprecated" in line or "迁移" in line for line in logs.output), logs.output)


def test_system_only_provider_fails_when_agent_requires_managed_provider(self):
    definition = {
        "display_name": "Codex",
        "bin": "codex",
        "require_managed_provider": True,
        "providers": {
            "npm": {"managed": True, "upgrade": "npm install -g @openai/codex@latest"},
        },
    }
    policy = {"enabled": True, "auto_update": True, "preferred_provider": "npm"}

    with (
        mock.patch.object(sync_agents, "detect_all_providers", return_value=["system"]),
        mock.patch.object(sync_agents, "run_cmd") as run_cmd,
        self.assertLogs(level="ERROR") as logs,
    ):
        result = sync_agents.reconcile_agent("codex", definition, policy)

    self.assertFalse(result)
    run_cmd.assert_not_called()
    self.assertTrue(any("未受管外部安装" in line for line in logs.output), logs.output)


def test_install_uses_platform_resolved_command(self):
    definition = {
        "display_name": "Claude Code",
        "bin": "claude",
        "providers": {
            "native": {
                "managed": True,
                "install": {"darwin": "curl -fsSL https://claude.ai/install.sh | bash"},
            }
        },
    }
    policy = {"enabled": True, "auto_install": True, "preferred_provider": "native"}

    with (
        mock.patch.object(sync_agents, "detect_all_providers", side_effect=[[], ["native"]]),
        mock.patch.object(sync_agents, "run_cmd", return_value=(True, "installed")) as run_cmd,
    ):
        result = sync_agents.reconcile_agent("claude-code", definition, policy)

    self.assertTrue(result)
    run_cmd.assert_called_once_with("curl -fsSL https://claude.ai/install.sh | bash")
```

- [ ] **Step 2: 运行 lifecycle 测试，确认它先失败**

Run: `python3 -m unittest tests.test_agent_lifecycle.ReconcileAgentTests -v`

Expected: FAIL，当前实现仍把 `brew-cask` 当成普通 managed provider，`system` 仍会静默跳过，平台映射命令也还未解析

- [ ] **Step 3: 实现 deprecated / strict managed provider 语义**

```python
from src.manage_runtime_probe import (
    ProviderInventory as ProviderCache,
    classify_provider_state,
    detect_all_providers as detect_all_providers_runtime,
    get_pkg_name,
    is_agent_installed,
    provider_is_managed,
    resolve_provider_command,
)


def detect_provider_state(bin_name, definition):
    actual = detect_all_providers(bin_name, definition)
    state = classify_provider_state(actual, definition)
    state["all"] = actual
    return state


def reconcile_agent(agent_id, definition, policy, dry_run=False, allow_install=True, offline=False):
    # ...
    state = detect_provider_state(bin_name, definition)
    managed_actual = state["managed"]
    deprecated_actual = state["deprecated"]
    system_actual = state["system_only"]
    unsupported_actual = state["unsupported"]

    if deprecated_actual:
        logging.error(f"[AGENT: {label}] 检测到已弃用安装来源: {', '.join(deprecated_actual)}；请先迁移到受管 provider。")
        return False

    if system_actual and not managed_actual and definition.get("require_managed_provider", False):
        logging.error(f"[AGENT: {label}] 检测到未受管外部安装，当前策略要求受管 provider。")
        return False

    install_cmd = resolve_provider_command(p_cfg.get("install")) if p_cfg else None
    upgrade_cmd = resolve_provider_command(p_cfg.get("upgrade")) if p_cfg else None
    uninstall_cmd = resolve_provider_command(p_cfg.get("uninstall")) if p_cfg else None
```

```python
def get_update_probe_cmd(provider_name, provider_cfg):
    if not provider_cfg:
        return None
    explicit = resolve_provider_command(provider_cfg.get("check_update"))
    if explicit:
        return explicit
    if provider_name.startswith("brew"):
        install_cmd = resolve_provider_command(provider_cfg.get("install"))
        pkg_name = get_pkg_name(install_cmd or "")
        if not pkg_name:
            return None
        if provider_name == "brew-cask":
            return f"brew outdated --cask {pkg_name}"
        return f"brew outdated {pkg_name}"
    return None
```

- [ ] **Step 4: 重新运行 lifecycle 测试**

Run: `python3 -m unittest tests.test_agent_lifecycle.ReconcileAgentTests -v`

Expected: PASS

- [ ] **Step 5: 提交 lifecycle 变更**

```bash
git add agent-lifecycle/sync_agents.py tests/test_agent_lifecycle.py
git commit -m "feat: enforce managed cli providers"
```

## Task 4: 更新 doctor 诊断为强约束

**Files:**
- Modify: `src/manage_doctor.py`
- Modify: `tests/test_manage_doctor.py`

- [ ] **Step 1: 先写失败中的 doctor 测试**

```python
def test_report_flags_deprecated_provider_as_error(self):
    with tempfile.TemporaryDirectory() as temp_dir:
        temp_root = Path(temp_dir) / "repo"
        shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

        (temp_root / "config" / "manager.json").write_text(
            '{\n  "agents": {\n    "claude-code": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "native" }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
            encoding="utf-8",
        )
        (temp_root / "config" / "agents.json").write_text(
            '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "claude",\n    "require_managed_provider": true,\n    "providers": {\n      "native": { "managed": true, "upgrade": "claude update" },\n      "brew": { "managed": false, "install": "brew install --cask claude-code" }\n    }\n  }\n}\n',
            encoding="utf-8",
        )

        with (
            mock.patch("src.manage_doctor.detect_all_providers", return_value=["brew-cask"]),
            mock.patch("src.manage_doctor.is_agent_installed", return_value=True),
        ):
            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

        agent_names = {entry["name"]: entry for entry in report["sections"]["agents"]}
        self.assertEqual(report["status"], "error")
        self.assertEqual(agent_names["Claude Code"]["status"], "error")
        self.assertIn("已弃用安装来源", agent_names["Claude Code"]["message"])
```

```python
def test_report_flags_system_only_provider_when_managed_provider_is_required(self):
    with tempfile.TemporaryDirectory() as temp_dir:
        temp_root = Path(temp_dir) / "repo"
        shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

        (temp_root / "config" / "manager.json").write_text(
            '{\n  "agents": {\n    "codex": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "npm" }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
            encoding="utf-8",
        )
        (temp_root / "config" / "agents.json").write_text(
            '{\n  "codex": {\n    "display_name": "Codex",\n    "bin": "codex",\n    "require_managed_provider": true,\n    "providers": {\n      "npm": { "managed": true, "upgrade": "npm install -g @openai/codex@latest" }\n    }\n  }\n}\n',
            encoding="utf-8",
        )

        with (
            mock.patch("src.manage_doctor.detect_all_providers", return_value=["system"]),
            mock.patch("src.manage_doctor.is_agent_installed", return_value=True),
        ):
            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

        agent_names = {entry["name"]: entry for entry in report["sections"]["agents"]}
        self.assertEqual(report["status"], "error")
        self.assertIn("未纳入 agent-manager 托管", agent_names["Codex"]["message"])
```

- [ ] **Step 2: 运行 doctor 测试，确认它先失败**

Run: `python3 -m unittest tests.test_manage_doctor -v`

Expected: FAIL，当前 `doctor` 还会把 deprecated provider 当成普通已安装项，`system-only` 也不会因为 strict managed provider 报错

- [ ] **Step 3: 实现 doctor 的 provider 分类诊断**

```python
for agent_id, policy in config.get("agents", {}).items():
    definition = agent_defs.get(agent_id, {})
    label = get_agent_display_name(agent_id, definition)
    providers = detect_all_providers(definition.get("bin", ""), definition, inventory=inventory)
    state = classify_provider_state(providers, definition)
    desired = policy.get("enabled", True)
    preferred = policy.get("preferred_provider") or definition.get("preferred_provider", "npm")
    runtime_ready = is_agent_installed(agent_id, definition)

    if state["deprecated"]:
        append_entry(
            report,
            "agents",
            make_entry(label, "error", f"检测到已弃用安装来源: {', '.join(state['deprecated'])}"),
        )
    elif state["system_only"] and definition.get("require_managed_provider", False):
        append_entry(
            report,
            "agents",
            make_entry(label, "error", "检测到外部安装，未纳入 agent-manager 托管"),
        )
    elif state["managed"] and not runtime_ready:
        append_entry(report, "agents", make_entry(label, "error", f"{state['managed'][0]} 已安装，但运行入口缺失"))
    elif bool(providers) == desired:
        detail = state["managed"][0] if state["managed"] else (providers[0] if providers else "未安装")
        append_entry(report, "agents", make_entry(label, "ok", detail))
    else:
        append_entry(report, "agents", make_entry(label, "error", f"漂移 (实际:{'已装' if providers else '未装'})"))
```

- [ ] **Step 4: 重新运行 doctor 测试**

Run: `python3 -m unittest tests.test_manage_doctor -v`

Expected: PASS

- [ ] **Step 5: 提交 doctor 变更**

```bash
git add src/manage_doctor.py tests/test_manage_doctor.py
git commit -m "feat: surface deprecated cli providers in doctor"
```

## Task 5: 刷新 README / Troubleshooting 与文档契约

**Files:**
- Modify: `tests/test_project_contracts.py`
- Modify: `README.md`
- Modify: `docs/TROUBLESHOOTING.md`

- [ ] **Step 1: 先写失败中的文档契约测试**

```python
def test_readme_documents_official_cli_lifecycle_policy(self):
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    self.assertIn("`Claude Code`、`Codex`、`Gemini CLI`", readme)
    self.assertIn("仅用于识别并提示迁移", readme)


def test_troubleshooting_documents_cli_provider_migration_paths(self):
    troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
    self.assertIn("历史 brew 安装", troubleshooting)
    self.assertIn("claude migrate-installer", troubleshooting)
    self.assertIn("Codex", troubleshooting)
```

- [ ] **Step 2: 运行文档契约测试，确认它先失败**

Run: `python3 -m unittest tests.test_project_contracts.ProjectContractTests.test_readme_documents_official_cli_lifecycle_policy tests.test_project_contracts.ProjectContractTests.test_troubleshooting_documents_cli_provider_migration_paths -v`

Expected: FAIL，README 目前仍把 `Codex` 写成仅 MCP / skills / rules 支持，troubleshooting 仍以保留 brew 为主叙述

- [ ] **Step 3: 更新 README 和故障排查文档**

```md
🚀 **AI 助手环境一键管理中枢**。通过单一入口，对 `Claude Code`、`Codex`、`Gemini CLI`、`Cursor`、`OpenCode`、`Kimi Code CLI` 做生命周期管理。

默认自动安装的 Agent 当前包含 `Claude Code`、`Codex` 和 `Gemini CLI`；`Cursor IDE`、`OpenCode` 与 `Kimi Code CLI` 仍受管，但默认不自动安装。

`Claude Code` 当前默认走官方 native installer；`Codex` 与 `Gemini CLI` 默认走官方 npm 包。历史 `brew` 安装仅用于识别并提示迁移，不再作为受管 provider。

`Claude Code local install` 不纳入 `agent-manager` lifecycle 托管，因为它依赖 shell alias 暴露入口，当前探测链路无法稳定接管。

当前实现只把 `Claude Code native` 的 install / update 视作已验证受管能力；若用户显式禁用该 agent，缺少官方已确认的卸载命令时应 fail-close，而不是猜测卸载行为。
```

```md
1. **历史 brew 安装被判定为不合规**
   - 症状：`doctor` 报告 `已弃用安装来源`，或 `update/bootstrap` 直接失败
   - 处理：
     - `Claude Code`：手工卸载 brew 版本；若当前是 npm 版并希望切到 native，按官方 `claude migrate-installer` 迁移
     - `Codex`：手工卸载 brew 版本，再执行 `npm install -g @openai/codex`
     - `Gemini CLI`：手工卸载 brew 版本，再执行 `npm install -g @google/gemini-cli`

2. **检测到外部安装但未纳入托管**
   - 症状：`doctor` 报告 `未纳入 agent-manager 托管`
   - 处理：确认当前 PATH 中的 `claude` / `codex` / `gemini` 来自官方受管来源，而不是系统残留或自定义包装脚本
```

- [ ] **Step 4: 重新运行文档契约测试**

Run: `python3 -m unittest tests.test_project_contracts.ProjectContractTests.test_readme_documents_official_cli_lifecycle_policy tests.test_project_contracts.ProjectContractTests.test_troubleshooting_documents_cli_provider_migration_paths -v`

Expected: PASS

- [ ] **Step 5: 提交文档契约变更**

```bash
git add tests/test_project_contracts.py README.md docs/TROUBLESHOOTING.md
git commit -m "docs: document official cli lifecycle policy"
```

## Task 6: 运行回归并复查变更范围

**Files:**
- Verify: `tests/test_manage_runtime_probe.py`
- Verify: `tests/test_agent_lifecycle.py`
- Verify: `tests/test_manage_doctor.py`
- Verify: `tests/test_project_contracts.py`

- [ ] **Step 1: 先跑本次变更的相关测试集合**

Run: `python3 -m unittest tests.test_manage_runtime_probe tests.test_agent_lifecycle tests.test_manage_doctor tests.test_project_contracts -v`

Expected: PASS

- [ ] **Step 2: 再跑一遍更宽的全量回归**

Run: `python3 -m unittest discover -s tests -p 'test*.py' -v`

Expected: PASS

- [ ] **Step 3: 复查 diff 与变更范围**

Run: `git diff --stat HEAD~5..HEAD && git diff --check`

Expected: 只包含本计划列出的 Python / JSON / Markdown 文件，没有 `agent-rule/rules/dist/` 这类生成产物被手改，也没有未预期的入口改动
