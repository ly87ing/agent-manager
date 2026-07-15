import json
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class RuleLayoutTests(unittest.TestCase):
    DIST_FILENAMES = {
        "codex": "AGENTS.md",
        "claude": "CLAUDE.md",
        "antigravity": "rules/agent-manager.md",
    }

    def _copy_repo(self, temp_dir: str) -> Path:
        temp_root = Path(temp_dir) / "repo"
        shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories", ".git", ".worktrees"))
        return temp_root

    def _run_builder(self, temp_root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(temp_root / "agent-rule" / "scripts" / "build-agent-rules.py")],
            cwd=temp_root / "agent-rule",
            capture_output=True,
            text=True,
            check=False,
        )

    def _dist_snapshot(self, root: Path) -> dict[str, str]:
        dist_root = root / "agent-rule" / "rules" / "dist"
        return {
            str(path.relative_to(dist_root)): path.read_text(encoding="utf-8")
            for path in sorted(dist_root.rglob("*"))
            if path.is_file()
        }

    def test_build_generates_target_rule_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            for target, filename in self.DIST_FILENAMES.items():
                path = temp_root / "agent-rule" / "rules" / "dist" / target / filename
                self.assertTrue(path.exists(), f"missing built rule for {target}: {combined}")
            self.assertFalse(
                (temp_root / "agent-rule" / "rules" / "dist" / "codex" / "main.md").exists(),
                combined,
            )
            self.assertFalse(
                (temp_root / "agent-rule" / "rules" / "dist" / "ide").exists(),
                combined,
            )
            self.assertFalse((temp_root / "agent-rule" / "rules" / "dist" / "copilot").exists(), combined)

    def test_build_omits_shared_modules(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            modules_dir = temp_root / "agent-rule" / "rules" / "dist" / "modules"
            self.assertFalse(modules_dir.exists(), combined)

    def test_frontend_verification_skill_captures_browser_tool_boundaries(self):
        frontend_skill = (
            ROOT
            / "agent-skill"
            / "repositories"
            / "agent-skills"
            / "frontend-verification"
            / "SKILL.md"
        ).read_text(encoding="utf-8")
        self.assertIn("Choose the browser tool by intent", frontend_skill)
        self.assertIn("Use Playwright for reproducible assertions, regression coverage", frontend_skill)
        self.assertIn("Use Chrome DevTools for one-off visual inspection", frontend_skill)
        self.assertIn("Preserve authenticated context deliberately", frontend_skill)
        self.assertIn("report that tool failure before falling back", frontend_skill)

    def test_build_removes_stale_dist_outputs(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            stale_dir = temp_root / "agent-rule" / "rules" / "dist" / "obsolete"
            stale_dir.mkdir(parents=True, exist_ok=True)
            (stale_dir / "main.md").write_text("stale\n", encoding="utf-8")

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertFalse(stale_dir.exists(), combined)

    def test_built_outputs_include_required_shared_clauses(self):
        language_clause = (
            "Use Chinese (Simplified) for interactions and generated content. "
            "Use English terms only when clearer."
        )
        required_clauses = (
            "For internet searches: strictly default to `web_search` and `web_fetch` (grok-search).",
            (
                "If you skip verification, cannot run it, or ran something that did not actually exercise "
                "the change, say so explicitly."
            ),
            (
                "Before edits that change behavior, touch shared code, or span multiple files, inspect the affected "
                "code and nearby tests first."
            ),
            "Never delete tests, make tests easier, loosen assertions, or skip/xfail failures just to make code pass",
            (
                "Prefer the simplest local change. No speculative features, abstractions, config, "
                "or impossible-case handling."
            ),
            "Keep edits surgical: match local style, avoid unrelated cleanup, and remove only dead code made obsolete.",
            "Never publish work product to a remote or shared location without explicit user consent.",
            "`change-discipline` skill",
            "`frontend-verification` skill",
            "`artifact-hygiene` skill",
            "`reader-facing-writing` skill",
        )
        for target, filename in self.DIST_FILENAMES.items():
            content = (ROOT / "agent-rule" / "rules" / "dist" / target / filename).read_text(encoding="utf-8")
            self.assertIn(language_clause, content, target)
            self.assertNotIn("modules/", content, target)
            self.assertNotIn("For codebase exploration:", content, target)
            self.assertNotIn("codebase-retrieval", content, target)
            self.assertNotIn("augment MCP", content, target)
            self.assertNotIn("graphify-out/GRAPH_REPORT.md", content, target)
            for clause in required_clauses:
                self.assertIn(clause, content, target)

    def test_shared_core_routes_only_to_existing_skills(self):
        core = (ROOT / "agent-rule" / "rules" / "src" / "shared" / "core.md").read_text(encoding="utf-8")
        referenced = set(re.findall(r"use the `([a-z0-9-]+)` skill", core))
        self.assertTrue(referenced, "shared core.md should route to at least one skill")

        skills_config = json.loads((ROOT / "config" / "skills.json").read_text(encoding="utf-8"))
        available = set(skills_config["repositories"]["agent-skills"]["skill_paths"])

        missing = sorted(referenced - available)
        self.assertEqual(
            missing,
            [],
            f"shared core.md routes to skills absent from agent-skills skill_paths: {missing}",
        )

    def test_built_outputs_stay_compact_and_omit_scaffolding_text(self):
        forbidden_fragments = (
            "Generated by agent-rule/scripts/build-agent-rules.py",
            "# Agent Behavior Guidelines",
            "# Claude Adapter",
            "# Gemini Adapter",
            "# Antigravity Adapter",
            "# IDE Adapter",
        )
        for target, filename in self.DIST_FILENAMES.items():
            content = (ROOT / "agent-rule" / "rules" / "dist" / target / filename).read_text(encoding="utf-8")
            for fragment in forbidden_fragments:
                self.assertNotIn(fragment, content, f"{target} should omit scaffolding text: {fragment}")
            self.assertNotIn("\n\n", content.strip(), f"{target} should not contain empty spacer lines")

    def test_committed_dist_matches_fresh_build_output(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertEqual(self._dist_snapshot(ROOT), self._dist_snapshot(temp_root))

    def test_legacy_global_rule_tree_has_been_removed(self):
        legacy_root = ROOT / "agent-rule" / "rules" / "global"
        self.assertFalse(legacy_root.exists(), "legacy rules/global tree should not exist once dist is the only artifact source")

    def test_legacy_materialized_home_rule_tree_has_been_removed(self):
        legacy_home_root = ROOT / "agent-rule" / "~"
        self.assertFalse(
            legacy_home_root.exists(),
            "materialized legacy home-rule fixtures should not live under agent-rule/~",
        )
