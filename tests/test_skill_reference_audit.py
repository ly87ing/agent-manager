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


def _write_discovered_root(tmp: str, repos, core_body: str) -> Path:
    """repos: {repo_name: {"skill_paths": [...], "skills": {rel_dir: frontmatter_name}}}

    与 _write_min_root 的差别：附带本地仓库缓存，让已知宇宙走"发现结果"路径。
    """
    root = Path(tmp)
    (root / "config").mkdir()
    repositories = {}
    for repo_name, spec in repos.items():
        repositories[repo_name] = {"skill_paths": spec["skill_paths"]}
        cache = root / "agent-skill" / "repositories" / repo_name
        for rel_dir, skill_name in spec.get("skills", {}).items():
            skill_dir = cache / rel_dir
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text(
                f"---\nname: {skill_name}\ndescription: demo\n---\n\n# {skill_name}\n",
                encoding="utf-8",
            )
    (root / "config" / "skills.json").write_text(
        json.dumps({"repositories": repositories, "agents": {}}),
        encoding="utf-8",
    )
    shared = root / "agent-rule" / "rules" / "src" / "shared"
    shared.mkdir(parents=True)
    (shared / "core.md").write_text(core_body, encoding="utf-8")
    return root


class SkillReferenceDiscoveryUniverseTests(unittest.TestCase):
    def test_universe_uses_frontmatter_name_from_discovered_cache(self):
        # 发现语义：skill 身份 = SKILL.md frontmatter name
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_discovered_root(
                tmp,
                {"agent-skills": {"skill_paths": ["pretty-dir"], "skills": {"pretty-dir": "real-name"}}},
                "- use the `real-name` skill.\n",
            )
            self.assertEqual(_run(root).returncode, 0, _run(root).stdout)

    def test_dirname_is_not_a_skill_identity_when_cache_present(self):
        # 反例：目录名不是 skill 身份；缓存存在时不再靠 basename 放通
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_discovered_root(
                tmp,
                {"agent-skills": {"skill_paths": ["pretty-dir"], "skills": {"pretty-dir": "real-name"}}},
                "- use the `pretty-dir` skill.\n",
            )
            result = _run(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("pretty-dir", result.stdout)

    def test_discovery_scope_parent_dir_covers_nested_skills(self):
        # v2 发现范围：skill_paths 指向父目录时，范围内所有 skill 都入已知宇宙
        with tempfile.TemporaryDirectory() as tmp:
            root = _write_discovered_root(
                tmp,
                {
                    "agent-skills": {
                        "skill_paths": ["bundle"],
                        "skills": {"bundle/skill-a": "alpha", "bundle/skill-b": "beta"},
                    }
                },
                "- use the `alpha` skill.\n- and the `beta` skill.\n",
            )
            self.assertEqual(_run(root).returncode, 0, _run(root).stdout)

if __name__ == "__main__":
    unittest.main()
