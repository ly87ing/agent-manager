import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ProjectContractTests(unittest.TestCase):
    def test_init_secrets_guidance_uses_update_or_init(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("make update", script)
        self.assertIn("make init", script)
        self.assertNotIn("初始化完成！现在可以运行：make all", script)

    def test_init_secrets_requires_explicit_repo_configuration(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("SECRETS_REPO_URL", script)
        self.assertNotIn("https://github.com/ly87ing/secrets.git", script)

    def test_makefile_exposes_smoke_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(".PHONY: help init update agents-update agents\\:update doctor smoke", makefile)
        self.assertIn("\nsmoke:\n\t@./scripts/ci-smoke.sh\n", makefile)


    def test_makefile_supports_colon_manage_subcommands(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            (temp_root / "Makefile").write_text((ROOT / "Makefile").read_text(encoding="utf-8"), encoding="utf-8")
            call_log = temp_root / "calls.log"
            (temp_root / "manage.sh").write_text(
                (
                    "#!/usr/bin/env bash\n"
                    f"printf '%s\\n' \"$*\" >> {str(call_log)!r}\n"
                    "exit 0\n"
                ),
                encoding="utf-8",
            )
            (temp_root / "manage.sh").chmod(0o755)

            for goal in ("agents:update",):
                result = subprocess.run(
                    ["make", goal],
                    cwd=temp_root,
                    capture_output=True,
                    text=True,
                    check=False,
                )

                self.assertEqual(result.returncode, 0, f"{goal}: {result.stdout}{result.stderr}")

            self.assertEqual(
                call_log.read_text(encoding="utf-8").splitlines(),
                ["agents:update"],
            )

    def test_ci_smoke_script_exercises_safe_preview_flows(self):
        smoke_script = ROOT / "scripts" / "ci-smoke.sh"
        self.assertTrue(smoke_script.exists(), "missing ci smoke script")

        content = smoke_script.read_text(encoding="utf-8")
        self.assertIn("run python3 ./scripts/check_no_secrets.py", content)
        self.assertIn("AGENT_MANAGER_BASE_CONFIG", content)
        self.assertIn("manager.smoke.json", content)
        self.assertIn("AGENT_MANAGER_LOCK_FILE", content)
        self.assertIn("AGENT_MANAGER_LOG_PREFIX", content)
        self.assertIn("run ./manage.sh help", content)
        self.assertIn("./manage.sh update --dry-run", content)
        self.assertIn("./manage.sh agents:update --dry-run", content)
        self.assertIn("./manage.sh init --dry-run", content)
        self.assertIn("make -n help init update agents-update doctor smoke", content)
        self.assertNotIn("./manage.sh bootstrap", content)

    def test_ci_workflow_runs_full_unittest_suite(self):
        workflow = ROOT / ".github" / "workflows" / "ci.yml"
        self.assertTrue(workflow.exists(), "missing GitHub Actions workflow")

        content = workflow.read_text(encoding="utf-8")
        self.assertIn("push:", content)
        self.assertIn("pull_request:", content)
        self.assertIn("workflow_dispatch:", content)
        self.assertIn("ubuntu-latest", content)
        self.assertIn("macos-latest", content)
        self.assertIn('- "3.9"', content)
        self.assertIn('- "3.10"', content)
        self.assertIn('- "3.11"', content)
        self.assertIn("Install tomli for Python < 3.11", content)
        self.assertIn("matrix.python-version != '3.11'", content)
        self.assertIn("python3 -m pip install tomli", content)
        self.assertIn("Run tracked secret scan", content)
        self.assertIn("python3 ./scripts/check_no_secrets.py", content)
        self.assertIn("python3 -m unittest discover -s tests -p 'test*.py' -v", content)
        self.assertIn("Run smoke validation", content)
        self.assertTrue("make smoke" in content or "./scripts/ci-smoke.sh" in content)

    def test_python_subengines_share_runtime_log_level_support(self):
        for rel_path in (
            "agent-lifecycle/sync_agents.py",
            "agent-tool/sync_tools.py",
            "agent-skill/link_skills.py",
            "agent-mcp/sync_mcp.py",
        ):
            content = (ROOT / rel_path).read_text(encoding="utf-8")
            self.assertIn("from src.runtime_logging import configure_logging", content, rel_path)
            self.assertIn("configure_logging()", content, rel_path)

    def test_troubleshooting_log_level_guidance_matches_runtime(self):
        troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
        self.assertIn("LOG_LEVEL=DEBUG ./manage.sh init", troubleshooting)
        self.assertNotIn("永久修改 manage.sh 中的 logging.basicConfig level", troubleshooting)

    def test_troubleshooting_mentions_rules_manifest_and_policy_scoped_dependencies(self):
        troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
        self.assertIn("如当前策略启用了 brew / npm / gh provider", troubleshooting)
        self.assertIn("rules-targets.list: 存在", troubleshooting)

    def test_troubleshooting_uses_latest_log_and_skill_cache_paths(self):
        troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
        self.assertIn("agent_manager_sync_<user>.latest", troubleshooting)
        self.assertIn("agent_manager_sync_<user>_<runid>.log", troubleshooting)
        self.assertIn("agent-skill/repositories/", troubleshooting)
        self.assertIn("find agent-skill/repositories/agent-skills -name SKILL.md", troubleshooting)
        self.assertNotIn("/tmp/agent_manager_sync_$(whoami).log*", troubleshooting)
        self.assertNotIn("agent-skill/<skill-name>", troubleshooting)
        self.assertNotIn("agent-skill/repositories/agent-skills/SKILL.md", troubleshooting)

    def test_docs_explain_tomli_fallback_for_toml_validation(self):
        troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
        mcp_guide = (ROOT / "docs" / "MCP-CONFIG-GUIDE.md").read_text(encoding="utf-8")
        self.assertIn("import tomli as tomllib", troubleshooting)
        self.assertIn("import tomli as tomllib", mcp_guide)
        self.assertIn("pip install tomli", troubleshooting)
        self.assertIn("pip install tomli", mcp_guide)

    def test_readme_atomic_write_contract_matches_runtime_scope(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("配置文件采用“临时文件原子替换”模式", readme)
        self.assertIn("目录型 rules 则采用收敛式同步", readme)
        self.assertNotIn("所有配置文件及规则文件均采用“临时文件原子替换”模式", readme)

    def test_readme_cleanup_contract_matches_runtime_scope(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("精准回收受管备份、碎片及冗余日志", readme)
        self.assertNotIn("孤儿仓库", readme)

    def test_readme_documents_local_override_and_terminal_boundary(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("manager.local.json", readme)
        self.assertIn("browser_us_sync.enabled", readme)
        self.assertIn("不接管终端配置", readme)

    def test_readme_documents_rules_source_build_dist_flow(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/", readme)
        self.assertIn("agent-rule/rules/dist/", readme)
        self.assertIn("agent-rule/scripts/build-agent-rules.py", readme)
        self.assertIn("AGENTS.md", readme)
        self.assertIn("CLAUDE.md", readme)

    def test_readme_documents_shared_rule_design_principles(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/shared/core.md", readme)
        self.assertIn("显式说明假设", readme)
        self.assertIn("最简单本地改动", readme)
        self.assertIn("surgical edits", readme)
        self.assertIn("verification point", readme)
        self.assertIn("低频专题进 skill", readme)
        self.assertIn("change-discipline", readme)
        self.assertIn("reader-facing-writing", readme)
        self.assertIn("agent-rule/rules/src/agents/*/adapter.md", readme)

    def test_readme_documents_scoped_claude_memory_layout(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("根目录 `CLAUDE.md`", readme)
        self.assertIn("agent-rule/CLAUDE.md", readme)
        self.assertIn("tests/CLAUDE.md", readme)
        self.assertIn("仓库级高频记忆", readme)

    def test_readme_documents_secret_scan_target(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("scripts/check_no_secrets.py", readme)
        self.assertIn("主仓库禁存 secrets", readme)
        self.assertIn("疑似明文敏感信息", readme)

    def test_readme_documents_codex_hooks_workflow(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("config/codex-hooks.json", readme)
        self.assertIn(".codex/hooks.json", readme)
        self.assertIn("随 `make update` 刷新", readme)

    def test_readme_documents_official_cli_lifecycle_policy(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("按两个当前受管 Agent 的官方推荐方式维护", readme)
        self.assertIn("仅用于识别并提示迁移", readme)
        self.assertIn("默认自动安装的 Agent 当前只包含 `Claude Code` 与 `Codex`", readme)
        self.assertIn("`Claude Code` 与 `Codex` 默认走官方 npm 包，`config/agents.json` 中的安装/升级命令刻意不锁版本", readme)
        self.assertIn("`Claude Code native` 仍作为兼容 provider 保留官方 install / update / uninstall 语义", readme)
        self.assertIn("`Claude Code` 和 `Codex` 的 npm provider 使用 `config/agents.json` 中的命令模板", readme)
        self.assertIn("`agent-manager` 不改写用户 shell profile", readme)
        self.assertIn("`Claude Code local install` 不纳入 `agent-manager` lifecycle 托管", readme)

    def test_readme_documents_current_agent_scope(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("对 `Claude Code` 与 `Codex` 做 Agent 生命周期、MCP、skills 和 rules 管理，并对 `Kimi Code` 做 MCP 与 skills 管理（不接管其生命周期与 rules）", readme)
        self.assertIn("│ Claude Code", readme)
        self.assertIn("│ Codex", readme)
        self.assertIn("│ Kimi Code", readme)

    def test_troubleshooting_documents_official_lifecycle_gap_guidance(self):
        troubleshooting = (ROOT / "docs" / "TROUBLESHOOTING.md").read_text(encoding="utf-8")
        heading = "### 问题：检测到官方生命周期动作缺口"
        self.assertIn(heading, troubleshooting)
        section = troubleshooting.split(heading, 1)[1].split("\n### ", 1)[0]
        self.assertIn("官方生命周期动作缺口", section)
        self.assertIn("官方当前未提供受管 uninstall 动作", section)
        self.assertIn("fail-close", section)
        self.assertIn("不会伪造删除命令", section)

    def test_readme_documents_agent_skills_scope_matches_config(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        agent_skills = skills_config["repositories"]["agent-skills"]

        self.assertIn("`agent-skills` 面向 `Claude Code`、`Codex` 与 `Kimi Code` 暴露", readme)
        for skill_path in agent_skills["skill_paths"]:
            self.assertIn(f"`{skill_path}`", readme)
        self.assertIn(f"这 {agent_skills['skill_count']} 个 skill 包", readme)

    def test_readme_documents_isolated_chrome_devtools_mcp(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("chrome-devtools-mcp --isolated", readme)
        self.assertIn("避免多个 agent 共享同一个 browser profile", readme)

    def test_committed_runtime_configs_do_not_use_dynamic_latest(self):
        for rel_path in ("config/agents.json", "config/tools.json", "config/mcp.json"):
            content = (ROOT / rel_path).read_text(encoding="utf-8")
            self.assertNotIn("@latest", content, rel_path)

    def test_claude_code_credentials_are_left_to_cc_switch(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        manage_sh = (ROOT / "manage.sh").read_text(encoding="utf-8")
        secrets_doc = (ROOT / "docs" / "SECRETS-MANAGEMENT.md").read_text(encoding="utf-8")
        public_rule = json.loads((ROOT / "config" / "claude-env.json").read_text(encoding="utf-8"))

        self.assertNotIn("claude-env-sync", makefile)
        self.assertNotIn("claude:env:sync", manage_sh)
        self.assertNotIn("run_claude_env_sync_task", manage_sh)
        self.assertEqual(public_rule["settings_path"], "~/.claude/settings.json")
        self.assertEqual(
            public_rule["env"],
            {
                "ANTHROPIC_AUTH_TOKEN": "${ANTHROPIC_AUTH_TOKEN}",
                "ANTHROPIC_BASE_URL": "${ANTHROPIC_BASE_URL}",
            },
        )
        self.assertFalse((ROOT / "scripts" / "claude" / "sync_env.py").exists())
        self.assertFalse((ROOT / "tests" / "test_sync_claude_env.py").exists())
        self.assertIn("Claude Code 凭证不再由 agent-manager 注入", secrets_doc)
        self.assertIn("config/claude-env.json", secrets_doc)
        self.assertIn("cc-switch", secrets_doc)

    def test_repo_root_agents_guidance_covers_setup_tests_done_when_and_generated_boundaries(self):
        agents = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
        self.assertIn("Python 3 and make", agents)
        self.assertIn("python3 -m unittest discover -s tests -p 'test*.py' -v", agents)
        self.assertIn("make smoke", agents)
        self.assertIn("done-when", agents)
        self.assertIn("generated", agents)
        self.assertIn("agent-rule/rules/dist/", agents)

    def test_agent_rule_agents_guidance_stays_narrow_and_build_focused(self):
        agents = (ROOT / "agent-rule" / "AGENTS.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/", agents)
        self.assertIn("agent-rule/rules/dist/", agents)
        self.assertIn("make update", agents)
        self.assertIn("agent-rule/scripts/build-agent-rules.py", agents)
        self.assertNotIn("python3 -m unittest discover -s tests -p 'test*.py' -v", agents)
        self.assertNotIn("## Setup", agents)
        self.assertNotIn("## Test commands", agents)
        self.assertNotIn("## done-when", agents)
        self.assertNotIn("README and docs match the implemented behavior.", agents)
        self.assertNotIn("No generated file was hand-edited.", agents)
        self.assertNotIn("make smoke", agents)

    def test_tests_agents_guidance_stays_narrow_and_contract_focused(self):
        agents = (ROOT / "tests" / "AGENTS.md").read_text(encoding="utf-8")
        self.assertIn("python3 -m unittest tests.test_project_contracts -v", agents)
        self.assertIn("fixtures", agents)
        self.assertIn("contract", agents)
        self.assertIn("keep each test focused", agents)
        self.assertNotIn("make rules-build", agents)
        self.assertNotIn("## Setup", agents)
        self.assertNotIn("## Test commands", agents)
        self.assertNotIn("## done-when", agents)
        self.assertNotIn("README and docs match the implemented behavior.", agents)
        self.assertNotIn("No generated file was hand-edited.", agents)
        self.assertNotIn("make smoke", agents)

    def test_architecture_documents_dry_run_contract(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("--dry-run", architecture)
        self.assertIn("只输出计划动作", architecture)

    def test_architecture_documents_local_override_and_terminal_boundary(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("manager.local.json", architecture)
        self.assertIn("browser_us_sync.enabled", architecture)
        self.assertIn("不接管终端配置", architecture)

    def test_architecture_documents_doctor_python_module(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        manage_script = (ROOT / "manage.sh").read_text(encoding="utf-8")
        self.assertIn("src/manage_doctor.py", architecture)
        self.assertIn("src.manage_doctor", manage_script)

    def test_architecture_documents_rules_build_flow(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/scripts/build-agent-rules.py", architecture)
        self.assertIn("agent-rule/rules/src/", architecture)
        self.assertIn("agent-rule/rules/dist/", architecture)
        self.assertIn("不再保留第二条平行的规则文本流", architecture)
        self.assertIn("{{REPO_ROOT}}", architecture)
        self.assertIn("dist/codex/AGENTS.md", architecture)
        self.assertIn("dist/claude/CLAUDE.md", architecture)

    def test_architecture_documents_codex_hooks_pipeline(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("config/codex-hooks.json", architecture)
        self.assertIn("agent-rule/scripts/sync-codex-hooks.py", architecture)
        self.assertIn(".codex/hooks.json", architecture)
        self.assertIn("随 `make update` 管线同步", architecture)

    def test_architecture_documents_current_agent_scope(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("对 `Claude Code` 与 `Codex` 做 Agent 生命周期、MCP、skills 和 rules 管理，并对 `Kimi Code` 做 MCP 与 skills 管理（不接管其生命周期与 rules）", architecture)
        self.assertIn("当前默认受管目标：生命周期与 rules 只包含 `Claude Code` 与 `Codex`；MCP 与 skills target 额外包含 `Kimi Code`", architecture)
        self.assertIn("如需启用某类能力，再声明对应的 MCP / skills / rules target path", architecture)
        self.assertNotIn("新增 Agent 时必须同时声明 MCP / skills / rules 三类 target path", architecture)

    def test_claude_memory_stays_project_specific(self):
        memory = (ROOT / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertNotIn("Primary Semantic Search", memory)
        self.assertNotIn("Verification is CRITICAL", memory)
        self.assertIn("config/rules-targets.list", memory)

    def test_claude_memory_matches_codex_lifecycle_policy(self):
        memory = (ROOT / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("`codex` is under Agent lifecycle management", memory)
        self.assertNotIn("`codex` is only a distribution target for MCP, Rules, and Skills and is not under lifecycle management.", memory)

    def test_root_claude_md_stays_concise_and_points_to_scoped_memory(self):
        memory = (ROOT / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertLessEqual(len(memory.splitlines()), 80)
        self.assertIn("make smoke", memory)
        self.assertIn("make update", memory)
        self.assertIn("make doctor", memory)
        self.assertIn("agent-rule/CLAUDE.md", memory)
        self.assertIn("tests/CLAUDE.md", memory)
        self.assertNotIn("Agent 配置文件路径", memory)
        self.assertNotIn("agent-lifecycle | `sync_agents.py`", memory)

    def test_agent_rule_claude_md_stays_narrow_and_build_focused(self):
        memory = (ROOT / "agent-rule" / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/", memory)
        self.assertIn("agent-rule/rules/dist/", memory)
        self.assertIn("make update", memory)
        self.assertNotIn("python3 -m unittest discover -s tests -p 'test*.py' -v", memory)
        self.assertNotIn("make smoke", memory)
        self.assertNotIn("manage.sh 是唯一顶层编排入口", memory)

    def test_tests_claude_md_stays_narrow_and_contract_focused(self):
        memory = (ROOT / "tests" / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("python3 -m unittest tests.test_project_contracts -v", memory)
        self.assertIn("fixtures", memory)
        self.assertIn("contract", memory)
        self.assertIn("keep each test focused", memory)
        self.assertNotIn("make rules-build", memory)
        self.assertNotIn("make smoke", memory)
        self.assertNotIn("manage.sh 是唯一顶层编排入口", memory)

    def test_every_manage_command_is_documented_in_help(self):
        # help 自称完整命令清单。新增 case 分支若不登记，用户就无处可查。
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        case_body = manage.split('case "$cmd" in', 1)[1].split("\nesac", 1)[0]

        commands = set()
        for line in case_body.splitlines():
            stripped = line.strip()
            if not stripped.endswith(")") or stripped.startswith("#"):
                continue
            label = stripped[:-1]
            if "(" in label or " " in label:
                continue
            for token in label.split("|"):
                if token and token != "*" and not token.startswith("-"):
                    commands.add(token)

        self.assertIn("update", commands)
        self.assertIn("agents:update", commands)

        help_text = subprocess.run(
            [str(ROOT / "manage.sh"), "help"], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout

        missing = sorted(command for command in commands if command not in help_text)
        self.assertEqual(missing, [], f"这些命令在 help 里查不到: {missing}")

    def test_documented_make_targets_all_exist_in_makefile(self):
        # 文档里推荐过的 make 目标必须真实存在。README 曾长期推荐
        # make claude-hooks-sync / codex-hooks-sync / restore 三个不存在的目标。
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        targets = set()
        for line in makefile.splitlines():
            if line.startswith("\t") or not line.strip() or line.lstrip().startswith("#"):
                continue
            head, sep, _ = line.replace("\\:", "\x00").partition(":")
            if not sep:
                continue
            for name in head.replace("\x00", ":").split():
                if name != ".PHONY":
                    targets.add(name)
        for name in makefile.split(".PHONY:", 1)[1].splitlines()[0].replace("\\:", ":").split():
            targets.add(name)

        documented = {}
        for path in [ROOT / "README.md", ROOT / "CLAUDE.md", *sorted((ROOT / "docs").glob("*.md"))]:
            for match in re.finditer(r"make ([a-z][a-z0-9:_-]*)", path.read_text(encoding="utf-8")):
                documented.setdefault(match.group(1), path.relative_to(ROOT).as_posix())

        missing = sorted((name, where) for name, where in documented.items() if name not in targets)
        self.assertEqual(missing, [], f"文档推荐了 Makefile 里不存在的目标: {missing}")

    def test_gui_ci_workflow_covers_engine_app_and_experimental_windows(self):
        workflow = ROOT / ".github" / "workflows" / "gui-ci.yml"
        self.assertTrue(workflow.exists(), "missing GUI CI workflow")

        content = workflow.read_text(encoding="utf-8")
        self.assertIn("engine-rs:", content)
        self.assertIn("app:", content)
        self.assertIn("app-windows:", content)
        self.assertIn("ubuntu-latest", content)
        self.assertIn("macos-latest", content)
        self.assertIn("windows-latest", content)
        self.assertIn("dtolnay/rust-toolchain@stable", content)
        self.assertIn("actions/cache@v4", content)
        self.assertIn("python3 -m unittest tests.test_engine_rs_parity tests.test_engine_cli -v", content)
        self.assertIn("pnpm typecheck", content)
        self.assertIn("pnpm test:unit run", content)
        self.assertIn("pnpm tauri build --bundles app", content)
        windows_body = content.split("app-windows:", 1)[1]
        self.assertIn("continue-on-error: true", windows_body)
        # 反例：CI 工作流不得引用任何签名密钥 secret（签名只属于 release 管线）。
        self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY", content)

    def test_gui_release_workflow_gates_signing_on_secret_presence(self):
        workflow = ROOT / ".github" / "workflows" / "gui-release.yml"
        self.assertTrue(workflow.exists(), "missing GUI release workflow")

        content = workflow.read_text(encoding="utf-8")
        self.assertIn('tags:', content)
        self.assertIn('"v*"', content)
        self.assertIn("tauri-apps/tauri-action@v0", content)
        self.assertIn("secrets.TAURI_SIGNING_PRIVATE_KEY", content)
        self.assertIn("if: ${{ env.TAURI_SIGNING_PRIVATE_KEY != '' }}", content)
        self.assertIn("createUpdaterArtifacts", content)
        self.assertIn("latest.json", content)
        # 反例：工作流只引用 secret 名称，不得包含任何密钥材料或占位公钥。
        self.assertNotIn("-----BEGIN", content)
        self.assertNotIn("YOUR_UPDATER_PUBKEY", content)

    def test_updater_config_uses_placeholder_pubkey_and_github_endpoint(self):
        config = json.loads((ROOT / "app" / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
        updater = config["plugins"]["updater"]
        self.assertEqual(
            updater["endpoints"],
            ["https://github.com/ly87ing/agent-manager/releases/latest/download/latest.json"],
        )
        # 反例：pubkey 必须是占位符，不允许出现真实公钥材料。
        self.assertEqual(updater["pubkey"], "YOUR_UPDATER_PUBKEY")

        capabilities = (ROOT / "app" / "src-tauri" / "capabilities" / "default.json").read_text(encoding="utf-8")
        self.assertIn("updater:default", capabilities)
        main_rs = (ROOT / "app" / "src-tauri" / "src" / "main.rs").read_text(encoding="utf-8")
        self.assertIn("tauri_plugin_updater::Builder::new().build()", main_rs)
        cargo_toml = (ROOT / "app" / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn('tauri-plugin-updater = "2"', cargo_toml)

    def test_gui_platform_status_reports_unverified_platforms_honestly(self):
        status = ROOT / "docs" / "GUI-PLATFORM-STATUS.md"
        self.assertTrue(status.exists(), "missing GUI platform status doc")

        content = status.read_text(encoding="utf-8")
        # macOS: 明确记录本机范围且属于本地候选/未发布范围
        self.assertIn("Darwin x86_64", content)
        self.assertIn("standalone", content)
        self.assertIn("本地候选", content)
        self.assertIn("未发布", content)
        # Linux: 明确桌面环境未验证
        self.assertIn("未在真实 Linux 桌面环境运行验证", content)
        self.assertIn("构建/引擎证据不等于桌面运行验收", content)
        # Windows: 明确 Bash 依赖限制与实验性门控
        self.assertIn("不支持独立 Bash 运行链，未进行桌面运行验收", content)
        self.assertIn("实验性，不宣称可用", content)
        self.assertIn("python-build-standalone", content)
        self.assertIn("continue-on-error: true", content)
        # 签名与自更新：明确未配置签名与占位符
        self.assertIn("当前未进行签名发布", content)
        self.assertIn("YOUR_UPDATER_PUBKEY", content)
        self.assertIn("TAURI_SIGNING_PRIVATE_KEY", content)
        self.assertIn("secret 不存在时跳过签名，只产未签名安装器", content)
        self.assertIn("SyncPage", content)
        # 强反例：状态报告不得宣称 Windows 完成验证，亦不得宣称全量实测通过或 CI 本轮已跑/全绿
        self.assertNotIn("Windows 全量实测", content)
        self.assertNotIn("全量实测通过", content)
        self.assertNotIn("CI本轮已跑", content)
        self.assertNotIn("C全绿", content)


if __name__ == "__main__":
    unittest.main()
