import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ProjectContractTests(unittest.TestCase):
    def test_init_secrets_guidance_uses_bump_or_init(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("make bump", script)
        self.assertIn("make init", script)
        self.assertNotIn("初始化完成！现在可以运行：make all", script)

    def test_init_secrets_requires_explicit_repo_configuration(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("SECRETS_REPO_URL", script)
        self.assertNotIn("https://github.com/ly87ing/secrets.git", script)

    def test_makefile_exposes_smoke_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(".PHONY: init apply bump update plan", makefile)
        self.assertIn("\nsmoke:\n\t@./scripts/ci-smoke.sh\n", makefile)


    def _extract_shell_function(self, source: str, name: str) -> str:
        header = f"function {name}() {{\n"
        start = source.index(header)
        end = source.index("\n}\n", start)
        return source[start:end]

    def _expand_pipeline_body(self, source: str, name: str) -> str:
        """展开管线函数：apply/bump/update/init 只组合 run_bump_steps / run_apply_steps，
        断言步骤集合前需要把这两个组合函数的函数体内联回来。"""
        body = self._extract_shell_function(source, name)
        for helper_name in ("run_bump_steps", "run_apply_steps"):
            if helper_name in body:
                body = body.replace(helper_name, self._extract_shell_function(source, helper_name))
        return body

    def test_bump_pipeline_refreshes_resources_without_agent_or_tool_lifecycle(self):
        manage = (ROOT / "manage.sh").read_text(encoding="utf-8")
        body = self._expand_pipeline_body(manage, "run_bump_pipeline")
        # bump 联网拉最新，需先刷新 secrets 仓库，否则新 MCP 配置会配不上新 key
        self.assertIn("maybe_init_secrets", body, "bump 应在拉取前刷新 secrets 凭证")
        # bump 拉取远端最新资源并下发：版本锁 / skill 版本
        for step in (
            "run_pipeline_step run_version_pins_update_task",
            "run_pipeline_step run_skills_upgrade_task",
        ):
            self.assertIn(step, body, f"bump 缺少步骤: {step}")
        # bump 明确不触碰 Agent / Tool / browser / shell 生命周期等
        for excluded in (
            "run_agent_lifecycle_task",
            "run_tools_sync_task",
            "run_browser_us_sync_task",
            "run_shell_aliases_sync_task",
            "run_rules_sync_task",
            "run_skills_link_task",
            "run_mcp_sync_task",
        ):
            self.assertNotIn(excluded, body, f"bump 不应包含重活步骤: {excluded}")


    def test_makefile_exposes_secrets_check_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("secrets-check", makefile)
        self.assertIn("secrets\\:check", makefile)
        self.assertIn("\nsecrets-check:\n\t@python3 ./scripts/check_no_secrets.py\n", makefile)

    def test_makefile_exposes_rules_build_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("rules-build", makefile)
        self.assertIn("rules\\:build", makefile)
        self.assertIn("\nrules-build:\n\t@./manage.sh rules:build\n", makefile)

    def test_makefile_exposes_instruction_quality_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("instructions-check", makefile)
        self.assertIn("instructions\\:check", makefile)
        self.assertIn("\ninstructions-check:\n", makefile)
        self.assertIn("tests.test_project_contracts", makefile)
        self.assertIn("tests.test_rule_budgets", makefile)
        self.assertIn("tests.test_rule_layout", makefile)
        self.assertIn("tests.test_skill_exposure_contracts", makefile)
        self.assertIn("tests.test_sync_rules", makefile)
        self.assertIn("tests.test_sync_claude_hooks", makefile)
        self.assertIn("tests.test_sync_codex_hooks", makefile)
        self.assertIn("tests.test_rule_effectiveness_eval", makefile)
        self.assertIn("tests.test_skill_usage_metrics", makefile)
        self.assertIn(
            "run_rule_effectiveness_eval.py --baseline ./scripts/evals/fixtures/rule_effectiveness_baseline.json",
            makefile,
        )
        self.assertIn("\ninstructions\\:check: instructions-check\n", makefile)

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

            for goal in (
                "state:list",
                "state:restore",
                "rules:build",
                "rules:apply",
                "claude:hooks:apply",
                "codex:hooks:apply",
                "skills:apply",
                "skills:bump",
                "mcp:apply",
                "tools:apply",
                "browser:apply",
                "help:all",
            ):
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
                [
                    "state:list",
                    "state:restore",
                    "rules:build",
                    "rules:apply",
                    "claude:hooks:apply",
                    "codex:hooks:apply",
                    "skills:apply",
                    "skills:bump",
                    "mcp:apply",
                    "tools:apply",
                    "browser:apply",
                    "help:all",
                ],
            )

    def test_ci_smoke_script_exercises_safe_preview_flows(self):
        smoke_script = ROOT / "scripts" / "ci-smoke.sh"
        self.assertTrue(smoke_script.exists(), "missing ci smoke script")

        content = smoke_script.read_text(encoding="utf-8")
        self.assertIn("run make secrets-check", content)
        self.assertIn("AGENT_MANAGER_BASE_CONFIG", content)
        self.assertIn("manager.smoke.json", content)
        self.assertIn("AGENT_MANAGER_LOCK_FILE", content)
        self.assertIn("AGENT_MANAGER_LOG_PREFIX", content)
        self.assertIn("run ./manage.sh help:all", content)
        self.assertIn("./manage.sh apply --dry-run", content)
        self.assertIn("./manage.sh bump --dry-run", content)
        self.assertIn("./manage.sh update --dry-run", content)
        self.assertIn("./manage.sh agents:apply --dry-run", content)
        self.assertIn("./manage.sh plan", content)
        self.assertIn("./manage.sh sys:upgrade --dry-run", content)
        self.assertIn("make -n apply bump update agents-apply init plan sys-upgrade help-all", content)
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
        self.assertIn("make secrets-check", content)
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
        self.assertIn("make rules-build", readme)
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

    def test_readme_documents_instruction_quality_regression_target(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("make instructions:check", readme)

    def test_readme_documents_secret_scan_target(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("make secrets-check", readme)
        self.assertIn("主仓库禁存 secrets", readme)
        self.assertIn("疑似明文敏感信息", readme)
        self.assertIn("共享指令", readme)
        self.assertIn("hooks", readme)
        self.assertIn("回归门", readme)
        self.assertIn("硬门禁", readme)
        self.assertIn("required_agents", readme)
        self.assertIn("success / verification 阈值", readme)

    def test_readme_documents_codex_hooks_workflow(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("config/codex-hooks.json", readme)
        self.assertIn(".codex/hooks.json", readme)
        self.assertIn("make codex:hooks:apply", readme)

    def test_readme_documents_official_cli_lifecycle_policy(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("按两个当前受管 Agent 的官方推荐方式维护", readme)
        self.assertIn("仅用于识别并提示迁移", readme)
        self.assertIn("默认自动安装的 Agent 当前只包含 `Claude Code` 与 `Codex`", readme)
        self.assertIn("`Claude Code` 与 `Codex` 默认走官方 npm 包，并使用 `config/agents.json` 中的命令模板加本机版本锁", readme)
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
        self.assertIn("chrome-devtools-mcp@<固定版本> --isolated", readme)
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
        self.assertIn("make rules-build", agents)
        self.assertIn("make rules:apply", agents)
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

    def test_architecture_dry_run_contract_mentions_local_maintenance_writes(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("sys:clean", architecture)
        self.assertIn("state:restore", architecture)
        self.assertIn("sys:clean-logs", architecture)
        self.assertIn("会修改本地状态的维护动作", architecture)

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

    def test_architecture_documents_instruction_quality_regression_target(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("make instructions:check", architecture)
        self.assertIn("共享指令", architecture)
        self.assertIn("skills exposure", architecture)
        self.assertIn("managed hooks", architecture)
        self.assertIn("硬门禁", architecture)
        self.assertIn("required_agents", architecture)
        self.assertIn("success / verification", architecture)

    def test_architecture_documents_codex_hooks_pipeline(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("config/codex-hooks.json", architecture)
        self.assertIn("agent-rule/scripts/sync-codex-hooks.py", architecture)
        self.assertIn(".codex/hooks.json", architecture)
        self.assertIn("make codex-hooks-apply", architecture)

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
        self.assertIn("make apply", memory)
        self.assertIn("make doctor", memory)
        self.assertIn("agent-rule/CLAUDE.md", memory)
        self.assertIn("tests/CLAUDE.md", memory)
        self.assertNotIn("Agent 配置文件路径", memory)
        self.assertNotIn("agent-lifecycle | `sync_agents.py`", memory)

    def test_agent_rule_claude_md_stays_narrow_and_build_focused(self):
        memory = (ROOT / "agent-rule" / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/", memory)
        self.assertIn("agent-rule/rules/dist/", memory)
        self.assertIn("make rules-build", memory)
        self.assertIn("make rules:apply", memory)
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

    def test_every_manage_command_is_documented_in_help_or_help_all(self):
        # help:all 自称「完整命令」。新增 case 分支若不登记，用户就无处可查——
        # agents:status 与 claude:shell-aliases:apply 曾经两层都查不到。
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
        self.assertIn("help:all", commands)

        help_text = subprocess.run(
            [str(ROOT / "manage.sh"), "help"], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout
        help_all_text = subprocess.run(
            [str(ROOT / "manage.sh"), "help:all"], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout
        documented = help_text + help_all_text

        missing = sorted(command for command in commands if command not in documented)
        self.assertEqual(missing, [], f"这些命令在 help 与 help:all 里都查不到: {missing}")

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


if __name__ == "__main__":
    unittest.main()
