import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "audit_skill_references.py"


def _run(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root-dir", str(root)],
        capture_output=True,
        text=True,
    )


def _write_min_root(tmp: str, skill_paths, core_body: str, installed=None, retired=None) -> Path:
    root = Path(tmp)
    (root / "config").mkdir()
    agents = {}
    if installed:
        inst = root / "installed-skills"
        inst.mkdir()
        for name in installed:
            (inst / name).mkdir()
        agents["local"] = str(inst)
    (root / "config" / "skills.json").write_text(
        json.dumps({"repositories": {"agent-skills": {"skill_paths": skill_paths}}, "agents": agents}),
        encoding="utf-8",
    )
    if retired is not None:
        (root / "config" / "retired-skills.json").write_text(
            json.dumps({"retired": retired}), encoding="utf-8"
        )
    shared = root / "agent-rule" / "rules" / "src" / "shared"
    shared.mkdir(parents=True)
    (shared / "core.md").write_text(core_body, encoding="utf-8")
    return root


class SkillReferenceAuditTests(unittest.TestCase):
    def test_current_repo_has_no_dead_skill_references(self):
        # Fail-close gate: managed rules/SKILL.md must not route to a non-existent skill.
        result = _run(ROOT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_detects_reference_to_absent_skill(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_min_root(
                tmp,
                skill_paths=["change-discipline"],
                core_body="- use the `ghost-skill` skill.\n",
            )
            result = _run(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("ghost-skill", result.stdout)

    def test_declared_skill_reference_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_min_root(
                tmp,
                skill_paths=["change-discipline"],
                core_body="- use the `change-discipline` skill.\n",
            )
            self.assertEqual(_run(root).returncode, 0)

    def test_detects_bare_retired_name(self):
        # A retired name in a table cell / prose (no `<name>` skill form) is still caught
        # via the retired-skills blacklist.
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_min_root(
                tmp,
                skill_paths=["change-discipline"],
                core_body="| Fix defects | fix-ones-bug |\n",
                retired=["fix-ones-bug"],
            )
            result = _run(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("fix-ones-bug", result.stdout)

    def test_installed_external_skill_is_not_flagged(self):
        # A cross-source skill (installed via another provider, absent from skill_paths)
        # must not be a false positive.
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_min_root(
                tmp,
                skill_paths=["change-discipline"],
                core_body="- use the `xiaoyu-bug-loop` skill.\n",
                installed=["xiaoyu-bug-loop"],
            )
            self.assertEqual(_run(root).returncode, 0, _run(root).stdout)


if __name__ == "__main__":
    unittest.main()
