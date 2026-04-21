# Kimi CLI Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `Kimi Code CLI` 接入 `agent-manager` 的 lifecycle、MCP、rules 和 skills 管理链路，同时保持 `~/.kimi/config.toml` 不在本轮托管范围。

**Architecture:** 这次变更复用现有 `Claude Code` / `Gemini CLI` / `Codex` 的管理模式，只在稳定 agent id、provider 探测、目标路径映射和规则分发目标上增加 `kimi-cli`。实现遵循最小本地改动原则：MCP 继续走 JSON `mcpServers`，skills 继续走目录 link，rules 新增 `kimi` target 并把模块引用重写到 `~/.kimi/modules/*.md`，不碰 Kimi 的登录态、provider、model 和 hooks。

**Tech Stack:** Python unittest, JSON config, Bash, Markdown, generated rule artifacts

---

### Task 1: 固定 Kimi lifecycle 契约与默认策略

**Files:**
- Modify: `tests/test_agent_lifecycle.py`
- Modify: `tests/test_manage_runtime_probe.py`
- Modify: `config/agents.json`
- Modify: `config/manager.json`
- Modify: `src/agent_ids.py`
- Modify: `src/manage_runtime_probe.py`

- [ ] **Step 1: 写失败中的 agent id 与默认策略测试**

```python
class AgentIdConsistencyTests(unittest.TestCase):
    def test_config_uses_stable_agent_ids(self):
        manager_agents = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))["agents"]
        lifecycle_agents = json.loads((ROOT / "config" / "agents.json").read_text(encoding="utf-8"))
        skill_targets = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))["agents"]
        mcp_targets = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))["agent_paths"]

        managed_agent_ids = {
            "claude-code",
            "gemini-cli",
            "cursor",
            "opencode",
            "kimi-cli",
        }

        self.assertEqual(set(manager_agents), managed_agent_ids)
        self.assertEqual(set(lifecycle_agents), managed_agent_ids)
        self.assertTrue(managed_agent_ids.issubset(set(skill_targets)))
        self.assertTrue(managed_agent_ids.issubset(set(mcp_targets)))
        self.assertIn("codex", skill_targets)
        self.assertIn("codex", mcp_targets)


class CommittedAgentPolicyTests(unittest.TestCase):
    def test_committed_agent_policy_disables_default_kimi_auto_install(self):
        manager = json.loads((ROOT / "config" / "manager.json").read_text(encoding="utf-8"))

        self.assertFalse(manager["agents"]["kimi-cli"]["auto_install"])
        self.assertEqual(manager["agents"]["kimi-cli"]["preferred_provider"], "uv")
```

- [ ] **Step 2: 写失败中的 uv provider 探测测试**

```python
class ManageRuntimeProbeTests(unittest.TestCase):
    def test_detect_all_providers_supports_uv_tool_install(self):
        command_outputs = {
            ("uv", "tool", "list"): (True, "kimi-cli v0.8.6\n- kimi\n"),
        }

        inventory = manage_runtime_probe.ProviderInventory(
            run_cmd=lambda cmd, timeout=10: command_outputs.get(tuple(cmd), (False, "")),
            which=lambda binary: {
                "uv": "/opt/homebrew/bin/uv",
                "kimi": "/Users/test/.local/bin/kimi",
            }.get(binary),
            path_exists=lambda _path: False,
            realpath=lambda path: path,
        )

        providers = manage_runtime_probe.detect_all_providers(
            "kimi",
            {
                "providers": {
                    "uv": {
                        "install": "uv tool install --python 3.13 kimi-cli",
                    }
                }
            },
            inventory=inventory,
        )

        self.assertEqual(providers, ["uv"])
```

- [ ] **Step 3: 运行测试确认它们先失败**

Run: `python3 -m unittest tests.test_agent_lifecycle.AgentIdConsistencyTests tests.test_agent_lifecycle.CommittedAgentPolicyTests tests.test_manage_runtime_probe.ManageRuntimeProbeTests.test_detect_all_providers_supports_uv_tool_install -v`

