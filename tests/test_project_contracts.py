import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ProjectContractTests(unittest.TestCase):
    def test_init_secrets_guidance_uses_update_or_bootstrap(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("make update", script)
        self.assertIn("make bootstrap", script)
        self.assertNotIn("初始化完成！现在可以运行：make all", script)

    def test_init_secrets_requires_explicit_repo_configuration(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("SECRETS_REPO_URL", script)
        self.assertNotIn("https://github.com/ly87ing/secrets.git", script)

    def test_makefile_exposes_smoke_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(".PHONY: all bootstrap sync update plan self-update smoke", makefile)
        self.assertIn("\nsmoke:\n\t@./scripts/ci-smoke.sh\n", makefile)

    def test_makefile_exposes_rules_build_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("rules-build", makefile)
        self.assertIn("rules\\:build", makefile)
        self.assertIn("\nrules-build:\n\t@./manage.sh rules:build\n", makefile)

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
                "backups:list",
                "backups:restore",
                "rules:build",
                "rules:sync",
                "skills:link",
                "skills:upgrade",
                "mcp:sync",
                "mcp:prepare",
                "tools:sync",
                "terminal:sync",
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
                    "backups:list",
                    "backups:restore",
                    "rules:build",
                    "rules:sync",
                    "skills:link",
                    "skills:upgrade",
                    "mcp:sync",
                    "mcp:prepare",
                    "tools:sync",
                    "terminal:sync",
                ],
            )

    def test_ci_smoke_script_exercises_safe_preview_flows(self):
        smoke_script = ROOT / "scripts" / "ci-smoke.sh"
        self.assertTrue(smoke_script.exists(), "missing ci smoke script")

        content = smoke_script.read_text(encoding="utf-8")
        self.assertIn("AGENT_MANAGER_BASE_CONFIG", content)
        self.assertIn("manager.smoke.json", content)
        self.assertIn("AGENT_MANAGER_LOCK_FILE", content)
        self.assertIn("AGENT_MANAGER_LOG_PREFIX", content)
        self.assertIn("./manage.sh sync --dry-run", content)
        self.assertIn("./manage.sh update --dry-run", content)
        self.assertIn("./manage.sh plan", content)
        self.assertIn("./manage.sh self-update --dry-run", content)
        self.assertIn("make -n sync update bootstrap all plan self-update offline dry-run", content)
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
        self.assertIn("LOG_LEVEL=DEBUG ./manage.sh bootstrap", troubleshooting)
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
        self.assertIn("find agent-skill/repositories/playwright-best-practices-skill -name SKILL.md", troubleshooting)
        self.assertNotIn("/tmp/agent_manager_sync_$(whoami).log*", troubleshooting)
        self.assertNotIn("agent-skill/<skill-name>", troubleshooting)
        self.assertNotIn("agent-skill/repositories/playwright-best-practices-skill/SKILL.md", troubleshooting)

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

    def test_readme_documents_local_override_and_terminal_opt_in(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("manager.local.json", readme)
        self.assertIn("终端配置默认关闭", readme)

    def test_readme_documents_rules_source_build_dist_flow(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/", readme)
        self.assertIn("agent-rule/rules/dist/", readme)
        self.assertIn("make rules-build", readme)

    def test_readme_documents_scoped_claude_memory_layout(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("根目录 `CLAUDE.md`", readme)
        self.assertIn("agent-rule/CLAUDE.md", readme)
        self.assertIn("tests/CLAUDE.md", readme)
        self.assertIn("仓库级高频记忆", readme)

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
        self.assertIn("make rules:sync", agents)
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
        self.assertIn("cleanup", architecture)
        self.assertIn("backups:restore", architecture)
        self.assertIn("clean-logs", architecture)
        self.assertIn("会修改本地状态的维护动作", architecture)

    def test_architecture_documents_local_override_and_terminal_policy(self):
        architecture = (ROOT / "docs" / "ARCHITECTURE.md").read_text(encoding="utf-8")
        self.assertIn("manager.local.json", architecture)
        self.assertIn("终端配置默认关闭", architecture)

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

    def test_claude_memory_stays_project_specific(self):
        memory = (ROOT / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertNotIn("Primary Semantic Search", memory)
        self.assertNotIn("Verification is CRITICAL", memory)
        self.assertIn("config/rules-targets.list", memory)

    def test_root_claude_md_stays_concise_and_points_to_scoped_memory(self):
        memory = (ROOT / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertLessEqual(len(memory.splitlines()), 80)
        self.assertIn("make smoke", memory)
        self.assertIn("make sync", memory)
        self.assertIn("make doctor", memory)
        self.assertIn("agent-rule/CLAUDE.md", memory)
        self.assertIn("tests/CLAUDE.md", memory)
        self.assertNotIn("Agent 配置文件路径", memory)
        self.assertNotIn("~/.cursor/mcp.json", memory)
        self.assertNotIn("agent-lifecycle | `sync_agents.py`", memory)

    def test_agent_rule_claude_md_stays_narrow_and_build_focused(self):
        memory = (ROOT / "agent-rule" / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn("agent-rule/rules/src/", memory)
        self.assertIn("agent-rule/rules/dist/", memory)
        self.assertIn("make rules-build", memory)
        self.assertIn("make rules:sync", memory)
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
