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
        "gemini": "GEMINI.md",
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

    def test_build_copies_shared_modules(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            modules_dir = temp_root / "agent-rule" / "rules" / "dist" / "modules"
            self.assertTrue((modules_dir / "architecture.md").exists(), combined)
            self.assertTrue((modules_dir / "style.md").exists(), combined)
            self.assertTrue((modules_dir / "frontend.md").exists(), combined)
            self.assertTrue((modules_dir / "artifact-hygiene.md").exists(), combined)

    def test_frontend_module_captures_browser_tool_boundaries(self):
        frontend_module = (ROOT / "agent-rule" / "rules" / "dist" / "modules" / "frontend.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("Choose by Intent, Not Tool Availability", frontend_module)
        self.assertIn("Use Playwright for reproducible assertions, regression coverage", frontend_module)
        self.assertIn("Use Chrome DevTools for one-off visual inspection", frontend_module)
        self.assertIn("Use `agent-browser` for persistent multi-step web tasks", frontend_module)
        self.assertIn("Preserve Authenticated Context Deliberately", frontend_module)
        self.assertIn("report the tool failure explicitly before falling back", frontend_module)

    def test_codex_module_paths_follow_manifest_target(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            manifest_path = temp_root / "config" / "rules-targets.list"
            manifest = manifest_path.read_text(encoding="utf-8")
            manifest_path.write_text(
                manifest.replace(
                    "codex_modules|global|rules/dist/modules|~/.codex/modules",
                    "codex_modules|global|rules/dist/modules|~/.codex-alt/modules",
                ),
                encoding="utf-8",
            )

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            codex_rule = (temp_root / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md").read_text(
                encoding="utf-8"
            )
            self.assertIn("read `~/.codex-alt/modules/architecture.md`", codex_rule)
            self.assertNotIn("read `~/.codex/modules/architecture.md`", codex_rule)

    def test_codex_rewrites_new_shared_module_references(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)
            core_path = temp_root / "agent-rule" / "rules" / "src" / "shared" / "core.md"
            core_path.write_text(
                core_path.read_text(encoding="utf-8")
                + "- If custom verification rules change, read `modules/custom.md` before editing.\n",
                encoding="utf-8",
            )
            (temp_root / "agent-rule" / "rules" / "src" / "shared" / "modules" / "custom.md").write_text(
                "# Custom\n",
                encoding="utf-8",
            )

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            codex_rule = (temp_root / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md").read_text(
                encoding="utf-8"
            )
            self.assertIn("read `~/.codex/modules/custom.md`", codex_rule)
            self.assertNotIn("read `modules/custom.md`", codex_rule)

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
        removed_language_clause = "Use Chinese (Simplified) for interactions and generated content."
        explicit_web_search_clause = (
            "For internet searches: strictly default to `web_search` and `web_fetch` (grok-search)."
        )
        explicit_verification_clause = (
            "If you skip verification, cannot run it, or ran something that did not actually exercise "
            "the change, say so explicitly."
        )
        explicit_context_clause = (
            "Before edits that change behavior, touch shared code, or span multiple files, inspect the affected "
            "code and nearby tests first."
        )
        for target, filename in self.DIST_FILENAMES.items():
            content = (ROOT / "agent-rule" / "rules" / "dist" / target / filename).read_text(encoding="utf-8")
            self.assertNotIn(removed_language_clause, content, target)
            self.assertNotIn("For codebase exploration:", content, target)
            self.assertNotIn("codebase-retrieval", content, target)
            self.assertNotIn("augment MCP", content, target)
            self.assertIn(explicit_web_search_clause, content, target)
            self.assertIn(explicit_verification_clause, content, target)
            self.assertNotIn("graphify-out/GRAPH_REPORT.md", content, target)
            self.assertIn(explicit_context_clause, content, target)
            self.assertIn("modules/artifact-hygiene.md", content, target)

    def test_built_outputs_stay_compact_and_omit_scaffolding_text(self):
        forbidden_fragments = (
            "Generated by agent-rule/scripts/build-agent-rules.py",
            "# Agent Behavior Guidelines",
            "# Claude Adapter",
            "# Gemini Adapter",
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
