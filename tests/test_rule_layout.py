import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class RuleLayoutTests(unittest.TestCase):
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
            for target in ("codex", "claude", "gemini", "ide"):
                path = temp_root / "agent-rule" / "rules" / "dist" / target / "main.md"
                self.assertTrue(path.exists(), f"missing built rule for {target}: {combined}")

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
        expected_clause = "Use Chinese (Simplified) for interactions and generated content."
        for target in ("codex", "claude", "gemini", "ide"):
            content = (ROOT / "agent-rule" / "rules" / "dist" / target / "main.md").read_text(encoding="utf-8")
            self.assertIn(expected_clause, content, target)
            self.assertIn("modules/artifact-hygiene.md", content, target)

    def test_committed_dist_matches_fresh_build_output(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = self._copy_repo(temp_dir)

            result = self._run_builder(temp_root)

            combined = result.stdout + result.stderr
            self.assertEqual(result.returncode, 0, combined)
            self.assertEqual(self._dist_snapshot(ROOT), self._dist_snapshot(temp_root))