Expected: FAIL，提示 `kimi-cli` 尚未出现在稳定 agent id、默认策略或 provider 探测结果中。

- [ ] **Step 4: 增加 Kimi agent 定义、默认策略和展示名**

```json
{
  "kimi-cli": {
    "display_name": "Kimi Code CLI",
    "bin": "kimi",
    "preferred_provider": "uv",
    "providers": {
      "uv": {
        "install": "uv tool install --python 3.13 kimi-cli",
        "uninstall": "uv tool uninstall kimi-cli",
        "upgrade": "uv tool upgrade kimi-cli --no-cache"
      }
    }
  }
}
```

```json
{
  "kimi-cli": {
    "enabled": true,
    "auto_install": false,
    "auto_update": true,
    "preferred_provider": "uv"
  }
}
```

```python
AGENT_METADATA = {
    "kimi-cli": {
        "display_name": "Kimi Code CLI",
        "aliases": ["Kimi", "Kimi CLI", "Kimi Code CLI"],
    },
}
```

- [ ] **Step 5: 在 runtime probe 中实现 uv tool 识别**

```python
import shlex


def get_uv_tool_name(cmd):
    if not cmd:
        return None
    tokens = shlex.split(cmd)
    if "tool" not in tokens:
        return None
    action = next((name for name in ("install", "upgrade", "uninstall") if name in tokens), None)
    if action is None:
        return None
    index = tokens.index(action) + 1
    while index < len(tokens):
        token = tokens[index]
        if token in {"--python"}:
            index += 2
            continue
        if token.startswith("-"):
            index += 1
            continue
        return token
    return None


class ProviderInventory:
    def __init__(
        self,
        run_cmd=None,
        which=None,
        path_exists=None,
        realpath=None,
    ):
        self._run_cmd = run_cmd or default_run_cmd
        self._which = which or shutil.which
        self._path_exists = path_exists or os.path.exists
        self._realpath = realpath or os.path.realpath
        self._brew_formulae = None
        self._brew_casks = None
        self._gh_extensions = None
        self._uv_tools = None

    def get_uv_tools(self):
        if self._uv_tools is None:
            self._uv_tools = set()
            if self._which("uv"):
                success, out = self._run_cmd(["uv", "tool", "list"], timeout=10)
                if success:
                    for raw_line in out.splitlines():
                        line = raw_line.strip()
                        if not line or line.startswith("-"):
                            continue
                        package = line.split()[0]
                        if package:
                            self._uv_tools.add(package)
        return self._uv_tools

    def detect_all_providers(self, bin_name, config=None):
        found = []
        config = config or {}
        bin_path = self._which(bin_name)
        resolved_bin_path = self._realpath(bin_path) if bin_path else ""

        brew_pkg = get_pkg_name(config.get("providers", {}).get("brew", {}).get("install", ""))
        if brew_pkg:
            if brew_pkg in self.get_brew_formulae():
                found.append("brew")
            elif brew_pkg in self.get_brew_casks():
                found.append("brew-cask")

        uv_pkg = get_uv_tool_name(config.get("providers", {}).get("uv", {}).get("install", ""))
        if uv_pkg and uv_pkg in self.get_uv_tools():
            found.append("uv")

        if detect_npm_from_paths(
            bin_path,
            resolved_bin_path,
            brew_detected=any(provider.startswith("brew") for provider in found),
        ):
            found.append("npm")

        ext_name = config.get("ext_name")
        if ext_name and any(line.strip().startswith(ext_name) for line in self.get_gh_extensions().splitlines()):
            found.append("gh")

        app_path = config.get("app_path")
        if (app_path and self._path_exists(app_path)) or bin_path:
            if not found:
                found.append("system")

        return found
```

说明：这里不改 `agent-lifecycle/sync_agents.py` 的控制流。现有 `canonical_provider_name()` 和 `reconcile_agent()` 已经可以接受非 `brew` provider，`uv` 只需要在 definition 和 runtime probe 中成为受管 provider。

- [ ] **Step 6: 重新运行 lifecycle 与 runtime probe 测试**

