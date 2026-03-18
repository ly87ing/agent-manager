import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ProjectContractTests(unittest.TestCase):
    def test_init_secrets_guidance_uses_update_or_bootstrap(self):
        script = (ROOT / "scripts" / "init-secrets.sh").read_text(encoding="utf-8")
        self.assertIn("make update", script)
        self.assertIn("make bootstrap", script)
        self.assertNotIn("初始化完成！现在可以运行：make all", script)

    def test_makefile_exposes_smoke_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(".PHONY: all bootstrap sync update plan self-update smoke", makefile)
        self.assertIn("\nsmoke:\n\t@./scripts/ci-smoke.sh\n", makefile)

    def test_ci_smoke_script_exercises_safe_preview_flows(self):
        smoke_script = ROOT / "scripts" / "ci-smoke.sh"
        self.assertTrue(smoke_script.exists(), "missing ci smoke script")

        content = smoke_script.read_text(encoding="utf-8")
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

    def test_readme_cleanup_contract_matches_runtime_scope(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("精准回收受管备份、碎片及冗余日志", readme)
        self.assertNotIn("孤儿仓库", readme)