Run: `python3 -m unittest tests.test_agent_lifecycle tests.test_manage_runtime_probe -v`

Expected: PASS，新增的 `kimi-cli` 契约测试通过，传入 `Kimi Code CLI` definition 时 `detect_all_providers("kimi", definition)` 返回 `["uv"]`。

- [ ] **Step 7: 提交这一组变更**

```bash
git add tests/test_agent_lifecycle.py tests/test_manage_runtime_probe.py config/agents.json config/manager.json src/agent_ids.py src/manage_runtime_probe.py
git commit -m "feat: add managed kimi cli lifecycle support"
```

### Task 2: 固定并实现 Kimi MCP 目标

**Files:**
- Modify: `tests/test_sync_mcp.py`
- Modify: `tests/test_manage_doctor.py`
- Modify: `config/mcp.json`
- Modify: `src/manage_doctor.py`

- [ ] **Step 1: 写失败中的 Kimi MCP 配置契约测试**

```python
class SyncMcpTests(unittest.TestCase):
    def test_committed_kimi_mcp_target_uses_json_mcpservers(self):
        definitions = json.loads((ROOT / "config" / "mcp.json").read_text(encoding="utf-8"))

        self.assertEqual(
            definitions["agent_paths"]["kimi-cli"],
            {"path": "~/.kimi/mcp.json", "format": "json"},
        )
```

- [ ] **Step 2: 写失败中的 doctor JSON target 审计测试**

```python
class ManageDoctorReportTests(unittest.TestCase):
    def test_report_flags_existing_kimi_mcp_file_with_missing_managed_entries(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            kimi_config = Path(temp_dir) / ".kimi" / "mcp.json"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            kimi_config.parent.mkdir(parents=True)
            kimi_config.write_text(json.dumps({"mcpServers": {"user-owned": {"command": "echo", "args": []}}}), encoding="utf-8")

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "kimi-cli": { "path": "%s", "format": "json" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["kimi-cli"]\n    }\n  }\n}\n'
                % kimi_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertIn("Kimi Code CLI", mcp_names)
            self.assertIn("期望 1 个托管服务", mcp_names["Kimi Code CLI"]["message"])
```

- [ ] **Step 3: 运行测试确认它们先失败**

Run: `python3 -m unittest tests.test_sync_mcp.SyncMcpTests.test_committed_kimi_mcp_target_uses_json_mcpservers tests.test_manage_doctor.ManageDoctorReportTests.test_report_flags_existing_kimi_mcp_file_with_missing_managed_entries -v`

Expected: FAIL，`config/mcp.json` 尚未声明 `kimi-cli` 目标，doctor 也还没有 `Kimi Code CLI` 路径覆盖。

- [ ] **Step 4: 增加 Kimi MCP target**

```json
{
  "agent_paths": {
    "claude-code": "~/.claude.json",
    "gemini-cli": "~/.gemini/settings.json",
    "cursor": "~/.cursor/mcp.json",
    "opencode": { "path": "~/.config/opencode/opencode.json", "format": "json" },
    "codex": { "path": "~/.codex/config.toml", "format": "toml" },
    "kimi-cli": { "path": "~/.kimi/mcp.json", "format": "json" }
  }
}
```

- [ ] **Step 5: 让 doctor 沿用普通 JSON target 审计 Kimi**

```python
RULE_TARGET_PREFIX_TO_ID = {
    "claude": "claude-code",
    "gemini": "gemini-cli",
    "cursor": "cursor",
    "opencode": "opencode",
    "codex": "codex",
    "kimi": "kimi-cli",
}
```

说明：MCP 注入逻辑本身不需要 Kimi 特例，`agent-mcp/sync_mcp.py` 继续让除 `opencode` 外的 JSON target 都使用 `mcpServers`。

- [ ] **Step 6: 重新运行 MCP 与 doctor 相关测试**

Run: `python3 -m unittest tests.test_sync_mcp tests.test_manage_doctor -v`

Expected: PASS，`~/.kimi/mcp.json` 被识别为标准 JSON MCP target，doctor 能用 `Kimi Code CLI` 展示该 target 的审计结果。

- [ ] **Step 7: 提交这一组变更**

```bash
git add tests/test_sync_mcp.py tests/test_manage_doctor.py config/mcp.json src/manage_doctor.py
git commit -m "feat: add kimi cli mcp target support"
```

### Task 3: 固定并实现 Kimi skills target 与暴露范围

**Files:**
- Modify: `tests/test_skill_exposure_contracts.py`
- Modify: `tests/test_link_skills.py`
- Modify: `config/skills.json`

- [ ] **Step 1: 写失败中的 skill target 与 scope 契约测试**

```python
class SkillExposureContractTests(unittest.TestCase):
    def test_multi_skill_repository_agent_scopes_stay_intentional(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        expected_scopes = {
            "agent-skills": ["claude-code", "kimi-cli", "codex"],
            "superpowers": ["claude-code", "gemini-cli", "cursor", "opencode", "kimi-cli", "codex"],
            "ui-ux-pro-max-skill": ["claude-code", "kimi-cli", "codex"],
        }

        for repo_name, expected_agents in expected_scopes.items():
            repo = skills_config["repositories"][repo_name]
            self.assertGreater(repo["skill_count"], 1)
            self.assertEqual(repo["agents"], expected_agents)
```

```python
class LinkSkillsTests(unittest.TestCase):
    def test_default_skill_catalog_includes_agent_skills_repo_with_explicit_agents(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        repo = skills_config["repositories"].get("agent-skills")

        self.assertIsNotNone(repo)
        self.assertEqual(repo["agents"], ["claude-code", "kimi-cli", "codex"])

    def test_default_skill_catalog_includes_superpowers_for_all_supported_agents(self):
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))

        repo = skills_config["repositories"].get("superpowers")

        self.assertIsNotNone(repo)
        self.assertEqual(
            set(repo["agents"]),
            {"claude-code", "gemini-cli", "cursor", "opencode", "kimi-cli", "codex"},
        )
```

- [ ] **Step 2: 运行测试确认它们先失败**

Run: `python3 -m unittest tests.test_skill_exposure_contracts tests.test_link_skills -v`

Expected: FAIL，`config/skills.json` 还没有 `kimi-cli` target，也还没把 `claude-code` 范围内的 skill 仓库扩到 `kimi-cli`。

- [ ] **Step 3: 增加 Kimi skills target 与仓库 scope**

```json
{
  "repositories": {
    "agent-skills": {
      "agents": ["claude-code", "kimi-cli", "codex"]
    },
    "playwright-best-practices-skill": {
      "agents": ["claude-code", "kimi-cli", "codex", "cursor", "opencode"]
    },
    "superpowers": {
      "agents": ["claude-code", "gemini-cli", "cursor", "opencode", "kimi-cli", "codex"]
    },
    "ui-ux-pro-max-skill": {
      "agents": ["claude-code", "kimi-cli", "codex"]
    }
  },
  "agents": {
    "gemini-cli": "~/.gemini/skills",
    "claude-code": "~/.claude/skills",
    "kimi-cli": "~/.kimi/skills",
    "codex": "~/.codex/skills",
    "cursor": "~/.cursor/rules",
    "opencode": "~/.config/opencode/skills"
  }
}
```

说明：`openai-frontend-skill` 继续保持 `codex` only，不扩大到 `kimi-cli`。

- [ ] **Step 4: 重新运行 skills 契约测试**

Run: `python3 -m unittest tests.test_skill_exposure_contracts tests.test_link_skills -v`

Expected: PASS，`kimi-cli` 作为新 target 出现在 `skills.json` 中，且只接收当前已经对 `claude-code` 开放的 skill 仓库。

- [ ] **Step 5: 提交这一组变更**

```bash
git add tests/test_skill_exposure_contracts.py tests/test_link_skills.py config/skills.json
git commit -m "feat: add kimi cli skill target support"
```

### Task 4: 固定并实现 Kimi rules 生成与同步

**Files:**
- Modify: `tests/test_sync_rules.py`
- Modify: `tests/test_manage_runtime_probe.py`
- Modify: `agent-rule/scripts/build-agent-rules.py`
- Modify: `config/rules-targets.list`
- Modify: `agent-rule/scripts/sync-agent-rules.sh`
- Modify: `src/manage_runtime_probe.py`
- Modify: `src/manage_doctor.py`

- [ ] **Step 1: 写失败中的 rules target 映射测试**

```python
class ManageRuntimeProbeTests(unittest.TestCase):
    def test_rule_target_agent_id_maps_kimi_prefix_to_kimi_cli(self):
        self.assertEqual(manage_runtime_probe.rule_target_agent_id("kimi_global"), "kimi-cli")
        self.assertEqual(manage_runtime_probe.rule_target_agent_id("kimi_modules"), "kimi-cli")
```

- [ ] **Step 2: 写失败中的 Kimi rules sync 测试**

```python
class SyncRulesTests(unittest.TestCase):
    def test_kimi_sync_supports_native_agents_filename(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            temp_bin = Path(temp_dir) / "bin"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            (temp_home / ".kimi").mkdir(parents=True)
            temp_bin.mkdir()
            kimi_bin = temp_bin / "kimi"
            kimi_bin.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            kimi_bin.chmod(0o755)

            target_path = temp_home / ".kimi" / "AGENTS.md"
            (temp_root / "config" / "rules-targets.list").write_text(
                f"kimi_global|global|rules/dist/kimi/AGENTS.md|{target_path}\n",
                encoding="utf-8",
            )

            env = os.environ.copy()
            env["HOME"] = str(temp_home)
            env["PATH"] = f"{temp_bin}:{env['PATH']}"
            result = subprocess.run(
                [str(temp_root / "agent-rule" / "scripts" / "sync-agent-rules.sh"), "--target", "kimi_global"],
                cwd=temp_root / "agent-rule",
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            source_path = temp_root / "agent-rule" / "rules" / "dist" / "kimi" / "AGENTS.md"
            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertTrue(target_path.exists(), combined)
            self.assertEqual(target_path.read_text(encoding="utf-8"), source_path.read_text(encoding="utf-8"))
```

- [ ] **Step 3: 运行测试确认它们先失败**

Run: `python3 -m unittest tests.test_manage_runtime_probe.ManageRuntimeProbeTests.test_rule_target_agent_id_maps_kimi_prefix_to_kimi_cli tests.test_sync_rules.SyncRulesTests.test_kimi_sync_supports_native_agents_filename -v`

Expected: FAIL，当前还没有 `kimi` rule target，也还没有 `.kimi` 路径的同步分支。

- [ ] **Step 4: 在 rules 构建器中增加 Kimi target，并把模块引用改写到 `~/.kimi/modules`**

```python
TARGETS = ("codex", "claude", "gemini", "cursor", "kimi")
MODULE_REFERENCE_TARGETS = {
    "codex": "codex_modules",
    "kimi": "kimi_modules",
}
TARGET_SPECS = {
    "codex": {"adapter": "codex", "filename": "AGENTS.md"},
    "claude": {"adapter": "claude", "filename": "CLAUDE.md"},
    "gemini": {"adapter": "gemini", "filename": "GEMINI.md"},
    "cursor": {
        "adapter": "ide",
        "filename": "agent-manager.mdc",
        "prefix_lines": ["---", "alwaysApply: true", "---"],
    },
    "kimi": {"adapter": "codex", "filename": "AGENTS.md"},
}


def render_target_content(target: str, core_lines: list[str], adapter_lines: list[str]) -> str:
    content = "\n".join([*core_lines, *adapter_lines])
    modules_target_name = MODULE_REFERENCE_TARGETS.get(target)
    if modules_target_name:
        modules_root = render_instruction_path(read_manifest_target(modules_target_name)).rstrip("/")
        content = MODULE_REFERENCE_PATTERN.sub(
            lambda match: f"`{modules_root}/{match.group(1)}`",
            content,
        )
    return content + "\n"
```

- [ ] **Step 5: 增加 manifest 与 rules sync 的 Kimi 目标**

```text
kimi_global|global|rules/dist/kimi/AGENTS.md|~/.kimi/AGENTS.md
kimi_modules|global|rules/dist/modules|~/.kimi/modules
```

```bash
agent_installed_for_target() {
  local dst_pat="$1"
  local expanded
  expanded="$(expand_target_path "$dst_pat")"
  case "$expanded" in
    "$HOME"/.claude/*|"$HOME"/.claude)
      command -v claude >/dev/null 2>&1 ;;
    "$HOME"/.gemini/*|"$HOME"/.gemini)
      command -v gemini >/dev/null 2>&1 ;;
    "$HOME"/.cursor/*|"$HOME"/.cursor)
      command -v cursor >/dev/null 2>&1 || [ -d "/Applications/Cursor.app" ] ;;
    "$HOME"/.config/opencode/*|"$HOME"/.config/opencode)
      command -v opencode >/dev/null 2>&1 ;;
    "$HOME"/.codex/*|"$HOME"/.codex)
      [ -d "$HOME/.codex" ] ;;
    "$HOME"/.kimi/*|"$HOME"/.kimi)
      command -v kimi >/dev/null 2>&1 ;;
    *)
      return 0 ;;
  esac
}
```

```python
RULE_TARGET_PREFIX_TO_ID = {
    "claude": "claude-code",
    "gemini": "gemini-cli",
    "cursor": "cursor",
    "opencode": "opencode",
    "codex": "codex",
    "kimi": "kimi-cli",
}
```

- [ ] **Step 6: 重新生成 rules dist 产物**

Run: `make rules-build`

Expected: PASS，并生成 `agent-rule/rules/dist/kimi/AGENTS.md`。

- [ ] **Step 7: 重新运行 rules 相关测试**

Run: `python3 -m unittest tests.test_manage_runtime_probe tests.test_sync_rules -v`

Expected: PASS，`rule_target_agent_id("kimi_global")` 和 `rule_target_agent_id("kimi_modules")` 都映射到 `kimi-cli`，rules sync 可以把新产物分发到 `~/.kimi/AGENTS.md`。

- [ ] **Step 8: 提交这一组变更**

```bash
git add tests/test_sync_rules.py tests/test_manage_runtime_probe.py agent-rule/scripts/build-agent-rules.py config/rules-targets.list agent-rule/scripts/sync-agent-rules.sh src/manage_runtime_probe.py src/manage_doctor.py agent-rule/rules/dist
git commit -m "feat: add kimi cli rules target support"
```

### Task 5: 更新 README / ARCHITECTURE 并锁定文档契约

**Files:**
- Modify: `tests/test_project_contracts.py`
- Modify: `README.md`
- Modify: `docs/ARCHITECTURE.md`

- [ ] **Step 1: 写失败中的 README 与架构文档契约测试**

```python
class ProjectContractTests(unittest.TestCase):
    def test_readme_documents_kimi_cli_management_paths(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("Kimi Code CLI", readme)
        self.assertIn("~/.kimi/mcp.json", readme)
        self.assertIn("~/.kimi/skills", readme)
        self.assertIn("~/.kimi/AGENTS.md", readme)
        self.assertIn("~/.kimi/modules", readme)
        self.assertIn("~/.kimi/config.toml", readme)
        self.assertIn("不在本轮托管范围", readme)

    def test_architecture_documents_kimi_cli_management_paths(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("Kimi Code CLI", architecture)
        self.assertIn("~/.kimi/mcp.json", architecture)
        self.assertIn("~/.kimi/skills", architecture)
        self.assertIn("~/.kimi/AGENTS.md", architecture)
        self.assertIn("~/.kimi/modules", architecture)
        self.assertIn("~/.kimi/config.toml", architecture)
        self.assertIn("不在本轮托管范围", architecture)
```

- [ ] **Step 2: 运行测试确认它们先失败**

Run: `python3 -m unittest tests.test_project_contracts.ProjectContractTests.test_readme_documents_kimi_cli_management_paths tests.test_project_contracts.ProjectContractTests.test_architecture_documents_kimi_cli_management_paths -v`

Expected: FAIL，README 和架构文档还没有 Kimi 管理路径及边界说明。

- [ ] **Step 3: 更新 README 的受管 agent、路径和边界说明**

```md
默认受管 Agent 当前包含 `Claude Code`、`Gemini CLI`、`Kimi Code CLI`、`Cursor IDE` 和 `OpenCode`；其中 `Kimi Code CLI` 的 lifecycle 由 `uv` 管理，MCP / skills / rules 目标分别为 `~/.kimi/mcp.json`、`~/.kimi/skills`、`~/.kimi/AGENTS.md` 和 `~/.kimi/modules`。`~/.kimi/config.toml` 不在本轮托管范围内。
```

- [ ] **Step 4: 更新架构文档的扩展性与 rules 分发说明**

```md
添加新 Agent 时，除 `config/agents.json` 与 `config/manager.json` 外，如需 MCP / skills / rules 支持，还需要分别在 `config/mcp.json`、`config/skills.json` 与 `config/rules-targets.list` 中声明目标路径。`Kimi Code CLI` 当前的受管路径为 `~/.kimi/mcp.json`、`~/.kimi/skills`、`~/.kimi/AGENTS.md` 和 `~/.kimi/modules`；`~/.kimi/config.toml` 保持用户自管。
```

- [ ] **Step 5: 重新运行文档契约测试**

Run: `python3 -m unittest tests.test_project_contracts -v`

Expected: PASS，README 与架构文档都明确记录 Kimi 的受管路径与不托管边界。

- [ ] **Step 6: 提交这一组变更**

```bash
git add tests/test_project_contracts.py README.md docs/ARCHITECTURE.md
git commit -m "docs: document managed kimi cli paths"
```

### Task 6: 运行完整回归并复查范围

**Files:**
- Verify: `tests/test_agent_lifecycle.py`
- Verify: `tests/test_manage_runtime_probe.py`
- Verify: `tests/test_sync_mcp.py`
- Verify: `tests/test_link_skills.py`
- Verify: `tests/test_sync_rules.py`
- Verify: `tests/test_manage_doctor.py`
- Verify: `tests/test_skill_exposure_contracts.py`
- Verify: `tests/test_project_contracts.py`
- Verify: `agent-rule/rules/dist/kimi/AGENTS.md`

- [ ] **Step 1: 运行本次变更的完整定向回归**

Run: `python3 -m unittest tests.test_agent_lifecycle tests.test_manage_runtime_probe tests.test_sync_mcp tests.test_link_skills tests.test_sync_rules tests.test_manage_doctor tests.test_skill_exposure_contracts tests.test_project_contracts -v`

Expected: PASS

- [ ] **Step 2: 重新生成 rules，确认生成产物没有手工漂移**

Run: `make rules-build`

Expected: PASS，`git diff --stat` 只显示本次计划内的源码、配置、测试、文档和生成产物变化。

- [ ] **Step 3: 复查不在范围内的 Kimi 配置没有被接入托管**

Run: `rg -n "~/.kimi/config.toml|config.toml" config src README.md docs tests`

Expected: 只出现文档层面的“不在本轮托管范围”说明，不出现新的 `sync` 或 `write` 逻辑把 `~/.kimi/config.toml` 纳入托管。

- [ ] **Step 4: 做最终提交**

```bash
git add config/agents.json config/manager.json config/mcp.json config/skills.json config/rules-targets.list src/agent_ids.py src/manage_runtime_probe.py src/manage_doctor.py agent-rule/scripts/build-agent-rules.py agent-rule/scripts/sync-agent-rules.sh agent-rule/rules/dist README.md docs/ARCHITECTURE.md tests/test_agent_lifecycle.py tests/test_manage_runtime_probe.py tests/test_sync_mcp.py tests/test_link_skills.py tests/test_sync_rules.py tests/test_manage_doctor.py tests/test_skill_exposure_contracts.py tests/test_project_contracts.py
git commit -m "feat: manage kimi cli with lifecycle mcp rules and skills"
```
